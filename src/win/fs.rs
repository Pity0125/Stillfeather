//! 檔案系統列舉：資料夾清單、檔名搜尋、樹狀子資料夾、磁碟與已知資料夾。
//! 只讀取中繼資料，不開啟檔案內容；全部在背景執行緒執行並可取消。

use super::sys::*;
use super::util::*;
use feather_core::entry::Entry;
use feather_core::{paths, sorting};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub const SEARCH_MAX_RESULTS: usize = 50_000;
pub const SEARCH_BATCH: usize = 128;
pub const SEARCH_BATCH_MS: u128 = 100;
pub const TREE_MAX_CHILDREN: usize = 2_000;

#[derive(Debug)]
pub enum FsError {
    Cancelled,
    Message(String),
}

impl From<String> for FsError {
    fn from(s: String) -> Self {
        FsError::Message(s)
    }
}

pub fn attributes(path: &str) -> Option<u32> {
    let w = wide(&paths::to_long_path(path));
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    if a == INVALID_FILE_ATTRIBUTES {
        None
    } else {
        Some(a)
    }
}

pub fn is_dir(path: &str) -> bool {
    attributes(path).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0)
}

pub fn is_file(path: &str) -> bool {
    attributes(path).is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0)
}

/// 本機的硬碟、隨身碟或 RAM 磁碟上的路徑（只看路徑與磁碟機類型，不碰那個位置本身）。
/// 網路位置（UNC、對應的網路磁碟機）離線時，連 [`is_file`]、[`is_dir`] 都要等到 SMB 逾時（約 20 秒）；
/// 光碟機也要等轉起來。這些位置不要在介面執行緒上檢查，交給背景讀取（見 [`read_dir_or_file`]）。
pub fn on_local_drive(path: &str) -> bool {
    if paths::is_unc(path) {
        return false;
    }
    let Some(root) = paths::drive_root(path) else { return false };
    let w = wide(&root);
    matches!(unsafe { GetDriveTypeW(w.as_ptr()) }, DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK)
}

/// 屬性、大小與修改時間（FILETIME）。預覽前檢查，以及判斷快取的預覽是否還是同一份檔案。
pub fn attributes_size_modified(path: &str) -> Option<(u32, u64, u64)> {
    let w = wide(&paths::to_long_path(path));
    let mut data: WIN32_FILE_ATTRIBUTE_DATA = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileAttributesExW(w.as_ptr(), GET_FILE_EX_INFO_STANDARD, &mut data) };
    if ok == 0 {
        None
    } else {
        let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
        let modified = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32) | data.ftLastWriteTime.dwLowDateTime as u64;
        Some((data.dwFileAttributes, size, modified))
    }
}

/// 是否為「連結」型的重新解析點（junction、符號連結）。GetFileAttributes 只看得出有重新解析點，
/// 要再讀標籤才分得出 OneDrive 等雲端檔案（不是連結，照一般項目處理）。讀不到標籤時保守地當作連結。
pub fn is_link_path(path: &str) -> bool {
    let w = wide(&paths::to_long_path(paths::trim_end_sep(path)));
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    let handle = unsafe { FindFirstFileExW(w.as_ptr(), FIND_EX_INFO_BASIC, &mut data, FIND_EX_SEARCH_NAME_MATCH, std::ptr::null(), 0) };
    if handle == INVALID_HANDLE_VALUE {
        return attributes(path).is_some_and(|a| a & FILE_ATTRIBUTE_REPARSE_POINT != 0);
    }
    unsafe {
        FindClose(handle);
    }
    data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && paths::reparse_is_link(data.dwReserved0)
}

struct Finder {
    handle: HANDLE,
    data: WIN32_FIND_DATAW,
    first: bool,
}

enum Next {
    Item,
    End,
    Error(u32),
}

impl Finder {
    /// 開始列舉；空的磁碟根目錄會回傳 Ok(None)。
    fn open(dir: &str) -> Result<Option<Finder>, u32> {
        let pattern = wide(&paths::to_long_path(&paths::join(dir, "*")));
        let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
        let handle = unsafe {
            FindFirstFileExW(
                pattern.as_ptr(),
                FIND_EX_INFO_BASIC,
                &mut data,
                FIND_EX_SEARCH_NAME_MATCH,
                std::ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let code = unsafe { GetLastError() };
            return if code == ERROR_FILE_NOT_FOUND { Ok(None) } else { Err(code) };
        }
        Ok(Some(Finder { handle, data, first: true }))
    }

    fn next(&mut self) -> Next {
        if self.first {
            self.first = false;
            return Next::Item;
        }
        if unsafe { FindNextFileW(self.handle, &mut self.data) } != 0 {
            Next::Item
        } else {
            match unsafe { GetLastError() } {
                ERROR_NO_MORE_FILES => Next::End,
                code => Next::Error(code),
            }
        }
    }

    fn name(&self) -> String {
        from_wide(&self.data.cFileName)
    }

    fn is_dots(&self) -> bool {
        let n = &self.data.cFileName;
        (n[0] == b'.' as u16 && n[1] == 0) || (n[0] == b'.' as u16 && n[1] == b'.' as u16 && n[2] == 0)
    }

    fn attrs(&self) -> u32 {
        self.data.dwFileAttributes
    }

    /// 連結（junction、符號連結）；OneDrive 等雲端檔案雖然也有重新解析點，但不算連結。
    fn is_link(&self) -> bool {
        self.attrs() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && paths::reparse_is_link(self.data.dwReserved0)
    }

    fn entry(&self, dir: &Arc<str>) -> Entry {
        let a = self.attrs();
        let d = &self.data;
        Entry {
            name: self.name(),
            dir: dir.clone(),
            is_dir: a & FILE_ATTRIBUTE_DIRECTORY != 0,
            size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
            modified: ((d.ftLastWriteTime.dwHighDateTime as u64) << 32) | d.ftLastWriteTime.dwLowDateTime as u64,
            is_link: self.is_link(),
        }
    }
}

impl Drop for Finder {
    fn drop(&mut self) {
        unsafe {
            FindClose(self.handle);
        }
    }
}

fn require_dir(path: &str, reject_reparse: bool) -> Result<u32, FsError> {
    check_dir(path, attributes(path), reject_reparse)
}

/// `attrs`：剛用 [`attributes`] 取得的屬性（失敗時的原因還在 GetLastError 裡）
fn check_dir(path: &str, attrs: Option<u32>, reject_reparse: bool) -> Result<u32, FsError> {
    let a = match attrs {
        Some(a) => a,
        None => {
            let code = unsafe { GetLastError() };
            return Err(FsError::Message(match code {
                ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => trf!("資料夾不存在：{path}", "The folder doesn't exist: {path}"),
                _ => error_text(code),
            }));
        }
    };
    if a & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(FsError::Message(trf!("這不是資料夾：{path}", "This isn't a folder: {path}")));
    }
    if reject_reparse && a & FILE_ATTRIBUTE_REPARSE_POINT != 0 && is_link_path(path) {
        return Err(FsError::Message(tr!("不支援搜尋目錄連結（junction 或符號連結），請改選實際目標資料夾。", "Searching directory links (junctions or symbolic links) isn't supported; choose the actual target folder instead.").into()));
    }
    Ok(a)
}

pub struct Listing {
    pub entries: Vec<Entry>,
    pub skipped: usize,
}

/// 讀取單一資料夾，不開啟檔案內容；根目錄的列舉錯誤會回報給呼叫端，不以空清單掩蓋。
pub fn read_dir(path: &str, show_hidden: bool, sort_col: usize, desc: bool, folders_on_top: bool, cancel: &AtomicBool) -> Result<Listing, FsError> {
    require_dir(path, false)?;
    list_dir(path, show_hidden, sort_col, desc, folders_on_top, cancel)
}

/// 同 [`read_dir`]，但 `path` 其實是檔案時回傳 `Ok(None)`，讓呼叫端改開它所在的資料夾並選取它。
/// 給還不知道是檔案還是資料夾的網路位置用：只碰一次那個位置就同時知道是哪一種，離線時也只等一次逾時。
pub fn read_dir_or_file(path: &str, show_hidden: bool, sort_col: usize, desc: bool, folders_on_top: bool, cancel: &AtomicBool) -> Result<Option<Listing>, FsError> {
    let attrs = attributes(path);
    if attrs.is_some_and(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0) {
        return Ok(None);
    }
    check_dir(path, attrs, false)?;
    list_dir(path, show_hidden, sort_col, desc, folders_on_top, cancel).map(Some)
}

fn list_dir(path: &str, show_hidden: bool, sort_col: usize, desc: bool, folders_on_top: bool, cancel: &AtomicBool) -> Result<Listing, FsError> {
    let dir: Arc<str> = Arc::from(path);
    let mut entries = Vec::new();
    let skipped = 0usize;
    let Some(mut f) = Finder::open(path).map_err(error_text)? else {
        return Ok(Listing { entries, skipped });
    };
    let mut count = 0usize;
    loop {
        match f.next() {
            Next::End => break,
            Next::Error(code) => return Err(FsError::Message(error_text(code))),
            Next::Item => {}
        }
        count += 1;
        if count % 512 == 0 && cancel.load(Ordering::Relaxed) {
            return Err(FsError::Cancelled);
        }
        if f.is_dots() {
            continue;
        }
        if !show_hidden && f.attrs() & FILE_ATTRIBUTE_HIDDEN != 0 {
            continue;
        }
        entries.push(f.entry(&dir));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(FsError::Cancelled);
    }
    sorting::sort_entries(&mut entries, sort_col, desc, folders_on_top);
    Ok(Listing { entries, skipped })
}

pub struct SearchSummary {
    pub scanned: usize,
    pub matched: usize,
    pub skipped: usize,
    pub truncated: bool,
}

/// 搜尋範圍
#[derive(Clone, Debug)]
pub enum SearchScope {
    /// 目前資料夾（與子資料夾）
    Folder(String),
    /// 整台電腦：所有本機磁碟與卸除式磁碟（不含網路磁碟與光碟機）
    Computer,
}

/// 「整台電腦」要搜尋的磁碟根目錄
pub fn computer_roots() -> Vec<String> {
    drives().into_iter().filter(|(_, kind)| matches!(*kind, DRIVE_FIXED | DRIVE_REMOVABLE)).map(|(root, _)| root).collect()
}

/// 子資料夾最深幾層（防止連結被換掉時無限遞迴）
const SEARCH_MAX_DEPTH: u16 = 128;

/// 搜尋整台電腦時略過的資料夾：系統的元件存放區與回收筒等，內容龐大、使用者幾乎不會要找，
/// 而且多半無法讀取，只會拖慢搜尋。
fn skip_in_computer_scope(dir: &str, name: &str) -> bool {
    let at_root = dir.len() <= 3; // 「C:\」
    if at_root {
        return name.eq_ignore_ascii_case("$Recycle.Bin") || name.eq_ignore_ascii_case("System Volume Information") || name.eq_ignore_ascii_case("$WinREAgent");
    }
    name.eq_ignore_ascii_case("WinSxS") && dir.len() == 10 && dir[3..].eq_ignore_ascii_case("Windows")
}

/// 等待列舉的資料夾（多個執行緒共用）
struct Work {
    stack: Vec<(String, u16)>,
    /// 正在列舉資料夾的執行緒數
    active: usize,
}

struct Shared<'a> {
    work: std::sync::Mutex<Work>,
    ready: std::sync::Condvar,
    /// 停止：取消、達到上限或根目錄讀取失敗
    halt: AtomicBool,
    cancel: &'a AtomicBool,
    needle: feather_core::namematch::Needle,
    recursive: bool,
    show_hidden: bool,
    computer: bool,
    /// 只搜尋單一資料夾時，根目錄讀不到要回報錯誤（整台電腦時讀不到的磁碟只算略過）
    strict_root: bool,
    scanned: std::sync::atomic::AtomicUsize,
    matched: std::sync::atomic::AtomicUsize,
    skipped: std::sync::atomic::AtomicUsize,
    truncated: AtomicBool,
    fatal: std::sync::Mutex<Option<String>>,
}

impl Shared<'_> {
    fn stopped(&self) -> bool {
        self.halt.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed)
    }

    fn fail(&self, message: String) {
        if let Ok(mut f) = self.fatal.lock() {
            f.get_or_insert(message);
        }
        self.halt.store(true, Ordering::Relaxed);
    }

    /// 取下一個資料夾；全部做完（或停止）時回傳 None。
    fn next(&self) -> Option<(String, u16)> {
        let mut w = self.work.lock().ok()?;
        loop {
            if self.stopped() {
                return None;
            }
            if let Some(item) = w.stack.pop() {
                w.active += 1;
                return Some(item);
            }
            if w.active == 0 {
                self.ready.notify_all();
                return None;
            }
            // 逾時也醒來，確認是否已取消
            w = self.ready.wait_timeout(w, std::time::Duration::from_millis(50)).ok()?.0;
        }
    }

    fn done(&self, subdirs: Vec<(String, u16)>) {
        if let Ok(mut w) = self.work.lock() {
            let more = !subdirs.is_empty();
            w.stack.extend(subdirs);
            w.active -= 1;
            if more || w.active == 0 {
                self.ready.notify_all();
            }
        }
    }
}

/// 一個搜尋執行緒：反覆取出資料夾、列舉、比對檔名，子資料夾放回共用的佇列。
fn search_worker(sh: &Shared, tx: std::sync::mpsc::Sender<Vec<Entry>>) {
    while let Some((dir, depth)) = sh.next() {
        let mut subdirs = Vec::new();
        let mut found = Vec::new();
        search_dir(sh, &dir, depth, &mut subdirs, &mut found);
        if !found.is_empty() && tx.send(found).is_err() {
            sh.halt.store(true, Ordering::Relaxed);
        }
        sh.done(subdirs);
    }
}

fn search_dir(sh: &Shared, dir: &str, depth: u16, subdirs: &mut Vec<(String, u16)>, found: &mut Vec<Entry>) {
    let root = depth == 0;
    let mut finder = match Finder::open(dir) {
        Ok(Some(f)) => f,
        Ok(None) => return,
        Err(code) => {
            if root && sh.strict_root {
                sh.fail(error_text(code));
            } else {
                sh.skipped.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
    };
    let mut dir_arc: Option<Arc<str>> = None;
    let mut scanned = 0usize;
    loop {
        match finder.next() {
            Next::End => break,
            Next::Error(code) => {
                if root && sh.strict_root {
                    sh.fail(error_text(code));
                } else {
                    sh.skipped.fetch_add(1, Ordering::Relaxed);
                }
                break;
            }
            Next::Item => {}
        }
        if finder.is_dots() {
            continue;
        }
        scanned += 1;
        if scanned.is_multiple_of(256) {
            sh.scanned.fetch_add(256, Ordering::Relaxed);
            if sh.stopped() {
                return;
            }
        }
        let a = finder.attrs();
        if !sh.show_hidden && a & FILE_ATTRIBUTE_HIDDEN != 0 {
            continue;
        }
        let is_dir = a & FILE_ATTRIBUTE_DIRECTORY != 0;
        let is_link = finder.is_link();
        // 目錄連結（junction、符號連結）一律不遞迴；OneDrive 等雲端資料夾照常搜尋
        if sh.recursive && is_dir && !is_link && depth < SEARCH_MAX_DEPTH {
            let name = finder.name();
            if !(sh.computer && skip_in_computer_scope(dir, &name)) {
                subdirs.push((paths::join(dir, &name), depth + 1));
            }
        }
        // 直接比對 UTF-16 檔名；不符合的項目不配置任何記憶體
        if sh.needle.matches(feather_core::namematch::until_nul(&finder.data.cFileName)) {
            // 只有真的還有下一筆符合時，才標示為「已達上限」。
            if sh.matched.fetch_add(1, Ordering::Relaxed) >= SEARCH_MAX_RESULTS {
                sh.truncated.store(true, Ordering::Relaxed);
                sh.halt.store(true, Ordering::Relaxed);
                break;
            }
            let arc = dir_arc.get_or_insert_with(|| Arc::from(dir)).clone();
            found.push(finder.entry(&arc));
        }
    }
    sh.scanned.fetch_add(scanned % 256, Ordering::Relaxed);
}

/// 只比對檔名，多個執行緒同時列舉資料夾。Scanned 含隱藏項目；Skipped 計算無法讀取的資料夾。
/// 目錄重新解析點一律不遞迴。結果每 128 筆或 100 毫秒分批回報（在呼叫端的執行緒上）。
pub fn search(
    scope: &SearchScope,
    query: &str,
    recursive: bool,
    show_hidden: bool,
    cancel: &AtomicBool,
    mut report: impl FnMut(Vec<Entry>, usize, usize),
) -> Result<SearchSummary, FsError> {
    let Some(needle) = feather_core::namematch::Needle::new(query) else {
        return Err(FsError::Message(tr!("請輸入要搜尋的檔案名稱。", "Enter a file name to search for.").into()));
    };
    let (roots, computer) = match scope {
        SearchScope::Folder(root) => {
            require_dir(root, true)?;
            (vec![root.clone()], false)
        }
        SearchScope::Computer => {
            let roots = computer_roots();
            if roots.is_empty() {
                return Err(FsError::Message(tr!("找不到可以搜尋的磁碟。", "Can't find any drives to search.").into()));
            }
            (roots, true)
        }
    };
    let sh = Shared {
        // 根目錄依反向放入，讓 C: 先被取出
        work: std::sync::Mutex::new(Work { stack: roots.into_iter().rev().map(|r| (r, 0)).collect(), active: 0 }),
        ready: std::sync::Condvar::new(),
        halt: AtomicBool::new(false),
        cancel,
        needle,
        recursive,
        show_hidden,
        computer,
        strict_root: !computer,
        scanned: Default::default(),
        matched: Default::default(),
        skipped: Default::default(),
        truncated: AtomicBool::new(false),
        fatal: std::sync::Mutex::new(None),
    };
    let threads = if recursive { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8) } else { 1 };
    let (tx, rx) = std::sync::mpsc::channel::<Vec<Entry>>();
    let mut batch: Vec<Entry> = Vec::with_capacity(SEARCH_BATCH);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let tx = tx.clone();
            let sh = &sh;
            scope.spawn(move || search_worker(sh, tx));
        }
        drop(tx);
        let mut last = Instant::now();
        loop {
            let disconnected = match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(mut entries) => {
                    batch.append(&mut entries);
                    false
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            };
            if cancel.load(Ordering::Relaxed) {
                sh.halt.store(true, Ordering::Relaxed);
            }
            if disconnected {
                break;
            }
            if batch.len() >= SEARCH_BATCH || last.elapsed().as_millis() >= SEARCH_BATCH_MS {
                report(std::mem::take(&mut batch), sh.scanned.load(Ordering::Relaxed), sh.skipped.load(Ordering::Relaxed));
                last = Instant::now();
            }
        }
    });
    let scanned = sh.scanned.load(Ordering::Relaxed);
    let skipped = sh.skipped.load(Ordering::Relaxed);
    if let Some(message) = sh.fatal.lock().ok().and_then(|mut f| f.take()) {
        return Err(FsError::Message(message));
    }
    report(batch, scanned, skipped);
    if cancel.load(Ordering::Relaxed) {
        return Err(FsError::Cancelled);
    }
    let matched = sh.matched.load(Ordering::Relaxed).min(SEARCH_MAX_RESULTS);
    Ok(SearchSummary { scanned, matched, skipped, truncated: sh.truncated.load(Ordering::Relaxed) })
}

pub struct ChildFolder {
    pub name: String,
    pub path: String,
    pub is_link: bool,
}

pub struct FolderListing {
    pub folders: Vec<ChildFolder>,
    pub truncated: bool,
}

/// 樹狀側欄展開時才讀取子資料夾，每次最多 2,000 個。
pub fn child_folders(path: &str, show_hidden: bool, cancel: &AtomicBool) -> Result<FolderListing, FsError> {
    require_dir(path, false)?;
    let mut folders = Vec::new();
    let mut truncated = false;
    let Some(mut f) = Finder::open(path).map_err(error_text)? else {
        return Ok(FolderListing { folders, truncated });
    };
    loop {
        match f.next() {
            Next::End => break,
            Next::Error(code) => return Err(FsError::Message(error_text(code))),
            Next::Item => {}
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(FsError::Cancelled);
        }
        let a = f.attrs();
        if f.is_dots() || a & FILE_ATTRIBUTE_DIRECTORY == 0 {
            continue;
        }
        if !show_hidden && a & FILE_ATTRIBUTE_HIDDEN != 0 {
            continue;
        }
        if folders.len() >= TREE_MAX_CHILDREN {
            truncated = true;
            break;
        }
        let name = f.name();
        folders.push(ChildFolder { path: paths::join(path, &name), name, is_link: f.is_link() });
    }
    folders.sort_by(|a, b| sorting::natural_cmp(&a.name, &b.name));
    Ok(FolderListing { folders, truncated })
}

/// 目前的磁碟根目錄與類型；只呼叫 GetDriveType，不讀取磁碟內容。
pub fn drives() -> Vec<(String, u32)> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
    let mut out = Vec::new();
    for part in buf[..n.min(buf.len())].split(|c| *c == 0) {
        if part.is_empty() {
            continue;
        }
        let root = String::from_utf16_lossy(part);
        let w = wide(&root);
        let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
        out.push((root, kind));
    }
    out
}

pub const FOLDERID_PROFILE: GUID = GUID::from_u128(0x5E6C858F_0E22_4760_9AFE_EA3317B67173);
pub const FOLDERID_DESKTOP: GUID = GUID::from_u128(0xB4BFCC3A_DB2C_424C_B029_7FE99A87C641);
pub const FOLDERID_DOCUMENTS: GUID = GUID::from_u128(0xFDD39AD0_238F_46AF_ADB4_6C85480369C7);
pub const FOLDERID_DOWNLOADS: GUID = GUID::from_u128(0x374DE290_123F_4565_9164_39C4925E467B);
pub const FOLDERID_PICTURES: GUID = GUID::from_u128(0x33E28130_4E1E_4676_835A_98395C3BC3BB);
pub const FOLDERID_MUSIC: GUID = GUID::from_u128(0x4BD8D571_6D19_48D3_BE97_422220080E43);
pub const FOLDERID_VIDEOS: GUID = GUID::from_u128(0x18989B1D_99B5_455B_841C_AB7C74E4DDFC);
pub const FOLDERID_SKYDRIVE: GUID = GUID::from_u128(0xA52BBA46_E9E1_435F_B3D9_28DAA648C0F6);

pub fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let mut p: *mut u16 = std::ptr::null_mut();
        let hr = SHGetKnownFolderPath(id, 0, 0, &mut p);
        if hr < 0 || p.is_null() {
            if !p.is_null() {
                CoTaskMemFree(p as *mut std::ffi::c_void);
            }
            return None;
        }
        let s = from_wide_ptr(p);
        CoTaskMemFree(p as *mut std::ffi::c_void);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

pub fn profile_dir() -> String {
    known_folder(&FOLDERID_PROFILE)
        .or_else(|| std::env::var("USERPROFILE").ok())
        .unwrap_or_else(|| "C:\\".to_string())
}

/// 不允許直接操作的系統與使用者設定檔根目錄。
pub fn protected_folders() -> Vec<String> {
    let mut out = Vec::new();
    for var in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData", "USERPROFILE"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                out.push(v);
            }
        }
    }
    if let Ok(root) = std::env::var("SystemRoot") {
        out.push(paths::join(&root, "System32"));
    }
    out.push(profile_dir());
    out
}

/// 路徑本身或任一上層是否為重新解析點（leaf_may_be_missing：最後一段可以不存在）。
pub fn reparse_in_ancestors(path: &str, leaf_may_be_missing: bool) -> Result<Option<String>, String> {
    let mut current = Some(paths::trim_end_sep(path).to_string());
    let mut leaf = true;
    while let Some(p) = current {
        match attributes(&p) {
            Some(a) if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 && is_link_path(&p) => return Ok(Some(p)),
            Some(_) => {}
            None if leaf && leaf_may_be_missing => {}
            None => return Err(trf!("無法讀取：{p}（{}）", "Couldn't read: {p} ({})", last_error_text())),
        }
        leaf = false;
        current = paths::parent(&p);
    }
    Ok(None)
}

/// 資料夾樹內是否含重新解析點；找到第一個就停止。
pub fn reparse_in_tree(root: &str) -> Result<Option<String>, String> {
    let mut pending = vec![root.to_string()];
    while let Some(dir) = pending.pop() {
        let Some(a) = attributes(&dir) else { return Err(trf!("無法讀取：{dir}（{}）", "Couldn't read: {dir} ({})", last_error_text())) };
        if a & FILE_ATTRIBUTE_REPARSE_POINT != 0 && is_link_path(&dir) {
            return Ok(Some(dir));
        }
        let mut f = match Finder::open(&dir) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(code) => return Err(trf!("無法列舉：{dir}（{}）", "Couldn't list: {dir} ({})", error_text(code))),
        };
        loop {
            match f.next() {
                Next::End => break,
                Next::Error(code) => return Err(trf!("無法列舉：{dir}（{}）", "Couldn't list: {dir} ({})", error_text(code))),
                Next::Item => {}
            }
            if f.is_dots() {
                continue;
            }
            let a = f.attrs();
            let child = paths::join(&dir, &f.name());
            if f.is_link() {
                return Ok(Some(child));
            }
            if a & FILE_ATTRIBUTE_DIRECTORY != 0 {
                pending.push(child);
            }
        }
    }
    Ok(None)
}

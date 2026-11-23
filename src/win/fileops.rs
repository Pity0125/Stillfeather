//! 複製、移動、刪除：在專用 STA 執行緒呼叫 Windows Shell（SHFileOperation），
//! 進度、衝突與取消對話方塊都由 Windows 處理。刪除一律先送資源回收筒；
//! 無法回收時 Windows 會先警告將永久刪除（FOF_WANTNUKEWARNING）。
//! 復原（Ctrl+Z）也在背景執行：把移動過的項目搬回原位、從資源回收筒找回刪除的項目。

use super::app::{self, WorkerMsg};
use super::fs;
use super::sys::*;
use super::util::*;
use feather_core::paths;
use feather_core::plan::{self, OpKind, Planned};
use feather_core::undo::{self, Action, RecycleInfo};
use std::collections::HashMap;

#[derive(Debug)]
pub struct OpReport {
    pub kind: OpKind,
    pub completed: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
    /// 這是復原操作（不再記入復原紀錄）
    pub undo: bool,
}

impl OpReport {
    fn rejected(kind: OpKind, count: usize, error: String) -> OpReport {
        OpReport { kind, completed: 0, skipped: count, errors: vec![error], cancelled: false, undo: false }
    }
}

/// 在背景執行，完成後送 WorkerMsg::FileOp 給主視窗。
pub fn start(kind: OpKind, sources: Vec<String>, destination: Option<String>, owner: HWND) {
    std::thread::spawn(move || {
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        }
        let report = execute(kind, &sources, destination.as_deref(), owner);
        unsafe {
            CoUninitialize();
        }
        app::post(WorkerMsg::FileOp(report));
    });
}

fn shell_error(code: i32) -> String {
    let text = match code {
        0x71 => tr!("來源與目的地是同一個檔案。", "The source and destination are the same file."),
        0x72 | 0x7A => tr!("來源與目的地數量不相符。", "The number of sources and destinations doesn't match."),
        0x73 => tr!("來源與目的地不在同一個資料夾。", "The source and destination aren't in the same folder."),
        0x74 => tr!("不能對根目錄執行這個操作。", "This operation can't be done on a root folder."),
        0x75 => tr!("操作已取消。", "The operation was cancelled."),
        0x76 | 0x7D => tr!("目的地位於來源資料夾之內。", "The destination is inside the source folder."),
        0x78 => tr!("無法存取來源，可能沒有權限。", "Can't access the source; you may not have permission."),
        0x79 => tr!("路徑過深。", "The path is too deep."),
        0x7C => tr!("來源或目的地路徑無效。", "The source or destination path is invalid."),
        0x7E => tr!("目的地是檔案而不是資料夾。", "The destination is a file, not a folder."),
        0x80 => tr!("目的地已有同名資料夾。", "A folder with the same name already exists at the destination."),
        0x81 => tr!("檔名或路徑過長。", "The file name or path is too long."),
        0x82..=0x84 => tr!("目的地是唯讀的光碟媒體。", "The destination is read-only disc media."),
        0x85 => tr!("檔案大小超過目的地檔案系統的上限。", "The file is larger than the destination file system allows."),
        0x86..=0x88 => tr!("來源是唯讀的光碟媒體。", "The source is read-only disc media."),
        0x10000 => tr!("目的地發生錯誤。", "An error occurred at the destination."),
        _ => "",
    };
    if !text.is_empty() {
        return text.to_string();
    }
    if (1..0x70).contains(&code) {
        return error_text(code as u32);
    }
    trf!("Windows Shell 錯誤 0x{code:X}", "Windows Shell error 0x{code:X}")
}

fn multi_sz(items: &[&str]) -> Vec<u16> {
    let mut v = Vec::new();
    for s in items {
        v.extend(s.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v
}

/// 呼叫一次 SHFileOperation。回傳 (是否被使用者取消/中止, 錯誤碼)。
fn shell_operation(kind: OpKind, sources: &[&str], dest: Option<&str>, rename_on_collision: bool, owner: HWND) -> (bool, i32) {
    let from = multi_sz(sources);
    let to = dest.map(|d| multi_sz(&[d]));
    let mut flags: u16 = FOF_ALLOWUNDO;
    let func = match kind {
        OpKind::Copy => FO_COPY,
        OpKind::Move => FO_MOVE,
        OpKind::Delete => FO_DELETE,
    };
    match kind {
        OpKind::Delete => flags |= FOF_NOCONFIRMATION | FOF_WANTNUKEWARNING,
        _ => flags |= FOF_NOCONFIRMMKDIR,
    }
    if rename_on_collision {
        flags |= FOF_RENAMEONCOLLISION;
    }
    let mut op = SHFILEOPSTRUCTW {
        hwnd: owner,
        wFunc: func,
        pFrom: from.as_ptr(),
        pTo: to.as_ref().map(|t| t.as_ptr()).unwrap_or(std::ptr::null()),
        fFlags: flags,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    let rc = unsafe { SHFileOperationW(&mut op) };
    (op.fAnyOperationsAborted != 0 || rc == 0x75 || rc == 1223, rc)
}

fn execute(kind: OpKind, sources: &[String], destination: Option<&str>, owner: HWND) -> OpReport {
    if sources.is_empty() {
        return OpReport { kind, completed: 0, skipped: 0, errors: vec![], cancelled: false, undo: false };
    }
    let protected = fs::protected_folders();
    let items = match plan::plan(kind, sources, destination, &protected) {
        Ok(items) => items,
        Err(e) => return OpReport::rejected(kind, sources.len(), e),
    };
    if let Some(dest) = destination {
        if !fs::is_dir(dest) {
            return OpReport::rejected(kind, sources.len(), tr!("目的地無法使用：目的地必須是已存在的資料夾。", "The destination can't be used: it must be an existing folder.").into());
        }
        match fs::reparse_in_ancestors(dest, false) {
            Ok(Some(p)) => {
                return OpReport::rejected(kind, sources.len(), trf!("目的地無法使用：此版本不處理連結（junction、符號連結）或透過連結的路徑：{p}", "The destination can't be used: this version doesn't handle links (junctions, symbolic links) or paths through them: {p}"))
            }
            Err(e) => return OpReport::rejected(kind, sources.len(), trf!("目的地無法使用：{e}", "The destination can't be used: {e}")),
            Ok(None) => {}
        }
    }

    let mut errors = Vec::new();
    let mut skipped = 0usize;
    let mut accepted: Vec<Planned> = Vec::new();
    for item in items {
        match item {
            Err(r) => {
                skipped += 1;
                errors.push(trf!("{}：{}", "{}: {}", r.label, r.reason));
            }
            Ok(p) => match check(kind, &p) {
                Ok(()) => accepted.push(p),
                Err(e) => {
                    skipped += 1;
                    errors.push(trf!("{}：{}", "{}: {}", p.source, e));
                }
            },
        }
    }

    // 同一資料夾內複製（自動改名）與一般操作分兩批，讓 Windows 對一般衝突仍會詢問。
    let mut completed = 0usize;
    for rename in [true, false] {
        let batch: Vec<&Planned> = accepted.iter().filter(|p| p.rename_on_collision == rename).collect();
        if batch.is_empty() {
            continue;
        }
        let srcs: Vec<&str> = batch.iter().map(|p| p.source.as_str()).collect();
        let (aborted, rc) = shell_operation(kind, &srcs, destination, rename, owner);
        if aborted {
            errors.push(trf!(
                "Windows 已取消或中止{}；可能已有部分內容完成，未自動復原。後續項目未執行。", "Windows cancelled or stopped the {}; some of it may already be done and wasn't undone automatically. The remaining items weren't processed.",
                kind.verb()
            ));
            let remaining = accepted.len() - completed;
            return OpReport { kind, completed, skipped: skipped + remaining, errors, cancelled: true, undo: false };
        }
        if rc != 0 {
            skipped += batch.len();
            errors.push(trn!(batch.len(); "{} 個項目：{} 可能已有部分內容完成，未自動復原。", "{} item: {} Some of it may already be done and wasn't undone automatically.", "{} items: {} Some of it may already be done and wasn't undone automatically.", batch.len(), shell_error(rc)));
        } else {
            completed += batch.len();
        }
    }
    OpReport { kind, completed, skipped, errors, cancelled: false, undo: false }
}

// ---------------------------------------------------------------- 復原

/// 在背景復原移動或刪除（重新命名在主執行緒直接處理）。完成後一樣送 WorkerMsg::FileOp。
pub fn start_undo(action: Action, owner: HWND) {
    std::thread::spawn(move || {
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        }
        let report = match action {
            Action::Move { items } => undo_moves(&items, owner),
            Action::Recycle { items, since, until } => undo_recycle(&items, since, until),
            Action::Rename { .. } => OpReport { kind: OpKind::Move, completed: 0, skipped: 0, errors: vec![], cancelled: false, undo: true },
        };
        unsafe {
            CoUninitialize();
        }
        app::post(WorkerMsg::FileOp(report));
    });
}

fn exists(path: &str) -> bool {
    fs::attributes(path).is_some()
}

fn ensure_parent(path: &str) -> Result<(), String> {
    match paths::parent(path) {
        Some(dir) if !fs::is_dir(&dir) => std::fs::create_dir_all(paths::to_long_path(&dir)).map_err(|e| trf!("無法建立原本的資料夾：{e}", "Couldn't create the original folder: {e}")),
        _ => Ok(()),
    }
}

/// 同一磁碟直接改路徑；不同磁碟（MoveFileEx 回報 ERROR_NOT_SAME_DEVICE）時交給 Windows 搬移，會顯示進度。
fn move_back(from: &str, to: &str, owner: HWND) -> Result<(), String> {
    let a = wide(&paths::to_long_path(from));
    let b = wide(&paths::to_long_path(to));
    if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) } != 0 {
        return Ok(());
    }
    let code = unsafe { GetLastError() };
    if code != ERROR_NOT_SAME_DEVICE {
        return Err(error_text(code));
    }
    let Some(dir) = paths::parent(to) else { return Err(tr!("找不到原本的資料夾。", "Can't find the original folder.").into()) };
    let (aborted, rc) = shell_operation(OpKind::Move, &[from], Some(&dir), false, owner);
    if aborted {
        Err(tr!("已取消。", "Cancelled.").into())
    } else if rc != 0 {
        Err(shell_error(rc))
    } else {
        Ok(())
    }
}

fn undo_moves(items: &[(String, String)], owner: HWND) -> OpReport {
    let mut report = OpReport { kind: OpKind::Move, completed: 0, skipped: 0, errors: vec![], cancelled: false, undo: true };
    for (original, moved) in items {
        let name = paths::file_name(original);
        let result = if !exists(moved) {
            Err(tr!("找不到移動後的項目，可能已被移動、重新命名或刪除。", "Can't find the moved item; it may have been moved, renamed or deleted.").to_string())
        } else if exists(original) {
            Err(tr!("原位置已有同名項目，沒有覆蓋。", "An item with the same name is already at the original location; it wasn't overwritten.").to_string())
        } else {
            ensure_parent(original).and_then(|()| move_back(moved, original, owner))
        };
        match result {
            Ok(()) => report.completed += 1,
            Err(e) => {
                report.skipped += 1;
                report.errors.push(trf!("{name}：{e}", "{name}: {e}"));
            }
        }
    }
    report
}

/// 一個磁碟的資源回收筒紀錄：（所在資料夾, $I 檔名, 內容）
type BinRecords = Vec<(std::path::PathBuf, String, RecycleInfo)>;

/// 讀出某個磁碟資源回收筒中的所有 `$I` 紀錄（只讀得到目前使用者自己的那一份）。
fn scan_bin(root: &str) -> BinRecords {
    let mut out = Vec::new();
    let bin = paths::join(root, "$Recycle.Bin");
    let Ok(users) = std::fs::read_dir(&bin) else { return out };
    for user in users.flatten() {
        if !user.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(user.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if !name.starts_with("$I") || f.metadata().is_ok_and(|m| m.len() > 65_536) {
                continue;
            }
            if let Some(info) = std::fs::read(f.path()).ok().and_then(|b| undo::parse_recycle_info(&b)) {
                out.push((user.path(), name, info));
            }
        }
    }
    out
}

fn undo_recycle(items: &[String], since: u64, until: u64) -> OpReport {
    let mut report = OpReport { kind: OpKind::Delete, completed: 0, skipped: 0, errors: vec![], cancelled: false, undo: true };
    let mut bins: HashMap<String, BinRecords> = HashMap::new();
    let mut touched: Vec<String> = Vec::new();
    for original in items {
        let name = paths::file_name(original).to_string();
        let result = (|| -> Result<(), String> {
            let root_len = paths::root_len(original);
            let root = original.get(..root_len).unwrap_or("");
            if root.is_empty() {
                return Err(tr!("路徑無效。", "Invalid path.").into());
            }
            if root.starts_with(r"\") && !root.starts_with(r"\?") {
                return Err(tr!("網路位置刪除的項目不會進資源回收筒，無法復原。", "Items deleted from network locations don't go to the Recycle Bin, so this can't be undone.").into());
            }
            if exists(original) {
                return Err(tr!("原位置已有同名項目，沒有覆蓋。", "An item with the same name is already at the original location; it wasn't overwritten.").into());
            }
            let records = bins.entry(root.to_lowercase()).or_insert_with(|| scan_bin(root));
            let infos: Vec<RecycleInfo> = records.iter().map(|r| r.2.clone()).collect();
            let Some(i) = undo::pick_restore(&infos, original, since, until) else {
                return Err(tr!("在資源回收筒找不到，可能已永久刪除或資源回收筒已清空。", "It's not in the Recycle Bin; it may have been permanently deleted or the Recycle Bin was emptied.").into());
            };
            let (dir, info_name, _) = records.remove(i);
            let data_name = undo::data_name_for(&info_name).ok_or(tr!("資源回收筒的紀錄格式不符。", "The Recycle Bin record has an unexpected format."))?;
            let data = dir.join(&data_name).to_string_lossy().into_owned();
            ensure_parent(original)?;
            super::fileops::rename(&data, original)?;
            let _ = std::fs::remove_file(dir.join(&info_name));
            touched.push(dir.to_string_lossy().into_owned());
            if let Some(p) = paths::parent(original) {
                touched.push(p);
            }
            Ok(())
        })();
        match result {
            Ok(()) => report.completed += 1,
            Err(e) => {
                report.skipped += 1;
                report.errors.push(trf!("{name}：{e}", "{name}: {e}"));
            }
        }
    }
    // 讓檔案總管與資源回收筒視窗知道內容變了
    touched.sort();
    touched.dedup();
    for dir in touched {
        let w = wide(&dir);
        unsafe {
            SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW, w.as_ptr() as *const std::ffi::c_void, std::ptr::null());
        }
    }
    report
}

/// 需要讀檔案系統的檢查（移植自 C#）。
fn check(kind: OpKind, p: &Planned) -> Result<(), String> {
    let Some(attrs) = fs::attributes(&p.source) else {
        return Err(trf!("找不到或無法讀取來源（{}）", "Can't find or read the source ({})", last_error_text()));
    };
    if kind == OpKind::Delete {
        return Ok(());
    }
    if let Some(link) = fs::reparse_in_ancestors(&p.source, false)? {
        return Err(trf!("此版本不處理連結（junction、符號連結）或透過連結的路徑：{link}", "This version doesn't handle links (junctions, symbolic links) or paths through them: {link}"));
    }
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_dir {
        if let Some(link) = fs::reparse_in_tree(&p.source)? {
            return Err(trf!("資料夾內含連結（junction、符號連結），未開始操作：{link}", "The folder contains links (junctions, symbolic links), so the operation wasn't started: {link}"));
        }
    }
    if let Some(target) = &p.target {
        if p.rename_on_collision {
            return Ok(());
        }
        if let Some(link) = fs::reparse_in_ancestors(target, true)? {
            return Err(trf!("此版本不處理連結（junction、符號連結）或透過連結的路徑：{link}", "This version doesn't handle links (junctions, symbolic links) or paths through them: {link}"));
        }
        if let Some(ta) = fs::attributes(target) {
            let target_dir = ta & FILE_ATTRIBUTE_DIRECTORY != 0;
            if target_dir != is_dir {
                return Err(tr!("目的地已有不同類型的同名項目，已略過。", "An item of a different type with the same name is already at the destination; skipped.").into());
            }
            if target_dir {
                if let Some(link) = fs::reparse_in_tree(target)? {
                    return Err(trf!("目的資料夾內含連結（junction、符號連結），未開始操作：{link}", "The destination folder contains links (junctions, symbolic links), so the operation wasn't started: {link}"));
                }
            }
        }
    }
    Ok(())
}

/// 在指定資料夾建立「新增資料夾」，名稱重複時加上 (2)、(3)…
pub fn create_new_folder(dir: &str) -> Result<String, String> {
    for i in 1..1000 {
        let name = if i == 1 { tr!("新增資料夾", "New folder").to_string() } else { trf!("新增資料夾 ({i})", "New folder ({i})") };
        let path = feather_core::paths::join(dir, &name);
        if fs::attributes(&path).is_some() {
            continue;
        }
        let w = wide(&feather_core::paths::to_long_path(&path));
        if unsafe { CreateDirectoryW(w.as_ptr(), std::ptr::null()) } != 0 {
            return Ok(path);
        }
        let code = unsafe { GetLastError() };
        if code != ERROR_ALREADY_EXISTS {
            return Err(error_text(code));
        }
    }
    Err(tr!("無法決定新資料夾名稱。", "Couldn't pick a name for the new folder.").into())
}

pub fn rename(from: &str, to: &str) -> Result<(), String> {
    let a = wide(&feather_core::paths::to_long_path(from));
    let b = wide(&feather_core::paths::to_long_path(to));
    if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        let code = unsafe { GetLastError() };
        Err(if code == ERROR_ALREADY_EXISTS || code == ERROR_ALREADY_EXISTS_FILE {
            tr!("相同名稱已存在。", "An item with that name already exists.").to_string()
        } else {
            error_text(code)
        })
    }
}

/// 以系統關聯程式開啟檔案。
pub fn shell_open(owner: HWND, path: &str) -> Result<(), String> {
    let file = wide(path);
    let dir = feather_core::paths::parent(path).map(|d| wide(&d));
    let rc = unsafe {
        ShellExecuteW(
            owner,
            std::ptr::null(),
            file.as_ptr(),
            std::ptr::null(),
            dir.as_ref().map(|d| d.as_ptr()).unwrap_or(std::ptr::null()),
            SW_SHOWNORMAL,
        )
    };
    if rc > 32 {
        Ok(())
    } else {
        Err(match rc {
            2 | 3 => tr!("找不到檔案。", "File not found.").to_string(),
            5 => tr!("拒絕存取。", "Access denied.").to_string(),
            31 => tr!("沒有可開啟這種檔案的應用程式。", "No app is associated with this type of file.").to_string(),
            _ => last_error_text(),
        })
    }
}

/// 顯示 Windows 的「內容」對話方塊。
pub fn show_properties(owner: HWND, path: &str) {
    let verb = wide("properties");
    let file = wide(path);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_INVOKEIDLIST;
    info.hwnd = owner;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.nShow = SW_SHOW;
    unsafe {
        ShellExecuteExW(&mut info);
    }
}

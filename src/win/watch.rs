//! 自動重新整理：畫面上顯示的資料夾（單欄一個、雙欄最多兩個）有變更時，清單自動更新。
//!
//! - 變更通知用 `SHChangeNotifyRegister`，和檔案總管視窗同一個來源：Stillfeather 或檔案總管裡的操作
//!   （Shell 層級）會立刻收到，其他程式直接寫入檔案系統的變更（中斷層級）也會收到。
//!   監看由 Windows 殼層（explorer.exe）代為進行，Stillfeather 自己不開啟資料夾的監看代號，
//!   不會因此多擋住資料夾改名、刪除或退出隨身碟。
//! - 收到通知後稍等一下，在背景重新讀取；同一個分頁連續變更時約每秒更新一次。
//! - 讀回來的內容和畫面上一樣就什麼都不動；不一樣才換掉，保留選取、焦點與捲動位置，不回到頂端。
//! - 正在重新命名、拖放、選單開著或按著滑鼠（框選、拖曳）時先等一下再更新。
//! - 顯示中的資料夾本身被改名或搬走時改顯示新的位置；被刪除時改顯示還在的上一層。
//! - 隱藏中的分頁不監看，切回來時補讀一次。
//!
//! 需要的 shell32 函式在執行期取得；取不到時只是不自動更新（仍可按 F5），程式照常啟動。

use super::app::{self, App, NavKind, WorkerMsg};
use super::fs::{self, FsError, Listing};
use super::sys::*;
use super::util::*;
use feather_core::paths;
use feather_core::sorting;
use feather_core::watch::{self, Effect, Schedule};
use feather_core::workspace::{Side, TabId};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// 殼層送來的變更通知
pub const WM_APP_WATCH: u32 = WM_APP + 6;
/// 到了預定重新讀取的時間
pub const TIMER_WATCH: usize = 14;

const SHCNRF_INTERRUPT_LEVEL: i32 = 0x0001;
const SHCNRF_SHELL_LEVEL: i32 = 0x0002;
const SHCNRF_NEW_DELIVERY: i32 = 0x8000;

const SHCNE_RENAMEITEM: u32 = 0x0000_0001;
const SHCNE_CREATE: u32 = 0x0000_0002;
const SHCNE_DELETE: u32 = 0x0000_0004;
const SHCNE_MKDIR: u32 = 0x0000_0008;
const SHCNE_RMDIR: u32 = 0x0000_0010;
const SHCNE_ATTRIBUTES: u32 = 0x0000_0800;
const SHCNE_UPDATEDIR: u32 = 0x0000_1000;
const SHCNE_UPDATEITEM: u32 = 0x0000_2000;
const SHCNE_RENAMEFOLDER: u32 = 0x0002_0000;
/// 中斷層級（直接來自檔案系統）的通知會帶這個旗標
const SHCNE_INTERRUPT: u32 = 0x8000_0000;
const EVENTS: u32 =
    SHCNE_RENAMEITEM | SHCNE_CREATE | SHCNE_DELETE | SHCNE_MKDIR | SHCNE_RMDIR | SHCNE_ATTRIBUTES | SHCNE_UPDATEDIR | SHCNE_UPDATEITEM | SHCNE_RENAMEFOLDER;

/// 延伸路徑最長 32,767 個字元
const PATH_CAPACITY: usize = 32_768;

#[repr(C)]
struct ShChangeNotifyEntry {
    pidl: *const c_void,
    recursive: BOOL,
}

/// SHChangeNotifyRegister(hwnd, fSources, fEvents, wMsg, cEntries, pshcne)
type RegisterFn = unsafe extern "system" fn(HWND, i32, i32, u32, i32, *const ShChangeNotifyEntry) -> u32;
/// SHChangeNotifyDeregister(ulID)
type DeregisterFn = unsafe extern "system" fn(u32) -> BOOL;
/// SHChangeNotification_Lock(hChange, dwProcId, pppidl, plEvent)
type LockFn = unsafe extern "system" fn(HANDLE, u32, *mut *mut *mut c_void, *mut i32) -> HANDLE;
/// SHChangeNotification_Unlock(hLock)
type UnlockFn = unsafe extern "system" fn(HANDLE) -> BOOL;
/// SHGetPathFromIDListEx(pidl, pszPath, cchPath, uOpts)
type PathFromPidlFn = unsafe extern "system" fn(*const c_void, *mut u16, u32, i32) -> BOOL;

struct Api {
    register: RegisterFn,
    deregister: DeregisterFn,
    lock: LockFn,
    unlock: UnlockFn,
    path_from_pidl: PathFromPidlFn,
}

static API: OnceLock<Option<Api>> = OnceLock::new();

fn api() -> Option<&'static Api> {
    API.get_or_init(|| unsafe { load_api() }).as_ref()
}

unsafe fn load_api() -> Option<Api> {
    let name = wide("shell32.dll");
    let module = LoadLibraryW(name.as_ptr());
    if module == 0 {
        return None;
    }
    // 序數是舊版 Windows 只以序數匯出時的備援
    let register = proc_address(module, "SHChangeNotifyRegister", 2);
    let deregister = proc_address(module, "SHChangeNotifyDeregister", 4);
    let lock = proc_address(module, "SHChangeNotification_Lock", 644);
    let unlock = proc_address(module, "SHChangeNotification_Unlock", 645);
    let path_from_pidl = proc_address(module, "SHGetPathFromIDListEx", 0);
    if [register, deregister, lock, unlock, path_from_pidl].iter().any(|p| p.is_null()) {
        super::crashlog::crumb("自動重新整理：shell32 缺少變更通知函式，停用");
        return None;
    }
    Some(Api {
        register: std::mem::transmute::<*const c_void, RegisterFn>(register),
        deregister: std::mem::transmute::<*const c_void, DeregisterFn>(deregister),
        lock: std::mem::transmute::<*const c_void, LockFn>(lock),
        unlock: std::mem::transmute::<*const c_void, UnlockFn>(unlock),
        path_from_pidl: std::mem::transmute::<*const c_void, PathFromPidlFn>(path_from_pidl),
    })
}

/// 一個註冊中的資料夾。註冊失敗時 id 是 0，保留這一筆以免每次更新畫面都重試。
struct Reg {
    path: String,
    id: u32,
    pidl: *mut c_void,
}

/// 一個分頁的自動重新讀取狀態
#[derive(Default)]
struct TabAuto {
    sched: Schedule,
    /// 背景讀取開始時分頁的世代（之後使用者自己導覽、搜尋，世代就不同，那次讀取作廢）
    generation: u64,
}

pub(super) struct Watches {
    regs: Vec<Reg>,
    tabs: HashMap<TabId, TabAuto>,
    epoch: Instant,
}

impl Default for Watches {
    fn default() -> Self {
        Watches { regs: Vec::new(), tabs: HashMap::new(), epoch: Instant::now() }
    }
}

impl Watches {
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    fn auto(&mut self, id: TabId) -> &mut TabAuto {
        self.tabs.entry(id).or_default()
    }
}

fn register(hwnd: HWND, path: &str) -> Reg {
    let mut reg = Reg { path: path.to_string(), id: 0, pidl: std::ptr::null_mut() };
    let Some(api) = api() else { return reg };
    let w = wide(path);
    let mut pidl: *mut c_void = std::ptr::null_mut();
    unsafe {
        if SHParseDisplayName(w.as_ptr(), std::ptr::null_mut(), &mut pidl, 0, std::ptr::null_mut()) < 0 || pidl.is_null() {
            super::crashlog::crumb("自動重新整理：無法取得資料夾的項目識別碼，這個資料夾不自動更新");
            return reg;
        }
        let entry = ShChangeNotifyEntry { pidl, recursive: 0 };
        let id = (api.register)(hwnd, SHCNRF_INTERRUPT_LEVEL | SHCNRF_SHELL_LEVEL | SHCNRF_NEW_DELIVERY, EVENTS as i32, WM_APP_WATCH, 1, &entry);
        if id == 0 {
            CoTaskMemFree(pidl);
            super::crashlog::crumb("自動重新整理：註冊變更通知失敗，這個資料夾不自動更新");
            return reg;
        }
        reg.id = id;
        reg.pidl = pidl;
    }
    reg
}

fn unregister(reg: Reg) {
    unsafe {
        if reg.id != 0 {
            if let Some(api) = api() {
                (api.deregister)(reg.id);
            }
        }
        if !reg.pidl.is_null() {
            CoTaskMemFree(reg.pidl);
        }
    }
}

unsafe fn pidl_path(api: &Api, pidl: *mut c_void) -> Option<String> {
    if pidl.is_null() {
        return None;
    }
    let mut buf = vec![0u16; PATH_CAPACITY];
    if (api.path_from_pidl)(pidl, buf.as_mut_ptr(), buf.len() as u32, 0) == 0 {
        return None;
    }
    let s = from_wide(&buf);
    (!s.is_empty()).then_some(s)
}

/// 拖放、選單、框選、拖分隔線或移動視窗進行中：先不要換清單內容。
fn ui_busy() -> bool {
    super::dragdrop::drag_active() || super::update::gui_busy()
}

impl App {
    /// 畫面上顯示的分頁（不含搜尋結果與還沒讀到資料夾的分頁）與它們的資料夾。
    fn watched_tabs(&self) -> Vec<(Side, TabId, String)> {
        let mut out = Vec::new();
        for &side in self.ws.visible_sides() {
            let Some(id) = self.ws.current(side) else { continue };
            if self.bound[side.index()] != Some(id) {
                continue;
            }
            let Some(t) = self.tabs.get(&id) else { continue };
            if t.search_mode || t.path.is_empty() {
                continue;
            }
            out.push((side, id, t.path.clone()));
        }
        out
    }

    /// 依畫面上顯示的資料夾增減註冊。畫面狀態變動後呼叫（update_chrome）；沒有變化時只比對字串。
    pub(super) fn sync_watches(&mut self) {
        // 視窗已經關閉（選擇器按下確定後，按鍵處理還會再更新一次畫面）：不再註冊
        if self.hwnd == 0 || unsafe { IsWindow(self.hwnd) } == 0 {
            return;
        }
        let mut wanted: Vec<String> = Vec::new();
        for (_, _, p) in self.watched_tabs() {
            if !wanted.iter().any(|w| paths::same_path(w, &p)) {
                wanted.push(p);
            }
        }
        let mut i = 0;
        while i < self.watch.regs.len() {
            if wanted.iter().any(|w| paths::same_path(w, &self.watch.regs[i].path)) {
                i += 1;
            } else {
                unregister(self.watch.regs.remove(i));
            }
        }
        for w in wanted {
            if !self.watch.regs.iter().any(|r| paths::same_path(&r.path, &w)) {
                let reg = register(self.hwnd, &w);
                self.watch.regs.push(reg);
            }
        }
    }

    /// 主視窗關閉：取消所有註冊。
    pub(super) fn stop_watching(&mut self) {
        for reg in self.watch.regs.drain(..) {
            unregister(reg);
        }
        self.watch.tabs.clear();
        unsafe {
            KillTimer(self.hwnd, TIMER_WATCH);
        }
    }

    /// 殼層送來的變更通知（SHCNRF_NewDelivery：wParam 是通知的代號，lParam 是送出的程序）。
    pub(super) fn on_watch_message(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let Some(api) = api() else { return };
        let mut pidls: *mut *mut c_void = std::ptr::null_mut();
        let mut event: i32 = 0;
        let (first, second) = unsafe {
            let lock = (api.lock)(wparam as HANDLE, lparam as u32, &mut pidls, &mut event);
            if lock == 0 {
                return;
            }
            let paths = if pidls.is_null() { (None, None) } else { (pidl_path(api, *pidls), pidl_path(api, *pidls.add(1))) };
            (api.unlock)(lock);
            paths
        };
        self.on_folder_event(event as u32 & !SHCNE_INTERRUPT, first.as_deref(), second.as_deref());
    }

    fn on_folder_event(&mut self, event: u32, first: Option<&str>, second: Option<&str>) {
        let renamed = event & SHCNE_RENAMEFOLDER != 0;
        let folders: Vec<String> = self.watch.regs.iter().filter(|r| r.id != 0).map(|r| r.path.clone()).collect();
        let mut hit: Vec<String> = Vec::new();
        let mut moved: Vec<(String, String)> = Vec::new();
        for f in &folders {
            match watch::effect(f, renamed, first, second) {
                Some(Effect::Moved(to)) => moved.push((f.clone(), to)),
                Some(Effect::Refresh) => hit.push(f.clone()),
                None => {}
            }
        }
        if hit.is_empty() && moved.is_empty() {
            // 看不出是哪一個資料夾（例如通知裡是虛擬位置）：顯示中的資料夾都重新讀一次，內容沒變就不會動到畫面
            hit = folders;
        }
        let mut followed = false;
        for (from, to) in moved {
            // 以檔案系統的現況為準：確定舊的位置已經不在、新的位置存在才跟過去
            if !fs::is_dir(&from) && fs::is_dir(&to) {
                self.follow_moved_folder(&from, &to);
                followed = true;
                hit.push(to);
            } else {
                hit.push(from);
            }
        }
        let now = self.watch.now();
        for (_, id, path) in self.watched_tabs() {
            if hit.iter().any(|h| paths::same_path(h, &path)) {
                self.watch.auto(id).sched.changed(now);
            }
        }
        if followed {
            self.update_chrome();
        }
        self.arm_watch_timer();
    }

    /// 顯示中的資料夾（或它的上層）改名、搬走了：顯示它（以及它底下）的分頁改用新的路徑。
    fn follow_moved_folder(&mut self, from: &str, to: &str) {
        let active = self.active_tab_id();
        for (id, t) in self.tabs.iter_mut() {
            if t.search_mode || t.path.is_empty() {
                continue;
            }
            if let Some(p) = watch::rebase(&t.path, from, to) {
                // 清單上的項目也改成新的位置，重新讀取回來之前開啟、拖曳都用對的路徑
                let dir: Arc<str> = Arc::from(p.as_str());
                for e in t.entries.iter_mut() {
                    e.dir = dir.clone();
                }
                t.path = p.clone();
                t.requested = p;
                if *id == active {
                    set_window_text(self.address, &t.path);
                }
            }
        }
        super::crashlog::crumb("自動重新整理：顯示中的資料夾改名或搬移，改顯示新位置");
    }

    /// 分頁重新顯示（切換分頁、雙欄重新顯示右欄）：隱藏期間的變更沒有收到通知，稍後補讀一次。
    pub(super) fn watch_catch_up(&mut self, id: TabId) {
        let Some(t) = self.tabs.get(&id) else { return };
        if t.working() || t.search_mode || t.path.is_empty() || t.stale {
            return;
        }
        let now = self.watch.now();
        self.watch.auto(id).sched.catch_up(now);
        self.arm_watch_timer();
    }

    fn arm_watch_timer(&mut self) {
        if self.hwnd == 0 || unsafe { IsWindow(self.hwnd) } == 0 {
            return;
        }
        let now = self.watch.now();
        unsafe {
            match watch::next_due(self.watch.tabs.values().map(|a| &a.sched)) {
                Some(due) => {
                    SetTimer(self.hwnd, TIMER_WATCH, due.saturating_sub(now).clamp(10, 60_000) as u32, std::ptr::null());
                }
                None => {
                    KillTimer(self.hwnd, TIMER_WATCH);
                }
            }
        }
    }

    /// 到了預定的時間：能更新的分頁開始在背景重新讀取，不方便的稍後再試。
    pub(super) fn on_watch_timer(&mut self) {
        unsafe {
            KillTimer(self.hwnd, TIMER_WATCH);
        }
        let now = self.watch.now();
        self.watch.tabs.retain(|id, _| self.tabs.contains_key(id));
        let shown = self.watched_tabs();
        let busy = ui_busy();
        let due: Vec<TabId> = self.watch.tabs.iter().filter(|(_, a)| a.sched.is_due(now)).map(|(id, _)| *id).collect();
        for id in due {
            let Some((side, _, _)) = shown.iter().find(|(_, t, _)| *t == id) else {
                // 已經不在畫面上（切到別的分頁、改成搜尋結果）：切回來時再補讀
                self.watch.auto(id).sched.cancel();
                continue;
            };
            let Some(tab) = self.tabs.get(&id) else { continue };
            let generation = tab.generation;
            let working = tab.working();
            let editing = send(self.lists[side.index()], LVM_GETEDITCONTROL, 0, 0) != 0;
            let auto = self.watch.auto(id);
            let reading = auto.sched.reading && auto.generation == generation;
            if !reading {
                auto.sched.finish();
            }
            if reading || working || editing || busy {
                auto.sched.postpone(now);
                continue;
            }
            auto.sched.start(now);
            match self.start_auto_read(id) {
                Some(g) => self.watch.auto(id).generation = g,
                None => self.watch.auto(id).sched.finish(),
            }
        }
        self.arm_watch_timer();
    }

    /// 在背景重新讀取分頁目前的資料夾。不顯示「正在讀取」、不擋住其他操作；
    /// 使用者在讀完前自己導覽、搜尋或按 F5 時，這次結果會因世代不同而被忽略。
    fn start_auto_read(&mut self, id: TabId) -> Option<u64> {
        let show_hidden = self.show_hidden;
        let folders_on_top = self.settings.folders_on_top;
        let tab = self.tabs.get_mut(&id)?;
        let (generation, cancel) = tab.begin_work();
        let (sort_col, desc) = (tab.sort_col, tab.desc);
        let path = tab.path.clone();
        std::thread::spawn(move || {
            let result = fs::read_dir(&path, show_hidden, sort_col, desc, folders_on_top, &cancel);
            // 讀不到時順便確認資料夾還在不在：網路位置斷線時這要等很久，不能在介面執行緒上做
            let gone_to = match &result {
                Err(FsError::Message(_)) if !fs::is_dir(&path) => watch::nearest_existing(&path, fs::is_dir),
                _ => None,
            };
            app::post(WorkerMsg::AutoDir { tab: id, generation, path, result, gone_to });
        });
        Some(generation)
    }

    /// 自動重新讀取的結果：內容有變才更新清單（保留選取、焦點與捲動位置）。
    pub(super) fn on_auto_dir(&mut self, id: TabId, generation: u64, path: String, result: Result<Listing, FsError>, gone_to: Option<String>) {
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.generation != generation {
            return;
        }
        let (sort_col, desc) = (tab.sort_col, tab.desc);
        let same_folder = !tab.search_mode && paths::same_path(&tab.path, &path);
        self.watch.auto(id).sched.finish();
        if !same_folder {
            return;
        }
        let mut listing = match result {
            Ok(l) => l,
            Err(FsError::Cancelled) => return,
            Err(FsError::Message(_)) => {
                if let Some(target) = gone_to {
                    self.folder_gone(id, &path, &target);
                }
                return;
            }
        };
        // 讀取期間換了排序方式：依目前的排序排好再比較
        sorting::sort_entries(&mut listing.entries, sort_col, desc, self.settings.folders_on_top);
        let Some(tab) = self.tabs.get(&id) else { return };
        let same_dir = tab.entries.first().is_none_or(|e| &*e.dir == path.as_str());
        if same_dir && watch::same_entries(&tab.entries, &listing.entries) {
            return;
        }
        // 讀取期間分頁被切走了：先不換（它記住的選取是列號），切回來時會再補讀一次
        let Some(side) = self.ws.side_of(id).filter(|s| self.bound[s.index()] == Some(id) && self.ws.visible_sides().contains(s)) else { return };
        // 讀取期間使用者開始重新命名、拖放或打開選單：換掉清單會打斷操作，等一下再重讀
        if send(self.lists[side.index()], LVM_GETEDITCONTROL, 0, 0) != 0 || ui_busy() {
            let now = self.watch.now();
            self.watch.auto(id).sched.postpone(now);
            self.arm_watch_timer();
            return;
        }
        let kept = self.keep_view(side);
        let folders: Vec<(String, bool)> = listing.entries.iter().filter(|e| e.is_dir).map(|e| (e.name.clone(), e.is_link)).collect();
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        tab.status = app::folder_status(listing.entries.len(), listing.skipped);
        tab.entries = listing.entries;
        tab.refilter();
        tab.resummarize();
        self.restore_view(side, kept);
        self.sync_tree_folder(&path, &folders);
        self.update_chrome();
    }

    /// 自動重新讀取時發現資料夾已經不在（被刪除、搬走）：改顯示還在的上一層。
    /// 只是暫時讀不到（例如網路斷線）時背景執行緒不會給 `target`，保留畫面，不跳到別的地方。
    fn folder_gone(&mut self, id: TabId, path: &str, target: &str) {
        super::crashlog::crumb("自動重新整理：顯示中的資料夾已不在，改顯示上一層");
        if id == self.active_tab_id() {
            self.set_footer(trf!("「{}」已被移走或刪除，改為顯示「{}」", "“{}” was moved or deleted, so “{}” is shown instead", paths::tab_title(path), paths::tab_title(target)));
        }
        self.navigate(id, target, NavKind::NoHistory, vec![]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_entry_layout() {
        // x64：PCIDLIST_ABSOLUTE（8）＋ BOOL（4）＋ 補齊（4）
        if std::mem::size_of::<usize>() != 8 {
            return;
        }
        assert_eq!(std::mem::size_of::<ShChangeNotifyEntry>(), 16);
    }

    #[test]
    fn interrupt_flag_is_masked() {
        let event = (SHCNE_CREATE | SHCNE_INTERRUPT) as i32;
        assert_eq!(event as u32 & !SHCNE_INTERRUPT, SHCNE_CREATE);
        assert_eq!(EVENTS & SHCNE_INTERRUPT, 0);
    }
}

//! 每個檔案各自的圖示（捷徑、程式、圖示檔…，哪些類型與快取的鍵見 `feather_core::fileicons`）。
//!
//! - 清單要畫某一列時（LVN_GETDISPINFO，只有畫面上看得到的列）：已經取到就用它，還沒有就先顯示副檔名的
//!   圖示，並把這個檔案排進背景佇列。和縮圖（thumbs.rs）一樣是有上限的工作執行緒（最多兩個）、
//!   最後要求的先做（捲動後新出現的列優先）、換資料夾或分頁時丟掉還沒做的要求、收到結果後合併重畫。
//! - 工作執行緒用檔案的完整路徑呼叫 SHGetFileInfo（Windows 建議在背景執行緒呼叫）：捷徑會依它指定的
//!   圖示或目標程式的圖示，捷徑類加上 SHGFI_LINKOVERLAY 保留捷徑箭頭。捷徑指向離線的網路位置時，
//!   等待的只是那個工作執行緒，介面照常。
//! - 結果回到主執行緒後加進清單的影像清單（`Icons::add_file_icon`），以檔案為鍵記住索引。清單是虛擬清單，
//!   每次畫都會重新問圖示：重畫時依檔案找索引，排序、捲動或換了資料夾都不會畫到別的列上。

use super::icons::Icons;
use super::sys::*;
use feather_core::entry::Entry;
use feather_core::fileicons;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

/// 工作執行緒取到一個圖示（lParam 是 `Box<Done>`）
pub const WM_APP_FILE_ICON: u32 = WM_APP + 17;
/// 收到圖示後合併重畫清單
pub const TIMER_REDRAW: usize = 17;
const REDRAW_MS: u32 = 60;
/// 同時最多幾個工作執行緒
const WORKERS: usize = 2;
/// 取不到圖示的記錄太多時整個清掉（之後再遇到會重試一次）
const FAILED_MAX: usize = 20_000;

struct Request {
    key: String,
    path: String,
    shortcut: bool,
}

/// 一個要求的結果：圖示（0 表示取不到；收到的一方負責 DestroyIcon）
pub struct Done {
    key: String,
    ext: String,
    icon: HICON,
}

#[derive(Default)]
struct Queue {
    items: VecDeque<(Request, String)>,
    started: usize,
    busy: usize,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

#[derive(Default)]
pub struct FileIcons {
    /// 已經排進佇列或正在取的
    pending: HashSet<String>,
    /// 取不到的（同一個檔案不再試）
    failed: HashSet<String>,
    shared: Arc<Shared>,
    redraw_armed: bool,
}

impl FileIcons {
    /// 清單裡這個項目的圖示索引：自己的圖示已經取到就用它；還沒有的話先給副檔名的圖示，並在背景取。
    pub fn index(&mut self, icons: &mut Icons, owner: HWND, e: &Entry) -> i32 {
        if fileicons::wants(e) {
            let key = fileicons::key(e);
            if let Some(i) = icons.file_icon(&key) {
                return i;
            }
            if icons.file_icon_count() < fileicons::MAX_FILE_ICONS && !self.pending.contains(&key) && !self.failed.contains(&key) {
                let ext = e.extension();
                let shortcut = fileicons::is_shortcut(&ext);
                self.pending.insert(key.clone());
                self.push(owner, Request { key, path: e.full_path(), shortcut }, ext);
            }
        }
        icons.index_for(e)
    }

    fn push(&mut self, owner: HWND, req: Request, ext: String) {
        let Ok(mut q) = self.shared.queue.lock() else { return };
        q.items.push_back((req, ext));
        let idle = q.started - q.busy;
        if q.started < WORKERS && q.items.len() > idle {
            q.started += 1;
            let shared = self.shared.clone();
            std::thread::spawn(move || worker_thread(shared, owner));
        }
        drop(q);
        self.shared.ready.notify_one();
    }

    /// 換資料夾、分頁或開始搜尋：丟掉還沒開始取的（正在取的照樣收下）
    pub fn forget_queue(&mut self) {
        let Ok(mut q) = self.shared.queue.lock() else { return };
        for (r, _) in q.items.drain(..) {
            self.pending.remove(&r.key);
        }
    }

    /// 工作執行緒取到一個圖示：加進影像清單，稍後合併重畫清單
    pub fn on_result(&mut self, owner: HWND, done: Done, icons: &mut Icons) {
        self.pending.remove(&done.key);
        let added = done.icon != 0 && icons.add_file_icon(&done.key, &done.ext, done.icon).is_some();
        if done.icon != 0 {
            unsafe {
                DestroyIcon(done.icon);
            }
        }
        if !added {
            if self.failed.len() >= FAILED_MAX {
                self.failed.clear();
            }
            self.failed.insert(done.key);
            return;
        }
        if !self.redraw_armed {
            self.redraw_armed = true;
            unsafe {
                SetTimer(owner, TIMER_REDRAW, REDRAW_MS, std::ptr::null());
            }
        }
    }

    /// 合併重畫的計時到了（之後由呼叫的人重畫清單）
    pub fn redraw_due(&mut self, owner: HWND) {
        self.redraw_armed = false;
        unsafe {
            KillTimer(owner, TIMER_REDRAW);
        }
    }
}

fn worker_thread(shared: Arc<Shared>, owner: HWND) {
    // 捷徑、網址捷徑等的圖示處理程式是 COM 物件
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    loop {
        let (req, ext) = {
            let Ok(mut q) = shared.queue.lock() else { return };
            loop {
                // 最後要求的先做：捲動後新出現的列優先
                if let Some(r) = q.items.pop_back() {
                    q.busy += 1;
                    break r;
                }
                let Ok(g) = shared.ready.wait(q) else { return };
                q = g;
            }
        };
        let icon = unsafe { load(&req) };
        if let Ok(mut q) = shared.queue.lock() {
            q.busy -= 1;
        }
        let done = Box::into_raw(Box::new(Done { key: req.key, ext, icon }));
        if unsafe { PostMessageW(owner, WM_APP_FILE_ICON, 0, done as LPARAM) } == 0 {
            // 主視窗已經關了
            let d = unsafe { Box::from_raw(done) };
            if d.icon != 0 {
                unsafe {
                    DestroyIcon(d.icon);
                }
            }
        }
    }
}

/// 用完整路徑向 Windows 取這個檔案的小圖示（捷徑類加上捷徑箭頭）。取不到時回傳 0。
unsafe fn load(req: &Request) -> HICON {
    // 殼層的函式不接受 \\?\ 開頭的延伸路徑：用原本的路徑（太長取不到時就維持副檔名的圖示）
    let path = super::util::wide(&req.path);
    let mut info: SHFILEINFOW = std::mem::zeroed();
    let flags = SHGFI_ICON | SHGFI_SMALLICON | if req.shortcut { SHGFI_LINKOVERLAY } else { 0 };
    if SHGetFileInfoW(path.as_ptr(), 0, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, flags) == 0 {
        return 0;
    }
    info.hIcon
}

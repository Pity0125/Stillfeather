//! 檔案清單的縮圖（哪些檔案、相片資料夾的判斷與傳輸格式見 `feather_core::thumbs`）。
//!
//! - 縮圖在子程序產生（本程式以 `--thumb-worker` 執行）：第三方的縮圖處理程式（影片解碼器、PDF、RAW…）
//!   在裡面當掉或卡住時，只有子程序結束，Stillfeather 不受影響。和快速預覽（每個檔案一個短命的子程序）不同，
//!   這裡的子程序一次處理很多個，資料夾裡幾百張相片也夠快；最多同時兩個，閒置一分鐘就結束。
//!   一個縮圖超過 15 秒沒回來就結束那個子程序（下一個要求再開新的），那個檔案改顯示圖示。
//! - 子程序用 Windows Shell 的縮圖介面（`IShellItemImageFactory`，只取縮圖），會用到系統的縮圖快取；
//!   雲端尚未下載的檔案只取快取裡已有的，不觸發下載。子程序以較低的優先順序執行，不搶介面的 CPU。
//! - 主程式：畫清單時才要求畫面上的項目（最後要求的先做，捲動後新出現的項目優先），結果放進快取（約 128 MiB），
//!   收到結果後合併重畫。換資料夾、分頁或檢視時丟掉還沒做的要求。
//! - 有縮圖的項目，清單的圖示改成透明的那一格（`Icons::blank_index`），縮圖在自繪的最後畫在圖示的位置。

use super::sys::*;
use super::util::*;
use feather_core::entry::Entry;
use feather_core::thumbs::{self, Cache, ExtStats, HEADER_LEN};
use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

/// 工作執行緒做完一個要求（lParam 是 `Box<Done>`）
pub const WM_APP_THUMB: u32 = WM_APP + 13;
/// 收到結果後合併重畫
pub const TIMER_REDRAW: usize = 16;
const REDRAW_MS: u32 = 60;
/// 同時最多幾個工作程序
const WORKERS: usize = 2;
/// 一個縮圖最多等多久
const TIMEOUT: Duration = Duration::from_secs(15);
/// 閒置多久就結束工作程序
const IDLE: Duration = Duration::from_secs(60);
const CACHE_BYTES: usize = 128 * 1024 * 1024;
/// 每張縮圖是一個 GDI 物件，遠低於每個程式一萬個的上限
const CACHE_ITEMS: usize = 1500;
/// 取不到縮圖的記錄太多時整個清掉（之後再遇到會重試一次）
const FAILED_MAX: usize = 20_000;

type Key = (String, i32);

/// 主程式裡的一張縮圖（DIB）；最後一個參照放掉時刪除
pub struct Bitmap {
    hbm: HBITMAP,
    w: i32,
    h: i32,
    /// 有透明的部分（像素已預乘，用 AlphaBlend 畫）
    alpha: bool,
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.hbm);
        }
    }
}

struct Request {
    key: Key,
    path: String,
    edge: i32,
    ext: String,
}

/// 一個要求的結果：寬、高與 BGRA 像素；取不到縮圖時是 None
pub struct Done {
    key: Key,
    ext: String,
    pixels: Option<(i32, i32, Vec<u32>)>,
}

#[derive(Default)]
struct Queue {
    items: VecDeque<Request>,
    /// 已經開的工作執行緒數、正在處理要求的數量
    started: usize,
    busy: usize,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

pub struct Thumbs {
    cache: Cache<Key, Rc<Bitmap>>,
    /// 已經排進佇列或正在做的
    pending: HashSet<Key>,
    /// 取不到縮圖的（同一個檔案、同一個大小不再試）
    failed: HashSet<Key>,
    exts: ExtStats,
    shared: Arc<Shared>,
    redraw_armed: bool,
}

impl Default for Thumbs {
    fn default() -> Self {
        Thumbs {
            cache: Cache::new(CACHE_BYTES, CACHE_ITEMS),
            pending: HashSet::new(),
            failed: HashSet::new(),
            exts: ExtStats::default(),
            shared: Arc::new(Shared::default()),
            redraw_armed: false,
        }
    }
}

impl Thumbs {
    /// 快取裡有這個項目的縮圖（清單取圖示時用：有縮圖就給透明的那一格）
    pub fn has(&self, e: &Entry, edge: i32) -> bool {
        thumbs::wants(e) && self.cache.contains(&thumbs::key(e, edge))
    }

    /// 這個項目的縮圖；還沒有而且值得取的話排進佇列，結果回來後清單會重畫
    pub fn get_or_request(&mut self, owner: HWND, e: &Entry, edge: i32) -> Option<Rc<Bitmap>> {
        if !thumbs::wants(e) {
            return None;
        }
        let key = thumbs::key(e, edge);
        if let Some(b) = self.cache.get(&key) {
            return Some(b.clone());
        }
        if self.pending.contains(&key) || self.failed.contains(&key) {
            return None;
        }
        let ext = e.extension();
        if self.exts.given_up(&ext) {
            return None;
        }
        self.pending.insert(key.clone());
        self.push(owner, Request { key, path: e.full_path(), edge, ext });
        None
    }

    /// 只看快取，不要求（自繪最後畫縮圖時用）
    pub fn cached(&mut self, e: &Entry, edge: i32) -> Option<Rc<Bitmap>> {
        if !thumbs::wants(e) {
            return None;
        }
        self.cache.get(&thumbs::key(e, edge)).cloned()
    }

    fn push(&mut self, owner: HWND, req: Request) {
        let Ok(mut q) = self.shared.queue.lock() else { return };
        q.items.push_back(req);
        // 有事做、而且沒有閒著的工作執行緒時再開一個（最多兩個）
        let idle = q.started - q.busy;
        if q.started < WORKERS && q.items.len() > idle {
            q.started += 1;
            let shared = self.shared.clone();
            std::thread::spawn(move || worker_thread(shared, owner));
        }
        drop(q);
        self.shared.ready.notify_one();
    }

    /// 換資料夾、分頁或檢視：丟掉還沒開始做的要求（正在做的照樣收下）
    pub fn forget_queue(&mut self) {
        let Ok(mut q) = self.shared.queue.lock() else { return };
        for r in q.items.drain(..) {
            self.pending.remove(&r.key);
        }
    }

    /// 工作執行緒做完一個要求：放進快取，稍後合併重畫
    pub fn on_result(&mut self, owner: HWND, done: Done) {
        self.pending.remove(&done.key);
        match done.pixels {
            Some((w, h, mut px)) if w > 0 && h > 0 && px.len() == (w * h) as usize => {
                self.exts.record(&done.ext, true);
                let alpha = thumbs::prepare_pixels(&mut px);
                match unsafe { make_dib(w, h, &px) } {
                    Some(hbm) => self.cache.put(done.key, Rc::new(Bitmap { hbm, w, h, alpha }), px.len() * 4),
                    None => {
                        self.failed.insert(done.key);
                    }
                }
            }
            _ => {
                self.exts.record(&done.ext, false);
                if self.failed.len() >= FAILED_MAX {
                    self.failed.clear();
                }
                self.failed.insert(done.key);
            }
        }
        if !self.redraw_armed {
            self.redraw_armed = true;
            unsafe {
                SetTimer(owner, TIMER_REDRAW, REDRAW_MS, std::ptr::null());
            }
        }
    }

    /// 合併重畫的計時到了（之後由呼叫的人重畫顯示縮圖的清單）
    pub fn redraw_due(&mut self, owner: HWND) {
        self.redraw_armed = false;
        unsafe {
            KillTimer(owner, TIMER_REDRAW);
        }
    }
}

unsafe fn make_dib(w: i32, h: i32, px: &[u32]) -> Option<HBITMAP> {
    let bi = BITMAPINFO::top_down_32(w, h);
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let hdc = GetDC(0);
    let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
    ReleaseDC(0, hdc);
    if hbm == 0 || bits.is_null() {
        if hbm != 0 {
            DeleteObject(hbm);
        }
        return None;
    }
    std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
    Some(hbm)
}

/// 在清單項目的圖示位置畫縮圖：`icon` 是圖示的矩形（LVIR_ICON），縮圖放進其中邊長 `edge` 的方格；
/// `bottom` 時貼齊方格底部（圖示檢視，靠近名稱），否則垂直置中（並排、內容檢視）。
pub unsafe fn draw(hdc: HDC, bmp: &Bitmap, icon: RECT, edge: i32, bottom: bool) {
    let bx = icon.left + (icon.width() - edge) / 2;
    let by = icon.top + (icon.height() - edge) / 2;
    let (x, y, w, h) = thumbs::place(bmp.w, bmp.h, edge, bottom);
    if w <= 0 || h <= 0 {
        return;
    }
    let mem = CreateCompatibleDC(hdc);
    if mem == 0 {
        return;
    }
    let old = SelectObject(mem, bmp.hbm);
    if bmp.alpha {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA };
        GdiAlphaBlend(hdc, bx + x, by + y, w, h, mem, 0, 0, bmp.w, bmp.h, blend);
    } else if (w, h) == (bmp.w, bmp.h) {
        BitBlt(hdc, bx + x, by + y, w, h, mem, 0, 0, SRCCOPY);
    } else {
        let mode = SetStretchBltMode(hdc, HALFTONE);
        SetBrushOrgEx(hdc, 0, 0, std::ptr::null_mut());
        StretchBlt(hdc, bx + x, by + y, w, h, mem, 0, 0, bmp.w, bmp.h, SRCCOPY);
        SetStretchBltMode(hdc, mode);
    }
    SelectObject(mem, old);
    DeleteDC(mem);
}

// ---------------------------------------------------------------- 工作執行緒（主程式裡，管理子程序）

fn worker_thread(shared: Arc<Shared>, owner: HWND) {
    let mut child: Option<WorkerChild> = None;
    let mut next_id = 0u64;
    loop {
        let req = {
            let Ok(mut q) = shared.queue.lock() else { return };
            loop {
                if let Some(r) = q.items.pop_back() {
                    q.busy += 1;
                    break r;
                }
                let Ok((g, wait)) = shared.ready.wait_timeout(q, IDLE) else { return };
                q = g;
                if wait.timed_out() && q.items.is_empty() {
                    // 閒置太久：結束子程序（下一個要求再開）
                    child = None;
                }
            }
        };
        let pixels = ask(&mut child, &mut next_id, &req);
        if let Ok(mut q) = shared.queue.lock() {
            q.busy -= 1;
        }
        let done = Box::into_raw(Box::new(Done { key: req.key, ext: req.ext, pixels }));
        if unsafe { PostMessageW(owner, WM_APP_THUMB, 0, done as LPARAM) } == 0 {
            // 主視窗已經關了
            drop(unsafe { Box::from_raw(done) });
        }
    }
}

/// 請子程序產生一張縮圖；子程序當掉、卡住或回傳的格式不對時結束它（下一個要求再開新的）
fn ask(child: &mut Option<WorkerChild>, next_id: &mut u64, req: &Request) -> Option<(i32, i32, Vec<u32>)> {
    if child.is_none() {
        *child = WorkerChild::start().ok();
    }
    let c = child.as_mut()?;
    *next_id += 1;
    match c.ask(*next_id, req) {
        Ok(result) => result,
        Err(()) => {
            *child = None;
            None
        }
    }
}

type Frame = (u64, u32, u32, Vec<u32>);

struct WorkerChild {
    process: Child,
    stdin: ChildStdin,
    frames: mpsc::Receiver<Frame>,
}

impl WorkerChild {
    fn start() -> std::io::Result<WorkerChild> {
        let exe = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        cmd.arg(thumbs::WORKER_ARG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut process = cmd.spawn()?;
        let (Some(stdin), Some(stdout)) = (process.stdin.take(), process.stdout.take()) else {
            let _ = process.kill();
            let _ = process.wait();
            return Err(std::io::Error::other("無法連接縮圖工作程序"));
        };
        let (tx, frames) = mpsc::channel();
        std::thread::spawn(move || read_frames(stdout, tx));
        Ok(WorkerChild { process, stdin, frames })
    }

    /// Ok(None)：沒有縮圖；Err：子程序不能再用
    fn ask(&mut self, id: u64, req: &Request) -> Result<Option<(i32, i32, Vec<u32>)>, ()> {
        let Some(line) = thumbs::encode_request(id, req.edge, &req.path) else { return Ok(None) };
        self.stdin.write_all(line.as_bytes()).and_then(|_| self.stdin.flush()).map_err(|_| ())?;
        loop {
            match self.frames.recv_timeout(TIMEOUT) {
                Ok((rid, w, h, px)) if rid == id => return Ok((w > 0 && h > 0).then_some((w as i32, h as i32, px))),
                // 之前逾時的要求晚到的回答
                Ok(_) => continue,
                Err(_) => return Err(()),
            }
        }
    }
}

impl Drop for WorkerChild {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

/// 讀子程序的回答；格式不對或子程序結束時停止（等待的一方會立刻知道，換新的子程序）
fn read_frames(out: std::process::ChildStdout, tx: mpsc::Sender<Frame>) {
    let mut out = std::io::BufReader::with_capacity(256 * 1024, out);
    loop {
        let mut header = [0u8; HEADER_LEN];
        if out.read_exact(&mut header).is_err() {
            return;
        }
        let Some((id, w, h)) = thumbs::parse_header(&header) else { return };
        let mut bytes = vec![0u8; (w as usize) * (h as usize) * 4];
        if out.read_exact(&mut bytes).is_err() {
            return;
        }
        let px = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        if tx.send((id, w, h, px)).is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------- 工作程序（--thumb-worker）

/// 子程序入口：從標準輸入一行一個要求，每個要求回答一次（標頭，加上像素）
pub fn worker_main() -> i32 {
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // 目前程序的虛擬代號（GetCurrentProcess 的回傳值）
        SetPriorityClass(-1, BELOW_NORMAL_PRIORITY_CLASS);
    }
    let stdin = std::io::stdin().lock();
    let mut out = std::io::stdout().lock();
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        let Some((id, edge, path)) = thumbs::parse_request(&line) else { continue };
        let image = unsafe { extract(path, edge) };
        let ok = match image {
            Some((w, h, px)) => {
                let mut buf = Vec::with_capacity(HEADER_LEN + px.len() * 4);
                buf.extend_from_slice(&thumbs::encode_header(id, w as u32, h as u32));
                for p in px {
                    buf.extend_from_slice(&p.to_le_bytes());
                }
                out.write_all(&buf)
            }
            None => out.write_all(&thumbs::encode_header(id, 0, 0)),
        };
        if ok.and_then(|_| out.flush()).is_err() {
            break;
        }
    }
    0
}

unsafe fn extract(path: &str, edge: i32) -> Option<(i32, i32, Vec<u32>)> {
    let attrs = GetFileAttributesW(wide(path).as_ptr());
    if attrs != INVALID_FILE_ATTRIBUTES && attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return None;
    }
    // 雲端尚未下載的檔案：只用系統快取裡已有的縮圖，不觸發下載
    let cloud = attrs != INVALID_FILE_ATTRIBUTES && attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0;
    let flags = SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY | if cloud { SIIGBF_INCACHEONLY } else { 0 };
    let (w, h, px) = super::preview::extract_with(path, edge, flags).ok()?;
    (w >= 1 && h >= 1 && w <= 2 * thumbs::MAX_EDGE && h <= 2 * thumbs::MAX_EDGE && px.len() == (w * h) as usize).then_some((w, h, px))
}

//! 當機診斷：程式被系統直接結束時（存取違規、堆疊用盡、堆積損壞——這些不會經過 panic 訊息），
//! 在記錄資料夾留下記錄（開發版在 exe 旁；安裝版在 %LOCALAPPDATA%\FeatherFiles\logs）：
//! - `trace.log`：經過的步驟，每一步立即寫入（程序被強制結束也留得住），最後一行就是出事的位置；
//!   嚴重的硬體例外也記在這裡。
//! - `crash.log`：未處理例外的代碼、位置與最後 64 個步驟。
//! - `stderr.log`：Rust 執行環境印到標準錯誤的訊息（例如「has overflowed its stack」）。
//!
//! 步驟紀錄（crumb）不配置記憶體，任何執行緒都可以呼叫。

use super::sys::*;
use std::sync::atomic::{AtomicIsize, AtomicPtr, AtomicUsize, Ordering};

const RING: usize = 64;
static CRUMB_PTR: [AtomicPtr<u8>; RING] = [const { AtomicPtr::new(std::ptr::null_mut()) }; RING];
static CRUMB_LEN: [AtomicUsize; RING] = [const { AtomicUsize::new(0) }; RING];
static NEXT: AtomicUsize = AtomicUsize::new(0);
/// crash.log 的完整路徑（UTF-16，結尾 0），啟動時決定
static LOG_PATH: AtomicPtr<u16> = AtomicPtr::new(std::ptr::null_mut());
/// trace.log 的檔案代號（以附加模式開啟，多執行緒寫入不會互相覆蓋）
static TRACE: AtomicIsize = AtomicIsize::new(0);
static TRACE_BYTES: AtomicUsize = AtomicUsize::new(0);
const TRACE_LIMIT: usize = 4 * 1024 * 1024;

/// 只記在記憶體裡的步驟（頻繁的，例如每次重畫）；當機時連同其他步驟寫進 crash.log。
pub fn mark(step: &'static str) {
    let i = NEXT.fetch_add(1, Ordering::Relaxed) % RING;
    CRUMB_LEN[i].store(step.len(), Ordering::Relaxed);
    CRUMB_PTR[i].store(step.as_ptr() as *mut u8, Ordering::Relaxed);
}

/// 記下經過的步驟，並立即寫進 trace.log。
pub fn crumb(step: &'static str) {
    mark(step);
    let mut b = Buf::<512>::new();
    b.time();
    b.put(b" [");
    b.dec(unsafe { GetCurrentThreadId() } as u64);
    b.put(b"] ");
    b.put(step.as_bytes());
    b.put(b"\r\n");
    trace_write(&b);
}

/// 記下帶數值的步驟（例如耗時毫秒、項目數），並立即寫進 trace.log。效能量測腳本（perf.ps1）讀這些行。
pub fn crumb_value(step: &'static str, value: u64) {
    mark(step);
    let mut b = Buf::<512>::new();
    b.time();
    b.put(b" [");
    b.dec(unsafe { GetCurrentThreadId() } as u64);
    b.put(b"] ");
    b.put(step.as_bytes());
    b.put(b" ");
    b.dec(value);
    b.put(b"\r\n");
    trace_write(&b);
}

fn trace_write<const N: usize>(b: &Buf<N>) {
    let h = TRACE.load(Ordering::Relaxed);
    if h == 0 || TRACE_BYTES.fetch_add(b.len, Ordering::Relaxed) > TRACE_LIMIT {
        return;
    }
    let mut written = 0u32;
    unsafe {
        WriteFile(h, b.data.as_ptr(), b.len as u32, &mut written, std::ptr::null_mut());
    }
}

fn exe_dir() -> Option<String> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(0, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..n]);
    feather_core::paths::parent(&path)
}

/// 記錄檔的資料夾：用安裝程式裝的（exe 旁有 Uninstall.exe）放在 %LOCALAPPDATA%\FeatherFiles\logs，
/// 安裝資料夾可能沒有寫入權限，解除安裝時也一併刪除；開發版放在 exe 旁。
pub fn log_dir() -> Option<String> {
    let dir = exe_dir()?;
    let installed = std::path::Path::new(&feather_core::paths::join(&dir, feather_core::install::UNINSTALLER_NAME)).exists();
    if !installed {
        return Some(dir);
    }
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let logs = feather_core::paths::join(&feather_core::paths::join(&base, feather_core::install::DATA_DIR_NAME), "logs");
    std::fs::create_dir_all(&logs).ok()?;
    Some(logs)
}

fn wide_path(dir: &str, name: &str) -> Vec<u16> {
    feather_core::paths::join(dir, name).encode_utf16().chain(std::iter::once(0)).collect()
}

/// 啟動時呼叫（只在主程式，不在預覽工作程序）：開始記錄步驟、註冊例外記錄、把標準錯誤導到檔案。
pub fn install() {
    let Some(dir) = log_dir() else { return };
    LOG_PATH.store(Box::leak(wide_path(&dir, "crash.log").into_boxed_slice()).as_mut_ptr(), Ordering::Release);
    unsafe {
        let err = wide_path(&dir, "stderr.log");
        let h = CreateFileW(err.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ_WRITE, std::ptr::null(), CREATE_ALWAYS, 0, 0);
        if h != INVALID_HANDLE_VALUE {
            SetStdHandle(STD_ERROR_HANDLE, h);
        }
        let trace = wide_path(&dir, "trace.log");
        // 先清空，再以附加模式開啟
        let h = CreateFileW(trace.as_ptr(), GENERIC_WRITE, FILE_SHARE_READ_WRITE, std::ptr::null(), CREATE_ALWAYS, 0, 0);
        if h != INVALID_HANDLE_VALUE {
            CloseHandle(h);
        }
        let h = CreateFileW(trace.as_ptr(), FILE_APPEND_DATA, FILE_SHARE_READ_WRITE, std::ptr::null(), OPEN_ALWAYS, 0, 0);
        if h != INVALID_HANDLE_VALUE {
            TRACE.store(h, Ordering::Relaxed);
        }
        AddVectoredExceptionHandler(1, vectored);
        SetUnhandledExceptionFilter(Some(filter));
    }
    crumb(concat!("啟動 Stillfeather ", env!("CARGO_PKG_VERSION")));
}

/// 不配置記憶體的小型緩衝區（當機時堆積可能已損壞）
struct Buf<const N: usize> {
    data: [u8; N],
    len: usize,
}

impl<const N: usize> Buf<N> {
    fn new() -> Self {
        Buf { data: [0; N], len: 0 }
    }

    fn put(&mut self, s: &[u8]) {
        let n = s.len().min(N - self.len);
        self.data[self.len..self.len + n].copy_from_slice(&s[..n]);
        self.len += n;
    }

    fn hex(&mut self, v: u64) {
        let mut tmp = [0u8; 18];
        tmp[0] = b'0';
        tmp[1] = b'x';
        for i in 0..16 {
            let d = ((v >> ((15 - i) * 4)) & 0xF) as u8;
            tmp[2 + i] = if d < 10 { b'0' + d } else { b'A' + d - 10 };
        }
        self.put(&tmp);
    }

    fn dec(&mut self, v: u64) {
        let mut tmp = [0u8; 20];
        let mut i = tmp.len();
        let mut v = v;
        loop {
            i -= 1;
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        self.put(&tmp[i..]);
    }

    fn dec2(&mut self, v: u16, width: usize) {
        let mut tmp = [b'0'; 4];
        let mut v = v;
        for i in (0..width.min(4)).rev() {
            tmp[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        self.put(&tmp[..width.min(4)]);
    }

    /// 時:分:秒.毫秒
    fn time(&mut self) {
        let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetLocalTime(&mut t) };
        self.dec2(t.wHour, 2);
        self.put(b":");
        self.dec2(t.wMinute, 2);
        self.put(b":");
        self.dec2(t.wSecond, 2);
        self.put(b".");
        self.dec2(t.wMilliseconds, 3);
    }

    /// 例外代碼、位置與存取違規的細節
    unsafe fn exception(&mut self, rec: &EXCEPTION_RECORD) {
        let base = GetModuleHandleW(std::ptr::null()) as u64;
        let addr = rec.address as u64;
        self.put("例外代碼 ".as_bytes());
        self.hex(rec.code as u64);
        self.put("，位置 ".as_bytes());
        self.hex(addr);
        if addr >= base && addr - base < 0x1000_0000 {
            self.put("（Stillfeather +".as_bytes());
            self.hex(addr - base);
            self.put("）".as_bytes());
        } else {
            self.put("（不在 Stillfeather 內，可能是系統或其他 DLL）".as_bytes());
        }
        if rec.code == 0xC000_0005 && rec.count >= 2 {
            self.put(match rec.info[0] {
                0 => "，讀取 ".as_bytes(),
                1 => "，寫入 ".as_bytes(),
                _ => "，執行 ".as_bytes(),
            });
            self.hex(rec.info[1] as u64);
        }
    }
}

/// 嚴重的硬體例外：立刻記到 trace.log（之後可能仍被系統或外掛處理掉，所以只記錄、不攔截）。
unsafe extern "system" fn vectored(info: *mut EXCEPTION_POINTERS) -> i32 {
    if info.is_null() || (*info).record.is_null() {
        return 0;
    }
    let rec = &*(*info).record;
    let serious = matches!(rec.code, 0xC000_0005 | 0xC000_00FD | 0xC000_0374 | 0xC000_0409 | 0xC000_001D | 0xC000_0096 | 0xC000_0006);
    if serious {
        let mut b = Buf::<512>::new();
        b.time();
        b.put(b" [");
        b.dec(GetCurrentThreadId() as u64);
        b.put("] !! ".as_bytes());
        b.exception(rec);
        b.put(b"\r\n");
        trace_write(&b);
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

unsafe extern "system" fn filter(info: *mut EXCEPTION_POINTERS) -> i32 {
    let path = LOG_PATH.load(Ordering::Acquire);
    if path.is_null() || info.is_null() || (*info).record.is_null() {
        return 0; // EXCEPTION_CONTINUE_SEARCH
    }
    let rec = &*(*info).record;
    let mut b = Buf::<8192>::new();
    b.put("Stillfeather 當機記錄\r\n時間 ".as_bytes());
    b.time();
    b.put(b"\r\n");
    b.exception(rec);
    b.put("\r\n最後經過的步驟（舊到新）：\r\n".as_bytes());
    let next = NEXT.load(Ordering::Relaxed);
    for k in next.saturating_sub(RING)..next {
        let i = k % RING;
        let p = CRUMB_PTR[i].load(Ordering::Relaxed);
        let n = CRUMB_LEN[i].load(Ordering::Relaxed);
        if !p.is_null() {
            b.put(b"  ");
            b.put(std::slice::from_raw_parts(p, n));
            b.put(b"\r\n");
        }
    }
    let h = CreateFileW(path, GENERIC_WRITE, FILE_SHARE_READ_WRITE, std::ptr::null(), CREATE_ALWAYS, 0, 0);
    if h != INVALID_HANDLE_VALUE {
        let mut written = 0u32;
        WriteFile(h, b.data.as_ptr(), b.len as u32, &mut written, std::ptr::null_mut());
        FlushFileBuffers(h);
        CloseHandle(h);
    }
    let mut t = Buf::<256>::new();
    t.time();
    t.put(" 未處理的例外，程式即將結束（詳見 crash.log）\r\n".as_bytes());
    trace_write(&t);
    0
}

//! 寬字串、錯誤訊息與常用的小型 Win32 包裝。

use super::sys::*;
use std::ffi::c_void;

/// 以 0 結尾的 UTF-16 字串。
/// 目前執行中的 exe 完整路徑
pub fn current_exe_path() -> Option<String> {
    let mut buf = vec![0u16; 1024];
    loop {
        let n = unsafe { GetModuleFileNameW(0, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(String::from_utf16_lossy(&buf[..n]));
        }
        if buf.len() >= 32 * 1024 {
            return None;
        }
        buf.resize(buf.len() * 2, 0);
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// 從以 0 結尾（或整個）緩衝區取出字串。
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 讀取以 0 結尾的 UTF-16 指標。
pub unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

/// 把文字複製到 Windows 提供的固定長度緩衝區（必要時截斷）。
pub unsafe fn copy_to_buffer(text: &str, buf: *mut u16, cap: i32) {
    if buf.is_null() || cap <= 0 {
        return;
    }
    let cap = cap as usize;
    let mut i = 0;
    for u in text.encode_utf16() {
        if i + 1 >= cap {
            break;
        }
        *buf.add(i) = u;
        i += 1;
    }
    *buf.add(i) = 0;
}

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

pub fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

pub fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

pub fn get_x_lparam(l: LPARAM) -> i32 {
    (l & 0xFFFF) as i16 as i32
}

pub fn get_y_lparam(l: LPARAM) -> i32 {
    ((l >> 16) & 0xFFFF) as i16 as i32
}

/// Windows 錯誤碼的系統訊息（使用系統語言，繁中系統會是中文）。
pub fn error_text(code: u32) -> String {
    // 網路位置常見的錯誤：Windows 原本的訊息看不出該怎麼辦，補上說明
    match code {
        53 | 67 => {
            return tr!(
                "找不到電腦或共用資料夾。請確認對方電腦開著、在同一個網路、有開啟檔案共用。",
                "Can't find the computer or shared folder. Make sure the other computer is on, on the same network, and has file sharing turned on."
            )
            .into()
        }
        1219 => {
            return tr!(
                "已經用另一組帳號連到這台電腦。Windows 對同一台電腦只能用一組帳號。",
                "You're already connected to this computer with a different account. Windows allows only one account per computer."
            )
            .into()
        }
        1223 => return tr!("已取消。", "Cancelled.").into(),
        1272 => {
            return tr!(
                "Windows 的安全性原則封鎖了訪客存取。Windows 11 24H2 之後，很多 NAS 的訪客共用都會遇到，需要在 NAS 上設定帳號。",
                "Windows security policy blocks guest access. Many NAS guest shares hit this on Windows 11 24H2 and later; set up an account on the NAS."
            )
            .into()
        }
        _ => {}
    }
    let mut buf = [0u16; 512];
    let n = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            std::ptr::null(),
            code,
            0,
            buf.as_mut_ptr(),
            buf.len() as u32,
            std::ptr::null(),
        )
    };
    let text = from_wide(&buf[..n as usize]).trim().to_string();
    if text.is_empty() {
        trf!("Windows 錯誤 {code}", "Windows error {code}")
    } else {
        text
    }
}

pub fn last_error_text() -> String {
    error_text(unsafe { GetLastError() })
}

pub fn window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

pub fn set_window_text(hwnd: HWND, text: &str) {
    if window_text(hwnd) != text {
        let w = wide(text);
        unsafe {
            SetWindowTextW(hwnd, w.as_ptr());
        }
    }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn message_box(owner: HWND, text: &str, kind: u32) -> i32 {
    let t = wide(text);
    let c = wide("Stillfeather");
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), kind) }
}

/// 強制回應的對話方塊開著時點到被停用的視窗：系統預設會發出提示音（DefWindowProc 呼叫 MessageBeep）。
/// 這裡改成安靜地讓對話方塊的標題列閃兩下並移到前面。在 WM_SETCURSOR 呼叫，回傳 true 表示已處理。
pub fn swallow_modal_click(hwnd: HWND, lparam: LPARAM) -> bool {
    let hit = (lparam & 0xFFFF) as u16 as i16 as i32;
    if hit != HTERROR {
        return false;
    }
    let mouse = ((lparam >> 16) & 0xFFFF) as u32;
    unsafe {
        if matches!(mouse, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
            let popup = GetLastActivePopup(GetAncestor(hwnd, GA_ROOT));
            if popup != 0 && popup != hwnd {
                SetForegroundWindow(popup);
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd: popup,
                    dwFlags: FLASHW_CAPTION,
                    uCount: 2,
                    dwTimeout: 70,
                };
                FlashWindowEx(&info);
            }
        }
        SetCursor(load_cursor(IDC_ARROW));
    }
    true
}

pub fn key_down(vk: i32) -> bool {
    unsafe { GetKeyState(vk) < 0 }
}

pub fn client_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut r);
    }
    r
}

pub fn cursor_pos() -> POINT {
    let mut p = POINT::default();
    unsafe {
        GetCursorPos(&mut p);
    }
    p
}

pub fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

pub fn send(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, w, l) }
}

pub fn send_ptr<T>(hwnd: HWND, msg: u32, w: WPARAM, p: *const T) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, w, p as *const c_void as LPARAM) }
}

pub fn move_window(hwnd: HWND, r: RECT) {
    unsafe {
        SetWindowPos(hwnd, 0, r.left, r.top, r.width().max(0), r.height().max(0), SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn show(hwnd: HWND, visible: bool) {
    unsafe {
        let now = IsWindowVisible(hwnd) != 0;
        if now != visible {
            ShowWindow(hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

/// 以系統時區格式化 FILETIME 為「yyyy/MM/dd HH:mm」。
pub fn format_filetime(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    unsafe {
        let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        if FileTimeToSystemTime(&f, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return String::new();
        }
        feather_core::format::format_date(local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
    }
}

/// 今天（本地時間）
pub fn local_today() -> feather_core::searchquery::Ymd {
    let mut st = SYSTEMTIME::default();
    unsafe {
        GetLocalTime(&mut st);
    }
    feather_core::searchquery::Ymd::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32).unwrap_or(feather_core::searchquery::Ymd { y: 2000, m: 1, d: 1 })
}

/// 使用者地區設定的一週第一天：0 是星期一……6 是星期日（讀不到時用星期一）
pub fn first_day_of_week() -> u32 {
    let mut buf = [0u16; 8];
    let n = unsafe { GetLocaleInfoEx(std::ptr::null(), LOCALE_IFIRSTDAYOFWEEK, buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 0 {
        return 0;
    }
    String::from_utf16_lossy(feather_core::namematch::until_nul(&buf)).trim().parse::<u32>().ok().filter(|d| *d <= 6).unwrap_or(0)
}

/// 本地時間某一天 0 點的 FILETIME（UTC）。日光節約時間由 Windows 依當天的規則換算。
pub fn local_midnight(d: feather_core::searchquery::Ymd) -> u64 {
    let local = SYSTEMTIME { wYear: d.y as u16, wMonth: d.m as u16, wDayOfWeek: 0, wDay: d.d as u16, wHour: 0, wMinute: 0, wSecond: 0, wMilliseconds: 0 };
    let mut utc = SYSTEMTIME::default();
    let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    unsafe {
        if TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) == 0 || SystemTimeToFileTime(&utc, &mut ft) == 0 {
            return 0;
        }
    }
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// 日期條件換算成 FILETIME 範圍（本地的日界線；「本週」從地區設定的一週第一天開始）
pub fn date_range(spec: &feather_core::searchquery::DateSpec) -> feather_core::searchquery::TimeRange {
    let (from, to) = spec.local_days(local_today(), first_day_of_week());
    feather_core::searchquery::TimeRange { from: from.map_or(0, local_midnight), to: to.map_or(u64::MAX, local_midnight) }
}

/// 搜尋框的多個日期條件換算後取交集；沒有日期條件時是 None
pub fn query_time(q: &feather_core::searchquery::Query) -> Option<feather_core::searchquery::TimeRange> {
    q.dates.iter().map(date_range).reduce(|a, b| a.intersect(b))
}

/// GetFullPathNameW + 展開環境變數 + 去掉引號，對應 C# 的 Path.GetFullPath(ExpandEnvironmentVariables(...))。
pub fn full_path(input: &str) -> Result<String, String> {
    full_path_in(input, None)
}

/// 同 [`full_path`]，但相對路徑（`..`、`子資料夾`、`\Windows`）以 `base`（目前資料夾）為準，
/// 而不是程式的工作目錄（那通常是 exe 或捷徑所在的資料夾，與使用者正在看的地方無關）。
pub fn full_path_in(input: &str, base: Option<&str>) -> Result<String, String> {
    let trimmed = input.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(tr!("路徑為空白。", "The path is blank.").into());
    }
    // 只輸入「C:」時前往磁碟根目錄，而不是該磁碟的目前工作目錄。
    let drive_only;
    let trimmed = if trimmed.len() == 2 && trimmed.as_bytes()[1] == b':' && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        drive_only = format!("{trimmed}\\");
        drive_only.as_str()
    } else {
        trimmed
    };
    let expanded = unsafe {
        let w = wide(trimmed);
        let n = ExpandEnvironmentStringsW(w.as_ptr(), std::ptr::null_mut(), 0);
        if n == 0 {
            trimmed.to_string()
        } else {
            let mut buf = vec![0u16; n as usize + 1];
            let m = ExpandEnvironmentStringsW(w.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
            if m == 0 {
                trimmed.to_string()
            } else {
                from_wide(&buf)
            }
        }
    };
    let expanded = match base.filter(|b| !b.is_empty()).and_then(|b| feather_core::paths::resolve_relative(b, &expanded)) {
        Some(joined) => joined,
        None => expanded,
    };
    unsafe {
        let w = wide(&expanded);
        let n = GetFullPathNameW(w.as_ptr(), 0, std::ptr::null_mut(), std::ptr::null_mut());
        if n == 0 {
            return Err(last_error_text());
        }
        let mut buf = vec![0u16; n as usize + 1];
        let m = GetFullPathNameW(w.as_ptr(), buf.len() as u32, buf.as_mut_ptr(), std::ptr::null_mut());
        if m == 0 || m as usize >= buf.len() {
            return Err(last_error_text());
        }
        let full = from_wide(&buf);
        Ok(feather_core::paths::trim_end_sep(&full).to_string())
    }
}

/// 讀取 DWORD 登錄值。
pub fn reg_dword(sub: &str, value: &str) -> Option<u32> {
    let s = wide(sub);
    let v = wide(value);
    let mut data: u32 = 0;
    let mut size: u32 = 4;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            s.as_ptr(),
            v.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut c_void,
            &mut size,
        )
    };
    if rc == 0 {
        Some(data)
    } else {
        None
    }
}

/// 暫時把游標換成指定樣式（例如沙漏）。
pub fn load_cursor(id: usize) -> HCURSOR {
    unsafe { LoadCursorW(0, id as *const u16) }
}

/// Windows 的組建編號（RtlGetVersion，不受相容性資訊清單影響）；取不到時當成 Windows 11（0 以外的最小值不會誤判成舊版）
pub fn windows_build() -> u32 {
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(|| unsafe {
        let mut info: OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        if RtlGetVersion(&mut info) == 0 && info.dwBuildNumber > 0 {
            info.dwBuildNumber
        } else {
            feather_core::winver::WINDOWS_11
        }
    })
}

pub fn scale(v: i32, dpi: u32) -> i32 {
    unsafe { MulDiv(v, dpi as i32, 96) }
}

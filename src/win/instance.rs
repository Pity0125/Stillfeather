//! 單一主視窗：同一 Windows 使用者、同一互動登入工作階段只有一個主視窗。
//! 第二次啟動以 WM_COPYDATA 把路徑交給既有視窗，得到確認（回傳 1）後才結束。
//! 視窗類別名稱與互斥鎖都含使用者名稱；Local\ 命名空間本身就以工作階段區隔。

use super::sys::*;
use super::util::*;
use feather_core::ipc;
use std::time::{Duration, Instant};

pub struct Instance {
    mutex: HANDLE,
    pub primary: bool,
}

fn user_tag() -> String {
    let mut buf = [0u16; 257];
    let mut len = buf.len() as u32;
    let name = if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } != 0 { from_wide(&buf) } else { "user".into() };
    // 物件名稱不可含反斜線
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub fn main_class() -> String {
    format!("FeatherFiles.Rust.Main.{}", user_tag())
}

impl Instance {
    pub fn acquire() -> Instance {
        let name = wide(&format!("Local\\FeatherFiles.Rust.Instance.v1.{}", user_tag()));
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            let existed = GetLastError() == ERROR_ALREADY_EXISTS;
            Instance { mutex, primary: mutex != 0 && !existed }
        }
    }

    /// 把開啟要求送給既有主視窗；主視窗可能還在啟動，最多等 5 秒。
    pub fn send_to_primary(&self, paths: &[String]) -> bool {
        let Ok(payload) = ipc::encode(paths) else { return false };
        let class = wide(&main_class());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
            if hwnd != 0 && send_to(hwnd, &payload) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(75));
        }
        false
    }
}

/// 主視窗已經開著時把開啟要求交給它（不等主視窗出現）。沒有開著或沒有回應時回傳 false。
pub fn send_if_running(paths: &[String]) -> bool {
    let Ok(payload) = ipc::encode(paths) else { return false };
    let class = wide(&main_class());
    let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
    hwnd != 0 && send_to(hwnd, &payload)
}

fn send_to(hwnd: HWND, payload: &[u16]) -> bool {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        AllowSetForegroundWindow(pid);
    }
    let cds = COPYDATASTRUCT {
        dwData: ipc::COPYDATA_TAG,
        cbData: (payload.len() * 2) as u32,
        lpData: payload.as_ptr() as *const std::ffi::c_void,
    };
    let mut result: usize = 0;
    let ok = unsafe { SendMessageTimeoutW(hwnd, WM_COPYDATA, 0, &cds as *const COPYDATASTRUCT as LPARAM, SMTO_ABORTIFHUNG, 3000, &mut result) };
    ok != 0 && result == 1
}

impl Drop for Instance {
    fn drop(&mut self) {
        if self.mutex != 0 {
            unsafe {
                CloseHandle(self.mutex);
            }
        }
    }
}

/// 主視窗收到 WM_COPYDATA 時解析；不是本程式的資料回傳 None。
pub unsafe fn decode_copydata(lparam: LPARAM) -> Option<Vec<String>> {
    let cds = (lparam as *const COPYDATASTRUCT).as_ref()?;
    if cds.dwData != ipc::COPYDATA_TAG || cds.cbData % 2 != 0 || cds.cbData as usize > ipc::MAX_BYTES || cds.lpData.is_null() {
        return None;
    }
    let data = std::slice::from_raw_parts(cds.lpData as *const u16, cds.cbData as usize / 2);
    ipc::decode(data).ok()
}

//! 背景攔截 Win+E（`Stillfeather.exe --win-e`）。為什麼需要、按鍵怎麼判斷，見 `feather_core::wine`。
//!
//! - 沒有視窗（只有一個訊息視窗），同一使用者、同一工作階段只執行一份（互斥鎖）。
//! - 低階鍵盤攔截（WH_KEYBOARD_LL）掛在這個執行緒：攔截程序只做判斷、送出遮罩鍵，開啟 Stillfeather 交給另一個執行緒，
//!   不拖慢鍵盤（太慢的話 Windows 會悄悄拿掉攔截）。每分鐘確認還要不要攔截（不要就結束），並重新掛上一次。
//! - 主程式（每次啟動、設定改變）與安裝程式以訊息視窗的類別名稱找到它，送 WM_CLOSE 請它結束（見 shellreg::stop_win_e）。

use super::instance;
use super::shellreg;
use super::sys::*;
use super::util::*;
use feather_core::install;
use feather_core::wine::{self, KeyAction, KeyEvent, WinE};
use std::cell::RefCell;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicIsize, Ordering};

/// 攔截到 Win+E：開啟 Stillfeather
const WM_OPEN: u32 = WM_APP + 1;
const TIMER_CHECK: usize = 1;
const CHECK_MS: u32 = 60 * 1000;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static KEYS: RefCell<WinE> = RefCell::new(WinE::default());
}

fn held(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) as u16 & 0x8000 != 0 }
}

/// 這一份 Stillfeather 要不要負責攔截（選項開著，預設檔案總管就是這一份）
fn should_run() -> bool {
    let Some(exe) = current_exe_path() else { return false };
    shellreg::win_e_hook_exe().is_some_and(|p| install::same_path(&p, &exe))
}

/// `--win-e` 的進入點
pub fn run() -> i32 {
    let user = shellreg::user_tag();
    let name = wide(&wine::mutex_name(&user));
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if mutex == 0 || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // 已經有一份在攔截
        if mutex != 0 {
            unsafe {
                CloseHandle(mutex);
            }
        }
        return 0;
    }
    // 登入時啟動，但選項已經關掉或預設檔案總管不是這一份：整理啟動命令後結束
    shellreg::sync_win_e_autorun();
    if !should_run() {
        unsafe {
            CloseHandle(mutex);
        }
        return 0;
    }
    let code = unsafe { message_loop(&user) };
    unsafe {
        CloseHandle(mutex);
    }
    code
}

unsafe fn message_loop(user: &str) -> i32 {
    let class = wide(&wine::window_class(user));
    let module = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: window_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: module,
        hIcon: 0,
        hCursor: 0,
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: 0,
    };
    RegisterClassExW(&wc);
    let hwnd = CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, 0, module, std::ptr::null());
    if hwnd == 0 {
        return 1;
    }
    WINDOW.store(hwnd, Ordering::Relaxed);
    if !hook() {
        DestroyWindow(hwnd);
        return 1;
    }
    SetTimer(hwnd, TIMER_CHECK, CHECK_MS, std::ptr::null());
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, 0, 0, 0) > 0 {
        DispatchMessageW(&msg);
    }
    let old = HOOK.swap(0, Ordering::Relaxed);
    if old != 0 {
        UnhookWindowsHookEx(old);
    }
    0
}

/// 掛上（或重新掛上）鍵盤攔截：先掛新的再拿掉舊的，中間不會漏掉按鍵
unsafe fn hook() -> bool {
    let new = SetWindowsHookExW(WH_KEYBOARD_LL, keyboard_proc, GetModuleHandleW(std::ptr::null()), 0);
    if new == 0 {
        return false;
    }
    let old = HOOK.swap(new, Ordering::Relaxed);
    if old != 0 {
        UnhookWindowsHookEx(old);
    }
    true
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_OPEN => {
            // 開啟可能要一點時間（啟動程式、等主視窗回應）：交給另一個執行緒，鍵盤攔截不會被耽擱
            std::thread::spawn(open_stillfeather);
            0
        }
        WM_TIMER if wparam == TIMER_CHECK => {
            // 選項關掉、預設檔案總管換成別的程式（或另一份 Stillfeather）：結束
            if !should_run() {
                DestroyWindow(hwnd);
            } else {
                // Windows 在攔截逾時後會悄悄拿掉它（不會通知）：每次檢查都重新掛上，失效最多一分鐘。
                // 重掛幾乎沒有成本：先掛新的再拿掉舊的，失敗時舊的仍在
                hook();
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION && lparam != 0 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        // 只看 E：其他按鍵不做任何事，直接交給下一個
        if k.vkCode == wine::VK_E {
            let msg = wparam as u32;
            let ev = KeyEvent {
                vk: k.vkCode,
                down: msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN,
                injected: k.flags & LLKHF_INJECTED != 0,
                time: k.time,
                win: held(VK_LWIN) || held(VK_RWIN),
                mods: held(VK_SHIFT) || held(VK_CONTROL) || held(VK_MENU),
                // 攔截程序被呼叫時，系統的按鍵狀態還沒更新：這是這次之前的狀態
                was_down: held(wine::VK_E as i32),
            };
            match KEYS.with(|k| k.borrow_mut().key(ev)) {
                KeyAction::Pass => {}
                KeyAction::Swallow => return 1,
                KeyAction::Open => {
                    send_mask_key();
                    // 送出按鍵的程序可以把前景交出去：讓 Stillfeather 的視窗能跳到最前面
                    AllowSetForegroundWindow(ASFW_ANY);
                    PostMessageW(WINDOW.load(Ordering::Relaxed), WM_OPEN, 0, 0);
                    return 1;
                }
            }
        }
    }
    CallNextHookEx(0, code, wparam, lparam)
}

/// E 被吃掉了：補一個無作用的按鍵，放開 Win 時 Windows 才不會以為只按了 Win 而打開開始功能表。
/// 要在攔截程序裡直接送出，才會排在使用者放開 Win 之前。
unsafe fn send_mask_key() {
    let key = |flags: u32| KEYBOARD_INPUT {
        kind: INPUT_KEYBOARD,
        ki: KEYBDINPUT { wVk: wine::VK_MASK, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        _pad: [0; 8],
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<KEYBOARD_INPUT>() as i32);
}

/// 和舊版 Windows 的 Win+E 一樣：Stillfeather 開著就在新分頁開首頁，沒開就啟動
fn open_stillfeather() {
    let home = [install::HOME_MARKER.to_string()];
    if instance::send_if_running(&home) {
        return;
    }
    if let Some(exe) = current_exe_path() {
        let _ = start(&exe, install::OPEN_HOME_ARG);
    }
}

/// 啟動 Stillfeather（不讓它繼承這個程序的記錄檔等標準輸出入）
fn start(exe: &str, arg: &str) -> std::io::Result<std::process::Child> {
    Command::new(exe).arg(arg).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
}

// ---------------------------------------------------------------- 主程式：啟動或停止

/// 執行中的背景攔截是哪一份 Stillfeather
fn running_exe(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 主程式每次啟動與設定改變後：依使用者的選擇與預設檔案總管啟動或停止背景攔截，並整理登入時的啟動命令。
/// 預設檔案總管換成另一份 Stillfeather（例如安裝版與開發版）時，改由那一份攔截。
pub fn maintain() {
    let exe = shellreg::win_e_hook_exe();
    let _ = shellreg::set_win_e_autorun(exe.as_deref());
    let hwnd = shellreg::win_e_window();
    match exe {
        Some(exe) => {
            if hwnd != 0 {
                if running_exe(hwnd).is_none_or(|p| install::same_path(&p, &exe)) {
                    return;
                }
                shellreg::stop_win_e(2000);
            }
            let _ = start(&exe, wine::WIN_E_ARG);
        }
        None => {
            if hwnd != 0 {
                shellreg::stop_win_e(2000);
            }
        }
    }
}

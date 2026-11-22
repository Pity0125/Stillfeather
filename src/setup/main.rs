//! Stillfeather 安裝程式（StillfeatherSetup.exe）；安裝後同一個檔案也複製成解除安裝程式（Uninstall.exe）。
//!
//! - 安裝：選擇位置（預設 %LOCALAPPDATA%\Programs\Stillfeather，不需要系統管理員權限）與選項 →
//!   寫入 Stillfeather（內嵌在安裝程式裡）、解除安裝程式、捷徑與「已安裝的應用程式」項目 → 需要時設為預設檔案總管。
//!   裝到需要權限的位置（例如 Program Files）時，以系統管理員身分重新執行自己做那一步。
//! - 解除安裝：在「設定 › 應用程式 › 已安裝的應用程式」按解除安裝時執行 Uninstall.exe：先把自己複製到暫存資料夾再執行，
//!   接著改回 Windows 檔案總管與 Windows 的開啟檔案視窗、刪除捷徑、程式檔案、設定與記錄（%LOCALAPPDATA%\FeatherFiles）和登錄，
//!   最後刪掉暫存的自己。
//! - 靜默模式（/S）給部署工具與商店使用：不顯示畫面，以結束代碼回報（0 成功、1 失敗、3 參數錯誤）。
//!   參數見 `feather_core::install::parse_setup_args`。
#![windows_subsystem = "windows"]

#[path = "../win/sys.rs"]
mod sys;
#[path = "../win/shellreg.rs"]
mod shellreg;
mod ffi;
mod ops;
mod ui;

use feather_core::install::{self, SetupMode};
use feather_core::paths;
use sys::*;

/// 內嵌的 Stillfeather（建置時由環境變數 STILLFEATHER_PAYLOAD 指定，見 build.ps1 -Setup）
pub static PAYLOAD: &[u8] = include_bytes!(env!("STILLFEATHER_PAYLOAD"));
/// 內嵌的「開啟檔案」視窗取代元件（STILLFEATHER_PICKER_PAYLOAD 指定；沒有指定時是空的，不安裝）
pub static PICKER_PAYLOAD: &[u8] = include_bytes!(env!("STILLFEATHER_PICKER_FILE"));
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    std::process::exit(run());
}

fn message(text: &str) {
    let t = ops::w(text);
    let c = ops::w("Stillfeather 安裝程式");
    unsafe {
        MessageBoxW(0, t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

fn run() -> i32 {
    unsafe {
        // 資訊清單已經宣告；沒有嵌入資訊清單的建置（略過資源時）也要能正確縮放
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
    }
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut args = match install::parse_setup_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            if !raw.iter().any(|a| matches!(a.to_ascii_lowercase().as_str(), "/s" | "/silent" | "/verysilent" | "/quiet" | "/q" | "--silent" | "--quiet")) {
                message(&e);
            }
            return 3;
        }
    };
    let me = ops::own_exe().unwrap_or_default();
    // 安裝資料夾裡的 Uninstall.exe（不論有沒有帶參數）一律是解除安裝
    if paths::file_name(&me).eq_ignore_ascii_case(install::UNINSTALLER_NAME) {
        args.mode = SetupMode::Uninstall;
    }
    match args.mode {
        SetupMode::Install => {
            if args.silent {
                ops::silent_install(&args)
            } else if args.update {
                ui::run_update(args)
            } else {
                ui::run_install(args)
            }
        }
        SetupMode::Uninstall => {
            let dir = match args.dir.clone() {
                Some(d) => d,
                None => {
                    // 從安裝資料夾啟動：換成暫存資料夾裡的那一份執行
                    let Some(dir) = paths::parent(&me) else { return 1 };
                    if let Some(code) = ops::relaunch_from_temp(&dir, args.silent) {
                        return code;
                    }
                    dir
                }
            };
            if args.silent {
                ops::silent_uninstall(&args, &dir)
            } else {
                ui::run_uninstall(args, dir)
            }
        }
    }
}

//! 連續啟動失敗的救援與資料夾搬移（判斷邏輯在 `feather_core::crashguard`）。
//!
//! - [`Marker`]：主程式每次啟動放一個記號，正常結束或撐過 `crashguard::STABLE_MS` 就拿掉。
//! - [`rescue_if_crash_loop`]：前面一分鐘內已經有三次沒能正常啟動時，不開 Stillfeather，而是改回 Windows 檔案總管
//!   （預設檔案總管、Win+E 的背景攔截、其他程式的開啟檔案視窗），用它開啟這次要開的位置，並說明怎麼重新開啟。
//!   使用者連資料夾都打不開的情況就不會發生；選擇記成「關」，修好之後在設定重新勾選即可。
//! - [`migrate_data_dir`]／[`remove_old_data_dir`]：舊版（0.1.0 與最初發佈的 0.1.1）的 %LOCALAPPDATA%\FeatherFiles 搬到 %LOCALAPPDATA%\Stillfeather。

use super::shellreg;
use super::util::*;
use super::sys::*;
use feather_core::{crashguard, install, paths};

fn local_app_data() -> Option<String> {
    std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty())
}

fn marker_dir() -> Option<String> {
    Some(paths::join(&install::data_dir(&local_app_data()?), crashguard::DIR_NAME))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// 這次啟動的記號：drop（正常結束）或撐過 `STABLE_MS` 時刪掉。程式當掉（panic = abort、存取違規）就留著。
pub struct Marker {
    path: Option<String>,
}

impl Marker {
    pub fn start() -> Marker {
        let Some(dir) = marker_dir() else { return Marker { path: None } };
        let _ = std::fs::create_dir_all(&dir);
        let path = paths::join(&dir, &crashguard::marker_name(now_ms(), std::process::id()));
        if std::fs::write(&path, b"").is_err() {
            return Marker { path: None };
        }
        let stable = path.clone();
        let _ = std::thread::Builder::new().name("startup-marker".into()).spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(crashguard::STABLE_MS));
            let _ = std::fs::remove_file(&stable);
        });
        Marker { path: Some(path) }
    }
}

impl Drop for Marker {
    fn drop(&mut self) {
        if let Some(p) = &self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 前面連續沒能正常啟動時救援，回傳 true（這次不開 Stillfeather）。`targets` 是這次要開的位置
/// （`install::HOME_MARKER` 表示首頁）。
pub fn rescue_if_crash_loop(targets: &[String]) -> bool {
    let Some(dir) = marker_dir() else { return false };
    let existing: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(_) => return false,
    };
    let check = crashguard::check(&existing, now_ms(), &process_running);
    for name in &check.remove {
        let _ = std::fs::remove_file(paths::join(&dir, name));
    }
    if !check.rescue {
        return false;
    }
    keep_logs();
    restore_windows();
    open_in_explorer(targets);
    let logs = super::crashlog::log_dir().unwrap_or_else(|| "%LOCALAPPDATA%\\Stillfeather\\logs".into());
    message_box(
        0,
        &trf!(
            "Stillfeather 剛才連續 {} 次沒能正常開啟。為了讓你照常開資料夾，已經先改回 Windows：\n\n\
             •  預設檔案總管與 Win+E：Windows 檔案總管\n\
             •  其他程式的開啟檔案視窗：Windows 原本的視窗\n\n\
             這次要開的位置已經用 Windows 檔案總管開啟。問題排除後（例如更新 Stillfeather），可以在 Stillfeather 的設定（F12）重新勾選。\n\n\
             歡迎回報問題，請附上這個資料夾裡的 crash-before-rescue.log 與 trace-before-rescue.log（可能含有資料夾路徑）：\n{logs}", "Stillfeather failed to start {} times in a row. So you can keep opening folders, Windows has been restored for now:\n\n\
             •  Default file manager and Win+E: File Explorer\n\
             •  Other programs' file dialogs: the standard Windows dialogs\n\n\
             The location you wanted was opened in File Explorer. Once the problem is fixed (for example, by updating Stillfeather), turn the options back on in Stillfeather's Settings (F12).\n\n\
             Please report the problem and attach crash-before-rescue.log and trace-before-rescue.log from this folder (they may contain folder paths):\n{logs}",
            crashguard::LIMIT
        ),
        MB_OK | MB_ICONWARNING,
    );
    true
}

/// 那個程序是不是還在執行（同時開好幾個資料夾時，其他還在啟動的 Stillfeather 的記號不算失敗）。
/// 打不開（例如以系統管理員身分執行的那一份）時當成還在執行：寧可不救援，也不要誤判。
fn process_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid);
        if h == 0 {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let alive = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        CloseHandle(h);
        alive
    }
}

const ERROR_ACCESS_DENIED: u32 = 5;
const WAIT_TIMEOUT: u32 = 0x102;

/// 下一次正常啟動會重新開始記錄；先留一份當掉時的記錄，方便回報。
fn keep_logs() {
    let Some(dir) = super::crashlog::log_dir() else { return };
    for (from, to) in [("crash.log", "crash-before-rescue.log"), ("trace.log", "trace-before-rescue.log"), ("stderr.log", "stderr-before-rescue.log")] {
        let src = paths::join(&dir, from);
        if std::path::Path::new(&src).is_file() {
            let _ = std::fs::copy(&src, paths::join(&dir, to));
        }
    }
}

/// 改回 Windows 檔案總管與 Windows 的開啟檔案視窗，並記下「關」（之後啟動不會自己又開回來）。
/// 「攔截 Win+E」的選擇不動：它只在預設檔案總管是 Stillfeather 時作用，使用者重新勾選預設檔案總管時照原本的選擇。
fn restore_windows() {
    let _ = shellreg::set_default_fm_choice(false);
    let _ = shellreg::restore_default_file_manager();
    let _ = shellreg::set_win_e_autorun(None);
    shellreg::stop_win_e(2000);
    let _ = shellreg::set_picker_choice(false);
    let _ = shellreg::disable_picker();
}

/// 用 Windows 檔案總管開啟這次要開的位置：資料夾直接開、檔案開它所在的資料夾、首頁就開「本機」。
/// 也用在已開啟的 Stillfeather 沒有回應時（見 win::entry）。
///
/// 這時 Stillfeather 可能仍是預設檔案總管，所以明確用系統的「開啟」（open）動詞：Stillfeather 只新增自己的動詞並設為預設，
/// 沒有改 open，open 一定由檔案總管處理。若用預設動詞（或可能照預設動詞轉手的方式）開，會又啟動 Stillfeather、
/// 再等它沒有回應，一直循環。open 失敗時才直接執行 explorer.exe。
pub(super) fn open_in_explorer(targets: &[String]) {
    // 殼層的 open 動詞可能透過 COM（DelegateExecute）執行
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let mut opened = false;
    for t in targets.iter().take(8) {
        let folder = if t == install::HOME_MARKER {
            None
        } else if std::path::Path::new(t).is_dir() {
            Some(t.clone())
        } else {
            paths::parent(t).filter(|p| std::path::Path::new(p).is_dir())
        };
        opened |= open_with_explorer(folder.as_deref());
    }
    if !opened {
        open_with_explorer(None);
    }
}

/// 以 open 動詞在檔案總管開 `folder`（None：「本機」）；不行就直接執行 explorer.exe
fn open_with_explorer(folder: Option<&str>) -> bool {
    const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
    let verb = wide("open");
    let file = wide(folder.unwrap_or(THIS_PC));
    let rc = unsafe { ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    if rc > 32 {
        return true;
    }
    let windows = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")).unwrap_or_else(|_| r"C:\Windows".into());
    let mut cmd = std::process::Command::new(paths::join(&windows, "explorer.exe"));
    if let Some(f) = folder {
        cmd.arg(f);
    }
    cmd.spawn().is_ok()
}

/// 舊版的 %LOCALAPPDATA%\FeatherFiles → %LOCALAPPDATA%\Stillfeather：把設定類的檔案搬過去（新資料夾已經有的不蓋掉）。
/// 主程式與選擇器都在讀設定之前呼叫；重複呼叫沒有影響。
pub fn migrate_data_dir() {
    let Some(base) = local_app_data() else { return };
    let old = install::old_data_dir(&base);
    if !std::path::Path::new(&old).is_dir() {
        return;
    }
    let new = install::data_dir(&base);
    if std::fs::create_dir_all(&new).is_err() {
        return;
    }
    for name in install::MIGRATED_FILES {
        let from = paths::join(&old, name);
        let to = paths::join(&new, name);
        if std::path::Path::new(&from).is_file() && !std::path::Path::new(&to).exists() && std::fs::rename(&from, &to).is_err() {
            let _ = std::fs::copy(&from, &to);
        }
    }
}

/// 舊資料夾剩下的（記錄、資訊清單、舊的取代元件）：取代元件已經不登記在那裡之後才刪
/// （主程式啟動時 picker::maintain 會把它換到新資料夾）。還載入在其他程式裡的刪不掉，下次啟動再試。
pub fn remove_old_data_dir() {
    let Some(base) = local_app_data() else { return };
    let old = install::old_data_dir(&base);
    if !std::path::Path::new(&old).is_dir() {
        return;
    }
    if shellreg::picker_registered_dll().is_some_and(|p| install::is_inside(&p, &old)) {
        return;
    }
    let _ = std::fs::remove_dir_all(&old);
}

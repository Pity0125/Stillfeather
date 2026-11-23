//! 安裝與解除安裝的實際步驟（不含畫面）：寫入檔案、捷徑、登錄、預設檔案總管、關閉執行中的 Stillfeather，
//! 以及需要時以系統管理員身分重新執行自己。畫面（ui.rs）與靜默模式都呼叫這裡。

use super::ffi::*;
use super::shellreg;
use super::sys::*;
use super::{PAYLOAD, PICKER_PAYLOAD, VERSION};
use feather_core::install::{self, SetupArgs};
use feather_core::paths;
use std::ffi::c_void;
use std::time::{Duration, Instant};

pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

/// 進度回報：(百分比, 說明)
pub type Progress<'a> = &'a mut dyn FnMut(u32, &str);

pub struct InstallOptions {
    pub dir: String,
    /// 設為預設檔案總管（並記下選擇）；None：維持原狀（更新時）
    pub default_fm: Option<bool>,
    /// 其他程式開啟檔案時也用 Stillfeather；None：維持使用者原本的選擇（沒選過就是開啟，Stillfeather 啟動時處理）
    pub picker: Option<bool>,
    /// 捷徑：Some(true) 建立、Some(false) 拿掉、None 維持原狀（更新時）
    pub desktop: Option<bool>,
    pub start_menu: Option<bool>,
}

pub fn own_exe() -> Option<String> {
    let mut buf = vec![0u16; 2048];
    let n = unsafe { GetModuleFileNameW(0, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
}

fn known_folder(id: &GUID) -> Option<String> {
    unsafe {
        let mut p: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(id, 0, 0, &mut p) != 0 || p.is_null() {
            return None;
        }
        let s = from_wide_ptr(p);
        CoTaskMemFree(p.cast());
        Some(s)
    }
}

pub fn local_app_data() -> Option<String> {
    known_folder(&FOLDERID_LOCAL_APP_DATA).or_else(|| std::env::var("LOCALAPPDATA").ok())
}

/// 預設安裝位置：之前裝過就用原本的位置，否則 %LOCALAPPDATA%\Programs\Stillfeather
pub fn default_install_dir() -> String {
    if let Some(dir) = shellreg::read_string(install::APP_KEY, "InstallDir").filter(|d| !d.trim().is_empty()) {
        return dir;
    }
    install::default_install_dir(&local_app_data().unwrap_or_else(|| r"C:\Users\Public".into()))
}

/// 不能安裝進去的資料夾
pub fn protected_dirs() -> Vec<String> {
    let windows = std::env::var("SystemRoot").or_else(|_| std::env::var("windir")).unwrap_or_else(|_| r"C:\Windows".into());
    vec![windows]
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevated = 0u32;
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TOKEN_ELEVATION_CLASS, (&mut elevated as *mut u32).cast(), 4, &mut len);
        CloseHandle(token);
        ok != 0 && elevated != 0
    }
}

pub enum Access {
    Ok,
    NeedsAdmin,
    Failed(String),
}

/// 能不能在這個資料夾寫入（不存在就建立）。沒有權限時回傳 NeedsAdmin。
pub fn check_writable(dir: &str) -> Access {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return match e.kind() {
            std::io::ErrorKind::PermissionDenied => Access::NeedsAdmin,
            _ => Access::Failed(format!("無法建立安裝資料夾：{e}")),
        };
    }
    let probe = paths::join(dir, &format!(".stillfeather-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            Access::Ok
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Access::NeedsAdmin,
        Err(e) => Access::Failed(format!("無法寫入安裝資料夾：{e}")),
    }
}

// ---------------------------------------------------------------- 執行中的 Stillfeather

fn user_tag() -> String {
    let mut buf = [0u16; 257];
    let mut len = buf.len() as u32;
    let name = if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) } != 0 {
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    } else {
        "user".into()
    };
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// Stillfeather 的主視窗（與主程式 instance.rs 的類別名稱相同）
fn main_window() -> HWND {
    let class = w(&format!("FeatherFiles.Rust.Main.{}", user_tag()));
    unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }
}

pub fn app_running() -> bool {
    main_window() != 0
}

/// 使用者之前對「其他程式開啟檔案時也用 Stillfeather」的選擇（沒選過是關閉）：安裝畫面的勾選框照它顯示
pub fn picker_choice() -> bool {
    shellreg::picker_choice()
}

/// 請執行中的 Stillfeather 關閉，等它結束（最多 15 秒）。
pub fn close_app() -> Result<(), String> {
    let hwnd = main_window();
    if hwnd == 0 {
        return Ok(());
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    let process = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if process != 0 && unsafe { WaitForSingleObject(process, 100) } == WAIT_OBJECT_0 {
            unsafe {
                CloseHandle(process);
            }
            return Ok(());
        }
        if process == 0 {
            std::thread::sleep(Duration::from_millis(100));
            if main_window() == 0 {
                std::thread::sleep(Duration::from_millis(500));
                return Ok(());
            }
        }
    }
    if process != 0 {
        unsafe {
            CloseHandle(process);
        }
    }
    Err("Stillfeather 沒有關閉（可能正在詢問要不要中斷檔案作業）。請先關閉 Stillfeather 再試一次。".into())
}

// ---------------------------------------------------------------- 檔案

/// 先寫成 .new 再改名取代，寫到一半失敗也不會留下壞掉的 exe。
fn write_file_atomic(path: &str, data: &[u8]) -> Result<(), String> {
    let tmp = format!("{path}.new");
    std::fs::write(&tmp, data).map_err(|e| format!("無法寫入 {path}：{e}"))?;
    replace_file(&tmp, path)
}

fn replace_file(from: &str, to: &str) -> Result<(), String> {
    let (f, t) = (w(from), w(to));
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } != 0 {
        return Ok(());
    }
    let code = unsafe { GetLastError() };
    let _ = std::fs::remove_file(from);
    Err(if code == 5 || code == ERROR_SHARING_VIOLATION {
        format!("{to} 正在使用中。")
    } else {
        format!("無法寫入 {to}（錯誤 {code}）。")
    })
}

fn delete_file(path: &str) -> bool {
    let p = w(path);
    let deleted = unsafe { DeleteFileW(p.as_ptr()) } != 0;
    deleted || !std::path::Path::new(path).exists()
}

/// 建立捷徑（.lnk）
fn create_shortcut(lnk: &str, target: &str, dir: &str) -> Result<(), String> {
    unsafe {
        let mut link: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(&CLSID_SHELL_LINK, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link);
        if hr < 0 || link.is_null() {
            return Err(format!("無法建立捷徑（0x{:08X}）。", hr as u32));
        }
        let v = vtbl::<IShellLinkWVtbl>(link);
        let (t, d, desc) = (w(target), w(dir), w("Stillfeather 檔案管理"));
        (v.SetPath)(link, t.as_ptr());
        (v.SetWorkingDirectory)(link, d.as_ptr());
        (v.SetDescription)(link, desc.as_ptr());
        (v.SetIconLocation)(link, t.as_ptr(), 0);
        let mut file: *mut c_void = std::ptr::null_mut();
        let hr = (v.base.QueryInterface)(link, &IID_IPERSIST_FILE, &mut file);
        let result = if hr >= 0 && !file.is_null() {
            let l = w(lnk);
            let hr = (vtbl::<IPersistFileVtbl>(file).Save)(file, l.as_ptr(), 1);
            release(file);
            if hr >= 0 {
                Ok(())
            } else {
                Err(format!("無法儲存捷徑 {lnk}（0x{:08X}）。", hr as u32))
            }
        } else {
            Err("無法儲存捷徑。".into())
        };
        release(link);
        result
    }
}

fn shortcut_paths() -> (Option<String>, Option<String>) {
    let name = format!("{}.lnk", install::APP_NAME);
    (known_folder(&FOLDERID_PROGRAMS).map(|d| paths::join(&d, &name)), known_folder(&FOLDERID_DESKTOP).map(|d| paths::join(&d, &name)))
}

fn today() -> String {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut t);
    }
    format!("{:04}{:02}{:02}", t.wYear, t.wMonth, t.wDay)
}

// ---------------------------------------------------------------- 安裝

/// 在這個行程裡安裝（呼叫前已確認資料夾可以寫入）。
pub fn install(opts: &InstallOptions, progress: Progress) -> Result<(), String> {
    let dir = &opts.dir;
    let exe = paths::join(dir, install::EXE_NAME);
    progress(5, "準備安裝資料夾…");
    std::fs::create_dir_all(dir).map_err(|e| format!("無法建立安裝資料夾：{e}"))?;

    progress(15, "寫入 Stillfeather…");
    // 背景攔截 Win+E 執行中會鎖住 exe：先請它結束（裝好後再啟動，見 start_win_e）
    shellreg::stop_win_e(5000);
    if let Err(first) = write_file_atomic(&exe, PAYLOAD) {
        // 正在執行的舊版鎖住了 exe：請它關閉後再寫一次
        if app_running() {
            progress(20, "正在關閉執行中的 Stillfeather…");
            close_app()?;
        }
        // 剛關閉的 Stillfeather（例如更新提醒啟動了這個安裝程式後自己關閉）可能還沒完全結束，exe 仍被占用：稍等再試
        let mut last = first;
        let written = (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(250));
            match write_file_atomic(&exe, PAYLOAD) {
                Ok(()) => true,
                Err(e) => {
                    last = e;
                    false
                }
            }
        });
        if !written {
            return Err(last);
        }
    }

    // 「其他程式開啟檔案時也用 Stillfeather」的取代元件：放在 exe 旁邊。登記的是 Stillfeather 複製到
    // %LOCALAPPDATA% 的那一份（其他程式載入的是它），所以這裡的一份不會被占用，可以直接取代。
    if !PICKER_PAYLOAD.is_empty() {
        progress(32, "寫入開啟檔案視窗元件…");
        write_file_atomic(&paths::join(dir, feather_core::picker::DLL_NAME), PICKER_PAYLOAD)?;
    }

    progress(40, "寫入解除安裝程式…");
    let me = own_exe().ok_or("找不到安裝程式本身的位置。")?;
    let uninstaller = paths::join(dir, install::UNINSTALLER_NAME);
    if !install::same_path(&me, &uninstaller) {
        let tmp = format!("{uninstaller}.new");
        let (f, t) = (w(&me), w(&tmp));
        if unsafe { CopyFileW(f.as_ptr(), t.as_ptr(), 0) } == 0 {
            return Err(format!("無法寫入解除安裝程式（錯誤 {}）。", unsafe { GetLastError() }));
        }
        replace_file(&tmp, &uninstaller)?;
    }

    progress(55, "登記到「已安裝的應用程式」…");
    let size_kb = ((PAYLOAD.len() + PICKER_PAYLOAD.len() + std::fs::metadata(&me).map(|m| m.len() as usize).unwrap_or(0)) / 1024) as u32;
    let entry = install::uninstall_entry(dir, VERSION, size_kb, &today());
    for (name, data) in &entry.strings {
        shellreg::set_string(install::UNINSTALL_KEY, name, data)?;
    }
    for (name, data) in &entry.dwords {
        shellreg::set_dword(install::UNINSTALL_KEY, name, *data)?;
    }
    shellreg::set_string(install::APP_KEY, "InstallDir", dir)?;
    shellreg::set_string(install::APP_KEY, "Version", VERSION)?;

    progress(70, "建立捷徑…");
    let (start, desktop) = shortcut_paths();
    for (path, wanted) in [(start, opts.start_menu), (desktop, opts.desktop)] {
        let Some(path) = path else { continue };
        match wanted {
            Some(true) => create_shortcut(&path, &exe, dir)?,
            // 這次沒勾選：拿掉之前安裝留下的捷徑
            Some(false) => {
                delete_file(&path);
            }
            // 更新：使用者原本有的捷徑留著（指向同一個位置）
            None => {}
        }
    }

    progress(85, "設定預設檔案總管…");
    match opts.default_fm {
        Some(true) => {
            shellreg::set_default_file_manager(&exe)?;
            let _ = shellreg::set_default_fm_choice(true);
        }
        Some(false) => {
            if shellreg::is_default_file_manager(&exe) {
                shellreg::restore_default_file_manager()?;
            }
            let _ = shellreg::set_default_fm_choice(false);
        }
        // 更新：維持原狀。舊版裝的沒有記下選擇（沒選過時 Stillfeather 啟動會設為預設）：
        // 照目前的狀態記下，使用者當初沒勾就不會被改掉
        None => {
            if shellreg::default_fm_choice().is_none() {
                let _ = shellreg::set_default_fm_choice(shellreg::default_fm_program().is_some());
            }
        }
    }

    // 選用的功能：失敗不影響安裝（Stillfeather 每次啟動會依使用者的選擇再處理一次）
    progress(92, "設定開啟檔案視窗…");
    match opts.picker {
        Some(true) => {
            let _ = shellreg::set_picker_choice(true);
            if !PICKER_PAYLOAD.is_empty() {
                if let Ok(dll) = shellreg::deploy_picker(&paths::join(dir, feather_core::picker::DLL_NAME)) {
                    // 另一個程式已經取代了這個視窗時不覆寫
                    let _ = shellreg::enable_picker(&dll, &exe);
                }
            }
        }
        Some(false) => {
            let _ = shellreg::set_picker_choice(false);
            let _ = shellreg::disable_picker();
        }
        None => {}
    }
    // 背景攔截 Win+E（新版 Windows 11 需要；選項預設開啟，只在預設檔案總管是 Stillfeather 時執行）：登入時啟動
    shellreg::sync_win_e_autorun();
    shellreg::notify_assoc_changed();
    progress(100, "完成");
    Ok(())
}

/// 背景攔截 Win+E：要的話（選項開著、預設檔案總管是 Stillfeather）而且還沒執行就啟動，不必等到下次登入。
/// 以系統管理員身分執行時不啟動：它要以一般權限執行（登入或開啟 Stillfeather 時會啟動）。
pub fn start_win_e() {
    if is_elevated() || shellreg::win_e_window() != 0 {
        return;
    }
    let Some(exe) = shellreg::win_e_hook_exe() else { return };
    let dir = paths::parent(&exe).unwrap_or_default();
    if let Some(p) = spawn(&exe, &[feather_core::wine::WIN_E_ARG.to_string()], &dir) {
        close(p);
    }
}

/// 依權限決定在這個行程安裝，或以系統管理員身分重新執行自己來安裝。
pub fn install_with_elevation(opts: &InstallOptions, elevated: bool, progress: Progress) -> Result<(), String> {
    let result = install_in_place_or_elevated(opts, elevated, progress);
    if result.is_ok() {
        start_win_e();
    }
    result
}

fn install_in_place_or_elevated(opts: &InstallOptions, elevated: bool, progress: Progress) -> Result<(), String> {
    match check_writable(&opts.dir) {
        Access::Ok => install(opts, progress),
        Access::Failed(e) => Err(e),
        Access::NeedsAdmin if elevated || is_elevated() => Err("沒有權限寫入這個資料夾。".into()),
        Access::NeedsAdmin => {
            progress(10, "這個位置需要系統管理員權限，請在 Windows 的提示中允許…");
            let flag = |on: bool, yes: &str, no: &str| if on { yes.to_string() } else { no.to_string() };
            let mut args = vec!["--elevated".to_string(), "/S".to_string(), "--dir".to_string(), opts.dir.clone()];
            if let Some(d) = opts.default_fm {
                args.push(flag(d, "--default-explorer", "--no-default-explorer"));
            }
            if let Some(p) = opts.picker {
                args.push(flag(p, "--file-picker", "--no-file-picker"));
            }
            // 沒有指定的捷徑維持原狀（--update 讓靜默模式不套用預設值）
            if opts.desktop.is_none() || opts.start_menu.is_none() {
                args.push("--update".into());
            }
            if let Some(d) = opts.desktop {
                args.push(flag(d, "--desktop", "--no-desktop"));
            }
            if let Some(m) = opts.start_menu {
                args.push(flag(m, "--start-menu", "--no-start-menu"));
            }
            args.push("--no-launch".into());
            run_elevated(args)
        }
    }
}

// ---------------------------------------------------------------- 解除安裝

/// 解除安裝（在暫存資料夾的那一份執行）。回傳仍需在程式結束後刪除的檔案（例如還在執行的 Uninstall.exe）。
pub fn uninstall(dir: &str, progress: Progress) -> Result<Vec<String>, String> {
    progress(5, "正在關閉 Stillfeather…");
    close_app()?;
    // 背景攔截 Win+E：結束它（它鎖住 exe）並拿掉登入時的啟動
    shellreg::stop_win_e(5000);
    let _ = shellreg::set_win_e_autorun(None);

    progress(20, "改回 Windows 檔案總管…");
    let mut errors = Vec::new();
    if let Err(e) = shellreg::restore_default_file_manager() {
        errors.push(e);
    }
    // 其他程式改回 Windows 原本的開啟檔案視窗
    if let Err(e) = shellreg::disable_picker() {
        errors.push(e);
    }

    progress(35, "刪除捷徑…");
    let (start, desktop) = shortcut_paths();
    let exe = paths::join(dir, install::EXE_NAME);
    for lnk in [start, desktop].into_iter().flatten() {
        delete_file(&lnk);
    }

    progress(50, "刪除設定與記錄…");
    if let Some(base) = local_app_data() {
        // 目前的資料夾，以及舊版用的 %LOCALAPPDATA%\FeatherFiles（還沒被新版清掉時）
        for data in [install::data_dir(&base), install::old_data_dir(&base)] {
            move_loaded_picker(&data);
            if std::path::Path::new(&data).exists() {
                if let Err(e) = std::fs::remove_dir_all(&data) {
                    errors.push(format!("無法刪除 {data}：{e}"));
                }
            }
        }
    }
    // 預覽與效能量測留下的暫存資料夾
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // 快速預覽、效能量測，以及更新提醒下載的安裝程式
            if (name.starts_with("FeatherFiles-preview-") || name.starts_with("FeatherFiles-perf-") || name == "Stillfeather-update") && e.path().is_dir() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    progress(70, "刪除程式檔案…");
    let mut later = Vec::new();
    if !delete_file(&exe) {
        errors.push(format!("無法刪除 {exe}。"));
    }
    for name in install::GENERATED_FILES {
        delete_file(&paths::join(dir, name));
    }
    let _ = std::fs::remove_file(paths::join(dir, &format!("{}.new", install::EXE_NAME)));
    delete_file(&paths::join(dir, feather_core::picker::DLL_NAME));
    let uninstaller = paths::join(dir, install::UNINSTALLER_NAME);
    if !delete_file(&uninstaller) {
        later.push(uninstaller);
    }

    progress(85, "移除登錄項目…");
    for key in [install::UNINSTALL_KEY, install::APP_KEY] {
        if let Err(e) = shellreg::delete_tree(key) {
            errors.push(e);
        }
    }
    if later.is_empty() {
        let d = w(dir);
        unsafe {
            RemoveDirectoryW(d.as_ptr());
        }
    }
    progress(100, "完成");
    if errors.is_empty() {
        Ok(later)
    } else {
        Err(errors.join("\n"))
    }
}

/// 取代元件可能還載入在其他程式裡（刪不掉，資料夾也就刪不乾淨）：移到暫存資料夾
/// （Windows 允許改名使用中的 DLL），那些程式結束後由系統的磁碟清理處理。
fn move_loaded_picker(data_dir: &str) {
    let dir = paths::join(data_dir, install::PICKER_DIR_NAME);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    for (i, e) in entries.flatten().enumerate() {
        if std::fs::remove_file(e.path()).is_ok() {
            continue;
        }
        let target = std::env::temp_dir().join(format!("Stillfeather-old-{stamp}-{i}-{}", e.file_name().to_string_lossy()));
        let _ = std::fs::rename(e.path(), target);
    }
}

/// 解除安裝前的權限確認：安裝資料夾不能寫入時，以系統管理員身分重新執行。
pub fn uninstall_with_elevation(dir: &str, elevated: bool, progress: Progress) -> Result<Vec<String>, String> {
    let needs_admin = std::path::Path::new(dir).exists() && matches!(check_writable(dir), Access::NeedsAdmin);
    if needs_admin && !elevated && !is_elevated() {
        progress(10, "需要系統管理員權限才能刪除程式檔案，請在 Windows 的提示中允許…");
        run_elevated(vec!["--uninstall".into(), "--elevated".into(), "/S".into(), "--dir".into(), dir.to_string()])?;
        return Ok(Vec::new());
    }
    uninstall(dir, progress)
}

/// 從安裝資料夾裡的 Uninstall.exe 啟動：把自己複製到暫存資料夾再執行那一份。
/// 回傳 Some(結束代碼) 表示已交給暫存資料夾的那一份；None 表示複製失敗，就地解除安裝。
pub fn relaunch_from_temp(dir: &str, silent: bool) -> Option<i32> {
    let me = own_exe()?;
    let tmp_dir = paths::join(&std::env::temp_dir().to_string_lossy(), &format!("Stillfeather-uninstall-{}", unsafe { GetTickCount64() }));
    std::fs::create_dir_all(&tmp_dir).ok()?;
    let copy = paths::join(&tmp_dir, install::UNINSTALLER_NAME);
    std::fs::copy(&me, &copy).ok()?;
    let mut args = vec!["--uninstall".to_string(), "--dir".to_string(), dir.to_string()];
    if silent {
        args.push("/S".into());
    }
    let process = spawn(&copy, &args, &tmp_dir)?;
    // 靜默模式：部署工具要等結果；畫面模式：馬上結束，安裝資料夾裡就沒有執行中的檔案
    let code = if silent { wait(process) } else { close(process) };
    Some(code as i32)
}

/// 程式結束後才刪得掉的檔案與資料夾（交給 cmd.exe）。
pub fn delete_after_exit(files: &[String], dirs: &[String]) {
    if files.is_empty() && dirs.is_empty() {
        return;
    }
    let f: Vec<&str> = files.iter().map(String::as_str).collect();
    let d: Vec<&str> = dirs.iter().map(String::as_str).collect();
    let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let cmd = paths::join(&paths::join(&system, "System32"), "cmd.exe");
    let line = format!("{} {}", install::quoted(&cmd), install::delayed_delete_command(&f, &d));
    unsafe {
        let mut cl = w(&line);
        let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..std::mem::zeroed() };
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let app = w(&cmd);
        if CreateProcessW(app.as_ptr(), cl.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, CREATE_NO_WINDOW, std::ptr::null(), std::ptr::null(), &si, &mut pi) != 0 {
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
        }
    }
}

/// 暫存資料夾裡的這一份解除安裝程式：結束後刪掉自己與暫存資料夾。
pub fn delete_self_if_temp() {
    let Some(me) = own_exe() else { return };
    let Some(dir) = paths::parent(&me) else { return };
    if paths::file_name(&dir).starts_with("Stillfeather-uninstall-") {
        delete_after_exit(&[me], &[dir]);
    }
}

// ---------------------------------------------------------------- 行程

fn command_line(exe: &str, args: &[String]) -> String {
    let mut line = install::quote_arg(exe);
    for a in args {
        line.push(' ');
        line.push_str(&install::quote_arg(a));
    }
    line
}

fn spawn(exe: &str, args: &[String], cwd: &str) -> Option<HANDLE> {
    unsafe {
        let mut cl = w(&command_line(exe, args));
        let app = w(exe);
        let dir = w(cwd);
        let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..std::mem::zeroed() };
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        if CreateProcessW(app.as_ptr(), cl.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 0, 0, std::ptr::null(), dir.as_ptr(), &si, &mut pi) == 0 {
            return None;
        }
        CloseHandle(pi.hThread);
        Some(pi.hProcess)
    }
}

fn wait(process: HANDLE) -> u32 {
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(process, INFINITE);
        GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
    }
    code
}

fn close(process: HANDLE) -> u32 {
    unsafe {
        CloseHandle(process);
    }
    0
}

/// 以系統管理員身分執行自己（帶 --result=暫存檔），等它結束並讀回結果。
fn run_elevated(mut args: Vec<String>) -> Result<(), String> {
    let me = own_exe().ok_or("找不到安裝程式本身的位置。")?;
    let result_file = paths::join(&std::env::temp_dir().to_string_lossy(), &format!("Stillfeather-setup-result-{}-{}.txt", std::process::id(), unsafe { GetTickCount64() }));
    let _ = std::fs::write(&result_file, "未完成：以系統管理員身分執行的安裝程式沒有回報結果。");
    args.push(format!("--result={result_file}"));
    let params = args.iter().map(|a| install::quote_arg(a)).collect::<Vec<_>>().join(" ");
    let (verb, file, p) = (w("runas"), w(&me), w(&params));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = p.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess == 0 {
        let code = unsafe { GetLastError() };
        let _ = std::fs::remove_file(&result_file);
        return Err(if code == ERROR_CANCELLED { "需要系統管理員權限才能繼續，已取消。可以改裝到你的使用者資料夾（不需要權限）。".into() } else { format!("無法以系統管理員身分執行（錯誤 {code}）。") });
    }
    let code = wait(info.hProcess);
    let text = std::fs::read_to_string(&result_file).unwrap_or_default();
    let _ = std::fs::remove_file(&result_file);
    if code == 0 && text.trim().is_empty() {
        Ok(())
    } else if text.trim().is_empty() {
        Err(format!("以系統管理員身分執行的步驟失敗（代碼 {code}）。"))
    } else {
        Err(text.trim().to_string())
    }
}

/// 以系統管理員身分執行的那一份：把結果寫給等待中的那一份。
pub fn write_result(args: &SetupArgs, result: &Result<(), String>) {
    if let Some(path) = &args.result {
        let _ = std::fs::write(path, result.as_ref().err().map(String::as_str).unwrap_or(""));
    }
}

/// 開啟剛裝好的 Stillfeather。自己是以系統管理員身分執行時，交給檔案總管開啟，Stillfeather 才不會也變成系統管理員。
pub fn launch_app(dir: &str) {
    let exe = paths::join(dir, install::EXE_NAME);
    if is_elevated() {
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let explorer = paths::join(&system, "explorer.exe");
        if let Some(p) = spawn(&explorer, &[exe], dir) {
            close(p);
        }
    } else if let Some(p) = spawn(&exe, &[], dir) {
        close(p);
    }
}

// ---------------------------------------------------------------- 靜默模式

/// 不顯示畫面的安裝（/S）：預設設為預設檔案總管（不要的話加 `--no-default-explorer`）、不建桌面捷徑（要的話加參數），結束代碼 0 表示成功。
/// 加上 `--update` 時沒有指定的選項一律維持原狀，並在完成後開啟 Stillfeather（除非 `--no-launch`）。
pub fn silent_install(a: &SetupArgs) -> i32 {
    let result = (|| {
        let raw = a.dir.clone().unwrap_or_else(default_install_dir);
        let prot = protected_dirs();
        let prot: Vec<&str> = prot.iter().map(String::as_str).collect();
        let dir = install::normalize_install_dir(&raw, &prot)?;
        let opts = if a.update {
            InstallOptions { dir: dir.clone(), default_fm: a.default_fm, picker: a.file_picker, desktop: a.desktop, start_menu: a.start_menu }
        } else {
            InstallOptions { dir: dir.clone(), default_fm: Some(a.default_fm.unwrap_or(true)), picker: a.file_picker, desktop: Some(a.desktop.unwrap_or(false)), start_menu: Some(a.start_menu.unwrap_or(true)) }
        };
        install_with_elevation(&opts, a.elevated, &mut |_, _| {})?;
        if a.launch.unwrap_or(a.update) {
            launch_app(&dir);
        }
        Ok(())
    })();
    write_result(a, &result);
    if result.is_ok() {
        0
    } else {
        1
    }
}

pub fn silent_uninstall(a: &SetupArgs, dir: &str) -> i32 {
    let result = uninstall_with_elevation(dir, a.elevated, &mut |_, _| {});
    let outcome = result.as_ref().map(|_| ()).map_err(Clone::clone);
    write_result(a, &outcome);
    if let Ok(later) = &result {
        // 安裝資料夾裡的 Uninstall.exe 還在執行（部署工具在等它）：結束後再刪
        let dirs = if later.is_empty() { Vec::new() } else { vec![dir.to_string()] };
        delete_after_exit(later, &dirs);
    }
    if !a.elevated {
        delete_self_if_temp();
    }
    if result.is_ok() {
        0
    } else {
        1
    }
}

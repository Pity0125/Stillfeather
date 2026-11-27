//! 登錄（目前使用者 HKCU）：設為／取消預設檔案總管（Win+E 與開啟資料夾）、背景攔截 Win+E 的登入啟動與停止，
//! 以及安裝程式用到的基本讀寫。
//!
//! Stillfeather 本體（設定對話方塊）與安裝程式共用這個檔案：兩邊都以模組引入，只依賴同層的 `sys`。
//! 要寫的機碼與值、哪些值要備份還原，都由 `feather_core::install` 決定（有單元測試）。

use super::sys::*;
use feather_core::install::{self, RegValue};
use feather_core::paths;
use feather_core::wine;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn name_ptr(name: &[u16]) -> *const u16 {
    // 只有結尾的 0：機碼的預設值
    if name.len() <= 1 {
        std::ptr::null()
    } else {
        name.as_ptr()
    }
}

fn reg_error(code: i32) -> String {
    match code as u32 {
        5 => tr!("沒有權限修改登錄。", "You don't have permission to change the registry.").into(),
        c => trf!("登錄錯誤（代碼 {c}）。", "Registry error (code {c})."),
    }
}

/// 讀取字串值；不存在時回傳 None。`name` 為空字串表示預設值。
pub fn read_string(key: &str, name: &str) -> Option<String> {
    let k = w(key);
    let n = w(name);
    unsafe {
        let mut size = 0u32;
        let rc = RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), name_ptr(&n), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size);
        if rc != 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize / 2).max(1)];
        let mut size = (buf.len() * 2) as u32;
        let rc = RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), name_ptr(&n), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size);
        if rc != 0 {
            return None;
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}

fn create(key: &str) -> Result<HKEY, String> {
    let k = w(key);
    let mut h: HKEY = 0;
    let rc = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, std::ptr::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE | KEY_QUERY_VALUE, std::ptr::null(), &mut h, std::ptr::null_mut()) };
    if rc == 0 {
        Ok(h)
    } else {
        Err(reg_error(rc))
    }
}

/// 寫入字串值（機碼不存在時建立）。
pub fn set_string(key: &str, name: &str, data: &str) -> Result<(), String> {
    let h = create(key)?;
    let n = w(name);
    let d = w(data);
    let rc = unsafe { RegSetValueExW(h, name_ptr(&n), 0, REG_SZ, d.as_ptr().cast(), (d.len() * 2) as u32) };
    unsafe {
        RegCloseKey(h);
    }
    if rc == 0 {
        Ok(())
    } else {
        Err(reg_error(rc))
    }
}

/// 只有安裝程式用到（解除安裝資訊的數值，見 setup/ops.rs）
#[allow(dead_code)]
pub fn set_dword(key: &str, name: &str, data: u32) -> Result<(), String> {
    let h = create(key)?;
    let n = w(name);
    let bytes = data.to_le_bytes();
    let rc = unsafe { RegSetValueExW(h, name_ptr(&n), 0, REG_DWORD, bytes.as_ptr(), 4) };
    unsafe {
        RegCloseKey(h);
    }
    if rc == 0 {
        Ok(())
    } else {
        Err(reg_error(rc))
    }
}

/// 刪除一個值；本來就沒有也算成功。
pub fn delete_value(key: &str, name: &str) -> Result<(), String> {
    let k = w(key);
    let mut h: HKEY = 0;
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_SET_VALUE, &mut h) };
    if rc as u32 == ERROR_FILE_NOT_FOUND || rc as u32 == ERROR_PATH_NOT_FOUND {
        return Ok(());
    }
    if rc != 0 {
        return Err(reg_error(rc));
    }
    let n = w(name);
    let rc = unsafe { RegDeleteValueW(h, name_ptr(&n)) };
    unsafe {
        RegCloseKey(h);
    }
    if rc == 0 || rc as u32 == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(reg_error(rc))
    }
}

/// 刪除整個機碼（含子機碼與值）；本來就沒有也算成功。
pub fn delete_tree(key: &str) -> Result<(), String> {
    let k = w(key);
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc != 0 && rc as u32 != ERROR_FILE_NOT_FOUND && rc as u32 != ERROR_PATH_NOT_FOUND {
        return Err(reg_error(rc));
    }
    // RegDeleteTreeW 有時只清空內容，機碼本身再刪一次
    let rc = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc == 0 || rc as u32 == ERROR_FILE_NOT_FOUND || rc as u32 == ERROR_PATH_NOT_FOUND {
        Ok(())
    } else {
        Err(reg_error(rc))
    }
}

/// 機碼存在、而且沒有任何值與子機碼時刪除它。
pub fn delete_key_if_empty(key: &str) {
    let k = w(key);
    unsafe {
        let mut h: HKEY = 0;
        if RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_READ, &mut h) != 0 {
            return;
        }
        let (mut subkeys, mut values) = (0u32, 0u32);
        let rc = RegQueryInfoKeyW(
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut subkeys,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut values,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        RegCloseKey(h);
        if rc == 0 && subkeys == 0 && values == 0 {
            RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr());
        }
    }
}

/// 通知檔案總管關聯已改變（不必登出就生效）。
pub fn notify_assoc_changed() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, std::ptr::null(), std::ptr::null());
    }
}

// ---------------------------------------------------------------- 預設檔案總管

/// 命令是不是啟動某一份 Stillfeather（安裝版 Stillfeather.exe 或開發版 FeatherFiles.exe，不論位置）。
fn is_any_stillfeather(cmd: &str) -> bool {
    let name = paths::file_name(install::command_program(cmd));
    name.eq_ignore_ascii_case(install::EXE_NAME) || name.eq_ignore_ascii_case("FeatherFiles.exe")
}

/// 這個共用值目前是不是 Stillfeather 寫的。
fn is_ours(key: &str, name: &str, current: &str) -> bool {
    if key == install::win_e_key() {
        if name.is_empty() {
            return is_any_stillfeather(current);
        }
        // DelegateExecute：命令是 Stillfeather 時才是我們的空值
        return current.is_empty() && read_string(key, "").is_some_and(|c| is_any_stillfeather(&c));
    }
    current == install::VERB
}

/// Stillfeather（這個 exe）目前是不是預設檔案總管：開啟資料夾與 Win+E 都指向它。
pub fn is_default_file_manager(exe: &str) -> bool {
    let folders = read_string(r"Software\Classes\Directory\shell", "").as_deref() == Some(install::VERB)
        && read_string(&format!(r"Software\Classes\Directory\shell\{}\command", install::VERB), "").is_some_and(|c| install::command_targets(&c, exe));
    let wine = read_string(&install::win_e_key(), "").is_some_and(|c| install::command_targets(&c, exe));
    folders && wine
}

/// 目前登記為預設檔案總管的 Stillfeather（任何一份）的程式路徑；預設檔案總管不是 Stillfeather 時是 None。
pub fn default_fm_program() -> Option<String> {
    let folders = (read_string(r"Software\Classes\Directory\shell", "").as_deref() == Some(install::VERB))
        .then(|| read_string(&format!(r"Software\Classes\Directory\shell\{}\command", install::VERB), ""))
        .flatten();
    let cmd = folders.or_else(|| read_string(&install::win_e_key(), "").filter(|c| is_any_stillfeather(c)))?;
    let program = install::command_program(&cmd);
    (!program.is_empty()).then(|| program.to_string())
}

/// 使用者對「設為預設檔案總管」的選擇（"1"／"0"）；沒選過是 None（預設開啟，見 install::default_fm_on_start）
pub fn default_fm_choice() -> Option<String> {
    read_string(install::APP_KEY, install::DEFAULT_FM_CHOICE_VALUE)
}

/// 記下使用者的選擇（設定或安裝程式的勾選）
pub fn set_default_fm_choice(on: bool) -> Result<(), String> {
    set_string(install::APP_KEY, install::DEFAULT_FM_CHOICE_VALUE, if on { "1" } else { "0" })
}

/// 設為預設檔案總管。覆寫其他程式設定的共用值之前先備份，取消時還原。
pub fn set_default_file_manager(exe: &str) -> Result<(), String> {
    for (key, name) in install::shared_values() {
        let backup = install::backup_name(&key, &name);
        if read_string(install::BACKUP_KEY, &backup).is_some() {
            continue; // 已經備份過（例如重新安裝），不要把自己的值當成備份
        }
        if let Some(current) = read_string(&key, &name) {
            if !is_ours(&key, &name, &current) {
                set_string(install::BACKUP_KEY, &backup, &current)?;
            }
        }
    }
    for RegValue { key, name, data } in install::default_fm_values(exe) {
        set_string(&key, &name, &data)?;
    }
    notify_assoc_changed();
    Ok(())
}

/// 取消預設檔案總管，改回 Windows 檔案總管（或安裝前的設定）。不論 Stillfeather 裝在哪裡都會清掉它的設定。
pub fn restore_default_file_manager() -> Result<(), String> {
    let mut first_error = None;
    let mut note = |r: Result<(), String>| {
        if let Err(e) = r {
            first_error.get_or_insert(e);
        }
    };
    // 先判斷每個值是不是我們的，再一起改（DelegateExecute 要看改之前的命令）
    let decisions: Vec<(String, String, bool, Option<String>)> = install::shared_values()
        .into_iter()
        .map(|(key, name)| {
            let ours = read_string(&key, &name).is_some_and(|c| is_ours(&key, &name, &c));
            let saved = read_string(install::BACKUP_KEY, &install::backup_name(&key, &name));
            (key, name, ours, saved)
        })
        .collect();
    for (key, name, ours, saved) in decisions {
        if ours {
            note(match saved {
                Some(old) => set_string(&key, &name, &old),
                None => delete_value(&key, &name),
            });
        }
    }
    for key in install::owned_keys() {
        note(delete_tree(&key));
    }
    for key in install::prune_keys() {
        delete_key_if_empty(&key);
    }
    note(delete_tree(install::BACKUP_KEY));
    delete_key_if_empty(install::APP_KEY);
    notify_assoc_changed();
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------- 背景攔截 Win+E

/// 目前的使用者名稱，去掉不能放進物件名稱的字元（例如反斜線）。主視窗類別、單一執行個體的互斥鎖、
/// 背景攔截 Win+E 的視窗與安裝程式找主視窗都用這個，名稱才會一致。
pub fn user_tag() -> String {
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

/// 使用者要不要攔截 Win+E（沒選過時預設開啟，見 feather_core::wine）
pub fn win_e_choice() -> bool {
    wine::wanted(read_string(install::APP_KEY, wine::CHOICE_VALUE).as_deref())
}

pub fn set_win_e_choice(on: bool) -> Result<(), String> {
    set_string(install::APP_KEY, wine::CHOICE_VALUE, if on { "1" } else { "0" })
}

/// 背景攔截 Win+E 要不要執行、由哪一份 Stillfeather 執行：使用者沒關掉（預設開啟），
/// 而且預設檔案總管是某一份 Stillfeather（程式還在）。回傳那一份的路徑。
pub fn win_e_hook_exe() -> Option<String> {
    if !win_e_choice() {
        return None;
    }
    let exe = default_fm_program()?;
    std::path::Path::new(&exe).is_file().then_some(exe)
}

/// 登入時啟動背景攔截：`Some` 寫入指向那一份的命令；`None` 拿掉（只拿掉 Stillfeather 自己的）
pub fn set_win_e_autorun(exe: Option<&str>) -> Result<(), String> {
    match exe {
        Some(exe) => {
            let cmd = wine::run_command(exe);
            if read_string(wine::RUN_KEY, wine::RUN_VALUE).as_deref() == Some(cmd.as_str()) {
                return Ok(());
            }
            set_string(wine::RUN_KEY, wine::RUN_VALUE, &cmd)
        }
        None => match read_string(wine::RUN_KEY, wine::RUN_VALUE) {
            Some(cmd) if wine::is_run_command(&cmd) => delete_value(wine::RUN_KEY, wine::RUN_VALUE),
            _ => Ok(()),
        },
    }
}

/// 依目前的選擇與預設檔案總管整理登入時的啟動命令
pub fn sync_win_e_autorun() {
    let _ = set_win_e_autorun(win_e_hook_exe().as_deref());
}

/// 執行中的背景攔截（它的訊息視窗）；沒有執行時是 0
pub fn win_e_window() -> HWND {
    let class = w(&wine::window_class(&user_tag()));
    unsafe { FindWindowExW(HWND_MESSAGE, 0, class.as_ptr(), std::ptr::null()) }
}

/// 請背景攔截結束，最多等 `wait_ms` 毫秒。沒有在執行或已經結束時回傳 true。
pub fn stop_win_e(wait_ms: u32) -> bool {
    let hwnd = win_e_window();
    if hwnd == 0 {
        return true;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(SYNCHRONIZE, 0, pid);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        if process == 0 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            return win_e_window() == 0;
        }
        let done = WaitForSingleObject(process, wait_ms) == WAIT_OBJECT_0;
        CloseHandle(process);
        done
    }
}

// ---------------------------------------------------------------- 其他程式的「開啟檔案」「另存新檔」視窗

/// 目前登記在目前使用者底下的「開啟檔案」視窗元件（沒有登記時是 None：用的是 Windows 原本的）
pub fn picker_registered_dll() -> Option<String> {
    read_string(&install::picker_server_key(), "")
}

/// 其他程式開啟檔案時會不會跳出 Stillfeather
pub fn picker_enabled() -> bool {
    picker_registered_dll().is_some_and(|p| install::is_picker_dll(&p))
}

/// 登記 Stillfeather 的取代元件（`dll`，「開啟檔案」視窗），並記下它要啟動的 Stillfeather（`exe`）。
/// 「另存新檔」視窗登記旁邊另一個檔名的那一份（deploy_picker 一起放好的，見 install::picker_save_server_key）。
/// 已經有別的程式取代了開啟檔案視窗時不覆寫，回報錯誤；只有另存新檔視窗被別的程式取代時，只登記開啟檔案。
pub fn enable_picker(dll: &str, exe: &str) -> Result<(), String> {
    if let Some(current) = picker_registered_dll() {
        if !install::is_picker_dll(&current) {
            return Err(trf!("另一個程式已經取代了 Windows 的開啟檔案視窗（{current}），為了不影響它，沒有變更。", "Another program has already replaced the Windows Open dialog ({current}); it was left unchanged so that program keeps working."));
        }
    }
    set_string(install::APP_KEY, install::PICKER_EXE_VALUE, exe)?;
    let server = install::picker_server_key();
    set_string(&server, "", dll)?;
    set_string(&server, "ThreadingModel", "Apartment")?;
    let save_dll = paths::parent(dll).map(|d| paths::join(&d, feather_core::picker::SAVE_DLL_NAME));
    let save_free = read_string(&install::picker_save_server_key(), "").is_none_or(|p| install::is_picker_dll(&p));
    if let Some(save_dll) = save_dll.filter(|p| save_free && std::path::Path::new(p).is_file()) {
        let save = install::picker_save_server_key();
        // 失敗不影響開啟檔案：存檔時照樣是 Windows 的視窗
        if set_string(&save, "", &save_dll).and_then(|_| set_string(&save, "ThreadingModel", "Apartment")).is_err() {
            let _ = delete_tree(&save);
        }
    }
    Ok(())
}

/// 另存新檔視窗也登記成 Stillfeather 的了嗎（診斷用）
pub fn picker_save_enabled() -> bool {
    read_string(&install::picker_save_server_key(), "").is_some_and(|p| install::is_picker_dll(&p))
}

/// 使用者要不要這個功能（沒選過時預設關閉，見 install::picker_wanted）
pub fn picker_choice() -> bool {
    install::picker_wanted(read_string(install::APP_KEY, install::PICKER_CHOICE_VALUE).as_deref())
}

/// 記下使用者的選擇（設定或安裝程式的勾選）
pub fn set_picker_choice(on: bool) -> Result<(), String> {
    set_string(install::APP_KEY, install::PICKER_CHOICE_VALUE, if on { "1" } else { "0" })
}

/// 實際登記的那一份的位置：%LOCALAPPDATA%\Stillfeather\Picker\StillfeatherPicker.dll（見 install::PICKER_DIR_NAME）
pub fn deployed_picker_path() -> Option<String> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let dir = paths::join(&install::data_dir(&base), install::PICKER_DIR_NAME);
    Some(paths::join(&dir, feather_core::picker::DLL_NAME))
}

/// 把程式資料夾裡的取代元件（`src`）複製到登記的位置（內容相同就不動），回傳它的路徑。
/// 旁邊再放一份另存新檔用的（同樣的內容、另一個檔名；放不好時另存新檔照樣用 Windows 的視窗）。
pub fn deploy_picker(src: &str) -> Result<String, String> {
    let dst = deployed_picker_path().ok_or(tr!("找不到 %LOCALAPPDATA%。", "Can't find %LOCALAPPDATA%."))?;
    let dir = paths::parent(&dst).ok_or(tr!("位置無效。", "Invalid location."))?;
    std::fs::create_dir_all(&dir).map_err(|e| trf!("無法建立 {dir}：{e}", "Couldn't create {dir}: {e}"))?;
    let data = std::fs::read(src).map_err(|e| trf!("無法讀取 {src}：{e}", "Couldn't read {src}: {e}"))?;
    deploy_copy(&data, &dst)?;
    let _ = deploy_copy(&data, &paths::join(&dir, feather_core::picker::SAVE_DLL_NAME));
    remove_old_picker_copies(&dir);
    Ok(dst)
}

/// 寫一份取代元件。舊的那一份可能正載入在其他程式裡（無法覆寫或刪除）：先改名讓出位置，那些程式結束後下次再清掉。
fn deploy_copy(data: &[u8], dst: &str) -> Result<(), String> {
    if std::fs::read(dst).is_ok_and(|old| old == data) {
        return Ok(());
    }
    if std::path::Path::new(dst).exists() && std::fs::remove_file(dst).is_err() {
        let old = format!("{dst}.old-{}", unsafe { GetTickCount64() });
        std::fs::rename(dst, &old).map_err(|e| trf!("無法更新 {dst}（正被其他程式使用）：{e}", "Couldn't update {dst} (it's being used by another program): {e}"))?;
    }
    std::fs::write(dst, data).map_err(|e| trf!("無法寫入 {dst}：{e}", "Couldn't write {dst}: {e}"))
}

/// 刪掉之前改名讓位的舊版（還在使用中的刪不掉，下次再試）
fn remove_old_picker_copies(dir: &str) {
    let prefixes = [feather_core::picker::DLL_NAME, feather_core::picker::SAVE_DLL_NAME].map(|n| format!("{n}.old-").to_lowercase());
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if prefixes.iter().any(|p| name.starts_with(p)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// 取消登記，其他程式恢復使用 Windows 原本的視窗。只拿掉 Stillfeather 寫的部分。
pub fn disable_picker() -> Result<(), String> {
    let mut result = Ok(());
    if picker_enabled() {
        result = delete_tree(&install::picker_server_key());
        delete_key_if_empty(&install::picker_clsid_key());
    }
    if picker_save_enabled() {
        let r = delete_tree(&install::picker_save_server_key());
        delete_key_if_empty(&install::picker_save_clsid_key());
        result = result.and(r);
    }
    let _ = delete_value(install::APP_KEY, install::PICKER_EXE_VALUE);
    delete_key_if_empty(install::APP_KEY);
    result
}

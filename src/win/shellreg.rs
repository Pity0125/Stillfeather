//! 登錄（目前使用者 HKCU）：設為／取消預設檔案總管（Win+E 與開啟資料夾），以及安裝程式用到的基本讀寫。
//!
//! Stillfeather 本體（設定對話方塊）與安裝程式共用這個檔案：兩邊都以模組引入，只依賴同層的 `sys`。
//! 要寫的機碼與值、哪些值要備份還原，都由 `feather_core::install` 決定（有單元測試）。

#![allow(dead_code)]

use super::sys::*;
use feather_core::install::{self, RegValue};
use feather_core::paths;

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
        5 => "沒有權限修改登錄。".into(),
        c => format!("登錄錯誤（代碼 {c}）。"),
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

pub fn key_exists(key: &str) -> bool {
    let k = w(key);
    let mut h: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, k.as_ptr(), 0, KEY_READ, &mut h) == 0 {
            RegCloseKey(h);
            true
        } else {
            false
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

// ---------------------------------------------------------------- 其他程式的「開啟檔案」視窗

/// 目前登記在目前使用者底下的「開啟檔案」視窗元件（沒有登記時是 None：用的是 Windows 原本的）
pub fn picker_registered_dll() -> Option<String> {
    read_string(&install::picker_server_key(), "")
}

/// 其他程式開啟檔案時會不會跳出 Stillfeather
pub fn picker_enabled() -> bool {
    picker_registered_dll().is_some_and(|p| install::is_picker_dll(&p))
}

/// 登記 Stillfeather 的取代元件（`dll`），並記下它要啟動的 Stillfeather（`exe`）。
/// 已經有別的程式取代了這個視窗時不覆寫，回報錯誤。
pub fn enable_picker(dll: &str, exe: &str) -> Result<(), String> {
    if let Some(current) = picker_registered_dll() {
        if !install::is_picker_dll(&current) {
            return Err(format!("另一個程式已經取代了 Windows 的開啟檔案視窗（{current}），為了不影響它，沒有變更。"));
        }
    }
    set_string(install::APP_KEY, install::PICKER_EXE_VALUE, exe)?;
    let server = install::picker_server_key();
    set_string(&server, "", dll)?;
    set_string(&server, "ThreadingModel", "Apartment")
}

/// 取消登記，其他程式恢復使用 Windows 原本的視窗。只拿掉 Stillfeather 寫的部分。
pub fn disable_picker() -> Result<(), String> {
    let mut result = Ok(());
    if picker_enabled() {
        result = delete_tree(&install::picker_server_key());
        delete_key_if_empty(&install::picker_clsid_key());
    }
    let _ = delete_value(install::APP_KEY, install::PICKER_EXE_VALUE);
    delete_key_if_empty(install::APP_KEY);
    result
}

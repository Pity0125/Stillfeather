//! 安裝、解除安裝與「預設檔案總管」用到的純邏輯：要寫哪些登錄值、安裝路徑檢查、命令列參數。
//! 不呼叫 Win32，可以在任何平台測試；實際讀寫登錄與檔案在 `src/win/shellreg.rs` 與 `src/setup/`。
//!
//! 預設檔案總管只寫在目前使用者底下（HKCU），不需要系統管理員權限：
//! - Win+E：`Software\Classes\CLSID\{52205fd8-…}\shell\opennewwindow\command`（「本機」的開新視窗動詞），
//!   預設值改成 Stillfeather，並加上空的 `DelegateExecute` 蓋過系統的委派執行。
//! - 開啟資料夾與磁碟：`Software\Classes\Directory\shell`、`Drive\shell` 的預設動詞改成 `Stillfeather`
//!   （另外新增這個動詞；系統原本的「開啟」動詞不動）。
//!
//! 會被其他程式共用的值（例如另一套檔案管理程式也改過預設動詞）在覆寫前先備份，移除時還原；
//! 原本沒有的就刪除，不會留下空機碼。

use crate::paths;

pub const APP_NAME: &str = "Stillfeather";
pub const EXE_NAME: &str = "Stillfeather.exe";
pub const UNINSTALLER_NAME: &str = "Uninstall.exe";
/// 程式執行時可能產生在安裝資料夾裡的記錄檔（開發版才會寫在 exe 旁；解除安裝時一併刪除）
pub const GENERATED_FILES: &[&str] = &["crash.log", "trace.log", "stderr.log", "picker-crash.log", "picker-trace.log", "picker-stderr.log"];
/// 「新增或移除程式」的項目
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Stillfeather";
/// Stillfeather 自己的登錄機碼：安裝位置、使用者的選擇與預設檔案總管的備份
pub const APP_KEY: &str = r"Software\Stillfeather";
pub const BACKUP_KEY: &str = r"Software\Stillfeather\DefaultFileManagerBackup";
/// 設定、資料夾檢視記錄與記錄檔所在的資料夾名稱（%LOCALAPPDATA% 底下）
pub const DATA_DIR_NAME: &str = "FeatherFiles";
/// 資料夾右鍵選單的動詞名稱與文字
pub const VERB: &str = "Stillfeather";
pub const VERB_TEXT: &str = "以 Stillfeather 開啟";
/// 「本機」：Win+E 開啟的位置
pub const WIN_E_CLSID: &str = "{52205fd8-5dfb-447d-801a-d0b52f2e83e1}";
/// Win+E 傳給 Stillfeather 的參數：在新分頁開啟首頁
pub const OPEN_HOME_ARG: &str = "--open-home";
/// 經單一實例轉送時代表「開首頁」的項目（不會是真的路徑）
pub const HOME_MARKER: &str = "::home";

/// 「設為預設檔案總管」的選擇（[`APP_KEY`] 底下的值）："1" 開、"0" 關；沒有這個值表示沒選過，預設開啟
pub const DEFAULT_FM_CHOICE_VALUE: &str = "DefaultExplorer";

/// 主程式啟動時對預設檔案總管要做的事（見 [`default_fm_on_start`]）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultFmStart {
    /// 不動
    Keep,
    /// 只記下「開」（已經有一份 Stillfeather 是預設檔案總管）
    Remember,
    /// 把這一份設為預設檔案總管，並記下「開」
    Take,
}

/// 主程式啟動時依使用者的選擇決定怎麼處理預設檔案總管。
/// `registered`：目前登記為預設檔案總管的 Stillfeather（任何一份）的程式還在不在；沒有登記任何一份是 None。
/// - 沒選過：預設開啟。已經有一份是預設檔案總管就只記下選擇（不搶另一份的，例如安裝版）；否則把這一份設為預設。
/// - 開：登記的那一份程式不見了（被移走或刪掉）才改成這一份；沒有登記（之後改用別的檔案管理程式）時不搶回來。
/// - 關：不動（關掉時已經改回）。
pub fn default_fm_on_start(choice: Option<&str>, registered: Option<bool>) -> DefaultFmStart {
    match choice.map(str::trim) {
        None => match registered {
            Some(true) => DefaultFmStart::Remember,
            _ => DefaultFmStart::Take,
        },
        Some("0") => DefaultFmStart::Keep,
        Some(_) => match registered {
            Some(false) => DefaultFmStart::Take,
            _ => DefaultFmStart::Keep,
        },
    }
}

/// 「其他程式開啟檔案時也用 Stillfeather」：取代元件啟動哪一個 Stillfeather（[`APP_KEY`] 底下的值）
pub const PICKER_EXE_VALUE: &str = "PickerExe";
/// 使用者的選擇（[`APP_KEY`] 底下的值）："1" 開、"0" 關；沒有這個值表示沒選過，預設開啟
pub const PICKER_CHOICE_VALUE: &str = "FilePicker";

/// 要不要讓其他程式開啟檔案時跳出 Stillfeather：預設開啟，使用者關掉（"0"）才不要
pub fn picker_wanted(choice: Option<&str>) -> bool {
    choice.is_none_or(|c| c.trim() != "0")
}
/// 取代元件實際登記的那一份放在 %LOCALAPPDATA%\FeatherFiles\Picker：
/// 不登記程式資料夾裡的那一份，程式資料夾被移走或刪掉時，其他程式的開檔視窗仍然正常（找不到 Stillfeather 就用 Windows 的視窗）。
pub const PICKER_DIR_NAME: &str = "Picker";

/// 目前使用者的「開啟檔案」視窗元件登記（蓋過 HKLM 的 comdlg32.dll）
pub fn picker_clsid_key() -> String {
    format!(r"Software\Classes\CLSID\{}", crate::picker::CLSID_FILE_OPEN_DIALOG)
}

pub fn picker_server_key() -> String {
    format!(r"{}\InprocServer32", picker_clsid_key())
}

/// 登記的元件是不是 Stillfeather 的（不論放在哪裡）
pub fn is_picker_dll(path: &str) -> bool {
    paths::file_name(path).eq_ignore_ascii_case(crate::picker::DLL_NAME)
}

/// 要寫入的字串登錄值（`name` 為空字串表示機碼的預設值）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegValue {
    pub key: String,
    pub name: String,
    pub data: String,
}

fn value(key: &str, name: &str, data: String) -> RegValue {
    RegValue { key: key.to_string(), name: name.to_string(), data }
}

pub fn win_e_key() -> String {
    format!(r"Software\Classes\CLSID\{WIN_E_CLSID}\shell\opennewwindow\command")
}

const FOLDER_CLASSES: [&str; 2] = [r"Software\Classes\Directory\shell", r"Software\Classes\Drive\shell"];

/// 加上引號的 exe 路徑
pub fn quoted(exe: &str) -> String {
    format!("\"{exe}\"")
}

pub fn folder_command(exe: &str) -> String {
    format!("{} \"%1\"", quoted(exe))
}

pub fn win_e_command(exe: &str) -> String {
    format!("{} {OPEN_HOME_ARG}", quoted(exe))
}

/// 設為預設檔案總管時要寫的值（依寫入順序：先建好動詞，最後才把它設為預設）。
pub fn default_fm_values(exe: &str) -> Vec<RegValue> {
    let mut v = Vec::new();
    for class in FOLDER_CLASSES {
        let verb = format!(r"{class}\{VERB}");
        v.push(value(&verb, "", VERB_TEXT.to_string()));
        v.push(value(&verb, "Icon", format!("{exe},0")));
        v.push(value(&format!(r"{verb}\command"), "", folder_command(exe)));
    }
    let wine = win_e_key();
    v.push(value(&wine, "DelegateExecute", String::new()));
    v.push(value(&wine, "", win_e_command(exe)));
    for class in FOLDER_CLASSES {
        v.push(value(class, "", VERB.to_string()));
    }
    v
}

/// 會被其他程式共用的值：覆寫前備份、移除時還原（沒有備份就刪除）。
pub fn shared_values() -> Vec<(String, String)> {
    let wine = win_e_key();
    vec![
        (FOLDER_CLASSES[0].to_string(), String::new()),
        (FOLDER_CLASSES[1].to_string(), String::new()),
        (wine.clone(), String::new()),
        (wine, "DelegateExecute".to_string()),
    ]
}

/// 完全屬於 Stillfeather 的機碼：移除時整個刪掉。
pub fn owned_keys() -> Vec<String> {
    FOLDER_CLASSES.iter().map(|c| format!(r"{c}\{VERB}")).collect()
}

/// 移除後若變成空的（沒有值也沒有子機碼）就刪除的機碼，由深到淺。
pub fn prune_keys() -> Vec<String> {
    let clsid = format!(r"Software\Classes\CLSID\{WIN_E_CLSID}");
    vec![
        win_e_key(),
        format!(r"{clsid}\shell\opennewwindow"),
        format!(r"{clsid}\shell"),
        clsid,
        FOLDER_CLASSES[0].to_string(),
        FOLDER_CLASSES[1].to_string(),
        r"Software\Classes\Directory".to_string(),
        r"Software\Classes\Drive".to_string(),
    ]
}

/// 備份值在 [`BACKUP_KEY`] 底下的名稱
pub fn backup_name(key: &str, name: &str) -> String {
    format!("{key}|{name}")
}

/// 命令列的第一個參數（程式路徑）：有引號就取引號內，否則取到第一個空白。
pub fn command_program(cmd: &str) -> &str {
    let c = cmd.trim_start();
    if let Some(rest) = c.strip_prefix('"') {
        return rest.split('"').next().unwrap_or("");
    }
    c.split_whitespace().next().unwrap_or("")
}

/// 兩個 Windows 路徑是否相同（不分大小寫，斜線方向與結尾分隔符號不計）。
pub fn same_path(a: &str, b: &str) -> bool {
    let n = |p: &str| paths::trim_end_sep(&paths::normalize_separators(p.trim())).to_lowercase();
    n(a) == n(b)
}

/// 這個命令是不是啟動 `exe`
pub fn command_targets(cmd: &str, exe: &str) -> bool {
    let program = command_program(cmd);
    !program.is_empty() && same_path(program, exe)
}

// ---------------------------------------------------------------- 安裝位置

/// 預設安裝位置：%LOCALAPPDATA%\Programs\Stillfeather（目前使用者，不需要系統管理員權限）
pub fn default_install_dir(local_app_data: &str) -> String {
    paths::join(&paths::join(local_app_data, "Programs"), APP_NAME)
}

/// 從「瀏覽」選到的資料夾：還不是 Stillfeather 資料夾就在底下建一個，避免把檔案散在別的資料夾裡。
pub fn with_app_folder(picked: &str) -> String {
    let p = paths::trim_end_sep(picked.trim());
    if paths::file_name(p).eq_ignore_ascii_case(APP_NAME) {
        p.to_string()
    } else {
        paths::join(p, APP_NAME)
    }
}

/// 檢查並整理使用者輸入的安裝位置。`protected` 是不能安裝進去的資料夾（例如 Windows 資料夾）。
pub fn normalize_install_dir(input: &str, protected: &[&str]) -> Result<String, String> {
    let p = input.trim().trim_matches('"').trim();
    if p.is_empty() {
        return Err("請輸入安裝位置。".into());
    }
    if !paths::is_fully_qualified(p) {
        return Err("安裝位置必須是完整路徑，例如 C:\\Users\\你的名字\\AppData\\Local\\Programs\\Stillfeather。".into());
    }
    if p.chars().enumerate().any(|(i, c)| matches!(c, '<' | '>' | '"' | '|' | '?' | '*') || (c == ':' && i != 1) || (c as u32) < 32) {
        return Err("安裝位置含有不能用在資料夾名稱的字元（< > : \" | ? *）。".into());
    }
    let p = paths::trim_end_sep(&paths::normalize_separators(p)).to_string();
    if paths::is_root(&p) {
        return Err("請選擇一個資料夾，不要直接安裝到磁碟根目錄。".into());
    }
    if p.split('\\').any(|part| part.ends_with('.') || part.ends_with(' ')) {
        return Err("資料夾名稱不能以句點或空白結尾。".into());
    }
    let lower = p.to_lowercase();
    for dir in protected.iter().filter(|d| !d.trim().is_empty()) {
        let d = paths::trim_end_sep(&paths::normalize_separators(dir)).to_lowercase();
        if lower == d || lower.starts_with(&format!("{d}\\")) {
            return Err(format!("不能安裝到系統資料夾（{dir}）。"));
        }
    }
    Ok(p)
}

// ---------------------------------------------------------------- 新增或移除程式

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UninstallEntry {
    pub strings: Vec<(&'static str, String)>,
    pub dwords: Vec<(&'static str, u32)>,
}

/// 「設定 › 應用程式 › 已安裝的應用程式」裡的項目。`date` 是 YYYYMMDD。
pub fn uninstall_entry(dir: &str, version: &str, size_kb: u32, date: &str) -> UninstallEntry {
    let exe = paths::join(dir, EXE_NAME);
    let uninstaller = quoted(&paths::join(dir, UNINSTALLER_NAME));
    UninstallEntry {
        strings: vec![
            ("DisplayName", APP_NAME.to_string()),
            ("DisplayIcon", format!("{exe},0")),
            ("DisplayVersion", version.to_string()),
            ("Publisher", APP_NAME.to_string()),
            ("InstallLocation", dir.to_string()),
            ("UninstallString", uninstaller.clone()),
            ("QuietUninstallString", format!("{uninstaller} /S")),
            ("InstallDate", date.to_string()),
        ],
        dwords: vec![("NoModify", 1), ("NoRepair", 1), ("EstimatedSize", size_kb)],
    }
}

// ---------------------------------------------------------------- 安裝程式的命令列

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupMode {
    Install,
    /// 解除安裝。從安裝資料夾裡的 Uninstall.exe 啟動時（沒有 `dir`），先把自己複製到暫存資料夾、
    /// 帶著 `--dir 安裝位置` 重新執行，安裝資料夾裡就沒有執行中的檔案，可以整個刪掉。
    Uninstall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupArgs {
    pub mode: SetupMode,
    pub silent: bool,
    pub dir: Option<String>,
    pub default_fm: Option<bool>,
    /// 其他程式開啟檔案時也用 Stillfeather（None：沒有指定，維持使用者原本的選擇；沒選過就是開啟）
    pub file_picker: Option<bool>,
    pub desktop: Option<bool>,
    pub start_menu: Option<bool>,
    pub launch: Option<bool>,
    /// 已經以系統管理員身分重新啟動過（不要再要求一次）
    pub elevated: bool,
    /// 以系統管理員身分執行的那一份把結果（錯誤訊息，成功時空白）寫到這個檔案
    pub result: Option<String>,
    /// 由 Stillfeather 的更新提醒啟動：直接開始安裝到 `dir`，維持原本的捷徑與預設檔案總管設定，完成後重新開啟
    pub update: bool,
}

/// 解析安裝程式的參數。支援常見安裝程式的寫法：
/// `/S`、`/SILENT`、`/VERYSILENT`、`/quiet`、`--silent`（不顯示畫面）；
/// `/D=路徑`（必須是最後一個參數，可以含空白）、`--dir=路徑`、`--dir 路徑`；
/// `--default-explorer`／`--no-default-explorer`、`--file-picker`／`--no-file-picker`、`--desktop`／`--no-desktop`、`--no-start-menu`、`--launch`／`--no-launch`；
/// `--uninstall`（解除安裝）；`--update`（更新：沒有選項畫面，維持原本的設定，完成後重新開啟）。
pub fn parse_setup_args(args: &[String]) -> Result<SetupArgs, String> {
    let mut a = SetupArgs { mode: SetupMode::Install, silent: false, dir: None, default_fm: None, file_picker: None, desktop: None, start_menu: None, launch: None, elevated: false, result: None, update: false };
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].as_str();
        let lower = raw.to_ascii_lowercase();
        if let Some(rest) = raw.strip_prefix("/D=").or_else(|| raw.strip_prefix("/d=")) {
            // NSIS 的慣例：/D= 之後全部都是路徑
            let mut dir = rest.to_string();
            for more in &args[i + 1..] {
                dir.push(' ');
                dir.push_str(more);
            }
            a.dir = Some(dir);
            break;
        }
        if let Some(rest) = lower.strip_prefix("--dir=") {
            a.dir = Some(raw[raw.len() - rest.len()..].to_string());
            i += 1;
            continue;
        }
        if let Some(rest) = lower.strip_prefix("--result=") {
            a.result = Some(raw[raw.len() - rest.len()..].to_string());
            i += 1;
            continue;
        }
        match lower.as_str() {
            "/s" | "/silent" | "/verysilent" | "/quiet" | "/q" | "--silent" | "--quiet" => a.silent = true,
            "--dir" | "/dir" => {
                i += 1;
                a.dir = Some(args.get(i).ok_or("--dir 後面需要安裝位置。")?.clone());
            }
            "--default-explorer" => a.default_fm = Some(true),
            "--no-default-explorer" => a.default_fm = Some(false),
            "--file-picker" => a.file_picker = Some(true),
            "--no-file-picker" => a.file_picker = Some(false),
            "--desktop" => a.desktop = Some(true),
            "--no-desktop" => a.desktop = Some(false),
            "--start-menu" => a.start_menu = Some(true),
            "--no-start-menu" => a.start_menu = Some(false),
            "--launch" => a.launch = Some(true),
            "--no-launch" => a.launch = Some(false),
            "--uninstall" | "/uninstall" => a.mode = SetupMode::Uninstall,
            "--elevated" => a.elevated = true,
            "--update" => a.update = true,
            _ => return Err(format!("不認得的參數：{raw}")),
        }
        i += 1;
    }
    Ok(a)
}

/// 依 Windows 命令列規則加上引號（CommandLineToArgvW 能還原成同一個字串）。
pub fn quote_arg(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// 程式結束後才刪得掉的檔案（執行中的解除安裝程式本身）：交給 cmd.exe，等兩秒讓程式先結束，
/// 刪除檔案後再移除資料夾（`rmdir` 不加 /s，資料夾裡還有別的東西就不會刪）。
pub fn delayed_delete_command(files: &[&str], dirs: &[&str]) -> String {
    let mut parts = vec!["ping -n 3 127.0.0.1 >nul".to_string()];
    parts.extend(files.iter().map(|f| format!("del /f /q \"{f}\"")));
    parts.extend(dirs.iter().map(|d| format!("rmdir \"{d}\"")));
    format!("/d /q /c \"{}\"", parts.join(" & "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\pity0\AppData\Local\Programs\Stillfeather\Stillfeather.exe";

    #[test]
    fn default_fm_values_cover_folders_drives_and_win_e() {
        let v = default_fm_values(EXE);
        let find = |key: &str, name: &str| v.iter().find(|x| x.key == key && x.name == name).map(|x| x.data.clone());
        assert_eq!(find(r"Software\Classes\Directory\shell", ""), Some("Stillfeather".into()));
        assert_eq!(find(r"Software\Classes\Drive\shell", ""), Some("Stillfeather".into()));
        assert_eq!(find(r"Software\Classes\Directory\shell\Stillfeather\command", ""), Some(format!("\"{EXE}\" \"%1\"")));
        assert_eq!(find(r"Software\Classes\Drive\shell\Stillfeather\command", ""), Some(format!("\"{EXE}\" \"%1\"")));
        let wine = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";
        assert_eq!(find(wine, ""), Some(format!("\"{EXE}\" --open-home")));
        assert_eq!(find(wine, "DelegateExecute"), Some(String::new()));
        // 預設動詞最後才設，動詞本身一定已經存在
        let pos_default = v.iter().position(|x| x.key == r"Software\Classes\Directory\shell" && x.name.is_empty()).unwrap();
        let pos_command = v.iter().position(|x| x.key == r"Software\Classes\Directory\shell\Stillfeather\command").unwrap();
        assert!(pos_command < pos_default);
    }

    #[test]
    fn every_shared_value_is_written_and_owned_keys_hold_the_rest() {
        let v = default_fm_values(EXE);
        for (key, name) in shared_values() {
            assert!(v.iter().any(|x| x.key == key && x.name == name), "{key}|{name}");
        }
        let owned = owned_keys();
        let shared = shared_values();
        for x in &v {
            let is_shared = shared.iter().any(|(k, n)| *k == x.key && *n == x.name);
            let in_owned = owned.iter().any(|k| x.key == *k || x.key.starts_with(&format!("{k}\\")));
            assert!(is_shared || in_owned, "{}|{} 沒有被移除", x.key, x.name);
        }
        // 刪除順序由深到淺
        let prune = prune_keys();
        assert!(prune[0].len() > prune[1].len() && prune[1].len() > prune[2].len() && prune[2].len() > prune[3].len());
    }

    #[test]
    fn command_matching() {
        assert_eq!(command_program("\"C:\\A B\\x.exe\" \"%1\""), "C:\\A B\\x.exe");
        assert_eq!(command_program("C:\\x.exe --open-home"), "C:\\x.exe");
        assert!(command_targets(&folder_command(EXE), &EXE.to_uppercase()));
        assert!(command_targets(&win_e_command(EXE), &EXE.replace('\\', "/")));
        assert!(!command_targets("\"C:\\Program Files\\Other\\fm.exe\" \"%1\"", EXE));
        assert!(!command_targets("", EXE));
    }

    #[test]
    fn install_dirs() {
        assert_eq!(default_install_dir(r"C:\Users\pity0\AppData\Local"), r"C:\Users\pity0\AppData\Local\Programs\Stillfeather");
        assert_eq!(with_app_folder(r"D:\Apps\"), r"D:\Apps\Stillfeather");
        assert_eq!(with_app_folder(r"D:\Apps\stillfeather"), r"D:\Apps\stillfeather");
        assert_eq!(with_app_folder("D:\\"), r"D:\Stillfeather");
        let prot = [r"C:\Windows"];
        assert_eq!(normalize_install_dir(" \"D:/Apps/Stillfeather/\" ", &prot).unwrap(), r"D:\Apps\Stillfeather");
        assert!(normalize_install_dir("", &prot).is_err());
        assert!(normalize_install_dir(r"Apps\Stillfeather", &prot).is_err());
        assert!(normalize_install_dir("D:\\", &prot).is_err());
        assert!(normalize_install_dir(r"D:\A|B", &prot).is_err());
        assert!(normalize_install_dir(r"D:\A:B", &prot).is_err());
        assert!(normalize_install_dir(r"D:\Apps.\x", &prot).is_err());
        assert!(normalize_install_dir(r"c:\windows\Stillfeather", &prot).is_err());
        assert!(normalize_install_dir(r"C:\WindowsApps2\Stillfeather", &prot).is_ok());
        assert!(normalize_install_dir(r"\\nas\share\Stillfeather", &prot).is_ok());
    }

    #[test]
    fn uninstall_entry_values() {
        let e = uninstall_entry(r"D:\Apps\Stillfeather", "0.1.0", 2600, "20261122");
        let s = |n: &str| e.strings.iter().find(|(k, _)| *k == n).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(s("DisplayName"), "Stillfeather");
        assert_eq!(s("UninstallString"), "\"D:\\Apps\\Stillfeather\\Uninstall.exe\"");
        assert_eq!(s("QuietUninstallString"), "\"D:\\Apps\\Stillfeather\\Uninstall.exe\" /S");
        assert_eq!(s("DisplayIcon"), "D:\\Apps\\Stillfeather\\Stillfeather.exe,0");
        assert!(e.dwords.contains(&("EstimatedSize", 2600)));
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn setup_args() {
        let a = parse_setup_args(&[]).unwrap();
        assert_eq!((a.mode, a.silent, a.dir.clone()), (SetupMode::Install, false, None));
        let a = parse_setup_args(&args(&["/S", "--no-desktop", "/D=C:\\Program Files\\Still", "feather"])).unwrap();
        assert!(a.silent);
        assert_eq!(a.desktop, Some(false));
        assert_eq!(a.dir.as_deref(), Some("C:\\Program Files\\Still feather"));
        let a = parse_setup_args(&args(&["--dir", "D:\\X", "--default-explorer", "--elevated"])).unwrap();
        assert_eq!((a.dir.as_deref(), a.default_fm, a.elevated), (Some("D:\\X"), Some(true), true));
        let a = parse_setup_args(&args(&["--Dir=D:\\Mixed Case"])).unwrap();
        assert_eq!(a.dir.as_deref(), Some("D:\\Mixed Case"));
        assert_eq!(parse_setup_args(&args(&["--uninstall", "/quiet"])).unwrap().mode, SetupMode::Uninstall);
        let a = parse_setup_args(&args(&["--uninstall", "--elevated", "/S", "--result=C:\\T\\r 1.txt"])).unwrap();
        assert_eq!((a.mode, a.elevated, a.silent, a.result.as_deref()), (SetupMode::Uninstall, true, true, Some("C:\\T\\r 1.txt")));
        assert!(parse_setup_args(&args(&["--bogus"])).is_err());
        // Stillfeather 的更新提醒啟動新版安裝程式的參數
        let dir = "C:\\Users\\me\\AppData\\Local\\Programs\\Stillfeather";
        let a = parse_setup_args(&crate::update::setup_args(dir)).unwrap();
        assert!(a.update && !a.silent && a.mode == SetupMode::Install);
        assert_eq!(a.dir.as_deref(), Some(dir));
        assert_eq!((a.default_fm, a.desktop, a.start_menu), (None, None, None), "更新時維持原狀");
        assert!(parse_setup_args(&args(&["--dir"])).is_err());
    }

    /// CommandLineToArgvW 的規則（簡化版），用來驗證 quote_arg
    fn split_args(cmd: &str) -> Vec<String> {
        let mut out = Vec::new();
        let chars: Vec<char> = cmd.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let mut arg = String::new();
            let mut quoted = false;
            while i < chars.len() && (quoted || chars[i] != ' ') {
                let mut bs = 0;
                while i < chars.len() && chars[i] == '\\' {
                    bs += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    arg.extend(std::iter::repeat_n('\\', bs / 2));
                    if bs % 2 == 1 {
                        arg.push('"');
                    } else {
                        quoted = !quoted;
                    }
                    i += 1;
                } else {
                    arg.extend(std::iter::repeat_n('\\', bs));
                    if i < chars.len() && (quoted || chars[i] != ' ') {
                        arg.push(chars[i]);
                        i += 1;
                    }
                }
            }
            out.push(arg);
        }
        out
    }

    #[test]
    fn quoting_roundtrips() {
        for s in ["plain", r"C:\Program Files\Stillfeather", r"C:\trailing\", "say \"hi\"", r"a\\b c\", "", r"\\nas\share x\"] {
            let q = quote_arg(s);
            assert_eq!(split_args(&q), vec![s.to_string()], "{s} → {q}");
        }
        assert_eq!(quote_arg(r"C:\x"), r"C:\x");
    }

    #[test]
    fn delayed_delete() {
        let c = delayed_delete_command(&[r"C:\T\s\Uninstall.exe", r"D:\Apps\Stillfeather\Uninstall.exe"], &[r"C:\T\s", r"D:\Apps\Stillfeather"]);
        assert!(c.starts_with("/d /q /c \"ping -n 3 127.0.0.1 >nul & del /f /q \"C:\\T\\s\\Uninstall.exe\""));
        assert!(c.contains("& rmdir \"C:\\T\\s\" & rmdir \"D:\\Apps\\Stillfeather\""));
        assert!(c.ends_with("\"\""));
        assert!(!c.contains("/s "));
    }

    #[test]
    fn picker_keys() {
        assert_eq!(picker_server_key(), r"Software\Classes\CLSID\{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}\InprocServer32");
        assert!(is_picker_dll(r"C:\Users\me\AppData\Local\FeatherFiles\Picker\stillfeatherpicker.dll"));
        assert!(!is_picker_dll(r"%SystemRoot%\System32\comdlg32.dll"));
        assert!(GENERATED_FILES.contains(&"picker-trace.log"));
    }

    #[test]
    fn file_picker_choice() {
        // 沒選過：預設開啟；只有明確關掉才不要
        assert!(picker_wanted(None));
        assert!(picker_wanted(Some("1")));
        assert!(!picker_wanted(Some("0")));
        assert!(!picker_wanted(Some(" 0 ")));
        let args = |v: &[&str]| parse_setup_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(args(&[]).file_picker, None);
        assert_eq!(args(&["--file-picker"]).file_picker, Some(true));
        assert_eq!(args(&["/S", "--no-file-picker"]).file_picker, Some(false));
    }

    #[test]
    fn default_fm_choice_on_start() {
        use DefaultFmStart::*;
        // 沒選過：預設開啟，但不搶另一份 Stillfeather 的
        assert_eq!(default_fm_on_start(None, None), Take);
        assert_eq!(default_fm_on_start(None, Some(true)), Remember);
        assert_eq!(default_fm_on_start(None, Some(false)), Take);
        // 開：只有登記的那一份不見了才接手；改用別的檔案管理程式時不搶回來
        assert_eq!(default_fm_on_start(Some("1"), None), Keep);
        assert_eq!(default_fm_on_start(Some("1"), Some(true)), Keep);
        assert_eq!(default_fm_on_start(Some("1"), Some(false)), Take);
        // 關：不動
        for r in [None, Some(true), Some(false)] {
            assert_eq!(default_fm_on_start(Some("0"), r), Keep);
            assert_eq!(default_fm_on_start(Some(" 0 "), r), Keep);
        }
    }
}

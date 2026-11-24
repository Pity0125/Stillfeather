//! 右鍵選單的「在這裡開啟 PowerShell 視窗」：要執行哪一個 PowerShell、傳什麼參數（純邏輯，可在任何平台測試）。
//!
//! 裝了 PowerShell 7（`pwsh.exe`）時用它，否則用 Windows 內建的 Windows PowerShell 5.1（和檔案總管的同名項目相同）。
//! 開在 Windows 終端機還是傳統的主控台視窗，由 Windows 的「預設終端機應用程式」設定決定。

/// PowerShell 的單引號字串。PowerShell 把 ' ‘ ’ ‚ ‛ 都當作單引號，所以每一個都重複一次來跳脫
/// （和 PowerShell 自己的 `CodeGeneration.EscapeSingleQuotedStringContent` 相同）；`$`、反引號在單引號裡不會展開。
pub fn ps_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        out.push(c);
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// 傳給 powershell.exe／pwsh.exe 的參數：開著視窗，切換到 `folder`。
///
/// 整個命令放在一對雙引號裡，Windows 拆命令列時才不會把路徑裡連續的空白壓成一個
/// （檔案總管的同名項目有這個問題）。檔名不能有雙引號，所以不必另外跳脫。
pub fn powershell_args(folder: &str) -> String {
    format!("-NoExit -Command \"Set-Location -LiteralPath {}\"", ps_single_quoted(folder))
}

/// 能不能同時當作新程序的工作目錄（Windows 限制目前目錄的長度，也不接受 `\\?\` 開頭的路徑；
/// 不能的話只靠參數切換位置）
pub fn usable_as_working_dir(folder: &str) -> bool {
    !folder.is_empty() && folder.encode_utf16().count() < 248 && !folder.starts_with(r"\\?\") && !folder.starts_with("::")
}

/// 要執行的 PowerShell：依序找 PowerShell 7（安裝版、Microsoft Store 版），都沒有就用內建的 Windows PowerShell。
/// `exists` 檢查檔案在不在（Microsoft Store 版是應用程式執行別名，要用不跟隨重新剖析點的方式檢查）。
pub fn pick_powershell(program_files: Option<&str>, local_app_data: Option<&str>, system_root: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    let mut candidates = Vec::new();
    if let Some(pf) = program_files.filter(|p| !p.is_empty()) {
        candidates.push(format!("{}\\PowerShell\\7\\pwsh.exe", pf.trim_end_matches('\\')));
    }
    if let Some(la) = local_app_data.filter(|p| !p.is_empty()) {
        candidates.push(format!("{}\\Microsoft\\WindowsApps\\pwsh.exe", la.trim_end_matches('\\')));
    }
    if let Some(found) = candidates.into_iter().find(|c| exists(c)) {
        return found;
    }
    let root = system_root.filter(|p| !p.is_empty()).unwrap_or("C:\\Windows");
    format!("{}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", root.trim_end_matches('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(ps_single_quoted(r"C:\Users\amy"), r"'C:\Users\amy'");
        assert_eq!(ps_single_quoted("C:\\Bob's files"), "'C:\\Bob''s files'");
        // 彎引號也是 PowerShell 的單引號
        assert_eq!(ps_single_quoted("D:\\it\u{2019}s"), "'D:\\it\u{2019}\u{2019}s'");
        assert_eq!(ps_single_quoted("D:\\\u{2018}x\u{201A}\u{201B}"), "'D:\\\u{2018}\u{2018}x\u{201A}\u{201A}\u{201B}\u{201B}'");
        // 變數與反引號在單引號裡不會展開，不必處理
        assert_eq!(ps_single_quoted("E:\\$env:x `n"), "'E:\\$env:x `n'");
        assert_eq!(ps_single_quoted("C:\\"), "'C:\\'");
    }

    #[test]
    fn args() {
        assert_eq!(powershell_args("C:\\a  b"), "-NoExit -Command \"Set-Location -LiteralPath 'C:\\a  b'\"");
        assert_eq!(powershell_args("C:\\"), "-NoExit -Command \"Set-Location -LiteralPath 'C:\\'\"");
        assert_eq!(powershell_args(r"\\server\share\x"), "-NoExit -Command \"Set-Location -LiteralPath '\\\\server\\share\\x'\"");
    }

    #[test]
    fn working_dir() {
        assert!(usable_as_working_dir("C:\\Users"));
        assert!(usable_as_working_dir(r"\\server\share"));
        assert!(!usable_as_working_dir(""));
        assert!(!usable_as_working_dir(r"\\?\C:\very\long"));
        assert!(!usable_as_working_dir("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        assert!(!usable_as_working_dir(&format!("C:\\{}", "a".repeat(250))));
        assert!(usable_as_working_dir(&format!("C:\\{}", "文".repeat(200))));
    }

    #[test]
    fn picks() {
        let none = |_: &str| false;
        assert_eq!(pick_powershell(Some("C:\\Program Files"), Some("C:\\Users\\a\\AppData\\Local"), Some("C:\\Windows"), none), "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        assert_eq!(pick_powershell(None, None, None, none), "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        assert_eq!(pick_powershell(Some("C:\\Program Files\\"), None, Some("D:\\Win\\"), |p| p == "C:\\Program Files\\PowerShell\\7\\pwsh.exe"), "C:\\Program Files\\PowerShell\\7\\pwsh.exe");
        let store = "C:\\Users\\a\\AppData\\Local\\Microsoft\\WindowsApps\\pwsh.exe";
        assert_eq!(pick_powershell(Some("C:\\Program Files"), Some("C:\\Users\\a\\AppData\\Local"), None, |p| p == store), store);
        // 兩種都有時用安裝版
        assert_eq!(pick_powershell(Some("C:\\Program Files"), Some("C:\\Users\\a\\AppData\\Local"), None, |_| true), "C:\\Program Files\\PowerShell\\7\\pwsh.exe");
    }
}

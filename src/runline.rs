//! 路徑列按 Enter 時，輸入的東西怎麼處理（同 Win+R 與檔案總管的路徑列）：
//!
//! - 資料夾：開啟（這裡不判斷，交給導覽）。
//! - 程式、批次檔、捷徑（[`is_runnable`]），或不含路徑的指令（`cmd`、`notepad`，[`is_bare_command`]）、
//!   網址等通訊協定（`https:`、`ms-settings:`，[`is_protocol`]）：交給 `ShellExecuteExW` 執行。
//! - `shell:startup`、`::{CLSID}` 這類位置（[`is_shell_location`]）：用 `SHParseDisplayName` 解析後開啟。
//! - 其他檔案：開啟所在的資料夾並選取它。

/// 直接執行（不開所在資料夾）的檔案類型：程式、批次檔、捷徑、安裝檔、主控台與控制台項目
const RUNNABLE: &[&str] = &["exe", "com", "bat", "cmd", "lnk", "url", "pif", "appref-ms", "msi", "msc", "cpl"];

/// `shell:startup`、`shell:::{CLSID}`、`::{CLSID}`
pub fn is_shell_location(text: &str) -> bool {
    let t = text.trim().trim_matches('"');
    let lower = t.to_ascii_lowercase();
    (lower.starts_with("shell:") && t.len() > "shell:".len()) || t.starts_with("::{")
}

/// 這個檔案要直接執行（看副檔名，不分大小寫）
pub fn is_runnable(path: &str) -> bool {
    let name = path.trim().trim_matches('"').rsplit(['\\', '/']).next().unwrap_or("");
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => RUNNABLE.iter().any(|r| r.eq_ignore_ascii_case(ext)),
        _ => false,
    }
}

/// 網址等通訊協定（`https://…`、`mailto:…`、`ms-settings:display`）。
/// 至少兩個字的協定名稱才算，`C:` 這種磁碟代號不算；`shell:` 另外處理。
pub fn is_protocol(text: &str) -> bool {
    let t = text.trim();
    let Some((scheme, rest)) = t.split_once(':') else { return false };
    let mut chars = scheme.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    first_ok
        && scheme.len() >= 2
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !scheme.eq_ignore_ascii_case("shell")
        && !rest.is_empty()
        && !t.contains(char::is_whitespace)
}

/// 拆成第一段（程式）與後面的參數：第一段用引號包起來時到下一個引號，否則到第一個空白
pub fn split_command(text: &str) -> (String, String) {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('"') {
        return match rest.split_once('"') {
            Some((file, args)) => (file.to_string(), args.trim().to_string()),
            None => (rest.to_string(), String::new()),
        };
    }
    match t.split_once(char::is_whitespace) {
        Some((file, args)) => (file.to_string(), args.trim().to_string()),
        None => (t.to_string(), String::new()),
    }
}

/// 不含路徑的指令（`cmd`、`notepad`、`calc.exe`）：沒有 `\`、`/`，也不是磁碟代號，交給 Windows 找
/// （PATH、App Paths，同 Win+R）
pub fn is_bare_command(token: &str) -> bool {
    let t = token.trim();
    !t.is_empty() && t != "." && t != ".." && !t.contains(['\\', '/', ':', '"', '*', '?', '<', '>', '|'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_locations() {
        assert!(is_shell_location("shell:startup"));
        assert!(is_shell_location("  SHELL:Downloads "));
        assert!(is_shell_location("shell:::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        assert!(is_shell_location("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        assert!(!is_shell_location("shell:"));
        assert!(!is_shell_location(r"C:\shell:x"));
        assert!(!is_shell_location("startup"));
    }

    #[test]
    fn runnable_files() {
        for p in [r"C:\Tools\app.exe", r"\\nas\setup\Setup.EXE", "run.bat", "build.cmd", r"C:\Users\me\Desktop\Steam.lnk", "site.url", "x.msi", "\"C:\\a b\\c.exe\""] {
            assert!(is_runnable(p), "{p}");
        }
        for p in [r"C:\報告.docx", r"C:\Tools", r"C:\Tools\.exe", "readme", r"C:\a.exe\b.txt", "photo.jpg"] {
            assert!(!is_runnable(p), "{p}");
        }
    }

    #[test]
    fn protocols() {
        assert!(is_protocol("https://example.com"));
        assert!(is_protocol("ms-settings:display"));
        assert!(is_protocol("mailto:a@b.c"));
        assert!(!is_protocol(r"C:\Windows"));
        assert!(!is_protocol("C:"));
        assert!(!is_protocol("shell:startup"));
        assert!(!is_protocol("notepad"));
        assert!(!is_protocol("https:"));
        assert!(!is_protocol("1x:abc"));
        assert!(!is_protocol("note: buy milk"));
    }

    #[test]
    fn splitting_commands() {
        assert_eq!(split_command("notepad"), ("notepad".into(), String::new()));
        assert_eq!(split_command("  cmd /k dir  "), ("cmd".into(), "/k dir".into()));
        assert_eq!(split_command(r#""C:\Program Files\App\app.exe" --new "x y""#), (r"C:\Program Files\App\app.exe".into(), r#"--new "x y""#.into()));
        assert_eq!(split_command(r#""C:\a b"#), (r"C:\a b".into(), String::new()));
    }

    #[test]
    fn bare_commands() {
        for t in ["cmd", "notepad", "calc.exe", "regedit", "control", "記事本"] {
            assert!(is_bare_command(t), "{t}");
        }
        for t in ["", " ", ".", "..", r"..\x", r"C:\x", "C:", "a/b", r"\\nas", "*.txt"] {
            assert!(!is_bare_command(t), "{t}");
        }
    }
}

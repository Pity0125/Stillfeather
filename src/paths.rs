//! Windows 路徑的純字串處理（不碰檔案系統）。

pub fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// `\\?\` 或 `\\.\` 開頭的延伸／裝置路徑。
pub fn is_extended(p: &str) -> bool {
    p.starts_with("\\\\?\\") || p.starts_with("\\\\.\\")
}

pub fn normalize_separators(p: &str) -> String {
    p.replace('/', "\\")
}

/// 根目錄長度：`C:\` → 3、`C:` → 2、`\\server\share` → 到 share 結尾。
/// 不是完整路徑時回傳 0。
pub fn root_len(p: &str) -> usize {
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        return if b.len() >= 3 && (b[2] == b'\\' || b[2] == b'/') { 3 } else { 2 };
    }
    if b.len() >= 2 && (b[0] == b'\\' || b[0] == b'/') && (b[1] == b'\\' || b[1] == b'/') {
        // \\server\share
        let rest = &p[2..];
        let mut parts = 0;
        let mut idx = 2;
        for (i, c) in rest.char_indices() {
            if is_sep(c) {
                parts += 1;
                if parts == 2 {
                    return 2 + i;
                }
            }
            idx = 2 + i + c.len_utf8();
        }
        if parts == 1 && idx > 2 && !p.ends_with(is_sep) {
            return p.len();
        }
        return 0;
    }
    0
}

/// `C:\...` 或 `\\server\share\...` 這種不依賴目前目錄的完整路徑。
pub fn is_fully_qualified(p: &str) -> bool {
    if is_extended(p) {
        return false;
    }
    let b = p.as_bytes();
    if b.len() >= 3 && b[1] == b':' && b[0].is_ascii_alphabetic() && (b[2] == b'\\' || b[2] == b'/') {
        return true;
    }
    if b.len() >= 5 && (b[0] == b'\\') && (b[1] == b'\\') {
        let rl = root_len(p);
        if rl > 2 {
            let root = &p[2..rl];
            let mut it = root.split(is_sep);
            let server = it.next().unwrap_or("");
            let share = it.next().unwrap_or("");
            return !server.is_empty() && !share.is_empty();
        }
    }
    false
}

/// 去掉結尾分隔符號，但保留磁碟根目錄的 `\`（`C:\`）。
pub fn trim_end_sep(p: &str) -> &str {
    let mut end = p.len();
    while end > 0 && p[..end].ends_with(is_sep) {
        let candidate = &p[..end - 1];
        // 保留 "C:\"
        if candidate.len() == 2 && candidate.as_bytes()[1] == b':' {
            break;
        }
        // 不要把 "\\" 開頭吃光
        if candidate.is_empty() || candidate == "\\" {
            break;
        }
        end -= 1;
    }
    &p[..end]
}

pub fn is_root(p: &str) -> bool {
    let t = trim_end_sep(p);
    let rl = root_len(t);
    rl > 0 && rl >= t.len()
}

pub fn parent(p: &str) -> Option<String> {
    let t = trim_end_sep(p);
    let rl = root_len(t);
    if rl == 0 || t.len() <= rl {
        return None;
    }
    let idx = t.rfind(is_sep)?;
    if idx < rl {
        // 父層就是根目錄
        let root = &t[..rl];
        return Some(if root.len() == 2 { format!("{root}\\") } else { root.to_string() });
    }
    let parent = &t[..idx];
    if parent.len() == 2 && parent.as_bytes()[1] == b':' {
        return Some(format!("{parent}\\"));
    }
    if parent.len() < rl {
        return Some(t[..rl].to_string());
    }
    Some(parent.to_string())
}

/// 最後一個路徑元件；根目錄回傳空字串。
pub fn file_name(p: &str) -> &str {
    let t = trim_end_sep(p);
    if is_root(t) {
        return "";
    }
    match t.rfind(is_sep) {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_string();
    }
    if dir.ends_with(is_sep) {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

fn fold(c: char) -> char {
    // Windows 檔名比較採大寫對照；單一字元對照即可涵蓋常見情況。
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

pub fn eq_ignore_case(a: &str, b: &str) -> bool {
    let mut ia = a.chars();
    let mut ib = b.chars();
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) => {
                if x != y && fold(x) != fold(y) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> Option<usize> {
    let mut is = s.char_indices();
    for pc in prefix.chars() {
        let (_, sc) = is.next()?;
        if sc != pc && fold(sc) != fold(pc) {
            return None;
        }
    }
    Some(is.next().map(|(i, _)| i).unwrap_or(s.len()))
}

/// 檔案系統語意的「同一路徑」（Windows 預設不分大小寫）。
pub fn same_path(a: &str, b: &str) -> bool {
    eq_ignore_case(trim_end_sep(&normalize_separators(a)), trim_end_sep(&normalize_separators(b)))
}

/// 拖放目標檢查：不能放進被拖曳的項目本身或它的子資料夾，
/// 也不必放回它們原本所在的資料夾（全部都已經在那裡）。
pub fn drop_target_ok(target: &str, sources: &[String]) -> bool {
    if sources.iter().any(|s| is_same_or_descendant(s, target)) {
        return false;
    }
    let already_there = !sources.is_empty() && sources.iter().all(|s| parent(s).is_some_and(|p| same_path(&p, target)));
    !already_there
}

/// candidate 等於 ancestor 或位於其下。
pub fn is_same_or_descendant(ancestor: &str, candidate: &str) -> bool {
    let a = normalize_separators(ancestor);
    let c = normalize_separators(candidate);
    let a = trim_end_sep(&a);
    let c = trim_end_sep(&c);
    match starts_with_ignore_case(c, a) {
        Some(end) if end == c.len() => true,
        Some(end) => a.ends_with('\\') || c[end..].starts_with('\\'),
        None => false,
    }
}

/// 對外部開啟要求比對「相同路徑字串」：與 C# 版相同，只去掉結尾分隔符號後逐字比較。
pub fn same_request_path(a: &str, b: &str) -> bool {
    trim_end_sep(a) == trim_end_sep(b)
}

const INVALID_NAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// 新名稱是否可用：不可為空、`.`、`..`，不含保留字元，結尾不可是空白或句點。
pub fn valid_name(name: &str) -> bool {
    if name.trim().is_empty() || name == "." || name == ".." {
        return false;
    }
    if name.chars().any(|c| (c as u32) < 32 || INVALID_NAME_CHARS.contains(&c)) {
        return false;
    }
    !(name.ends_with(' ') || name.ends_with('.'))
}

const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 是否為 Windows 保留裝置名稱（CON、NUL、COM1…，不論副檔名）。
pub fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end();
    RESERVED_DEVICE_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem))
}

/// 路徑每個元件都不含 Shell 不支援、或會被 Windows 自動正規化的名稱。
pub fn components_valid(p: &str) -> bool {
    let rl = root_len(p);
    if rl == 0 {
        return false;
    }
    p[rl..].split(is_sep).filter(|c| !c.is_empty()).all(valid_name)
}

/// 重新解析點標籤是否為「連結」（符號連結、junction、掛載點、WSL 連結等名稱代理）。
///
/// 只看 FILE_ATTRIBUTE_REPARSE_POINT 會把 OneDrive 等雲端檔案（IO_REPARSE_TAG_CLOUD_*）、
/// 重複資料刪除、App 執行別名也當成連結：類型顯示成「連結」、側欄展不開、搜尋與複製被拒絕。
/// 這些都不是名稱代理（IsReparseTagNameSurrogate 的 0x20000000 位元為 0），照一般項目處理即可。
pub fn reparse_is_link(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

/// 路徑列輸入的相對路徑（`..`、`子資料夾`、`\Windows`）以目前資料夾 `base` 為準組成完整路徑；
/// 已經是完整路徑（`C:\…`、`C:`、`\\server\share`、延伸格式）時回傳 None。
/// `..` 等元件交給 GetFullPathNameW 正規化。
pub fn resolve_relative(base: &str, input: &str) -> Option<String> {
    if input.is_empty() || root_len(input) > 0 || is_extended(input) {
        return None;
    }
    let rl = root_len(base);
    if rl == 0 {
        return None;
    }
    if input.starts_with(is_sep) {
        // `\\server` 這種不完整的 UNC 不猜
        if input.len() >= 2 && input[1..].starts_with(is_sep) {
            return None;
        }
        // `\Windows`：目前磁碟（或網路共用）的根目錄底下
        return Some(format!("{}{}", trim_end_sep(&base[..rl]).trim_end_matches(is_sep), input));
    }
    Some(join(base, input))
}

/// 分頁標題：資料夾名稱，根目錄則顯示整個根路徑。
pub fn tab_title(path: &str) -> String {
    if path.is_empty() {
        return tr!("新分頁", "New tab").to_string();
    }
    let name = file_name(path);
    if name.is_empty() {
        trim_end_sep(path).to_string() + if root_len(path) == 2 { "\\" } else { "" }
    } else {
        name.to_string()
    }
}

/// 超過傳統 MAX_PATH 時改用 `\\?\` 延伸格式，讓列舉與屬性查詢不受 260 字元限制。
pub fn to_long_path(p: &str) -> String {
    if p.len() < 240 || is_extended(p) {
        return p.to_string();
    }
    if p.starts_with("\\\\") {
        format!("\\\\?\\UNC\\{}", &p[2..])
    } else {
        format!("\\\\?\\{p}")
    }
}

/// 不分大小寫的副檔名（不含句點，小寫）。".gitignore" 的副檔名為 "gitignore"，與 .NET 相同。
pub fn extension_lower(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// 麵包屑：把路徑拆成（顯示名稱, 完整路徑）。
/// `C:\Users\a` → [("C:", "C:\"), ("Users", "C:\Users"), ("a", "C:\Users\a")]；
/// UNC 的第一段是整個 `\\server\share`。
pub fn segments(path: &str) -> Vec<(String, String)> {
    let p = normalize_separators(path);
    let p = trim_end_sep(&p);
    let rl = root_len(p);
    if rl == 0 {
        return if p.is_empty() { vec![] } else { vec![(p.to_string(), p.to_string())] };
    }
    let root = &p[..rl];
    let (label, mut current) = if root.len() <= 3 && root.as_bytes().get(1) == Some(&b':') {
        (root[..2].to_string(), format!("{}\\", &root[..2]))
    } else {
        (trim_end_sep(root).to_string(), trim_end_sep(root).to_string())
    };
    let mut out = vec![(label, current.clone())];
    for part in p[rl..].split(is_sep).filter(|c| !c.is_empty()) {
        current = join(&current, part);
        out.push((part.to_string(), current.clone()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots() {
        assert_eq!(root_len("C:\\a"), 3);
        assert_eq!(root_len("C:"), 2);
        assert_eq!(root_len("\\\\srv\\share\\x"), 11);
        assert_eq!(root_len("\\\\srv\\share"), 11);
        assert_eq!(root_len("relative\\x"), 0);
        assert!(is_root("C:\\"));
        assert!(is_root("C:"));
        assert!(is_root("\\\\srv\\share\\"));
        assert!(!is_root("C:\\Users"));
    }

    #[test]
    fn qualified() {
        assert!(is_fully_qualified("C:\\x"));
        assert!(is_fully_qualified("\\\\srv\\share\\x"));
        assert!(is_fully_qualified("\\\\srv\\share"));
        assert!(!is_fully_qualified("C:x"));
        assert!(!is_fully_qualified("\\x"));
        assert!(!is_fully_qualified("\\\\srv"));
        assert!(!is_fully_qualified("\\\\?\\C:\\x"));
    }

    #[test]
    fn parents() {
        assert_eq!(parent("C:\\Users\\pity0\\Desktop").as_deref(), Some("C:\\Users\\pity0"));
        assert_eq!(parent("C:\\Users\\").as_deref(), Some("C:\\"));
        assert_eq!(parent("C:\\Users").as_deref(), Some("C:\\"));
        assert_eq!(parent("C:\\"), None);
        assert_eq!(parent("\\\\srv\\share\\a\\b").as_deref(), Some("\\\\srv\\share\\a"));
        assert_eq!(parent("\\\\srv\\share\\a").as_deref(), Some("\\\\srv\\share"));
        assert_eq!(parent("\\\\srv\\share"), None);
    }

    #[test]
    fn names_and_titles() {
        assert_eq!(file_name("C:\\Users\\pity0\\Desktop\\win11\\"), "win11");
        assert_eq!(file_name("C:\\"), "");
        assert_eq!(tab_title("C:\\"), "C:\\");
        assert_eq!(tab_title("D:"), "D:\\");
        assert_eq!(tab_title("C:\\Users\\pity0\\桌面"), "桌面");
        assert_eq!(tab_title(""), "新分頁");
        assert_eq!(join("C:\\", "a"), "C:\\a");
        assert_eq!(join("C:\\a", "b"), "C:\\a\\b");
        assert_eq!(trim_end_sep("C:\\a\\\\"), "C:\\a");
        assert_eq!(trim_end_sep("C:\\"), "C:\\");
    }

    #[test]
    fn comparisons() {
        assert!(same_path("C:\\Data\\", "c:\\data"));
        assert!(same_path("C:/Data", "C:\\DATA"));
        assert!(!same_path("C:\\Data", "C:\\Data2"));
        assert!(is_same_or_descendant("C:\\Data", "C:\\data\\x\\y"));
        assert!(is_same_or_descendant("C:\\Data", "C:\\Data"));
        assert!(!is_same_or_descendant("C:\\Data", "C:\\Database"));
        assert!(is_same_or_descendant("C:\\", "C:\\Database"));
        assert!(same_request_path("C:\\a\\", "C:\\a"));
        assert!(!same_request_path("C:\\A", "C:\\a"));
        assert!(eq_ignore_case("ÄBC", "äbc"));
    }

    #[test]
    fn name_validation() {
        assert!(valid_name("報告.txt"));
        assert!(!valid_name(""));
        assert!(!valid_name(" "));
        assert!(!valid_name(".."));
        assert!(!valid_name("a:b"));
        assert!(!valid_name("a."));
        assert!(!valid_name("a "));
        assert!(!valid_name("a\u{7}"));
        assert!(is_reserved_device_name("con"));
        assert!(is_reserved_device_name("NUL.txt"));
        assert!(!is_reserved_device_name("console"));
        assert!(components_valid("C:\\a\\b c\\d.txt"));
        assert!(!components_valid("C:\\a\\b.\\d"));
    }

    #[test]
    fn breadcrumb_segments() {
        let v = segments("C:\\Users\\pity0\\桌面\\");
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], ("C:".to_string(), "C:\\".to_string()));
        assert_eq!(v[1], ("Users".to_string(), "C:\\Users".to_string()));
        assert_eq!(v[3], ("桌面".to_string(), "C:\\Users\\pity0\\桌面".to_string()));
        assert_eq!(segments("D:\\"), vec![("D:".to_string(), "D:\\".to_string())]);
        let u = segments("\\\\nas\\share\\photos");
        assert_eq!(u[0], ("\\\\nas\\share".to_string(), "\\\\nas\\share".to_string()));
        assert_eq!(u[1], ("photos".to_string(), "\\\\nas\\share\\photos".to_string()));
        assert!(segments("").is_empty());
    }

    #[test]
    fn long_paths_and_extensions() {
        let long = format!("C:\\{}", "a".repeat(260));
        assert!(to_long_path(&long).starts_with("\\\\?\\C:\\"));
        let unc = format!("\\\\srv\\share\\{}", "b".repeat(260));
        assert!(to_long_path(&unc).starts_with("\\\\?\\UNC\\srv\\share\\"));
        assert_eq!(to_long_path("C:\\short"), "C:\\short");
        assert_eq!(extension_lower("Photo.JPG"), "jpg");
        assert_eq!(extension_lower(".gitignore"), "gitignore");
        assert_eq!(extension_lower("noext"), "");
        assert_eq!(extension_lower("trail."), "");
    }

    #[test]
    fn drop_targets() {
        let src = vec!["C:\\Work\\a.txt".to_string(), "C:\\Work\\Photos".to_string()];
        assert!(drop_target_ok("D:\\Backup", &src));
        assert!(drop_target_ok("C:\\Work\\Other", &src));
        // 放回原本的資料夾：沒有意義
        assert!(!drop_target_ok("C:\\Work", &src));
        assert!(!drop_target_ok("c:\\work\\", &src));
        // 放進自己或自己的子資料夾
        assert!(!drop_target_ok("C:\\Work\\Photos", &src));
        assert!(!drop_target_ok("C:\\Work\\Photos\\2024", &src));
        // 只有部分項目在目標資料夾：仍然可以
        let mixed = vec!["C:\\Work\\a.txt".to_string(), "D:\\b.txt".to_string()];
        assert!(drop_target_ok("C:\\Work", &mixed));
        // 虛擬項目（沒有路徑）：交給 Windows 判斷
        assert!(drop_target_ok("C:\\Work", &[]));
    }

    #[test]
    fn reparse_tags() {
        // 連結：junction／掛載點、符號連結、WSL 符號連結
        assert!(reparse_is_link(0xA000_0003));
        assert!(reparse_is_link(0xA000_000C));
        assert!(reparse_is_link(0xA000_001D));
        // 不是連結：OneDrive 雲端檔案（含各種子類型）、OneDrive 舊標籤、重複資料刪除、App 執行別名、投影檔案系統
        for tag in [0x9000_001A, 0x9000_101A, 0x9000_F01A, 0x8000_0021, 0x8000_0013, 0x8000_001B, 0x9000_001C] {
            assert!(!reparse_is_link(tag), "{tag:#x}");
        }
    }

    #[test]
    fn relative_address_input() {
        let base = "C:\\Users\\pity0\\Desktop";
        assert_eq!(resolve_relative(base, "..").as_deref(), Some("C:\\Users\\pity0\\Desktop\\.."));
        assert_eq!(resolve_relative(base, "win11-rust\\src").as_deref(), Some("C:\\Users\\pity0\\Desktop\\win11-rust\\src"));
        assert_eq!(resolve_relative(base, "\\Windows").as_deref(), Some("C:\\Windows"));
        assert_eq!(resolve_relative("C:\\", "\\Windows").as_deref(), Some("C:\\Windows"));
        assert_eq!(resolve_relative("C:\\", "Users").as_deref(), Some("C:\\Users"));
        assert_eq!(resolve_relative("\\\\nas\\share\\a", "\\b").as_deref(), Some("\\\\nas\\share\\b"));
        assert_eq!(resolve_relative("\\\\nas\\share\\a", "b").as_deref(), Some("\\\\nas\\share\\a\\b"));
        // 已經是完整路徑：不動
        assert_eq!(resolve_relative(base, "D:\\x"), None);
        assert_eq!(resolve_relative(base, "D:"), None);
        assert_eq!(resolve_relative(base, "\\\\nas\\share"), None);
        assert_eq!(resolve_relative(base, "\\\\nas"), None);
        assert_eq!(resolve_relative(base, "\\\\?\\C:\\x"), None);
        // 沒有目前資料夾（新分頁還沒讀完）
        assert_eq!(resolve_relative("", "abc"), None);
    }
}

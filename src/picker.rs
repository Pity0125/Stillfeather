//! 檔案選擇器：讓其他程式（瀏覽器上傳檔案、各種「開啟舊檔」「選擇資料夾」）跳出 Stillfeather。
//!
//! 流程：其他程式建立 Windows 的「開啟檔案」視窗（IFileOpenDialog）時，載入的是 picker/ 的取代元件；
//! 它在 Show 時把設定寫成一個請求檔，以 `Stillfeather.exe --pick <請求檔>` 開啟精簡的選擇視窗，
//! 等使用者選好，再讀回結果檔交給那個程式。
//!
//! 這裡是兩邊共用的純邏輯：請求與結果的格式、檔案類型篩選（`*.jpg;*.png`）、檔名欄的解析，
//! 以及每個程式上次用的資料夾。picker/ 以 `#[path]` 引入這個檔案，所以只能依賴 `crate::paths`。

use crate::paths;

/// Stillfeather 以選擇器模式啟動的參數
pub const PICK_ARG: &str = "--pick";

/// 請求檔與結果檔的格式版本（第一行 `stillfeather-pick=<版本>`、`stillfeather-picked=<版本>`）。
///
/// 已經開著的程式會一直用它當初載入的取代元件，直到重新開啟；所以新版 Stillfeather 一定會收到舊版元件寫的請求，
/// 舊版元件也會讀到新版 Stillfeather 寫的結果。規則：
/// - 只「加」欄位（舊的讀取端會略過不認得的欄位）時，版本不變。
/// - 改掉或拿掉欄位的意思時才加一；讀取端要能讀所有 ≤ 自己版本的格式（見 `old_requests_still_parse` 測試）。
/// - 讀到比自己新的請求就不處理（結束代碼 2），元件改開 Windows 原本的視窗；
///   結果檔一律用版本 1 寫（`stillfeather-picked=1`），所有版本的元件都讀得懂。
pub const PROTOCOL: u32 = 1;
const REQUEST_HEADER: &str = "stillfeather-pick=";
const RESPONSE_HEADER: &str = "stillfeather-picked=";

/// 第一行的版本號：`prefix` 後面是 1 到 `max` 的數字才算
fn header_version(line: Option<&str>, prefix: &str, max: u32) -> Option<u32> {
    let v: u32 = line?.trim_start_matches('\u{feff}').trim().strip_prefix(prefix)?.parse().ok()?;
    (1..=max).contains(&v).then_some(v)
}
/// 取代元件的檔名（放在 Stillfeather.exe 旁邊）
pub const DLL_NAME: &str = "StillfeatherPicker.dll";
/// Windows「開啟檔案」視窗的類別識別碼
pub const CLSID_FILE_OPEN_DIALOG: &str = "{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}";

/// 選擇器的結束代碼：選好了
pub const EXIT_OK: i32 = 0;
/// 取消
pub const EXIT_CANCEL: i32 = 1;
/// 使用者按了「使用 Windows 視窗」：改開 Windows 原本的視窗
pub const EXIT_SYSTEM: i32 = 3;

/// 每個程式記住的資料夾最多幾筆
pub const RECENT_MAX: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// 選檔案（上傳、開啟舊檔）
    #[default]
    Files,
    /// 選資料夾
    Folders,
}

/// 一種檔案類型，例如（「圖片」, `*.jpg;*.png`）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    pub name: String,
    pub spec: String,
}

impl FileType {
    /// 選單上顯示的名稱：名稱裡沒有寫出副檔名時補上。
    pub fn label(&self) -> String {
        let name = self.name.trim();
        let spec = self.spec.trim();
        if name.is_empty() {
            spec.to_string()
        } else if spec.is_empty() || name.contains(spec) || spec == "*.*" || spec == "*" || name.contains('(') {
            name.to_string()
        } else {
            format!("{name} ({spec})")
        }
    }
}

/// 其他程式交給選擇器的設定
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    pub mode: Mode,
    /// 可以選多個
    pub multi: bool,
    /// 視窗標題（沒有指定時依模式顯示「開啟」或「選擇資料夾」）
    pub title: String,
    /// 確定按鈕的文字
    pub ok_label: String,
    /// 檔名欄前面的文字
    pub name_label: String,
    /// 一定從這個資料夾開始（SetFolder）
    pub folder: String,
    /// 這個程式沒有記錄時才從這裡開始（SetDefaultFolder）
    pub default_folder: String,
    /// 檔名欄預先填入的文字
    pub file_name: String,
    pub types: Vec<FileType>,
    /// 選中的類型（從 1 起算；0 表示沒有指定）
    pub type_index: u32,
    pub default_ext: String,
    /// 只能選已經存在的檔案
    pub must_exist: bool,
    /// 一律顯示隱藏的項目
    pub show_hidden: bool,
    /// 要求的程式的視窗（選擇器以它為擁有者，蓋在它上面）
    pub owner: isize,
    /// 取代元件在選擇期間停用了 owner：選擇器關閉前先恢復它，焦點才會回到那個視窗
    pub enable_owner: bool,
    pub host_pid: u32,
    /// 要求的程式（exe 檔名），用來記住它上次用的資料夾
    pub host: String,
    /// 結果檔
    pub out: String,
}

/// 值不能有換行與定位字元（一行一個設定，類型以定位字元分隔名稱與副檔名）
fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' || c == '\t' { ' ' } else { c }).collect()
}

fn flag(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

impl Request {
    pub fn to_text(&self) -> String {
        let mut out = format!("{REQUEST_HEADER}{PROTOCOL}\n");
        let mut put = |k: &str, v: &str| {
            out.push_str(k);
            out.push('=');
            out.push_str(&clean(v));
            out.push('\n');
        };
        put("mode", if self.mode == Mode::Folders { "folders" } else { "files" });
        put("multi", flag(self.multi));
        put("title", &self.title);
        put("ok", &self.ok_label);
        put("label", &self.name_label);
        put("folder", &self.folder);
        put("default_folder", &self.default_folder);
        put("file_name", &self.file_name);
        put("type_index", &self.type_index.to_string());
        put("default_ext", &self.default_ext);
        put("must_exist", flag(self.must_exist));
        put("show_hidden", flag(self.show_hidden));
        put("owner", &self.owner.to_string());
        put("enable_owner", flag(self.enable_owner));
        put("host_pid", &self.host_pid.to_string());
        put("host", &self.host);
        put("out", &self.out);
        for t in &self.types {
            out.push_str("type=");
            out.push_str(&clean(&t.name));
            out.push('\t');
            out.push_str(&clean(&t.spec));
            out.push('\n');
        }
        out
    }

    pub fn parse(text: &str) -> Result<Request, String> {
        let mut lines = text.lines();
        let first = lines.next();
        if header_version(first, REQUEST_HEADER, PROTOCOL).is_none() {
            return Err(if first.is_some_and(|l| l.trim_start_matches('\u{feff}').starts_with(REQUEST_HEADER)) {
                "選擇器請求的格式比這一版 Stillfeather 新（請更新 Stillfeather）。".into()
            } else {
                "不是 Stillfeather 的選擇器請求。".into()
            });
        }
        let mut r = Request::default();
        for line in lines {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "mode" => r.mode = if v == "folders" { Mode::Folders } else { Mode::Files },
                "multi" => r.multi = v == "1",
                "title" => r.title = v.to_string(),
                "ok" => r.ok_label = v.to_string(),
                "label" => r.name_label = v.to_string(),
                "folder" => r.folder = v.to_string(),
                "default_folder" => r.default_folder = v.to_string(),
                "file_name" => r.file_name = v.to_string(),
                "type_index" => r.type_index = v.parse().unwrap_or(0),
                "default_ext" => r.default_ext = v.to_string(),
                "must_exist" => r.must_exist = v == "1",
                "show_hidden" => r.show_hidden = v == "1",
                "owner" => r.owner = v.parse().unwrap_or(0),
                "enable_owner" => r.enable_owner = v == "1",
                "host_pid" => r.host_pid = v.parse().unwrap_or(0),
                "host" => r.host = v.to_string(),
                "out" => r.out = v.to_string(),
                "type" => {
                    let (name, spec) = v.split_once('\t').unwrap_or((v, ""));
                    r.types.push(FileType { name: name.to_string(), spec: spec.to_string() });
                }
                _ => {}
            }
        }
        if r.out.is_empty() {
            return Err("選擇器請求缺少結果檔的位置。".into());
        }
        Ok(r)
    }

    /// 一開始選中的類型（從 0 起算）。
    pub fn initial_type(&self) -> usize {
        if self.types.is_empty() {
            return 0;
        }
        (self.type_index.max(1) as usize - 1).min(self.types.len() - 1)
    }

    /// 視窗標題
    pub fn window_title(&self) -> String {
        if !self.title.trim().is_empty() {
            self.title.trim().to_string()
        } else if self.mode == Mode::Folders {
            "選擇資料夾".into()
        } else if self.multi {
            "選擇檔案".into()
        } else {
            "開啟".into()
        }
    }

    /// 確定按鈕的文字（去掉 Windows 選單用的 & 加速鍵記號）
    pub fn ok_text(&self) -> String {
        let label = self.ok_label.replace('&', "");
        if !label.trim().is_empty() {
            label.trim().to_string()
        } else if self.mode == Mode::Folders {
            "選擇資料夾".into()
        } else {
            "開啟".into()
        }
    }

    /// 檔名欄前面的文字
    pub fn name_text(&self) -> String {
        let label = self.name_label.replace('&', "");
        let label = label.trim().trim_end_matches([':', '：']).trim();
        if !label.is_empty() {
            label.to_string()
        } else if self.mode == Mode::Folders {
            "資料夾".into()
        } else {
            "檔案名稱".into()
        }
    }
}

/// 選擇器交回的結果
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// 選取的完整路徑
    pub paths: Vec<String>,
    /// 最後選中的類型（從 1 起算；0 表示沒有類型）
    pub type_index: u32,
}

impl Response {
    pub fn to_text(&self) -> String {
        // 一律寫版本 1：已經載入在其他程式裡的舊版元件只認得這個（見 PROTOCOL）
        let mut out = format!("{RESPONSE_HEADER}1\n");
        out.push_str(&format!("type_index={}\n", self.type_index));
        for p in &self.paths {
            out.push_str("path=");
            out.push_str(&clean(p));
            out.push('\n');
        }
        out
    }

    pub fn parse(text: &str) -> Option<Response> {
        let mut lines = text.lines();
        header_version(lines.next(), RESPONSE_HEADER, PROTOCOL)?;
        let mut r = Response::default();
        for line in lines {
            match line.split_once('=') {
                Some(("type_index", v)) => r.type_index = v.parse().unwrap_or(0),
                Some(("path", v)) if !v.is_empty() => r.paths.push(v.to_string()),
                _ => {}
            }
        }
        Some(r)
    }
}

// ---------------------------------------------------------------- 類型篩選

/// 清單上要顯示哪些項目：資料夾一律顯示（才能進去找）；檔案依類型的萬用字元。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// 選資料夾：不顯示檔案
    pub folders_only: bool,
    /// 小寫的萬用字元；空的表示全部檔案
    pub patterns: Vec<String>,
}

impl Filter {
    /// `*.jpg;*.png` 這種寫法（也接受空白與逗號分隔）
    pub fn from_spec(spec: &str) -> Filter {
        let patterns: Vec<String> = spec.split([';', ',', ' ']).map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect();
        // 「全部檔案」不必比對
        if patterns.iter().any(|p| p == "*" || p == "*.*") {
            return Filter::default();
        }
        Filter { folders_only: false, patterns }
    }

    pub fn folders() -> Filter {
        Filter { folders_only: true, patterns: Vec::new() }
    }

    pub fn matches(&self, name: &str, is_dir: bool) -> bool {
        if is_dir {
            return true;
        }
        if self.folders_only {
            return false;
        }
        if self.patterns.is_empty() {
            return true;
        }
        let lower = name.to_lowercase();
        self.patterns.iter().any(|p| wildcard_match(p, &lower))
    }

    /// 是否會藏起任何檔案
    pub fn is_all(&self) -> bool {
        !self.folders_only && self.patterns.is_empty()
    }
}

/// `*`（任意長度）與 `?`（一個字元）的比對，區分大小寫（呼叫端先轉成小寫）。
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while j < n.len() {
        if i < p.len() && (p[i] == '?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

// ---------------------------------------------------------------- 檔名欄

/// 檔名欄的內容：`"a.txt" "b.txt"`（多個，以引號分開）或一個名稱、路徑。
pub fn parse_names(text: &str) -> Vec<String> {
    let text = text.trim();
    if !text.contains('"') {
        return if text.is_empty() { Vec::new() } else { vec![text.to_string()] };
    }
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                let name = after[..end].trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
                rest = &after[end + 1..];
            }
            None => {
                let name = after.trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
                rest = "";
            }
        }
    }
    out
}

/// 選取的項目寫回檔名欄：一個就直接寫名稱，多個各自加引號。
pub fn names_text(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        many => many.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(" "),
    }
}

/// 檔名欄打的是萬用字元（例如 `*.txt`）：當作臨時的類型篩選。
pub fn is_pattern(text: &str) -> bool {
    text.contains('*') || text.contains('?')
}

/// 檔名欄的一個名稱換成完整路徑：已經是完整路徑就照用，否則以目前的資料夾為準。
pub fn resolve(folder: &str, name: &str) -> String {
    let name = paths::normalize_separators(name.trim());
    if paths::is_fully_qualified(&name) {
        name
    } else if let Some(rest) = name.strip_prefix('\\') {
        // \Windows：目前磁碟的根目錄底下
        let root_len = paths::root_len(folder);
        let root = paths::trim_end_sep(&folder[..root_len.min(folder.len())]).to_string();
        paths::join(&root, rest)
    } else {
        paths::join(folder, &name)
    }
}

// ---------------------------------------------------------------- 每個程式上次用的資料夾

/// 記錄檔內容是一行一筆：`程式\t資料夾`，最近用的在最前面。
pub fn recent_folder(text: &str, host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    text.lines().filter_map(|l| l.split_once('\t')).find(|(h, _)| h.eq_ignore_ascii_case(host)).map(|(_, f)| f.to_string()).filter(|f| !f.is_empty())
}

/// 記下這個程式這次用的資料夾，回傳新的記錄檔內容（最多 RECENT_MAX 筆）。
pub fn remember_folder(text: &str, host: &str, folder: &str) -> String {
    let mut lines = vec![format!("{}\t{}", clean(host), clean(folder))];
    for l in text.lines() {
        let Some((h, f)) = l.split_once('\t') else { continue };
        if h.eq_ignore_ascii_case(host) || f.is_empty() {
            continue;
        }
        if lines.len() >= RECENT_MAX {
            break;
        }
        lines.push(l.to_string());
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Request {
        Request {
            mode: Mode::Files,
            multi: true,
            title: "選擇要上傳的檔案".into(),
            ok_label: "&Open".into(),
            name_label: "File &name:".into(),
            folder: String::new(),
            default_folder: "C:\\Users\\me\\Pictures".into(),
            file_name: "".into(),
            types: vec![
                FileType { name: "圖片".into(), spec: "*.jpg;*.png".into() },
                FileType { name: "All files (*.*)".into(), spec: "*.*".into() },
            ],
            type_index: 2,
            default_ext: "jpg".into(),
            must_exist: true,
            show_hidden: false,
            owner: 0x1234,
            enable_owner: true,
            host_pid: 4321,
            host: "chrome.exe".into(),
            out: "C:\\Temp\\out.txt".into(),
        }
    }

    #[test]
    fn request_round_trip() {
        let r = sample();
        let back = Request::parse(&r.to_text()).unwrap();
        assert_eq!(back, r);
        // 換行與定位字元不會破壞格式
        let mut odd = sample();
        odd.title = "a\nb\tc".into();
        let back = Request::parse(&odd.to_text()).unwrap();
        assert_eq!(back.title, "a b c");
        assert_eq!(back.types.len(), 2);
    }

    /// v0.1.0～v0.1.2 的取代元件寫出的請求（原樣保留，不要改）：之後的 Stillfeather 都必須讀得懂，
    /// 因為已經開著的程式會一直用舊的元件。
    const REQUEST_V0_1: &str = "stillfeather-pick=1\nmode=files\nmulti=1\ntitle=選擇要上傳的檔案\nok=&Open\nlabel=File &name:\nfolder=\ndefault_folder=C:\\Users\\me\\Pictures\nfile_name=\ntype_index=2\ndefault_ext=png\nmust_exist=1\nshow_hidden=0\nowner=263690\nenable_owner=1\nhost_pid=4321\nhost=chrome.exe\nout=C:\\Temp\\Stillfeather-pick-4321-99-out.txt\ntype=All Files\t*.*\ntype=Images\t*.png;*.jpg\n";
    /// 同一時期的 Stillfeather 寫出的結果：舊版元件只認得這個樣子。
    const RESPONSE_V0_1: &str = "stillfeather-picked=1\ntype_index=2\npath=C:\\Users\\me\\Pictures\\a.png\npath=C:\\Users\\me\\Pictures\\b c.jpg\n";

    #[test]
    fn old_requests_still_parse() {
        let r = Request::parse(REQUEST_V0_1).unwrap();
        assert_eq!(r.mode, Mode::Files);
        assert!(r.multi && r.must_exist && r.enable_owner && !r.show_hidden);
        assert_eq!(r.host, "chrome.exe");
        assert_eq!(r.host_pid, 4321);
        assert_eq!(r.owner, 263690);
        assert_eq!(r.type_index, 2);
        assert_eq!(r.default_folder, "C:\\Users\\me\\Pictures");
        assert_eq!(r.types.len(), 2);
        assert_eq!(r.types[1].spec, "*.png;*.jpg");
        assert_eq!(r.out, "C:\\Temp\\Stillfeather-pick-4321-99-out.txt");
        // 新版寫出的請求，舊版的讀法（只認 stillfeather-pick=1、略過不認得的欄位）也讀得懂
        assert!(sample().to_text().starts_with("stillfeather-pick=1\n"));
    }

    #[test]
    fn responses_stay_readable_by_old_components() {
        let r = Response { paths: vec!["C:\\Users\\me\\Pictures\\a.png".into(), "C:\\Users\\me\\Pictures\\b c.jpg".into()], type_index: 2 };
        assert_eq!(r.to_text(), RESPONSE_V0_1);
        assert_eq!(Response::parse(RESPONSE_V0_1), Some(r));
    }

    #[test]
    fn newer_formats_fall_back_to_windows() {
        // 比自己新的請求：不處理（Stillfeather 以結束代碼 2 結束，元件改開 Windows 的視窗）
        let newer = REQUEST_V0_1.replacen("stillfeather-pick=1", &format!("stillfeather-pick={}", PROTOCOL + 1), 1);
        assert!(Request::parse(&newer).unwrap_err().contains("新"));
        assert!(Request::parse(&REQUEST_V0_1.replacen("=1", "=0", 1)).is_err());
        assert!(Request::parse(&REQUEST_V0_1.replacen("=1", "=x", 1)).is_err());
        let newer = RESPONSE_V0_1.replacen("picked=1", &format!("picked={}", PROTOCOL + 1), 1);
        assert_eq!(Response::parse(&newer), None);
    }

    #[test]
    fn request_rejects_other_files() {
        assert!(Request::parse("hello\nout=x").is_err());
        assert!(Request::parse("stillfeather-pick=1\nmode=files\n").is_err());
        // BOM 與未知的欄位
        let r = Request::parse("\u{feff}stillfeather-pick=1\nfuture=1\nout=C:\\o.txt\nmode=folders\n").unwrap();
        assert_eq!(r.mode, Mode::Folders);
        assert_eq!(r.out, "C:\\o.txt");
    }

    #[test]
    fn request_texts() {
        let r = sample();
        assert_eq!(r.window_title(), "選擇要上傳的檔案");
        assert_eq!(r.ok_text(), "Open");
        assert_eq!(r.name_text(), "File name");
        assert_eq!(r.initial_type(), 1);
        let d = Request::default();
        assert_eq!(d.window_title(), "開啟");
        assert_eq!(d.ok_text(), "開啟");
        assert_eq!(d.name_text(), "檔案名稱");
        assert_eq!(d.initial_type(), 0);
        let f = Request { mode: Mode::Folders, ..Default::default() };
        assert_eq!(f.window_title(), "選擇資料夾");
        assert_eq!(f.ok_text(), "選擇資料夾");
        assert_eq!(f.name_text(), "資料夾");
        let m = Request { multi: true, ..Default::default() };
        assert_eq!(m.window_title(), "選擇檔案");
        let big = Request { type_index: 9, types: sample().types, ..Default::default() };
        assert_eq!(big.initial_type(), 1);
    }

    #[test]
    fn type_labels() {
        assert_eq!(FileType { name: "圖片".into(), spec: "*.jpg;*.png".into() }.label(), "圖片 (*.jpg;*.png)");
        assert_eq!(FileType { name: "Images (*.jpg)".into(), spec: "*.jpg".into() }.label(), "Images (*.jpg)");
        assert_eq!(FileType { name: "全部檔案".into(), spec: "*.*".into() }.label(), "全部檔案");
        assert_eq!(FileType { name: "".into(), spec: "*.txt".into() }.label(), "*.txt");
    }

    #[test]
    fn response_round_trip() {
        let r = Response { paths: vec!["C:\\a.txt".into(), "D:\\相片\\b.png".into()], type_index: 2 };
        assert_eq!(Response::parse(&r.to_text()), Some(r));
        assert_eq!(Response::parse("other"), None);
        assert_eq!(Response::parse("stillfeather-picked=1\npath=\n").unwrap().paths.len(), 0);
    }

    #[test]
    fn filters() {
        let f = Filter::from_spec("*.JPG; *.png");
        assert!(f.matches("photo.jpg", false));
        assert!(f.matches("PHOTO.JPG", false));
        assert!(f.matches("a.png", false));
        assert!(!f.matches("a.gif", false));
        assert!(f.matches("任何資料夾", true));
        assert!(Filter::from_spec("*.*").is_all());
        assert!(Filter::from_spec("*").is_all());
        assert!(Filter::from_spec("").is_all());
        assert!(Filter::from_spec("*.*").matches("Makefile", false));
        let folders = Filter::folders();
        assert!(!folders.matches("a.txt", false));
        assert!(folders.matches("dir", true));
        assert!(!folders.is_all());
        assert!(Filter::from_spec("report-??.docx").matches("report-01.docx", false));
        assert!(!Filter::from_spec("report-??.docx").matches("report-1.docx", false));
    }

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*.txt", "a.txt"));
        assert!(wildcard_match("*.txt", ".txt"));
        assert!(!wildcard_match("*.txt", "a.txt.bak"));
        assert!(wildcard_match("a*b*c", "aXXbYYc"));
        assert!(wildcard_match("a*b*c", "abc"));
        assert!(!wildcard_match("a*b*c", "acb"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("?", ""));
        assert!(wildcard_match("照片*.jpg", "照片001.jpg"));
        assert!(wildcard_match("**x", "abcx"));
    }

    #[test]
    fn names() {
        assert_eq!(parse_names("  report.docx "), vec!["report.docx"]);
        assert_eq!(parse_names("\"a b.txt\" \"c.txt\""), vec!["a b.txt", "c.txt"]);
        assert_eq!(parse_names("\"a.txt\" \"unfinished"), vec!["a.txt", "unfinished"]);
        assert!(parse_names("   ").is_empty());
        assert!(parse_names("\"\"").is_empty());
        assert_eq!(names_text(&["a.txt"]), "a.txt");
        assert_eq!(names_text(&["a.txt", "b c.txt"]), "\"a.txt\" \"b c.txt\"");
        assert_eq!(names_text(&[]), "");
        assert!(is_pattern("*.txt"));
        assert!(!is_pattern("a.txt"));
    }

    #[test]
    fn resolve_names() {
        assert_eq!(resolve("C:\\work", "a.txt"), "C:\\work\\a.txt");
        assert_eq!(resolve("C:\\work", "D:\\x\\b.txt"), "D:\\x\\b.txt");
        assert_eq!(resolve("C:\\work", "sub/c.txt"), "C:\\work\\sub\\c.txt");
        assert_eq!(resolve("C:\\work\\deep", "\\Windows"), "C:\\Windows");
        assert_eq!(resolve("C:\\", "a.txt"), "C:\\a.txt");
        assert_eq!(resolve("\\\\srv\\share\\x", "\\y"), "\\\\srv\\share\\y");
    }

    #[test]
    fn recent() {
        let t = remember_folder("", "chrome.exe", "C:\\Users\\me\\Downloads");
        assert_eq!(recent_folder(&t, "CHROME.EXE").as_deref(), Some("C:\\Users\\me\\Downloads"));
        let t = remember_folder(&t, "code.exe", "D:\\src");
        let t = remember_folder(&t, "chrome.exe", "C:\\Users\\me\\Pictures");
        assert_eq!(recent_folder(&t, "chrome.exe").as_deref(), Some("C:\\Users\\me\\Pictures"));
        assert_eq!(recent_folder(&t, "code.exe").as_deref(), Some("D:\\src"));
        assert_eq!(t.lines().count(), 2);
        assert!(t.starts_with("chrome.exe\t"));
        assert_eq!(recent_folder(&t, "other.exe"), None);
        assert_eq!(recent_folder(&t, ""), None);
        // 上限
        let mut big = String::new();
        for i in 0..100 {
            big = remember_folder(&big, &format!("app{i}.exe"), "C:\\x");
        }
        assert_eq!(big.lines().count(), RECENT_MAX);
        assert!(big.starts_with("app99.exe\t"));
        // 壞掉的行略過
        assert_eq!(recent_folder("garbage\nchrome.exe\tC:\\d\n", "chrome.exe").as_deref(), Some("C:\\d"));
    }
}

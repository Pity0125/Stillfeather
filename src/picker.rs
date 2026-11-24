//! 檔案選擇器：讓其他程式（瀏覽器上傳檔案、各種「開啟舊檔」「另存新檔」「選擇資料夾」）跳出 Stillfeather。
//!
//! 流程：其他程式建立 Windows 的「開啟檔案」或「另存新檔」視窗（IFileOpenDialog、IFileSaveDialog）時，載入的是 picker/ 的取代元件；
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
/// - 改掉或拿掉欄位的意思、或舊的讀取端會誤會的內容時才加一；讀取端要能讀所有 ≤ 自己版本的格式（見 `old_requests_still_parse` 測試）。
/// - 讀到比自己新的請求就不處理（結束代碼 2），元件改開 Windows 原本的視窗；
///   結果檔一律用版本 1 寫（`stillfeather-picked=1`），所有版本的元件都讀得懂。
///
/// 版本 2：另存新檔（`mode=save`）。舊版 Stillfeather 不認得這個模式，會把它當成開啟檔案，
/// 所以另存新檔的請求寫成版本 2（舊版讀到就讓元件改開 Windows 的視窗）；開啟檔案與選擇資料夾的請求照樣寫版本 1。
pub const PROTOCOL: u32 = 2;
const REQUEST_HEADER: &str = "stillfeather-pick=";
const RESPONSE_HEADER: &str = "stillfeather-picked=";

/// 第一行的版本號：`prefix` 後面是 1 到 `max` 的數字才算
fn header_version(line: Option<&str>, prefix: &str, max: u32) -> Option<u32> {
    let v: u32 = line?.trim_start_matches('\u{feff}').trim().strip_prefix(prefix)?.parse().ok()?;
    (1..=max).contains(&v).then_some(v)
}
/// 取代元件的檔名（放在 Stillfeather.exe 旁邊）
pub const DLL_NAME: &str = "StillfeatherPicker.dll";
/// 「另存新檔」用的另一份取代元件（內容和 DLL_NAME 相同，只是檔名不同，見 install::picker_save_server_key）
pub const SAVE_DLL_NAME: &str = "StillfeatherSavePicker.dll";
/// Windows「開啟檔案」視窗的類別識別碼
pub const CLSID_FILE_OPEN_DIALOG: &str = "{DC1C5A9C-E88A-4dde-A5A1-60F82A20AEF7}";
/// Windows「另存新檔」視窗的類別識別碼
pub const CLSID_FILE_SAVE_DIALOG: &str = "{C0B4E2F3-BA21-4773-8DBA-335EC946EB8B}";

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
    /// 另存新檔：選一個資料夾並輸入檔名（檔案可以還不存在）
    Save,
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
    /// 另存新檔：檔案已經存在時先問要不要取代（FOS_OVERWRITEPROMPT）
    pub overwrite_prompt: bool,
    /// 另存新檔：副檔名一定要符合選中的類型，不符合時補上（FOS_STRICTFILETYPES）
    pub strict_types: bool,
    /// 另存新檔：不能選唯讀的檔案（FOS_NOREADONLYRETURN）
    pub no_readonly: bool,
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
    /// 寫成請求檔。版本用能表達這個請求的最低版本（見 PROTOCOL）。
    pub fn to_text(&self) -> String {
        let version = if self.mode == Mode::Save { 2 } else { 1 };
        let mut out = format!("{REQUEST_HEADER}{version}\n");
        let mut put = |k: &str, v: &str| {
            out.push_str(k);
            out.push('=');
            out.push_str(&clean(v));
            out.push('\n');
        };
        put(
            "mode",
            match self.mode {
                Mode::Files => "files",
                Mode::Folders => "folders",
                Mode::Save => "save",
            },
        );
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
        if self.mode == Mode::Save {
            put("overwrite", flag(self.overwrite_prompt));
            put("strict_types", flag(self.strict_types));
            put("no_readonly", flag(self.no_readonly));
        }
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
                tr!("選擇器請求的格式比這一版 Stillfeather 新（請更新 Stillfeather）。", "The picker request is in a newer format than this version of Stillfeather (please update Stillfeather).").into()
            } else {
                tr!("不是 Stillfeather 的選擇器請求。", "This isn't a Stillfeather picker request.").into()
            });
        }
        let mut r = Request::default();
        for line in lines {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k {
                "mode" => {
                    r.mode = match v {
                        "folders" => Mode::Folders,
                        "save" => Mode::Save,
                        _ => Mode::Files,
                    }
                }
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
                "overwrite" => r.overwrite_prompt = v == "1",
                "strict_types" => r.strict_types = v == "1",
                "no_readonly" => r.no_readonly = v == "1",
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
            return Err(tr!("選擇器請求缺少結果檔的位置。", "The picker request is missing the result file location.").into());
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
            tr!("選擇資料夾", "Select Folder").into()
        } else if self.mode == Mode::Save {
            tr!("另存新檔", "Save As").into()
        } else if self.multi {
            tr!("選擇檔案", "Select Files").into()
        } else {
            tr!("開啟", "Open").into()
        }
    }

    /// 確定按鈕的文字（去掉 Windows 選單用的 & 加速鍵記號）
    pub fn ok_text(&self) -> String {
        let label = self.ok_label.replace('&', "");
        if !label.trim().is_empty() {
            label.trim().to_string()
        } else if self.mode == Mode::Folders {
            tr!("選擇資料夾", "Select Folder").into()
        } else if self.mode == Mode::Save {
            tr!("存檔", "Save").into()
        } else {
            tr!("開啟", "Open").into()
        }
    }

    /// 檔名欄前面的文字
    pub fn name_text(&self) -> String {
        let label = self.name_label.replace('&', "");
        let label = label.trim().trim_end_matches([':', '：']).trim();
        if !label.is_empty() {
            label.to_string()
        } else if self.mode == Mode::Folders {
            tr!("資料夾", "Folder").into()
        } else {
            tr!("檔案名稱", "File name").into()
        }
    }

    /// 另存新檔時自動加上的副檔名（不含點）：程式指定了預設副檔名才會加（和 Windows 的視窗相同），
    /// 加的是選中的類型的副檔名（例如選了「網頁 (*.htm;*.html)」就是 htm）；類型是「全部檔案」時用程式指定的那一個。
    pub fn save_extension(&self, type_index: usize) -> Option<String> {
        let default = self.default_ext.trim().trim_start_matches('.').trim();
        if default.is_empty() {
            return None;
        }
        let from_type = self.types.get(type_index).and_then(|t| spec_extension(&t.spec));
        Some(from_type.unwrap_or_else(|| default.to_string()))
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

// ---------------------------------------------------------------- 另存新檔的檔名

/// 類型的第一個具體副檔名：`*.txt;*.log` → `txt`。`*.*`、`*`、`*.tar.*` 這種沒有具體副檔名的回傳 None。
pub fn spec_extension(spec: &str) -> Option<String> {
    spec.split([';', ',', ' ']).map(str::trim).find_map(|p| {
        let ext = p.strip_prefix("*.")?;
        (!ext.is_empty() && !ext.contains(['*', '?', '.'])).then(|| ext.to_string())
    })
}

/// 檔名的副檔名（不含點）：`a.txt` → `txt`；`.gitignore`、`README`、`a.` 都沒有。
pub fn extension_of(name: &str) -> Option<&str> {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let dot = file.rfind('.')?;
    (dot > 0 && dot + 1 < file.len()).then(|| &file[dot + 1..])
}

/// 另存新檔時使用者打的名稱加上副檔名（照 Windows 的視窗的做法）：
/// - 加了引號（`"README"`）或結尾是點（`README.`）：照打的，不加副檔名。
/// - 沒有副檔名：加上 `ext`。
/// - 有副檔名但 `known` 說系統不認得（例如 `第 1.2 版` 的 `2 版`）：當成檔名的一部分，再加上 `ext`。
/// - `strict`（FOS_STRICTFILETYPES）：副檔名不符合選中的類型時也加上 `ext`。
///
/// `ext` 是 None（程式沒有指定預設副檔名）時一律照打的。
pub fn save_name(text: &str, ext: Option<&str>, strict: Option<&Filter>, known: impl Fn(&str) -> bool) -> String {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return t[1..t.len() - 1].trim().to_string();
    }
    if t.ends_with('.') {
        return t.trim_end_matches(['.', ' ']).to_string();
    }
    let Some(ext) = ext.map(|e| e.trim_start_matches('.')).filter(|e| !e.is_empty()) else { return t.to_string() };
    let file = t.rsplit(['\\', '/']).next().unwrap_or(t);
    let keep = match extension_of(t) {
        None => false,
        Some(e) if e.eq_ignore_ascii_case(ext) => true,
        Some(e) => strict.is_none_or(|f| f.matches(file, false)) && known(e),
    };
    if keep {
        t.to_string()
    } else {
        format!("{t}.{ext}")
    }
}

/// 換了檔案類型：檔名欄的副檔名是舊類型的就換成新的（`報告.txt` → `報告.html`）。沒有要換時回傳 None。
pub fn swap_extension(text: &str, old: Option<&str>, new: Option<&str>) -> Option<String> {
    let (old, new) = (old?, new?);
    let t = text.trim_end();
    let ext = extension_of(t)?;
    if !ext.eq_ignore_ascii_case(old) || ext == new {
        return None;
    }
    Some(format!("{}{}", &t[..t.len() - ext.len()], new))
}

/// 檔名不能用的字元（只看最後一段；前面的資料夾由 resolve 處理）
pub fn invalid_name_char(name: &str) -> Option<char> {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    file.chars().find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (*c as u32) < 32)
}

/// Windows 保留的裝置名稱（`CON`、`NUL`、`COM1.txt`…），不能當檔名
pub fn reserved_name(name: &str) -> bool {
    let file = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let stem = file.split('.').next().unwrap_or(file).trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0')
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
            overwrite_prompt: false,
            strict_types: false,
            no_readonly: false,
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

    /// 舊版（0.1.0 與 2026-11-22 最初發佈的 0.1.1）的取代元件寫出的請求（原樣保留，不要改）：之後的 Stillfeather 都必須讀得懂，
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
    fn save_requests() {
        let r = Request {
            mode: Mode::Save,
            file_name: "未命名.txt".into(),
            overwrite_prompt: true,
            strict_types: false,
            no_readonly: true,
            default_ext: "txt".into(),
            out: "C:\\o.txt".into(),
            ..Default::default()
        };
        let text = r.to_text();
        // 舊版 Stillfeather 不認得另存新檔：寫成版本 2，舊版讀到就改開 Windows 的視窗
        assert!(text.starts_with("stillfeather-pick=2\n"));
        assert_eq!(Request::parse(&text).unwrap(), r);
        // 開啟檔案的請求不寫這些欄位，版本也維持 1
        assert!(!sample().to_text().contains("overwrite="));
    }

    #[test]
    fn save_extensions() {
        let types = vec![
            FileType { name: "文字文件".into(), spec: "*.txt".into() },
            FileType { name: "網頁".into(), spec: "*.htm;*.html".into() },
            FileType { name: "全部檔案".into(), spec: "*.*".into() },
        ];
        let r = Request { mode: Mode::Save, types: types.clone(), default_ext: ".txt".into(), ..Default::default() };
        assert_eq!(r.save_extension(0).as_deref(), Some("txt"));
        assert_eq!(r.save_extension(1).as_deref(), Some("htm"));
        assert_eq!(r.save_extension(2).as_deref(), Some("txt"));
        // 程式沒有指定預設副檔名：不自動加（和 Windows 的視窗相同）
        let none = Request { mode: Mode::Save, types, ..Default::default() };
        assert_eq!(none.save_extension(0), None);
        assert_eq!(spec_extension("*.tar.*"), None);
        assert_eq!(spec_extension("*.*"), None);
        assert_eq!(spec_extension("readme;*.MD"), Some("MD".into()));

        assert_eq!(extension_of("a.txt"), Some("txt"));
        assert_eq!(extension_of("C:\\x.y\\README"), None);
        assert_eq!(extension_of(".gitignore"), None);
        assert_eq!(extension_of("a."), None);

        let known = |e: &str| ["txt", "html", "md", "png"].contains(&e.to_ascii_lowercase().as_str());
        let txt = Some("txt");
        assert_eq!(save_name("報告", txt, None, known), "報告.txt");
        assert_eq!(save_name("  報告.txt ", txt, None, known), "報告.txt");
        assert_eq!(save_name("報告.TXT", txt, None, known), "報告.TXT");
        assert_eq!(save_name("notes.md", txt, None, known), "notes.md");
        assert_eq!(save_name("第 1.2 版", txt, None, known), "第 1.2 版.txt");
        assert_eq!(save_name("\"README\"", txt, None, known), "README");
        assert_eq!(save_name("README.", txt, None, known), "README");
        assert_eq!(save_name("報告", None, None, known), "報告");
        assert_eq!(save_name("sub\\報告", txt, None, known), "sub\\報告.txt");
        // 嚴格的類型：副檔名不符合選中的類型就補上
        let strict = Filter::from_spec("*.txt");
        assert_eq!(save_name("notes.md", txt, Some(&strict), known), "notes.md.txt");
        assert_eq!(save_name("notes.txt", txt, Some(&strict), known), "notes.txt");

        assert_eq!(swap_extension("報告.txt", Some("txt"), Some("htm")).as_deref(), Some("報告.htm"));
        assert_eq!(swap_extension("報告.TXT", Some("txt"), Some("htm")).as_deref(), Some("報告.htm"));
        assert_eq!(swap_extension("報告.md", Some("txt"), Some("htm")), None);
        assert_eq!(swap_extension("報告", Some("txt"), Some("htm")), None);
        assert_eq!(swap_extension("報告.txt", None, Some("htm")), None);
    }

    #[test]
    fn file_name_checks() {
        assert_eq!(invalid_name_char("a?.txt"), Some('?'));
        assert_eq!(invalid_name_char("C:\\dir\\a.txt"), None);
        assert_eq!(invalid_name_char("a|b"), Some('|'));
        assert_eq!(invalid_name_char("正常的名稱 (1).txt"), None);
        assert!(reserved_name("CON"));
        assert!(reserved_name("nul.txt"));
        assert!(reserved_name("C:\\x\\com1.log"));
        assert!(!reserved_name("COM0"));
        assert!(!reserved_name("console.txt"));
        assert!(!reserved_name("LPT10"));
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
        let save = Request { mode: Mode::Save, ..Default::default() };
        assert_eq!(save.window_title(), "另存新檔");
        assert_eq!(save.ok_text(), "存檔");
        assert_eq!(save.name_text(), "檔案名稱");
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
        // 首頁（沒有目前的資料夾）：只有完整路徑才是完整的，其他都是相對路徑，呼叫的地方要擋下來
        assert_eq!(resolve("", "報告.txt"), "報告.txt");
        assert!(!paths::is_fully_qualified(&resolve("", "報告.txt")));
        assert_eq!(paths::parent(&resolve("", "報告.txt")), None);
        assert!(!paths::is_fully_qualified(&resolve("", "\\報告.txt")));
        assert!(paths::is_fully_qualified(&resolve("", "D:\\文件\\報告.txt")));
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

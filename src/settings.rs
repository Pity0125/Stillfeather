//! 使用者設定（`%LOCALAPPDATA%\Stillfeather\settings.json`）。欄位格式與 C# 雛形的
//! `%LOCALAPPDATA%\FeatherFiles\settings.json` 相容（舊版用的就是那一份，新版啟動時搬過來）。

use crate::json::{self, Value};
use crate::paths;
use crate::recent::{self, RecentSearch};
use crate::sidebar::{self, Section};
use crate::view::ViewMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::System => "System",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }

    pub fn parse(s: &str) -> Option<ThemeMode> {
        match s {
            "System" => Some(ThemeMode::System),
            "Light" => Some(ThemeMode::Light),
            "Dark" => Some(ThemeMode::Dark),
            _ => None,
        }
    }
}

/// 介面語言
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    /// 跟隨 Windows 的顯示語言：中文（繁體、簡體）用繁體中文，其他用英文
    #[default]
    System,
    Chinese,
    English,
}

impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Language::System => "System",
            Language::Chinese => "zh-TW",
            Language::English => "en",
        }
    }

    pub fn parse(s: &str) -> Option<Language> {
        match s {
            "System" => Some(Language::System),
            "zh-TW" | "zh" => Some(Language::Chinese),
            "en" | "en-US" => Some(Language::English),
            _ => None,
        }
    }

    /// 要不要用英文：`system_chinese` 是 Windows 的顯示語言是不是中文
    pub fn english(self, system_chinese: bool) -> bool {
        match self {
            Language::System => !system_chinese,
            Language::Chinese => false,
            Language::English => true,
        }
    }
}

pub const SIDEBAR_MIN: i32 = 130;
pub const SIDEBAR_MAX: i32 = 500;
pub const SPLIT_MIN: i32 = 25;
pub const SPLIT_MAX: i32 = 75;
pub const MAX_FAVORITES: usize = 100;
/// 文字大小（百分比）：檔名、欄位、側欄與狀態列的字跟著放大；100 是標準大小（9 點）
pub const TEXT_SIZES: [u32; 4] = [100, 115, 130, 150];
pub const TEXT_SIZE_MIN: u32 = 100;
pub const TEXT_SIZE_MAX: u32 = 200;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    /// 主題（`themes::PRESETS` 的名稱，Rust 版專用）。不認得的名稱照樣保留，畫面用預設主題
    pub theme: String,
    /// 「進階」的自訂重點色、背景色：設了就取代主題的顏色
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub sidebar_width: i32,
    pub split_percent: i32,
    pub compact_rows: bool,
    pub show_extensions: bool,
    /// 顯示隱藏的項目（整體設定，Rust 版專用）
    pub show_hidden: bool,
    /// 快速預覽的總開關（Rust 版專用）。關閉時滑鼠停留、空白鍵都不預覽；右鍵選單的「快速預覽」照常
    pub preview: bool,
    /// 以前「快速預覽圖片與影片」「快速預覽 PDF」的個別開關。設定視窗改成只有總開關（[`Settings::preview`]），
    /// 這兩個值不再使用，只為了讀寫 C# 版與舊的設定檔而保留
    pub image_preview: bool,
    pub pdf_preview: bool,
    /// 預覽方式：true 是空白鍵預覽（滑鼠停在項目上不自動預覽，按住空白鍵時才預覽滑鼠下的項目），
    /// false 是懸停預覽（Rust 版專用）
    pub space_preview: bool,
    pub favorites: Vec<String>,
    /// 側欄的自訂標籤（和「常用位置」「磁碟」同一層的大標籤）：名稱與放在裡面的資料夾（資料夾也列在 `favorites`）
    pub bookmark_groups: Vec<(String, Vec<String>)>,
    /// 側欄大標籤由上到下的順序（見 sidebar.rs；Rust 版專用）
    pub sidebar_order: Vec<String>,
    /// 收起來的側欄大標籤（名稱同 `sidebar_order`；Rust 版專用）
    pub collapsed_sections: Vec<String>,
    /// 最近搜尋，最新的在前面（見 recent.rs；Rust 版專用）
    pub recent_searches: Vec<RecentSearch>,
    pub shortcuts: Vec<(String, Vec<String>)>,
    /// 以前側欄底部「個人化」卡片是否收合。卡片已經拿掉（設定在右上角的齒輪），只為了讀寫舊的設定檔而保留
    pub personalize_collapsed: bool,
    /// 清單的檢視模式（Rust 版專用）
    pub view_mode: ViewMode,
    /// 有新版本時提醒（Rust 版專用）；設定裡的「不要啟用更新提醒」就是把它關掉
    pub update_reminders: bool,
    /// 啟動時還原上次的分頁（Rust 版專用，見 session.rs）
    pub restore_tabs: bool,
    /// 相片資料夾（圖片與影片佔一半以上）沒記住檢視時用大圖示顯示縮圖，和檔案總管相同（Rust 版專用）。
    /// 「套用到所有資料夾」時關掉：之後所有資料夾都用同一個檢視。
    pub media_folder_icons: bool,
    /// 介面語言（Rust 版專用；改了要重新開啟才套用）
    pub language: Language,
    /// 排序時資料夾一律在最前面（Rust 版專用）。關著時和檔案總管相同：遞增在前、遞減在後。
    pub folders_on_top: bool,
    /// 文字大小（百分比，Rust 版專用；見 [`TEXT_SIZES`]）
    pub text_size: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_mode: ThemeMode::System,
            theme: crate::themes::DEFAULT.to_string(),
            accent_color: None,
            background_color: None,
            sidebar_width: 240,
            split_percent: 50,
            compact_rows: false,
            show_extensions: true,
            show_hidden: false,
            preview: true,
            image_preview: true,
            pdf_preview: true,
            space_preview: false,
            favorites: Vec::new(),
            bookmark_groups: Vec::new(),
            sidebar_order: Vec::new(),
            collapsed_sections: Vec::new(),
            recent_searches: Vec::new(),
            shortcuts: Vec::new(),
            personalize_collapsed: false,
            view_mode: ViewMode::Details,
            update_reminders: true,
            restore_tabs: true,
            media_folder_icons: true,
            language: Language::System,
            folders_on_top: false,
            text_size: 100,
        }
    }
}

/// `#RRGGBB` → (r, g, b)
pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

pub fn color_to_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn string_lists(root: &Value, key: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let Some(value) = root.get(key) else { return Ok(Vec::new()) };
    let Value::Obj(entries) = value else { return Err(format!("Invalid {key}")) };
    entries.iter().map(|(name, value)| {
        let Value::Arr(list) = value else { return Err(format!("Invalid {key}: {name}")) };
        let strings = list.iter().map(|v| match v {
            Value::Str(s) => Ok(s.clone()),
            _ => Err(format!("Invalid {key}: {name}")),
        }).collect::<Result<Vec<_>, _>>()?;
        Ok((name.clone(), strings))
    }).collect()
}

fn encode_lists(entries: &[(String, Vec<String>)]) -> String {
    let fields = entries.iter().map(|(key, values)| {
        let values = values.iter().map(|v| json::quote(v)).collect::<Vec<_>>().join(", ");
        format!("{}: [{values}]", json::quote(key))
    }).collect::<Vec<_>>().join(", ");
    format!("{{{fields}}}")
}

impl Settings {
    fn group_names(&self) -> Vec<&str> {
        self.bookmark_groups.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// 側欄由上到下的大標籤（常用位置、自訂標籤、磁碟）
    pub fn sidebar_sections(&self) -> Vec<Section> {
        sidebar::sections(&self.sidebar_order, &self.group_names())
    }

    pub fn set_sidebar_sections(&mut self, sections: &[Section]) {
        self.sidebar_order = sidebar::order_names(sections, &self.group_names());
    }

    /// 新增自訂標籤，放在 `after` 那個標籤的下面（None：磁碟前面）。名稱有問題時回傳原因。
    pub fn add_group(&mut self, name: &str, after: Option<Section>) -> Result<usize, String> {
        let name = name.trim();
        if self.bookmark_groups.len() >= sidebar::MAX_GROUPS {
            return Err(trf!("側欄最多 {} 個自訂標籤。", "Up to {} custom sections are allowed.", sidebar::MAX_GROUPS));
        }
        if let Some(e) = sidebar::name_error(name, &self.group_names(), None) {
            return Err(e);
        }
        let mut sections = self.sidebar_sections();
        let index = self.bookmark_groups.len();
        self.bookmark_groups.push((name.to_string(), Vec::new()));
        let at = match after.and_then(|a| sections.iter().position(|s| *s == a)) {
            Some(i) => i + 1,
            None => sections.iter().position(|s| *s == Section::Drives).unwrap_or(sections.len()),
        };
        sections.insert(at, Section::Group(index));
        self.set_sidebar_sections(&sections);
        Ok(index)
    }

    pub fn rename_group(&mut self, index: usize, name: &str) -> Result<(), String> {
        let name = name.trim();
        if index >= self.bookmark_groups.len() {
            return Ok(());
        }
        if let Some(e) = sidebar::name_error(name, &self.group_names(), Some(index)) {
            return Err(e);
        }
        let sections = self.sidebar_sections();
        let collapsed = self.is_collapsed(Section::Group(index));
        self.set_collapsed(Section::Group(index), false);
        self.bookmark_groups[index].0 = name.to_string();
        self.set_sidebar_sections(&sections);
        self.set_collapsed(Section::Group(index), collapsed);
        Ok(())
    }

    /// 設定檔裡代表這個大標籤的名稱
    fn section_name(&self, section: Section) -> Option<String> {
        sidebar::order_names(&[section], &self.group_names()).into_iter().next()
    }

    /// 這個大標籤收起來了
    pub fn is_collapsed(&self, section: Section) -> bool {
        self.section_name(section).is_some_and(|n| self.collapsed_sections.iter().any(|c| c.to_lowercase() == n.to_lowercase()))
    }

    /// 記下大標籤收起來或展開；回傳有沒有變
    pub fn set_collapsed(&mut self, section: Section, collapsed: bool) -> bool {
        if self.is_collapsed(section) == collapsed {
            return false;
        }
        let Some(name) = self.section_name(section) else { return false };
        if collapsed {
            self.collapsed_sections.push(name);
        } else {
            self.collapsed_sections.retain(|c| c.to_lowercase() != name.to_lowercase());
        }
        true
    }

    /// 刪除自訂標籤：裡面的資料夾移回「常用位置」（資料夾本身與釘選都還在）
    pub fn remove_group(&mut self, index: usize) {
        if index >= self.bookmark_groups.len() {
            return;
        }
        self.set_collapsed(Section::Group(index), false);
        let sections: Vec<Section> = self
            .sidebar_sections()
            .into_iter()
            .filter(|s| *s != Section::Group(index))
            .map(|s| match s {
                Section::Group(i) if i > index => Section::Group(i - 1),
                other => other,
            })
            .collect();
        self.bookmark_groups.remove(index);
        self.set_sidebar_sections(&sections);
    }

    /// 把資料夾放進第 `group` 個自訂標籤（None：移回「常用位置」）；不在常用位置的不動
    pub fn move_bookmark(&mut self, path: &str, group: Option<usize>) {
        for (_, members) in &mut self.bookmark_groups {
            members.retain(|p| !paths::eq_ignore_case(p, path));
        }
        if let Some((_, members)) = group.and_then(|g| self.bookmark_groups.get_mut(g)) {
            members.push(path.to_string());
        }
    }

    /// 資料夾在哪個自訂標籤裡（None：在「常用位置」或沒有釘選）
    pub fn group_of(&self, path: &str) -> Option<usize> {
        self.bookmark_groups.iter().position(|(_, members)| members.iter().any(|p| paths::eq_ignore_case(p, path)))
    }

    /// 與 C# `SettingsService.Validate` 相同的規則；常用位置會去除重複（不分大小寫）。
    pub fn validate(&mut self) -> Result<(), String> {
        if !(SIDEBAR_MIN..=SIDEBAR_MAX).contains(&self.sidebar_width) {
            return Err(trf!("側欄寬度需介於 {SIDEBAR_MIN} 與 {SIDEBAR_MAX}。", "The sidebar width must be between {SIDEBAR_MIN} and {SIDEBAR_MAX}."));
        }
        if !(SPLIT_MIN..=SPLIT_MAX).contains(&self.split_percent) {
            return Err(trf!("左欄比例需介於 {SPLIT_MIN}% 與 {SPLIT_MAX}%。", "The left pane ratio must be between {SPLIT_MIN}% and {SPLIT_MAX}%."));
        }
        for c in [&self.accent_color, &self.background_color].into_iter().flatten() {
            if parse_color(c).is_none() {
                return Err(tr!("自訂顏色必須是 #RRGGBB，或留白使用預設。", "Custom colors must be #RRGGBB, or leave them blank to use the default.").to_string());
            }
        }
        if !(TEXT_SIZE_MIN..=TEXT_SIZE_MAX).contains(&self.text_size) {
            return Err(trf!("文字大小需介於 {TEXT_SIZE_MIN}% 與 {TEXT_SIZE_MAX}%。", "The text size must be between {TEXT_SIZE_MIN}% and {TEXT_SIZE_MAX}%."));
        }
        if self.favorites.len() > MAX_FAVORITES {
            return Err(trf!("常用位置最多 {MAX_FAVORITES} 個。", "Up to {MAX_FAVORITES} favorites are allowed."));
        }
        if self.favorites.iter().any(|p| p.trim().is_empty() || !paths::is_fully_qualified(p)) {
            return Err(tr!("常用位置必須是完整資料夾路徑。", "Favorites must be full folder paths.").to_string());
        }
        let mut unique: Vec<String> = Vec::with_capacity(self.favorites.len());
        for f in self.favorites.drain(..) {
            if !unique.iter().any(|u| paths::eq_ignore_case(u, &f)) {
                unique.push(f);
            }
        }
        self.favorites = unique;
        Ok(())
    }

    pub fn from_json(text: &str) -> Result<Settings, String> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(tr!("設定檔超過大小限制。", "The settings file is larger than the size limit.").to_string());
        }
        let root = json::parse(text)?;
        if !matches!(root, Value::Obj(_)) {
            return Err(tr!("設定檔內容必須是 JSON 物件。", "The settings file must contain a JSON object.").to_string());
        }
        let mut s = Settings::default();
        if let Some(v) = root.get("ThemeMode") {
            match v {
                Value::Str(m) => {
                    s.theme_mode = ThemeMode::parse(m).ok_or(tr!("主題模式必須為跟隨系統、淺色或深色。", "The theme must be System, Light or Dark."))?;
                }
                Value::Null => {}
                _ => return Err(tr!("ThemeMode 必須是字串。", "ThemeMode must be a string.").into()),
            }
        }
        match root.get("Theme") {
            None | Some(Value::Null) => {}
            // 不認得的主題（更新的版本加的）照樣保留，顯示時用預設主題
            Some(Value::Str(t)) if !t.trim().is_empty() => s.theme = t.trim().to_string(),
            Some(Value::Str(_)) => {}
            Some(_) => return Err(tr!("Theme 必須是字串。", "Theme must be a string.").into()),
        }
        s.accent_color = opt_string(&root, "AccentColor")?;
        s.background_color = opt_string(&root, "BackgroundColor")?;
        if let Some(n) = opt_int(&root, "SidebarWidth")? {
            s.sidebar_width = n;
        }
        if let Some(n) = opt_int(&root, "SplitPercent")? {
            s.split_percent = n;
        }
        if let Some(n) = opt_int(&root, "TextSize")? {
            s.text_size = n.max(0) as u32;
        }
        for (key, slot) in [
            ("CompactRows", &mut s.compact_rows),
            ("ShowExtensions", &mut s.show_extensions),
            ("ShowHidden", &mut s.show_hidden),
            ("Preview", &mut s.preview),
            ("ImagePreview", &mut s.image_preview),
            ("PdfPreview", &mut s.pdf_preview),
            ("SpacePreview", &mut s.space_preview),
            ("PersonalizeCollapsed", &mut s.personalize_collapsed),
            ("UpdateReminders", &mut s.update_reminders),
            ("RestoreTabs", &mut s.restore_tabs),
            ("MediaFolderIcons", &mut s.media_folder_icons),
            ("FoldersOnTop", &mut s.folders_on_top),
        ] {
            match root.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Bool(b)) => *slot = *b,
                Some(_) => return Err(trf!("{key} 必須是 true 或 false。", "{key} must be true or false.")),
            }
        }
        match root.get("Language") {
            None | Some(Value::Null) => {}
            // 不認得的語言（例如更新的版本加的）當成跟隨系統，不讓整個設定檔讀不進來
            Some(Value::Str(l)) => s.language = Language::parse(l).unwrap_or_default(),
            Some(_) => return Err(tr!("Language 必須是字串。", "Language must be a string.").into()),
        }
        match root.get("ViewMode") {
            None | Some(Value::Null) => {}
            Some(Value::Str(m)) => s.view_mode = ViewMode::parse(m).ok_or(tr!("ViewMode 不是可用的檢視模式。", "ViewMode isn't an available view."))?,
            Some(_) => return Err(tr!("ViewMode 必須是字串。", "ViewMode must be a string.").into()),
        }
        match root.get("Favorites") {
            None | Some(Value::Null) => {}
            Some(Value::Arr(items)) => {
                for item in items {
                    match item {
                        Value::Str(p) => s.favorites.push(p.clone()),
                        _ => return Err(tr!("常用位置必須是字串陣列。", "Favorites must be an array of strings.").into()),
                    }
                }
            }
            Some(_) => return Err(tr!("常用位置必須是字串陣列。", "Favorites must be an array of strings.").into()),
        }
        s.bookmark_groups = string_lists(&root, "BookmarkGroups")?;
        match root.get("SidebarOrder") {
            None | Some(Value::Null) => {}
            Some(Value::Arr(items)) => {
                for item in items.iter().take(sidebar::MAX_GROUPS + 2) {
                    if let Value::Str(name) = item {
                        s.sidebar_order.push(name.clone());
                    }
                }
            }
            Some(_) => return Err(tr!("SidebarOrder 必須是字串陣列。", "SidebarOrder must be an array of strings.").into()),
        }
        // 收起來的標籤只是外觀：看不懂就當成都展開
        if let Some(Value::Arr(items)) = root.get("CollapsedSections") {
            for item in items.iter().take(sidebar::MAX_GROUPS + 2) {
                if let Value::Str(name) = item {
                    s.collapsed_sections.push(name.clone());
                }
            }
        }
        // 最近搜尋只是方便用的紀錄：看不懂的那一筆略過，不讓整個設定檔讀不進來
        s.recent_searches = recent::from_json(root.get("RecentSearches"));
        s.shortcuts = string_lists(&root, "Shortcuts")?;
        s.validate()?;
        Ok(s)
    }

    pub fn to_json(&self) -> String {
        let sidebar_order = format!("[{}]", self.sidebar_order.iter().map(|n| json::quote(n)).collect::<Vec<_>>().join(", "));
        let collapsed = format!("[{}]", self.collapsed_sections.iter().map(|n| json::quote(n)).collect::<Vec<_>>().join(", "));
        let opt = |v: &Option<String>| v.as_deref().map(json::quote).unwrap_or_else(|| "null".to_string());
        let favorites = if self.favorites.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = self.favorites.iter().map(|f| format!("    {}", json::quote(f))).collect();
            format!("[\n{}\n  ]", items.join(",\n"))
        };
        format!(
            "{{\n  \"ThemeMode\": {},\n  \"Theme\": {},\n  \"AccentColor\": {},\n  \"BackgroundColor\": {},\n  \"SidebarWidth\": {},\n  \"SplitPercent\": {},\n  \"CompactRows\": {},\n  \"ShowExtensions\": {},\n  \"ShowHidden\": {},\n  \"Preview\": {},\n  \"ImagePreview\": {},\n  \"PdfPreview\": {},\n  \"SpacePreview\": {},\n  \"PersonalizeCollapsed\": {},\n  \"ViewMode\": {},\n  \"UpdateReminders\": {},\n  \"RestoreTabs\": {},\n  \"MediaFolderIcons\": {},\n  \"Language\": {},\n  \"FoldersOnTop\": {},\n  \"TextSize\": {},\n  \"BookmarkGroups\": {},\n  \"SidebarOrder\": {},\n  \"CollapsedSections\": {},\n  \"RecentSearches\": {},\n  \"Shortcuts\": {},\n  \"Favorites\": {}\n}}",
            json::quote(self.theme_mode.as_str()),
            json::quote(&self.theme),
            opt(&self.accent_color),
            opt(&self.background_color),
            self.sidebar_width,
            self.split_percent,
            self.compact_rows,
            self.show_extensions,
            self.show_hidden,
            self.preview,
            self.image_preview,
            self.pdf_preview,
            self.space_preview,
            self.personalize_collapsed,
            json::quote(self.view_mode.as_str()),
            self.update_reminders,
            self.restore_tabs,
            self.media_folder_icons,
            json::quote(self.language.as_str()),
            self.folders_on_top,
            self.text_size,
            encode_lists(&self.bookmark_groups),
            sidebar_order,
            collapsed,
            recent::to_json(&self.recent_searches),
            encode_lists(&self.shortcuts),
            favorites
        )
    }
}

fn opt_string(root: &Value, key: &str) -> Result<Option<String>, String> {
    match root.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(trf!("{key} 必須是字串或 null。", "{key} must be a string or null.")),
    }
}

fn opt_int(root: &Value, key: &str) -> Result<Option<i32>, String> {
    match root.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Num(n)) if n.fract() == 0.0 && n.abs() < 1e9 => Ok(Some(*n as i32)),
        Some(_) => Err(trf!("{key} 必須是整數。", "{key} must be an integer.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip() {
        let s = Settings::default();
        let back = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn full_roundtrip_with_unicode() {
        let s = Settings {
            theme_mode: ThemeMode::Dark,
            theme: "sage".into(),
            accent_color: Some("#60CDFF".into()),
            background_color: None,
            sidebar_width: 260,
            split_percent: 40,
            compact_rows: true,
            show_extensions: false,
            show_hidden: true,
            preview: false,
            image_preview: false,
            pdf_preview: true,
            space_preview: true,
            favorites: vec!["C:\\Users\\pity0\\桌面\\專案".into(), "\\\\nas\\共用".into()],
            bookmark_groups: vec![("專案".into(), vec![r"\\nas\共用".into()])],
            sidebar_order: vec!["專案".into(), "::favorites".into(), "::drives".into()],
            collapsed_sections: vec!["::drives".into()],
            recent_searches: vec![crate::recent::RecentSearch {
                text: "報告".into(),
                global: false,
                recursive: false,
                category: crate::category::Category::Document,
                date: Some(crate::searchquery::DateSpec::ThisWeek),
                size: None,
            }],
            shortcuts: vec![("new-tab".into(), vec!["Ctrl+K".into()]), ("refresh".into(), vec![])],
            personalize_collapsed: true,
            view_mode: ViewMode::Tiles,
            update_reminders: false,
            restore_tabs: false,
            media_folder_icons: false,
            language: Language::English,
            folders_on_top: true,
            text_size: 130,
        };
        let text = s.to_json();
        assert!(text.contains("專案"));
        assert_eq!(Settings::from_json(&text).unwrap(), s);
    }

    #[test]
    fn reads_csharp_file_with_escapes() {
        let text = "{\n  \"ThemeMode\": \"Light\",\n  \"AccentColor\": \"#005FB8\",\n  \"BackgroundColor\": null,\n  \"SidebarWidth\": 200,\n  \"SplitPercent\": 50,\n  \"CompactRows\": false,\n  \"ShowExtensions\": true,\n  \"ImagePreview\": true,\n  \"PdfPreview\": true,\n  \"Favorites\": [\n    \"C:\\\\Users\\\\pity0\\\\Desktop\\\\win11\",\n    \"C:\\\\\\u8CC7\\u6599\",\n    \"c:\\\\users\\\\PITY0\\\\desktop\\\\WIN11\"\n  ]\n}";
        let s = Settings::from_json(text).unwrap();
        assert_eq!(s.theme_mode, ThemeMode::Light);
        assert_eq!(s.favorites, vec!["C:\\Users\\pity0\\Desktop\\win11".to_string(), "C:\\資料".to_string()]);
        // 舊的設定檔沒有這兩個欄位：用預設值（還原分頁、相片資料夾用大圖示都開著）
        assert!(s.restore_tabs && s.media_folder_icons);
        assert_eq!(s.language, Language::System);
        // 舊的設定檔沒有 FoldersOnTop：和檔案總管相同（遞減時資料夾在後面）
        assert!(!s.folders_on_top);
        // 舊的設定檔沒有 TextSize：標準大小
        assert_eq!(s.text_size, 100);
        // 舊的設定檔沒有 Theme：預設主題（自訂的重點色照樣保留，在「進階」）
        assert_eq!(s.theme, crate::themes::DEFAULT);
        assert_eq!(s.accent_color.as_deref(), Some("#005FB8"));
    }

    #[test]
    fn sidebar_sections_follow_groups() {
        use crate::sidebar::Section::*;
        let mut s = Settings { favorites: vec![r"C:\工作".into(), r"D:\照片".into()], ..Settings::default() };
        assert_eq!(s.sidebar_sections(), vec![Favorites, Drives]);
        // 從空白處新增：放在磁碟前面；從某個標籤新增：放在它下面
        assert_eq!(s.add_group("工作", None), Ok(0));
        assert_eq!(s.add_group("照片", Some(Favorites)), Ok(1));
        assert_eq!(s.sidebar_sections(), vec![Favorites, Group(1), Group(0), Drives]);
        assert!(s.add_group("照片", None).is_err());
        s.move_bookmark(r"C:\工作", Some(0));
        s.move_bookmark(r"d:\照片", Some(1));
        assert_eq!(s.group_of(r"c:\工作"), Some(0));
        s.move_bookmark(r"C:\工作", Some(1));
        assert_eq!(s.group_of(r"C:\工作"), Some(1));
        // 收起來：存的是名稱，改名時跟著改
        assert!(s.set_collapsed(Group(1), true));
        assert!(!s.set_collapsed(Group(1), true));
        assert!(s.set_collapsed(Drives, true));
        assert!(s.is_collapsed(Group(1)) && s.is_collapsed(Drives) && !s.is_collapsed(Favorites));
        // 改名：順序與收起來的狀態跟著
        s.rename_group(1, "相片").unwrap();
        assert!(s.is_collapsed(Group(1)));
        assert_eq!(s.collapsed_sections, vec!["::drives".to_string(), "相片".into()]);
        assert_eq!(s.sidebar_order, vec!["::favorites".to_string(), "相片".into(), "工作".into(), "::drives".into()]);
        assert!(s.rename_group(1, "工作").is_err());
        // 刪掉第一個：後面的索引往前，裡面的資料夾回到常用位置（還在 favorites）
        s.remove_group(0);
        assert_eq!(s.bookmark_groups.len(), 1);
        assert_eq!(s.sidebar_sections(), vec![Favorites, Group(0), Drives]);
        s.remove_group(0);
        assert_eq!(s.group_of(r"C:\工作"), None);
        // 刪掉的標籤不再記著收起來
        assert_eq!(s.collapsed_sections, vec!["::drives".to_string()]);
        assert_eq!(s.favorites.len(), 2);
        let back = Settings::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        // 看不懂的最近搜尋略過，不影響其他設定
        let t = Settings::from_json("{\"RecentSearches\": [{\"Text\": 1}, {\"Text\": \"報告\"}], \"SidebarOrder\": [\"::drives\"]}").unwrap();
        assert_eq!(t.recent_searches.len(), 1);
        assert_eq!(t.sidebar_sections(), vec![Favorites, Drives]);
        assert!(Settings::from_json("{\"SidebarOrder\": 3}").is_err());
    }

    #[test]
    fn preview_switch() {
        // 舊的設定檔沒有 Preview：預覽開著，預覽方式照舊
        let s = Settings::from_json("{\"SpacePreview\":true}").unwrap();
        assert!(s.preview && s.space_preview);
        let s = Settings::from_json("{\"Preview\":false}").unwrap();
        assert!(!s.preview && !s.space_preview);
        assert!(Settings::from_json("{\"Preview\":1}").is_err());
    }

    #[test]
    fn themes() {
        assert_eq!(Settings::from_json("{\"Theme\":\"teal\"}").unwrap().theme, "teal");
        // 不認得的主題照樣保留（更新的版本寫的），顯示時用預設主題
        assert_eq!(Settings::from_json("{\"Theme\":\"aurora\"}").unwrap().theme, "aurora");
        assert_eq!(Settings::from_json("{\"Theme\":\"  \"}").unwrap().theme, crate::themes::DEFAULT);
        assert!(Settings::from_json("{\"Theme\":3}").is_err());
    }

    /// 設定視窗的「不要啟用更新提醒」改成正向的「啟動時提醒更新」：存的一直是 UpdateReminders，
    /// 以前勾了「不要啟用」的人（false）在新的勾選方塊上就是沒勾，選擇不會變。
    #[test]
    fn update_reminder_choice_is_kept() {
        assert!(!Settings::from_json("{\"UpdateReminders\": false}").unwrap().update_reminders);
        assert!(Settings::from_json("{\"UpdateReminders\": true}").unwrap().update_reminders);
        // 沒有這個欄位（更早的版本）：提醒
        assert!(Settings::from_json("{}").unwrap().update_reminders);
        let s = Settings { update_reminders: false, ..Settings::default() };
        assert!(!Settings::from_json(&s.to_json()).unwrap().update_reminders);
    }

    #[test]
    fn languages() {
        assert_eq!(Settings::from_json("{\"Language\":\"en\"}").unwrap().language, Language::English);
        assert_eq!(Settings::from_json("{\"Language\":\"zh-TW\"}").unwrap().language, Language::Chinese);
        // 不認得的語言：跟隨系統，設定檔其他部分照常讀
        assert_eq!(Settings::from_json("{\"Language\":\"fr\",\"CompactRows\":true}").unwrap().language, Language::System);
        assert!(Settings::from_json("{\"Language\":1}").is_err());
        assert!(Language::System.english(false));
        assert!(!Language::System.english(true));
        assert!(!Language::Chinese.english(false));
        assert!(Language::English.english(true));
    }

    #[test]
    fn validation_rules() {
        let bad = [
            "{\"ThemeMode\":\"Blue\"}",
            "{\"SidebarWidth\":100}",
            "{\"SidebarWidth\":501}",
            "{\"SplitPercent\":80}",
            "{\"AccentColor\":\"red\"}",
            "{\"Favorites\":[\"relative\\\\path\"]}",
            "{\"Favorites\":[1]}",
            "{\"CompactRows\":\"yes\"}",
            "{\"ViewMode\":\"Huge\"}",
            "{\"ViewMode\":3}",
            "{\"TextSize\":90}",
            "{\"TextSize\":250}",
            "{\"TextSize\":\"big\"}",
            "[1,2]",
        ];
        for b in bad {
            assert!(Settings::from_json(b).is_err(), "{b} should fail");
        }
        let many: Vec<String> = (0..101).map(|i| format!("\"C:\\\\f{i}\"")).collect();
        assert!(Settings::from_json(&format!("{{\"Favorites\":[{}]}}", many.join(","))).is_err());
        // 空字串顏色視同未設定
        assert_eq!(Settings::from_json("{\"AccentColor\":\"\"}").unwrap().accent_color, None);
        // 未知欄位忽略
        assert!(Settings::from_json("{\"Future\":123}").is_ok());
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#00ff7F"), Some((0, 255, 127)));
        assert_eq!(parse_color("00ff7F"), None);
        assert_eq!(parse_color("#00ff7"), None);
        assert_eq!(color_to_hex(0, 95, 184), "#005FB8");
    }
}

//! 使用者設定（`%LOCALAPPDATA%\Stillfeather\settings.json`）。欄位格式與 C# 雛形的
//! `%LOCALAPPDATA%\FeatherFiles\settings.json` 相容（舊版用的就是那一份，新版啟動時搬過來）。

use crate::json::{self, Value};
use crate::paths;
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

pub const SIDEBAR_MIN: i32 = 130;
pub const SIDEBAR_MAX: i32 = 500;
pub const SPLIT_MIN: i32 = 25;
pub const SPLIT_MAX: i32 = 75;
pub const MAX_FAVORITES: usize = 100;
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    pub accent_color: Option<String>,
    pub background_color: Option<String>,
    pub sidebar_width: i32,
    pub split_percent: i32,
    pub compact_rows: bool,
    pub show_extensions: bool,
    /// 顯示隱藏的項目（整體設定，Rust 版專用）
    pub show_hidden: bool,
    pub image_preview: bool,
    pub pdf_preview: bool,
    /// 按空白鍵才預覽：滑鼠停在項目上不自動預覽，按住空白鍵時才預覽滑鼠下的項目（Rust 版專用）
    pub space_preview: bool,
    pub favorites: Vec<String>,
    /// 側欄底部的「個人化」卡片是否收合（Rust 版專用；C# 版會忽略這個欄位）
    pub personalize_collapsed: bool,
    /// 清單的檢視模式（Rust 版專用）
    pub view_mode: ViewMode,
    /// 有新版本時提醒（Rust 版專用）；設定裡的「不要啟用更新提醒」就是把它關掉
    pub update_reminders: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_mode: ThemeMode::System,
            accent_color: None,
            background_color: None,
            sidebar_width: 240,
            split_percent: 50,
            compact_rows: false,
            show_extensions: true,
            show_hidden: false,
            image_preview: true,
            pdf_preview: true,
            space_preview: false,
            favorites: Vec::new(),
            personalize_collapsed: false,
            view_mode: ViewMode::Details,
            update_reminders: true,
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

impl Settings {
    /// 與 C# `SettingsService.Validate` 相同的規則；常用位置會去除重複（不分大小寫）。
    pub fn validate(&mut self) -> Result<(), String> {
        if !(SIDEBAR_MIN..=SIDEBAR_MAX).contains(&self.sidebar_width) {
            return Err(format!("側欄寬度需介於 {SIDEBAR_MIN} 與 {SIDEBAR_MAX}。"));
        }
        if !(SPLIT_MIN..=SPLIT_MAX).contains(&self.split_percent) {
            return Err(format!("左欄比例需介於 {SPLIT_MIN}% 與 {SPLIT_MAX}%。"));
        }
        for c in [&self.accent_color, &self.background_color].into_iter().flatten() {
            if parse_color(c).is_none() {
                return Err("自訂顏色必須是 #RRGGBB，或留白使用預設。".to_string());
            }
        }
        if self.favorites.len() > MAX_FAVORITES {
            return Err(format!("常用位置最多 {MAX_FAVORITES} 個。"));
        }
        if self.favorites.iter().any(|p| p.trim().is_empty() || !paths::is_fully_qualified(p)) {
            return Err("常用位置必須是完整資料夾路徑。".to_string());
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
            return Err("設定檔超過大小限制。".to_string());
        }
        let root = json::parse(text)?;
        if !matches!(root, Value::Obj(_)) {
            return Err("設定檔內容必須是 JSON 物件。".to_string());
        }
        let mut s = Settings::default();
        if let Some(v) = root.get("ThemeMode") {
            match v {
                Value::Str(m) => {
                    s.theme_mode = ThemeMode::parse(m).ok_or("主題模式必須為跟隨系統、淺色或深色。")?;
                }
                Value::Null => {}
                _ => return Err("ThemeMode 必須是字串。".into()),
            }
        }
        s.accent_color = opt_string(&root, "AccentColor")?;
        s.background_color = opt_string(&root, "BackgroundColor")?;
        if let Some(n) = opt_int(&root, "SidebarWidth")? {
            s.sidebar_width = n;
        }
        if let Some(n) = opt_int(&root, "SplitPercent")? {
            s.split_percent = n;
        }
        for (key, slot) in [
            ("CompactRows", &mut s.compact_rows),
            ("ShowExtensions", &mut s.show_extensions),
            ("ShowHidden", &mut s.show_hidden),
            ("ImagePreview", &mut s.image_preview),
            ("PdfPreview", &mut s.pdf_preview),
            ("SpacePreview", &mut s.space_preview),
            ("PersonalizeCollapsed", &mut s.personalize_collapsed),
            ("UpdateReminders", &mut s.update_reminders),
        ] {
            match root.get(key) {
                None | Some(Value::Null) => {}
                Some(Value::Bool(b)) => *slot = *b,
                Some(_) => return Err(format!("{key} 必須是 true 或 false。")),
            }
        }
        match root.get("ViewMode") {
            None | Some(Value::Null) => {}
            Some(Value::Str(m)) => s.view_mode = ViewMode::parse(m).ok_or("ViewMode 不是可用的檢視模式。")?,
            Some(_) => return Err("ViewMode 必須是字串。".into()),
        }
        match root.get("Favorites") {
            None | Some(Value::Null) => {}
            Some(Value::Arr(items)) => {
                for item in items {
                    match item {
                        Value::Str(p) => s.favorites.push(p.clone()),
                        _ => return Err("常用位置必須是字串陣列。".into()),
                    }
                }
            }
            Some(_) => return Err("常用位置必須是字串陣列。".into()),
        }
        s.validate()?;
        Ok(s)
    }

    pub fn to_json(&self) -> String {
        let opt = |v: &Option<String>| v.as_deref().map(json::quote).unwrap_or_else(|| "null".to_string());
        let favorites = if self.favorites.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = self.favorites.iter().map(|f| format!("    {}", json::quote(f))).collect();
            format!("[\n{}\n  ]", items.join(",\n"))
        };
        format!(
            "{{\n  \"ThemeMode\": {},\n  \"AccentColor\": {},\n  \"BackgroundColor\": {},\n  \"SidebarWidth\": {},\n  \"SplitPercent\": {},\n  \"CompactRows\": {},\n  \"ShowExtensions\": {},\n  \"ShowHidden\": {},\n  \"ImagePreview\": {},\n  \"PdfPreview\": {},\n  \"SpacePreview\": {},\n  \"PersonalizeCollapsed\": {},\n  \"ViewMode\": {},\n  \"UpdateReminders\": {},\n  \"Favorites\": {}\n}}",
            json::quote(self.theme_mode.as_str()),
            opt(&self.accent_color),
            opt(&self.background_color),
            self.sidebar_width,
            self.split_percent,
            self.compact_rows,
            self.show_extensions,
            self.show_hidden,
            self.image_preview,
            self.pdf_preview,
            self.space_preview,
            self.personalize_collapsed,
            json::quote(self.view_mode.as_str()),
            self.update_reminders,
            favorites
        )
    }
}

fn opt_string(root: &Value, key: &str) -> Result<Option<String>, String> {
    match root.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{key} 必須是字串或 null。")),
    }
}

fn opt_int(root: &Value, key: &str) -> Result<Option<i32>, String> {
    match root.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Num(n)) if n.fract() == 0.0 && n.abs() < 1e9 => Ok(Some(*n as i32)),
        Some(_) => Err(format!("{key} 必須是整數。")),
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
            accent_color: Some("#60CDFF".into()),
            background_color: None,
            sidebar_width: 260,
            split_percent: 40,
            compact_rows: true,
            show_extensions: false,
            show_hidden: true,
            image_preview: false,
            pdf_preview: true,
            space_preview: true,
            favorites: vec!["C:\\Users\\pity0\\桌面\\專案".into(), "\\\\nas\\共用".into()],
            personalize_collapsed: true,
            view_mode: ViewMode::Tiles,
            update_reminders: false,
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

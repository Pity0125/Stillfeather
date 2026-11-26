//! 預先調好的主題：每一組都有淺色與深色兩套配色（底色、卡片、側欄、文字、重點色）。
//!
//! 底色只帶一點點主題的色調，顏色主要表現在重點色（選取、底線、主要按鈕）；這樣換主題看得出差別，
//! 又不會因為整片背景染色而難以閱讀。每一組都經過對比檢查（見測試）：文字對底色至少 7:1、
//! 重點色對底色與卡片至少 3:1（底線、選取框看得到）、主要按鈕上的文字至少 4:1。
//!
//! 使用者可以在「進階」自訂重點色與背景色，取代主題的顏色（見 `win::theme::Palette::from_settings`）。
//! 不呼叫 Win32，顏色以 (r, g, b) 表示。

pub type Rgb = (u8, u8, u8);

/// 一套配色（淺色或深色）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colors {
    /// 頁面底色
    pub background: Rgb,
    /// 卡片（清單、對話方塊、搜尋框）
    pub surface: Rgb,
    /// 側欄卡片
    pub sidebar: Rgb,
    /// 主要文字
    pub ink: Rgb,
    /// 重點色
    pub accent: Rgb,
}

#[derive(Clone, Copy, Debug)]
pub struct Preset {
    /// 存在設定檔的名稱（`Theme`）
    pub id: &'static str,
    pub name_zh: &'static str,
    pub name_en: &'static str,
    pub light: Colors,
    pub dark: Colors,
}

impl Preset {
    pub fn name(&self) -> &'static str {
        if crate::i18n::english() {
            self.name_en
        } else {
            self.name_zh
        }
    }

    pub fn colors(&self, dark: bool) -> &Colors {
        if dark {
            &self.dark
        } else {
            &self.light
        }
    }
}

/// 預設主題（陶土：米白底、陶土橘）
pub const DEFAULT: &str = "clay";

pub const PRESETS: [Preset; 8] = [
    Preset {
        id: "clay",
        name_zh: "陶土",
        name_en: "Clay",
        light: Colors { background: (248, 246, 242), surface: (255, 255, 255), sidebar: (240, 237, 232), ink: (46, 43, 41), accent: (210, 118, 86) },
        dark: Colors { background: (18, 18, 18), surface: (30, 30, 30), sidebar: (25, 25, 25), ink: (236, 234, 231), accent: (232, 150, 118) },
    },
    Preset {
        id: "sage",
        name_zh: "鼠尾草",
        name_en: "Sage",
        light: Colors { background: (244, 246, 241), surface: (255, 255, 255), sidebar: (232, 237, 229), ink: (37, 44, 38), accent: (70, 128, 86) },
        dark: Colors { background: (16, 20, 17), surface: (27, 32, 28), sidebar: (21, 26, 22), ink: (229, 236, 230), accent: (132, 190, 146) },
    },
    Preset {
        id: "harbor",
        name_zh: "海港",
        name_en: "Harbor",
        light: Colors { background: (243, 246, 249), surface: (255, 255, 255), sidebar: (231, 237, 244), ink: (31, 40, 52), accent: (44, 110, 190) },
        dark: Colors { background: (14, 18, 24), surface: (24, 30, 39), sidebar: (19, 24, 32), ink: (227, 234, 243), accent: (112, 166, 234) },
    },
    Preset {
        id: "lavender",
        name_zh: "薰衣草",
        name_en: "Lavender",
        light: Colors { background: (246, 244, 250), surface: (255, 255, 255), sidebar: (236, 232, 245), ink: (42, 36, 54), accent: (118, 88, 190) },
        dark: Colors { background: (19, 17, 25), surface: (30, 27, 40), sidebar: (24, 22, 32), ink: (236, 232, 244), accent: (174, 150, 236) },
    },
    Preset {
        id: "rose",
        name_zh: "玫瑰",
        name_en: "Rose",
        light: Colors { background: (250, 244, 245), surface: (255, 255, 255), sidebar: (244, 233, 236), ink: (52, 37, 42), accent: (186, 72, 104) },
        dark: Colors { background: (23, 16, 18), surface: (36, 26, 29), sidebar: (29, 21, 24), ink: (244, 232, 235), accent: (236, 132, 160) },
    },
    Preset {
        id: "amber",
        name_zh: "琥珀",
        name_en: "Amber",
        light: Colors { background: (249, 247, 239), surface: (255, 255, 255), sidebar: (242, 237, 224), ink: (46, 41, 30), accent: (160, 112, 20) },
        dark: Colors { background: (20, 18, 14), surface: (32, 29, 23), sidebar: (26, 24, 19), ink: (241, 236, 224), accent: (226, 178, 84) },
    },
    Preset {
        id: "teal",
        name_zh: "湖水綠",
        name_en: "Teal",
        light: Colors { background: (241, 247, 246), surface: (255, 255, 255), sidebar: (228, 239, 238), ink: (28, 44, 44), accent: (18, 128, 128) },
        dark: Colors { background: (13, 20, 21), surface: (22, 33, 34), sidebar: (17, 26, 27), ink: (225, 238, 237), accent: (86, 196, 192) },
    },
    Preset {
        id: "graphite",
        name_zh: "石墨",
        name_en: "Graphite",
        light: Colors { background: (244, 244, 245), surface: (255, 255, 255), sidebar: (233, 234, 236), ink: (30, 31, 34), accent: (74, 88, 112) },
        dark: Colors { background: (13, 13, 14), surface: (25, 25, 27), sidebar: (19, 19, 21), ink: (232, 233, 236), accent: (156, 170, 196) },
    },
];

/// 依名稱找主題；不認得的名稱（例如更新的版本加的）用預設主題
pub fn find(id: &str) -> &'static Preset {
    PRESETS.iter().find(|p| p.id.eq_ignore_ascii_case(id)).unwrap_or(&PRESETS[0])
}

/// 第幾個主題（設定視窗的選項順序）；不認得的名稱是預設主題
pub fn index_of(id: &str) -> usize {
    PRESETS.iter().position(|p| p.id.eq_ignore_ascii_case(id)).unwrap_or(0)
}

/// WCAG 的相對亮度（0～1）
pub fn luminance(c: Rgb) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// WCAG 對比（1～21）
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// 深色文字（暖炭灰）與淺色文字（米白）
pub const DARK_TEXT: Rgb = (43, 33, 28);
pub const LIGHT_TEXT: Rgb = (255, 250, 244);

/// 放在 `bg` 上的文字顏色：深色或淺色文字中對比較高的那一個
pub fn text_on(bg: Rgb) -> Rgb {
    if contrast(DARK_TEXT, bg) >= contrast(LIGHT_TEXT, bg) {
        DARK_TEXT
    } else {
        LIGHT_TEXT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_named() {
        assert!((6..=8).contains(&PRESETS.len()));
        for (i, p) in PRESETS.iter().enumerate() {
            assert!(!p.name_zh.is_empty() && !p.name_en.is_empty());
            assert!(PRESETS[i + 1..].iter().all(|q| q.id != p.id && q.name_zh != p.name_zh), "{}", p.id);
            assert_eq!(index_of(p.id), i);
        }
        assert_eq!(PRESETS[0].id, DEFAULT);
        // 不認得的名稱：預設主題
        assert_eq!(find("future-theme").id, DEFAULT);
        assert_eq!(index_of("future-theme"), 0);
        assert_eq!(find("SAGE").id, "sage");
    }

    /// 每一組主題的淺色與深色都要好讀：文字、重點色（底線、選取框）、主要按鈕上的字
    #[test]
    fn presets_have_enough_contrast() {
        for p in &PRESETS {
            for (dark, c) in [(false, &p.light), (true, &p.dark)] {
                let who = format!("{} {}", p.id, if dark { "dark" } else { "light" });
                assert!(contrast(c.ink, c.background) >= 7.0, "{who}: ink/background");
                assert!(contrast(c.ink, c.surface) >= 7.0, "{who}: ink/surface");
                assert!(contrast(c.ink, c.sidebar) >= 7.0, "{who}: ink/sidebar");
                assert!(contrast(c.accent, c.surface) >= 3.0, "{who}: accent/surface");
                assert!(contrast(c.accent, c.background) >= 3.0, "{who}: accent/background");
                assert!(contrast(text_on(c.accent), c.accent) >= 4.0, "{who}: button text");
                // 深淺要和模式一致：深色模式的底色是暗的，淺色模式是亮的
                assert_eq!(luminance(c.background) < 0.18, dark, "{who}: background brightness");
                // 卡片和底色分得出來，但不會差太多（底色只帶一點色調）
                let gap = contrast(c.surface, c.background);
                assert!((1.02..1.4).contains(&gap), "{who}: surface/background {gap}");
            }
        }
    }

    #[test]
    fn text_on_picks_the_readable_one() {
        assert_eq!(text_on((255, 255, 255)), DARK_TEXT);
        assert_eq!(text_on((0, 0, 0)), LIGHT_TEXT);
        assert_eq!(text_on((44, 110, 190)), LIGHT_TEXT);
        assert_eq!(text_on((232, 150, 118)), DARK_TEXT);
        assert!((contrast((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 0.01);
    }
}

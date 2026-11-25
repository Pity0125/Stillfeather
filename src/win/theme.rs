//! 配色、字型與深色模式。
//!
//! 預設配色：淺色以米白為底、白色卡片、淺燕麥色側欄；深色以近黑灰為主。
//! 重點色是柔和的陶土橘，圓角卡片與膠囊按鈕。
//! 使用者可在設定中自訂重點色與背景色，其餘顏色會依背景亮度推導。

use super::sys::*;
use super::util::*;
use feather_core::category::Category;
use feather_core::settings::{parse_color, Settings, ThemeMode};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// 主要文字
    pub ink: COLORREF,
    /// 次要文字
    pub muted: COLORREF,
    /// 重點色（陶土橘）
    pub accent: COLORREF,
    /// 頁面底色（米白／近黑）
    pub background: COLORREF,
    /// 卡片、搜尋框、圓形按鈕的底色
    pub surface: COLORREF,
    /// 細框線
    pub line: COLORREF,
    pub hover: COLORREF,
    /// 選取列、作用中分段選項的淡重點色
    pub selected_tint: COLORREF,
    /// 側欄卡片（淺燕麥色／深灰）
    pub sidebar: COLORREF,
    /// 側欄選取項目的膠囊底色
    pub sidebar_selected: COLORREF,
    pub sidebar_hover: COLORREF,
    pub sidebar_ink: COLORREF,
    pub sidebar_muted: COLORREF,
    /// 篩選籤底色與作用中篩選籤（深色實心）
    pub chip: COLORREF,
    pub chip_active: COLORREF,
    pub chip_active_ink: COLORREF,
    /// 側欄底部的淡杏色提示卡
    pub peach: COLORREF,
    /// 卡片陰影的兩層顏色
    pub shadow_near: COLORREF,
    pub shadow_far: COLORREF,
    /// 原生控制項（捲軸、選單、標題列）是否使用深色。
    pub dark: bool,
}

fn parts(c: COLORREF) -> (f32, f32, f32) {
    ((c & 0xFF) as f32, ((c >> 8) & 0xFF) as f32, ((c >> 16) & 0xFF) as f32)
}

pub fn blend(fg: COLORREF, bg: COLORREF, amount: f32) -> COLORREF {
    let (fr, fg_, fb) = parts(fg);
    let (br, bg_, bb) = parts(bg);
    let mix = |f: f32, b: f32| (f * amount + b * (1.0 - amount)).round().clamp(0.0, 255.0) as u8;
    rgb(mix(fr, br), mix(fg_, bg_), mix(fb, bb))
}

/// HSL 亮度（0～1）。
fn brightness(c: COLORREF) -> f32 {
    let (r, g, b) = parts(c);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    (max + min) / 2.0 / 255.0
}

/// 顏色是否偏暗（決定原生捲軸等要用深色還是淺色樣式）。
pub fn is_dark_color(c: COLORREF) -> bool {
    brightness(c) < 0.5
}

/// 放在指定底色上的文字顏色（深或淺）。
pub fn contrast_text(bg: COLORREF) -> COLORREF {
    let (r, g, b) = parts(bg);
    if r * 0.299 + g * 0.587 + b * 0.114 > 150.0 {
        rgb(43, 33, 28)
    } else {
        rgb(255, 250, 244)
    }
}

pub fn system_uses_dark() -> bool {
    reg_dword("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "AppsUseLightTheme") == Some(0)
}

/// 預設配色的原色。
mod base {
    use super::rgb;
    use super::COLORREF;
    // 淺色：米白底、白卡片、淺燕麥色側欄、暖炭灰文字。
    pub const LIGHT_BACKGROUND: COLORREF = rgb(248, 246, 242);
    pub const LIGHT_SURFACE: COLORREF = rgb(255, 255, 255);
    pub const LIGHT_SIDEBAR: COLORREF = rgb(240, 237, 232);
    pub const LIGHT_INK: COLORREF = rgb(46, 43, 41);
    pub const LIGHT_ACCENT: COLORREF = rgb(210, 118, 86);
    pub const LIGHT_PEACH: COLORREF = rgb(249, 235, 226);
    // 深色：近黑底、深灰卡片與側欄（中性灰，不帶色偏）。
    pub const DARK_BACKGROUND: COLORREF = rgb(18, 18, 18);
    pub const DARK_SURFACE: COLORREF = rgb(30, 30, 30);
    pub const DARK_SIDEBAR: COLORREF = rgb(25, 25, 25);
    pub const DARK_INK: COLORREF = rgb(236, 234, 231);
    pub const DARK_ACCENT: COLORREF = rgb(232, 150, 118);
    pub const DARK_PEACH: COLORREF = rgb(42, 34, 30);
}

impl Palette {
    pub fn from_settings(s: &Settings) -> Palette {
        let is_dark = s.theme_mode == ThemeMode::Dark || (s.theme_mode == ThemeMode::System && system_uses_dark());
        let (mut background, mut surface, mut sidebar, mut peach) = if is_dark {
            (base::DARK_BACKGROUND, base::DARK_SURFACE, base::DARK_SIDEBAR, base::DARK_PEACH)
        } else {
            (base::LIGHT_BACKGROUND, base::LIGHT_SURFACE, base::LIGHT_SIDEBAR, base::LIGHT_PEACH)
        };
        let mut accent = if is_dark { base::DARK_ACCENT } else { base::LIGHT_ACCENT };
        if let Some((r, g, b)) = s.accent_color.as_deref().and_then(parse_color) {
            accent = rgb(r, g, b);
        }
        let custom_bg = s.background_color.as_deref().and_then(parse_color).map(|(r, g, b)| rgb(r, g, b));
        if let Some(bg) = custom_bg {
            // 自訂背景：卡片比背景稍亮（或稍暗），側欄與提示卡往背景色靠攏。
            let light = brightness(bg) >= 0.5;
            background = bg;
            surface = if light { blend(rgb(255, 255, 255), bg, 0.55) } else { blend(rgb(255, 255, 255), bg, 0.07) };
            sidebar = blend(sidebar, bg, 0.45);
            peach = blend(accent, bg, 0.22);
        }
        let dark = brightness(surface) < 0.5;
        let ink = if custom_bg.is_none() {
            if is_dark { base::DARK_INK } else { base::LIGHT_INK }
        } else if dark {
            base::DARK_INK
        } else {
            base::LIGHT_INK
        };
        let sidebar_dark = brightness(sidebar) < 0.5;
        let sidebar_ink = if sidebar_dark { base::DARK_INK } else { base::LIGHT_INK };
        let shadow_base = if dark { rgb(0, 0, 0) } else { rgb(90, 80, 70) };
        Palette {
            ink,
            muted: blend(ink, surface, 0.58),
            accent,
            background,
            surface,
            line: blend(ink, background, 0.11),
            hover: blend(ink, surface, 0.06),
            selected_tint: blend(accent, surface, 0.2),
            sidebar,
            sidebar_selected: if sidebar_dark { blend(rgb(255, 255, 255), sidebar, 0.1) } else { blend(surface, sidebar, 0.86) },
            sidebar_hover: if sidebar_dark { blend(rgb(255, 255, 255), sidebar, 0.05) } else { blend(surface, sidebar, 0.38) },
            sidebar_ink,
            sidebar_muted: blend(sidebar_ink, sidebar, 0.6),
            chip: blend(ink, background, 0.045),
            chip_active: if dark { blend(ink, surface, 0.9) } else { ink },
            chip_active_ink: if dark { surface } else { rgb(252, 250, 247) },
            peach,
            shadow_near: blend(shadow_base, background, if dark { 0.35 } else { 0.07 }),
            shadow_far: blend(shadow_base, background, if dark { 0.18 } else { 0.035 }),
            dark,
        }
    }

    /// 篩選籤前面的小圓點顏色。
    pub fn category_dot(&self, c: Category) -> COLORREF {
        let raw = match c {
            Category::All => self.muted,
            Category::Folder => rgb(226, 176, 74),
            Category::Image => rgb(233, 135, 120),
            Category::Document => rgb(122, 166, 204),
            Category::Media => rgb(164, 140, 214),
            Category::Archive => rgb(204, 132, 172),
            Category::Other => rgb(168, 162, 156),
        };
        if self.dark {
            blend(raw, self.surface, 0.85)
        } else {
            raw
        }
    }
}

pub struct Brushes {
    pub background: HBRUSH,
    pub surface: HBRUSH,
    pub line: HBRUSH,
    pub accent: HBRUSH,
}

impl Brushes {
    fn new(p: &Palette) -> Brushes {
        unsafe {
            Brushes {
                background: CreateSolidBrush(p.background),
                surface: CreateSolidBrush(p.surface),
                line: CreateSolidBrush(p.line),
                accent: CreateSolidBrush(p.accent),
            }
        }
    }

    fn destroy(&mut self) {
        unsafe {
            for b in [self.background, self.surface, self.line, self.accent] {
                if b != 0 {
                    DeleteObject(b);
                }
            }
        }
    }
}

pub struct Fonts {
    pub ui: HFONT,
    pub ui_bold: HFONT,
    /// 小字：區段標籤、快捷鍵提示、欄位標題
    pub small: HFONT,
    pub small_bold: HFONT,
    /// 側欄的程式名稱
    pub brand: HFONT,
    /// 頁面大標題（目前資料夾名稱）
    pub title: HFONT,
    pub icon: HFONT,
    pub icon_small: HFONT,
    /// 單行文字高度（像素）。
    pub ui_height: i32,
    /// 內容文字：檔名與欄位、側欄、分頁、狀態列。跟著文字大小放大（`text_size` 百分比；設定視窗另外用
    /// `ui_font` 建立同樣比例的字型），頂列與動作列的按鈕、選單、其他對話方塊維持 `ui` 的大小，版面才不會被撐開。
    pub body: HFONT,
    pub body_bold: HFONT,
    /// 內容的小字：欄位標題、側欄的區段標籤、提示列
    pub body_small: HFONT,
    pub body_small_bold: HFONT,
    /// `body` 的單行文字高度（像素）
    pub body_height: i32,
    /// 路徑列最後一段（目前資料夾），取代原本的大標題
    pub heading: HFONT,
}

fn create_font(face: &str, points: i32, weight: i32, dpi: u32) -> HFONT {
    create_font_scaled(face, points, 100, weight, dpi)
}

/// `points` 點放大 `percent`%（文字大小設定）
fn create_font_scaled(face: &str, points: i32, percent: u32, weight: i32, dpi: u32) -> HFONT {
    let f = wide(face);
    unsafe {
        CreateFontW(-MulDiv(points * percent as i32, dpi as i32, 7200), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, f.as_ptr())
    }
}

/// 介面字型（微軟正黑體 UI）`points` 點放大 `percent`%；`bold` 用半粗體。呼叫端負責 DeleteObject。
/// 設定視窗依文字大小建立自己的字型、預覽各個文字大小時用。
pub fn ui_font(points: i32, percent: u32, bold: bool, dpi: u32) -> HFONT {
    create_font_scaled(UI_FACE, points, percent, if bold { FW_SEMIBOLD } else { FW_NORMAL }, dpi)
}

/// 字型的單行高度（以中英混排的樣本量）
pub fn line_height(font: HFONT) -> i32 {
    unsafe {
        let hdc = GetDC(0);
        let old = SelectObject(hdc, font);
        let mut size = SIZE::default();
        let sample = wide("羽Ag");
        GetTextExtentPoint32W(hdc, sample.as_ptr(), 3, &mut size);
        SelectObject(hdc, old);
        ReleaseDC(0, hdc);
        size.cy.max(12)
    }
}

/// 字型是否真的存在（建立後讀回實際字型名稱比對）。
fn font_exists(font: HFONT, face: &str) -> bool {
    unsafe {
        let hdc = GetDC(0);
        let old = SelectObject(hdc, font);
        let mut buf = [0u16; 64];
        let n = GetTextFaceW(hdc, buf.len() as i32, buf.as_mut_ptr());
        SelectObject(hdc, old);
        ReleaseDC(0, hdc);
        n > 0 && from_wide(&buf).eq_ignore_ascii_case(face)
    }
}

const UI_FACE: &str = "Microsoft JhengHei UI";
const FW_BOLD: i32 = 700;

impl Fonts {
    fn new(dpi: u32, text_size: u32) -> Fonts {
        let ui = create_font(UI_FACE, 9, FW_NORMAL, dpi);
        let ui_bold = create_font(UI_FACE, 9, FW_SEMIBOLD, dpi);
        let small = create_font(UI_FACE, 8, FW_NORMAL, dpi);
        let small_bold = create_font(UI_FACE, 8, FW_SEMIBOLD, dpi);
        let brand = create_font(UI_FACE, 13, FW_BOLD, dpi);
        let title = create_font(UI_FACE, 21, FW_BOLD, dpi);
        let mut icon_face = "Segoe Fluent Icons";
        let probe = create_font(icon_face, 11, FW_NORMAL, dpi);
        if !font_exists(probe, icon_face) {
            icon_face = "Segoe MDL2 Assets";
        }
        unsafe {
            DeleteObject(probe);
        }
        let icon = create_font(icon_face, 11, FW_NORMAL, dpi);
        let icon_small = create_font(icon_face, 8, FW_NORMAL, dpi);
        let ui_height = line_height(ui);
        let body = create_font_scaled(UI_FACE, 9, text_size, FW_NORMAL, dpi);
        let body_bold = create_font_scaled(UI_FACE, 9, text_size, FW_SEMIBOLD, dpi);
        let body_small = create_font_scaled(UI_FACE, 8, text_size, FW_NORMAL, dpi);
        let body_small_bold = create_font_scaled(UI_FACE, 8, text_size, FW_SEMIBOLD, dpi);
        let body_height = line_height(body);
        let heading = create_font(UI_FACE, 12, FW_SEMIBOLD, dpi);
        Fonts { ui, ui_bold, small, small_bold, brand, title, icon, icon_small, ui_height, body, body_bold, body_small, body_small_bold, body_height, heading }
    }

    fn destroy(&mut self) {
        unsafe {
            for f in [self.ui, self.ui_bold, self.small, self.small_bold, self.brand, self.title, self.icon, self.icon_small, self.body, self.body_bold, self.body_small, self.body_small_bold, self.heading] {
                if f != 0 {
                    DeleteObject(f);
                }
            }
        }
    }
}

pub struct Theme {
    pub pal: Palette,
    pub brushes: Brushes,
    pub fonts: Fonts,
    pub dpi: u32,
    /// 內容文字大小（百分比，見 `Fonts::body`）
    pub text_size: u32,
}

impl Theme {
    pub fn new(settings: &Settings, dpi: u32) -> Theme {
        let pal = Palette::from_settings(settings);
        Theme { brushes: Brushes::new(&pal), fonts: Fonts::new(dpi, settings.text_size), pal, dpi, text_size: settings.text_size }
    }

    pub fn set_palette(&mut self, settings: &Settings) {
        self.brushes.destroy();
        self.pal = Palette::from_settings(settings);
        self.brushes = Brushes::new(&self.pal);
    }

    pub fn set_dpi(&mut self, dpi: u32) {
        self.fonts.destroy();
        self.fonts = Fonts::new(dpi, self.text_size);
        self.dpi = dpi;
    }

    /// 換內容文字大小：重建字型（呼叫端要把新字型套到清單、樹狀圖並重排）
    pub fn set_text_size(&mut self, text_size: u32) {
        self.fonts.destroy();
        self.fonts = Fonts::new(self.dpi, text_size);
        self.text_size = text_size;
    }

    pub fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }

    /// 內容的寬度跟著文字大小放大（欄寬等）
    pub fn st(&self, v: i32) -> i32 {
        scale(v * self.text_size as i32 / 100, self.dpi)
    }
}

impl Drop for Theme {
    fn drop(&mut self) {
        self.brushes.destroy();
        self.fonts.destroy();
    }
}

/// 標題列深淺色。
pub fn apply_title_bar(hwnd: HWND, dark: bool) {
    let value: BOOL = dark as BOOL;
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &value as *const BOOL as *const std::ffi::c_void, 4);
    }
}

/// 標題列與 App 融為一體：標題列底色用 `caption`（主視窗用頁面底色、對話方塊用卡片色），
/// 文字用主要文字色，外框用細框線色。Windows 11 才支援自訂顏色，Windows 10 只套用深淺色。
pub fn style_caption(hwnd: HWND, pal: &Palette, caption: COLORREF) {
    apply_title_bar(hwnd, pal.dark);
    let set = |attr: u32, c: COLORREF| unsafe {
        DwmSetWindowAttribute(hwnd, attr, &c as *const COLORREF as *const std::ffi::c_void, 4);
    };
    set(DWMWA_CAPTION_COLOR, caption);
    set(DWMWA_TEXT_COLOR, pal.ink);
    set(DWMWA_BORDER_COLOR, blend(pal.ink, caption, 0.14));
}

/// 讓清單、樹狀圖與捲軸採用檔案總管樣式（深色時使用系統的深色檔案總管樣式）。
pub fn apply_explorer_theme(hwnd: HWND, dark: bool) {
    unsafe {
        allow_dark_mode_for_window(hwnd, dark);
        let name = wide(if dark { "DarkMode_Explorer" } else { "Explorer" });
        SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
    }
}

pub fn apply_header_theme(hwnd: HWND, dark: bool) {
    unsafe {
        allow_dark_mode_for_window(hwnd, dark);
        let name = wide(if dark { "DarkMode_ItemsView" } else { "ItemsView" });
        SetWindowTheme(hwnd, name.as_ptr(), std::ptr::null());
    }
}

/// 右鍵選單等系統元件的深淺色（2 = ForceDark，3 = ForceLight）。
pub fn apply_app_mode(dark: bool) {
    unsafe {
        set_preferred_app_mode(if dark { 2 } else { 3 });
    }
}

// ---------------------------------------------------------------- 繪圖小工具

pub fn fill(hdc: HDC, r: &RECT, color: COLORREF) {
    unsafe {
        SetDCBrushColor(hdc, color);
        FillRect(hdc, r, GetStockObject(DC_BRUSH));
    }
}

pub fn round_rect(hdc: HDC, r: &RECT, radius: i32, fill_color: Option<COLORREF>, border: Option<COLORREF>) {
    unsafe {
        let old_brush = SelectObject(hdc, if fill_color.is_some() { GetStockObject(DC_BRUSH) } else { GetStockObject(NULL_BRUSH) });
        let old_pen = SelectObject(hdc, if border.is_some() { GetStockObject(DC_PEN) } else { GetStockObject(NULL_PEN) });
        if let Some(c) = fill_color {
            SetDCBrushColor(hdc, c);
        }
        if let Some(c) = border {
            SetDCPenColor(hdc, c);
        }
        // NULL_PEN 時 RoundRect 會少畫一個像素，補回來。
        let extra = if border.is_some() { 0 } else { 1 };
        RoundRect(hdc, r.left, r.top, r.right + extra, r.bottom + extra, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
    }
}

/// 浮起的圓角卡片：底下先畫兩層很淡的偏移陰影（GDI 沒有模糊，以色階近似）。
pub fn card(hdc: HDC, r: &RECT, radius: i32, fill_color: COLORREF, border: Option<COLORREF>, pal: &Palette, dpi: u32) {
    let s1 = scale(2, dpi).max(1);
    let far = RECT { left: r.left - s1 / 2, top: r.top + s1, right: r.right + s1 / 2, bottom: r.bottom + s1 * 3 };
    round_rect(hdc, &far, radius + s1 * 2, Some(pal.shadow_far), None);
    let near = RECT { left: r.left, top: r.top + s1 / 2, right: r.right, bottom: r.bottom + s1 };
    round_rect(hdc, &near, radius + s1, Some(pal.shadow_near), None);
    round_rect(hdc, r, radius, Some(fill_color), border);
}

/// 置中的圓形（直徑取矩形較短邊）。
pub fn circle(hdc: HDC, r: &RECT, fill_color: Option<COLORREF>, border: Option<COLORREF>) {
    let d = r.width().min(r.height());
    let c = RECT::new(r.left + (r.width() - d) / 2, r.top + (r.height() - d) / 2, d, d);
    round_rect(hdc, &c, d, fill_color, border);
}

pub fn draw_text(hdc: HDC, text: &str, r: &RECT, font: HFONT, color: COLORREF, format: u32) {
    // 空字串不呼叫 DrawTextW：空的 Vec 指標是懸置位址（0x2），DrawTextW 加上 DT_END_ELLIPSIS
    // 時即使長度為 0 仍會讀取它而造成存取違規（快速預覽載入中、底部說明是空的時就是這樣閃退）。
    if text.is_empty() {
        return;
    }
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut rc = *r;
    unsafe {
        let old = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rc, format | DT_NOPREFIX);
        SelectObject(hdc, old);
    }
}

/// 指定寬度內自動換行時的文字高度。
pub fn text_height(hdc: HDC, text: &str, font: HFONT, width: i32, format: u32) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut rc = RECT { left: 0, top: 0, right: width.max(1), bottom: 0 };
    unsafe {
        let old = SelectObject(hdc, font);
        DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut rc, format | DT_CALCRECT | DT_NOPREFIX);
        SelectObject(hdc, old);
    }
    rc.height()
}

pub fn text_width(hdc: HDC, text: &str, font: HFONT) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        let old = SelectObject(hdc, font);
        GetTextExtentPoint32W(hdc, w.as_ptr(), w.len() as i32, &mut size);
        SelectObject(hdc, old);
    }
    size.cx
}

/// 雙緩衝繪圖：在記憶體 DC 畫完再一次貼上，避免閃爍。
pub struct Buffered {
    pub hdc: HDC,
    target: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    rect: RECT,
}

impl Buffered {
    pub fn new(target: HDC, rect: RECT) -> Buffered {
        unsafe {
            let hdc = CreateCompatibleDC(target);
            let bitmap = CreateCompatibleBitmap(target, rect.width().max(1), rect.height().max(1));
            let old = SelectObject(hdc, bitmap);
            Buffered { hdc, target, bitmap, old, rect }
        }
    }
}

impl Drop for Buffered {
    fn drop(&mut self) {
        unsafe {
            BitBlt(self.target, self.rect.left, self.rect.top, self.rect.width(), self.rect.height(), self.hdc, 0, 0, SRCCOPY);
            SelectObject(self.hdc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.hdc);
        }
    }
}

/// Segoe Fluent Icons／MDL2 字形。
pub mod glyph {
    pub const BACK: char = '\u{E72B}';
    pub const FORWARD: char = '\u{E72A}';
    pub const UP: char = '\u{E74A}';
    pub const REFRESH: char = '\u{E72C}';
    pub const NEW_FOLDER: char = '\u{E8F4}';
    pub const CUT: char = '\u{E8C6}';
    pub const COPY: char = '\u{E8C8}';
    pub const PASTE: char = '\u{E77F}';
    pub const RENAME: char = '\u{E8AC}';
    pub const DELETE: char = '\u{E74D}';
    pub const DUAL: char = '\u{E89F}';
    pub const COPY_ACROSS: char = '\u{E8AB}';
    pub const HIDDEN: char = '\u{E890}';
    pub const SETTINGS: char = '\u{E713}';
    pub const HELP: char = '\u{E897}';
    /// 圓圈裡的問號（側欄標頭的說明按鈕）
    pub const HELP_CIRCLE: char = '\u{E9CE}';
    pub const PROPERTIES: char = '\u{E946}';
    pub const ADD: char = '\u{E710}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const CHEVRON_LEFT: char = '\u{E76B}';
    pub const CHEVRON_RIGHT: char = '\u{E76C}';
    pub const CHECKBOX: char = '\u{E739}';
    pub const CHECKBOX_CHECKED: char = '\u{E73A}';
    pub const SEARCH: char = '\u{E721}';
    pub const FOLDER: char = '\u{E8B7}';
    /// 篩選（選擇器的檔案類型）
    pub const FILTER: char = '\u{E71C}';
    pub const MOON: char = '\u{E708}';
    pub const SUN: char = '\u{E706}';
    pub const PALETTE: char = '\u{E790}';
    pub const SORT_UP: char = '\u{E70E}';
    pub const SORT_DOWN: char = '\u{E70D}';
    pub const CHEVRON_DOWN_SMALL: char = '\u{E96E}';
    // 右鍵選單（碼位依 Segoe MDL2／Fluent Icons）
    pub const OPEN: char = '\u{E8E5}';
    pub const OPEN_NEW_TAB: char = '\u{E8A7}';
    pub const PIN: char = '\u{E718}';
    pub const UNPIN: char = '\u{E77A}';
    pub const LINK: char = '\u{E71B}';
    pub const SELECT_ALL: char = '\u{E8B3}';
    pub const UNDO: char = '\u{E7A7}';
    pub const PREVIEW: char = '\u{E8FF}';
    /// 命令提示字元（「在這裡開啟 PowerShell 視窗」）
    pub const TERMINAL: char = '\u{E756}';
    // 排序、檢視選單（碼位依 Microsoft Segoe Fluent Icons 文件）
    pub const SORT: char = '\u{E8CB}';
    pub const FONT_SIZE: char = '\u{E8D2}';
    pub const VIEW_DETAILS: char = '\u{E8FD}';
    pub const VIEW_LIST: char = '\u{EA37}';
    pub const VIEW_SMALL: char = '\u{E8A9}';
    pub const VIEW_MEDIUM: char = '\u{E7C5}';
    pub const VIEW_LARGE: char = '\u{E8B9}';
    pub const VIEW_TILES: char = '\u{E8BC}';
    pub const VIEW_CONTENT: char = '\u{E8E4}';
    pub const CHEVRON_SMALL: char = '\u{E974}';
    pub const WARNING: char = '\u{E7BA}';
    pub const ERROR: char = '\u{E783}';
    // 側欄位置圖示（碼位依 Microsoft Segoe Fluent Icons 文件）
    pub const HOME: char = '\u{E80F}';
    pub const MONITOR: char = '\u{E7F4}';
    pub const DOCUMENT: char = '\u{E8A5}';
    pub const DOWNLOAD: char = '\u{E896}';
    pub const PHOTO: char = '\u{E91B}';
    pub const MUSIC: char = '\u{EC4F}';
    pub const VIDEO: char = '\u{E714}';
    pub const CLOUD: char = '\u{E753}';
    pub const FOLDER_FILL: char = '\u{E8D5}';
    pub const HARD_DRIVE: char = '\u{EDA2}';
    pub const USB: char = '\u{E88E}';
    pub const MAP_DRIVE: char = '\u{E8CE}';
}

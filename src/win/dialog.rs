//! 與 App 風格一致的對話方塊，取代 Windows 的 MessageBox：
//! 卡片色底、重點色圓形圖示（開啟時彈出）、膠囊按鈕，標題列與卡片同色。
//!
//! 安靜：不呼叫 MessageBeep，也不經過 IsDialogMessage（它在按到沒有對應的字母時會發出提示音），
//! 鍵盤由這裡自己處理：Enter 確認、Esc 取消、Tab／方向鍵切換按鈕、按鈕上標示的字母直接選取、
//! Ctrl+C 複製對話方塊文字（與 MessageBox 相同）。
//! 開啟與關閉沿用 Windows 視窗原生的淡入縮放動畫。
//!
//! 也可以帶輸入欄位（[`Field`]：日期用 Windows 內建的日期挑選器，文字用單行編輯框），以 [`Spec::show_form`] 開啟；
//! 焦點在欄位裡時按鍵交給欄位（Enter 確認、Esc 取消、Tab 切換欄位與按鈕照常）。
//!
//! 文字與版面跟著「文字大小」放大（同設定視窗）：依比例建立自己的字型（[`DlgFonts`]），所有尺寸以
//! [`DlgFonts::k`] 換算。整個對話方塊放不進螢幕的工作區時，比例每次降 5%，最小 100%。

use super::access;
use super::anim;
use super::app;
use super::clipboard;
use super::controls::{self, Kind, Surface};
use super::sys::*;
use super::theme::{self, glyph, Buffered, Theme};
use super::util::*;
use feather_core::anim::{Ease, Tween};
use feather_core::searchquery::Ymd;

pub const CLASS: &str = "FeatherFiles.Dialog";
const FIRST_BUTTON_ID: u16 = 100;
const FIRST_FIELD_ID: u16 = 200;
const POP_MS: f64 = 420.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Question,
    Info,
    Warning,
    Error,
    Delete,
}

pub struct Button {
    pub id: i32,
    pub label: String,
    pub primary: bool,
    /// 直接按這個字母也會選取（例如「是」= Y、「否」= N）。
    pub key: Option<char>,
}

impl Button {
    pub fn primary(id: i32, label: &str) -> Button {
        Button { id, label: label.into(), primary: true, key: None }
    }

    pub fn normal(id: i32, label: &str) -> Button {
        Button { id, label: label.into(), primary: false, key: None }
    }

    pub fn key(mut self, k: char) -> Button {
        self.key = Some(k);
        self
    }
}

/// 對話方塊裡的輸入欄位
pub enum Field {
    /// 日期（Windows 內建的日期挑選器）。`optional`：前面有勾選框，不勾是「不限」
    Date { label: String, value: Option<Ymd>, optional: bool },
    /// 一行文字；`cue` 是空著時顯示的淡色提示
    Text { label: String, value: String, cue: String },
}

impl Field {
    fn label(&self) -> &str {
        match self {
            Field::Date { label, .. } | Field::Text { label, .. } => label,
        }
    }
}

/// 關閉時欄位的內容（順序同 [`Spec::fields`]）
#[derive(Clone, Debug, PartialEq)]
pub enum FieldValue {
    Date(Option<Ymd>),
    Text(String),
}

/// 對話方塊內容。以 `Spec::new` 建立後串接設定，最後呼叫 `show`。
pub struct Spec {
    title: String,
    icon: Icon,
    heading: String,
    body: String,
    rows: Vec<(String, String)>,
    fields: Vec<Field>,
    footnote: String,
    buttons: Vec<Button>,
    cancel: i32,
    width: i32,
    /// 開啟時焦點所在的按鈕 id（None：主要按鈕）
    focus: Option<i32>,
}

impl Spec {
    pub fn new(icon: Icon, heading: &str) -> Spec {
        Spec {
            title: "Stillfeather".into(),
            icon,
            heading: heading.into(),
            body: String::new(),
            rows: Vec::new(),
            fields: Vec::new(),
            footnote: String::new(),
            buttons: vec![Button::primary(1, tr!("確定", "OK"))],
            cancel: 1,
            width: 440,
            focus: None,
        }
    }

    pub fn body(mut self, text: &str) -> Spec {
        self.body = text.into();
        self
    }

    /// 兩欄列表（左欄以小膠囊顯示，例如快捷鍵）。
    pub fn rows(mut self, rows: Vec<(String, String)>) -> Spec {
        self.rows = rows;
        self
    }

    /// 輸入欄位（在內文與註腳之間，一列一個）；開啟時焦點在第一個欄位
    pub fn fields(mut self, fields: Vec<Field>) -> Spec {
        self.fields = fields;
        self
    }

    pub fn footnote(mut self, text: &str) -> Spec {
        self.footnote = text.into();
        self
    }

    /// 開啟時焦點所在的按鈕（依 id）；沒設定時是主要按鈕。Enter 選焦點所在的按鈕。
    pub fn focus(mut self, id: i32) -> Spec {
        self.focus = Some(id);
        self
    }

    /// 按鈕由左到右排列；`cancel` 是按 Esc 或關閉視窗時的結果。
    pub fn buttons(mut self, buttons: Vec<Button>, cancel: i32) -> Spec {
        self.buttons = buttons;
        self.cancel = cancel;
        self
    }

    /// 內容寬度（未縮放的像素）。
    pub fn width(mut self, w: i32) -> Spec {
        self.width = w;
        self
    }

    pub fn show(self, owner: HWND) -> i32 {
        show(owner, self).0
    }

    /// 顯示帶輸入欄位的對話方塊：回傳按下的按鈕與各欄位的內容
    pub fn show_form(self, owner: HWND) -> (i32, Vec<FieldValue>) {
        show(owner, self)
    }

    fn plain_text(&self) -> String {
        let mut parts = vec![self.heading.clone()];
        if !self.body.is_empty() {
            parts.push(self.body.clone());
        }
        if !self.rows.is_empty() {
            parts.push(self.rows.iter().map(|(k, v)| format!("{k}\t{v}")).collect::<Vec<_>>().join("\r\n"));
        }
        if !self.fields.is_empty() {
            parts.push(self.fields.iter().map(|f| f.label().to_string()).collect::<Vec<_>>().join("\r\n"));
        }
        if !self.footnote.is_empty() {
            parts.push(self.footnote.clone());
        }
        parts.join("\r\n\r\n").replace('\n', "\r\n").replace("\r\r\n", "\r\n")
    }
}

/// 確認對話方塊：按下主要按鈕回傳 true。主要按鈕可按 Y、取消可按 N。
pub fn confirm(owner: HWND, icon: Icon, heading: &str, body: &str, ok: &str) -> bool {
    Spec::new(icon, heading)
        .body(body)
        .buttons(vec![Button::primary(1, ok).key('Y'), Button::normal(0, tr!("取消", "Cancel")).key('N')], 0)
        .show(owner)
        == 1
}

/// 危險、無法復原的動作（例如永久刪除）：警告圖示，預設焦點在「取消」，一路按 Enter 不會誤觸。
/// 確認按鈕沒有快捷字母。
pub fn confirm_danger(owner: HWND, heading: &str, body: &str, ok: &str) -> bool {
    Spec::new(Icon::Warning, heading)
        .body(body)
        .buttons(vec![Button::primary(1, ok), Button::normal(0, tr!("取消", "Cancel")).key('N')], 0)
        .focus(0)
        .show(owner)
        == 1
}

pub fn text_input(owner: HWND, title: &str, label: &str, current: &str, cue: &str) -> Option<String> {
    let (button, fields) = Spec::new(Icon::Info, title)
        .fields(vec![Field::Text { label: label.into(), value: current.into(), cue: cue.into() }])
        .buttons(vec![Button::primary(1, tr!("確定", "OK")), Button::normal(2, tr!("取消", "Cancel"))], 2)
        .show_form(owner);
    match fields.into_iter().next() {
        Some(FieldValue::Text(value)) if button == 1 => Some(value.trim().into()),
        _ => None,
    }
}

pub fn info(owner: HWND, heading: &str, body: &str) {
    Spec::new(Icon::Info, heading).body(body).show(owner);
}

pub fn error(owner: HWND, heading: &str, body: &str) {
    Spec::new(Icon::Error, heading).body(body).show(owner);
}

// ---------------------------------------------------------------- 實作

/// 對話方塊自己的字型：依文字大小（縮放比例 `percent`）建立，關閉時刪除
struct DlgFonts {
    percent: u32,
    dpi: u32,
    heading: HFONT,
    body: HFONT,
    body_bold: HFONT,
    small: HFONT,
    small_bold: HFONT,
    icon: HFONT,
    line: i32,
}

impl DlgFonts {
    fn new(t: &Theme, percent: u32) -> DlgFonts {
        let body = theme::ui_font(9, percent, false, t.dpi);
        DlgFonts {
            percent,
            dpi: t.dpi,
            heading: theme::ui_font_weight(13, percent, theme::FW_BOLD, t.dpi),
            body,
            body_bold: theme::ui_font(9, percent, true, t.dpi),
            small: theme::ui_font(8, percent, false, t.dpi),
            small_bold: theme::ui_font(8, percent, true, t.dpi),
            icon: theme::icon_font(t.fonts.icon_face, 11, percent, t.dpi),
            line: theme::line_height(body),
        }
    }

    /// 邏輯像素 → 實際像素（含 DPI 與文字大小的比例）
    fn k(&self, v: i32) -> i32 {
        scale(v * self.percent as i32 / 100, self.dpi)
    }

    fn destroy(&mut self) {
        for f in [self.heading, self.body, self.body_bold, self.small, self.small_bold, self.icon] {
            if f != 0 {
                unsafe {
                    DeleteObject(f);
                }
            }
        }
        *self = DlgFonts { percent: self.percent, dpi: self.dpi, heading: 0, body: 0, body_bold: 0, small: 0, small_bold: 0, icon: 0, line: self.line };
    }
}

#[derive(Default)]
struct Layout {
    icon: RECT,
    heading: RECT,
    body: RECT,
    rows: Vec<(RECT, RECT)>,
    /// 欄位：標籤、外框（文字欄的框線畫在外框上）、控制項本身
    fields: Vec<(RECT, RECT, RECT)>,
    footnote: RECT,
    buttons: Vec<RECT>,
    width: i32,
    height: i32,
}

struct State {
    spec: Spec,
    fonts: DlgFonts,
    layout: Layout,
    buttons: Vec<HWND>,
    /// 輸入欄位的控制項（順序同 spec.fields）
    inputs: Vec<HWND>,
    result: i32,
    done: bool,
    pop: Tween,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: dialog_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: 0,
        hCursor: load_cursor(IDC_ARROW),
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: 0,
    };
    unsafe {
        RegisterClassExW(&wc);
    }
}

fn icon_style(icon: Icon, t: &Theme) -> (char, COLORREF) {
    let p = &t.pal;
    match icon {
        Icon::Question => (glyph::HELP, p.accent),
        Icon::Info => (glyph::PROPERTIES, p.accent),
        Icon::Delete => (glyph::DELETE, p.accent),
        Icon::Warning => (glyph::WARNING, if p.dark { rgb(236, 176, 92) } else { rgb(204, 138, 40) }),
        Icon::Error => (glyph::ERROR, if p.dark { rgb(240, 128, 120) } else { rgb(204, 78, 70) }),
    }
}

fn compute_layout(spec: &Spec, f: &DlgFonts, hdc: HDC) -> Layout {
    let s = |v: i32| f.k(v);
    let pad = s(26);
    let icon_d = s(44);
    let width = s(spec.width);
    let x0 = pad + icon_d + s(16);
    let tw = width - x0 - pad;
    let mut l = Layout { width, icon: RECT::new(pad, pad, icon_d, icon_d), ..Default::default() };
    let wrap = DT_WORDBREAK;
    let hh = theme::text_height(hdc, &spec.heading, f.heading, tw, wrap).max(f.line);
    let mut y = pad;
    let only_heading = spec.body.is_empty() && spec.rows.is_empty() && spec.fields.is_empty() && spec.footnote.is_empty();
    if only_heading && hh < icon_d {
        y = pad + (icon_d - hh) / 2;
    } else if hh < icon_d {
        // 標題與圖示上緣對齊時稍微下移，看起來和圓心在同一條視線上
        y = pad + s(10).min((icon_d - hh) / 2);
    }
    l.heading = RECT::new(x0, y, tw, hh);
    y += hh;
    if !spec.body.is_empty() {
        y += s(8);
        let h = theme::text_height(hdc, &spec.body, f.body, tw, wrap);
        l.body = RECT::new(x0, y, tw, h);
        y += h;
    }
    if !spec.rows.is_empty() {
        y += s(16);
        let key_w = spec.rows.iter().map(|(k, _)| theme::text_width(hdc, k, f.small_bold)).max().unwrap_or(0) + s(22);
        let key_w = key_w.min(tw / 2);
        let row_h = s(30).max(f.line + s(10));
        let chip_h = s(24).max(f.line + s(6));
        for _ in &spec.rows {
            let key = RECT::new(x0, y + (row_h - chip_h) / 2, key_w, chip_h);
            let desc = RECT { left: x0 + key_w + s(14), top: y, right: x0 + tw, bottom: y + row_h };
            l.rows.push((key, desc));
            y += row_h;
        }
    }
    if !spec.fields.is_empty() {
        y += s(16);
        let label_w = (spec.fields.iter().map(|fd| theme::text_width(hdc, fd.label(), f.body)).max().unwrap_or(0) + s(14)).min(tw / 2);
        let row_h = s(42).max(f.line + s(20));
        let frame_h = s(34).max(f.line + s(14));
        for fd in &spec.fields {
            let label = RECT { left: x0, top: y, right: x0 + label_w, bottom: y + row_h };
            let frame = RECT { left: x0 + label_w, top: y + (row_h - frame_h) / 2, right: x0 + tw, bottom: y + (row_h - frame_h) / 2 + frame_h };
            let control = match fd {
                // 日期挑選器自己有框線，填滿外框
                Field::Date { .. } => frame,
                // 編輯框沒有框線：放在外框裡、垂直置中，框線由這裡畫
                Field::Text { .. } => {
                    let eh = f.line + s(2);
                    RECT { left: frame.left + s(10), top: frame.top + (frame_h - eh) / 2, right: frame.right - s(10), bottom: frame.top + (frame_h - eh) / 2 + eh }
                }
            };
            l.fields.push((label, frame, control));
            y += row_h;
        }
    }
    if !spec.footnote.is_empty() {
        y += s(16);
        let h = theme::text_height(hdc, &spec.footnote, f.small, tw, wrap);
        l.footnote = RECT::new(x0, y, tw, h);
        y += h;
    }
    y = y.max(pad + icon_d);
    y += s(28);
    let bh = s(36);
    let widths: Vec<i32> = spec.buttons.iter().map(|b| (theme::text_width(hdc, &b.label, f.body_bold) + s(44)).max(s(100))).collect();
    let gap = s(10);
    let total: i32 = widths.iter().sum::<i32>() + gap * (widths.len() as i32 - 1).max(0);
    let mut x = width - pad - total;
    for w in widths {
        l.buttons.push(RECT::new(x, y, w, bh));
        x += w + gap;
    }
    l.height = y + bh + s(22);
    l
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut()
}

fn ymd_to_systemtime(d: Ymd) -> SYSTEMTIME {
    SYSTEMTIME { wYear: d.y as u16, wMonth: d.m as u16, wDayOfWeek: 0, wDay: d.d as u16, wHour: 0, wMinute: 0, wSecond: 0, wMilliseconds: 0 }
}

/// 建立欄位的控制項並填入初始值
unsafe fn create_field(dlg: HWND, i: usize, field: &Field, r: RECT, fonts: &DlgFonts, dark: bool) -> HWND {
    let (class, style) = match field {
        Field::Date { optional, .. } => (DATETIMEPICK_CLASS, DTS_SHORTDATEFORMAT | if *optional { DTS_SHOWNONE } else { 0 }),
        Field::Text { .. } => ("Edit", ES_AUTOHSCROLL),
    };
    let class = wide(class);
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        std::ptr::null(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
        r.left,
        r.top,
        r.width(),
        r.height(),
        dlg,
        (FIRST_FIELD_ID + i as u16) as HMENU,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null(),
    );
    if h == 0 {
        return 0;
    }
    SendMessageW(h, WM_SETFONT, fonts.body as WPARAM, 1);
    match field {
        Field::Date { value, .. } => {
            theme::apply_explorer_theme(h, dark);
            match value {
                Some(d) => {
                    let st = ymd_to_systemtime(*d);
                    SendMessageW(h, DTM_SETSYSTEMTIME, GDT_VALID, &st as *const SYSTEMTIME as LPARAM);
                }
                None => {
                    SendMessageW(h, DTM_SETSYSTEMTIME, GDT_NONE, 0);
                }
            }
        }
        Field::Text { value, cue, .. } => {
            set_window_text(h, value);
            if !cue.is_empty() {
                let w = wide(cue);
                SendMessageW(h, EM_SETCUEBANNER, 1, w.as_ptr() as LPARAM);
            }
        }
    }
    h
}

/// 欄位目前的內容
unsafe fn field_value(field: &Field, h: HWND) -> FieldValue {
    match field {
        Field::Date { value, .. } => {
            if h == 0 {
                return FieldValue::Date(*value);
            }
            let mut st = SYSTEMTIME::default();
            let r = SendMessageW(h, DTM_GETSYSTEMTIME, 0, &mut st as *mut SYSTEMTIME as LPARAM);
            FieldValue::Date(if r as usize == GDT_VALID { Ymd::new(st.wYear as i32, st.wMonth as u32, st.wDay as u32) } else { None })
        }
        Field::Text { value, .. } => FieldValue::Text(if h == 0 { value.clone() } else { window_text(h) }),
    }
}

/// `h` 是輸入欄位（或欄位裡的子視窗，例如日期挑選器的勾選框）
unsafe fn input_index(st: &State, h: HWND) -> Option<usize> {
    st.inputs.iter().position(|i| *i != 0 && (*i == h || IsChild(*i, h) != 0))
}

/// 顯示對話方塊（強制回應），回傳按下的按鈕 id 與輸入欄位的內容。
fn show(owner: HWND, spec: Spec) -> (i32, Vec<FieldValue>) {
    let Some(t) = app::theme_opt() else {
        // 主視窗建立前（啟動失敗）只能用系統訊息方塊。
        let text = spec.plain_text();
        message_box(owner, &text, MB_OK);
        let values = spec.fields.iter().map(|f| unsafe { field_value(f, 0) }).collect();
        return (spec.cancel, values);
    };
    let cancel = spec.cancel;
    unsafe {
        // 跟著文字大小放大；放不進螢幕的工作區時縮小（最小 100%）
        let fit_mon = if owner != 0 { MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST) } else { MonitorFromPoint(cursor_pos(), MONITOR_DEFAULTTONEAREST) };
        let mut fit_mi: MONITORINFO = std::mem::zeroed();
        fit_mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let fit_work = if GetMonitorInfoW(fit_mon, &mut fit_mi) != 0 { Some(fit_mi.rcWork) } else { None };
        let screen = GetDC(0);
        let mut percent = t.text_size.max(100);
        let (fonts, layout) = loop {
            let mut fonts = DlgFonts::new(t, percent);
            let layout = compute_layout(&spec, &fonts, screen);
            let fits = fit_work.is_none_or(|w| layout.width <= w.width() * 96 / 100 && layout.height <= w.height() * 92 / 100);
            if fits || percent <= 100 {
                break (fonts, layout);
            }
            fonts.destroy();
            percent = (percent - 5).max(100);
        };
        ReleaseDC(0, screen);
        let class = wide(CLASS);
        let title = wide(&spec.title);
        let dlg = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            0,
            0,
            layout.width,
            layout.height,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        if dlg == 0 {
            message_box(owner, &spec.plain_text(), MB_OK);
            let values = spec.fields.iter().map(|f| field_value(f, 0)).collect();
            return (cancel, values);
        }
        theme::style_caption(dlg, &t.pal, t.pal.surface);
        // 依框線補足視窗大小，置中於 owner（沒有 owner 時置中於游標所在螢幕）。
        let mut wr = RECT::default();
        GetWindowRect(dlg, &mut wr);
        let cr = client_rect(dlg);
        let (w, h) = (layout.width + wr.width() - cr.width(), layout.height + wr.height() - cr.height());
        let area = if owner != 0 && IsIconic(owner) == 0 {
            let mut r = RECT::default();
            GetWindowRect(owner, &mut r);
            r
        } else {
            let mon = MonitorFromPoint(cursor_pos(), MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            mi.rcWork
        };
        // 不要超出螢幕工作區
        let mon = MonitorFromWindow(if owner != 0 { owner } else { dlg }, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let work = if GetMonitorInfoW(mon, &mut mi) != 0 { mi.rcWork } else { area };
        let x = (area.left + (area.width() - w) / 2).clamp(work.left, (work.right - w).max(work.left));
        let y = (area.top + (area.height() - h) / 2).clamp(work.top, (work.bottom - h).max(work.top));
        SetWindowPos(dlg, 0, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);

        let mut buttons = Vec::new();
        for (i, b) in spec.buttons.iter().enumerate() {
            let label = match b.key {
                Some(k) => format!("{} ({})", b.label, k.to_ascii_uppercase()),
                None => b.label.clone(),
            };
            let h = controls::create(dlg, FIRST_BUTTON_ID + i as u16, Kind::Text, &label, Surface::Card, true);
            if b.primary {
                controls::set_primary(h);
            }
            controls::set_fonts(h, fonts.body, fonts.body_bold);
            move_window(h, layout.buttons[i]);
            buttons.push(h);
        }
        let inputs: Vec<HWND> = spec.fields.iter().enumerate().map(|(i, f)| create_field(dlg, i, f, layout.fields[i].2, &fonts, t.pal.dark)).collect();
        // 螢幕閱讀器：標題、內文都是自己畫的，讀不到；把完整的文字設成工作區的名稱，角色設成對話方塊
        access::set_role(dlg, access::ROLE_DIALOG);
        access::set_name(dlg, &spec.plain_text());
        let default_index = spec
            .focus
            .and_then(|id| spec.buttons.iter().position(|b| b.id == id))
            .or_else(|| spec.buttons.iter().position(|b| b.primary))
            .unwrap_or(0);
        let mut pop = Tween::new(0.0).with_ease(Ease::OutBack);
        pop.retarget(1.0, anim::now(), anim::dur(POP_MS));
        let first_input = inputs.iter().copied().find(|h| *h != 0);
        let raw = Box::into_raw(Box::new(State { spec, fonts, layout, buttons, inputs, result: cancel, done: false, pop }));
        SetWindowLongPtrW(dlg, GWLP_USERDATA, raw as isize);

        let owner_was_enabled = owner != 0 && IsWindowEnabled(owner) != 0;
        if owner_was_enabled {
            EnableWindow(owner, 0);
        }
        ShowWindow(dlg, SW_SHOW);
        let default_button = (&*raw).buttons.get(default_index).copied();
        // 有輸入欄位時焦點在第一個欄位（文字欄全選，直接打字就取代）
        if let Some(h) = first_input {
            SetFocus(h);
            SendMessageW(h, EM_SETSEL, 0, -1);
        } else if let Some(b) = default_button {
            SetFocus(b);
        }

        let mut msg: MSG = std::mem::zeroed();
        let mut quit = None;
        while !std::ptr::read_volatile(std::ptr::addr_of!((*raw).done)) {
            let r = GetMessageW(&mut msg, 0, 0, 0);
            if r <= 0 {
                quit = Some(msg.wParam as i32);
                break;
            }
            let ours = msg.hwnd == dlg || IsChild(dlg, msg.hwnd) != 0;
            if ours && handle_key(dlg, &mut *raw, &msg) {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // 先把 owner 交回使用者，再關閉對話方塊，啟用中的視窗才不會跳到別的程式。
        if owner_was_enabled {
            EnableWindow(owner, 1);
            SetActiveWindow(owner);
        }
        // 欄位的內容要在視窗關閉前讀出來
        let values: Vec<FieldValue> = {
            let st = &*raw;
            st.spec.fields.iter().zip(&st.inputs).map(|(f, h)| field_value(f, *h)).collect()
        };
        SetWindowLongPtrW(dlg, GWLP_USERDATA, 0);
        access::clear(dlg);
        DestroyWindow(dlg);
        let mut st = Box::from_raw(raw);
        st.fonts.destroy();
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
        (st.result, values)
    }
}

/// 對話方塊的鍵盤處理；回傳 true 表示已處理（不再翻譯成字元訊息，因此不會有提示音）。
unsafe fn handle_key(dlg: HWND, st: &mut State, msg: &MSG) -> bool {
    // 焦點在輸入欄位：按鍵交給欄位（打字、方向鍵、Ctrl+C／V），只有 Enter、Esc、Tab 由對話方塊處理
    if let Some(index) = input_index(st, GetFocus()) {
        let vk = msg.wParam as u32;
        match msg.message {
            WM_KEYDOWN if matches!(vk, VK_RETURN | VK_ESCAPE) => {
                if msg.lParam & (1 << 30) == 0 {
                    st.result = if vk == VK_ESCAPE { st.spec.cancel } else { st.spec.buttons.iter().find(|b| b.primary).map_or(st.spec.cancel, |b| b.id) };
                    st.done = true;
                }
                return true;
            }
            WM_KEYDOWN if vk == VK_TAB => {
                let order: Vec<HWND> = st.inputs.iter().chain(st.buttons.iter()).copied().filter(|h| *h != 0).collect();
                let cur = order.iter().position(|h| *h == st.inputs[index]).unwrap_or(0) as isize;
                let step = if key_down(VK_SHIFT) { -1 } else { 1 };
                if let Some(next) = order.get((cur + step).rem_euclid(order.len().max(1) as isize) as usize) {
                    SetFocus(*next);
                    SendMessageW(*next, EM_SETSEL, 0, -1);
                }
                return true;
            }
            // Enter、Esc、Tab 的字元訊息不送進編輯框（不然會發出提示音）
            WM_CHAR if matches!(msg.wParam as u32, 0x0D | 0x1B | 0x09) => return true,
            WM_SYSKEYDOWN if vk == VK_F4 => {
                st.result = st.spec.cancel;
                st.done = true;
                return true;
            }
            _ => return false,
        }
    }
    match msg.message {
        WM_KEYDOWN => {
            let vk = msg.wParam as u32;
            // 自動重複（按住不放）的 Enter、空白鍵、Esc 與按鈕字母不算選擇：
            // 對話方塊開啟前就按著的鍵（例如在重新命名框按住 Enter）不會一出現就替使用者按下按鈕。
            let repeat = msg.lParam & (1 << 30) != 0;
            let choosing = matches!(vk, VK_RETURN | VK_SPACE | VK_ESCAPE) || st.spec.buttons.iter().any(|b| b.key.is_some_and(|k| k.to_ascii_uppercase() as u32 == vk));
            if repeat && choosing {
                return true;
            }
            let focus = GetFocus();
            let focused = st.buttons.iter().position(|b| *b == focus);
            let mut chosen: Option<i32> = None;
            let mut step: isize = 0;
            match vk {
                VK_ESCAPE => chosen = Some(st.spec.cancel),
                VK_RETURN | VK_SPACE => {
                    let index = focused.or_else(|| st.spec.buttons.iter().position(|b| b.primary)).unwrap_or(0);
                    chosen = Some(st.spec.buttons.get(index).map(|b| b.id).unwrap_or(st.spec.cancel));
                }
                VK_TAB => {
                    // 從按鈕往回、或從最後一個按鈕往下：回到輸入欄位
                    let order: Vec<HWND> = st.inputs.iter().chain(st.buttons.iter()).copied().filter(|h| *h != 0).collect();
                    if order.len() > st.buttons.len() {
                        let cur = order.iter().position(|h| *h == focus).unwrap_or(0) as isize;
                        let next = order[(cur + if key_down(VK_SHIFT) { -1 } else { 1 }).rem_euclid(order.len() as isize) as usize];
                        SetFocus(next);
                        SendMessageW(next, EM_SETSEL, 0, -1);
                    } else {
                        step = if key_down(VK_SHIFT) { -1 } else { 1 };
                    }
                }
                VK_LEFT | VK_UP => step = -1,
                VK_RIGHT | VK_DOWN => step = 1,
                _ if key_down(VK_CONTROL) && vk == b'C' as u32 => {
                    let _ = clipboard::set_text(dlg, &st.spec.plain_text());
                }
                _ => {
                    chosen = st.spec.buttons.iter().find(|b| b.key.is_some_and(|k| k.to_ascii_uppercase() as u32 == vk)).map(|b| b.id);
                }
            }
            let n = st.buttons.len();
            if step != 0 && n > 0 {
                let cur = focused.unwrap_or(0) as isize;
                SetFocus(st.buttons[(cur + step).rem_euclid(n as isize) as usize]);
            }
            if let Some(id) = chosen {
                st.result = id;
                st.done = true;
            }
            true
        }
        WM_SYSKEYDOWN => {
            if msg.wParam as u32 == VK_F4 {
                st.result = st.spec.cancel;
                st.done = true;
            }
            true
        }
        WM_KEYUP | WM_SYSKEYUP | WM_CHAR | WM_SYSCHAR => true,
        _ => false,
    }
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(st) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            paint(hwnd, st);
            return 0;
        }
        WM_CTLCOLOREDIT => {
            let t = app::theme();
            let hdc = wparam as HDC;
            SetTextColor(hdc, t.pal.ink);
            SetBkColor(hdc, t.pal.surface);
            return t.brushes.surface;
        }
        WM_COMMAND => {
            let id = loword(wparam);
            let code = hiword(wparam) as u32;
            if id >= FIRST_FIELD_ID {
                // 編輯框得到、失去焦點：外框換顏色
                if code == EN_SETFOCUS || code == EN_KILLFOCUS {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                return 0;
            }
            if id >= FIRST_BUTTON_ID {
                if let Some(b) = st.spec.buttons.get((id - FIRST_BUTTON_ID) as usize) {
                    st.result = b.id;
                    st.done = true;
                }
            }
            return 0;
        }
        WM_CLOSE => {
            st.result = st.spec.cancel;
            st.done = true;
            return 0;
        }
        WM_SYSCOMMAND => {
            // Alt＋字母沒有對應選單時，系統會發出提示音；直接略過。
            if wparam & 0xFFF0 == SC_KEYMENU && lparam != b' ' as LPARAM {
                return 0;
            }
        }
        WM_SETCURSOR => {
            if swallow_modal_click(hwnd, lparam) {
                return 1;
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn paint(hwnd: HWND, st: &State) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let rect = client_rect(hwnd);
    {
        let buf = Buffered::new(hdc, rect);
        let dc = buf.hdc;
        let t = app::theme();
        let p = &t.pal;
        let f = &st.fonts;
        let l = &st.layout;
        theme::fill(dc, &rect, p.surface);

        // 圓形圖示：開啟時從 60% 彈出到原尺寸
        let (g, color) = icon_style(st.spec.icon, t);
        let v = anim::value(&st.pop);
        let full = l.icon.width();
        let d = ((full as f32) * (0.6 + 0.4 * v)).round() as i32;
        let cx = l.icon.left + full / 2;
        let cy = l.icon.top + full / 2;
        let circle = RECT::new(cx - d / 2, cy - d / 2, d, d);
        let tint = theme::blend(color, p.surface, if p.dark { 0.22 } else { 0.15 });
        theme::circle(dc, &circle, Some(anim::mix(p.surface, tint, v.min(1.0))), None);
        theme::draw_text(dc, &g.to_string(), &circle, f.icon, anim::mix(p.surface, color, v.min(1.0)), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        if anim::running(&st.pop) {
            anim::request_frame(hwnd);
        }

        theme::draw_text(dc, &st.spec.heading, &l.heading, f.heading, p.ink, DT_LEFT | DT_WORDBREAK);
        if !st.spec.body.is_empty() {
            theme::draw_text(dc, &st.spec.body, &l.body, f.body, theme::blend(p.ink, p.surface, 0.78), DT_LEFT | DT_WORDBREAK);
        }
        for ((key, desc), (k, v)) in l.rows.iter().zip(&st.spec.rows) {
            theme::round_rect(dc, key, f.k(8), Some(p.chip), Some(p.line));
            theme::draw_text(dc, k, key, f.small_bold, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            theme::draw_text(dc, v, desc, f.body, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        let focus = GetFocus();
        for (i, ((label, frame, _), field)) in l.fields.iter().zip(&st.spec.fields).enumerate() {
            theme::draw_text(dc, field.label(), label, f.body, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            if matches!(field, Field::Text { .. }) {
                let focused = st.inputs.get(i).is_some_and(|h| *h != 0 && *h == focus);
                theme::round_rect(dc, frame, f.k(10), Some(p.surface), Some(if focused { p.accent } else { p.line }));
            }
        }
        if !st.spec.footnote.is_empty() {
            theme::draw_text(dc, &st.spec.footnote, &l.footnote, f.small, p.muted, DT_LEFT | DT_WORDBREAK);
        }
    }
    EndPaint(hwnd, &ps);
}

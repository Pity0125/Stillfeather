//! 與 App 風格一致的對話方塊，取代 Windows 的 MessageBox：
//! 卡片色底、重點色圓形圖示（開啟時彈出）、膠囊按鈕，標題列與卡片同色。
//!
//! 安靜：不呼叫 MessageBeep，也不經過 IsDialogMessage（它在按到沒有對應的字母時會發出提示音），
//! 鍵盤由這裡自己處理：Enter 確認、Esc 取消、Tab／方向鍵切換按鈕、按鈕上標示的字母直接選取、
//! Ctrl+C 複製對話方塊文字（與 MessageBox 相同）。
//! 開啟與關閉沿用 Windows 視窗原生的淡入縮放動畫。

use super::anim;
use super::app;
use super::clipboard;
use super::controls::{self, Kind, Surface};
use super::sys::*;
use super::theme::{self, glyph, Buffered, Theme};
use super::util::*;
use feather_core::anim::{Ease, Tween};

pub const CLASS: &str = "FeatherFiles.Dialog";
const FIRST_BUTTON_ID: u16 = 100;
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

/// 對話方塊內容。以 `Spec::new` 建立後串接設定，最後呼叫 `show`。
pub struct Spec {
    title: String,
    icon: Icon,
    heading: String,
    body: String,
    rows: Vec<(String, String)>,
    footnote: String,
    buttons: Vec<Button>,
    cancel: i32,
    width: i32,
}

impl Spec {
    pub fn new(icon: Icon, heading: &str) -> Spec {
        Spec {
            title: "Stillfeather".into(),
            icon,
            heading: heading.into(),
            body: String::new(),
            rows: Vec::new(),
            footnote: String::new(),
            buttons: vec![Button::primary(1, "確定")],
            cancel: 1,
            width: 440,
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

    pub fn footnote(mut self, text: &str) -> Spec {
        self.footnote = text.into();
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
        .buttons(vec![Button::primary(1, ok).key('Y'), Button::normal(0, "取消").key('N')], 0)
        .show(owner)
        == 1
}

pub fn info(owner: HWND, heading: &str, body: &str) {
    Spec::new(Icon::Info, heading).body(body).show(owner);
}

pub fn error(owner: HWND, heading: &str, body: &str) {
    Spec::new(Icon::Error, heading).body(body).show(owner);
}

// ---------------------------------------------------------------- 實作

#[derive(Default)]
struct Layout {
    icon: RECT,
    heading: RECT,
    body: RECT,
    rows: Vec<(RECT, RECT)>,
    footnote: RECT,
    buttons: Vec<RECT>,
    width: i32,
    height: i32,
}

struct State {
    spec: Spec,
    layout: Layout,
    buttons: Vec<HWND>,
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

fn compute_layout(spec: &Spec, t: &Theme, hdc: HDC) -> Layout {
    let s = |v: i32| t.s(v);
    let f = &t.fonts;
    let pad = s(26);
    let icon_d = s(44);
    let width = s(spec.width);
    let x0 = pad + icon_d + s(16);
    let tw = width - x0 - pad;
    let mut l = Layout { width, icon: RECT::new(pad, pad, icon_d, icon_d), ..Default::default() };
    let wrap = DT_WORDBREAK;
    let hh = theme::text_height(hdc, &spec.heading, f.brand, tw, wrap).max(f.ui_height);
    let mut y = pad;
    let only_heading = spec.body.is_empty() && spec.rows.is_empty() && spec.footnote.is_empty();
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
        let h = theme::text_height(hdc, &spec.body, f.ui, tw, wrap);
        l.body = RECT::new(x0, y, tw, h);
        y += h;
    }
    if !spec.rows.is_empty() {
        y += s(16);
        let key_w = spec.rows.iter().map(|(k, _)| theme::text_width(hdc, k, f.small_bold)).max().unwrap_or(0) + s(22);
        let key_w = key_w.min(tw / 2);
        let row_h = s(30);
        let chip_h = s(24);
        for _ in &spec.rows {
            let key = RECT::new(x0, y + (row_h - chip_h) / 2, key_w, chip_h);
            let desc = RECT { left: x0 + key_w + s(14), top: y, right: x0 + tw, bottom: y + row_h };
            l.rows.push((key, desc));
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
    let widths: Vec<i32> = spec.buttons.iter().map(|b| (theme::text_width(hdc, &b.label, f.ui_bold) + s(44)).max(s(100))).collect();
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

/// 顯示對話方塊（強制回應），回傳按下的按鈕 id。
fn show(owner: HWND, spec: Spec) -> i32 {
    let Some(t) = app::theme_opt() else {
        // 主視窗建立前（啟動失敗）只能用系統訊息方塊。
        let text = spec.plain_text();
        message_box(owner, &text, MB_OK);
        return spec.cancel;
    };
    let cancel = spec.cancel;
    unsafe {
        let screen = GetDC(0);
        let layout = compute_layout(&spec, t, screen);
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
            return cancel;
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
            move_window(h, layout.buttons[i]);
            buttons.push(h);
        }
        let default_index = spec.buttons.iter().position(|b| b.primary).unwrap_or(0);
        let mut pop = Tween::new(0.0).with_ease(Ease::OutBack);
        pop.retarget(1.0, anim::now(), anim::dur(POP_MS));
        let raw = Box::into_raw(Box::new(State { spec, layout, buttons, result: cancel, done: false, pop }));
        SetWindowLongPtrW(dlg, GWLP_USERDATA, raw as isize);

        let owner_was_enabled = owner != 0 && IsWindowEnabled(owner) != 0;
        if owner_was_enabled {
            EnableWindow(owner, 0);
        }
        ShowWindow(dlg, SW_SHOW);
        let default_button = (&*raw).buttons.get(default_index).copied();
        if let Some(b) = default_button {
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
        SetWindowLongPtrW(dlg, GWLP_USERDATA, 0);
        DestroyWindow(dlg);
        let st = Box::from_raw(raw);
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
        st.result
    }
}

/// 對話方塊的鍵盤處理；回傳 true 表示已處理（不再翻譯成字元訊息，因此不會有提示音）。
unsafe fn handle_key(dlg: HWND, st: &mut State, msg: &MSG) -> bool {
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
                VK_TAB => step = if key_down(VK_SHIFT) { -1 } else { 1 },
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
        WM_COMMAND => {
            let id = loword(wparam);
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
        let f = &t.fonts;
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

        theme::draw_text(dc, &st.spec.heading, &l.heading, f.brand, p.ink, DT_LEFT | DT_WORDBREAK);
        if !st.spec.body.is_empty() {
            theme::draw_text(dc, &st.spec.body, &l.body, f.ui, theme::blend(p.ink, p.surface, 0.78), DT_LEFT | DT_WORDBREAK);
        }
        for ((key, desc), (k, v)) in l.rows.iter().zip(&st.spec.rows) {
            theme::round_rect(dc, key, t.s(8), Some(p.chip), Some(p.line));
            theme::draw_text(dc, k, key, f.small_bold, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            theme::draw_text(dc, v, desc, f.ui, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        if !st.spec.footnote.is_empty() {
            theme::draw_text(dc, &st.spec.footnote, &l.footnote, f.small, p.muted, DT_LEFT | DT_WORDBREAK);
        }
    }
    EndPaint(hwnd, &ps);
}

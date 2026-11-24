//! 自繪的扁平按鈕：平時沒有外框，懸停與按下以低調底色回應，停用時淡化，
//! 鍵盤焦點以重點色細框表示。也用作勾選方塊（Toggle）與分段選項（Segment）。
//! 懸停、按下與勾選狀態都以短動畫淡入淡出。

use super::access;
use super::anim;
use super::app;
use feather_core::anim::Tween;
use super::sys::*;
use super::theme::{self, Buffered};
use super::util::*;

pub const BUTTON_CLASS: &str = "FeatherFiles.Button";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 只有圖示字形，平時透明，懸停時出現圓形底色；視窗文字作為提示與螢幕閱讀器名稱。
    Icon(char),
    /// 圓形按鈕：卡片色底與細框（像參考圖右上角的月亮、鈴鐺）。
    Round(char),
    /// 膠囊形文字按鈕；設為 primary 時以重點色實心顯示。
    Text,
    /// 膠囊形按鈕，前面帶圖示字形。
    Pill(char),
    /// 下拉選單按鈕：圖示字形＋文字＋向下箭頭；選單開著時以勾選狀態顯示（淡重點色底）。
    Menu(char),
    /// 勾選方塊：字形 + 文字。
    Toggle,
    /// 分段選項：選取時以淡重點色底與重點色框顯示。
    Segment,
}

/// 按鈕所在區域的底色。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// 頁面底（米白／近黑）
    Page,
    /// 卡片、對話方塊、搜尋框內
    Card,
    /// 側欄卡片
    Sidebar,
    /// 側欄的淡杏色提示卡
    Peach,
}

struct Data {
    kind: Kind,
    surface: Surface,
    checked: bool,
    hot: bool,
    pressed: bool,
    tracking: bool,
    focus_on_click: bool,
    primary: bool,
    /// 低調的圖示按鈕：平時用次要文字色，懸停才轉成主要文字色
    subtle: bool,
    /// 動畫中的懸停、按下與勾選程度（0～1）
    hover_fx: Tween,
    press_fx: Tween,
    check_fx: Tween,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(BUTTON_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: button_proc,
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

/// 建立按鈕。`text` 同時是可及性名稱；圖示按鈕的提示文字也用它。
pub fn create(parent: HWND, id: u16, kind: Kind, text: &str, surface: Surface, focus_on_click: bool) -> HWND {
    let data = Box::new(Data {
        kind,
        surface,
        checked: false,
        hot: false,
        pressed: false,
        tracking: false,
        focus_on_click,
        primary: false,
        subtle: false,
        hover_fx: Tween::new(0.0),
        press_fx: Tween::new(0.0),
        check_fx: Tween::new(0.0),
    });
    let class = wide(BUTTON_CLASS);
    let name = wide(text);
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            name.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            0,
            10,
            10,
            parent,
            id as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        // 螢幕閱讀器：系統只把這個類別當成一般的窗格，補上角色（名稱就是視窗文字）
        match kind {
            Kind::Toggle | Kind::Segment => {
                access::set_role(hwnd, if kind == Kind::Toggle { access::ROLE_CHECKBUTTON } else { access::ROLE_RADIOBUTTON });
                access::set_check_state(hwnd, false, false);
            }
            _ => access::set_role(hwnd, access::ROLE_PUSHBUTTON),
        }
        hwnd
    }
}

/// 有勾選狀態的按鈕（核取方塊、分段選項）
fn checkable(d: &Data) -> bool {
    matches!(d.kind, Kind::Toggle | Kind::Segment)
}

unsafe fn data<'a>(hwnd: HWND) -> Option<&'a mut Data> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data;
    p.as_mut()
}

pub fn set_checked(hwnd: HWND, checked: bool) {
    unsafe {
        if let Some(d) = data(hwnd) {
            if d.checked != checked {
                d.checked = checked;
                // 還沒顯示的按鈕（例如對話方塊剛建立時）直接套用，不播放動畫。
                if IsWindowVisible(hwnd) != 0 {
                    anim::to(&mut d.check_fx, checked as u8 as f32, anim::TOGGLE_MS);
                } else {
                    d.check_fx.snap(checked as u8 as f32);
                }
                if checkable(d) {
                    access::set_check_state(hwnd, checked, true);
                }
                invalidate(hwnd);
            }
        }
    }
}

/// 更換圖示字形（例如深淺色切換鈕的月亮／太陽）。
pub fn set_glyph(hwnd: HWND, g: char) {
    unsafe {
        if let Some(d) = data(hwnd) {
            let new = match d.kind {
                Kind::Icon(_) => Kind::Icon(g),
                Kind::Round(_) => Kind::Round(g),
                Kind::Pill(_) => Kind::Pill(g),
                Kind::Menu(_) => Kind::Menu(g),
                k => k,
            };
            if new != d.kind {
                d.kind = new;
                invalidate(hwnd);
            }
        }
    }
}

pub fn is_checked(hwnd: HWND) -> bool {
    unsafe { data(hwnd).map(|d| d.checked).unwrap_or(false) }
}

/// 對話方塊的主要按鈕（儲存）以重點色顯示。
pub fn set_primary(hwnd: HWND) {
    unsafe {
        if let Some(d) = data(hwnd) {
            d.primary = true;
        }
    }
}

/// 圖示按鈕改成低調樣式（例如側欄標頭的說明按鈕），平時不搶程式名稱的視線。
pub fn set_subtle(hwnd: HWND) {
    unsafe {
        if let Some(d) = data(hwnd) {
            d.subtle = true;
            invalidate(hwnd);
        }
    }
}

pub fn set_enabled(hwnd: HWND, enabled: bool) {
    unsafe {
        if (IsWindowEnabled(hwnd) != 0) != enabled {
            EnableWindow(hwnd, enabled as BOOL);
            invalidate(hwnd);
        }
    }
}

fn click(hwnd: HWND, d: &mut Data) {
    if matches!(d.kind, Kind::Toggle) {
        d.checked = !d.checked;
        anim::to(&mut d.check_fx, d.checked as u8 as f32, anim::TOGGLE_MS);
        access::set_check_state(hwnd, d.checked, true);
    }
    invalidate(hwnd);
    unsafe {
        let id = GetWindowLongPtrW(hwnd, -12 /* GWLP_ID */) as usize & 0xFFFF;
        PostMessageW(GetParent(hwnd), WM_COMMAND, id, hwnd);
    }
}

unsafe extern "system" fn button_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(d) = data(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            paint(hwnd, d);
            return 0;
        }
        WM_MOUSEMOVE => {
            if !d.tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                d.tracking = true;
            }
            let r = client_rect(hwnd);
            let inside = r.contains(get_x_lparam(lparam), get_y_lparam(lparam));
            if inside != d.hot {
                d.hot = inside;
                invalidate(hwnd);
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            d.tracking = false;
            d.hot = false;
            invalidate(hwnd);
            return 0;
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if d.focus_on_click {
                SetFocus(hwnd);
            }
            d.pressed = true;
            d.hot = true;
            SetCapture(hwnd);
            invalidate(hwnd);
            return 0;
        }
        WM_LBUTTONUP => {
            let was = d.pressed;
            d.pressed = false;
            if GetCapture() == hwnd {
                ReleaseCapture();
            }
            let r = client_rect(hwnd);
            if was && r.contains(get_x_lparam(lparam), get_y_lparam(lparam)) {
                click(hwnd, d);
            }
            invalidate(hwnd);
            return 0;
        }
        WM_CAPTURECHANGED => {
            d.pressed = false;
            invalidate(hwnd);
            return 0;
        }
        WM_KEYDOWN => {
            if wparam as u32 == VK_SPACE || wparam as u32 == VK_RETURN {
                click(hwnd, d);
                return 0;
            }
        }
        WM_GETDLGCODE => {
            let m = lparam as *const MSG;
            if !m.is_null() && (*m).message == WM_KEYDOWN && (*m).wParam as u32 == VK_RETURN {
                return DLGC_WANTALLKEYS;
            }
            return DLGC_BUTTON;
        }
        WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE | WM_SETTEXT => {
            let r = DefWindowProcW(hwnd, msg, wparam, lparam);
            // 覆寫了狀態的按鈕：焦點與啟用要自己更新（啟用改變時通知）
            if checkable(d) && msg != WM_SETTEXT {
                access::set_check_state(hwnd, d.checked, msg == WM_ENABLE);
            }
            invalidate(hwnd);
            return r;
        }
        WM_NCDESTROY => {
            access::clear(hwnd);
            let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Data;
            if !p.is_null() {
                drop(Box::from_raw(p));
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn paint(hwnd: HWND, d: &mut Data) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let rect = client_rect(hwnd);
    {
        let buf = Buffered::new(hdc, rect);
        let dc = buf.hdc;
        let t = app::theme();
        let p = &t.pal;
        let base = match d.surface {
            Surface::Page => p.background,
            Surface::Card => p.surface,
            Surface::Sidebar => p.sidebar,
            Surface::Peach => p.peach,
        };
        theme::fill(dc, &rect, base);
        let enabled = IsWindowEnabled(hwnd) != 0;
        let focused = GetFocus() == hwnd;
        let inner = rect.inflate(-1, -1);
        let pill = inner.height(); // 圓角半徑等於高度 → 膠囊形

        // 依狀態更新動畫目標，再取目前的值
        anim::to(&mut d.hover_fx, (enabled && d.hot) as u8 as f32, anim::HOVER_MS);
        anim::to(&mut d.press_fx, (enabled && d.pressed && d.hot) as u8 as f32, anim::PRESS_MS);
        if (d.check_fx.target() > 0.5) != d.checked {
            anim::to(&mut d.check_fx, d.checked as u8 as f32, anim::TOGGLE_MS);
        }
        let h = anim::value(&d.hover_fx);
        let pr = anim::value(&d.press_fx);
        let ck = anim::value(&d.check_fx).clamp(0.0, 1.0);
        if anim::running(&d.hover_fx) || anim::running(&d.press_fx) || anim::running(&d.check_fx) {
            anim::request_frame(hwnd);
        }

        let normal_ink = if enabled { p.ink } else { theme::blend(p.ink, base, 0.35) };
        // 懸停時在底色上疊一層淡淡的文字色，按下時再深一點
        let hover_on = |c: COLORREF| theme::blend(p.ink, c, (0.07 + 0.05 * pr) * h);
        let text = window_text(hwnd);

        match d.kind {
            Kind::Icon(g) => {
                let checked_fill = anim::mix(base, p.selected_tint, ck);
                let fill = hover_on(checked_fill);
                if fill != base {
                    theme::circle(dc, &inner, Some(fill), None);
                }
                // 低調樣式：平時是次要文字色，懸停時淡入主要文字色
                let resting = if d.subtle && enabled { anim::mix(theme::blend(normal_ink, base, 0.6), normal_ink, h) } else { normal_ink };
                let ink = if enabled { anim::mix(resting, p.accent, ck) } else { normal_ink };
                theme::draw_text(dc, &g.to_string(), &rect, t.fonts.icon, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            Kind::Round(g) => {
                let fill = hover_on(anim::mix(p.surface, p.selected_tint, ck));
                let border = anim::mix(p.line, p.accent, ck);
                theme::circle(dc, &inner, Some(fill), Some(border));
                let ink = if enabled { anim::mix(normal_ink, p.accent, ck) } else { normal_ink };
                theme::draw_text(dc, &g.to_string(), &rect, t.fonts.icon, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            Kind::Text | Kind::Pill(_) | Kind::Menu(_) | Kind::Segment => {
                let is_segment = matches!(d.kind, Kind::Segment);
                let is_menu = matches!(d.kind, Kind::Menu(_));
                let (fill, border, ink) = if d.primary && enabled {
                    let lifted = theme::blend(p.surface, p.accent, 0.12 * h);
                    (theme::blend(p.ink, lifted, 0.15 * pr), None, theme::contrast_text(p.accent))
                } else {
                    let resting = if d.surface == Surface::Card { p.surface } else { theme::blend(p.surface, base, 0.85) };
                    let sel = if is_segment || is_menu { ck } else { 0.0 };
                    let fill = anim::mix(hover_on(resting), p.selected_tint, sel);
                    (fill, Some(anim::mix(p.line, p.accent, sel)), normal_ink)
                };
                theme::round_rect(dc, &inner, pill, Some(fill), border);
                if let Kind::Menu(g) = d.kind {
                    // 圖示 + 文字 + 向下箭頭，整組置中
                    let gw = t.s(22);
                    let cw = t.s(18);
                    // 太窄時只畫圖示與箭頭（文字仍是提示與可及性名稱）
                    let full_tw = theme::text_width(dc, &text, t.fonts.ui);
                    let tw = if gw + full_tw + cw + t.s(20) <= rect.width() { full_tw } else { 0 };
                    let total = gw + tw + cw;
                    let x = rect.left + (rect.width() - total).max(0) / 2;
                    let gr = RECT { left: x, top: rect.top, right: x + gw, bottom: rect.bottom };
                    theme::draw_text(dc, &g.to_string(), &gr, t.fonts.icon_small, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    if tw > 0 {
                        let tr = RECT { left: x + gw, top: rect.top, right: x + gw + tw, bottom: rect.bottom };
                        theme::draw_text(dc, &text, &tr, t.fonts.ui, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    }
                    let cr = RECT { left: x + gw + tw, top: rect.top + t.s(1), right: x + total, bottom: rect.bottom };
                    theme::draw_text(dc, &theme::glyph::CHEVRON_DOWN_SMALL.to_string(), &cr, t.fonts.icon_small, anim::mix(ink, p.muted, 0.35), DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
                } else if let Kind::Pill(g) = d.kind {
                    // 圖示 + 文字，整組置中
                    let gw = t.s(22);
                    let tw = theme::text_width(dc, &text, t.fonts.ui_bold);
                    let total = gw + tw;
                    let x = rect.left + (rect.width() - total).max(0) / 2;
                    let gr = RECT { left: x, top: rect.top, right: x + gw, bottom: rect.bottom };
                    theme::draw_text(dc, &g.to_string(), &gr, t.fonts.icon_small, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    let tr = RECT { left: x + gw, top: rect.top, right: rect.right, bottom: rect.bottom };
                    theme::draw_text(dc, &text, &tr, t.fonts.ui_bold, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                } else {
                    let font = if d.primary || (is_segment && d.checked) { t.fonts.ui_bold } else { t.fonts.ui };
                    theme::draw_text(dc, &text, &rect, font, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
            Kind::Toggle => {
                let hover_fill = hover_on(base);
                if hover_fill != base {
                    theme::round_rect(dc, &inner, pill, Some(hover_fill), None);
                }
                let box_w = t.s(24);
                let gr = RECT { left: rect.left + t.s(6), top: rect.top, right: rect.left + t.s(6) + box_w, bottom: rect.bottom };
                // 空框與打勾交叉淡化
                let off = theme::blend(normal_ink, base, 0.7);
                let on = if enabled { p.accent } else { normal_ink };
                if ck < 1.0 {
                    theme::draw_text(dc, &theme::glyph::CHECKBOX.to_string(), &gr, t.fonts.icon, anim::mix(base, off, 1.0 - ck), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                if ck > 0.0 {
                    theme::draw_text(dc, &theme::glyph::CHECKBOX_CHECKED.to_string(), &gr, t.fonts.icon, anim::mix(base, on, ck), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                let tr = RECT { left: gr.right + t.s(4), top: rect.top, right: rect.right - t.s(4), bottom: rect.bottom };
                theme::draw_text(dc, &text, &tr, t.fonts.ui, normal_ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            }
        }
        if focused {
            match d.kind {
                Kind::Icon(_) | Kind::Round(_) => theme::circle(dc, &inner, None, Some(p.accent)),
                _ => theme::round_rect(dc, &inner, pill, None, Some(p.accent)),
            }
        }
    }
    EndPaint(hwnd, &ps);
}

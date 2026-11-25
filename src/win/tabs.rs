//! 自繪分頁列：每欄一條，顯示資料夾名稱與關閉按鈕，右側有「＋」。
//! 分頁過多時縮小寬度，仍放不下就出現左右切換箭頭。
//! 操作一律以 PostMessage 交給主視窗處理，避免在繪圖程序中重入。

use super::anim;
use super::app;
use super::sys::*;
use super::theme::{self, glyph, Buffered};
use super::util::*;
use feather_core::anim::Tween;
use feather_core::workspace::Side;

pub const TABS_CLASS: &str = "FeatherFiles.Tabs";
pub const WM_APP_TAB: u32 = WM_APP + 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Select = 0,
    Close = 1,
    New = 2,
    Menu = 3,
}

impl Action {
    pub fn from_bits(v: usize) -> Option<Action> {
        match v {
            0 => Some(Action::Select),
            1 => Some(Action::Close),
            2 => Some(Action::New),
            3 => Some(Action::Menu),
            _ => None,
        }
    }
}

/// 分頁列需要的資料（由 app 提供）。
pub struct Model {
    pub titles: Vec<String>,
    pub selected: usize,
    pub active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Tab(usize),
    Close(usize),
    Plus,
    Prev,
    Next,
}

struct Data {
    side: Side,
    hot: Hit,
    pressed: Hit,
    tracking: bool,
    scroll: usize,
    /// 底線左右兩端（滑到新選取的分頁）
    bar_l: Tween,
    bar_r: Tween,
    /// 上次繪製時的選取分頁與視窗寬度：只是視窗縮放時底線直接跟上，不播放滑動。
    bar_state: Option<(usize, i32)>,
    /// 各分頁的懸停底色
    hover: Vec<Tween>,
    /// 拖放時滑鼠下的分頁
    drop: Option<usize>,
}

struct Layout {
    tabs: Vec<(usize, RECT, RECT)>,
    plus: RECT,
    prev: Option<RECT>,
    next: Option<RECT>,
    can_prev: bool,
    can_next: bool,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(TABS_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: tabs_proc,
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

pub fn create(parent: HWND, side: Side) -> HWND {
    let data = Box::new(Data {
        side,
        hot: Hit::None,
        pressed: Hit::None,
        tracking: false,
        scroll: 0,
        bar_l: Tween::new(0.0),
        bar_r: Tween::new(0.0),
        bar_state: None,
        hover: Vec::new(),
        drop: None,
    });
    let class = wide(TABS_CLASS);
    let name = wide(if side == Side::Left { tr!("左欄分頁", "Left pane tabs") } else { tr!("右欄分頁", "Right pane tabs") });
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            name.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            10,
            10,
            parent,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        hwnd
    }
}

unsafe fn data<'a>(hwnd: HWND) -> Option<&'a mut Data> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data).as_mut()
}

fn layout(hwnd: HWND, d: &mut Data, model: &Model, hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    let top = t.s(2);
    let bottom = r.bottom;
    let left = 0;
    let right = r.right - t.s(2);
    let plus_w = t.s(34);
    let arrow_w = t.s(26);
    // 分頁標題跟著「文字大小」放大（`body` 字型）：寬度也照同樣的比例
    let min_w = t.st(88);
    let max_w = t.st(220);
    let n = model.titles.len();
    let natural: Vec<i32> = model
        .titles
        .iter()
        .map(|title| (theme::text_width(hdc, title, t.fonts.body_bold) + t.s(14 + 6 + 18 + 8)).clamp(min_w, max_w))
        .collect();
    let avail = (right - left - plus_w - t.s(4)).max(min_w);
    let total: i32 = natural.iter().sum();
    let close_box = |tab: &RECT| {
        let size = t.s(18);
        let y = tab.top + (tab.height() - size) / 2 - t.s(1);
        RECT { left: tab.right - t.s(8) - size, top: y, right: tab.right - t.s(8), bottom: y + size }
    };
    let mut out = Layout { tabs: Vec::new(), plus: RECT::default(), prev: None, next: None, can_prev: false, can_next: false };
    if n == 0 {
        out.plus = RECT { left, top, right: left + plus_w, bottom };
        return out;
    }
    let shrink_w = (avail / n as i32).max(min_w);
    let (widths, first, count, scrolling) = if total <= avail {
        (natural.clone(), 0, n, false)
    } else if shrink_w * n as i32 <= avail {
        (natural.iter().map(|w| (*w).min(shrink_w)).collect(), 0, n, false)
    } else {
        let avail2 = avail - 2 * arrow_w;
        let visible = ((avail2 / min_w).max(1) as usize).min(n);
        if model.selected < d.scroll {
            d.scroll = model.selected;
        }
        if model.selected >= d.scroll + visible {
            d.scroll = model.selected + 1 - visible;
        }
        d.scroll = d.scroll.min(n - visible);
        let w = (avail2 / visible as i32).max(min_w).min(max_w);
        (vec![w; n], d.scroll, visible, true)
    };
    if !scrolling {
        d.scroll = 0;
    }
    let mut x = left;
    for i in first..first + count {
        let tab = RECT { left: x, top, right: x + widths[i], bottom };
        out.tabs.push((i, tab, close_box(&tab)));
        x += widths[i];
    }
    if scrolling {
        let arrows_left = right - plus_w - 2 * arrow_w;
        out.prev = Some(RECT { left: arrows_left, top, right: arrows_left + arrow_w, bottom });
        out.next = Some(RECT { left: arrows_left + arrow_w, top, right: arrows_left + 2 * arrow_w, bottom });
        out.can_prev = first > 0;
        out.can_next = first + count < n;
        out.plus = RECT { left: right - plus_w, top, right, bottom };
    } else {
        out.plus = RECT { left: x + t.s(4), top, right: x + t.s(4) + plus_w, bottom };
    }
    out
}

fn hit_test(l: &Layout, x: i32, y: i32) -> Hit {
    if l.plus.contains(x, y) {
        return Hit::Plus;
    }
    if l.prev.is_some_and(|r| r.contains(x, y)) {
        return Hit::Prev;
    }
    if l.next.is_some_and(|r| r.contains(x, y)) {
        return Hit::Next;
    }
    for (i, tab, close) in &l.tabs {
        if close.contains(x, y) {
            return Hit::Close(*i);
        }
        if tab.contains(x, y) {
            return Hit::Tab(*i);
        }
    }
    Hit::None
}

unsafe fn with_layout<R>(hwnd: HWND, d: &mut Data, f: impl FnOnce(&mut Data, &Layout, &Model) -> R) -> R {
    let model = app::tab_model(d.side);
    let hdc = GetDC(hwnd);
    let l = layout(hwnd, d, &model, hdc);
    ReleaseDC(hwnd, hdc);
    f(d, &l, &model)
}

/// 拖放用：螢幕座標下的分頁索引。
pub fn drop_hit(hwnd: HWND, screen: POINT) -> Option<usize> {
    let mut p = screen;
    unsafe {
        ScreenToClient(hwnd, &mut p);
        let d = data(hwnd)?;
        match with_layout(hwnd, d, |_, l, _| hit_test(l, p.x, p.y)) {
            Hit::Tab(i) | Hit::Close(i) => Some(i),
            _ => None,
        }
    }
}

/// 標示拖放中的分頁；None 清除。
pub fn set_drop(hwnd: HWND, index: Option<usize>) {
    unsafe {
        if let Some(d) = data(hwnd) {
            if d.drop != index {
                d.drop = index;
                invalidate(hwnd);
            }
        }
    }
}

/// 請主視窗切換到某個分頁（拖放時停在分頁上）。
pub fn request_select(side: Side, index: usize) {
    post(side, Action::Select, index, 0);
}

fn post(side: Side, action: Action, index: usize, lparam: LPARAM) {
    let wparam = (side.index()) | ((action as usize) << 4) | (index << 8);
    unsafe {
        PostMessageW(app::main_hwnd(), WM_APP_TAB, wparam, lparam);
    }
}

fn screen_lparam(hwnd: HWND, x: i32, y: i32) -> LPARAM {
    let mut p = POINT { x, y };
    unsafe {
        ClientToScreen(hwnd, &mut p);
    }
    ((p.x as u16 as u32) | ((p.y as u16 as u32) << 16)) as i32 as LPARAM
}

unsafe extern "system" fn tabs_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(d) = data(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    let (x, y) = (get_x_lparam(lparam), get_y_lparam(lparam));
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
            let hit = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y));
            if hit != d.hot {
                d.hot = hit;
                invalidate(hwnd);
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            d.tracking = false;
            d.hot = Hit::None;
            invalidate(hwnd);
            return 0;
        }
        WM_LBUTTONDOWN => {
            let hit = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y));
            d.pressed = hit;
            match hit {
                Hit::Tab(i) => post(d.side, Action::Select, i, 0),
                Hit::Prev => {
                    d.scroll = d.scroll.saturating_sub(1);
                    invalidate(hwnd);
                }
                Hit::Next => {
                    d.scroll += 1;
                    invalidate(hwnd);
                }
                Hit::None => post(d.side, Action::Select, usize::MAX >> 8, 0),
                _ => {}
            }
            return 0;
        }
        WM_LBUTTONUP => {
            let hit = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y));
            if hit == d.pressed {
                match hit {
                    Hit::Close(i) => post(d.side, Action::Close, i, 0),
                    Hit::Plus => post(d.side, Action::New, 0, 0),
                    _ => {}
                }
            }
            d.pressed = Hit::None;
            return 0;
        }
        WM_LBUTTONDBLCLK => {
            let hit = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y));
            if hit == Hit::None {
                post(d.side, Action::New, 0, 0);
            }
            return 0;
        }
        WM_MBUTTONUP => {
            if let Hit::Tab(i) | Hit::Close(i) = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y)) {
                post(d.side, Action::Close, i, 0);
            }
            return 0;
        }
        WM_RBUTTONUP => {
            let index = match with_layout(hwnd, d, |_, l, _| hit_test(l, x, y)) {
                Hit::Tab(i) | Hit::Close(i) => i,
                _ => usize::MAX >> 8,
            };
            post(d.side, Action::Menu, index, screen_lparam(hwnd, x, y));
            return 0;
        }
        WM_MOUSEWHEEL => {
            let delta = (hiword(wparam) as i16) as i32;
            let model = app::tab_model(d.side);
            if !model.titles.is_empty() {
                let n = model.titles.len() as isize;
                let next = (model.selected as isize + if delta < 0 { 1 } else { -1 }).rem_euclid(n) as usize;
                post(d.side, Action::Select, next, 0);
            }
            return 0;
        }
        WM_NCDESTROY => {
            let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Data;
            if !p.is_null() {
                drop(Box::from_raw(p));
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 參考圖的「Songs / Albums / Artists」：文字分頁，選取的分頁加粗並在下方畫重點色圓角底線。
unsafe fn paint(hwnd: HWND, d: &mut Data) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let rect = client_rect(hwnd);
    {
        let buf = Buffered::new(hdc, rect);
        let dc = buf.hdc;
        let t = app::theme();
        let p = &t.pal;
        theme::fill(dc, &rect, p.background);
        let model = app::tab_model(d.side);
        let l = layout(hwnd, d, &model, dc);
        let mut busy = false;
        if d.hover.len() != model.titles.len() {
            d.hover.resize(model.titles.len(), Tween::new(0.0));
        }
        let mut bar_target: Option<RECT> = None;
        for (i, tab, close) in &l.tabs {
            let selected = *i == model.selected;
            let hot = matches!(d.hot, Hit::Tab(h) | Hit::Close(h) if h == *i);
            let hv = match d.hover.get_mut(*i) {
                Some(tw) => {
                    anim::to(tw, (hot && !selected) as u8 as f32, anim::HOVER_MS);
                    busy |= anim::running(tw);
                    anim::value(tw)
                }
                None => 0.0,
            };
            let dropping = d.drop == Some(*i);
            if dropping {
                // 拖放目標：淡重點色底加重點色外框（停一下就切換過去）
                let r = RECT { left: tab.left + t.s(2), top: tab.top + t.s(4), right: tab.right - t.s(2), bottom: tab.bottom - t.s(8) };
                theme::round_rect(dc, &r, r.height(), Some(theme::blend(p.selected_tint, p.background, 0.7)), Some(p.accent));
            } else if hv > 0.001 {
                let r = RECT { left: tab.left + t.s(2), top: tab.top + t.s(4), right: tab.right - t.s(2), bottom: tab.bottom - t.s(8) };
                theme::round_rect(dc, &r, r.height(), Some(anim::mix(p.background, p.chip, hv)), None);
            }
            let title = model.titles.get(*i).map(String::as_str).unwrap_or("");
            let show_close = selected || hot;
            let text_right = if show_close { close.left - t.s(2) } else { tab.right - t.s(10) };
            let tr = RECT { left: tab.left + t.s(14), top: tab.top, right: text_right, bottom: tab.bottom - t.s(6) };
            let color = if selected || dropping { p.ink } else { anim::mix(p.muted, p.ink, hv) };
            let font = if selected { t.fonts.body_bold } else { t.fonts.body };
            theme::draw_text(dc, title, &tr, font, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            if selected {
                // 底線寬度跟著文字
                let text_w = theme::text_width(dc, title, t.fonts.body_bold).min(tr.width());
                let bar_h = t.s(3);
                bar_target = Some(RECT {
                    left: tr.left - t.s(4),
                    top: tab.bottom - t.s(4) - bar_h,
                    right: tr.left + text_w + t.s(4),
                    bottom: tab.bottom - t.s(4),
                });
            }
            if show_close {
                let close_hot = d.hot == Hit::Close(*i);
                if close_hot {
                    theme::circle(dc, close, Some(theme::blend(p.ink, p.background, 0.1)), None);
                }
                let c = if close_hot { p.ink } else { p.muted };
                let cr = RECT { bottom: close.bottom, ..*close };
                theme::draw_text(dc, &glyph::CLOSE.to_string(), &cr, t.fonts.icon_small, c, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
        }
        // 底線：切換分頁時滑過去；只是視窗縮放就直接跟上。
        match bar_target {
            Some(target) => {
                let state = (model.selected, rect.width());
                let slide = matches!(d.bar_state, Some((sel, w)) if sel != state.0 || w == state.1);
                if slide {
                    anim::to(&mut d.bar_l, target.left as f32, anim::SLIDE_MS);
                    anim::to(&mut d.bar_r, target.right as f32, anim::SLIDE_MS);
                } else {
                    d.bar_l.snap(target.left as f32);
                    d.bar_r.snap(target.right as f32);
                }
                d.bar_state = Some(state);
                busy |= anim::running(&d.bar_l) || anim::running(&d.bar_r);
                let bar = RECT { left: anim::px(anim::value(&d.bar_l)), right: anim::px(anim::value(&d.bar_r)), ..target };
                let bar_color = if model.active { p.accent } else { theme::blend(p.muted, p.background, 0.6) };
                theme::round_rect(dc, &bar, bar.height(), Some(bar_color), None);
            }
            None => d.bar_state = None,
        }
        if busy {
            anim::request_frame(hwnd);
        }
        let draw_round = |r: &RECT, g: char, hot: bool, enabled: bool, filled: bool| {
            let size = t.s(26);
            let c = RECT::new(r.left + (r.width() - size) / 2, r.top + (r.height() - size) / 2 - t.s(3), size, size);
            let fill = if hot && enabled { theme::blend(p.ink, p.surface, 0.07) } else { p.surface };
            if filled || (hot && enabled) {
                theme::circle(dc, &c, Some(fill), Some(p.line));
            }
            let color = if enabled { p.ink } else { theme::blend(p.ink, p.background, 0.3) };
            theme::draw_text(dc, &g.to_string(), &c, t.fonts.icon_small, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        };
        if let Some(r) = l.prev {
            draw_round(&r, glyph::CHEVRON_LEFT, d.hot == Hit::Prev, l.can_prev, false);
        }
        if let Some(r) = l.next {
            draw_round(&r, glyph::CHEVRON_RIGHT, d.hot == Hit::Next, l.can_next, false);
        }
        draw_round(&l.plus, glyph::ADD, d.hot == Hit::Plus, true, true);
    }
    EndPaint(hwnd, &ps);
}

//! 麵包屑路徑（參考圖的「Lullo › Library › Albums」）。
//! 點上層名稱直接前往；點最後一段、空白處或「…」就切換成可輸入的路徑框（Ctrl+E、Alt+D 亦同）。
//! 每一段右邊的箭頭（最後一段也有）和名稱分開點：點箭頭列出那一層的子資料夾（見 crumbmenu.rs），
//! 選單開著時箭頭朝下；另一段的選單開著時，滑鼠移到這一段的箭頭就換成這一段的選單。

use super::anim;
use super::app;
use super::sys::*;
use super::theme::{self, glyph, Buffered};
use super::util::*;
use feather_core::anim::Tween;

pub const CRUMBS_CLASS: &str = "FeatherFiles.Crumbs";
pub const WM_APP_CRUMB: u32 = WM_APP + 12;
/// wParam：要前往的段落索引；EDIT 表示切換成輸入模式。
pub const EDIT: usize = usize::MAX >> 1;
/// 點了某一段右邊的箭頭：wParam 是段落索引；lParam 為 1 表示只是滑鼠移過去（別的選單開著時才換過去）
pub const WM_APP_CRUMB_MENU: u32 = WM_APP + 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Segment(usize),
    /// 段落右邊的箭頭（子資料夾選單）
    Arrow(usize),
    Ellipsis,
}

struct Data {
    hot: Hit,
    tracking: bool,
    /// 各段落與「…」的懸停底色（路徑改變時重設）
    hover: Vec<Tween>,
    arrow_hover: Vec<Tween>,
    ellipsis_hover: Tween,
    /// 滑鼠在空白處（或最後一段）時，右邊淡淡顯示「點一下或按 Ctrl+E 輸入路徑」
    hint: Tween,
    path: String,
    /// 拖放時滑鼠下的段落（放開就移動／複製到那一層）
    drop: Option<usize>,
}

struct Layout {
    /// （段落索引, 文字範圍）
    items: Vec<(usize, RECT)>,
    /// （段落索引, 箭頭範圍）：每一段右邊一個，最後一段也有
    arrows: Vec<(usize, RECT)>,
    /// 「…」右邊的分隔符號（不能點）
    separators: Vec<RECT>,
    ellipsis: Option<RECT>,
}

pub fn register(hinst: HINSTANCE) {
    register_class(hinst, CRUMBS_CLASS, CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, crumbs_proc, IDC_ARROW);
}

pub fn create(parent: HWND) -> HWND {
    let data = Box::new(Data {
        hot: Hit::None,
        tracking: false,
        hover: Vec::new(),
        arrow_hover: Vec::new(),
        ellipsis_hover: Tween::new(0.0),
        hint: Tween::new(0.0),
        path: String::new(),
        drop: None,
    });
    let class = wide(CRUMBS_CLASS);
    let name = wide(tr!("目前位置", "Current location"));
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

/// 懸停、拖放時的膠囊高度：容得下內容文字（文字大小放大時跟著加高）
fn pill_height(t: &theme::Theme) -> i32 {
    t.s(28).max(t.fonts.body_height + t.s(10))
}

fn layout(hwnd: HWND, segments: &[(String, String)], hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    // 箭頭與左右留白跟著文字大小放大
    let sep_w = t.s(20 * t.text_size as i32 / 100);
    let pad = t.s(6);
    let widths: Vec<i32> = segments
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            // 最後一段是目前資料夾：用較大的字，取代原本另一列的大標題
            let font = if i + 1 == segments.len() { t.fonts.body_heading } else { t.fonts.body };
            theme::text_width(hdc, label, font) + 2 * pad
        })
        .collect();
    // 從最後一段往前放，放不下的前段收成「…」。每一段後面都有箭頭（含最後一段）。
    let ellipsis_w = theme::text_width(hdc, "…", t.fonts.body) + 2 * pad + sep_w;
    let mut first = 0;
    let total = |from: usize| -> i32 { widths[from..].iter().sum::<i32>() + sep_w * (widths.len() - from) as i32 };
    while first + 1 < widths.len() && total(first) + if first > 0 { ellipsis_w } else { 0 } > r.width() {
        first += 1;
    }
    let mut out = Layout { items: Vec::new(), arrows: Vec::new(), separators: Vec::new(), ellipsis: None };
    let mut x = 0;
    if first > 0 {
        let w = ellipsis_w - sep_w;
        out.ellipsis = Some(RECT { left: x, top: 0, right: x + w, bottom: r.bottom });
        x += w;
        out.separators.push(RECT { left: x, top: 0, right: x + sep_w, bottom: r.bottom });
        x += sep_w;
    }
    for (i, w) in widths.iter().enumerate().skip(first) {
        // 只剩一段還放不下時名稱縮短（結尾「…」），箭頭仍留在看得到的地方
        let right = (x + w).min(r.right - sep_w).max(x);
        out.items.push((i, RECT { left: x, top: 0, right, bottom: r.bottom }));
        x = right;
        out.arrows.push((i, RECT { left: x, top: 0, right: x + sep_w, bottom: r.bottom }));
        x += sep_w;
    }
    out
}

fn hit_test(l: &Layout, x: i32, y: i32) -> Hit {
    if l.ellipsis.is_some_and(|r| r.contains(x, y)) {
        return Hit::Ellipsis;
    }
    for (i, r) in &l.arrows {
        if r.contains(x, y) {
            return Hit::Arrow(*i);
        }
    }
    for (i, r) in &l.items {
        if r.contains(x, y) {
            return Hit::Segment(*i);
        }
    }
    Hit::None
}

unsafe fn current_hit(hwnd: HWND, x: i32, y: i32) -> (Hit, usize) {
    let segments = app::crumb_model();
    let hdc = GetDC(hwnd);
    let l = layout(hwnd, &segments, hdc);
    ReleaseDC(hwnd, hdc);
    (hit_test(&l, x, y), segments.len())
}

/// 螢幕座標下是不是某一段的箭頭（點選單外面時，點的是箭頭就交給麵包屑切換選單，不先關掉）。
pub fn arrow_at(hwnd: HWND, screen: POINT) -> Option<usize> {
    let mut p = screen;
    unsafe {
        ScreenToClient(hwnd, &mut p);
        match current_hit(hwnd, p.x, p.y) {
            (Hit::Arrow(i), _) => Some(i),
            _ => None,
        }
    }
}

/// 某一段箭頭的螢幕範圍（畫出來的圓角底色那麼高），選單出現在它下方。這一段被收進「…」時回傳 None。
pub fn arrow_rect(hwnd: HWND, index: usize) -> Option<RECT> {
    unsafe {
        let segments = app::crumb_model();
        let hdc = GetDC(hwnd);
        let l = layout(hwnd, &segments, hdc);
        ReleaseDC(hwnd, hdc);
        let r = l.arrows.iter().find(|(i, _)| *i == index)?.1;
        let ph = pill_height(app::theme()).min(r.height());
        let mut tl = POINT { x: r.left, y: r.top + (r.height() - ph) / 2 };
        ClientToScreen(hwnd, &mut tl);
        Some(RECT { left: tl.x, top: tl.y, right: tl.x + r.width(), bottom: tl.y + ph })
    }
}

/// 拖放用：螢幕座標下的段落索引（含最後一段，也就是目前的資料夾）。
pub fn drop_hit(hwnd: HWND, screen: POINT) -> Option<usize> {
    let mut p = screen;
    unsafe {
        ScreenToClient(hwnd, &mut p);
        match current_hit(hwnd, p.x, p.y) {
            (Hit::Segment(i), _) => Some(i),
            _ => None,
        }
    }
}

/// 標示拖放中的段落；None 清除。
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

fn post(index: usize) {
    unsafe {
        PostMessageW(app::main_hwnd(), WM_APP_CRUMB, index, 0);
    }
}

fn post_menu(index: usize, hover: bool) {
    unsafe {
        PostMessageW(app::main_hwnd(), WM_APP_CRUMB_MENU, index, hover as LPARAM);
    }
}

unsafe extern "system" fn crumbs_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(d) = data(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    let (x, y) = (get_x_lparam(lparam), get_y_lparam(lparam));
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            paint(hwnd, d);
            return 0;
        }
        WM_SETCURSOR => {
            let mut p = cursor_pos();
            ScreenToClient(hwnd, &mut p);
            let (hit, n) = current_hit(hwnd, p.x, p.y);
            let cursor = match hit {
                Hit::Segment(i) if i + 1 < n => IDC_HAND,
                Hit::Arrow(_) => IDC_HAND,
                _ => IDC_IBEAM,
            };
            SetCursor(load_cursor(cursor));
            return 1;
        }
        WM_MOUSEMOVE => {
            if !d.tracking {
                track_mouse_leave(hwnd);
                d.tracking = true;
                // 進來時就在空白處：要畫出提示
                invalidate(hwnd);
            }
            let (hit, _) = current_hit(hwnd, x, y);
            if hit != d.hot {
                d.hot = hit;
                invalidate(hwnd);
                // 另一段的選單開著：移到這一段的箭頭就換成這一段的選單（同選單列）
                if let Hit::Arrow(i) = hit {
                    post_menu(i, true);
                }
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            d.tracking = false;
            d.hot = Hit::None;
            invalidate(hwnd);
            return 0;
        }
        // 箭頭按下就開（或關）選單；連點兩下時第二下是 WM_LBUTTONDBLCLK，一樣當作再點一次
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if let (Hit::Arrow(i), _) = current_hit(hwnd, x, y) {
                post_menu(i, false);
            }
            return 0;
        }
        // 中鍵點一段：在新分頁開啟那一層（最後一段是目前的資料夾，也可以）
        WM_MBUTTONUP => {
            if let (Hit::Segment(i), _) = current_hit(hwnd, x, y) {
                PostMessageW(app::main_hwnd(), WM_APP_CRUMB, i, 1);
            }
            return 0;
        }
        WM_LBUTTONUP => {
            match current_hit(hwnd, x, y) {
                (Hit::Segment(i), n) if i + 1 < n => post(i),
                (Hit::Arrow(_), _) => {}
                _ => post(EDIT),
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
        let segments = app::crumb_model();
        let l = layout(hwnd, &segments, dc);
        // 換了路徑：懸停狀態從頭開始
        let path = segments.last().map(|s| s.1.clone()).unwrap_or_default();
        if path != d.path || d.hover.len() != segments.len() {
            d.path = path;
            d.hover = vec![Tween::new(0.0); segments.len()];
            d.arrow_hover = vec![Tween::new(0.0); segments.len()];
        }
        let open = app::crumb_menu_segment();
        let mut busy = false;
        let hover_color = theme::blend(p.ink, p.background, 0.06);
        let pill = |r: &RECT, h: f32| {
            let ph = pill_height(t).min(r.height());
            let rr = RECT { top: r.top + (r.height() - ph) / 2, bottom: r.top + (r.height() - ph) / 2 + ph, ..*r };
            theme::round_rect(dc, &rr, ph, Some(anim::mix(p.background, hover_color, h)), None);
        };
        if let Some(r) = l.ellipsis {
            anim::to(&mut d.ellipsis_hover, (d.hot == Hit::Ellipsis) as u8 as f32, anim::HOVER_MS);
            busy |= anim::running(&d.ellipsis_hover);
            let h = anim::value(&d.ellipsis_hover);
            if h > 0.001 {
                pill(&r, h);
            }
            theme::draw_text(dc, "…", &r, t.fonts.body, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        for (i, r) in &l.items {
            let last = *i + 1 == segments.len();
            let hot = d.hot == Hit::Segment(*i) && !last;
            let h = match d.hover.get_mut(*i) {
                Some(tw) => {
                    anim::to(tw, hot as u8 as f32, anim::HOVER_MS);
                    busy |= anim::running(tw);
                    anim::value(tw)
                }
                None => 0.0,
            };
            let dropping = d.drop == Some(*i);
            if dropping {
                // 拖放目標：與清單的放置標示相同，淡重點色底加重點色外框
                let ph = (pill_height(t) + t.s(2)).min(r.height());
                let rr = RECT { left: r.left - t.s(2), top: r.top + (r.height() - ph) / 2, right: r.right + t.s(2), bottom: r.top + (r.height() - ph) / 2 + ph };
                theme::round_rect(dc, &rr, ph, Some(theme::blend(p.selected_tint, p.background, 0.7)), Some(p.accent));
            } else if h > 0.001 {
                pill(r, h);
            }
            let (font, color) = if last { (t.fonts.body_heading, p.ink) } else if dropping { (t.fonts.body, p.ink) } else { (t.fonts.body, anim::mix(p.muted, p.ink, h)) };
            theme::draw_text(dc, &segments[*i].0, r, font, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        for s in &l.separators {
            theme::draw_text(dc, &glyph::CHEVRON_SMALL.to_string(), s, t.fonts.body_icon_small, theme::blend(p.muted, p.background, 0.7), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        // 子資料夾選單的箭頭：懸停時有自己的圓角底色；選單開著時朝下、底色維持
        for (i, r) in &l.arrows {
            let is_open = open == Some(*i);
            let hot = d.hot == Hit::Arrow(*i) || is_open;
            let h = match d.arrow_hover.get_mut(*i) {
                Some(tw) => {
                    anim::to(tw, hot as u8 as f32, anim::HOVER_MS);
                    busy |= anim::running(tw);
                    anim::value(tw)
                }
                None => 0.0,
            };
            if h > 0.001 {
                pill(r, h);
            }
            let g = if is_open { glyph::CHEVRON_DOWN_SMALL } else { glyph::CHEVRON_SMALL };
            let color = anim::mix(theme::blend(p.muted, p.background, 0.7), p.ink, h);
            theme::draw_text(dc, &g.to_string(), r, t.fonts.body_icon_small, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        // 空白處與最後一段點了都是輸入路徑：滑鼠停在那裡時，右邊空著的地方淡淡寫出來（也告訴使用者 Ctrl+E）
        let editing_hit = match d.hot {
            Hit::None => d.drop.is_none(),
            Hit::Segment(i) => i + 1 == segments.len(),
            _ => false,
        };
        anim::to(&mut d.hint, (d.tracking && editing_hit) as u8 as f32, anim::HOVER_MS);
        busy |= anim::running(&d.hint);
        let hv = anim::value(&d.hint);
        if hv > 0.001 {
            let text = tr!("點一下或按 Ctrl+E 輸入路徑", "Click or press Ctrl+E to type a path");
            let used = l.arrows.last().map_or(0, |(_, r)| r.right);
            let tw = theme::text_width(dc, text, t.fonts.body_small);
            let hr = RECT { left: rect.right - t.s(10) - tw, right: rect.right - t.s(10), ..rect };
            if hr.left >= used + t.s(16) {
                let color = anim::mix(p.background, theme::blend(p.muted, p.background, 0.8), hv);
                theme::draw_text(dc, text, &hr, t.fonts.body_small, color, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
            }
        }
        if busy {
            anim::request_frame(hwnd);
        }
    }
    EndPaint(hwnd, &ps);
}

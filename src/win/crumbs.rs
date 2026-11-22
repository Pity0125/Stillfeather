//! 麵包屑路徑（參考圖的「Lullo › Library › Albums」）。
//! 點上層名稱直接前往；點最後一段、空白處或「…」就切換成可輸入的路徑框（Ctrl+L 亦同）。

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Segment(usize),
    Ellipsis,
}

struct Data {
    hot: Hit,
    tracking: bool,
    /// 各段落與「…」的懸停底色（路徑改變時重設）
    hover: Vec<Tween>,
    ellipsis_hover: Tween,
    path: String,
    /// 拖放時滑鼠下的段落（放開就移動／複製到那一層）
    drop: Option<usize>,
}

struct Layout {
    /// （段落索引, 文字範圍）
    items: Vec<(usize, RECT)>,
    separators: Vec<RECT>,
    ellipsis: Option<RECT>,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(CRUMBS_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: crumbs_proc,
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

pub fn create(parent: HWND) -> HWND {
    let data = Box::new(Data { hot: Hit::None, tracking: false, hover: Vec::new(), ellipsis_hover: Tween::new(0.0), path: String::new(), drop: None });
    let class = wide(CRUMBS_CLASS);
    let name = wide("目前位置");
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

fn layout(hwnd: HWND, segments: &[(String, String)], hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    let sep_w = t.s(20);
    let pad = t.s(6);
    let widths: Vec<i32> = segments
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let font = if i + 1 == segments.len() { t.fonts.ui_bold } else { t.fonts.ui };
            theme::text_width(hdc, label, font) + 2 * pad
        })
        .collect();
    // 從最後一段往前放，放不下的前段收成「…」。
    let ellipsis_w = theme::text_width(hdc, "…", t.fonts.ui) + 2 * pad + sep_w;
    let mut first = 0;
    let total = |from: usize| -> i32 { widths[from..].iter().sum::<i32>() + sep_w * (widths.len() - from).saturating_sub(1) as i32 };
    while first + 1 < widths.len() && total(first) + if first > 0 { ellipsis_w } else { 0 } > r.width() {
        first += 1;
    }
    let mut out = Layout { items: Vec::new(), separators: Vec::new(), ellipsis: None };
    let mut x = 0;
    if first > 0 {
        let w = ellipsis_w - sep_w;
        out.ellipsis = Some(RECT { left: x, top: 0, right: x + w, bottom: r.bottom });
        x += w;
        out.separators.push(RECT { left: x, top: 0, right: x + sep_w, bottom: r.bottom });
        x += sep_w;
    }
    for (i, w) in widths.iter().enumerate().skip(first) {
        let right = (x + w).min(r.right);
        out.items.push((i, RECT { left: x, top: 0, right, bottom: r.bottom }));
        x = right;
        if i + 1 < widths.len() {
            out.separators.push(RECT { left: x, top: 0, right: x + sep_w, bottom: r.bottom });
            x += sep_w;
        }
    }
    out
}

fn hit_test(l: &Layout, x: i32, y: i32) -> Hit {
    if l.ellipsis.is_some_and(|r| r.contains(x, y)) {
        return Hit::Ellipsis;
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
                _ => IDC_IBEAM,
            };
            SetCursor(load_cursor(cursor));
            return 1;
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
            let (hit, _) = current_hit(hwnd, x, y);
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
        WM_LBUTTONUP => {
            match current_hit(hwnd, x, y) {
                (Hit::Segment(i), n) if i + 1 < n => post(i),
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
        }
        let mut busy = false;
        let hover_color = theme::blend(p.ink, p.background, 0.06);
        let pill = |r: &RECT, h: f32| {
            let ph = t.s(28).min(r.height());
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
            theme::draw_text(dc, "…", &r, t.fonts.ui, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
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
                let ph = t.s(30).min(r.height());
                let rr = RECT { left: r.left - t.s(2), top: r.top + (r.height() - ph) / 2, right: r.right + t.s(2), bottom: r.top + (r.height() - ph) / 2 + ph };
                theme::round_rect(dc, &rr, ph, Some(theme::blend(p.selected_tint, p.background, 0.7)), Some(p.accent));
            } else if h > 0.001 {
                pill(r, h);
            }
            let (font, color) = if last { (t.fonts.ui_bold, p.ink) } else if dropping { (t.fonts.ui, p.ink) } else { (t.fonts.ui, anim::mix(p.muted, p.ink, h)) };
            theme::draw_text(dc, &segments[*i].0, r, font, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        for s in &l.separators {
            theme::draw_text(dc, &glyph::CHEVRON_SMALL.to_string(), s, t.fonts.icon_small, theme::blend(p.muted, p.background, 0.7), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        if busy {
            anim::request_frame(hwnd);
        }
    }
    EndPaint(hwnd, &ps);
}

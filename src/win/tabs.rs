//! 自繪分頁列：每欄一條，顯示資料夾名稱與關閉按鈕，右側有「＋」。
//! 分頁過多時縮小寬度，仍放不下就出現左右切換箭頭。
//!
//! 樣子同 Chrome：選取的分頁是和下面清單卡片同色的分頁形狀（上緣圓角、底下兩側往外彎），和卡片連成一片；
//! 主視窗畫卡片時把分頁底下那一段上框擦掉（[`active_span`]）。其他分頁沒有底色，滑鼠停著時才有淡淡的圓角底色。
//! 每個分頁前面有圖示（資料夾、搜尋結果），夠寬時每個分頁都有關閉按鈕，分頁之間不畫分隔線。
//! 操作一律以 PostMessage 交給主視窗處理，避免在繪圖程序中重入。
//!
//! 拖曳分頁調整順序：按住分頁、移動超過系統的拖曳距離（SM_CXDRAG／SM_CYDRAG）才開始，滑鼠被這條分頁列
//! 抓住（SetCapture）。目標位置畫一條重點色的直線；雙欄時可以拖到另一欄的分頁列（一欄的最後一個分頁不能拖走）。
//! 放開時把「從哪一欄第幾個、到哪一欄第幾個縫」交給主視窗（[`Action::Move`]）；按 Esc（[`cancel_drag`]）、
//! 失去滑鼠（WM_CAPTURECHANGED）或在分頁列外面放開就取消。

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
    /// 拖曳分頁放開：index 是被拖的分頁；lparam 的 16～19 位元是目標欄、0～15 位元是目標的縫
    Move = 4,
    Detach = 5,
}

impl Action {
    pub fn from_bits(v: usize) -> Option<Action> {
        match v {
            0 => Some(Action::Select),
            1 => Some(Action::Close),
            2 => Some(Action::New),
            3 => Some(Action::Menu),
            4 => Some(Action::Move),
            5 => Some(Action::Detach),
            _ => None,
        }
    }
}

/// 分頁列需要的資料（由 app 提供）。
pub struct Model {
    pub titles: Vec<String>,
    /// 標題前面的圖示（資料夾、搜尋結果）
    pub icons: Vec<char>,
    pub selected: usize,
    /// 選取的分頁的外框顏色：和下面清單卡片的外框相同（雙欄時作用中的一欄是重點色）
    pub outline: COLORREF,
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
    /// 上次畫的選取分頁的左右範圍（含喇叭口）：換了位置時請主視窗重畫卡片上框被擦掉的那一段
    span: Option<(i32, i32)>,
    /// 各分頁的懸停底色
    hover: Vec<Tween>,
    /// 拖放時滑鼠下的分頁
    drop: Option<usize>,
    /// 按住分頁準備拖曳（或拖曳中）
    drag: Option<Drag>,
    /// 拖曳分頁時，這條分頁列上的插入位置（直線的 x）
    insert: Option<i32>,
}

#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    start: POINT,
    /// 已經移動超過拖曳距離
    active: bool,
}

/// 拖曳中的分頁列（按 Esc 時取消）
static DRAGGING: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

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
        span: None,
        hover: Vec::new(),
        drop: None,
        drag: None,
        insert: None,
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

/// 分頁形狀的尺寸：上緣圓角的半徑、底下喇叭口的半徑、分頁頂端離分頁列上緣的距離
struct Shape {
    corner: i32,
    flare: i32,
    top: i32,
}

fn shape() -> Shape {
    let t = app::theme();
    Shape { corner: t.s(8), flare: t.s(7), top: t.s(5) }
}

fn layout(hwnd: HWND, d: &mut Data, model: &Model, hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    let top = shape().top;
    let bottom = r.bottom;
    // 第一個分頁的喇叭口要落在卡片上緣平直的地方（卡片的圓角半徑是 10），所以分頁從左邊縮進來一點
    let left = t.s(10) + shape().flare;
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
        .map(|title| (theme::text_width(hdc, title, t.fonts.body) + t.st(12) + icon_width() + t.s(6 + 18 + 8)).clamp(min_w, max_w))
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

/// 標題前面圖示佔的寬度（跟著文字大小）
fn icon_width() -> i32 {
    app::theme().st(22)
}

/// 選取的分頁（含兩側喇叭口）在分頁列上的左右範圍；看不到選取的分頁（捲到旁邊去）時是 None。
/// 主視窗畫清單卡片時把這一段的上框擦掉，分頁和卡片才連成一片。
pub fn active_span(hwnd: HWND) -> Option<(i32, i32)> {
    unsafe {
        let d = data(hwnd)?;
        with_layout(hwnd, d, |_, l, model| span_of(l, model))
    }
}

fn span_of(l: &Layout, model: &Model) -> Option<(i32, i32)> {
    let f = shape().flare;
    l.tabs.iter().find(|(i, _, _)| *i == model.selected).map(|(_, tab, _)| (tab.left - f, tab.right + f))
}

/// 請主視窗重畫分頁列正下方那一條（清單卡片的上框）
unsafe fn invalidate_card_edge(hwnd: HWND) {
    let parent = GetParent(hwnd);
    if parent == 0 {
        return;
    }
    let mut r = RECT::default();
    GetWindowRect(hwnd, &mut r);
    let mut tl = POINT { x: r.left, y: r.bottom };
    ScreenToClient(parent, &mut tl);
    let band = RECT { left: tl.x, top: tl.y - 1, right: tl.x + r.width(), bottom: tl.y + app::theme().s(3) };
    InvalidateRect(parent, &band, 0);
}

/// Chrome 那樣的分頁形狀：上緣兩個圓角，底下兩側往外彎成喇叭口，底邊開著（接到下面的卡片）。
/// 先以 `fill` 填滿，再沿著外形描 `outline`（底邊不描）。
unsafe fn tab_shape(dc: HDC, x0: i32, x1: i32, top: i32, bottom: i32, fill: COLORREF, outline: Option<COLORREF>) {
    let s = shape();
    let (rc, rf) = (s.corner as f32, s.flare as f32);
    // 四分之一圓的貝茲控制點比例
    const K: f32 = 0.5523;
    let p = |x: f32, y: f32| POINT { x: x.round() as i32, y: y.round() as i32 };
    let (x0f, x1f, tf, bf) = (x0 as f32, x1 as f32, top as f32, bottom as f32);
    let trace = |closed: bool| {
        BeginPath(dc);
        MoveToEx(dc, x0 - s.flare, bottom, std::ptr::null_mut());
        // 左下喇叭口：圓心在 (x0 - rf, bottom - rf)，從圓的底端彎到右端
        let left_flare = [p(x0f - rf + K * rf, bf), p(x0f, bf - rf + K * rf), p(x0f, bf - rf)];
        PolyBezierTo(dc, left_flare.as_ptr(), 3);
        LineTo(dc, x0, top + s.corner);
        let top_left = [p(x0f, tf + rc - K * rc), p(x0f + rc - K * rc, tf), p(x0f + rc, tf)];
        PolyBezierTo(dc, top_left.as_ptr(), 3);
        LineTo(dc, x1 - s.corner, top);
        let top_right = [p(x1f - rc + K * rc, tf), p(x1f, tf + rc - K * rc), p(x1f, tf + rc)];
        PolyBezierTo(dc, top_right.as_ptr(), 3);
        LineTo(dc, x1, bottom - s.flare);
        let right_flare = [p(x1f, bf - rf + K * rf), p(x1f + rf - K * rf, bf), p(x1f + rf, bf)];
        PolyBezierTo(dc, right_flare.as_ptr(), 3);
        if closed {
            // 底邊往下多一點，填滿到分頁列的最下面一列
            LineTo(dc, x1 + s.flare, bottom + 1);
            LineTo(dc, x0 - s.flare, bottom + 1);
            CloseFigure(dc);
        }
        EndPath(dc);
    };
    let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
    SetDCBrushColor(dc, fill);
    trace(true);
    FillPath(dc);
    if let Some(c) = outline {
        SelectObject(dc, GetStockObject(DC_PEN));
        SetDCPenColor(dc, c);
        trace(false);
        StrokePath(dc);
    }
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
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

/// 拖曳分頁中按了 Esc：取消拖曳。有在拖曳時回傳 true（按鍵不再做別的事）。
pub fn cancel_drag() -> bool {
    let hwnd = DRAGGING.load(std::sync::atomic::Ordering::Relaxed);
    if hwnd == 0 {
        return false;
    }
    unsafe {
        if let Some(d) = data(hwnd) {
            if d.drag.is_some_and(|g| g.active) {
                end_drag(hwnd, d);
                return true;
            }
        }
    }
    false
}

/// 設定分頁列 `strip` 的插入線。`strip` 就是正在處理訊息的這一條（`hwnd`）時直接改 `d`，不再取一次它的資料。
unsafe fn insert_on(hwnd: HWND, d: &mut Data, strip: HWND, x: Option<i32>) {
    if strip == hwnd {
        if d.insert != x {
            d.insert = x;
            invalidate(hwnd);
        }
    } else {
        set_insert(strip, x);
    }
}

/// 結束拖曳（放開、取消都走這裡）：清掉兩條分頁列上的插入線、放開滑鼠
unsafe fn end_drag(hwnd: HWND, d: &mut Data) {
    d.drag = None;
    DRAGGING.store(0, std::sync::atomic::Ordering::Relaxed);
    for side in [Side::Left, Side::Right] {
        let strip = app::tab_strip(side);
        if strip != 0 {
            insert_on(hwnd, d, strip, None);
        }
    }
    if GetCapture() == hwnd {
        ReleaseCapture();
    }
}

fn set_insert(hwnd: HWND, x: Option<i32>) {
    unsafe {
        if let Some(d) = data(hwnd) {
            if d.insert != x {
                d.insert = x;
                invalidate(hwnd);
            }
        }
    }
}

/// 拖曳到 `screen` 時的目標：哪一條分頁列、第幾個縫、插入線的 x。同一欄放回原位（拖了等於沒動）、
/// 拖走一欄的最後一個分頁、不在分頁列上時回傳 None。
unsafe fn detach_at(strip: HWND, screen: POINT) -> bool {
    let main = app::main_hwnd();
    let mut bounds = RECT::default();
    if GetWindowRect(main, &mut bounds) != 0 && !bounds.contains(screen.x, screen.y) { return true; }
    // On a maximized single monitor there may be no reachable point outside the window.
    IsZoomed(main) != 0 && GetWindowRect(strip, &mut bounds) != 0 && screen.y > bounds.bottom + app::theme().s(48)
}

unsafe fn drag_target(hwnd: HWND, own: &mut Data, index: usize, screen: POINT) -> Option<(Side, usize, HWND, i32)> {
    let from = own.side;
    for side in [Side::Left, Side::Right] {
        let strip = app::tab_strip(side);
        if strip == 0 || IsWindowVisible(strip) == 0 {
            continue;
        }
        let mut p = screen;
        ScreenToClient(strip, &mut p);
        let r = client_rect(strip);
        // 上下多留一點，拖的時候不必剛好在分頁列裡
        let slack = app::theme().s(16);
        if p.x < r.left || p.x >= r.right || p.y < r.top - slack || p.y >= r.bottom + slack {
            continue;
        }
        // 這一條就是正在處理訊息的分頁列：用手上的資料，不再取一次
        let d = if strip == hwnd { &mut *own } else { data(strip)? };
        let (gap, x, count) = with_layout(strip, d, |_, l, model| {
            let mut gap = None;
            let mut x = 0;
            for (i, tab, _) in &l.tabs {
                if p.x < (tab.left + tab.right) / 2 {
                    gap = Some(*i);
                    x = tab.left;
                    break;
                }
                x = tab.right;
            }
            let gap = gap.unwrap_or_else(|| l.tabs.last().map_or(0, |(i, _, _)| i + 1));
            (gap, x, model.titles.len())
        });
        if side == from {
            if gap == index || gap == index + 1 {
                return None;
            }
        } else if app::tab_model(from).titles.len() < 2 {
            // 那一欄只剩這個分頁：拖走就空了
            return None;
        }
        let _ = count;
        return Some((side, gap, strip, x));
    }
    None
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
            if let Some(mut g) = d.drag {
                let moved = (x - g.start.x).abs() > GetSystemMetrics(SM_CXDRAG) || (y - g.start.y).abs() > GetSystemMetrics(SM_CYDRAG);
                if !g.active && moved {
                    g.active = true;
                    DRAGGING.store(hwnd, std::sync::atomic::Ordering::Relaxed);
                }
                if g.active {
                    let mut screen = POINT { x, y };
                    ClientToScreen(hwnd, &mut screen);
                    let target = drag_target(hwnd, d, g.index, screen);
                    for side in [Side::Left, Side::Right] {
                        let strip = app::tab_strip(side);
                        if strip != 0 {
                            insert_on(hwnd, d, strip, target.filter(|t| t.2 == strip).map(|t| t.3));
                        }
                    }
                    let outside = detach_at(hwnd, screen);
                    SetCursor(load_cursor(if target.is_some() || outside { IDC_ARROW } else { IDC_NO }));
                }
                d.drag = Some(g);
                if g.active {
                    return 0;
                }
            }
            let hit = with_layout(hwnd, d, |_, l, _| hit_test(l, x, y));
            if hit != d.hot {
                d.hot = hit;
                invalidate(hwnd);
            }
            return 0;
        }
        WM_CAPTURECHANGED => {
            // 別的視窗拿走了滑鼠（例如跳出了對話方塊）：取消拖曳
            if d.drag.is_some() && lparam != hwnd {
                end_drag(hwnd, d);
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
                Hit::Tab(i) => {
                    post(d.side, Action::Select, i, 0);
                    // 可能是要拖曳調整順序：移動超過拖曳距離才開始
                    d.drag = Some(Drag { index: i, start: POINT { x, y }, active: false });
                    SetCapture(hwnd);
                }
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
            if let Some(g) = d.drag {
                end_drag(hwnd, d);
                if g.active {
                    let mut point = POINT { x, y };
                    ClientToScreen(hwnd, &mut point);
                    if detach_at(hwnd, point) {
                        post(d.side, Action::Detach, g.index, screen_lparam(hwnd, x, y));
                    } else if let Some((to, gap, _, _)) = drag_target(hwnd, d, g.index, point) {
                        post(d.side, Action::Move, g.index, ((to.index() << 16) | gap.min(0xFFFF)) as LPARAM);
                    }
                    d.pressed = Hit::None;
                    return 0;
                }
            }
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

/// Chrome 樣式的分頁列：選取的分頁是和清單卡片連成一片的分頁形狀，其他分頁只有圖示與標題（滑鼠停著時有淡底色）。
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
        let sh = shape();
        let bottom = rect.bottom;
        // 先畫沒選取的分頁的底色：沒有底色，滑鼠停著時淡淡的圓角底色，拖放目標加上重點色外框
        let mut hover_of = vec![0.0f32; model.titles.len()];
        for (i, tab, _) in &l.tabs {
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
            if let Some(v) = hover_of.get_mut(*i) {
                *v = hv;
            }
            let pill = RECT { left: tab.left + t.s(3), top: sh.top + t.s(1), right: tab.right - t.s(3), bottom: bottom - t.s(4) };
            if d.drop == Some(*i) {
                theme::round_rect(dc, &pill, t.s(14), Some(theme::blend(p.selected_tint, p.background, 0.7)), Some(p.accent));
            } else if hv > 0.001 {
                theme::round_rect(dc, &pill, t.s(14), Some(anim::mix(p.background, p.chip, hv)), None);
            }
        }
        // 選取的分頁：和下面清單卡片同色的分頁形狀（喇叭口伸進隔壁分頁的邊上，蓋過隔壁的懸停底色）
        if let Some((_, tab, _)) = l.tabs.iter().find(|(i, _, _)| *i == model.selected) {
            tab_shape(dc, tab.left, tab.right, sh.top, bottom, p.surface, Some(model.outline));
        }
        // 選取的分頁換了位置（換分頁、捲動、視窗縮放）：主視窗重畫卡片上框，擦掉的那一段跟著移動
        let span = span_of(&l, &model);
        if span != d.span {
            d.span = span;
            invalidate_card_edge(hwnd);
        }
        // 圖示、標題與關閉按鈕
        for (i, tab, close) in &l.tabs {
            let selected = *i == model.selected;
            let hot = matches!(d.hot, Hit::Tab(h) | Hit::Close(h) if h == *i);
            let hv = hover_of.get(*i).copied().unwrap_or(0.0);
            let dropping = d.drop == Some(*i);
            let title = model.titles.get(*i).map(String::as_str).unwrap_or("");
            // 夠寬時每個分頁都有關閉按鈕（同 Chrome）；窄的只有選取或滑鼠停著的才有
            let show_close = selected || hot || tab.width() >= t.st(120);
            let content = RECT { top: sh.top, bottom, ..*tab };
            let ink = if selected || dropping { p.ink } else { anim::mix(p.muted, p.ink, hv) };
            let ir = RECT { left: tab.left + t.st(12), right: tab.left + t.st(12) + icon_width(), ..content };
            let icon = model.icons.get(*i).copied().unwrap_or(glyph::FOLDER);
            theme::draw_text(dc, &icon.to_string(), &ir, t.fonts.body_icon_small, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            let text_right = if show_close { close.left - t.s(2) } else { tab.right - t.s(10) };
            let tr = RECT { left: ir.right, right: text_right, ..content };
            theme::draw_text(dc, title, &tr, t.fonts.body, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            if show_close {
                let close_hot = d.hot == Hit::Close(*i);
                if close_hot {
                    let under = if selected { p.surface } else { anim::mix(p.background, p.chip, hv) };
                    theme::circle(dc, close, Some(theme::blend(p.ink, under, 0.1)), None);
                }
                let c = if close_hot { p.ink } else { p.muted };
                theme::draw_text(dc, &glyph::CLOSE.to_string(), close, t.fonts.icon_small, c, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
        }
        // 拖曳分頁時的插入位置：重點色直線
        if let Some(x) = d.insert {
            let w = t.s(3);
            let line = RECT { left: x - w / 2, top: t.s(5), right: x - w / 2 + w, bottom: rect.bottom - t.s(7) };
            theme::round_rect(dc, &line, w, Some(p.accent), None);
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

//! 詳細資料檢視的空白處與框選，同檔案總管。
//!
//! 一列的「項目本身」只到圖示左邊一點與最後一欄的右緣（選取、懸停的底色也只畫在這裡）：
//! 按在上面照舊交給清單——點選、整列都可以拖放檔案、雙擊開啟。
//! 項目以外是空白處：圖示左邊的留白、欄位右側沒有資訊的地方、最後一列下面。
//! 在空白處按住拖曳拉出方框，選取碰到的項目（Ctrl 反轉、Shift 加入原本的選取，拖到上下緣外自動捲動）；
//! 只點一下是取消選取；按右鍵是資料夾空白處的選單。
//!
//! 清單是整列選取（LVS_EX_FULLROWSELECT），對清單來說圖示左邊的留白也算那一列，內建的框選又只能從最後一列下面開始，
//! 所以空白處由這裡接手。其他檢視（圖示、並排、清單…）項目之間本來就有空白，用清單內建的框選。
//! 計算的部分見 `feather_core::marquee`。
//!
//! 拖曳時要跟得上滑鼠：方框是兩個疊在清單上、不接收滑鼠的半透明小視窗（填色一個、外框一個），移動時只搬視窗，
//! 清單不必重畫；碰到的列直接換成選取（不播淡入動畫）；狀態列的「已選取 N 個項目」最多每 0.1 秒更新一次
//! （更新狀態列會重畫整個主視窗）。

use super::app::App;
use super::sys::*;
use super::theme;
use super::util::*;
use feather_core::marquee::{self, Combine};
use feather_core::view::ViewMode;
use feather_core::workspace::Side;
use std::cell::Cell;
use std::time::{Duration, Instant};

/// 詳細資料檢視的項目往右縮排幾格（一格是小圖示的寬度），圖示不貼著清單左緣，左邊留出空白處
pub(super) const INDENT: i32 = 1;
/// 選取、懸停的底色從圖示左邊多少（邏輯像素）開始
const ICON_PAD: i32 = 6;
/// 拖到清單外時自動捲動的計時器（設在清單視窗上，不和清單自己的計時器撞號）
pub(super) const SCROLL_TIMER: usize = 0x5346_4D51;
const SCROLL_MS: u32 = 40;
/// 方框填色的不透明度
const FILL_ALPHA: u8 = 0x38;
/// 框選中狀態列最多多久更新一次
const SYNC_EVERY: Duration = Duration::from_millis(100);
/// 方框外框視窗的透明色（外框以外的部分畫成這個顏色，看起來就是透明的）
const FRAME_KEY: COLORREF = 0x00FF_00FF;
const OVERLAY_CLASS: &str = "Stillfeather.Marquee";

thread_local! {
    /// 方框視窗的類別已經登記
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
    /// 在空白處按下右鍵的那一欄：放開時顯示資料夾空白處的選單
    static BLANK_RCLICK: Cell<Option<usize>> = const { Cell::new(None) };
    /// 滑鼠上次在哪一欄、哪一列，在項目上還是旁邊的空白（換了才重畫懸停的底色）
    static POINTER: Cell<Option<(usize, usize, bool)>> = const { Cell::new(None) };
}

pub(super) struct Marquee {
    side: Side,
    /// 按下的位置（清單用戶區座標）
    start: POINT,
    /// 按下的位置（內容座標：以第一列左上角為原點，捲動時不變）
    anchor: (i32, i32),
    ctrl: bool,
    shift: bool,
    /// 已經開始拖曳（移動超過系統的拖曳距離）
    active: bool,
    combine: Combine,
    /// 開始拖曳前選取的列，由小到大（Combine::Replace 時用不到，是空的）
    before: Vec<usize>,
    /// 方框目前碰到的列
    range: Option<(usize, usize)>,
    /// 畫面上的方框（用戶區座標，只在欄位標題下面）
    drawn: Option<RECT>,
    /// 方框的填色與外框視窗（開始拖曳時建立）
    fill: HWND,
    frame: HWND,
    /// 選取改了、狀態列還沒跟上；上次更新狀態列的時間
    dirty: bool,
    synced: Instant,
}

/// 詳細資料檢視的列：第一列（用戶區座標，捲動後可能是負的）、列高、列數，以及項目本身的左右邊（用戶區座標）
struct Rows {
    origin: RECT,
    row_h: i32,
    count: usize,
    left: i32,
    right: i32,
}

impl Rows {
    fn content(&self, pt: POINT) -> (i32, i32) {
        (pt.x - self.origin.left, pt.y - self.origin.top)
    }

    /// 項目本身的左右邊（內容座標）
    fn span(&self) -> (i32, i32) {
        (self.left - self.origin.left, self.right - self.origin.left)
    }

    /// 這個高度是第幾列（最後一列下面、第一列上面沒有）
    fn row_at_y(&self, y: i32) -> Option<usize> {
        let y = y - self.origin.top;
        if y < 0 {
            return None;
        }
        let i = (y / self.row_h) as usize;
        (i < self.count).then_some(i)
    }

    /// 這個位置在哪一列的項目上（項目兩側的空白、最後一列下面不算）
    fn item_at(&self, pt: POINT) -> Option<usize> {
        if pt.x < self.left || pt.x >= self.right {
            return None;
        }
        self.row_at_y(pt.y)
    }
}

/// 清單放列的地方（用戶區座標）：欄位標題下面
fn rows_area(list: HWND) -> RECT {
    let mut area = client_rect(list);
    let header = send(list, LVM_GETHEADER, 0, 0) as HWND;
    if header != 0 {
        area.top = area.top.max(client_rect(header).bottom);
    }
    area
}

fn intersect(a: RECT, b: RECT) -> Option<RECT> {
    let r = RECT { left: a.left.max(b.left), top: a.top.max(b.top), right: a.right.min(b.right), bottom: a.bottom.min(b.bottom) };
    (r.left < r.right && r.top < r.bottom).then_some(r)
}

fn set_state(list: HWND, index: isize, state: u32, mask: u32) {
    let mut item: LVITEMW = unsafe { std::mem::zeroed() };
    item.stateMask = mask;
    item.state = state;
    send(list, LVM_SETITEMSTATE, index as usize, &mut item as *mut LVITEMW as LPARAM);
}

fn selected_rows(list: HWND) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = -1isize;
    loop {
        i = send(list, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as LPARAM) as isize;
        if i < 0 {
            break;
        }
        out.push(i as usize);
    }
    out
}

fn item_rect(list: HWND, index: usize, part: i32) -> Option<RECT> {
    let mut r = RECT { left: part, ..Default::default() };
    (send(list, LVM_GETITEMRECT, index, &mut r as *mut RECT as LPARAM) != 0).then_some(r)
}

fn redraw(list: HWND, r: Option<RECT>) {
    if let Some(r) = r {
        let r = r.inflate(1, 1);
        unsafe {
            InvalidateRect(list, &r, 0);
        }
    }
}

fn cursor_in(hwnd: HWND) -> POINT {
    let mut pt = POINT::default();
    unsafe {
        GetCursorPos(&mut pt);
        ScreenToClient(hwnd, &mut pt);
    }
    pt
}

/// 方框視窗：填色（外框寬度 0，整個視窗半透明）或外框（中間是透明色）。視窗的 GWLP_USERDATA 是顏色，
/// 外框寬度放在高 32 位元。不接收滑鼠、不會被啟用。
unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as u64;
            let color = (data & 0xFFFF_FFFF) as COLORREF;
            let w = (data >> 32) as i32;
            let r = client_rect(hwnd);
            if w == 0 {
                theme::fill(hdc, &r, color);
            } else {
                theme::fill(hdc, &r, FRAME_KEY);
                for edge in [
                    RECT { bottom: r.top + w, ..r },
                    RECT { top: r.bottom - w, ..r },
                    RECT { right: r.left + w, ..r },
                    RECT { left: r.right - w, ..r },
                ] {
                    theme::fill(hdc, &edge, color);
                }
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_NCHITTEST => return HTTRANSPARENT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE,
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn create_overlay(owner: HWND, color: COLORREF, border: i32) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = wide(OVERLAY_CLASS);
        if !REGISTERED.with(|r| r.replace(true)) {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: overlay_proc,
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
            RegisterClassExW(&wc);
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            owner,
            0,
            hinst,
            std::ptr::null(),
        );
        if hwnd == 0 {
            return 0;
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (color as u64 | (border.max(0) as u64) << 32) as isize);
        let off: BOOL = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const BOOL as *const std::ffi::c_void, 4);
        if border == 0 {
            SetLayeredWindowAttributes(hwnd, 0, FILL_ALPHA, LWA_ALPHA);
        } else {
            SetLayeredWindowAttributes(hwnd, FRAME_KEY, 0, LWA_COLORKEY);
        }
        hwnd
    }
}

/// 把方框視窗搬到清單上的 `r`（用戶區座標）；沒有時藏起來
fn place_overlays(list: HWND, wins: [HWND; 2], r: Option<RECT>) {
    unsafe {
        match r {
            None => {
                for h in wins {
                    if h != 0 {
                        ShowWindow(h, SW_HIDE);
                    }
                }
            }
            Some(r) => {
                let mut pt = POINT { x: r.left, y: r.top };
                ClientToScreen(list, &mut pt);
                for h in wins {
                    if h != 0 {
                        SetWindowPos(h, 0, pt.x, pt.y, r.width(), r.height(), SWP_NOACTIVATE | SWP_NOZORDER | SWP_SHOWWINDOW);
                        UpdateWindow(h);
                    }
                }
            }
        }
    }
}

impl App {
    fn details(&self, side: Side) -> bool {
        self.view[side.index()] == ViewMode::Details
    }

    /// 選取、懸停底色的範圍（用戶區座標）。詳細資料檢視只到項目本身（圖示左邊一點到最後一欄的右緣），
    /// 其他檢視是整個項目。
    pub(super) fn item_capsule(&self, side: Side, index: usize) -> Option<RECT> {
        let list = self.lists[side.index()];
        let b = item_rect(list, index, LVIR_BOUNDS)?;
        if self.details(side) {
            let left = item_rect(list, index, LVIR_ICON).map_or(b.left, |icon| icon.left - self.s(ICON_PAD));
            Some(RECT { left, top: b.top + 1, right: b.right - self.s(2), bottom: b.bottom - 1 })
        } else {
            Some(RECT { left: b.left.max(0) + self.s(2), top: b.top + 1, right: b.right - self.s(2), bottom: b.bottom - 1 })
        }
    }

    fn rows(&self, side: Side) -> Option<Rows> {
        let list = self.lists[side.index()];
        let count = send(list, LVM_GETITEMCOUNT, 0, 0);
        if count <= 0 {
            return None;
        }
        let origin = item_rect(list, 0, LVIR_BOUNDS).filter(|r| r.height() > 0)?;
        let left = item_rect(list, 0, LVIR_ICON).map_or(origin.left, |icon| icon.left - self.s(ICON_PAD));
        Some(Rows { origin, row_h: origin.height(), count: count as usize, left, right: origin.right })
    }

    /// 詳細資料檢視中，這個位置（清單用戶區座標）不在任何項目上：旁邊的空白或最後一列下面。
    /// 這些地方不顯示懸停的底色、不預覽。其他檢視一律 false（由清單自己判斷）。
    pub(super) fn outside_items(&self, side: Side, pt: POINT) -> bool {
        self.details(side) && self.rows(side).is_some_and(|rows| rows.item_at(pt).is_none())
    }

    /// 滑鼠目前在清單的空白處（畫懸停的底色前檢查）
    pub(super) fn pointer_outside_items(&self, side: Side) -> bool {
        self.outside_items(side, cursor_in(self.lists[side.index()]))
    }

    /// 滑鼠在同一列的項目與旁邊的空白之間移動時，對清單來說還是同一列，不會重畫：自己重畫那一列的懸停底色
    pub(super) fn track_pointer(&self, side: Side, pt: POINT) {
        let si = side.index();
        let now = if self.details(side) {
            self.rows(side).and_then(|rows| rows.row_at_y(pt.y).map(|row| (si, row, rows.item_at(pt).is_some())))
        } else {
            None
        };
        let before = POINTER.with(|p| p.replace(now));
        if let (Some((s0, r0, in0)), Some((s1, r1, in1))) = (before, now) {
            if s0 == s1 && r0 == r1 && in0 != in1 {
                redraw(self.lists[si], item_rect(self.lists[si], r1, LVIR_BOUNDS));
            }
        }
    }

    /// 這次按下是不是由空白處接手：詳細資料檢視、可以多選、有項目，而且沒按在項目上
    fn blank_press(&self, side: Side, pt: POINT) -> Option<Rows> {
        let list = self.lists[side.index()];
        if !self.details(side) || self.marquee.is_some() {
            return None;
        }
        // 只能選一個的清單（選擇器開單一檔案時）照舊交給清單
        if unsafe { GetWindowLongPtrW(list, GWL_STYLE) } as u32 & LVS_SINGLESEL != 0 {
            return None;
        }
        let rows = self.rows(side)?;
        rows.item_at(pt).is_none().then_some(rows)
    }

    /// 清單按下左鍵（或雙擊的第二下）：按在空白處時由這裡接手（回傳 true，清單不再處理這次按下）。
    pub(super) fn marquee_press(&mut self, side: Side, wparam: WPARAM, lparam: LPARAM) -> bool {
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        let Some(rows) = self.blank_press(side, pt) else { return false };
        let list = self.lists[side.index()];
        unsafe {
            SetFocus(list);
            SetCapture(list);
        }
        let keys = wparam as u32;
        self.marquee = Some(Marquee {
            side,
            start: pt,
            anchor: rows.content(pt),
            ctrl: keys & MK_CONTROL != 0,
            shift: keys & MK_SHIFT != 0,
            active: false,
            combine: Combine::Replace,
            before: Vec::new(),
            range: None,
            drawn: None,
            fill: 0,
            frame: 0,
            dirty: false,
            synced: Instant::now(),
        });
        true
    }

    /// 清單的滑鼠移動：按下後由框選接手時回傳 true
    pub(super) fn marquee_move(&mut self, side: Side, lparam: LPARAM) -> bool {
        let Some(m) = self.marquee.as_ref().filter(|m| m.side == side) else { return false };
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        if !m.active {
            let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if (pt.x - m.start.x).abs() <= dx && (pt.y - m.start.y).abs() <= dy {
                return true;
            }
            self.marquee_begin(side);
        }
        self.marquee_update(side, pt);
        true
    }

    /// 移動超過拖曳距離：開始框選
    fn marquee_begin(&mut self, side: Side) {
        let list = self.lists[side.index()];
        let (owner, accent, border) = (self.hwnd, self.theme.pal.accent, self.s(1).max(1));
        let Some(m) = self.marquee.as_mut() else { return };
        m.active = true;
        // 先建立填色再建立外框：外框在上面
        m.fill = create_overlay(owner, accent, 0);
        m.frame = create_overlay(owner, accent, border);
        m.combine = Combine::from_keys(m.ctrl, m.shift);
        if m.combine == Combine::Replace {
            self.clear_selection(side);
        } else {
            m.before = selected_rows(list);
        }
        unsafe {
            SetTimer(list, SCROLL_TIMER, SCROLL_MS, std::ptr::null());
        }
    }

    fn clear_selection(&mut self, side: Side) {
        let was = std::mem::replace(&mut self.updating_list, true);
        set_state(self.lists[side.index()], -1, 0, LVIS_SELECTED);
        self.updating_list = was;
        self.selection_changed(side);
    }

    /// 方框拉到 `pt`（清單用戶區座標）：更新選取與畫面上的方框
    fn marquee_update(&mut self, side: Side, pt: POINT) {
        let list = self.lists[side.index()];
        let Some(rows) = self.rows(side) else { return };
        let Some(m) = self.marquee.as_ref() else { return };
        let rect = marquee::span(m.anchor, rows.content(pt));
        let range = marquee::rows_touched(rect, rows.row_h, rows.span(), rows.count);
        if range != m.range {
            let inside = |i: usize| range.is_some_and(|(a, b)| a <= i && i <= b);
            let updates: Vec<(usize, bool)> = marquee::changed_rows(m.range, range).into_iter().map(|i| (i, m.combine.wanted(m.before.binary_search(&i).is_ok(), inside(i)))).collect();
            let was = std::mem::replace(&mut self.updating_list, true);
            let fx = &mut self.list_fx[side.index()];
            for (i, on) in updates {
                // 直接換成新的樣子，不播淡入淡出（框選要跟得上滑鼠）
                fx.forget(i);
                set_state(list, i as isize, if on { LVIS_SELECTED } else { 0 }, LVIS_SELECTED);
            }
            self.updating_list = was;
            if let Some(m) = self.marquee.as_mut() {
                m.range = range;
                m.dirty = true;
            }
            self.marquee_sync(side, false);
        }
        let o = rows.origin;
        let client = RECT { left: rect.0 + o.left, top: rect.1 + o.top, right: rect.2 + o.left + 1, bottom: rect.3 + o.top + 1 };
        let shown = intersect(client, rows_area(list));
        let Some(m) = self.marquee.as_mut() else { return };
        if m.drawn != shown {
            m.drawn = shown;
            place_overlays(list, [m.fill, m.frame], shown);
        }
    }

    /// 選取改了：更新狀態列等（會重畫整個主視窗，所以框選中最多每 0.1 秒一次；`now` 時馬上）
    fn marquee_sync(&mut self, side: Side, now: bool) {
        let Some(m) = self.marquee.as_mut().filter(|m| m.side == side && m.dirty) else { return };
        if !now && m.synced.elapsed() < SYNC_EVERY {
            return;
        }
        m.dirty = false;
        m.synced = Instant::now();
        self.selection_changed(side);
    }

    /// 框選中拖到清單外：每隔一下往那個方向捲動（離邊緣越遠越快）
    pub(super) fn marquee_tick(&mut self, side: Side) {
        let list = self.lists[side.index()];
        if !self.marquee.as_ref().is_some_and(|m| m.side == side && m.active) {
            return;
        }
        self.marquee_sync(side, false);
        let Some(rows) = self.rows(side) else { return };
        let pt = cursor_in(list);
        let area = rows_area(list);
        let dy = if pt.y < area.top {
            -marquee::scroll_step(area.top - pt.y, rows.row_h)
        } else if pt.y >= area.bottom {
            marquee::scroll_step(pt.y - area.bottom + 1, rows.row_h)
        } else {
            0
        };
        let dx = if pt.x < area.left {
            -marquee::scroll_step(area.left - pt.x, self.s(24))
        } else if pt.x >= area.right {
            marquee::scroll_step(pt.x - area.right + 1, self.s(24))
        } else {
            0
        };
        if dx == 0 && dy == 0 {
            return;
        }
        send(list, LVM_SCROLL, dx as isize as usize, dy as LPARAM);
        let moved = self.rows(side).is_some_and(|r| r.origin.left != rows.origin.left || r.origin.top != rows.origin.top);
        if moved {
            self.marquee_update(side, pt);
        }
    }

    /// 放開左鍵：結束框選；沒有拖曳就是在空白處點一下，取消選取（按著 Ctrl、Shift 時保留）。由這裡處理時回傳 true
    pub(super) fn marquee_release(&mut self, side: Side) -> bool {
        self.marquee_sync(side, true);
        let Some(m) = self.marquee.take_if(|m| m.side == side) else { return false };
        self.marquee_finish(side, &m);
        let list = self.lists[side.index()];
        unsafe {
            if GetCapture() == list {
                ReleaseCapture();
            }
        }
        if !m.active && !m.ctrl && !m.shift {
            self.clear_selection(side);
        }
        true
    }

    /// 滑鼠被搶走（切換到別的視窗等）：停在目前的選取
    pub(super) fn marquee_lost(&mut self, side: Side) {
        self.marquee_sync(side, true);
        if let Some(m) = self.marquee.take_if(|m| m.side == side) {
            self.marquee_finish(side, &m);
        }
    }

    /// 清單內容換了（換資料夾、重新整理、換檢視）：取消框選，列號已經對不上
    pub(super) fn cancel_marquee(&mut self, side: Side) {
        if self.marquee.as_ref().is_some_and(|m| m.side == side) {
            self.marquee_lost(side);
            let list = self.lists[side.index()];
            unsafe {
                if GetCapture() == list {
                    ReleaseCapture();
                }
            }
        }
    }

    fn marquee_finish(&mut self, side: Side, m: &Marquee) {
        let list = self.lists[side.index()];
        unsafe {
            KillTimer(list, SCROLL_TIMER);
            for h in [m.frame, m.fill] {
                if h != 0 {
                    DestroyWindow(h);
                }
            }
        }
    }

    /// 在空白處按下右鍵：取消選取（按著 Ctrl 時保留），放開時顯示資料夾空白處的選單。由這裡處理時回傳 true
    pub(super) fn blank_rpress(&mut self, side: Side, wparam: WPARAM, lparam: LPARAM) -> bool {
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        if self.blank_press(side, pt).is_none() {
            return false;
        }
        unsafe {
            SetFocus(self.lists[side.index()]);
        }
        if wparam as u32 & MK_CONTROL == 0 {
            self.clear_selection(side);
        }
        BLANK_RCLICK.with(|c| c.set(Some(side.index())));
        true
    }

    pub(super) fn blank_rrelease(&mut self, side: Side, lparam: LPARAM) -> bool {
        if BLANK_RCLICK.with(|c| c.get()) != Some(side.index()) {
            return false;
        }
        BLANK_RCLICK.with(|c| c.set(None));
        let mut pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        unsafe {
            ClientToScreen(self.lists[side.index()], &mut pt);
        }
        self.list_context_menu(side, pt, false);
        true
    }
}

//! 篩選籤列（參考圖的「All 15 · Pop 3 · Lo-fi 5…」）：依類型篩選目前清單
//! （全部／資料夾／圖片／文件／影音／壓縮檔／其他，附數量與色點），和動作按鈕放在同一列（見 App::relayout）。
//! 搜尋時右側多一個「搜尋含子資料夾」。排序原本也在這裡（A–Z／最新／大小），和「排序」選單重複，已併入選單。
//! 操作以 PostMessage 交給主視窗處理。

use super::anim;
use super::app;
use super::sys::*;
use super::theme::{self, glyph, Buffered};
use super::util::*;
use feather_core::anim::Tween;
use feather_core::category::{self, Category};

pub const CHIPS_CLASS: &str = "FeatherFiles.Chips";
pub const WM_APP_CHIP: u32 = WM_APP + 11;

pub const ACTION_FILTER: usize = 0;
pub const ACTION_RECURSIVE: usize = 2;

pub struct Model {
    pub counts: [usize; category::COUNT],
    pub filter: Category,
    pub recursive: bool,
    /// 顯示「搜尋含子資料夾」：搜尋框展開或正在顯示搜尋結果時
    pub show_recursive: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Filter(Category),
    Recursive,
}

struct Data {
    hot: Hit,
    tracking: bool,
    /// 第一次繪製時直接套用目前狀態，不播放動畫。
    ready: bool,
    /// 各類型篩選籤的「作用中」與懸停程度
    active: [Tween; category::COUNT],
    hover: [Tween; category::COUNT],
    rec_check: Tween,
    rec_hover: Tween,
}

struct Layout {
    filters: Vec<(Category, RECT)>,
    /// 「搜尋含子資料夾」（沒顯示時是 None）
    recursive: Option<RECT>,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(CHIPS_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: chips_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: 0,
        hCursor: load_cursor(IDC_HAND),
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
    let data = Box::new(Data {
        hot: Hit::None,
        tracking: false,
        ready: false,
        active: [Tween::new(0.0); category::COUNT],
        hover: [Tween::new(0.0); category::COUNT],
        rec_check: Tween::new(0.0),
        rec_hover: Tween::new(0.0),
    });
    let class = wide(CHIPS_CLASS);
    let name = wide(tr!("類型篩選", "Type filters"));
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

fn chip_text(c: Category, count: usize) -> (String, String) {
    (c.label().to_string(), feather_core::format::group_digits(count as u64))
}

/// 要顯示的篩選籤與各自的寬度
fn chip_widths(model: &Model, hdc: HDC) -> Vec<(Category, i32)> {
    let t = app::theme();
    category::ALL
        .into_iter()
        .filter(|c| *c == Category::All || model.counts[c.index()] > 0 || model.filter == *c)
        .map(|c| {
            let (label, num) = chip_text(c, model.counts[c.index()]);
            let dot = if c == Category::All { 0 } else { t.s(14) };
            (c, t.s(14) + dot + theme::text_width(hdc, &label, t.fonts.ui) + t.s(7) + theme::text_width(hdc, &num, t.fonts.small_bold) + t.s(14))
        })
        .collect()
}

fn recursive_width(hdc: HDC) -> i32 {
    let t = app::theme();
    theme::text_width(hdc, tr!("搜尋含子資料夾", "Include subfolders"), t.fonts.ui) + t.s(44)
}

/// 全部篩選籤（與搜尋時的「搜尋含子資料夾」）不必省略時需要的寬度：主視窗用來決定同一列的動作按鈕要不要只顯示圖示
pub fn natural_width(hwnd: HWND, model: &Model) -> i32 {
    let t = app::theme();
    unsafe {
        let hdc = GetDC(hwnd);
        let chips: i32 = chip_widths(model, hdc).iter().map(|(_, w)| w + t.s(8)).sum();
        let rec = if model.show_recursive { recursive_width(hdc) + t.s(12) } else { 0 };
        ReleaseDC(hwnd, hdc);
        chips + rec
    }
}

fn layout(hwnd: HWND, model: &Model, hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    let h = t.s(32).min(r.height());
    let y = (r.height() - h) / 2;
    // 搜尋時右側是「搜尋含子資料夾」
    let recursive = model.show_recursive.then(|| {
        let w = recursive_width(hdc);
        RECT { left: r.right - w, top: y, right: r.right, bottom: y + h }
    });
    // 左側：類型篩選籤，放不下的省略
    let mut filters = Vec::new();
    let mut x = 0;
    let limit = recursive.map_or(r.right, |rr| rr.left - t.s(12));
    for (c, w) in chip_widths(model, hdc) {
        if x + w > limit {
            break;
        }
        filters.push((c, RECT { left: x, top: y, right: x + w, bottom: y + h }));
        x += w + t.s(8);
    }
    Layout { filters, recursive }
}

fn hit_test(l: &Layout, x: i32, y: i32) -> Hit {
    for (c, r) in &l.filters {
        if r.contains(x, y) {
            return Hit::Filter(*c);
        }
    }
    if l.recursive.is_some_and(|r| r.contains(x, y)) {
        return Hit::Recursive;
    }
    Hit::None
}

fn post(action: usize, value: usize) {
    unsafe {
        PostMessageW(app::main_hwnd(), WM_APP_CHIP, action | (value << 8), 0);
    }
}

unsafe fn current_hit(hwnd: HWND, x: i32, y: i32) -> Hit {
    let model = app::chip_model();
    let hdc = GetDC(hwnd);
    let l = layout(hwnd, &model, hdc);
    ReleaseDC(hwnd, hdc);
    hit_test(&l, x, y)
}

unsafe extern "system" fn chips_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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
            let cursor = if current_hit(hwnd, p.x, p.y) == Hit::None { IDC_ARROW } else { IDC_HAND };
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
            let hit = current_hit(hwnd, x, y);
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
                Hit::Filter(c) => post(ACTION_FILTER, c.index()),
                Hit::Recursive => post(ACTION_RECURSIVE, 0),
                Hit::None => {}
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

pub fn category_from_index(i: usize) -> Option<Category> {
    category::ALL.get(i).copied()
}

/// 移動補間；第一次繪製時直接跳到目標。回傳目前值，並記錄是否仍在播放。
fn step(tw: &mut Tween, target: f32, ms: f64, first: bool, busy: &mut bool) -> f32 {
    if first {
        tw.snap(target);
    } else {
        anim::to(tw, target, ms);
    }
    *busy |= anim::running(tw);
    anim::value(tw)
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
        let model = app::chip_model();
        let l = layout(hwnd, &model, dc);
        let first = !d.ready;
        d.ready = true;
        let mut busy = false;
        let soft_hover = theme::blend(p.ink, p.background, 0.05);

        // 類型篩選籤：作用中的籤以深色實心淡入
        for c in category::ALL {
            let i = c.index();
            let shown = l.filters.iter().any(|(fc, _)| *fc == c);
            let a = step(&mut d.active[i], (model.filter == c) as u8 as f32, anim::TOGGLE_MS, first || !shown, &mut busy);
            let h = step(&mut d.hover[i], (d.hot == Hit::Filter(c) && model.filter != c) as u8 as f32, anim::HOVER_MS, first || !shown, &mut busy);
            let Some((_, r)) = l.filters.iter().find(|(fc, _)| *fc == c) else { continue };
            let resting = anim::mix(p.surface, theme::blend(p.ink, p.surface, 0.06), h);
            let fill = anim::mix(resting, p.chip_active, a);
            let border = if a < 0.999 { Some(anim::mix(p.line, fill, a)) } else { None };
            let ink = anim::mix(p.ink, p.chip_active_ink, a);
            let count_ink = anim::mix(p.muted, theme::blend(p.chip_active_ink, p.chip_active, 0.65), a);
            theme::round_rect(dc, r, r.height(), Some(fill), border);
            let mut x = r.left + t.s(14);
            if c != Category::All {
                let size = t.s(8);
                let dot = RECT::new(x, r.top + (r.height() - size) / 2, size, size);
                theme::circle(dc, &dot, Some(p.category_dot(c)), None);
                x += t.s(14);
            }
            let (label, num) = chip_text(c, model.counts[i]);
            let lw = theme::text_width(dc, &label, t.fonts.ui);
            theme::draw_text(dc, &label, &RECT { left: x, right: x + lw + 1, ..*r }, t.fonts.ui, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            x += lw + t.s(7);
            theme::draw_text(dc, &num, &RECT { left: x, right: r.right, ..*r }, t.fonts.small_bold, count_ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        }

        // 搜尋含子資料夾（搜尋時才顯示）：勾選框交叉淡化
        if let Some(rr) = l.recursive {
            let h = step(&mut d.rec_hover, (d.hot == Hit::Recursive) as u8 as f32, anim::HOVER_MS, first, &mut busy);
            let ck = step(&mut d.rec_check, model.recursive as u8 as f32, anim::TOGGLE_MS, first, &mut busy).clamp(0.0, 1.0);
            if h > 0.001 {
                theme::round_rect(dc, &rr, rr.height(), Some(anim::mix(p.background, soft_hover, h)), None);
            }
            let gr = RECT { left: rr.left + t.s(8), right: rr.left + t.s(32), ..rr };
            if ck < 1.0 {
                theme::draw_text(dc, &glyph::CHECKBOX.to_string(), &gr, t.fonts.icon, anim::mix(p.background, p.muted, 1.0 - ck), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            if ck > 0.0 {
                theme::draw_text(dc, &glyph::CHECKBOX_CHECKED.to_string(), &gr, t.fonts.icon, anim::mix(p.background, p.accent, ck), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            let tr = RECT { left: gr.right + t.s(2), ..rr };
            theme::draw_text(dc, tr!("搜尋含子資料夾", "Include subfolders"), &tr, t.fonts.ui, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        } else {
            d.rec_check.snap(model.recursive as u8 as f32);
        }
        if busy {
            anim::request_frame(hwnd);
        }
    }
    EndPaint(hwnd, &ps);
}

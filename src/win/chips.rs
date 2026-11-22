//! 篩選籤列（參考圖的「All 15 · Pop 3 · Lo-fi 5…」）：
//! 左側依類型篩選目前清單（全部／資料夾／圖片／文件／影音／壓縮檔／其他，附數量與色點），
//! 右側是「搜尋含子資料夾」與排序切換（A–Z／最新／大小）。
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
pub const ACTION_SORT: usize = 1;
pub const ACTION_RECURSIVE: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Name,
    Newest,
    Largest,
}

impl Sort {
    pub const ALL: [Sort; 3] = [Sort::Name, Sort::Newest, Sort::Largest];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Name => "A–Z",
            Sort::Newest => "最新",
            Sort::Largest => "大小",
        }
    }

    /// （欄位, 是否遞減）
    pub fn column(self) -> (usize, bool) {
        match self {
            Sort::Name => (feather_core::sorting::COL_NAME, false),
            Sort::Newest => (feather_core::sorting::COL_MODIFIED, true),
            Sort::Largest => (feather_core::sorting::COL_SIZE, true),
        }
    }

    pub fn from_column(col: usize, desc: bool) -> Option<Sort> {
        Sort::ALL.into_iter().find(|s| s.column() == (col, desc))
    }

    fn index(self) -> usize {
        self as usize
    }
}

pub struct Model {
    pub counts: [usize; category::COUNT],
    pub filter: Category,
    pub sort: Option<Sort>,
    pub recursive: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    None,
    Filter(Category),
    Sort(Sort),
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
    /// 排序的白色膠囊（滑到選取的選項）
    sort_l: Tween,
    sort_r: Tween,
    sort_state: Option<(usize, i32)>,
    sort_hover: [Tween; 3],
    rec_check: Tween,
    rec_hover: Tween,
}

struct Layout {
    filters: Vec<(Category, RECT)>,
    sorts: Vec<(Sort, RECT)>,
    sort_box: RECT,
    recursive: RECT,
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
        sort_l: Tween::new(0.0),
        sort_r: Tween::new(0.0),
        sort_state: None,
        sort_hover: [Tween::new(0.0); 3],
        rec_check: Tween::new(0.0),
        rec_hover: Tween::new(0.0),
    });
    let class = wide(CHIPS_CLASS);
    let name = wide("類型篩選與排序");
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

fn layout(hwnd: HWND, model: &Model, hdc: HDC) -> Layout {
    let t = app::theme();
    let r = client_rect(hwnd);
    let h = t.s(32).min(r.height());
    let y = (r.height() - h) / 2;
    // 右側：排序分段
    let seg_w: Vec<i32> = Sort::ALL.iter().map(|s| theme::text_width(hdc, s.label(), t.fonts.ui_bold) + t.s(26)).collect();
    let box_w: i32 = seg_w.iter().sum::<i32>() + t.s(8);
    let sort_box = RECT { left: r.right - box_w, top: y, right: r.right, bottom: y + h };
    let mut sorts = Vec::new();
    let mut x = sort_box.left + t.s(4);
    for (s, w) in Sort::ALL.iter().zip(seg_w) {
        sorts.push((*s, RECT { left: x, top: y + t.s(3), right: x + w, bottom: y + h - t.s(3) }));
        x += w;
    }
    let rec_w = theme::text_width(hdc, "搜尋含子資料夾", t.fonts.ui) + t.s(44);
    let recursive = RECT { left: sort_box.left - t.s(12) - rec_w, top: y, right: sort_box.left - t.s(12), bottom: y + h };
    // 左側：類型篩選籤
    let mut filters = Vec::new();
    let mut x = 0;
    let limit = recursive.left - t.s(8);
    for c in category::ALL {
        let count = model.counts[c.index()];
        if c != Category::All && count == 0 && model.filter != c {
            continue;
        }
        let (label, num) = chip_text(c, count);
        let dot = if c == Category::All { 0 } else { t.s(14) };
        let w = t.s(14) + dot + theme::text_width(hdc, &label, t.fonts.ui) + t.s(7) + theme::text_width(hdc, &num, t.fonts.small_bold) + t.s(14);
        if x + w > limit {
            break;
        }
        filters.push((c, RECT { left: x, top: y, right: x + w, bottom: y + h }));
        x += w + t.s(8);
    }
    Layout { filters, sorts, sort_box, recursive }
}

fn hit_test(l: &Layout, x: i32, y: i32) -> Hit {
    for (c, r) in &l.filters {
        if r.contains(x, y) {
            return Hit::Filter(*c);
        }
    }
    for (s, r) in &l.sorts {
        if r.contains(x, y) {
            return Hit::Sort(*s);
        }
    }
    if l.recursive.contains(x, y) {
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
                Hit::Sort(s) => post(ACTION_SORT, s.index()),
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

pub fn sort_from_index(i: usize) -> Option<Sort> {
    Sort::ALL.get(i).copied()
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

        // 搜尋含子資料夾：勾選框交叉淡化
        let rr = l.recursive;
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
        theme::draw_text(dc, "搜尋含子資料夾", &tr, t.fonts.ui, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE);

        // 排序分段：選取的選項是浮起的白色膠囊，切換時滑過去
        let selected = model.sort.map(|s| s.index());
        for (s, r) in &l.sorts {
            let i = s.index();
            let h = step(&mut d.sort_hover[i], (d.hot == Hit::Sort(*s) && selected != Some(i)) as u8 as f32, anim::HOVER_MS, first, &mut busy);
            if h > 0.001 {
                theme::round_rect(dc, r, r.height(), Some(anim::mix(p.background, soft_hover, h)), None);
            }
        }
        match selected.and_then(|i| l.sorts.get(i).map(|(_, r)| (i, *r))) {
            Some((i, target)) => {
                let state = (i, rect.width());
                let slide = !first && matches!(d.sort_state, Some((sel, w)) if sel != state.0 || w == state.1);
                if slide {
                    anim::to(&mut d.sort_l, target.left as f32, anim::SLIDE_MS);
                    anim::to(&mut d.sort_r, target.right as f32, anim::SLIDE_MS);
                } else {
                    d.sort_l.snap(target.left as f32);
                    d.sort_r.snap(target.right as f32);
                }
                d.sort_state = Some(state);
                busy |= anim::running(&d.sort_l) || anim::running(&d.sort_r);
                let pill = RECT { left: anim::px(anim::value(&d.sort_l)), right: anim::px(anim::value(&d.sort_r)), ..target };
                theme::card(dc, &pill, pill.height(), p.surface, None, p, t.dpi);
            }
            None => d.sort_state = None,
        }
        for (s, r) in &l.sorts {
            let i = s.index();
            let on = selected == Some(i);
            let (font, color) = if on { (t.fonts.ui_bold, p.ink) } else { (t.fonts.ui, anim::mix(p.muted, p.ink, anim::value(&d.sort_hover[i]))) };
            theme::draw_text(dc, s.label(), r, font, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        let _ = l.sort_box;
        if busy {
            anim::request_frame(hwnd);
        }
    }
    EndPaint(hwnd, &ps);
}

//! 設定 › 快速鍵：一個功能分類（分頁與雙欄、檢視、移動與搜尋、檔案與資料夾、其他）的命令與它們的按法
//! （自己畫、可以捲動的清單，設定視窗的子視窗；上面的分類按鈕由設定視窗放，用 [`set_section`] 換）。
//!
//! 點一個命令（或用方向鍵選到再按 Enter）就開始等新的按法：直接在鍵盤上按想要的組合就換成它，Esc 取消。
//! 不能用的按法（沒有 Ctrl 或 Alt 的字母、數字、空白鍵等，見 `keymap::Combo::reserved`）在那一列說明原因；
//! 已經是別的命令的按法時先問要不要改給這個命令。改過的命令後面有「還原」，「不用快速鍵」把它清空。
//! 改的是這份清單自己的複本，按設定視窗的「儲存」才寫進設定（`Settings::shortcuts`）。

use super::app;
use super::dialog::{self, Icon};
use super::sys::*;
use super::theme;
use super::util::*;
use feather_core::keymap::{Cmd, Combo, Keymap, Section};
use std::cell::Cell;

pub const CLASS: &str = "FeatherFiles.KeyList";

/// 分類（設定視窗上面的分類按鈕依這個順序）
pub const SECTIONS: [Section; 5] = [Section::Tabs, Section::View, Section::Navigate, Section::Files, Section::Other];

thread_local! {
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

/// 滑鼠下的東西
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hot {
    Row(usize),
    /// 「還原」：改回預設的按法
    Reset(usize),
    /// 等按法時的「不用快速鍵」
    Clear(usize),
}

pub struct Fonts {
    pub normal: HFONT,
    pub small: HFONT,
}

struct Data {
    /// 控制項代號（通知設定視窗時用）
    id: u16,
    overrides: Vec<(String, Vec<String>)>,
    map: Keymap,
    /// 目前分類的命令
    lines: Vec<Cmd>,
    fonts: Fonts,
    percent: u32,
    scroll: i32,
    hot: Option<Hot>,
    /// 鍵盤選到的列
    focus: usize,
    /// 正在等新按法的那一列
    capture: Option<usize>,
    /// 等按法時的說明（按了不能用的組合時換成原因）
    message: Option<String>,
    tracking: bool,
}

impl Data {
    fn k(&self, v: i32) -> i32 {
        scale(v * self.percent as i32 / 100, app::theme().dpi)
    }

    fn row_h(&self) -> i32 {
        self.k(38).max(theme::line_height(self.fonts.normal) + self.k(14))
    }

    /// 每一列的上緣（內容座標）與總高度
    fn tops(&self) -> (Vec<i32>, i32) {
        let h = self.row_h();
        let out: Vec<i32> = (0..self.lines.len() as i32).map(|i| i * h).collect();
        (out, self.lines.len() as i32 * h + self.k(4))
    }

    fn command(&self, row: usize) -> Option<Cmd> {
        self.lines.get(row).copied()
    }

    fn overridden(&self, cmd: Cmd) -> bool {
        self.overrides.iter().any(|(id, _)| Cmd::from_id(id) == Some(cmd))
    }

    fn rebuild(&mut self) {
        self.map = Keymap::with_overrides(&self.overrides).0;
    }

    fn set_override(&mut self, cmd: Cmd, combos: Vec<String>) {
        let id = cmd.id();
        self.overrides.retain(|(i, _)| Cmd::from_id(i) != Some(cmd));
        self.overrides.push((id, combos));
        self.rebuild();
    }
}

/// 自訂改了：告訴設定視窗（WM_COMMAND，通知碼 1），分類按鈕上的標示跟著更新
unsafe fn notify(hwnd: HWND) {
    let Some(id) = data(hwnd).map(|d| d.id as usize) else { return };
    SendMessageW(GetParent(hwnd), WM_COMMAND, id | (1 << 16), hwnd as LPARAM);
}

fn lines(section: Section) -> Vec<Cmd> {
    Cmd::CUSTOMIZABLE.iter().copied().filter(|c| c.section() == section).collect()
}

fn register(hinst: HINSTANCE) {
    if REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    register_class(hinst, CLASS, CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, keylist_proc, IDC_ARROW);
}

/// 建立清單，先顯示第一個分類（字型由設定視窗擁有，這裡不刪）
pub fn create(parent: HWND, id: u16, rect: RECT, fonts: Fonts, percent: u32, overrides: Vec<(String, Vec<String>)>) -> HWND {
    register(unsafe { GetModuleHandleW(std::ptr::null()) });
    let mut d = Box::new(Data { id, overrides, map: Keymap::default(), lines: lines(SECTIONS[0]), fonts, percent, scroll: 0, hot: None, focus: 0, capture: None, message: None, tracking: false });
    d.rebuild();
    let class = wide(CLASS);
    let name = wide(tr!("快速鍵", "Keyboard shortcuts"));
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            name.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL,
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            parent,
            id as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        if hwnd == 0 {
            return 0;
        }
        theme::apply_explorer_theme(hwnd, app::theme().pal.dark);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(d) as isize);
        update_scroll(hwnd);
        hwnd
    }
}

unsafe fn data<'a>(hwnd: HWND) -> Option<&'a mut Data> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Data).as_mut()
}

/// 目前的自訂（儲存設定時讀）
pub fn overrides(hwnd: HWND) -> Vec<(String, Vec<String>)> {
    unsafe { data(hwnd).map(|d| d.overrides.clone()).unwrap_or_default() }
}

/// 換成另一份自訂（「還原『快速鍵』的預設值」是空的）
pub fn set_overrides(hwnd: HWND, overrides: Vec<(String, Vec<String>)>) {
    unsafe {
        if let Some(d) = data(hwnd) {
            d.overrides = overrides;
            d.capture = None;
            d.message = None;
            d.rebuild();
            invalidate(hwnd);
        }
    }
}

/// 正在等新的按法（設定視窗的按鍵處理先不要攔 Ctrl+Tab 等換分類的鍵）
pub fn capturing(hwnd: HWND) -> bool {
    unsafe { data(hwnd).is_some_and(|d| d.capture.is_some()) }
}

/// 換成另一個分類的命令（設定視窗上面的分類按鈕）
pub fn set_section(hwnd: HWND, section: Section) {
    unsafe {
        if let Some(d) = data(hwnd) {
            d.lines = lines(section);
            d.capture = None;
            d.message = None;
            d.focus = 0;
            d.hot = None;
            d.scroll = 0;
        }
        update_scroll(hwnd);
        invalidate(hwnd);
    }
}

/// 這個分類裡有幾個命令改過（分類按鈕上標示用）
pub fn changed_in(hwnd: HWND, section: Section) -> usize {
    unsafe { data(hwnd).map_or(0, |d| lines(section).into_iter().filter(|c| d.overridden(*c)).count()) }
}

unsafe fn update_scroll(hwnd: HWND) {
    let Some(d) = data(hwnd) else { return };
    let view = client_rect(hwnd).height().max(1);
    let (_, total) = d.tops();
    d.scroll = d.scroll.clamp(0, (total - view).max(0));
    let si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL, nMin: 0, nMax: total.max(1) - 1, nPage: view as u32, nPos: d.scroll, nTrackPos: 0 };
    SetScrollInfo(hwnd, SB_VERT, &si, 1);
}

unsafe fn scroll_to(hwnd: HWND, pos: i32) {
    if let Some(d) = data(hwnd) {
        d.scroll = pos;
    }
    update_scroll(hwnd);
    invalidate(hwnd);
}

/// 讓第 `row` 列整個看得到
unsafe fn ensure_visible(hwnd: HWND, row: usize) {
    let Some(d) = data(hwnd) else { return };
    let (tops, _) = d.tops();
    let Some(top) = tops.get(row).copied() else { return };
    let bottom = top + d.row_h();
    let view = client_rect(hwnd).height();
    if top < d.scroll {
        scroll_to(hwnd, top);
    } else if bottom > d.scroll + view {
        scroll_to(hwnd, bottom - view);
    }
}

/// 一列右邊「還原」「不用快速鍵」的位置（客戶區座標）
fn link_rects(d: &Data, row_rect: &RECT, hdc: HDC) -> (RECT, RECT) {
    let pad = d.k(12);
    let reset_w = theme::text_width(hdc, tr!("還原", "Reset"), d.fonts.small) + d.k(16);
    let clear_w = theme::text_width(hdc, tr!("不用快速鍵", "No shortcut"), d.fonts.small) + d.k(16);
    let reset = RECT { left: row_rect.right - pad - reset_w, right: row_rect.right - pad, ..*row_rect };
    let clear = RECT { left: reset.left - d.k(4) - clear_w, right: reset.left - d.k(4), ..*row_rect };
    (reset, clear)
}

unsafe fn hit(hwnd: HWND, x: i32, y: i32) -> Option<Hot> {
    let d = data(hwnd)?;
    let (tops, _) = d.tops();
    let r = client_rect(hwnd);
    let cy = y + d.scroll;
    let row = tops.iter().enumerate().rev().find(|(_, t)| **t <= cy).map(|(i, _)| i)?;
    let cmd = d.command(row)?;
    let row_rect = RECT { left: 0, top: tops[row] - d.scroll, right: r.right, bottom: tops[row] - d.scroll + d.row_h() };
    if !row_rect.contains(x, y) {
        return None;
    }
    let hdc = GetDC(hwnd);
    let (reset, clear) = link_rects(d, &row_rect, hdc);
    ReleaseDC(hwnd, hdc);
    if d.overridden(cmd) && reset.contains(x, y) {
        return Some(Hot::Reset(row));
    }
    if d.capture == Some(row) && clear.contains(x, y) {
        return Some(Hot::Clear(row));
    }
    Some(Hot::Row(row))
}

/// 開始等第 `row` 列的新按法
unsafe fn begin_capture(hwnd: HWND, row: usize) {
    let Some(d) = data(hwnd) else { return };
    if d.command(row).is_none() {
        return;
    }
    d.capture = Some(row);
    d.focus = row;
    d.message = None;
    SetFocus(hwnd);
    ensure_visible(hwnd, row);
    invalidate(hwnd);
}

unsafe fn end_capture(hwnd: HWND) {
    if let Some(d) = data(hwnd) {
        d.capture = None;
        d.message = None;
    }
    invalidate(hwnd);
}

/// 等按法時按了 `combo`
unsafe fn take_combo(hwnd: HWND, combo: Combo) {
    let Some(d) = data(hwnd) else { return };
    let Some(row) = d.capture else { return };
    let Some(cmd) = d.command(row) else { return };
    if combo.reserved() {
        d.message = Some(trf!(
            "「{}」不能用：打字跳到項目、預覽或換焦點要用。請加上 Ctrl 或 Alt。",
            "“{}” can't be used: it's needed for typing, preview or focus. Add Ctrl or Alt.",
            combo.label()
        ));
        invalidate(hwnd);
        return;
    }
    if !combo.round_trips() {
        d.message = Some(tr!("這個按鍵沒辦法存進設定，請換一個。", "This key can't be saved in the settings; try another.").into());
        invalidate(hwnd);
        return;
    }
    let taken = d.map.taken_by(cmd, combo);
    if !taken.is_empty() {
        let others = taken.iter().map(|c| format!("「{}」", c.display_name())).collect::<Vec<_>>().join(tr!("、", ", "));
        let owner = GetAncestor(hwnd, GA_ROOT);
        let ok = dialog::confirm(
            owner,
            Icon::Question,
            &trf!("{} 已經是{}的快速鍵", "{} is already the shortcut for {}", combo.label(), others),
            &trf!("改給「{}」之後，{}就不再用這個按法。", "If “{}” uses it instead, {} won't use it anymore.", cmd.display_name(), others),
            tr!("改用這個按法", "Use it here"),
        );
        // 對話方塊關閉後資料還在（同一個視窗），但再取一次
        let Some(d) = data(hwnd) else { return };
        if !ok {
            d.capture = None;
            d.message = None;
            invalidate(hwnd);
            SetFocus(hwnd);
            return;
        }
        // 拿走的命令也是自訂的：從它的自訂裡拿掉這個按法，設定檔才乾淨
        for other in &taken {
            if let Some((_, list)) = d.overrides.iter_mut().find(|(id, _)| Cmd::from_id(id) == Some(*other)) {
                list.retain(|t| feather_core::keymap::parse_combo(t) != Some(combo));
            }
        }
        SetFocus(hwnd);
    }
    let Some(d) = data(hwnd) else { return };
    d.set_override(cmd, vec![combo.label()]);
    d.capture = None;
    d.message = None;
    invalidate(hwnd);
    notify(hwnd);
}

unsafe extern "system" fn keylist_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(d) = data(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            paint(hwnd, d);
            return 0;
        }
        WM_SIZE => {
            update_scroll(hwnd);
            return 0;
        }
        // 等按法時所有按鍵（含 Tab、Esc、Enter）都給這裡；平常只要方向鍵與 Enter
        WM_GETDLGCODE => {
            if d.capture.is_some() {
                return DLGC_WANTALLKEYS;
            }
            // Enter 開始等按法（不是按下對話方塊的預設按鈕）
            let m = lparam as *const MSG;
            if !m.is_null() && (*m).message == WM_KEYDOWN && (*m).wParam as u32 == VK_RETURN {
                return DLGC_WANTARROWS | DLGC_WANTALLKEYS;
            }
            return DLGC_WANTARROWS;
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            // 點到別的地方：不再等按法
            if msg == WM_KILLFOCUS && d.capture.is_some() {
                d.capture = None;
                d.message = None;
            }
            invalidate(hwnd);
            return 0;
        }
        WM_VSCROLL => {
            let view = client_rect(hwnd).height();
            let line = d.k(36);
            let pos = match loword(wparam) as usize {
                SB_LINEUP => d.scroll - line,
                SB_LINEDOWN => d.scroll + line,
                SB_PAGEUP => d.scroll - (view - line).max(line),
                SB_PAGEDOWN => d.scroll + (view - line).max(line),
                SB_TOP => 0,
                SB_BOTTOM => i32::MAX / 2,
                SB_THUMBTRACK | SB_THUMBPOSITION => {
                    let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, nMin: 0, nMax: 0, nPage: 0, nPos: 0, nTrackPos: 0 };
                    GetScrollInfo(hwnd, SB_VERT, &mut si);
                    si.nTrackPos
                }
                _ => d.scroll,
            };
            scroll_to(hwnd, pos);
            return 0;
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            let pos = d.scroll - delta * d.k(36) * 3 / 120;
            scroll_to(hwnd, pos);
            return 0;
        }
        WM_MOUSEMOVE => {
            if !d.tracking {
                track_mouse_leave(hwnd);
                d.tracking = true;
            }
            let h = hit(hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            let d = data(hwnd).expect("data");
            if h != d.hot {
                d.hot = h;
                invalidate(hwnd);
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            d.tracking = false;
            d.hot = None;
            invalidate(hwnd);
            return 0;
        }
        WM_SETCURSOR => {
            let mut p = cursor_pos();
            ScreenToClient(hwnd, &mut p);
            let link = matches!(hit(hwnd, p.x, p.y), Some(Hot::Reset(_) | Hot::Clear(_)));
            SetCursor(load_cursor(if link { IDC_HAND } else { IDC_ARROW }));
            return 1;
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            SetFocus(hwnd);
            match hit(hwnd, get_x_lparam(lparam), get_y_lparam(lparam)) {
                Some(Hot::Reset(row)) => {
                    let d = data(hwnd).expect("data");
                    if let Some(cmd) = d.command(row) {
                        d.overrides.retain(|(id, _)| Cmd::from_id(id) != Some(cmd));
                        d.rebuild();
                        d.capture = None;
                        d.message = None;
                        d.focus = row;
                    }
                    invalidate(hwnd);
                    notify(hwnd);
                }
                Some(Hot::Clear(row)) => {
                    let d = data(hwnd).expect("data");
                    if let Some(cmd) = d.command(row) {
                        d.set_override(cmd, Vec::new());
                        d.capture = None;
                        d.message = None;
                    }
                    invalidate(hwnd);
                    notify(hwnd);
                }
                Some(Hot::Row(row)) => {
                    let d = data(hwnd).expect("data");
                    if d.capture == Some(row) {
                        end_capture(hwnd);
                    } else {
                        begin_capture(hwnd, row);
                    }
                }
                None => {}
            }
            return 0;
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let mut vk = wparam as u32;
            if vk == VK_PROCESSKEY {
                vk = ImmGetVirtualKey(hwnd);
            }
            let ctrl = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            // F10 也是 WM_SYSKEYDOWN，但沒有按 Alt
            let alt = key_down(VK_MENU);
            if d.capture.is_some() {
                // 只按了 Ctrl、Shift、Alt、Windows 鍵：還沒按完，繼續等
                if matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5) {
                    return 0;
                }
                if vk == VK_ESCAPE && !ctrl && !shift && !alt {
                    end_capture(hwnd);
                    return 0;
                }
                // 按住不放的自動重複不算
                if lparam & (1 << 30) == 0 {
                    take_combo(hwnd, Combo { key: vk, ctrl, shift, alt });
                }
                return 0;
            }
            let n = d.lines.len();
            let (first, last) = (0, n.saturating_sub(1));
            let step = |from: usize, delta: isize| -> usize { (from as isize + delta).clamp(0, last as isize) as usize };
            let target = match vk {
                VK_UP => Some(step(d.focus, -1)),
                VK_DOWN => Some(step(d.focus, 1)),
                VK_HOME => Some(first),
                VK_END => Some(last),
                VK_PRIOR => Some((0..8).fold(d.focus, |f, _| step(f, -1))),
                VK_NEXT => Some((0..8).fold(d.focus, |f, _| step(f, 1))),
                VK_RETURN | VK_SPACE => {
                    let row = d.focus;
                    begin_capture(hwnd, row);
                    None
                }
                _ => None,
            };
            if let Some(row) = target {
                let d = data(hwnd).expect("data");
                d.focus = row.min(n.saturating_sub(1));
                let f = d.focus;
                ensure_visible(hwnd, f);
                invalidate(hwnd);
            }
            return 0;
        }
        // 字元訊息不處理（不發出提示音）
        WM_CHAR | WM_SYSCHAR => return 0,
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

unsafe fn paint(hwnd: HWND, d: &Data) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let rect = client_rect(hwnd);
    {
        let buf = theme::Buffered::new(hdc, rect);
        let dc = buf.hdc;
        let t = app::theme();
        let p = &t.pal;
        theme::fill(dc, &rect, p.surface);
        let focused = GetFocus() == hwnd;
        let (tops, _) = d.tops();
        let warn = if p.dark { rgb(236, 176, 92) } else { rgb(176, 112, 20) };
        let flags = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS;
        for (i, cmd) in d.lines.iter().enumerate() {
            let top = tops[i] - d.scroll;
            let h = d.row_h();
            if top + h < 0 || top > rect.bottom {
                continue;
            }
            let row = RECT { left: 0, top, right: rect.right, bottom: top + h };
            {
                {
                    let capturing = d.capture == Some(i);
                    let hot = matches!(d.hot, Some(Hot::Row(r) | Hot::Reset(r) | Hot::Clear(r)) if r == i);
                    let pill = RECT { left: row.left + d.k(2), top: row.top + d.k(2), right: row.right - d.k(2), bottom: row.bottom - d.k(2) };
                    if capturing {
                        theme::round_rect(dc, &pill, d.k(10), Some(theme::blend(p.accent, p.surface, 0.10)), Some(p.accent));
                    } else if hot || (focused && d.focus == i) {
                        let border = (focused && d.focus == i).then_some(theme::blend(p.ink, p.surface, 0.25));
                        theme::round_rect(dc, &pill, d.k(10), Some(theme::blend(p.ink, p.surface, 0.05)), border);
                    }
                    // 命令名稱（改過的前面一個重點色小圓點）
                    let overridden = d.overridden(*cmd);
                    let name_x = row.left + d.k(22);
                    if overridden {
                        let s = d.k(6);
                        let dot = RECT::new(row.left + d.k(9), row.top + (h - s) / 2, s, s);
                        theme::circle(dc, &dot, Some(p.accent), None);
                    }
                    let col = row.left + (row.width() * 44 / 100).max(d.k(180));
                    theme::draw_text(dc, &cmd.display_name(), &RECT { left: name_x, right: col - d.k(12), ..row }, d.fonts.normal, p.ink, flags);
                    let (reset, clear) = link_rects(d, &row, dc);
                    let chips_right = if overridden || capturing { (if capturing { clear.left } else { reset.left }) - d.k(8) } else { row.right - d.k(12) };
                    if capturing {
                        let text = d.message.clone().unwrap_or_else(|| tr!("請直接按新的快速鍵…（Esc 取消）", "Press the new shortcut… (Esc cancels)").into());
                        let color = if d.message.is_some() { warn } else { p.accent };
                        theme::draw_text(dc, &text, &RECT { left: col, right: chips_right, ..row }, d.fonts.small, color, flags);
                        let c_hot = d.hot == Some(Hot::Clear(i));
                        theme::draw_text(dc, tr!("不用快速鍵", "No shortcut"), &clear, d.fonts.small, if c_hot { p.accent } else { p.muted }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    } else {
                        // 目前的按法：小膠囊；沒有時寫「（沒有）」
                        let combos = d.map.combos(*cmd);
                        let mut x = col;
                        if combos.is_empty() {
                            theme::draw_text(dc, tr!("（沒有）", "(none)"), &RECT { left: x, right: chips_right, ..row }, d.fonts.small, p.muted, flags);
                        }
                        let chip_h = theme::line_height(d.fonts.small) + d.k(8);
                        for c in combos {
                            let label = c.label();
                            let w = theme::text_width(dc, &label, d.fonts.small) + d.k(18);
                            if x + w > chips_right {
                                theme::draw_text(dc, "…", &RECT { left: x, right: chips_right, ..row }, d.fonts.small, p.muted, flags);
                                break;
                            }
                            let chip = RECT::new(x, row.top + (h - chip_h) / 2, w, chip_h);
                            theme::round_rect(dc, &chip, d.k(8), Some(p.chip), Some(p.line));
                            theme::draw_text(dc, &label, &chip, d.fonts.small, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                            x += w + d.k(6);
                        }
                    }
                    if overridden {
                        let r_hot = d.hot == Some(Hot::Reset(i));
                        theme::draw_text(dc, tr!("還原", "Reset"), &reset, d.fonts.small, if r_hot { p.accent } else { p.muted }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    }
                }
            }
        }
    }
    EndPaint(hwnd, &ps);
}

//! 說明頁：獨立的視窗，不擋住主視窗（可以一邊看一邊操作），左邊是分類，右邊是可以捲動的內容。
//! 原本塞在一個對話方塊裡，快速鍵加上說明文字放不下，搜尋語法還要另外開一個。
//!
//! - 快速鍵：從快速鍵表（`feather_core::keymap::Keymap::help_sections`）產生，依分頁、檢視、移動與搜尋、檔案分段；
//!   快速預覽的按鍵依目前的設定（懸停預覽或空白鍵預覽）；另有滑鼠的操作
//! - 搜尋：範圍、篩選條件、搜尋語法（`searchquery::syntax_rows`，和 README 的表相同）、相關性排序、限制
//! - 快速預覽、檔案與資料夾、檢視與版面、關於
//!
//! 只開一個：已經開著時換成新的內容（設定可能改了）並切到要看的分類。文字跟著「文字大小」放大（同設定視窗，
//! 放不進螢幕時縮小，最小 100%）；顏色跟著主題。按鍵：↑↓／PgUp／PgDn／Home／End 捲動，Ctrl+Tab／Ctrl+PgDn
//! 下一個分類，Ctrl+C 複製這一頁的文字，Esc 關閉。

use super::app;
use super::clipboard;
use super::controls::{self, Kind, Surface};
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::keymap::Keymap;
use feather_core::settings::Settings;
use std::sync::atomic::{AtomicIsize, Ordering};

pub const CLASS: &str = "FeatherFiles.Help";

/// 邏輯尺寸（跟著文字大小放大）
const BASE_W: i32 = 900;
const BASE_H: i32 = 660;
const NAV_W: i32 = 210;
const MARGIN: i32 = 36;
const NAV_FIRST: u16 = 100;

/// 內容的一段
pub enum Block {
    /// 小標題
    Section(String),
    Text(String),
    /// 兩欄表：左欄以小膠囊顯示（快速鍵、搜尋語法），右欄是說明
    Keys(Vec<(String, String)>),
    Bullets(Vec<String>),
    /// 淡色的小字
    Note(String),
}

pub struct Page {
    pub title: String,
    pub glyph: char,
    pub blocks: Vec<Block>,
}

/// 開啟時先看的分類
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Topic {
    Shortcuts = 0,
    Search = 1,
}

/// 開著的說明頁（只開一個）
static HELP: AtomicIsize = AtomicIsize::new(0);

struct Fonts {
    title: HFONT,
    section: HFONT,
    body: HFONT,
    small: HFONT,
    chip: HFONT,
    nav: HFONT,
    nav_bold: HFONT,
}

impl Fonts {
    fn new(percent: u32, dpi: u32) -> Fonts {
        Fonts {
            title: theme::ui_font_weight(15, percent, theme::FW_BOLD, dpi),
            section: theme::ui_font(11, percent, true, dpi),
            body: theme::ui_font(10, percent, false, dpi),
            small: theme::ui_font(9, percent, false, dpi),
            chip: theme::ui_font(9, percent, true, dpi),
            nav: theme::ui_font(9, percent, false, dpi),
            nav_bold: theme::ui_font(9, percent, true, dpi),
        }
    }

    fn destroy(&self) {
        for f in [self.title, self.section, self.body, self.small, self.chip, self.nav, self.nav_bold] {
            if f != 0 {
                unsafe {
                    DeleteObject(f);
                }
            }
        }
    }
}

/// 排好位置的內容（y 從內容的最上面算，畫的時候減去捲動的位置）
enum Item {
    Title(String, RECT),
    Section(String, RECT),
    Text(String, RECT),
    Note(String, RECT),
    Bullet(String, RECT),
    Key { key: String, chip: RECT, desc: String, text: RECT },
}

struct State {
    pages: Vec<Page>,
    page: usize,
    nav: Vec<HWND>,
    fonts: Fonts,
    percent: u32,
    items: Vec<Item>,
    /// 內容的總高度與目前捲到的位置
    height: i32,
    scroll: i32,
}

impl State {
    fn k(&self, v: i32) -> i32 {
        scale(v * self.percent as i32 / 100, app::theme().dpi)
    }
}

pub fn register(hinst: HINSTANCE) {
    register_class(hinst, CLASS, CS_HREDRAW | CS_VREDRAW, help_proc, IDC_ARROW);
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut()
}

/// 版面縮放比例：跟著文字大小，但整個視窗要放得進螢幕的工作區（放不下就縮小，最小 100%）
fn layout_percent(text_size: u32, work: &RECT, dpi: u32) -> u32 {
    let px = |v: i32, pct: u32| (v as i64 * pct as i64 * dpi as i64 / (100 * 96)) as i32;
    let mut pct = text_size.max(100);
    while pct > 100 && (px(BASE_W, pct) > work.width() * 96 / 100 || px(BASE_H, pct) > work.height() * 92 / 100) {
        pct -= 5;
    }
    pct.max(100)
}

/// 開啟說明頁（已經開著就換上新的內容、切到 `topic`，並帶到前面）
pub fn open(owner: HWND, pages: Vec<Page>, topic: Topic) {
    let t = app::theme();
    let work = unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: RECT::default(), rcWork: RECT::default(), dwFlags: 0 };
        GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut mi);
        mi.rcWork
    };
    let percent = layout_percent(t.text_size, &work, t.dpi);
    let existing = HELP.load(Ordering::Relaxed);
    unsafe {
        if existing != 0 && IsWindow(existing) != 0 {
            match state(existing) {
                // 文字大小改了：關掉重開，字型與版面才會對
                Some(st) if st.percent == percent && st.pages.len() == pages.len() => {
                    st.pages = pages;
                    select(existing, st, topic as usize);
                    if IsIconic(existing) != 0 {
                        ShowWindow(existing, SW_RESTORE);
                    }
                    SetForegroundWindow(existing);
                    return;
                }
                _ => {
                    DestroyWindow(existing);
                }
            }
        }
    }
    let k = |v: i32| scale(v * percent as i32 / 100, t.dpi);
    let (cw, ch) = (k(BASE_W), k(BASE_H));
    let class = wide(CLASS);
    let title = wide(tr!("Stillfeather 說明", "Stillfeather Help"));
    unsafe {
        let mut or = RECT::default();
        GetWindowRect(owner, &mut or);
        let x = (or.left + (or.width() - cw) / 2).clamp(work.left, (work.right - cw).max(work.left));
        let y = (or.top + (or.height() - ch) / 2).clamp(work.top, (work.bottom - ch).max(work.top));
        // 有擁有者（主視窗）：留在主視窗前面、不另佔工作列，主視窗關閉時一起關閉；不是強制回應，主視窗照常可以操作
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            (WS_OVERLAPPEDWINDOW & !WS_MINIMIZEBOX) | WS_VSCROLL | WS_CLIPCHILDREN,
            x,
            y,
            cw,
            ch,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        if hwnd == 0 {
            return;
        }
        theme::style_caption(hwnd, &t.pal, t.pal.surface);
        // 捲軸跟著深淺色
        theme::apply_explorer_theme(hwnd, t.pal.dark);
        let fonts = Fonts::new(percent, t.dpi);
        let mut st = Box::new(State { pages, page: 0, nav: Vec::new(), fonts, percent, items: Vec::new(), height: 0, scroll: 0 });
        create_nav(hwnd, &mut st);
        let raw = Box::into_raw(st);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        HELP.store(hwnd, Ordering::Relaxed);
        select(hwnd, &mut *raw, topic as usize);
        ShowWindow(hwnd, SW_SHOW);
        // 焦點留在說明頁本身（分類按鈕按了也不搶焦點），按鍵才會到這裡
        SetFocus(hwnd);
    }
}

unsafe fn create_nav(hwnd: HWND, st: &mut State) {
    for h in st.nav.drain(..) {
        DestroyWindow(h);
    }
    let nav_h = st.k(40);
    for (i, page) in st.pages.iter().enumerate() {
        let h = controls::create(hwnd, NAV_FIRST + i as u16, Kind::Nav(page.glyph), &page.title, Surface::Sidebar, false);
        controls::set_fonts(h, st.fonts.nav, st.fonts.nav_bold);
        move_window(h, RECT::new(st.k(12), st.k(16) + i as i32 * (nav_h + st.k(4)), st.k(NAV_W) - st.k(24), nav_h));
        st.nav.push(h);
    }
}

/// 切到第 `page` 個分類：回到最上面、重排內容
unsafe fn select(hwnd: HWND, st: &mut State, page: usize) {
    st.page = page.min(st.pages.len().saturating_sub(1));
    // 內容換了，分類的名稱也可能換了（語言、設定）
    for (i, h) in st.nav.iter().enumerate() {
        controls::set_checked(*h, i == st.page);
        if let Some(p) = st.pages.get(i) {
            set_window_text(*h, &p.title);
        }
    }
    st.scroll = 0;
    relayout(hwnd, st);
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

/// 依視窗寬度排好這一頁的內容（換分類、縮放視窗時）
unsafe fn relayout(hwnd: HWND, st: &mut State) {
    let r = client_rect(hwnd);
    let x0 = st.k(NAV_W + MARGIN);
    let width = (r.right - st.k(MARGIN) - x0).max(st.k(240));
    let hdc = GetDC(hwnd);
    let mut items = Vec::new();
    let wrap = DT_WORDBREAK;
    let f = &st.fonts;
    let mut y = st.k(24);
    let Some(page) = st.pages.get(st.page) else {
        ReleaseDC(hwnd, hdc);
        return;
    };
    let h = theme::line_height(f.title);
    items.push(Item::Title(page.title.clone(), RECT::new(x0, y, width, h)));
    y += h + st.k(10);
    for block in &page.blocks {
        match block {
            Block::Section(text) => {
                y += st.k(18);
                let h = theme::text_height(hdc, text, f.section, width, wrap).max(theme::line_height(f.section));
                items.push(Item::Section(text.clone(), RECT::new(x0, y, width, h)));
                y += h + st.k(8);
            }
            Block::Text(text) => {
                let h = theme::text_height(hdc, text, f.body, width, wrap);
                items.push(Item::Text(text.clone(), RECT::new(x0, y, width, h)));
                y += h + st.k(10);
            }
            Block::Note(text) => {
                let h = theme::text_height(hdc, text, f.small, width, wrap);
                items.push(Item::Note(text.clone(), RECT::new(x0, y, width, h)));
                y += h + st.k(10);
            }
            Block::Bullets(list) => {
                let indent = st.k(18);
                for text in list {
                    let h = theme::text_height(hdc, text, f.body, width - indent, wrap);
                    items.push(Item::Bullet(text.clone(), RECT::new(x0 + indent, y, width - indent, h)));
                    y += h + st.k(6);
                }
                y += st.k(4);
            }
            Block::Keys(rows) => {
                // 左欄的寬度：最寬的按鍵，但不超過內容的 45%（太長的以「…」省略）
                let key_w = (rows.iter().map(|(k, _)| theme::text_width(hdc, k, f.chip)).max().unwrap_or(0) + st.k(22)).min(width * 45 / 100);
                let desc_x = x0 + key_w + st.k(16);
                let desc_w = (x0 + width - desc_x).max(st.k(80));
                let chip_h = st.k(26).max(theme::line_height(f.chip) + st.k(8));
                for (key, desc) in rows {
                    let dh = theme::text_height(hdc, desc, f.body, desc_w, wrap).max(theme::line_height(f.body));
                    let row_h = (chip_h + st.k(8)).max(dh + st.k(12));
                    // 一行的說明和膠囊垂直置中；好幾行時膠囊對齊第一行
                    let chip_top = if dh <= theme::line_height(f.body) { y + (row_h - chip_h) / 2 } else { y + st.k(4) };
                    let text_top = if dh <= theme::line_height(f.body) { y + (row_h - dh) / 2 } else { y + st.k(4) + (chip_h - theme::line_height(f.body)) / 2 };
                    items.push(Item::Key { key: key.clone(), chip: RECT::new(x0, chip_top, key_w, chip_h), desc: desc.clone(), text: RECT::new(desc_x, text_top, desc_w, dh) });
                    y += row_h;
                }
                y += st.k(10);
            }
        }
    }
    ReleaseDC(hwnd, hdc);
    st.items = items;
    st.height = y + st.k(30);
    set_scroll(hwnd, st, st.scroll);
}

/// 捲到 `pos`（限制在範圍內），更新捲軸
unsafe fn set_scroll(hwnd: HWND, st: &mut State, pos: i32) {
    let view = client_rect(hwnd).height().max(1);
    let max = (st.height - view).max(0);
    let pos = pos.clamp(0, max);
    let changed = pos != st.scroll;
    st.scroll = pos;
    // 捲軸一直在（不需要捲動時停用）：顯示、隱藏捲軸會改變內容的寬度，又要重排一次
    let si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL, nMin: 0, nMax: st.height.max(1) - 1, nPage: view as u32, nPos: pos, nTrackPos: 0 };
    SetScrollInfo(hwnd, SB_VERT, &si, 1);
    if changed {
        let r = client_rect(hwnd);
        let content = RECT { left: st.k(NAV_W), ..r };
        InvalidateRect(hwnd, &content, 0);
    }
}

/// 這一頁的純文字（Ctrl+C）
fn page_text(page: &Page) -> String {
    let mut out = vec![page.title.clone()];
    for block in &page.blocks {
        match block {
            Block::Section(t) => out.push(format!("\r\n{t}")),
            Block::Text(t) | Block::Note(t) => out.push(t.clone()),
            Block::Bullets(list) => out.extend(list.iter().map(|t| format!("• {t}"))),
            Block::Keys(rows) => out.extend(rows.iter().map(|(k, d)| format!("{k}\t{d}"))),
        }
    }
    out.join("\r\n").replace('\n', "\r\n").replace("\r\r\n", "\r\n")
}

unsafe extern "system" fn help_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(st) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            paint(hwnd, st);
            return 0;
        }
        WM_SIZE => {
            relayout(hwnd, st);
            InvalidateRect(hwnd, std::ptr::null(), 0);
            return 0;
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize = POINT { x: st.k(NAV_W + 420), y: st.k(360) };
            return 0;
        }
        WM_COMMAND => {
            let id = loword(wparam);
            if (NAV_FIRST..NAV_FIRST + st.pages.len() as u16).contains(&id) {
                select(hwnd, st, (id - NAV_FIRST) as usize);
                SetFocus(hwnd);
            }
            return 0;
        }
        WM_VSCROLL => {
            let view = client_rect(hwnd).height();
            let line = st.k(40);
            let pos = match loword(wparam) as usize {
                SB_LINEUP => st.scroll - line,
                SB_LINEDOWN => st.scroll + line,
                SB_PAGEUP => st.scroll - (view - line).max(line),
                SB_PAGEDOWN => st.scroll + (view - line).max(line),
                SB_TOP => 0,
                SB_BOTTOM => st.height,
                SB_THUMBTRACK | SB_THUMBPOSITION => {
                    let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, nMin: 0, nMax: 0, nPage: 0, nPos: 0, nTrackPos: 0 };
                    GetScrollInfo(hwnd, SB_VERT, &mut si);
                    si.nTrackPos
                }
                _ => st.scroll,
            };
            set_scroll(hwnd, st, pos);
            return 0;
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            // 一格三行（同系統預設）
            set_scroll(hwnd, st, st.scroll - delta * st.k(20) * 3 / 120);
            return 0;
        }
        WM_KEYDOWN => {
            let vk = wparam as u32;
            let ctrl = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            let view = client_rect(hwnd).height();
            let line = st.k(40);
            let n = st.pages.len().max(1);
            match vk {
                VK_ESCAPE => {
                    DestroyWindow(hwnd);
                }
                VK_TAB if ctrl => select(hwnd, st, (st.page + if shift { n - 1 } else { 1 }) % n),
                VK_NEXT if ctrl => select(hwnd, st, (st.page + 1) % n),
                VK_PRIOR if ctrl => select(hwnd, st, (st.page + n - 1) % n),
                VK_UP => set_scroll(hwnd, st, st.scroll - line),
                VK_DOWN => set_scroll(hwnd, st, st.scroll + line),
                VK_PRIOR => set_scroll(hwnd, st, st.scroll - (view - line).max(line)),
                VK_NEXT | VK_SPACE => set_scroll(hwnd, st, st.scroll + (view - line).max(line)),
                VK_HOME => set_scroll(hwnd, st, 0),
                VK_END => set_scroll(hwnd, st, st.height),
                _ if ctrl && vk == b'C' as u32 => {
                    if let Some(page) = st.pages.get(st.page) {
                        let _ = clipboard::set_text(hwnd, &page_text(page));
                    }
                }
                _ => {}
            }
            return 0;
        }
        // 字元訊息不處理（一般視窗不會發出提示音）
        WM_CHAR => return 0,
        WM_SETCURSOR => {
            if swallow_modal_click(hwnd, lparam) {
                return 1;
            }
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let _ = HELP.compare_exchange(hwnd, 0, Ordering::Relaxed, Ordering::Relaxed);
            let st = Box::from_raw(st as *mut State);
            st.fonts.destroy();
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
        let buf = theme::Buffered::new(hdc, rect);
        let dc = buf.hdc;
        let t = app::theme();
        let p = &t.pal;
        let f = &st.fonts;
        theme::fill(dc, &rect, p.surface);
        // 左邊的分類：側欄的底色（同設定視窗）
        let nav = RECT { right: st.k(NAV_W), ..rect };
        theme::fill(dc, &nav, p.sidebar);
        let body = theme::blend(p.ink, p.surface, 0.82);
        let dy = -st.scroll;
        let shift = |r: &RECT| RECT { top: r.top + dy, bottom: r.bottom + dy, ..*r };
        let visible = |r: &RECT| r.bottom + dy >= 0 && r.top + dy <= rect.bottom;
        let wrap = DT_LEFT | DT_WORDBREAK;
        for item in &st.items {
            match item {
                Item::Title(text, r) if visible(r) => theme::draw_text(dc, text, &shift(r), f.title, p.ink, DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS),
                Item::Section(text, r) if visible(r) => theme::draw_text(dc, text, &shift(r), f.section, p.ink, wrap),
                Item::Text(text, r) if visible(r) => theme::draw_text(dc, text, &shift(r), f.body, body, wrap),
                Item::Note(text, r) if visible(r) => theme::draw_text(dc, text, &shift(r), f.small, p.muted, wrap),
                Item::Bullet(text, r) if visible(r) => {
                    let r = shift(r);
                    let d = st.k(5);
                    let line = theme::line_height(f.body);
                    let dot = RECT::new(r.left - st.k(14), r.top + (line - d) / 2, d, d);
                    theme::circle(dc, &dot, Some(p.accent), None);
                    theme::draw_text(dc, text, &r, f.body, body, wrap);
                }
                Item::Key { key, chip, desc, text } if visible(chip) || visible(text) => {
                    let chip = shift(chip);
                    theme::round_rect(dc, &chip, st.k(8), Some(p.chip), Some(p.line));
                    let inner = RECT { left: chip.left + st.k(8), right: chip.right - st.k(8), ..chip };
                    theme::draw_text(dc, key, &inner, f.chip, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    theme::draw_text(dc, desc, &shift(text), f.body, p.ink, wrap);
                }
                _ => {}
            }
        }
    }
    EndPaint(hwnd, &ps);
}

// ---------------------------------------------------------------- 內容

/// 說明頁的內容（依目前的設定與快速鍵表）
pub fn pages(settings: &Settings, keymap: &Keymap) -> Vec<Page> {
    let s = |v: &str| v.to_string();
    let rows = |list: &[(&str, &str)]| list.iter().map(|(k, d)| (k.to_string(), d.to_string())).collect::<Vec<_>>();
    let mut pages = Vec::new();

    // ---- 快速鍵
    let mut blocks = vec![Block::Text(s(tr!(
        "在清單或側欄直接打字，可以跳到名稱開頭相符的項目。下面的快速鍵依目前的設定列出；在「設定 › 快速鍵」點一下命令、再直接按鍵盤就能改。",
        "Type in the list or the sidebar to jump to an item whose name starts with what you type. The shortcuts below reflect your current settings; to change one, click it in Settings › Shortcuts and press the new keys."
    )))];
    for (section, list) in keymap.help_sections() {
        blocks.push(Block::Section(s(section.title())));
        blocks.push(Block::Keys(list));
    }
    blocks.push(Block::Section(s(tr!("快速預覽", "Quick preview"))));
    let menu_row = (tr!("右鍵 › 快速預覽", "Right-click › Quick preview"), tr!("預覽選取的項目並維持開著，空白鍵或 Esc 關閉", "Preview the selected item and keep it open; Space or Esc closes it"));
    let preview_rows: Vec<(&str, &str)> = if !settings.preview {
        vec![menu_row]
    } else if settings.space_preview {
        vec![(tr!("滑鼠停在項目上＋按住空白鍵", "Hover over an item + hold Space"), tr!("預覽滑鼠下的項目，放開就關閉", "Preview the item under the mouse; release to close")), menu_row]
    } else {
        vec![
            (tr!("滑鼠停在項目上", "Hover over an item"), tr!("在旁邊預覽圖片、文字、程式碼、PDF 與影片，移開就關閉", "Preview images, text, code, PDFs and videos next to it; move away to close")),
            (tr!("按住空白鍵", "Hold Space"), tr!("放大預覽（懸停或選取的項目），放開就關閉；按住時方向鍵換下一個", "Enlarge the preview (the hovered or selected item); release to close. Arrow keys move to the next item while holding")),
            menu_row,
        ]
    };
    blocks.push(Block::Keys(rows(&preview_rows)));
    blocks.push(Block::Section(s(tr!("滑鼠", "Mouse"))));
    blocks.push(Block::Keys(rows(&[
        (tr!("滑鼠側鍵", "Mouse side buttons"), tr!("上一頁／下一頁", "Back/forward")),
        (tr!("分頁上按中鍵", "Middle-click a tab"), tr!("關閉分頁", "Close the tab")),
        (tr!("資料夾上按中鍵", "Middle-click a folder"), tr!("在新分頁開啟，不切過去（按住 Shift 時切過去）；清單、側欄與路徑列都可以", "Open it in a new tab without switching to it (hold Shift to switch); works in the list, the sidebar and the path bar")),
        (tr!("拖曳側欄的標籤標題", "Drag a sidebar section heading"), tr!("調整「常用位置」「磁碟」與自訂標籤的順序", "Reorder Favorites, Drives and your own sections")),
        (tr!("拖曳分頁", "Drag a tab"), tr!("調整順序；雙欄時也可以拖到另一欄", "Reorder tabs; in dual-pane mode you can drag to the other pane")),
        (tr!("拖曳空白處", "Drag in empty space"), tr!("框選多個項目（Ctrl 反轉、Shift 加入原本的選取）", "Select several items with a box (Ctrl inverts, Shift adds)")),
    ])));
    pages.push(Page { title: s(tr!("快速鍵", "Shortcuts")), glyph: glyph::KEYBOARD, blocks });

    // ---- 搜尋
    let syntax = feather_core::searchquery::syntax_rows().into_iter().map(|(k, d)| (k.to_string(), d.to_string())).collect();
    pages.push(Page {
        title: s(tr!("搜尋", "Search")),
        glyph: glyph::SEARCH,
        blocks: vec![
            Block::Text(s(tr!(
                "按 Ctrl+F 或右上角的放大鏡，輸入檔名後按 Enter。預設搜尋目前的資料夾（勾選「含子資料夾」時連子資料夾一起）；勾選搜尋框裡的「全域搜尋」會搜尋整台電腦的本機與卸除式磁碟。只比對檔名，不讀取檔案內容；結果邊找邊出現，按 Esc 取消。",
                "Press Ctrl+F or the magnifier at the top right, type a file name and press Enter. Search covers the current folder (and its subfolders when “Include subfolders” is checked); check “Whole PC” in the search box to search every local and removable drive. Only file names are matched, not contents. Results appear as they're found; press Esc to cancel."
            ))),
            Block::Section(s(tr!("最近搜尋", "Recent searches"))),
            Block::Text(s(tr!(
                "在搜尋框按 ↓ 列出最近 10 次搜尋，連篩選條件（類型、日期、大小）與範圍（整台電腦、含不含子資料夾）一起記；選一個就照樣再搜一次。最下面的「清除最近搜尋」可以清掉紀錄。搜尋框空著時的淡色提示會寫出這次搜尋的範圍，例如「在「下載」及子資料夾中搜尋…」。",
                "Press ↓ in the search box to list your last 10 searches, remembered with their filters (type, date, size) and scope (whole PC, with or without subfolders); pick one to run it again. “Clear recent searches” at the bottom clears the list. While the box is empty, its hint shows where the search will look, such as “Search “Downloads” and its subfolders…”."
            ))),
            Block::Section(s(tr!("篩選條件", "Filters"))),
            Block::Text(s(tr!(
                "篩選籤列可以只看資料夾、圖片、文件、影音或壓縮檔；後面的「日期」可選今天、昨天、本週、本月、今年或自訂範圍，「大小」可選小於 1 MB、1～100 MB、100 MB～1 GB、大於 1 GB 或自訂。選了的條件顯示在按鈕上，按 × 清除，底部狀態列的「清除篩選」一次清掉。條件一改，現有的結果馬上跟著篩選，不必重新搜尋；有大小條件時不顯示資料夾。",
                "The filter row shows only folders, images, documents, media or archives. After it, “Date” offers today, yesterday, this week, this month, this year or a custom range, and “Size” offers under 1 MB, 1–100 MB, 100 MB–1 GB, over 1 GB or custom. A chosen condition shows on its button; click × to clear it, or “Clear filters” in the status bar to clear them all. Changing a condition filters the current results right away, without searching again. Folders are hidden while a size condition is set."
            ))),
            Block::Text(s(tr!(
                "搜尋框空著也可以搜尋，只要選了篩選條件：例如在「下載」選「文件」「本週」後按 Enter，就列出這週的文件。只寫條件、不寫檔名也可以（例如 ext:pdf dm:thisweek）。",
                "You can search with an empty box as long as a filter is set: for example, in Downloads pick “Documents” and “This week”, then press Enter to list this week's documents. Conditions alone work too, with no name (for example ext:pdf dm:thisweek)."
            ))),
            Block::Section(s(tr!("搜尋語法", "Search syntax"))),
            Block::Text(s(tr!("在搜尋框輸入這些條件，可以和篩選籤一起用。冒號也可以用全形的「：」。", "Type these in the search box; they combine with the filters. A full-width colon works too."))),
            Block::Keys(syntax),
            Block::Section(s(tr!("排序", "Sorting"))),
            Block::Text(s(tr!(
                "搜尋結果預設依相關性排序：檔名（不算副檔名）完全相同的最前面，再來是開頭相同、在詞的開頭出現、中間有出現；同一級時路徑淺的、比較新的在前。結果邊找邊照順序插進去，選取和捲動位置不會跳。按欄位標題改依那一欄排序，「排序 › 相關性」換回來。",
                "Results are sorted by relevance: an exact name (ignoring the extension) first, then names that start with the word, have it at the start of a word, or contain it; within each group, shallower and newer items come first. New results slot into place as they arrive without moving your selection or scroll position. Click a column heading to sort by it; “Sort › Relevance” switches back."
            ))),
            Block::Section(s(tr!("限制", "Limits"))),
            Block::Bullets(vec![
                s(tr!("最多顯示 50,000 個結果。超過時會標示「未完整搜尋」，這時改篩選條件會帶著條件重新搜尋。", "Up to 50,000 results are shown. Beyond that the search is marked incomplete, and changing a filter searches again with the filter applied.")),
                s(tr!("不沿目錄連結（junction、符號連結）往下搜尋。", "Directory links (junctions, symbolic links) aren't followed.")),
                s(tr!("搜尋整台電腦時略過資源回收筒、System Volume Information 與 Windows\\WinSxS。", "Searching the whole PC skips the Recycle Bin, System Volume Information and Windows\\WinSxS.")),
                s(tr!("尚未提供索引搜尋：每次都直接掃描資料夾。", "There's no index yet: every search scans the folders directly.")),
            ]),
        ],
    });

    // ---- 快速預覽
    let mode = if !settings.preview {
        tr!("快速預覽目前關閉（設定 › 預覽）。右鍵選單的「快速預覽」照樣可以用。", "Quick preview is off (Settings › Preview). “Quick preview” in the right-click menu still works.")
    } else if settings.space_preview {
        tr!("目前是空白鍵預覽：滑鼠停在項目上、按住空白鍵就預覽，放開就關閉。", "Preview with Space is on: hover over an item and hold Space to preview it; release to close.")
    } else {
        tr!("目前是懸停預覽：滑鼠停在項目上約半秒，旁邊就出現預覽，移開就關閉；按住空白鍵放大到視窗中央，按住時方向鍵換下一個。", "Hover preview is on: rest the mouse on an item for about half a second to see a preview beside it; move away to close. Hold Space to enlarge it to the middle of the window, and use the arrow keys to move to the next item while holding.")
    };
    pages.push(Page {
        title: s(tr!("快速預覽", "Quick preview")),
        glyph: glyph::PREVIEW,
        blocks: vec![
            Block::Text(s(mode)),
            Block::Text(s(tr!(
                "在設定 › 預覽可以換成「空白鍵預覽」（只有按住空白鍵才預覽），或取消「啟用快速預覽」。右鍵選單的「快速預覽」在畫面中間開啟並維持開著，按空白鍵或 Esc 關閉。",
                "In Settings › Preview you can switch to “Preview with Space” (preview only while holding Space) or turn quick preview off. “Quick preview” in the right-click menu opens in the middle of the window and stays open until you press Space or Esc."
            ))),
            Block::Section(s(tr!("可以預覽的檔案", "What can be previewed"))),
            Block::Keys(rows(&[
                (tr!("圖片", "Images"), tr!("常見格式都可以；WebP、HEIC 等只要系統裝有解碼器也可以", "Common formats; WebP, HEIC and others when Windows has the codec")),
                (tr!("影片", "Video"), tr!("縮圖、解析度與長度", "Thumbnail, resolution and length")),
                ("PDF", tr!("第一頁與總頁數", "First page and page count")),
                (tr!("文字與程式碼", "Text and code"), tr!("自動判斷 UTF-8、UTF-16 或 Big5，附行號，二十多種語言有語法上色", "Detects UTF-8, UTF-16 or Big5, with line numbers and syntax highlighting for 20+ languages")),
                (tr!("Office 文件", "Office files"), tr!("Word、Excel、PowerPoint、OpenDocument：不需要安裝 Office，直接讀出段落、標題、清單、投影片與表格", "Word, Excel, PowerPoint, OpenDocument: no Office needed; paragraphs, headings, lists, slides and tables are read directly")),
            ])),
            Block::Note(s(tr!(
                "預覽 Office 文件時只讀取文字與表格，不會執行巨集或任何內嵌內容。雲端尚未下載的檔案不會因為預覽而被下載。",
                "Office previews only read text and tables; macros and embedded content are never run. Cloud files that aren't downloaded yet won't be downloaded just to preview them."
            ))),
        ],
    });

    // ---- 檔案與資料夾
    pages.push(Page {
        title: s(tr!("檔案與資料夾", "Files and folders")),
        glyph: glyph::FOLDER,
        blocks: vec![
            Block::Section(s(tr!("拖放", "Drag and drop"))),
            Block::Text(s(tr!(
                "檔案與資料夾可以直接拖到資料夾、另一欄、側欄的資料夾或磁碟。規則和檔案總管一樣：同一個磁碟移動、不同磁碟複製；按住 Ctrl 複製、Shift 移動、Alt 建立捷徑，用右鍵拖曳會跳出選單。拖到分頁上停一下就切換到那個分頁。把資料夾拖到側欄的「常用位置」或自訂標籤的標題，就會釘選在那個標籤裡。",
                "Drag files and folders onto a folder, the other pane, or a folder or drive in the sidebar. The rules match File Explorer: the same drive moves, another drive copies; hold Ctrl to copy, Shift to move or Alt to create a shortcut, or drag with the right button for a menu. Pause over a tab to switch to it. Drag a folder onto the sidebar's “Favorites” heading, or one of your own section headings, to pin it there."
            ))),
            Block::Section(s(tr!("側欄的標籤", "Sidebar sections"))),
            Block::Bullets(vec![
                s(tr!("在側欄的空白處或標籤的標題按右鍵，選「新增標籤…」，就多一個和「常用位置」「磁碟」同一層的標籤（例如「工作」「照片」）。", "Right-click empty space in the sidebar or a section heading and choose “New section…” to add a section alongside Favorites and Drives (for example Work or Photos).")),
                s(tr!("把資料夾拖到標籤的標題上就放進那個標籤；已經釘選的資料夾按右鍵「移到其他標籤…」可以換位置。", "Drag a folder onto a section heading to add it there; right-click a pinned folder and choose “Move to another section…” to move it.")),
                s(tr!("點標籤的標題收起或展開（下次開啟時照樣）；收起來時標題後面有「›」。", "Click a section heading to collapse or expand it (it stays that way next time); a collapsed section shows “›” after its name.")),
                s(tr!("按住標籤的標題上下拖曳可以調整順序（常用位置與磁碟也可以），也可以在標題按右鍵選「往上移」「往下移」。", "Drag a section heading up or down to reorder it (Favorites and Drives too), or right-click it and choose “Move up” or “Move down”.")),
                s(tr!("自訂標籤可以重新命名、刪除；刪除時裡面的資料夾移回「常用位置」，資料夾本身不會刪除。", "Your own sections can be renamed or deleted; deleting one moves its folders back to Favorites, and the folders themselves aren't deleted.")),
            ]),
            Block::Section(s(tr!("刪除與復原", "Delete and undo"))),
            Block::Bullets(vec![
                s(tr!("Delete 送到資源回收筒；要不要先確認照 Windows 資源回收筒「內容」裡的設定。", "Delete sends items to the Recycle Bin; whether it asks first follows the Recycle Bin's own setting in Windows.")),
                s(tr!("Shift+Delete 永久刪除，一定先確認，無法復原。", "Shift+Delete deletes permanently; it always asks first and can't be undone.")),
                s(tr!("Ctrl+Z 復原最近 30 個重新命名、移動與刪除；原位置已經有同名檔案時不會覆蓋它。", "Ctrl+Z undoes your last 30 renames, moves and deletes; it never overwrites a file of the same name.")),
            ]),
            Block::Section(s(tr!("路徑與網路位置", "Paths and network locations"))),
            Block::Text(s(tr!(
                "按路徑列的任何一層直接前往，按每一層右邊的箭頭列出那一層的子資料夾。Ctrl+E 改成輸入框，可以輸入完整路徑、相對路徑（..、子資料夾名稱）或檔案路徑（開啟所在資料夾並選取它）。輸入 \\\\電腦名稱 列出那台電腦或 NAS 分享的資料夾，需要帳號密碼時會跳出 Windows 的登入視窗。",
                "Click any part of the path bar to go there, or the arrow beside it to list its subfolders. Ctrl+E turns it into a text box for a full path, a relative path (.., a subfolder name) or a file path (opens its folder and selects it). Type \\\\computer-name to list what that PC or NAS shares; Windows asks for a user name and password when needed."
            ))),
            Block::Text(s(tr!(
                "路徑列也可以像 Win+R 一樣執行：程式、批次檔、捷徑（.exe、.bat、.cmd、.lnk…）直接執行；不含路徑的指令（cmd、notepad、cmd /k dir）和網址交給 Windows 執行，工作資料夾是目前的資料夾（在這裡輸入 cmd 就在這裡開命令提示字元）。shell:startup、shell:downloads 這類位置會解析後開啟，控制台等不在檔案系統裡的位置交給 Windows 開。",
                "The path bar can also run things like Win+R: programs, batch files and shortcuts (.exe, .bat, .cmd, .lnk…) run directly; commands without a path (cmd, notepad, cmd /k dir) and web addresses are handed to Windows, starting in the current folder (type cmd to open a command prompt right here). Locations such as shell:startup or shell:downloads are resolved and opened; ones outside the file system, such as Control Panel, open in Windows."
            ))),
            Block::Section(s(tr!("右鍵選單", "Right-click menu"))),
            Block::Text(s(tr!(
                "除了 Stillfeather 自己的命令，也有已安裝程式加入的項目（在終端中開啟、7-Zip、Git、傳送到…），不必再按「顯示其他選項」。資料夾、空白處或側欄按右鍵可以在那裡開啟 PowerShell 視窗。",
                "Besides Stillfeather's own commands, the menu includes items added by installed programs (Open in Terminal, 7-Zip, Git, Send to…), with no “Show more options” step. Right-click a folder, empty space or the sidebar to open a PowerShell window there."
            ))),
        ],
    });

    // ---- 檢視與版面
    pages.push(Page {
        title: s(tr!("檢視與版面", "View and layout")),
        glyph: glyph::VIEW_DETAILS,
        blocks: vec![
            Block::Text(s(tr!(
                "標題右側的「排序」「檢視」可以換排序方式與檢視模式。檢視模式記在各個資料夾，「檢視 › 套用到所有資料夾」可以統一。顯示隱藏的項目、副檔名與精簡檢視在「檢視 › 顯示」，是整體設定，會一直記住。",
                "Use “Sort” and “View” to the right of the title to change the sort order and view. Each folder remembers its view (“View › Apply to all folders” makes them all the same). Hidden items, file name extensions and compact view are under “View › Show”; they apply everywhere and are always remembered."
            ))),
            Block::Text(s(tr!(
                "「檢視 › 文字大小」（或設定 › 文字與語言）可以把檔名、欄位、側欄、分頁、路徑列、篩選籤、選單與對話方塊的字放大到 150%。",
                "“View › Text size” (or Settings › Text & language) enlarges file names, columns, the sidebar, tabs, the path bar, filters, menus and dialogs up to 150%."
            ))),
            Block::Section(s(tr!("分頁與雙欄", "Tabs and panes"))),
            Block::Text(s(tr!(
                "左右兩欄各自有分頁和上一頁／下一頁紀錄；Ctrl+\\ 切換單欄或雙欄，檔案可以直接在兩欄之間拖放，或用「複製到另一欄」。關閉時的分頁下次開啟時會照樣開回來（設定 › 檔案瀏覽 可以關掉）。",
                "Each pane has its own tabs and back/forward history; Ctrl+\\ toggles dual-pane mode, and you can drag files between panes or use “Copy to the other pane”. Your tabs come back the next time you open Stillfeather (turn this off in Settings › Browsing)."
            ))),
        ],
    });

    // ---- 關於
    pages.push(Page {
        title: s(tr!("關於", "About")),
        glyph: glyph::HELP_CIRCLE,
        blocks: vec![
            Block::Text(format!("Stillfeather {} · {}", env!("CARGO_PKG_VERSION"), tr!("Rust／原生 Win32 版", "Rust / native Win32"))),
            Block::Text(s(tr!(
                "輕巧、安靜的 Windows 檔案總管：分頁與雙欄、快速預覽、檔名搜尋，和檔案總管一樣的右鍵選單與拖放。",
                "A light, quiet file manager for Windows: tabs and dual panes, quick preview, file name search, and the same right-click menu and drag-and-drop rules as File Explorer."
            ))),
            Block::Note(s(tr!("以 GPL-3.0 授權發佈。原始碼：github.com/Pity0125/Stillfeather", "Released under the GPL-3.0 license. Source: github.com/Pity0125/Stillfeather"))),
        ],
    });
    pages
}

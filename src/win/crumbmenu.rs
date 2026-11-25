//! 麵包屑的子資料夾選單：點路徑段右邊的箭頭，列出那一層底下的資料夾；選一個就在目前分頁開啟
//! （一般的導覽，按「上一頁」回到原位置）。目前位置經過的那個資料夾會標示出來並預先選好。
//!
//! - 展開時才在背景執行緒讀取（`fs::menu_folders`），順序與隱藏項目跟清單相同。很快讀好就直接顯示內容，
//!   比較久（例如慢速或離線的網路磁碟）先顯示「讀取中…」；這段時間照樣可以關閉選單、切換分頁。
//!   同一個資料夾還在讀取時再展開，就等同一個結果，不會一直多開執行緒。
//! - 和快速預覽一樣不搶焦點：選單開著時，主視窗的鍵盤處理（`App::handle_key`）把方向鍵、Enter、Esc
//!   與打字跳到項目轉給這裡（見 [`key`]、[`type_char`]）。
//! - 點選單外面、視窗失去作用、移動或縮放、換了分頁或資料夾時關閉（見 app.rs 的 `close_crumb_menu`）。

use super::anim;
use super::app;
use super::fs::{self, FsError, MenuFolders};
use super::sys::*;
use super::theme::{self, Buffered, Theme};
use super::util::*;
use feather_core::paths;
use feather_core::typeahead::{self, TypeAhead};
use feather_core::workspace::TabId;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const MENU_CLASS: &str = "FeatherFiles.CrumbMenu";
const SHOW_TIMER: usize = 1;
/// 讀取超過這個時間才先顯示「讀取中…」；本機磁碟通常更快就讀好，選單直接帶著內容出現，不閃一下
const LOADING_DELAY_MS: u32 = 120;
/// 一次最多顯示幾列，其餘捲動
const MAX_ROWS: usize = 18;

/// 要開啟的選單
pub struct Request {
    /// 第幾個路徑段（麵包屑把那一段的箭頭畫成朝下）
    pub segment: usize,
    pub tab: TabId,
    /// 要列出子資料夾的資料夾
    pub path: String,
    /// 目前位置經過的子資料夾名稱（標示用；最後一段沒有）
    pub current: Option<String>,
    /// 箭頭的螢幕範圍：選單出現在它下方
    pub anchor: RECT,
    pub show_hidden: bool,
    pub sort_col: usize,
    pub desc: bool,
}

enum Content {
    Loading,
    Folders { names: Vec<String>, truncated: bool },
    Failed(String),
}

struct Menu {
    req: Request,
    token: u64,
    content: Content,
    /// 反白的項目（滑鼠所在或方向鍵選到的）
    sel: Option<usize>,
    /// 捲動：第一個顯示的項目
    top: usize,
    /// 顯示幾列項目（不含最下面的說明列）
    rows: usize,
    row_h: i32,
    pad: i32,
    typed: TypeAhead,
    shown: bool,
}

/// 背景讀取中的資料夾
struct Pending {
    key: (String, bool, usize, bool),
    token: u64,
    cancel: Arc<AtomicBool>,
}

pub struct State {
    pub popup: HWND,
    menu: Option<Menu>,
    next_token: u64,
    pending: Vec<Pending>,
}

impl State {
    pub fn new(popup: HWND) -> State {
        State { popup, menu: None, next_token: 0, pending: Vec::new() }
    }
}

/// 選單開著時按鍵的處理結果
pub enum Key {
    /// 選單用掉了
    Handled,
    /// 開啟這個資料夾
    Pick(String),
    /// 關閉選單
    Close,
    /// 關閉選單，按鍵照常處理（切換分頁、上一頁等）
    Pass,
    /// 可以打的字：交給 TranslateMessage 變成 WM_CHAR，再由 [`type_char`] 跳到項目
    Char,
    /// 換成左邊（-1）或右邊（+1）那一段的選單
    Neighbor(isize),
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(MENU_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DROPSHADOW,
        lpfnWndProc: menu_proc,
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

pub fn create_popup(owner: HWND) -> HWND {
    let class = wide(MENU_CLASS);
    let name = wide(tr!("子資料夾", "Subfolders"));
    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            name.as_ptr(),
            WS_POPUP,
            0,
            0,
            10,
            10,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        let round: u32 = DWMWCP_ROUNDSMALL;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const u32 as *const std::ffi::c_void, 4);
        hwnd
    }
}

pub fn is_open(state: &State) -> bool {
    state.menu.is_some()
}

/// 開著的是哪一段的選單
pub fn segment(state: &State) -> Option<usize> {
    state.menu.as_ref().map(|m| m.req.segment)
}

/// 開著的選單屬於哪個分頁、列的是哪個資料夾
pub fn target(state: &State) -> Option<(TabId, &str)> {
    state.menu.as_ref().map(|m| (m.req.tab, m.req.path.as_str()))
}

/// 開啟選單（已經開著別的就換掉）。
pub fn open(state: &mut State, req: Request) {
    close(state);
    state.menu = Some(Menu {
        req,
        token: 0,
        content: Content::Loading,
        sel: None,
        top: 0,
        rows: 0,
        row_h: 0,
        pad: 0,
        typed: TypeAhead::default(),
        shown: false,
    });
    start(state);
    unsafe {
        SetTimer(state.popup, SHOW_TIMER, LOADING_DELAY_MS, std::ptr::null());
    }
}

/// 關閉選單；原本開著就回傳 true。還在讀取的話通知背景執行緒不用讀了。
pub fn close(state: &mut State) -> bool {
    let Some(m) = state.menu.take() else { return false };
    if let Some(p) = state.pending.iter().find(|p| p.token == m.token) {
        p.cancel.store(true, Ordering::Relaxed);
    }
    unsafe {
        KillTimer(state.popup, SHOW_TIMER);
        ShowWindow(state.popup, SW_HIDE);
    }
    true
}

fn start(state: &mut State) {
    let Some(m) = state.menu.as_mut() else { return };
    let key = (m.req.path.to_lowercase(), m.req.show_hidden, m.req.sort_col, m.req.desc);
    // 同一個資料夾還在讀取（例如離線的網路磁碟還在等逾時）：等同一個結果
    if let Some(p) = state.pending.iter().find(|p| p.key == key) {
        p.cancel.store(false, Ordering::Relaxed);
        m.token = p.token;
        return;
    }
    state.next_token += 1;
    let token = state.next_token;
    m.token = token;
    let cancel = Arc::new(AtomicBool::new(false));
    state.pending.push(Pending { key, token, cancel: cancel.clone() });
    let (path, show_hidden, sort_col, desc) = (m.req.path.clone(), m.req.show_hidden, m.req.sort_col, m.req.desc);
    std::thread::spawn(move || {
        let result = fs::menu_folders(&path, show_hidden, sort_col, desc, &cancel);
        app::post(app::WorkerMsg::CrumbMenu { token, result });
    });
}

/// 背景讀取完成。`icon_px`：清單小圖示的邊長。
pub fn on_loaded(state: &mut State, token: u64, result: Result<MenuFolders, FsError>, t: &Theme, icon_px: i32) {
    state.pending.retain(|p| p.token != token);
    let Some(m) = state.menu.as_mut().filter(|m| m.token == token) else { return };
    m.content = match result {
        Ok(listing) => {
            let names: Vec<String> = listing.folders.into_iter().map(|e| e.name).collect();
            m.sel = m.req.current.as_deref().and_then(|c| names.iter().position(|n| paths::eq_ignore_case(n, c)));
            Content::Folders { names, truncated: listing.truncated }
        }
        // 關掉後很快又開同一個資料夾，上一次的讀取剛好已經停了：重新讀
        Err(FsError::Cancelled) => {
            start(state);
            return;
        }
        Err(FsError::Message(e)) => Content::Failed(trf!("無法讀取：{e}", "Couldn't read: {e}")),
    };
    unsafe {
        KillTimer(state.popup, SHOW_TIMER);
    }
    reveal(state, t, icon_px);
}

/// 讀取比較久：先顯示「讀取中…」。
fn on_show_timer(state: &mut State, t: &Theme, icon_px: i32) {
    unsafe {
        KillTimer(state.popup, SHOW_TIMER);
    }
    if state.menu.as_ref().is_some_and(|m| !m.shown) {
        reveal(state, t, icon_px);
    }
}

fn names(m: &Menu) -> &[String] {
    match &m.content {
        Content::Folders { names, .. } => names,
        _ => &[],
    }
}

/// 最下面的說明列（讀取中、沒有子資料夾、讀取失敗、只列出一部分）
fn note(m: &Menu) -> Option<String> {
    match &m.content {
        Content::Loading => Some(tr!("讀取中…", "Loading…").into()),
        Content::Failed(e) => Some(e.clone()),
        Content::Folders { names, .. } if names.is_empty() => Some(tr!("沒有子資料夾", "No subfolders").into()),
        Content::Folders { truncated: true, .. } => {
            Some(trf!("僅列出前 {} 個資料夾", "Only the first {} folders are listed", feather_core::format::group_digits(fs::MENU_MAX_FOLDERS as u64)))
        }
        Content::Folders { .. } => None,
    }
}

fn work_area(r: &RECT) -> RECT {
    let pt = POINT { x: r.left, y: r.bottom };
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: RECT::default(), rcWork: RECT::default(), dwFlags: 0 };
        if GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi) != 0 {
            mi.rcWork
        } else {
            RECT { left: pt.x - 800, top: pt.y - 600, right: pt.x + 800, bottom: pt.y + 600 }
        }
    }
}

/// 依內容決定大小與位置並顯示（不搶焦點）。
fn reveal(state: &mut State, t: &Theme, icon_px: i32) {
    let popup = state.popup;
    let Some(m) = state.menu.as_mut() else { return };
    let f = &t.fonts;
    m.row_h = t.s(30).max(f.body_height + t.s(12));
    m.pad = t.s(5);
    let note = note(m);
    let count = names(m).len();
    let (text_w, note_w) = unsafe {
        let hdc = GetDC(popup);
        // 名稱很多時只量前面一部分：寬度有上限，量全部只是浪費時間
        let text_w = names(m).iter().take(2_000).map(|n| theme::text_width(hdc, n, f.body_bold)).max().unwrap_or(0);
        let note_w = note.as_deref().map(|n| theme::text_width(hdc, n, f.body_small)).unwrap_or(0);
        ReleaseDC(popup, hdc);
        (text_w, note_w)
    };
    let lead = lead_px(t, icon_px);
    let width = (lead + text_w.max(note_w) + t.s(24)).clamp(t.s(220), t.s(460));
    let anchor = m.req.anchor;
    let work = work_area(&anchor);
    let top = anchor.bottom + t.s(4);
    let note_rows = note.is_some() as i32;
    let room = ((work.bottom - t.s(8) - top - 2 * m.pad) / m.row_h - note_rows).max(1) as usize;
    m.rows = count.min(MAX_ROWS).min(room);
    m.top = scroll_to(m.top, m.sel, m.rows, count);
    let height = 2 * m.pad + (m.rows as i32 + note_rows) * m.row_h;
    let x = (anchor.left - t.s(6)).min(work.right - width).max(work.left);
    m.shown = true;
    unsafe {
        let border = theme::blend(t.pal.ink, t.pal.surface, 0.14);
        DwmSetWindowAttribute(popup, DWMWA_BORDER_COLOR, &border as *const COLORREF as *const std::ffi::c_void, 4);
        // 放到最上層（選單屬於主視窗：一定在主視窗上面，切到別的程式時跟著被蓋住）
        SetWindowPos(popup, 0, x, top, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
    invalidate(popup);
}

/// 項目文字的左邊界：選單邊緣到圖示的留白、圖示與間距（圖示畫在 `pad` + 9 的位置）
fn lead_px(t: &Theme, icon_px: i32) -> i32 {
    t.s(14) + icon_px + t.s(10)
}

/// 讓 `sel` 在可見範圍內的捲動位置
fn scroll_to(top: usize, sel: Option<usize>, rows: usize, count: usize) -> usize {
    let max_top = count.saturating_sub(rows);
    let top = top.min(max_top);
    match sel {
        Some(s) if rows > 0 && s < top => s,
        Some(s) if rows > 0 && s >= top + rows => (s + 1 - rows).min(max_top),
        _ => top,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Move {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// 方向鍵等移動反白（上下到頭會繞回另一端，同原生選單）
fn step(sel: Option<usize>, count: usize, how: Move, page: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let page = page.max(1);
    Some(match (how, sel) {
        (Move::Up, None) | (Move::End, _) => last,
        (Move::Down, None) | (Move::Home, _) => 0,
        (Move::Up, Some(s)) => {
            if s == 0 {
                last
            } else {
                (s - 1).min(last)
            }
        }
        (Move::Down, Some(s)) => {
            if s >= last {
                0
            } else {
                s + 1
            }
        }
        (Move::PageUp, s) => s.unwrap_or(0).saturating_sub(page),
        (Move::PageDown, s) => (s.unwrap_or(0) + page).min(last),
    })
}

fn set_sel(state: &mut State, sel: Option<usize>) {
    let popup = state.popup;
    let Some(m) = state.menu.as_mut() else { return };
    if m.sel != sel {
        m.sel = sel;
        m.top = scroll_to(m.top, sel, m.rows, names(m).len());
        invalidate(popup);
    }
}

fn picked(m: &Menu, index: usize) -> Option<String> {
    names(m).get(index).map(|n| paths::join(&m.req.path, n))
}

/// 選單開著時的按鍵（WM_KEYDOWN；`vk` 已換成輸入法之前的按鍵）。
pub fn key(state: &mut State, vk: u32, ctrl: bool, alt: bool) -> Key {
    let Some(m) = state.menu.as_ref() else { return Key::Pass };
    let count = names(m).len();
    // Alt 組合鍵（上一頁、Alt+F4 等）與切換、開關分頁、編輯路徑：關掉選單後照常處理
    if alt || vk == VK_BROWSER_BACK || vk == VK_BROWSER_BACK + 1 {
        return Key::Pass;
    }
    if ctrl {
        let pass = vk == VK_TAB || vk == VK_PRIOR || vk == VK_NEXT || matches!(char::from_u32(vk), Some('T' | 'N' | 'W' | 'L' | 'F'));
        return if pass { Key::Pass } else { Key::Handled };
    }
    let how = match vk {
        VK_ESCAPE => return Key::Close,
        VK_RETURN => {
            return match m.sel.and_then(|i| picked(m, i)) {
                Some(path) => Key::Pick(path),
                None if matches!(m.content, Content::Loading) => Key::Handled,
                None if count == 0 => Key::Close,
                None => Key::Handled,
            };
        }
        VK_LEFT => return Key::Neighbor(-1),
        VK_RIGHT => return Key::Neighbor(1),
        VK_TAB => return Key::Pass,
        VK_UP => Move::Up,
        VK_DOWN => Move::Down,
        VK_HOME => Move::Home,
        VK_END => Move::End,
        VK_PRIOR => Move::PageUp,
        VK_NEXT => Move::PageDown,
        // 可以打的字（字母、數字、標點；不含空白鍵）：變成 WM_CHAR 後跳到項目
        0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF => return Key::Char,
        // 其他按鍵（Delete、F2、空白鍵等）不要作用在後面的清單上
        _ => return Key::Handled,
    };
    let sel = step(m.sel, count, how, m.rows.saturating_sub(1));
    set_sel(state, sel);
    Key::Handled
}

/// 選單開著時打的字：跳到名稱以這些字開頭的資料夾。
pub fn type_char(state: &mut State, c: char) {
    let Some(m) = state.menu.as_mut() else { return };
    let query = m.typed.push(c, anim::now());
    let found = {
        let names = names(m);
        typeahead::find(names.len(), m.sel, &query, |i| names.get(i).cloned())
    };
    if found.is_some() {
        set_sel(state, found);
    }
}

/// 滾輪捲動（`delta` 同 WM_MOUSEWHEEL，正值往上）。
pub fn wheel(state: &mut State, delta: i32) {
    let popup = state.popup;
    let Some(m) = state.menu.as_mut() else { return };
    let count = names(m).len();
    let lines = -(delta as f64 / 120.0 * 3.0).round() as isize;
    let max_top = count.saturating_sub(m.rows) as isize;
    let top = (m.top as isize + lines).clamp(0, max_top) as usize;
    if top != m.top {
        m.top = top;
        invalidate(popup);
    }
}

/// 選單裡第幾個項目在 y（用戶區座標）
fn row_at(m: &Menu, y: i32) -> Option<usize> {
    if m.row_h <= 0 || y < m.pad {
        return None;
    }
    let row = ((y - m.pad) / m.row_h) as usize;
    (row < m.rows).then_some(m.top + row).filter(|&i| i < names(m).len())
}

/// 清單裡資料夾的圖示（影像清單、索引、寬、高；高度含上下留白）
struct FolderIcon {
    list: HIMAGELIST,
    index: i32,
    width: i32,
    height: i32,
}

unsafe fn paint(state: &State, t: &Theme, icon: &FolderIcon, hdc: HDC, rect: RECT) {
    let p = &t.pal;
    let f = &t.fonts;
    theme::fill(hdc, &rect, p.surface);
    let Some(m) = state.menu.as_ref() else { return };
    let names = names(m);
    let lead = lead_px(t, icon.width);
    let hover = theme::blend(p.ink, p.surface, if p.dark { 0.10 } else { 0.07 });
    for (k, i) in (m.top..(m.top + m.rows).min(names.len())).enumerate() {
        let y = m.pad + k as i32 * m.row_h;
        let row = RECT { left: m.pad, top: y, right: rect.right - m.pad, bottom: y + m.row_h };
        if m.sel == Some(i) {
            theme::round_rect(hdc, &row, t.s(6), Some(hover), None);
        }
        let name = &names[i];
        let current = m.req.current.as_deref().is_some_and(|c| paths::eq_ignore_case(c, name));
        if current {
            // 目前位置經過的資料夾：左邊一條重點色短線，名稱用粗體
            let bh = t.s(16).min(row.height() - t.s(8));
            let bar = RECT { left: row.left + t.s(3), top: row.top + (row.height() - bh) / 2, right: row.left + t.s(6), bottom: row.top + (row.height() - bh) / 2 + bh };
            theme::round_rect(hdc, &bar, t.s(3), Some(p.accent), None);
        }
        if icon.index >= 0 && icon.list != 0 {
            (comctl().image_list_draw)(icon.list, icon.index, hdc, row.left + t.s(9), row.top + (row.height() - icon.height) / 2, ILD_TRANSPARENT);
        }
        let tr = RECT { left: lead, right: row.right - t.s(8), ..row };
        let font = if current { f.body_bold } else { f.body };
        theme::draw_text(hdc, name, &tr, font, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    if let Some(n) = note(m) {
        let y = m.pad + m.rows as i32 * m.row_h;
        let left = if names.is_empty() { m.pad + t.s(12) } else { lead };
        let r = RECT { left, top: y, right: rect.right - m.pad - t.s(8), bottom: y + m.row_h };
        theme::draw_text(hdc, &n, &r, f.body_small, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    // 捲動位置：右邊一條細線（用滾輪、方向鍵或打字捲動）
    if names.len() > m.rows && m.rows > 0 {
        let track = m.rows as i32 * m.row_h - t.s(8);
        let thumb = (track * m.rows as i32 / names.len() as i32).max(t.s(18)).min(track);
        let max_top = (names.len() - m.rows) as i32;
        let y = m.pad + t.s(4) + (track - thumb) * m.top as i32 / max_top.max(1);
        let bar = RECT { left: rect.right - t.s(5), top: y, right: rect.right - t.s(2), bottom: y + thumb };
        theme::round_rect(hdc, &bar, t.s(3), Some(theme::blend(p.muted, p.surface, 0.55)), None);
    }
}

unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(a) = app::app_opt() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => return 1,
        // 點選單不搶走清單的焦點（鍵盤仍由主視窗轉來）
        WM_MOUSEACTIVATE => return MA_NOACTIVATE,
        WM_PAINT => {
            let icon = FolderIcon { index: a.icons.folder_index(), list: a.icons.list, width: a.icons.icon_px, height: a.icons.row_px };
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let rect = client_rect(hwnd);
            {
                let buf = Buffered::new(hdc, rect);
                paint(&a.crumb_menu, &a.theme, &icon, buf.hdc, rect);
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_TIMER if wparam == SHOW_TIMER => {
            on_show_timer(&mut a.crumb_menu, &a.theme, a.icons.icon_px);
            return 0;
        }
        WM_MOUSEMOVE => {
            let sel = a.crumb_menu.menu.as_ref().and_then(|m| row_at(m, get_y_lparam(lparam)));
            if sel.is_some() {
                set_sel(&mut a.crumb_menu, sel);
            }
            return 0;
        }
        WM_MOUSEWHEEL => {
            wheel(&mut a.crumb_menu, ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32);
            return 0;
        }
        WM_LBUTTONUP => {
            let path = a.crumb_menu.menu.as_ref().and_then(|m| row_at(m, get_y_lparam(lparam)).and_then(|i| picked(m, i)));
            if let Some(path) = path {
                a.pick_crumb_menu(path);
            }
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_moves_wrap_like_native_menus() {
        assert_eq!(step(None, 0, Move::Down, 5), None);
        assert_eq!(step(None, 4, Move::Down, 5), Some(0));
        assert_eq!(step(None, 4, Move::Up, 5), Some(3));
        assert_eq!(step(Some(3), 4, Move::Down, 5), Some(0));
        assert_eq!(step(Some(0), 4, Move::Up, 5), Some(3));
        assert_eq!(step(Some(1), 4, Move::Down, 5), Some(2));
        assert_eq!(step(Some(2), 10, Move::Home, 5), Some(0));
        assert_eq!(step(Some(2), 10, Move::End, 5), Some(9));
        assert_eq!(step(Some(2), 10, Move::PageDown, 5), Some(7));
        assert_eq!(step(Some(8), 10, Move::PageDown, 5), Some(9));
        assert_eq!(step(Some(3), 10, Move::PageUp, 5), Some(0));
        assert_eq!(step(None, 10, Move::PageDown, 0), Some(1));
    }

    #[test]
    fn selection_stays_visible() {
        // 18 列可見、100 個項目
        assert_eq!(scroll_to(0, Some(5), 18, 100), 0);
        assert_eq!(scroll_to(0, Some(40), 18, 100), 23);
        assert_eq!(scroll_to(30, Some(10), 18, 100), 10);
        assert_eq!(scroll_to(0, Some(99), 18, 100), 82);
        // 項目變少時捲動位置不超過最後一頁
        assert_eq!(scroll_to(90, None, 18, 20), 2);
        assert_eq!(scroll_to(5, None, 0, 0), 0);
    }
}

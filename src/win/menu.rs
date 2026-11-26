//! 下拉選單（排序、檢視、篩選籤的日期與大小、分頁的右鍵選單、設定裡的預覽方式）：原生選單（鍵盤、子選單、
//! 位置都交給 Windows），項目自己畫（MFT_OWNERDRAW），字型跟著「文字大小」放大，顏色跟著 Stillfeather 的主題：
//! 檔案總管那樣的圓點（單選）、勾號、項目前面的圖示、Tab 後面的快速鍵對齊成一欄、子選單箭頭。
//! Windows 自己畫的選單字型是系統的，不會跟著放大，所以改成自己畫。
//!
//! 自己畫的訊息（WM_MEASUREITEM、WM_DRAWITEM）送到選單的擁有者視窗：主視窗與設定視窗的視窗程序先交給
//! [`owner_draw`]。項目的資料在選單開著時放在這個執行緒的 `ROWS`，`itemData` 是 `TAG` 加上索引。
//!
//! 右鍵選單（shellmenu.rs）混有 Windows 的項目（傳送到、內容等），維持 Windows 畫，項目圖示用 [`glyph_bitmap`]。

use super::app;
use super::controls;
use super::sys::*;
use super::theme;
use super::theme::glyph;
use super::util::*;
use std::cell::RefCell;

/// 項目前面的標記
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    /// 圓點：一組中只會有一個（目前的檢視、排序欄位）
    Radio,
    /// 勾號：可以各自開關（副檔名、隱藏的項目）
    Check,
}

pub enum Item<'a> {
    Command { id: u16, text: &'a str, glyph: Option<char>, mark: Mark, on: bool, enabled: bool },
    Submenu { text: &'a str, glyph: Option<char>, items: Vec<Item<'a>> },
    Separator,
}

impl<'a> Item<'a> {
    pub fn radio(id: u16, text: &'a str, on: bool) -> Item<'a> {
        Item::Command { id, text, glyph: None, mark: Mark::Radio, on, enabled: true }
    }

    pub fn check(id: u16, text: &'a str, on: bool) -> Item<'a> {
        Item::Command { id, text, glyph: None, mark: Mark::Check, on, enabled: true }
    }

    pub fn with_glyph(mut self, g: char) -> Item<'a> {
        match &mut self {
            Item::Command { glyph, .. } | Item::Submenu { glyph, .. } => *glyph = Some(g),
            Item::Separator => {}
        }
        self
    }
}

/// 右鍵選單（Windows 畫的）項目圖示的字型、顏色與大小（見 [`glyph_bitmap`]）
pub struct Look {
    pub font: HFONT,
    pub color: COLORREF,
    pub size: i32,
}

/// 圖示字形畫成預乘 alpha 的 32 位元點陣圖（Windows 畫的選單項目的 hbmpItem，右鍵選單用）。
pub(super) unsafe fn glyph_bitmap(g: char, look: &Look) -> HBITMAP {
    let size = look.size.max(8);
    let screen = GetDC(0);
    let dc = CreateCompatibleDC(screen);
    ReleaseDC(0, screen);
    if dc == 0 {
        return 0;
    }
    let bi = BITMAPINFO::top_down_32(size, size);
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbm = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
    if hbm == 0 || bits.is_null() {
        DeleteDC(dc);
        return 0;
    }
    let old = SelectObject(dc, hbm);
    let n = (size * size) as usize;
    std::ptr::write_bytes(bits as *mut u32, 0, n);
    // 黑底白字畫一次，以亮度當作覆蓋率，再換成指定顏色
    let old_font = SelectObject(dc, look.font);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, 0x00FF_FFFF);
    let text: Vec<u16> = g.to_string().encode_utf16().collect();
    let mut r = RECT { left: 0, top: 0, right: size, bottom: size };
    DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old_font);
    GdiFlush();
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, n);
    let (cr, cg, cb) = ((look.color & 0xFF), (look.color >> 8) & 0xFF, (look.color >> 16) & 0xFF);
    for p in px.iter_mut() {
        let cov = ((*p >> 16) & 0xFF).max((*p >> 8) & 0xFF).max(*p & 0xFF);
        *p = (cov << 24) | ((cr * cov / 255) << 16) | ((cg * cov / 255) << 8) | (cb * cov / 255);
    }
    SelectObject(dc, old);
    DeleteDC(dc);
    hbm
}

/// 自己畫的一個項目
struct Row {
    text: String,
    /// Tab 後面的快速鍵（例如「Ctrl+Shift+1」）
    accel: String,
    glyph: Option<char>,
    mark: Mark,
    on: bool,
    enabled: bool,
    submenu: bool,
    separator: bool,
    /// 同一層選單的欄寬（同一層的每個項目都一樣，文字與快速鍵才對得齊）
    cols: Cols,
}

#[derive(Clone, Copy, Default)]
struct Cols {
    glyph: bool,
    text: i32,
    accel: i32,
    submenu: bool,
}

thread_local! {
    /// 開著的選單（含子選單）的項目；選單關閉後清空
    static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
}

/// `itemData` 的標記：高位是這個值的才是這裡的項目（右鍵選單的 Windows 項目也會送 WM_DRAWITEM）
const TAG: usize = 0x5346_0000;
const TAG_MASK: usize = 0xFFFF_0000;

/// 尺寸（跟著文字大小放大）
struct Metrics {
    pad: i32,
    mark: i32,
    glyph: i32,
    gap: i32,
    arrow: i32,
    row: i32,
    separator: i32,
}

fn metrics() -> Metrics {
    let t = app::theme();
    Metrics {
        pad: t.st(6),
        mark: t.st(26),
        glyph: t.st(26),
        gap: t.st(36),
        arrow: t.st(24),
        row: t.fonts.body_height + t.st(12),
        separator: t.st(9),
    }
}

fn split_accel(text: &str) -> (String, String) {
    match text.split_once('\t') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (text.to_string(), String::new()),
    }
}

/// 建立一層選單（子選單遞迴），項目資料放進 `rows`
unsafe fn build(items: &[Item], rows: &mut Vec<Row>, hdc: HDC, brush: HBRUSH) -> HMENU {
    let t = app::theme();
    let menu = CreatePopupMenu();
    // 選單的底色（項目之間、上下的空白）也用主題的顏色
    let mi = MENUINFO { cbSize: std::mem::size_of::<MENUINFO>() as u32, fMask: MIM_BACKGROUND, dwStyle: 0, cyMax: 0, hbrBack: brush, dwContextHelpID: 0, dwMenuData: 0 };
    SetMenuInfo(menu, &mi);
    // 這一層的欄寬
    let mut cols = Cols::default();
    for item in items {
        match item {
            Item::Command { text, glyph, .. } | Item::Submenu { text, glyph, .. } => {
                let (main, accel) = split_accel(text);
                cols.text = cols.text.max(theme::text_width(hdc, &main, t.fonts.body));
                cols.accel = cols.accel.max(theme::text_width(hdc, &accel, t.fonts.body));
                cols.glyph |= glyph.is_some();
                cols.submenu |= matches!(item, Item::Submenu { .. });
            }
            Item::Separator => {}
        }
    }
    for (pos, item) in items.iter().enumerate() {
        let mut mii: MENUITEMINFOW = std::mem::zeroed();
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        let index = rows.len();
        // 螢幕閱讀器讀的文字（自己畫的項目照樣設定）；要活到 InsertMenuItemW 複製完為止
        let mut text_buf: Vec<u16> = match item {
            Item::Command { text, .. } | Item::Submenu { text, .. } => wide(text),
            Item::Separator => Vec::new(),
        };
        let mut row = Row { text: String::new(), accel: String::new(), glyph: None, mark: Mark::None, on: false, enabled: true, submenu: false, separator: false, cols };
        match item {
            Item::Separator => {
                mii.fMask = MIIM_FTYPE | MIIM_DATA;
                mii.fType = MFT_SEPARATOR | MFT_OWNERDRAW;
                row.separator = true;
            }
            Item::Command { id, text, glyph, mark, on, enabled } => {
                mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING | MIIM_DATA;
                mii.fType = MFT_OWNERDRAW | if *mark == Mark::Radio { MFT_RADIOCHECK } else { 0 };
                mii.fState = if *on && *mark != Mark::None { MFS_CHECKED } else { 0 } | if *enabled { 0 } else { MFS_GRAYED };
                mii.wID = *id as u32;
                mii.dwTypeData = text_buf.as_mut_ptr();
                (row.text, row.accel) = split_accel(text);
                (row.glyph, row.mark, row.on, row.enabled) = (*glyph, *mark, *on, *enabled);
            }
            Item::Submenu { text, glyph, items } => {
                mii.fMask = MIIM_FTYPE | MIIM_SUBMENU | MIIM_STRING | MIIM_DATA;
                mii.fType = MFT_OWNERDRAW;
                mii.dwTypeData = text_buf.as_mut_ptr();
                (row.text, row.glyph, row.submenu) = (text.to_string(), *glyph, true);
                // 子選單的項目接在後面；這一項的索引先佔好
                rows.push(row);
                mii.hSubMenu = build(items, rows, hdc, brush);
                mii.dwItemData = TAG | index;
                InsertMenuItemW(menu, pos as u32, 1, &mii);
                continue;
            }
        }
        rows.push(row);
        mii.dwItemData = TAG | index;
        InsertMenuItemW(menu, pos as u32, 1, &mii);
    }
    menu
}

/// 建立選單、顯示並等使用者選（TPM_RETURNCMD），關閉後清掉項目資料
fn track(owner: HWND, items: &[Item], x: i32, y: i32, flags: u32, exclude: Option<RECT>) -> Option<u16> {
    let t = app::theme();
    unsafe {
        let screen = GetDC(0);
        let mut rows = Vec::new();
        let menu = build(items, &mut rows, screen, t.brushes.surface);
        ReleaseDC(0, screen);
        ROWS.with(|r| *r.borrow_mut() = rows);
        let params = exclude.map(|r| TPMPARAMS { cbSize: std::mem::size_of::<TPMPARAMS>() as u32, rcExclude: r });
        let p = params.as_ref().map_or(std::ptr::null(), |p| p as *const TPMPARAMS as *const std::ffi::c_void);
        let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | flags, x, y, owner, p);
        DestroyMenu(menu);
        ROWS.with(|r| r.borrow_mut().clear());
        (cmd > 0).then_some(cmd as u16)
    }
}

/// 在按鈕下方顯示選單（不蓋住按鈕；選單開著時按鈕維持按下的樣子），回傳選到的命令。
pub fn drop_down(owner: HWND, button: HWND, items: &[Item]) -> Option<u16> {
    let mut r = RECT::default();
    unsafe {
        GetWindowRect(button, &mut r);
    }
    controls::set_checked(button, true);
    let cmd = drop_down_at(owner, r, items);
    controls::set_checked(button, false);
    // 選單開著時按鈕收不到滑鼠離開的通知：先當作離開，滑鼠還在上面時下一次移動會再亮起
    unsafe {
        PostMessageW(button, WM_MOUSELEAVE, 0, 0);
    }
    cmd
}

/// 在畫面上一塊範圍（螢幕座標，例如自己畫的按鈕）下方顯示選單，不蓋住那塊範圍；回傳選到的命令。
pub fn drop_down_at(owner: HWND, anchor: RECT, items: &[Item]) -> Option<u16> {
    track(owner, items, anchor.left, anchor.bottom, TPM_VERTICAL, Some(anchor))
}

/// 在一個點（螢幕座標，例如右鍵按下的位置）顯示選單；回傳選到的命令。
pub fn popup_at(owner: HWND, pt: POINT, items: &[Item]) -> Option<u16> {
    track(owner, items, pt.x, pt.y, 0, None)
}

/// 選單擁有者的視窗程序先呼叫這個：是這裡的項目就量大小、畫出來或處理打字，回傳要給 Windows 的結果。
pub fn owner_draw(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_MEASUREITEM => unsafe {
            let mis = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            if mis.CtlType != ODT_MENU || mis.itemData & TAG_MASK != TAG {
                return None;
            }
            let index = mis.itemData & !TAG_MASK;
            ROWS.with(|rows| {
                let rows = rows.borrow();
                let row = rows.get(index)?;
                let m = metrics();
                if row.separator {
                    mis.itemWidth = 0;
                    mis.itemHeight = m.separator as u32;
                    return Some(());
                }
                let c = row.cols;
                let mut w = m.pad + m.mark + if c.glyph { m.glyph } else { 0 } + c.text + m.pad;
                if c.accel > 0 {
                    w += m.gap + c.accel;
                }
                if c.submenu {
                    w += m.arrow;
                }
                // （Windows 可能再替自己畫的項目加上一個勾號的寬度；文字從左邊排，右邊多一點空白不影響）
                mis.itemWidth = w.max(m.mark) as u32;
                mis.itemHeight = m.row as u32;
                Some(())
            })?;
            Some(1)
        },
        WM_DRAWITEM => unsafe {
            let dis = &*(lparam as *const DRAWITEMSTRUCT);
            if dis.CtlType != ODT_MENU || dis.itemData & TAG_MASK != TAG {
                return None;
            }
            let index = dis.itemData & !TAG_MASK;
            ROWS.with(|rows| {
                let rows = rows.borrow();
                let row = rows.get(index)?;
                draw(dis, row);
                Some(())
            })?;
            Some(1)
        },
        // 打字：跳到下一個開頭相同的項目（不分大小寫）；沒有相符的就留在原處，不發出提示音
        WM_MENUCHAR => unsafe {
            let menu = lparam as HMENU;
            let typed = char::from_u32(loword(wparam) as u32)?.to_lowercase().next()?;
            let data = |pos: i32| -> Option<usize> {
                let mut mii: MENUITEMINFOW = std::mem::zeroed();
                mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
                mii.fMask = MIIM_DATA;
                (GetMenuItemInfoW(menu, pos as u32, 1, &mut mii) != 0 && mii.dwItemData & TAG_MASK == TAG).then_some(mii.dwItemData & !TAG_MASK)
            };
            let count = GetMenuItemCount(menu);
            // 不是這裡的選單（例如右鍵選單）交給原本的處理
            data(0)?;
            let current = (0..count).find(|&i| GetMenuState(menu, i as u32, MF_BYPOSITION) & MF_HILITE != 0);
            let start = current.map_or(0, |c| c + 1);
            let found = ROWS.with(|rows| {
                let rows = rows.borrow();
                (0..count).map(|k| (start + k) % count.max(1)).find(|&pos| {
                    data(pos).and_then(|i| rows.get(i)).is_some_and(|r| !r.separator && r.enabled && r.text.chars().next().and_then(|c| c.to_lowercase().next()) == Some(typed))
                })
            });
            let pos = found.or(current).unwrap_or(0);
            Some((MNC_SELECT << 16) | (pos as isize & 0xFFFF))
        },
        _ => None,
    }
}

unsafe fn draw(dis: &DRAWITEMSTRUCT, row: &Row) {
    let t = app::theme();
    let p = &t.pal;
    let m = metrics();
    let dc = dis.hDC;
    let rc = dis.rcItem;
    theme::fill(dc, &rc, p.surface);
    if row.separator {
        let y = rc.top + rc.height() / 2;
        theme::fill(dc, &RECT { left: rc.left + m.pad + t.st(6), top: y, right: rc.right - m.pad - t.st(6), bottom: y + 1 }, p.line);
        ExcludeClipRect(dc, rc.left, rc.top, rc.right, rc.bottom);
        return;
    }
    let enabled = row.enabled && dis.itemState & (ODS_GRAYED | ODS_DISABLED) == 0;
    // 滑鼠停著或用方向鍵選到：圓角的淡底（同 Windows 11 的選單）
    if dis.itemState & ODS_SELECTED != 0 && enabled {
        let hl = RECT { left: rc.left + t.st(4), top: rc.top + t.st(2), right: rc.right - t.st(4), bottom: rc.bottom - t.st(2) };
        theme::round_rect(dc, &hl, t.st(8), Some(theme::blend(p.ink, p.surface, 0.08)), None);
    }
    let ink = if enabled { p.ink } else { theme::blend(p.ink, p.surface, 0.4) };
    let faint = if enabled { p.muted } else { theme::blend(p.muted, p.surface, 0.5) };
    let flags = DT_LEFT | DT_VCENTER | DT_SINGLELINE;
    let mut x = rc.left + m.pad;
    // 圓點（單選）或勾號
    let mark = RECT { left: x, right: x + m.mark, ..rc };
    if row.on {
        match row.mark {
            Mark::Radio => {
                let d = t.st(6);
                let dot = RECT::new(mark.left + (mark.width() - d) / 2, mark.top + (mark.height() - d) / 2, d, d);
                theme::circle(dc, &dot, Some(ink), None);
            }
            Mark::Check => theme::draw_text(dc, &glyph::CHECK_MARK.to_string(), &mark, t.fonts.body_icon_small, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE),
            Mark::None => {}
        }
    }
    x += m.mark;
    if row.cols.glyph {
        if let Some(g) = row.glyph {
            let gr = RECT { left: x, right: x + m.glyph, ..rc };
            theme::draw_text(dc, &g.to_string(), &gr, t.fonts.body_icon_small, ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
        }
        x += m.glyph;
    }
    let text_right = if row.cols.submenu { rc.right - m.arrow } else { rc.right - m.pad };
    theme::draw_text(dc, &row.text, &RECT { left: x, right: (x + row.cols.text + 1).min(text_right), ..rc }, t.fonts.body, ink, flags | DT_END_ELLIPSIS);
    if !row.accel.is_empty() {
        let ax = x + row.cols.text + m.gap;
        theme::draw_text(dc, &row.accel, &RECT { left: ax, right: text_right, ..rc }, t.fonts.body, faint, flags | DT_END_ELLIPSIS);
    }
    if row.submenu {
        let ar = RECT { left: rc.right - m.arrow, right: rc.right - m.pad, ..rc };
        theme::draw_text(dc, &glyph::CHEVRON_RIGHT.to_string(), &ar, t.fonts.body_icon_small, faint, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    // 子選單的箭頭由這裡畫：擋住 Windows 之後要補畫的那一個
    ExcludeClipRect(dc, rc.left, rc.top, rc.right, rc.bottom);
}

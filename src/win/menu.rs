//! 下拉選單（排序、檢視）：原生選單加上檔案總管那樣的圓點（單選）、勾選與子選單，
//! 項目前面的小圖示用圖示字型畫成 32 位元點陣圖。選單會跟著系統的深淺色（見 set_preferred_app_mode）。

use super::controls;
use super::sys::*;
use super::util::*;

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

/// 畫圖示用的字型、顏色與大小
pub struct Look {
    pub font: HFONT,
    pub color: COLORREF,
    pub size: i32,
}

/// 圖示字形畫成預乘 alpha 的 32 位元點陣圖（選單項目的 hbmpItem）。
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

unsafe fn build(items: &[Item], look: &Look, bitmaps: &mut Vec<HBITMAP>) -> HMENU {
    let menu = CreatePopupMenu();
    for (pos, item) in items.iter().enumerate() {
        let mut mii: MENUITEMINFOW = std::mem::zeroed();
        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
        // 文字要活到 InsertMenuItemW 複製完為止
        let mut text_buf: Vec<u16> = match item {
            Item::Command { text, .. } | Item::Submenu { text, .. } => wide(text),
            Item::Separator => Vec::new(),
        };
        match item {
            Item::Separator => {
                mii.fMask = MIIM_FTYPE;
                mii.fType = MFT_SEPARATOR;
            }
            Item::Command { id, glyph, mark, on, enabled, .. } => {
                mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING;
                mii.fType = MFT_STRING | if *mark == Mark::Radio { MFT_RADIOCHECK } else { 0 };
                mii.fState = if *on && *mark != Mark::None { MFS_CHECKED } else { 0 } | if *enabled { 0 } else { MFS_GRAYED };
                mii.wID = *id as u32;
                mii.dwTypeData = text_buf.as_mut_ptr();
                if let Some(g) = glyph {
                    let b = glyph_bitmap(*g, look);
                    if b != 0 {
                        bitmaps.push(b);
                        mii.fMask |= MIIM_BITMAP;
                        mii.hbmpItem = b;
                    }
                }
            }
            Item::Submenu { glyph, items, .. } => {
                mii.fMask = MIIM_FTYPE | MIIM_SUBMENU | MIIM_STRING;
                mii.fType = MFT_STRING;
                mii.hSubMenu = build(items, look, bitmaps);
                mii.dwTypeData = text_buf.as_mut_ptr();
                if let Some(g) = glyph {
                    let b = glyph_bitmap(*g, look);
                    if b != 0 {
                        bitmaps.push(b);
                        mii.fMask |= MIIM_BITMAP;
                        mii.hbmpItem = b;
                    }
                }
            }
        }
        InsertMenuItemW(menu, pos as u32, 1, &mii);
    }
    menu
}

/// 在按鈕下方顯示選單（不蓋住按鈕；選單開著時按鈕維持按下的樣子），回傳選到的命令。
pub fn drop_down(owner: HWND, button: HWND, items: &[Item], look: &Look) -> Option<u16> {
    unsafe {
        let mut bitmaps = Vec::new();
        let menu = build(items, look, &mut bitmaps);
        let mut r = RECT::default();
        GetWindowRect(button, &mut r);
        let params = TPMPARAMS { cbSize: std::mem::size_of::<TPMPARAMS>() as u32, rcExclude: r };
        controls::set_checked(button, true);
        let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_VERTICAL, r.left, r.bottom, owner, &params as *const TPMPARAMS as *const std::ffi::c_void);
        controls::set_checked(button, false);
        // 選單開著時按鈕收不到滑鼠離開的通知：先當作離開，滑鼠還在上面時下一次移動會再亮起
        PostMessageW(button, WM_MOUSELEAVE, 0, 0);
        DestroyMenu(menu);
        for b in bitmaps {
            DeleteObject(b);
        }
        (cmd > 0).then_some(cmd as u16)
    }
}

//! Windows 的檔案右鍵選單（IContextMenu），讓 Stillfeather 的右鍵選單也有使用者安裝的程式加入的項目：
//! 在終端中開啟、使用 Visual Studio 開啟、Open Git Bash here、7-Zip、傳送到、授與存取權給、
//! 新增 ▸ 等，與檔案總管的「其他選項」選單相同來源。
//!
//! 開啟、剪下、複製、貼上、刪除、重新命名、內容這些 Stillfeather 本來就有的命令，會把 Windows 的同名項目藏起來，
//! 仍由 Stillfeather 處理（行為一致，刪除也走 Stillfeather 的確認對話方塊）。
//!
//! 「新增 ▸」「傳送到 ▸」這類子選單是打開時才由 Windows 填入，圖示也可能由擴充程式自己畫，
//! 所以選單顯示期間主視窗要把 WM_INITMENUPOPUP、WM_DRAWITEM、WM_MEASUREITEM、WM_MENUCHAR
//! 轉給它（見 `handle_menu_msg`）。

use super::menu;
use super::sys::*;
use super::util::*;
use feather_core::paths;
use std::cell::Cell;
use std::ffi::c_void;

pub(super) const IID_ISHELLFOLDER: GUID = GUID::from_u128(0x000214E6_0000_0000_C000_000000000046);
const IID_ICONTEXTMENU: GUID = GUID::from_u128(0x000214E4_0000_0000_C000_000000000046);
const IID_ICONTEXTMENU2: GUID = GUID::from_u128(0x000214F4_0000_0000_C000_000000000046);
const IID_ICONTEXTMENU3: GUID = GUID::from_u128(0xBCFCE0A0_EC17_11D0_8D10_00A0C90F2719);

/// Windows 項目的命令編號範圍（Stillfeather 自己的命令都小於 0x1000）。
const FIRST: u32 = 0x2000;
const LAST: u32 = 0x7FFF;

type QueryInterface = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
struct ComObject {
    vtbl: *const c_void,
}

#[repr(C)]
pub(super) struct ShellFolderVtbl {
    pub query_interface: QueryInterface,
    pub add_ref: usize,
    pub release: Release,
    parse_display_name: usize,
    enum_objects: usize,
    bind_to_object: usize,
    bind_to_storage: usize,
    compare_ids: usize,
    pub create_view_object: unsafe extern "system" fn(*mut c_void, HWND, *const GUID, *mut *mut c_void) -> HRESULT,
    get_attributes_of: usize,
    pub get_ui_object_of: unsafe extern "system" fn(*mut c_void, HWND, u32, *const *const c_void, *const GUID, *mut u32, *mut *mut c_void) -> HRESULT,
    get_display_name_of: usize,
    set_name_of: usize,
}

/// IContextMenu → IContextMenu2 → IContextMenu3 的方法依序排在同一張表；
/// 只呼叫實際取得的介面版本有的方法。
#[repr(C)]
struct ContextMenuVtbl {
    query_interface: QueryInterface,
    add_ref: usize,
    release: Release,
    query_context_menu: unsafe extern "system" fn(*mut c_void, HMENU, u32, u32, u32, u32) -> HRESULT,
    invoke_command: unsafe extern "system" fn(*mut c_void, *const CMINVOKECOMMANDINFOEX) -> HRESULT,
    get_command_string: unsafe extern "system" fn(*mut c_void, usize, u32, *mut u32, *mut u8, u32) -> HRESULT,
    handle_menu_msg: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM) -> HRESULT,
    handle_menu_msg2: unsafe extern "system" fn(*mut c_void, u32, WPARAM, LPARAM, *mut LRESULT) -> HRESULT,
}

pub(super) unsafe fn vtbl<T>(obj: *mut c_void) -> &'static T {
    &*((*(obj as *mut ComObject)).vtbl as *const T)
}

/// 釋放 COM 物件（null 時略過）。
pub(super) unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        (vtbl::<ContextMenuVtbl>(obj).release)(obj);
    }
}

/// SHParseDisplayName 傳回的項目識別碼，用完以 CoTaskMemFree 釋放。
pub(super) struct Pidl(pub *mut c_void);

impl Pidl {
    pub(super) fn parse(path: &str) -> Option<Pidl> {
        let w = wide(path);
        let mut pidl: *mut c_void = std::ptr::null_mut();
        let hr = unsafe { SHParseDisplayName(w.as_ptr(), std::ptr::null_mut(), &mut pidl, 0, std::ptr::null_mut()) };
        (hr >= 0 && !pidl.is_null()).then_some(Pidl(pidl))
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(self.0) }
    }
}

thread_local! {
    /// 正在顯示的 Windows 選單（IContextMenu2、IContextMenu3），供主視窗轉送選單訊息。
    static ACTIVE: Cell<(*mut c_void, *mut c_void)> = const { Cell::new((std::ptr::null_mut(), std::ptr::null_mut())) };
}

pub struct ShellMenu {
    cm: *mut c_void,
    cm2: *mut c_void,
    cm3: *mut c_void,
    /// 資料夾空白處的選單交給 Windows 的站台（「新增 ▸」建好項目後藉它進入改名，見 [`super::menusite`]）
    site: *mut c_void,
    /// 選到 Windows 項目時，選單要留到執行命令之後才銷毀（部分擴充程式執行時還會查詢它）。
    menu: Cell<HMENU>,
}

impl Drop for ShellMenu {
    fn drop(&mut self) {
        unsafe {
            if self.menu.get() != 0 {
                DestroyMenu(self.menu.get());
            }
            if !self.site.is_null() {
                super::menusite::detach(self.cm);
                super::menusite::release_site(self.site);
            }
            release(self.cm3);
            release(self.cm2);
            release(self.cm);
        }
    }
}

impl ShellMenu {
    fn wrap(cm: *mut c_void) -> ShellMenu {
        let mut cm2: *mut c_void = std::ptr::null_mut();
        let mut cm3: *mut c_void = std::ptr::null_mut();
        unsafe {
            let qi = vtbl::<ContextMenuVtbl>(cm).query_interface;
            if qi(cm, &IID_ICONTEXTMENU3, &mut cm3) < 0 {
                cm3 = std::ptr::null_mut();
            }
            if qi(cm, &IID_ICONTEXTMENU2, &mut cm2) < 0 {
                cm2 = std::ptr::null_mut();
            }
        }
        ShellMenu { cm, cm2, cm3, site: std::ptr::null_mut(), menu: Cell::new(0) }
    }

    /// 選取項目的選單。項目必須在同一個資料夾（搜尋結果跨資料夾時回傳 None，只顯示 Stillfeather 自己的項目）。
    pub fn for_items(owner: HWND, items: &[String]) -> Option<ShellMenu> {
        let cm = unsafe { ui_object(owner, items, &IID_ICONTEXTMENU)? };
        Some(ShellMenu::wrap(cm))
    }

    /// 資料夾空白處的選單（貼上捷徑、新增 ▸、在終端中開啟、Git 等）。
    pub fn for_background(owner: HWND, folder: &str) -> Option<ShellMenu> {
        let pidl = Pidl::parse(folder)?;
        unsafe {
            let mut sf: *mut c_void = std::ptr::null_mut();
            if SHBindToObject(std::ptr::null_mut(), pidl.0, std::ptr::null_mut(), &IID_ISHELLFOLDER, &mut sf) < 0 || sf.is_null() {
                return None;
            }
            let mut cm: *mut c_void = std::ptr::null_mut();
            let hr = (vtbl::<ShellFolderVtbl>(sf).create_view_object)(sf, owner, &IID_ICONTEXTMENU, &mut cm);
            release(sf);
            if hr < 0 || cm.is_null() {
                return None;
            }
            let mut menu = ShellMenu::wrap(cm);
            // 在填入選單之前交給它：Windows 在 QueryContextMenu 時載入「新增 ▸」等處理程式，並把站台轉給它們
            menu.site = super::menusite::attach(cm, owner, folder);
            Some(menu)
        }
    }

    /// Windows 項目的標準動詞（open、delete、properties…）；擴充程式的項目常常沒有。
    fn verb(&self, cmd: u32) -> Option<String> {
        if !(FIRST..=LAST).contains(&cmd) {
            return None;
        }
        let mut buf = [0u16; 128];
        let hr = unsafe {
            (vtbl::<ContextMenuVtbl>(self.cm).get_command_string)(
                self.cm,
                (cmd - FIRST) as usize,
                GCS_VERBW,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut u8,
                buf.len() as u32,
            )
        };
        if hr < 0 {
            return None;
        }
        let v = from_wide(&buf);
        (!v.is_empty()).then(|| v.to_ascii_lowercase())
    }

    /// 執行使用者選的 Windows 項目。`directory` 是目前資料夾（例如「在終端中開啟」的起始位置）。
    pub fn invoke(&self, owner: HWND, cmd: u32, directory: &str, pt: POINT) -> Result<(), String> {
        let dir = wide(directory);
        let offset = (cmd - FIRST) as usize;
        let mut info: CMINVOKECOMMANDINFOEX = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32;
        info.fMask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
        if key_down(VK_SHIFT) {
            info.fMask |= CMIC_MASK_SHIFT_DOWN;
        }
        if key_down(VK_CONTROL) {
            info.fMask |= CMIC_MASK_CONTROL_DOWN;
        }
        info.hwnd = owner;
        // MAKEINTRESOURCE：以「相對於第一個命令的位移」指定命令
        info.lpVerb = offset as *const u8;
        info.lpVerbW = offset as *const u16;
        if !directory.is_empty() {
            info.lpDirectoryW = dir.as_ptr();
        }
        info.nShow = SW_SHOWNORMAL;
        info.ptInvoke = pt;
        let hr = unsafe { (vtbl::<ContextMenuVtbl>(self.cm).invoke_command)(self.cm, &info) };
        // 使用者在 Windows 自己的對話方塊按取消（ERROR_CANCELLED）不算錯誤
        if hr >= 0 || hr as u32 == 0x800704C7 {
            Ok(())
        } else {
            Err(trf!("Windows 回報錯誤 0x{:08X}。", "Windows reported error 0x{:08X}.", hr as u32))
        }
    }
}

/// 同一個資料夾中的項目的 Shell 物件（右鍵選單、拖曳資料、拖放目標…）。
/// 項目不在同一個資料夾時回傳 None。
pub(super) unsafe fn ui_object(owner: HWND, items: &[String], iid: &GUID) -> Option<*mut c_void> {
    let first = items.first()?;
    let parent = paths::parent(first).unwrap_or_default();
    if items.iter().any(|p| !paths::same_path(&paths::parent(p).unwrap_or_default(), &parent)) {
        return None;
    }
    let pidls: Vec<Pidl> = items.iter().map(|p| Pidl::parse(p)).collect::<Option<_>>()?;
    // 每個項目都以 SHBindToParent 取得「在父資料夾內的相對識別碼」；父資料夾用第一個的。
    let mut folder: *mut c_void = std::ptr::null_mut();
    let mut children: Vec<*const c_void> = Vec::with_capacity(pidls.len());
    for (i, pidl) in pidls.iter().enumerate() {
        let mut sf: *mut c_void = std::ptr::null_mut();
        let mut child: *const c_void = std::ptr::null();
        if SHBindToParent(pidl.0, &IID_ISHELLFOLDER, &mut sf, &mut child) < 0 || sf.is_null() {
            release(folder);
            return None;
        }
        if i == 0 {
            folder = sf;
        } else {
            release(sf);
        }
        children.push(child);
    }
    let mut obj: *mut c_void = std::ptr::null_mut();
    let hr = (vtbl::<ShellFolderVtbl>(folder).get_ui_object_of)(folder, owner, children.len() as u32, children.as_ptr(), iid, std::ptr::null_mut(), &mut obj);
    release(folder);
    drop(pidls);
    (hr >= 0 && !obj.is_null()).then_some(obj)
}

/// 選單顯示期間由主視窗呼叫：屬於 Windows 選單的訊息交給它處理。
pub fn handle_menu_msg(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        return None;
    }
    let (cm2, cm3) = ACTIVE.with(|a| a.get());
    unsafe {
        if !cm3.is_null() {
            let mut result: LRESULT = 0;
            if (vtbl::<ContextMenuVtbl>(cm3).handle_menu_msg2)(cm3, msg, wparam, lparam, &mut result) >= 0 {
                return Some(result);
            }
        }
        if !cm2.is_null() && msg != WM_MENUCHAR && (vtbl::<ContextMenuVtbl>(cm2).handle_menu_msg)(cm2, msg, wparam, lparam) >= 0 {
            return Some(if msg == WM_INITMENUPOPUP { 0 } else { 1 });
        }
    }
    None
}

/// Stillfeather 自己的選單項目：命令編號、文字（\t 後是快捷鍵）、是否可用、前面的圖示字形。id 為 0 是分隔線。
#[derive(Clone, Copy)]
pub struct Cmd<'a> {
    pub id: u16,
    pub text: &'a str,
    pub enabled: bool,
    pub glyph: Option<char>,
}

impl<'a> Cmd<'a> {
    pub fn new(id: u16, text: &'a str, glyph: char) -> Cmd<'a> {
        Cmd { id, text, enabled: true, glyph: Some(glyph) }
    }

    pub fn enabled(mut self, on: bool) -> Cmd<'a> {
        self.enabled = on;
        self
    }
}

/// 選單中的一段：Stillfeather 自己的項目，或 Windows 的項目（已安裝的程式加入的命令）。段與段之間自動加分隔線。
pub enum Part<'a> {
    Ours(Vec<Cmd<'a>>),
    Shell,
}

pub enum Pick {
    Ours(u16),
    Shell(u32),
}

/// 顯示合併的右鍵選單。`hide` 是要藏起來的 Windows 動詞（Stillfeather 已有同樣的命令）；
/// `default` 是粗體的預設項目（通常是「開啟」）。
/// Stillfeather 的項目前面也畫上圖示（`look`），和 Windows 項目（程式圖示）對齊。
#[allow(clippy::too_many_arguments)]
pub fn popup(owner: HWND, parts: &[Part], shell: Option<&ShellMenu>, item_menu: bool, hide: &[&str], default: Option<u16>, pt: POINT, look: &menu::Look) -> Option<Pick> {
    unsafe {
        let menu = CreatePopupMenu();
        let mut shell_added = false;
        let mut bitmaps: Vec<HBITMAP> = Vec::new();
        for part in parts {
            match part {
                Part::Ours(items) => {
                    if GetMenuItemCount(menu) > 0 {
                        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    }
                    for c in items.iter() {
                        if c.id == 0 {
                            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                            continue;
                        }
                        let title = if let Some(cmd) = super::app::shortcut_command(c.id) {
                            super::app::app().keymap.with_hint(c.text.split('\t').next().unwrap_or(c.text), cmd, "\t")
                        } else { c.text.into() };
                        let mut text = wide(&title);
                        let mut mii: MENUITEMINFOW = std::mem::zeroed();
                        mii.cbSize = std::mem::size_of::<MENUITEMINFOW>() as u32;
                        mii.fMask = MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_STRING;
                        mii.fType = MFT_STRING;
                        mii.fState = if c.enabled { 0 } else { MFS_GRAYED };
                        mii.wID = c.id as u32;
                        mii.dwTypeData = text.as_mut_ptr();
                        if let Some(g) = c.glyph {
                            let b = menu::glyph_bitmap(g, look);
                            if b != 0 {
                                bitmaps.push(b);
                                mii.fMask |= MIIM_BITMAP;
                                mii.hbmpItem = b;
                            }
                        }
                        InsertMenuItemW(menu, GetMenuItemCount(menu).max(0) as u32, 1, &mii);
                    }
                }
                Part::Shell => {
                    let Some(sm) = shell else { continue };
                    let start = GetMenuItemCount(menu).max(0) as u32;
                    if start > 0 {
                        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                    }
                    let start = GetMenuItemCount(menu).max(0) as u32;
                    let mut flags = CMF_NORMAL;
                    if item_menu {
                        flags |= CMF_ITEMMENU;
                    }
                    if key_down(VK_SHIFT) {
                        flags |= CMF_EXTENDEDVERBS;
                    }
                    let hr = (vtbl::<ContextMenuVtbl>(sm.cm).query_context_menu)(sm.cm, menu, start, FIRST, LAST, flags);
                    if hr < 0 {
                        continue;
                    }
                    shell_added = true;
                    // 藏起與 Stillfeather 重複的項目
                    let mut pos = start as i32;
                    while pos < GetMenuItemCount(menu) {
                        let id = GetMenuItemID(menu, pos);
                        if id != u32::MAX && (FIRST..=LAST).contains(&id) && sm.verb(id).is_some_and(|v| hide.contains(&v.as_str())) {
                            DeleteMenu(menu, pos as u32, MF_BYPOSITION);
                            continue;
                        }
                        pos += 1;
                    }
                }
            }
        }
        tidy_separators(menu);
        if let Some(d) = default {
            SetMenuDefaultItem(menu, d as u32, 0);
        }
        let (cm2, cm3) = match shell {
            Some(sm) if shell_added => (sm.cm2, sm.cm3),
            _ => (std::ptr::null_mut(), std::ptr::null_mut()),
        };
        ACTIVE.with(|a| a.set((cm2, cm3)));
        let cmd = TrackPopupMenuEx(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, owner, std::ptr::null());
        ACTIVE.with(|a| a.set((std::ptr::null_mut(), std::ptr::null_mut())));
        // 選單已經關閉，不會再畫：圖示點陣圖可以先釋放（選了 Windows 項目時選單本身還要留給 invoke）
        for b in bitmaps {
            DeleteObject(b);
        }
        match (cmd as u32, shell) {
            (c, Some(sm)) if (FIRST..=LAST).contains(&c) => {
                sm.menu.set(menu);
                Some(Pick::Shell(c))
            }
            (c, _) => {
                DestroyMenu(menu);
                (c != 0 && c < FIRST).then_some(Pick::Ours(c as u16))
            }
        }
    }
}

/// 移除開頭、結尾與連續的分隔線。
unsafe fn tidy_separators(menu: HMENU) {
    let is_sep = |pos: i32| {
        let state = GetMenuState(menu, pos as u32, MF_BYPOSITION);
        state != u32::MAX && state & MF_POPUP == 0 && state & MF_SEPARATOR != 0
    };
    let mut pos = 0;
    let mut prev_sep = true;
    while pos < GetMenuItemCount(menu) {
        let sep = is_sep(pos);
        if sep && prev_sep {
            DeleteMenu(menu, pos as u32, MF_BYPOSITION);
            continue;
        }
        prev_sep = sep;
        pos += 1;
    }
    let n = GetMenuItemCount(menu);
    if n > 0 && is_sep(n - 1) {
        DeleteMenu(menu, (n - 1) as u32, MF_BYPOSITION);
    }
}

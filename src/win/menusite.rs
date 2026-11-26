//! 資料夾空白處右鍵選單的「站台」（site）：讓 Windows 的「新增 ▸」建好項目後能叫 Stillfeather 選取並重新命名。
//!
//! 檔案總管裡用「新增 ▸ 資料夾」（或文字文件等）建立的項目會馬上進入改名。做法是右鍵選單的處理程式透過
//! `IObjectWithSite::SetSite` 拿到的站台，以 `IServiceProvider::QueryService` 要 `IShellView`，再呼叫
//! `IShellView::SelectItem`（旗標含 `SVSI_EDIT`）。Stillfeather 的清單不是 Windows 的資料夾檢視，
//! 沒有站台時「新增 ▸」只會建立項目，不會進入改名。
//!
//! 這裡提供一個只做這件事的小物件：`IServiceProvider`（不管要哪一個服務，要的是 `IShellView`／`IOleWindow`
//! 就回答，同 [`super::shellwin`]）與 `IShellView`。`SelectItem` 給的是相對於資料夾的子項目識別碼，轉成完整路徑後
//! 以 [`WM_APP_BEGIN_RENAME`](super::app::WM_APP_BEGIN_RENAME) 交給主視窗（清單可能還沒有這個項目：主視窗會補上）。
//! 呼叫一律在介面執行緒（右鍵選單在這裡執行），物件以參考計數管理，選單釋放時先 `SetSite(null)`。

use super::app::{NewItem, WM_APP_BEGIN_RENAME};
use super::crashlog;
use super::sys::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

const S_OK: HRESULT = 0;
const E_NOTIMPL: HRESULT = 0x8000_4001_u32 as HRESULT;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
const E_POINTER: HRESULT = 0x8000_4003_u32 as HRESULT;
const E_FAIL: HRESULT = 0x8000_4005_u32 as HRESULT;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
const IID_ISERVICEPROVIDER: GUID = GUID::from_u128(0x6D5140C1_7436_11CE_8034_00AA006009FA);
const IID_IOLEWINDOW: GUID = GUID::from_u128(0x00000114_0000_0000_C000_000000000046);
const IID_ISHELLVIEW: GUID = GUID::from_u128(0x000214E3_0000_0000_C000_000000000046);
pub(super) const IID_IOBJECTWITHSITE: GUID = GUID::from_u128(0xFC4801A3_2BA9_11CF_A229_00AA003D7352);

/// IShellView::SelectItem 的旗標
const SVSI_SELECT: u32 = 0x1;
/// SVSI_EDIT 是 0x3（含 SVSI_SELECT）；只看多出來的那一位
const SVSI_EDIT_BIT: u32 = 0x2;
const SVSI_ENSUREVISIBLE: u32 = 0x8;
const SVSI_FOCUSED: u32 = 0x10;

type QueryFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;
type Plain = unsafe extern "system" fn(*mut c_void) -> HRESULT;

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

#[repr(C)]
struct Site {
    /// 第一個指標是 IServiceProvider（也是 IUnknown 的身分）
    sp: *const ServiceProviderVtbl,
    /// 第二個是 IShellView（也是 IOleWindow）
    sv: *const ShellViewVtbl,
    refs: AtomicU32,
    owner: HWND,
    folder: String,
}

/// IShellView 的 this 指標往前一格就是物件開頭
unsafe fn from_sv(this: *mut c_void) -> *mut Site {
    (this as *mut u8).sub(std::mem::size_of::<usize>()) as *mut Site
}

unsafe fn query(site: *mut Site, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = std::ptr::null_mut();
    if riid.is_null() {
        return E_POINTER;
    }
    let riid = &*riid;
    let p: *mut c_void = if same(riid, &IID_IUNKNOWN) || same(riid, &IID_ISERVICEPROVIDER) {
        &mut (*site).sp as *mut _ as *mut c_void
    } else if same(riid, &IID_ISHELLVIEW) || same(riid, &IID_IOLEWINDOW) {
        &mut (*site).sv as *mut _ as *mut c_void
    } else {
        return E_NOINTERFACE;
    };
    add_ref(site);
    *out = p;
    S_OK
}

unsafe fn add_ref(site: *mut Site) -> u32 {
    (*site).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe fn release(site: *mut Site) -> u32 {
    let left = (*site).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        drop(Box::from_raw(site));
    }
    left
}

// ---- IServiceProvider

#[repr(C)]
struct ServiceProviderVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    query_service: unsafe extern "system" fn(*mut c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT,
}

unsafe extern "system" fn sp_qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    query(this as *mut Site, riid, out)
}
unsafe extern "system" fn sp_add_ref(this: *mut c_void) -> u32 {
    add_ref(this as *mut Site)
}
unsafe extern "system" fn sp_release(this: *mut c_void) -> u32 {
    release(this as *mut Site)
}
unsafe extern "system" fn query_service(this: *mut c_void, _service: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let hr = query(this as *mut Site, riid, out);
    if hr == S_OK && !riid.is_null() && same(&*riid, &IID_ISHELLVIEW) {
        crashlog::crumb("右鍵選單：Windows 向站台要 IShellView");
    }
    hr
}

static SERVICE_PROVIDER_VTBL: ServiceProviderVtbl = ServiceProviderVtbl { query_interface: sp_qi, add_ref: sp_add_ref, release: sp_release, query_service };

// ---- IShellView（IOleWindow 2 + IShellView 11）

#[repr(C)]
struct ShellViewVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    get_window: unsafe extern "system" fn(*mut c_void, *mut HWND) -> HRESULT,
    context_sensitive_help: Plain,
    translate_accelerator: Plain,
    enable_modeless: Plain,
    ui_activate: Plain,
    refresh: Plain,
    create_view_window: unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void, *mut c_void, *const c_void, *mut HWND) -> HRESULT,
    destroy_view_window: Plain,
    get_current_info: unsafe extern "system" fn(*mut c_void, *mut [u32; 2]) -> HRESULT,
    add_property_sheet_pages: Plain,
    save_view_state: Plain,
    select_item: unsafe extern "system" fn(*mut c_void, *const c_void, u32) -> HRESULT,
    get_item_object: unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void) -> HRESULT,
}

unsafe extern "system" fn sv_qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    query(from_sv(this), riid, out)
}
unsafe extern "system" fn sv_add_ref(this: *mut c_void) -> u32 {
    add_ref(from_sv(this))
}
unsafe extern "system" fn sv_release(this: *mut c_void) -> u32 {
    release(from_sv(this))
}
unsafe extern "system" fn sv_get_window(this: *mut c_void, out: *mut HWND) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = (*from_sv(this)).owner;
    S_OK
}
unsafe extern "system" fn sv_not_impl(_this: *mut c_void) -> HRESULT {
    E_NOTIMPL
}
unsafe extern "system" fn sv_ok(_this: *mut c_void) -> HRESULT {
    S_OK
}
unsafe extern "system" fn sv_create_view_window(_this: *mut c_void, _prev: *mut c_void, _fs: *const c_void, _browser: *mut c_void, _rect: *const c_void, out: *mut HWND) -> HRESULT {
    if !out.is_null() {
        *out = 0;
    }
    E_NOTIMPL
}
unsafe extern "system" fn sv_get_current_info(_this: *mut c_void, out: *mut [u32; 2]) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    *out = [0, 0];
    S_OK
}
unsafe extern "system" fn sv_get_item_object(_this: *mut c_void, _item: u32, _riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    E_NOINTERFACE
}

/// 「新增 ▸」建好項目後呼叫：選取它，旗標有 SVSI_EDIT 時進入改名。
unsafe extern "system" fn sv_select_item(this: *mut c_void, child: *const c_void, flags: u32) -> HRESULT {
    let site = from_sv(this);
    if child.is_null() || flags & (SVSI_SELECT | SVSI_FOCUSED | SVSI_ENSUREVISIBLE) == 0 {
        return S_OK;
    }
    let Some(path) = super::shellwin::item_path(&(*site).folder, child) else {
        crashlog::crumb("右鍵選單：SelectItem 取不到新項目的路徑");
        return E_FAIL;
    };
    let edit = flags & SVSI_EDIT_BIT != 0;
    crashlog::crumb(if edit { "右鍵選單：SelectItem 要求改名新項目" } else { "右鍵選單：SelectItem 要求選取項目" });
    let msg = Box::into_raw(Box::new(NewItem { tab: None, path, edit }));
    if PostMessageW((*site).owner, WM_APP_BEGIN_RENAME, 0, msg as LPARAM) == 0 {
        drop(Box::from_raw(msg));
        return E_FAIL;
    }
    S_OK
}

static SHELL_VIEW_VTBL: ShellViewVtbl = ShellViewVtbl {
    query_interface: sv_qi,
    add_ref: sv_add_ref,
    release: sv_release,
    get_window: sv_get_window,
    context_sensitive_help: sv_not_impl,
    translate_accelerator: sv_not_impl,
    enable_modeless: sv_ok,
    ui_activate: sv_ok,
    refresh: sv_ok,
    create_view_window: sv_create_view_window,
    destroy_view_window: sv_ok,
    get_current_info: sv_get_current_info,
    add_property_sheet_pages: sv_not_impl,
    save_view_state: sv_ok,
    select_item: sv_select_item,
    get_item_object: sv_get_item_object,
};

// ---- IObjectWithSite（右鍵選單那一邊）

#[repr(C)]
struct ObjectWithSiteVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    set_site: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    get_site: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
}

/// 新的站台（參考計數 1；呼叫端用完以 [`release_site`] 釋放）
fn new_site(owner: HWND, folder: &str) -> *mut c_void {
    let site = Box::new(Site { sp: &SERVICE_PROVIDER_VTBL, sv: &SHELL_VIEW_VTBL, refs: AtomicU32::new(1), owner, folder: folder.to_string() });
    Box::into_raw(site) as *mut c_void
}

/// 釋放 [`attach`] 傳回的站台
pub(super) unsafe fn release_site(site: *mut c_void) {
    if !site.is_null() {
        release(site as *mut Site);
    }
}

/// 把站台交給右鍵選單（`cm`：IContextMenu）。選單不支援站台時回傳 null。
/// 傳回的站台由呼叫端保管：選單釋放前呼叫 [`detach`]，再 [`release_site`]。
pub(super) unsafe fn attach(cm: *mut c_void, owner: HWND, folder: &str) -> *mut c_void {
    if cm.is_null() {
        return std::ptr::null_mut();
    }
    let qi = (*(*(cm as *mut *const ObjectWithSiteVtbl))).query_interface;
    let mut ows: *mut c_void = std::ptr::null_mut();
    if qi(cm, &IID_IOBJECTWITHSITE, &mut ows) < 0 || ows.is_null() {
        return std::ptr::null_mut();
    }
    let site = new_site(owner, folder);
    let v = &*(*(ows as *mut *const ObjectWithSiteVtbl));
    let hr = (v.set_site)(ows, site);
    (v.release)(ows);
    if hr < 0 {
        release_site(site);
        return std::ptr::null_mut();
    }
    site
}

/// 右鍵選單放開站台（`SetSite(null)`），之後才釋放選單與站台
pub(super) unsafe fn detach(cm: *mut c_void) {
    if cm.is_null() {
        return;
    }
    let qi = (*(*(cm as *mut *const ObjectWithSiteVtbl))).query_interface;
    let mut ows: *mut c_void = std::ptr::null_mut();
    if qi(cm, &IID_IOBJECTWITHSITE, &mut ows) < 0 || ows.is_null() {
        return;
    }
    let v = &*(*(ows as *mut *const ObjectWithSiteVtbl));
    (v.set_site)(ows, std::ptr::null_mut());
    (v.release)(ows);
}

// 會連結到 Windows 的函式（PostMessageW 等），只在 Windows 上執行
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn site_interfaces_share_one_identity() {
        unsafe {
            let site = new_site(0, "C:\\");
            let mut sv: *mut c_void = std::ptr::null_mut();
            assert_eq!(query(site as *mut Site, &IID_ISHELLVIEW, &mut sv), S_OK);
            assert_eq!(from_sv(sv), site as *mut Site);
            let mut unk: *mut c_void = std::ptr::null_mut();
            assert_eq!(sv_qi(sv, &IID_IUNKNOWN, &mut unk), S_OK);
            assert_eq!(unk, site);
            let mut none: *mut c_void = std::ptr::null_mut();
            assert_eq!(sp_qi(site, &IID_IOBJECTWITHSITE, &mut none), E_NOINTERFACE);
            assert!(none.is_null());
            // 1（建立）+ 2（兩次 QueryInterface）
            assert_eq!((*(site as *mut Site)).refs.load(Ordering::Relaxed), 3);
            sv_release(sv);
            sp_release(unk);
            release_site(site);
        }
    }
}

//! 螢幕閱讀器（朗讀程式、NVDA）：自繪的按鈕與對話方塊用 Dynamic Annotation（`IAccPropServices`）補上角色、狀態與文字。
//!
//! 系統對不認得的視窗類別（例如 controls.rs 的自繪按鈕）只當成一般的「窗格」：讀得到視窗文字，但不知道它是按鈕、
//! 也不知道勾選了沒有；對話方塊自己畫的標題與內文更是完全讀不到。這裡在系統預設的無障礙物件上覆寫幾個屬性：
//! - 按鈕：角色（按鈕、核取方塊、選項按鈕）；核取方塊與選項按鈕另外給狀態（勾選、焦點、停用、隱藏），勾選改變時通知。
//! - 對話方塊：工作區的名稱是完整的文字（標題、內文、資料列與附註），角色是對話方塊。
//!
//! 只呼叫系統的元件（oleacc），不實作自己的 COM 物件；任何一步失敗都只是少了這些資訊，不影響其他功能。
//! 需要已初始化 COM 的執行緒（介面執行緒是 STA）。

use super::sys::*;
use super::util::wide;
use std::cell::Cell;
use std::ffi::c_void;

const CLSID_ACC_PROP_SERVICES: GUID = GUID::from_u128(0xb5f8350b_0548_48b1_a6ee_88bd00b4a5e7);
const IID_IACC_PROP_SERVICES: GUID = GUID::from_u128(0x6e26e776_04f0_495d_80e4_3330352e3169);
const PROPID_ACC_NAME: GUID = GUID::from_u128(0x608d3df8_8128_4aa7_a428_f55e49267291);
const PROPID_ACC_ROLE: GUID = GUID::from_u128(0xcb905ff2_7bd1_4c05_b3c8_e6c241364d70);
const PROPID_ACC_STATE: GUID = GUID::from_u128(0xa8d4d5b0_0a21_42d0_a5c0_514e984f457b);

const OBJID_CLIENT: i32 = -4;
const CHILDID_SELF: i32 = 0;
const EVENT_OBJECT_STATECHANGE: u32 = 0x800A;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;
const VT_I4: u16 = 3;

pub const ROLE_DIALOG: i32 = 0x12;
pub const ROLE_PUSHBUTTON: i32 = 0x2B;
pub const ROLE_CHECKBUTTON: i32 = 0x2C;
pub const ROLE_RADIOBUTTON: i32 = 0x2D;

const STATE_UNAVAILABLE: u32 = 0x1;
const STATE_FOCUSED: u32 = 0x4;
const STATE_CHECKED: u32 = 0x10;
const STATE_FOCUSABLE: u32 = 0x10_0000;

/// IAccPropServices 用到的方法（前三個是 IUnknown）
#[repr(C)]
struct ServicesVtbl {
    _unknown: [usize; 3],
    _set_prop_value: usize,
    _set_prop_server: usize,
    _clear_props: usize,
    set_hwnd_prop: unsafe extern "system" fn(*mut c_void, HWND, u32, u32, GUID, PROPVARIANT) -> HRESULT,
    set_hwnd_prop_str: unsafe extern "system" fn(*mut c_void, HWND, u32, u32, GUID, *const u16) -> HRESULT,
    _set_hwnd_prop_server: usize,
    clear_hwnd_props: unsafe extern "system" fn(*mut c_void, HWND, u32, u32, *const GUID, i32) -> HRESULT,
}

thread_local! {
    /// 0：還沒建立；1：建立失敗（不再試）；其他：IAccPropServices（整個執行緒共用，不釋放）
    static SERVICES: Cell<usize> = const { Cell::new(0) };
}

fn services() -> Option<(*mut c_void, &'static ServicesVtbl)> {
    let p = SERVICES.with(|s| {
        if s.get() == 0 {
            let mut out: *mut c_void = std::ptr::null_mut();
            let hr = unsafe { CoCreateInstance(&CLSID_ACC_PROP_SERVICES, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IACC_PROP_SERVICES, &mut out) };
            s.set(if hr >= 0 && !out.is_null() { out as usize } else { 1 });
        }
        s.get()
    });
    if p <= 1 {
        return None;
    }
    let this = p as *mut c_void;
    Some((this, unsafe { &**(this as *const *const ServicesVtbl) }))
}

fn int_variant(v: i32) -> PROPVARIANT {
    PROPVARIANT { vt: VT_I4, reserved: [0; 3], data: [v as u32 as u64, 0] }
}

/// 視窗工作區（OBJID_CLIENT）的角色
pub fn set_role(hwnd: HWND, role: i32) {
    if let Some((this, vt)) = services() {
        unsafe {
            (vt.set_hwnd_prop)(this, hwnd, OBJID_CLIENT as u32, CHILDID_SELF as u32, PROPID_ACC_ROLE, int_variant(role));
        }
    }
}

/// 視窗工作區的名稱（螢幕閱讀器念的文字）
pub fn set_name(hwnd: HWND, name: &str) {
    if let Some((this, vt)) = services() {
        let w = wide(name);
        unsafe {
            (vt.set_hwnd_prop_str)(this, hwnd, OBJID_CLIENT as u32, CHILDID_SELF as u32, PROPID_ACC_NAME, w.as_ptr());
            NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, hwnd, OBJID_CLIENT, CHILDID_SELF);
        }
    }
}

/// 核取方塊、選項按鈕的狀態。覆寫狀態時系統不再自己算焦點與停用，所以這裡一起給；
/// 勾選、焦點或啟用改變時都要再呼叫一次（`notify`：通知螢幕閱讀器狀態變了）。
/// 不給「隱藏」：父視窗顯示時按鈕收不到通知，給了反而會一直被當成隱藏。
pub fn set_check_state(hwnd: HWND, checked: bool, notify: bool) {
    let Some((this, vt)) = services() else { return };
    let mut state = STATE_FOCUSABLE;
    unsafe {
        if GetFocus() == hwnd {
            state |= STATE_FOCUSED;
        }
        if IsWindowEnabled(hwnd) == 0 {
            state |= STATE_UNAVAILABLE;
        }
    }
    if checked {
        state |= STATE_CHECKED;
    }
    unsafe {
        (vt.set_hwnd_prop)(this, hwnd, OBJID_CLIENT as u32, CHILDID_SELF as u32, PROPID_ACC_STATE, int_variant(state as i32));
        if notify {
            NotifyWinEvent(EVENT_OBJECT_STATECHANGE, hwnd, OBJID_CLIENT, CHILDID_SELF);
        }
    }
}

/// 視窗結束前拿掉覆寫的屬性（WM_NCDESTROY）
pub fn clear(hwnd: HWND) {
    if let Some((this, vt)) = services() {
        let props = [PROPID_ACC_NAME, PROPID_ACC_ROLE, PROPID_ACC_STATE];
        unsafe {
            (vt.clear_hwnd_props)(this, hwnd, OBJID_CLIENT as u32, CHILDID_SELF as u32, props.as_ptr(), props.len() as i32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        // IUnknown 3 + SetPropValue、SetPropServer、ClearProps、SetHwndProp、SetHwndPropStr、SetHwndPropServer、ClearHwndProps
        assert_eq!(std::mem::size_of::<ServicesVtbl>(), 10 * 8);
        assert_eq!(std::mem::offset_of!(ServicesVtbl, set_hwnd_prop), 6 * 8);
        assert_eq!(std::mem::offset_of!(ServicesVtbl, clear_hwnd_props), 9 * 8);
        // VARIANT 和 PROPVARIANT 的配置相同：型別 2 位元組、保留 6 位元組、資料 16 位元組
        assert_eq!(std::mem::size_of::<PROPVARIANT>(), 24);
        let v = int_variant(ROLE_CHECKBUTTON);
        assert_eq!((v.vt, v.data[0]), (VT_I4, 0x2C));
    }
}

//! 安裝程式另外需要的 Win32 宣告（主程式的 sys.rs 沒有的部分）：行程、權杖、捷徑與選擇資料夾的 COM 介面。
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms, dead_code)]

use super::sys::*;
use std::ffi::c_void;

pub const WS_OVERLAPPED: u32 = 0;
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
pub const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
pub const INFINITE: u32 = 0xFFFF_FFFF;
pub const WAIT_OBJECT_0: u32 = 0;
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const TOKEN_QUERY: u32 = 0x0008;
pub const TOKEN_ELEVATION_CLASS: u32 = 20;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const ERROR_CANCELLED: u32 = 1223;
pub const ERROR_SHARING_VIOLATION: u32 = 32;
pub const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
pub const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
pub const FOS_PICKFOLDERS: u32 = 0x20;
pub const FOS_FORCEFILESYSTEM: u32 = 0x40;
pub const FOS_PATHMUSTEXIST: u32 = 0x800;
pub const SIGDN_FILESYSPATH: u32 = 0x8005_8000;

pub const CLSID_FILE_OPEN_DIALOG: GUID = GUID::from_u128(0xDC1C5A9C_E88A_4DDE_A5A1_60F82A20AEF7);
pub const IID_IFILE_OPEN_DIALOG: GUID = GUID::from_u128(0xD57C7288_D4AD_4768_BE02_9D969532D960);
pub const IID_ISHELL_ITEM: GUID = GUID::from_u128(0x43826D1E_E718_42EE_BC55_A1E261C37BFE);
pub const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_C000_000000000046);
pub const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214F9_0000_0000_C000_000000000046);
pub const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010B_0000_0000_C000_000000000046);
pub const FOLDERID_PROGRAMS: GUID = GUID::from_u128(0xA77F5D77_2E2B_44C3_A6A2_ABA601054A51);
pub const FOLDERID_DESKTOP: GUID = GUID::from_u128(0xB4BFCC3A_DB2C_424C_B029_7FE99A87C641);
pub const FOLDERID_LOCAL_APP_DATA: GUID = GUID::from_u128(0xF1B32785_6FBA_4FCF_9D55_7B8E7F157091);

#[repr(C)]
pub struct STARTUPINFOW {
    pub cb: u32,
    pub lpReserved: *mut u16,
    pub lpDesktop: *mut u16,
    pub lpTitle: *mut u16,
    pub dwX: u32,
    pub dwY: u32,
    pub dwXSize: u32,
    pub dwYSize: u32,
    pub dwXCountChars: u32,
    pub dwYCountChars: u32,
    pub dwFillAttribute: u32,
    pub dwFlags: u32,
    pub wShowWindow: u16,
    pub cbReserved2: u16,
    pub lpReserved2: *mut u8,
    pub hStdInput: HANDLE,
    pub hStdOutput: HANDLE,
    pub hStdError: HANDLE,
}

#[repr(C)]
pub struct PROCESS_INFORMATION {
    pub hProcess: HANDLE,
    pub hThread: HANDLE,
    pub dwProcessId: u32,
    pub dwThreadId: u32,
}

#[cfg_attr(windows, link(name = "kernel32", kind = "raw-dylib"))]
extern "system" {
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    pub fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: BOOL) -> BOOL;
    pub fn DeleteFileW(path: *const u16) -> BOOL;
    pub fn RemoveDirectoryW(path: *const u16) -> BOOL;
    pub fn CreateProcessW(
        app: *const u16,
        cmdline: *mut u16,
        process_attrs: *const c_void,
        thread_attrs: *const c_void,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        cwd: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    pub fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    pub fn GetCurrentProcess() -> HANDLE;
}

#[cfg_attr(windows, link(name = "advapi32", kind = "raw-dylib"))]
extern "system" {
    pub fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    pub fn GetTokenInformation(token: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> BOOL;
}

#[cfg_attr(windows, link(name = "gdi32", kind = "raw-dylib"))]
extern "system" {
    pub fn MoveToEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    pub fn LineTo(hdc: HDC, x: i32, y: i32) -> BOOL;
}

#[cfg_attr(windows, link(name = "user32", kind = "raw-dylib"))]
extern "system" {
    pub fn AdjustWindowRectExForDpi(rect: *mut RECT, style: u32, menu: BOOL, ex_style: u32, dpi: u32) -> BOOL;
}

// ---------------------------------------------------------------- COM 介面（只宣告用到的方法，其他位置留空）

type Slot = *const c_void;

#[repr(C)]
pub struct IUnknownVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
pub struct IShellItemVtbl {
    pub base: IUnknownVtbl,
    pub BindToHandler: Slot,
    pub GetParent: Slot,
    pub GetDisplayName: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
    pub GetAttributes: Slot,
    pub Compare: Slot,
}

/// IFileOpenDialog（IModalWindow → IFileDialog → IFileOpenDialog），到 GetResult 為止
#[repr(C)]
pub struct IFileOpenDialogVtbl {
    pub base: IUnknownVtbl,
    pub Show: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    pub SetFileTypes: Slot,
    pub SetFileTypeIndex: Slot,
    pub GetFileTypeIndex: Slot,
    pub Advise: Slot,
    pub Unadvise: Slot,
    pub SetOptions: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    pub GetOptions: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    pub SetDefaultFolder: Slot,
    pub SetFolder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub GetFolder: Slot,
    pub GetCurrentSelection: Slot,
    pub SetFileName: Slot,
    pub GetFileName: Slot,
    pub SetTitle: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub SetOkButtonLabel: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub SetFileNameLabel: Slot,
    pub GetResult: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub struct IShellLinkWVtbl {
    pub base: IUnknownVtbl,
    pub GetPath: Slot,
    pub GetIDList: Slot,
    pub SetIDList: Slot,
    pub GetDescription: Slot,
    pub SetDescription: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub GetWorkingDirectory: Slot,
    pub SetWorkingDirectory: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub GetArguments: Slot,
    pub SetArguments: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub GetHotkey: Slot,
    pub SetHotkey: Slot,
    pub GetShowCmd: Slot,
    pub SetShowCmd: Slot,
    pub GetIconLocation: Slot,
    pub SetIconLocation: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
    pub SetRelativePath: Slot,
    pub Resolve: Slot,
    pub SetPath: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
}

#[repr(C)]
pub struct IPersistFileVtbl {
    pub base: IUnknownVtbl,
    pub GetClassID: Slot,
    pub IsDirty: Slot,
    pub Load: Slot,
    pub Save: unsafe extern "system" fn(*mut c_void, *const u16, BOOL) -> HRESULT,
}

/// 取得 COM 物件的函式表
pub unsafe fn vtbl<'a, T>(obj: *mut c_void) -> &'a T {
    &**(obj as *mut *const T)
}

pub unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        (vtbl::<IUnknownVtbl>(obj).Release)(obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn layouts() {
        assert_eq!(size_of::<STARTUPINFOW>(), 104);
        assert_eq!(size_of::<PROCESS_INFORMATION>(), 24);
        // 函式表的位置：IFileDialog::GetResult 是第 20 個、IShellLinkW::SetPath 是第 20 個、IPersistFile::Save 是第 6 個
        assert_eq!(size_of::<IFileOpenDialogVtbl>() / 8, 21);
        assert_eq!(size_of::<IShellLinkWVtbl>() / 8, 21);
        assert_eq!(size_of::<IPersistFileVtbl>() / 8, 7);
        assert_eq!(size_of::<IShellItemVtbl>() / 8, 8);
    }
}

//! StillfeatherPicker.dll：讓其他程式的「開啟檔案」「另存新檔」「選擇資料夾」視窗改用 Stillfeather。
//!
//! 在「設定」打開這個功能時，Stillfeather 在目前使用者的登錄（HKCU\Software\Classes\CLSID）把
//! Windows「開啟檔案」視窗（CLSID_FileOpenDialog）指到這個 DLL，「另存新檔」視窗（CLSID_FileSaveDialog）
//! 指到旁邊另一個檔名的同一份 DLL（StillfeatherSavePicker.dll，原因見 src/install.rs 的 picker_save_server_key）。
//! 之後其他程式建立這兩種視窗時，拿到的是這裡的物件：
//!
//! - 它同時建立一個 Windows 原本的視窗物件（從 HKLM 登記的 comdlg32.dll 直接取得，不經過 COM 查表，
//!   所以不會又繞回這裡），程式做的每一個設定都照樣轉交給它。
//! - 程式呼叫 Show 時，把設定寫成請求檔，啟動 `Stillfeather.exe --pick <請求檔>`（同一個資料夾裡的那一份），
//!   等使用者選好（期間照常處理程式的視窗訊息），再把結果以 IShellItem 交回，並照 Windows 的做法通知程式（OnFileOk）。
//! - 找不到 Stillfeather、它無法啟動、使用者按「使用 Windows 視窗」、要選的不是一般檔案（例如手機、媒體櫃）、
//!   程式在視窗裡加了自己的選項（IFileDialogCustomize 的勾選方塊、下拉選單、按鈕，例如「編碼」「以唯讀開啟」），
//!   存檔時要讓使用者填檔案內容（SetProperties、SetCollectedProperties 的作者、標籤欄），
//!   或是 Stillfeather 自己的程式要開檔時，就直接顯示 Windows 原本的視窗：程式永遠可以選到檔案，也不會少了它的選項。
//! - 另存新檔時要存的檔案通常還不存在：結果以「簡單的」IShellItem（SHSimpleIDListFromPath）交回，
//!   和 Windows 的視窗一樣可以取得它的路徑。檔案已經存在時的「要取代嗎？」由 Stillfeather 問（不呼叫程式的 OnOverwrite）。
//!
//! 每個 COM 方法都接住 panic 並回傳錯誤（這個 DLL 是在別人的程式裡執行）。只支援 64 位元：
//! IFileDialogCustomize 以「最多三個暫存器參數」的方式原封轉交，這在 x64 的呼叫慣例下才成立。
//! 32 位元程式讀的是另一個登錄位置，會照常使用 Windows 的視窗。

#![allow(non_snake_case, clippy::missing_safety_doc, clippy::upper_case_acronyms)]

#[cfg(not(target_pointer_width = "64"))]
compile_error!("StillfeatherPicker.dll 只支援 64 位元。");

// src/paths.rs、src/picker.rs 用到 tr!（這個元件沒有畫面，一律是中文）
#[path = "../../src/i18n.rs"]
#[macro_use]
#[allow(dead_code, unused_macros)]
mod i18n;
#[path = "../../src/paths.rs"]
#[allow(dead_code)]
mod paths;
#[path = "../../src/picker.rs"]
#[allow(dead_code)]
mod protocol;

use protocol::{FileType, Mode, Request, Response};
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

// ---------------------------------------------------------------- Win32 宣告

type HWND = isize;
type HANDLE = isize;
type HMODULE = isize;
type HRESULT = i32;
type BOOL = i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

const CLSID_FILE_OPEN_DIALOG: GUID = guid(0xDC1C5A9C, 0xE88A, 0x4DDE, [0xA5, 0xA1, 0x60, 0xF8, 0x2A, 0x20, 0xAE, 0xF7]);
const CLSID_FILE_SAVE_DIALOG: GUID = guid(0xC0B4E2F3, 0xBA21, 0x4773, [0x8D, 0xBA, 0x33, 0x5E, 0xC9, 0x46, 0xEB, 0x8B]);
const IID_IUNKNOWN: GUID = guid(0x00000000, 0x0000, 0x0000, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ICLASSFACTORY: GUID = guid(0x00000001, 0x0000, 0x0000, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMODALWINDOW: GUID = guid(0xB4DB1657, 0x70D7, 0x485E, [0x8E, 0x3E, 0x6F, 0xCB, 0x5A, 0x5C, 0x18, 0x02]);
const IID_IFILEDIALOG: GUID = guid(0x42F85136, 0xDB7E, 0x439C, [0x85, 0xF1, 0xE4, 0x07, 0x5D, 0x13, 0x5F, 0xC8]);
const IID_IFILEOPENDIALOG: GUID = guid(0xD57C7288, 0xD4AD, 0x4768, [0xBE, 0x02, 0x9D, 0x96, 0x95, 0x32, 0xD9, 0x60]);
const IID_IFILESAVEDIALOG: GUID = guid(0x84BCCD23, 0x5FDE, 0x4CDB, [0xAE, 0xA4, 0xAF, 0x64, 0xB8, 0x3D, 0x78, 0xAB]);
const IID_IFILEDIALOGCUSTOMIZE: GUID = guid(0xE6FDD21A, 0x163F, 0x4975, [0x9C, 0x8C, 0xA6, 0x9F, 0x1B, 0xA3, 0x70, 0x34]);
const IID_ISHELLITEM: GUID = guid(0x43826D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);

const S_OK: HRESULT = 0;
const E_NOTIMPL: HRESULT = 0x8000_4001_u32 as i32;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003_u32 as i32;
const E_FAIL: HRESULT = 0x8000_4005_u32 as i32;
const E_UNEXPECTED: HRESULT = 0x8000_FFFF_u32 as i32;
const E_OUTOFMEMORY: HRESULT = 0x8007_000E_u32 as i32;
const CLASS_E_NOAGGREGATION: HRESULT = 0x8004_0110_u32 as i32;
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = 0x8004_0111_u32 as i32;
/// HRESULT_FROM_WIN32(ERROR_CANCELLED)：使用者取消
const E_CANCELLED: HRESULT = 0x8007_04C7_u32 as i32;
const S_FALSE: HRESULT = 1;

const FOS_OVERWRITEPROMPT: u32 = 0x2;
const FOS_STRICTFILETYPES: u32 = 0x4;
const FOS_PICKFOLDERS: u32 = 0x20;
const FOS_ALLNONSTORAGEITEMS: u32 = 0x80;
const FOS_ALLOWMULTISELECT: u32 = 0x200;
const FOS_FILEMUSTEXIST: u32 = 0x1000;
const FOS_NOREADONLYRETURN: u32 = 0x8000;
const FOS_FORCESHOWHIDDEN: u32 = 0x1000_0000;
/// 開啟檔案視窗的預設選項（FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST）
const FOS_OPEN_DEFAULT: u32 = 0x1808;
/// 另存新檔視窗的預設選項（FOS_OVERWRITEPROMPT | FOS_NOCHANGEDIR | FOS_PATHMUSTEXIST | FOS_NOREADONLYRETURN）
const FOS_SAVE_DEFAULT: u32 = 0x880A;
const SIGDN_FILESYSPATH: u32 = 0x8005_8000;

const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
const RRF_RT_REG_SZ: u32 = 0x0000_0002;
const RRF_RT_REG_EXPAND_SZ: u32 = 0x0000_0004;
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
const GA_ROOT: u32 = 2;
const QS_ALLINPUT: u32 = 0x04FF;
const MWMO_INPUTAVAILABLE: u32 = 0x0004;
const PM_REMOVE: u32 = 1;
const WM_QUIT: u32 = 0x0012;
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;

#[repr(C)]
struct Msg {
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt_x: i32,
    pt_y: i32,
    private: u32,
}

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_chars: u32,
    y_chars: u32,
    fill: u32,
    flags: u32,
    show_window: u16,
    reserved2_len: u16,
    reserved2: *mut u8,
    std_input: HANDLE,
    std_output: HANDLE,
    std_error: HANDLE,
}

#[repr(C)]
struct ProcessInformation {
    process: HANDLE,
    thread: HANDLE,
    process_id: u32,
    thread_id: u32,
}

/// COMDLG_FILTERSPEC
#[repr(C)]
struct FilterSpec {
    name: *const u16,
    spec: *const u16,
}

#[cfg_attr(windows, link(name = "kernel32", kind = "raw-dylib"))]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> HMODULE;
    fn GetProcAddress(module: HMODULE, name: *const u8) -> *const c_void;
    fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut HMODULE) -> BOOL;
    fn GetModuleFileNameW(module: HMODULE, buf: *mut u16, size: u32) -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, size: u32) -> u32;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        process_attrs: *const c_void,
        thread_attrs: *const c_void,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        cwd: *const u16,
        startup: *const StartupInfo,
        info: *mut ProcessInformation,
    ) -> BOOL;
    fn GetExitCodeProcess(process: HANDLE, code: *mut u32) -> BOOL;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount64() -> u64;
}

#[cfg_attr(windows, link(name = "user32", kind = "raw-dylib"))]
extern "system" {
    fn IsWindow(hwnd: HWND) -> BOOL;
    fn IsWindowEnabled(hwnd: HWND) -> BOOL;
    fn EnableWindow(hwnd: HWND, enable: BOOL) -> BOOL;
    fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    fn AllowSetForegroundWindow(pid: u32) -> BOOL;
    fn GetAncestor(hwnd: HWND, flags: u32) -> HWND;
    fn MsgWaitForMultipleObjectsEx(count: u32, handles: *const HANDLE, ms: u32, mask: u32, flags: u32) -> u32;
    fn PeekMessageW(msg: *mut Msg, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const Msg) -> BOOL;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn PostQuitMessage(code: i32);
}

#[cfg_attr(windows, link(name = "advapi32", kind = "raw-dylib"))]
extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
}

#[cfg_attr(windows, link(name = "ole32", kind = "raw-dylib"))]
extern "system" {
    fn CoTaskMemAlloc(size: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
}

#[cfg_attr(windows, link(name = "shell32", kind = "raw-dylib"))]
extern "system" {
    fn SHCreateItemFromParsingName(path: *const u16, bc: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    fn SHParseDisplayName(name: *const u16, bc: *mut c_void, pidl: *mut *mut c_void, sfgao_in: u32, sfgao_out: *mut u32) -> HRESULT;
    fn SHCreateShellItemArrayFromIDLists(count: u32, pidls: *const *const c_void, out: *mut *mut c_void) -> HRESULT;
    fn SHCreateItemFromIDList(pidl: *const c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 && len < 65_536 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

fn from_wide_buf(buf: &[u16]) -> String {
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

// ---------------------------------------------------------------- 一般的 COM 介面呼叫

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

unsafe fn unknown_vtbl<'a>(p: *mut c_void) -> &'a UnknownVtbl {
    &**(p as *const *const UnknownVtbl)
}

unsafe fn release(p: *mut c_void) {
    if !p.is_null() {
        (unknown_vtbl(p).release)(p);
    }
}

unsafe fn add_ref(p: *mut c_void) {
    if !p.is_null() {
        (unknown_vtbl(p).add_ref)(p);
    }
}

/// 第 n 格的函式（0、1、2 是 IUnknown）
unsafe fn slot(p: *mut c_void, n: usize) -> *const c_void {
    let vt = *(p as *const *const *const c_void);
    *vt.add(n)
}

#[repr(C)]
struct ClassFactoryVtbl {
    base: UnknownVtbl,
    create_instance: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    lock_server: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
}

/// IShellItem 的檔案系統路徑（不是檔案系統項目時回傳 None）
unsafe fn shell_item_path(item: *mut c_void) -> Option<String> {
    if item.is_null() {
        return None;
    }
    type GetDisplayName = unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT;
    let f: GetDisplayName = std::mem::transmute::<*const c_void, GetDisplayName>(slot(item, 5));
    let mut name: *mut u16 = std::ptr::null_mut();
    if f(item, SIGDN_FILESYSPATH, &mut name) < 0 || name.is_null() {
        return None;
    }
    let s = from_wide_ptr(name);
    CoTaskMemFree(name as *mut c_void);
    (!s.is_empty()).then_some(s)
}

unsafe fn item_from_path(path: &str, out: *mut *mut c_void) -> HRESULT {
    let w = wide(path);
    SHCreateItemFromParsingName(w.as_ptr(), std::ptr::null_mut(), &IID_ISHELLITEM, out)
}

type SimpleIdListFn = unsafe extern "system" fn(*const u16) -> *mut c_void;

/// shell32 的 SHSimpleIDListFromPath。執行時才找（以名稱，找不到再以序號 162）：
/// 萬一這一版 Windows 沒有以名稱匯出，也只是另存新檔改用 Windows 的視窗，不會讓這個 DLL 整個載入失敗。
fn simple_id_list_fn() -> Option<SimpleIdListFn> {
    static CACHED: OnceLock<Option<usize>> = OnceLock::new();
    let p = *CACHED.get_or_init(|| unsafe {
        let module = LoadLibraryW(wide("shell32.dll").as_ptr());
        if module == 0 {
            return None;
        }
        let mut f = GetProcAddress(module, c"SHSimpleIDListFromPath".as_ptr().cast());
        if f.is_null() {
            f = GetProcAddress(module, 162usize as *const u8);
        }
        (!f.is_null()).then_some(f as usize)
    });
    p.map(|p| unsafe { std::mem::transmute::<usize, SimpleIdListFn>(p) })
}

/// 另存新檔的結果：檔案已經存在就照一般的方式，還不存在（通常是這樣）就建立「簡單的」項目，和 Windows 的視窗一樣。
unsafe fn item_for_save(path: &str, out: *mut *mut c_void) -> HRESULT {
    let hr = item_from_path(path, out);
    if hr >= 0 {
        return hr;
    }
    let Some(simple) = simple_id_list_fn() else { return hr };
    let w = wide(path);
    let pidl = simple(w.as_ptr());
    if pidl.is_null() {
        return hr;
    }
    let created = SHCreateItemFromIDList(pidl, &IID_ISHELLITEM, out);
    CoTaskMemFree(pidl);
    created
}

unsafe fn co_string(s: &str) -> *mut u16 {
    let w = wide(s);
    let p = CoTaskMemAlloc(w.len() * 2) as *mut u16;
    if !p.is_null() {
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
    }
    p
}

/// 每個 COM 方法的外殼：接住 panic，回傳錯誤，不讓別人的程式當掉。
fn guard(f: impl FnOnce() -> HRESULT) -> HRESULT {
    static QUIET: OnceLock<()> = OnceLock::new();
    QUIET.get_or_init(|| std::panic::set_hook(Box::new(|_| {})));
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(E_FAIL)
}

// ---------------------------------------------------------------- Windows 原本的視窗

type DllGetClassObjectFn = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

/// 取代的是哪一種視窗
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// 開啟檔案、選擇資料夾（IFileOpenDialog）
    Open,
    /// 另存新檔（IFileSaveDialog）
    Save,
}

impl Kind {
    fn clsid(self) -> GUID {
        match self {
            Kind::Open => CLSID_FILE_OPEN_DIALOG,
            Kind::Save => CLSID_FILE_SAVE_DIALOG,
        }
    }

    fn clsid_text(self) -> &'static str {
        match self {
            Kind::Open => protocol::CLSID_FILE_OPEN_DIALOG,
            Kind::Save => protocol::CLSID_FILE_SAVE_DIALOG,
        }
    }

    fn iid(self) -> GUID {
        match self {
            Kind::Open => IID_IFILEOPENDIALOG,
            Kind::Save => IID_IFILESAVEDIALOG,
        }
    }
}

/// HKLM 登記的「開啟檔案」或「另存新檔」視窗元件（通常是 System32\comdlg32.dll）的 DllGetClassObject
fn system_class_object(kind: Kind) -> Option<DllGetClassObjectFn> {
    static OPEN: OnceLock<Option<usize>> = OnceLock::new();
    static SAVE: OnceLock<Option<usize>> = OnceLock::new();
    let cache = if kind == Kind::Open { &OPEN } else { &SAVE };
    let p = *cache.get_or_init(|| unsafe {
        let mut candidates = Vec::new();
        let key = wide(&format!(r"SOFTWARE\Classes\CLSID\{}\InprocServer32", kind.clsid_text()));
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // RegGetValueW 會展開 %SystemRoot%
        if RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) == 0 {
            let path = from_wide_buf(&buf);
            // 萬一 HKLM 也指到 Stillfeather（不應該發生），不要載入自己
            let name = paths::file_name(&path);
            if !name.eq_ignore_ascii_case(protocol::DLL_NAME) && !name.eq_ignore_ascii_case(protocol::SAVE_DLL_NAME) {
                candidates.push(path);
            }
        }
        let mut sys = vec![0u16; 512];
        let n = GetSystemDirectoryW(sys.as_mut_ptr(), sys.len() as u32) as usize;
        if n > 0 && n < sys.len() {
            candidates.push(paths::join(&String::from_utf16_lossy(&sys[..n]), "comdlg32.dll"));
        }
        for path in candidates {
            let w = wide(&path);
            let module = LoadLibraryW(w.as_ptr());
            if module == 0 {
                continue;
            }
            let f = GetProcAddress(module, c"DllGetClassObject".as_ptr().cast());
            if !f.is_null() {
                return Some(f as usize);
            }
        }
        None
    });
    p.map(|p| unsafe { std::mem::transmute::<usize, DllGetClassObjectFn>(p) })
}

/// 建立 Windows 原本的「開啟檔案」或「另存新檔」視窗物件
unsafe fn create_system_dialog(kind: Kind) -> *mut c_void {
    let Some(get) = system_class_object(kind) else { return std::ptr::null_mut() };
    let mut factory: *mut c_void = std::ptr::null_mut();
    if get(&kind.clsid(), &IID_ICLASSFACTORY, &mut factory) < 0 || factory.is_null() {
        return std::ptr::null_mut();
    }
    let vt = &**(factory as *const *const ClassFactoryVtbl);
    let mut dialog: *mut c_void = std::ptr::null_mut();
    let hr = (vt.create_instance)(factory, std::ptr::null_mut(), &kind.iid(), &mut dialog);
    release(factory);
    if hr < 0 {
        return std::ptr::null_mut();
    }
    dialog
}

// ---------------------------------------------------------------- 找 Stillfeather

fn module_path(module: HMODULE) -> Option<String> {
    let mut buf = vec![0u16; 32_768];
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0 && n < buf.len()).then(|| String::from_utf16_lossy(&buf[..n]))
}

/// 這個 DLL 所在的資料夾
fn dll_dir() -> Option<String> {
    let mut module: HMODULE = 0;
    let ok = unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, dll_dir as *const c_void, &mut module) };
    if ok == 0 {
        return None;
    }
    paths::parent(&module_path(module)?)
}

/// 讀目前使用者的字串登錄值
fn read_hkcu(key: &str, name: &str) -> Option<String> {
    let k = wide(key);
    let n = wide(name);
    let mut buf = vec![0u16; 32_768];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe { RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    (rc == 0).then(|| from_wide_buf(&buf)).filter(|s| !s.is_empty())
}

/// 要啟動的 Stillfeather：設定開啟時記下的那一份（登錄 HKCU\Software\Stillfeather 的 PickerExe）；
/// 沒有記錄時找 DLL 旁邊的（安裝版 Stillfeather.exe，開發版 FeatherFiles.exe）。都不在就回傳 None（改用 Windows 的視窗）。
fn stillfeather_exe() -> Option<String> {
    if let Some(exe) = read_hkcu(r"Software\Stillfeather", "PickerExe") {
        if std::path::Path::new(&exe).is_file() {
            return Some(exe);
        }
    }
    let dir = dll_dir()?;
    ["Stillfeather.exe", "FeatherFiles.exe"].iter().map(|n| paths::join(&dir, n)).find(|p| std::path::Path::new(p).is_file())
}

/// 要開視窗的程式（exe 檔名）
fn host_name() -> String {
    module_path(0).map(|p| paths::file_name(&p).to_string()).unwrap_or_default()
}

/// 這些情況直接用 Windows 原本的視窗
fn use_system_dialog(options: u32) -> bool {
    // 要選的不是檔案系統裡的項目（手機、媒體櫃等）
    if options & FOS_ALLNONSTORAGEITEMS != 0 {
        return true;
    }
    // Stillfeather 自己的程式（例如安裝程式選安裝位置）；以及 Windows 殼層本身（「執行」的瀏覽…）：
    // 殼層出問題會連工作列一起重新啟動，不值得冒這個險
    let host = host_name().to_ascii_lowercase();
    if host.starts_with("stillfeather") || host.starts_with("featherfiles") || host == "explorer.exe" {
        return true;
    }
    // 偵錯用：設環境變數 STILLFEATHER_PICKER=off 就一律用 Windows 的視窗
    std::env::var("STILLFEATHER_PICKER").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
}

// ---------------------------------------------------------------- 取代的視窗物件

#[derive(Default)]
enum Shown {
    /// 還沒顯示過（或取消了）
    #[default]
    None,
    /// 使用者在 Stillfeather 裡選好了
    Ours { paths: Vec<String> },
    /// 顯示的是 Windows 原本的視窗：之後的查詢都轉交給它
    System,
}

#[derive(Default)]
struct State {
    options: u32,
    title: String,
    ok_label: String,
    name_label: String,
    folder: String,
    default_folder: String,
    file_name: String,
    default_ext: String,
    types: Vec<FileType>,
    type_index: u32,
    /// 程式登記的事件接收者（cookie, IFileDialogEvents）
    sinks: Vec<(u32, *mut c_void)>,
    next_cookie: u32,
    shown: Shown,
    /// 程式加了自己的選項（勾選方塊、下拉選單…）：Stillfeather 的視窗顯示不出來，改用 Windows 的視窗
    custom_controls: bool,
    /// 另存新檔時程式要讓使用者填檔案內容（作者、標籤…）：同樣改用 Windows 的視窗
    properties: bool,
}

#[repr(C)]
struct Dialog {
    /// OPEN_VTBL 或 SAVE_VTBL（兩者開頭都是 IFileDialog 的方法）
    vtbl: *const DialogVtbl,
    custom_vtbl: *const CustomizeVtbl,
    refs: AtomicU32,
    kind: Kind,
    /// Windows 原本的視窗物件（IFileOpenDialog 或 IFileSaveDialog，和 kind 一致）；建立失敗時是 null，只用 Stillfeather
    real: *mut c_void,
    /// 它的 IFileDialogCustomize（程式加的勾選方塊、下拉選單都交給它保存）
    real_custom: *mut c_void,
    state: UnsafeCell<State>,
}

impl Dialog {
    unsafe fn state<'a>(&self) -> &'a mut State {
        &mut *self.state.get()
    }

    fn real_vtbl(&self) -> Option<&DialogVtbl> {
        if self.real.is_null() {
            None
        } else {
            Some(unsafe { &**(self.real as *const *const DialogVtbl) })
        }
    }

    /// Windows 原本的物件的 IFileOpenDialog 方法（只有開啟檔案視窗有）
    fn real_open_vtbl(&self) -> Option<&OpenVtbl> {
        (self.kind == Kind::Open && !self.real.is_null()).then(|| unsafe { &**(self.real as *const *const OpenVtbl) })
    }

    /// Windows 原本的物件的 IFileSaveDialog 方法（只有另存新檔視窗有）
    fn real_save_vtbl(&self) -> Option<&SaveVtbl> {
        (self.kind == Kind::Save && !self.real.is_null()).then(|| unsafe { &**(self.real as *const *const SaveVtbl) })
    }
}

unsafe fn dialog<'a>(this: *mut c_void) -> &'a Dialog {
    &*(this as *const Dialog)
}

unsafe fn dialog_from_custom<'a>(this: *mut c_void) -> &'a Dialog {
    &*((this as *mut u8).sub(std::mem::offset_of!(Dialog, custom_vtbl)) as *const Dialog)
}

#[repr(C)]
struct DialogVtbl {
    base: UnknownVtbl,
    show: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    set_file_types: unsafe extern "system" fn(*mut c_void, u32, *const FilterSpec) -> HRESULT,
    set_file_type_index: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_file_type_index: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    advise: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    set_options: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    get_options: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    set_default_folder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_folder: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    get_folder: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_current_selection: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    set_file_name: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    get_file_name: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HRESULT,
    set_title: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    set_ok_button_label: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    set_file_name_label: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    add_place: unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> HRESULT,
    set_default_extension: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    close: unsafe extern "system" fn(*mut c_void, HRESULT) -> HRESULT,
    set_client_guid: unsafe extern "system" fn(*mut c_void, *const GUID) -> HRESULT,
    clear_client_data: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    set_filter: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

/// IFileOpenDialog：IFileDialog 再加兩個方法
#[repr(C)]
struct OpenVtbl {
    dialog: DialogVtbl,
    get_results: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    get_selected_items: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// IFileSaveDialog：IFileDialog 再加五個方法
#[repr(C)]
struct SaveVtbl {
    dialog: DialogVtbl,
    set_save_as_item: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_properties: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    set_collected_properties: unsafe extern "system" fn(*mut c_void, *mut c_void, BOOL) -> HRESULT,
    get_properties: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    apply_properties: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, HWND, *mut c_void) -> HRESULT,
}

static OPEN_VTBL: OpenVtbl = OpenVtbl { dialog: DIALOG_METHODS, get_results, get_selected_items };

static SAVE_VTBL: SaveVtbl = SaveVtbl { dialog: DIALOG_METHODS, set_save_as_item, set_properties, set_collected_properties, get_properties, apply_properties };

/// 兩種視窗共用的 IFileDialog 方法
const DIALOG_METHODS: DialogVtbl = DialogVtbl {
    base: UnknownVtbl { query_interface, add_ref: dialog_add_ref, release: dialog_release },
    show,
    set_file_types,
    set_file_type_index,
    get_file_type_index,
    advise,
    unadvise,
    set_options,
    get_options,
    set_default_folder,
    set_folder,
    get_folder,
    get_current_selection,
    set_file_name,
    get_file_name,
    set_title,
    set_ok_button_label,
    set_file_name_label,
    get_result,
    add_place,
    set_default_extension,
    close,
    set_client_guid,
    clear_client_data,
    set_filter,
};

unsafe extern "system" fn query_interface(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() || riid.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        let iid = *riid;
        if iid == IID_IUNKNOWN || iid == IID_IMODALWINDOW || iid == IID_IFILEDIALOG || iid == d.kind.iid() {
            *out = this;
        } else if iid == IID_IFILEDIALOGCUSTOMIZE && !d.real_custom.is_null() {
            *out = std::ptr::addr_of!(d.custom_vtbl) as *mut c_void;
        } else if let Some(vt) = d.real_vtbl() {
            // 其他介面（例如 IFileDialog2）交給 Windows 原本的物件：程式若透過它顯示視窗，就是 Windows 的視窗
            return (vt.base.query_interface)(d.real, riid, out);
        } else {
            return E_NOINTERFACE;
        }
        dialog_add_ref(this);
        S_OK
    })
}

unsafe extern "system" fn dialog_add_ref(this: *mut c_void) -> u32 {
    dialog(this).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn dialog_release(this: *mut c_void) -> u32 {
    let left = dialog(this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let d = Box::from_raw(this as *mut Dialog);
            for (_, sink) in d.state().sinks.drain(..) {
                release(sink);
            }
            release(d.real_custom);
            release(d.real);
        }));
    }
    left
}

/// 目前的選項（Windows 原本的物件有就問它，否則用自己記的）
unsafe fn current_options(d: &Dialog) -> u32 {
    if let Some(vt) = d.real_vtbl() {
        let mut o = 0u32;
        if (vt.get_options)(d.real, &mut o) >= 0 {
            return o;
        }
    }
    d.state().options
}

unsafe extern "system" fn show(this: *mut c_void, owner: HWND) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        // 狀態每次重新取用，不跨過會呼叫回程式的地方（程式可能在事件裡再呼叫這個物件）
        d.state().shown = Shown::None;
        let options = current_options(d);
        let (custom_controls, properties) = (d.state().custom_controls, d.state().properties);
        // 另存新檔時要填檔案內容，或是選資料夾（不是一般的用法），或是沒辦法為還不存在的檔案建立結果：也交給 Windows
        let unusual_save = d.kind == Kind::Save && (properties || options & FOS_PICKFOLDERS != 0 || simple_id_list_fn().is_none());
        if use_system_dialog(options) || custom_controls || unusual_save {
            return show_system(d, owner);
        }
        // 程式在 OnFileOk 拒絕時（例如檔案不符合它的要求）重新選，最多幾次
        for _ in 0..5 {
            match run_picker(d, owner, options) {
                Outcome::Picked(response) => {
                    if response.type_index > 0 {
                        d.state().type_index = response.type_index;
                        if let Some(vt) = d.real_vtbl() {
                            (vt.set_file_type_index)(d.real, response.type_index);
                        }
                    }
                    d.state().shown = Shown::Ours { paths: response.paths };
                    if file_ok(this, d) {
                        return S_OK;
                    }
                    d.state().shown = Shown::None;
                }
                Outcome::Cancelled => return E_CANCELLED,
                Outcome::UseSystem => return show_system(d, owner),
            }
        }
        E_CANCELLED
    })
}

/// 顯示 Windows 原本的視窗
unsafe fn show_system(d: &Dialog, owner: HWND) -> HRESULT {
    match d.real_vtbl() {
        Some(vt) => {
            d.state().shown = Shown::System;
            (vt.show)(d.real, owner)
        }
        None => E_FAIL,
    }
}

/// 照 Windows 的做法通知程式「使用者按了開啟」：每個事件接收者都問，有一個拒絕就不算選定。
unsafe fn file_ok(this: *mut c_void, d: &Dialog) -> bool {
    type OnFileOk = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT;
    let sinks: Vec<*mut c_void> = d.state().sinks.iter().map(|(_, s)| *s).collect();
    for sink in sinks {
        add_ref(sink);
        let f: OnFileOk = std::mem::transmute::<*const c_void, OnFileOk>(slot(sink, 3));
        let hr = f(sink, this);
        release(sink);
        if !file_ok_accepts(hr) {
            return false;
        }
    }
    true
}

/// OnFileOk 的回傳值算不算接受：只有 S_FALSE 是拒絕（Windows 的視窗也是這樣）。
/// 很多程式不處理這個事件而回傳 E_NOTIMPL，例如 Chrome、Edge 與 Electron 程式（AutoCloseDialogEventHandler）；
/// 把它當成拒絕的話，上傳時按「開啟」會一直重新跳出選擇器。
fn file_ok_accepts(hr: HRESULT) -> bool {
    hr != S_FALSE
}

enum Outcome {
    Picked(Response),
    Cancelled,
    UseSystem,
}

/// 啟動 Stillfeather 的選擇器並等它結束（期間照常處理這個程式的視窗訊息）。
unsafe fn run_picker(d: &Dialog, owner: HWND, options: u32) -> Outcome {
    let Some(exe) = stillfeather_exe() else { return Outcome::UseSystem };
    let root = if owner != 0 && IsWindow(owner) != 0 { GetAncestor(owner, GA_ROOT) } else { 0 };
    let root = if root != 0 { root } else { owner };
    // 和 Windows 的視窗一樣：選擇期間程式的視窗不能操作。程式自己已經停用它時不動（之後也不恢復）
    let disable = root != 0 && IsWindowEnabled(root) != 0;
    let tag = format!("Stillfeather-pick-{}-{}", GetCurrentProcessId(), GetTickCount64());
    let tmp = std::env::temp_dir().to_string_lossy().into_owned();
    let request_path = paths::join(&tmp, &format!("{tag}.txt"));
    let out_path = paths::join(&tmp, &format!("{tag}-out.txt"));
    let st = d.state();
    let saving = d.kind == Kind::Save;
    let request = Request {
        mode: if saving {
            Mode::Save
        } else if options & FOS_PICKFOLDERS != 0 {
            Mode::Folders
        } else {
            Mode::Files
        },
        multi: !saving && options & FOS_ALLOWMULTISELECT != 0,
        title: st.title.clone(),
        ok_label: st.ok_label.clone(),
        name_label: st.name_label.clone(),
        folder: st.folder.clone(),
        default_folder: st.default_folder.clone(),
        file_name: st.file_name.clone(),
        types: st.types.clone(),
        type_index: st.type_index,
        default_ext: st.default_ext.clone(),
        must_exist: !saving && options & FOS_FILEMUSTEXIST != 0,
        show_hidden: options & FOS_FORCESHOWHIDDEN != 0,
        overwrite_prompt: saving && options & FOS_OVERWRITEPROMPT != 0,
        strict_types: saving && options & FOS_STRICTFILETYPES != 0,
        no_readonly: saving && options & FOS_NOREADONLYRETURN != 0,
        owner: root,
        enable_owner: disable,
        host_pid: GetCurrentProcessId(),
        host: host_name(),
        out: out_path.clone(),
    };
    if std::fs::write(&request_path, request.to_text()).is_err() {
        return Outcome::UseSystem;
    }
    let cleanup = || {
        let _ = std::fs::remove_file(&request_path);
        let _ = std::fs::remove_file(&out_path);
    };
    let mut cmd = wide(&format!("\"{exe}\" {} \"{request_path}\"", protocol::PICK_ARG));
    // 工作目錄設在 Stillfeather 的資料夾：不要沿用這個程式的目前資料夾（會讓那個資料夾在選擇期間無法改名）
    let cwd = wide(&paths::parent(&exe).unwrap_or_default());
    let mut si: StartupInfo = std::mem::zeroed();
    si.cb = std::mem::size_of::<StartupInfo>() as u32;
    let mut pi: ProcessInformation = std::mem::zeroed();
    let ok = CreateProcessW(
        std::ptr::null(),
        cmd.as_mut_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        0,
        0,
        std::ptr::null(),
        cwd.as_ptr(),
        &si,
        &mut pi,
    );
    if ok == 0 {
        cleanup();
        return Outcome::UseSystem;
    }
    // 讓選擇器可以到前景（使用者剛在這個程式裡按了上傳，前景權限在這裡）
    AllowSetForegroundWindow(pi.process_id);
    // 選擇器關閉前會先恢復它，焦點才會回到這裡
    if disable {
        EnableWindow(root, 0);
    }
    let finished = wait_pumping(pi.process);
    let mut code = 2u32;
    GetExitCodeProcess(pi.process, &mut code);
    CloseHandle(pi.thread);
    CloseHandle(pi.process);
    if disable && IsWindow(root) != 0 {
        EnableWindow(root, 1);
        SetForegroundWindow(root);
    }
    let outcome = match code as i32 {
        // 沒等到選擇器結束（程式正在結束，或等待失敗）：當作取消。選擇器還在執行（結束代碼是 STILL_ACTIVE），
        // 不能落到下面的「改用 Windows 的視窗」，否則程式結束前又多跳出一個開檔視窗。選擇器發現程式結束會自己關閉。
        _ if !finished || code == STILL_ACTIVE => Outcome::Cancelled,
        protocol::EXIT_OK => match std::fs::read_to_string(&out_path).ok().and_then(|t| Response::parse(&t)) {
            Some(r) if !r.paths.is_empty() => Outcome::Picked(r),
            _ => Outcome::UseSystem,
        },
        protocol::EXIT_CANCEL => Outcome::Cancelled,
        // 使用者要 Windows 的視窗，或 Stillfeather 沒能正常開啟（例如請求讀不到、程式當掉）
        _ => Outcome::UseSystem,
    };
    cleanup();
    outcome
}

/// GetExitCodeProcess：程序還在執行
const STILL_ACTIVE: u32 = 259;

/// 等選擇器結束；期間處理這個執行緒的視窗訊息（程式的畫面才不會卡住，也不會顯示「沒有回應」）。
/// 回傳 false：沒等到選擇器結束就返回（收到 WM_QUIT，或等待失敗）。
/// MWMO_INPUTAVAILABLE：佇列裡已經有、但之前被看過而沒取走的訊息也算，不會因此一直等下去。
unsafe fn wait_pumping(process: HANDLE) -> bool {
    loop {
        let r = MsgWaitForMultipleObjectsEx(1, &process, INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        if r == WAIT_OBJECT_0 {
            return true;
        }
        if r != WAIT_OBJECT_0 + 1 {
            return false;
        }
        let mut msg: Msg = std::mem::zeroed();
        while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                // 程式正在結束：把結束訊息放回去，不再等（選擇器發現程式結束會自己關閉）
                PostQuitMessage(msg.wparam as i32);
                return false;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

// ---- 設定：記下來，也照樣交給 Windows 原本的物件

unsafe extern "system" fn set_file_types(this: *mut c_void, count: u32, specs: *const FilterSpec) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        let st = d.state();
        st.types.clear();
        if !specs.is_null() {
            for i in 0..count.min(256) as usize {
                let s = &*specs.add(i);
                st.types.push(FileType { name: from_wide_ptr(s.name), spec: from_wide_ptr(s.spec) });
            }
        }
        match d.real_vtbl() {
            Some(vt) => (vt.set_file_types)(d.real, count, specs),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn set_file_type_index(this: *mut c_void, index: u32) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        d.state().type_index = index;
        match d.real_vtbl() {
            Some(vt) => (vt.set_file_type_index)(d.real, index),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn get_file_type_index(this: *mut c_void, out: *mut u32) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        let d = dialog(this);
        let st = d.state();
        match (&st.shown, d.real_vtbl()) {
            (Shown::Ours { .. }, _) | (_, None) => {
                *out = st.type_index.max(1);
                S_OK
            }
            (_, Some(vt)) => (vt.get_file_type_index)(d.real, out),
        }
    })
}

unsafe extern "system" fn advise(this: *mut c_void, sink: *mut c_void, cookie: *mut u32) -> HRESULT {
    guard(|| {
        if sink.is_null() || cookie.is_null() {
            return E_POINTER;
        }
        let d = dialog(this);
        let st = d.state();
        // 也登記給 Windows 原本的物件：改用它的視窗時，程式照樣收到事件
        let mut id = 0u32;
        let hr = match d.real_vtbl() {
            Some(vt) => (vt.advise)(d.real, sink, &mut id),
            None => E_FAIL,
        };
        if hr < 0 || st.sinks.iter().any(|(c, _)| *c == id) {
            st.next_cookie += 1;
            id = 0x5F00_0000 | st.next_cookie;
        }
        add_ref(sink);
        st.sinks.push((id, sink));
        *cookie = id;
        S_OK
    })
}

unsafe extern "system" fn unadvise(this: *mut c_void, cookie: u32) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        let st = d.state();
        if let Some(i) = st.sinks.iter().position(|(c, _)| *c == cookie) {
            let (_, sink) = st.sinks.remove(i);
            release(sink);
        }
        if let Some(vt) = d.real_vtbl() {
            (vt.unadvise)(d.real, cookie);
        }
        S_OK
    })
}

unsafe extern "system" fn set_options(this: *mut c_void, options: u32) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        d.state().options = options;
        match d.real_vtbl() {
            Some(vt) => (vt.set_options)(d.real, options),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn get_options(this: *mut c_void, out: *mut u32) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.get_options)(d.real, out),
            None => {
                *out = d.state().options;
                S_OK
            }
        }
    })
}

unsafe extern "system" fn set_default_folder(this: *mut c_void, item: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        d.state().default_folder = shell_item_path(item).unwrap_or_default();
        match d.real_vtbl() {
            Some(vt) => (vt.set_default_folder)(d.real, item),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn set_folder(this: *mut c_void, item: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        d.state().folder = shell_item_path(item).unwrap_or_default();
        match d.real_vtbl() {
            Some(vt) => (vt.set_folder)(d.real, item),
            None => S_OK,
        }
    })
}

macro_rules! set_text {
    ($name:ident, $field:ident, $forward:ident) => {
        unsafe extern "system" fn $name(this: *mut c_void, text: *const u16) -> HRESULT {
            guard(|| {
                let d = dialog(this);
                d.state().$field = from_wide_ptr(text);
                match d.real_vtbl() {
                    Some(vt) => (vt.$forward)(d.real, text),
                    None => S_OK,
                }
            })
        }
    };
}

set_text!(set_file_name, file_name, set_file_name);
set_text!(set_title, title, set_title);
set_text!(set_ok_button_label, ok_label, set_ok_button_label);
set_text!(set_file_name_label, name_label, set_file_name_label);
set_text!(set_default_extension, default_ext, set_default_extension);

unsafe extern "system" fn add_place(this: *mut c_void, item: *mut c_void, place: i32) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.add_place)(d.real, item, place),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn close(this: *mut c_void, hr: HRESULT) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.close)(d.real, hr),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn set_client_guid(this: *mut c_void, g: *const GUID) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.set_client_guid)(d.real, g),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn clear_client_data(this: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.clear_client_data)(d.real),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn set_filter(this: *mut c_void, filter: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_vtbl() {
            Some(vt) => (vt.set_filter)(d.real, filter),
            None => S_OK,
        }
    })
}

// ---- 結果：在 Stillfeather 選好的就自己回答，否則問 Windows 原本的物件

/// Stillfeather 選好的路徑；沒有（還沒選或用了 Windows 的視窗）時回傳 None
unsafe fn our_paths<'a>(d: &Dialog) -> Option<&'a Vec<String>> {
    match &d.state().shown {
        Shown::Ours { paths } => Some(paths),
        _ => None,
    }
}

unsafe extern "system" fn get_result(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match (our_paths(d), d.real_vtbl()) {
            (Some(paths), _) => match paths.first() {
                Some(p) if d.kind == Kind::Save => item_for_save(p, out),
                Some(p) => item_from_path(p, out),
                None => E_UNEXPECTED,
            },
            (None, Some(vt)) => (vt.get_result)(d.real, out),
            (None, None) => E_UNEXPECTED,
        }
    })
}

unsafe fn results_array(paths: &[String], out: *mut *mut c_void) -> HRESULT {
    let mut pidls: Vec<*mut c_void> = Vec::new();
    for p in paths {
        let w = wide(p);
        let mut pidl: *mut c_void = std::ptr::null_mut();
        if SHParseDisplayName(w.as_ptr(), std::ptr::null_mut(), &mut pidl, 0, std::ptr::null_mut()) >= 0 && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    let hr = if pidls.is_empty() {
        E_FAIL
    } else {
        let list: Vec<*const c_void> = pidls.iter().map(|p| *p as *const c_void).collect();
        SHCreateShellItemArrayFromIDLists(list.len() as u32, list.as_ptr(), out)
    };
    for p in pidls {
        CoTaskMemFree(p);
    }
    hr
}

unsafe extern "system" fn get_results(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match (our_paths(d), d.real_open_vtbl()) {
            (Some(paths), _) => results_array(paths, out),
            (None, Some(vt)) => (vt.get_results)(d.real, out),
            (None, None) => E_UNEXPECTED,
        }
    })
}

unsafe extern "system" fn get_selected_items(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match (our_paths(d), d.real_open_vtbl()) {
            (Some(paths), _) => results_array(paths, out),
            (None, Some(vt)) => (vt.get_selected_items)(d.real, out),
            (None, None) => E_UNEXPECTED,
        }
    })
}

unsafe extern "system" fn get_current_selection(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match (our_paths(d), d.real_vtbl()) {
            (Some(paths), _) => match paths.first() {
                Some(p) if d.kind == Kind::Save => item_for_save(p, out),
                Some(p) => item_from_path(p, out),
                None => E_FAIL,
            },
            (None, Some(vt)) => (vt.get_current_selection)(d.real, out),
            (None, None) => E_FAIL,
        }
    })
}

unsafe extern "system" fn get_folder(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match (our_paths(d), d.real_vtbl()) {
            (Some(paths), _) => match paths.first().and_then(|p| paths::parent(p)) {
                Some(dir) => item_from_path(&dir, out),
                None => E_FAIL,
            },
            (None, Some(vt)) => (vt.get_folder)(d.real, out),
            (None, None) => E_FAIL,
        }
    })
}

unsafe extern "system" fn get_file_name(this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        let name = match (our_paths(d), d.real_vtbl()) {
            (Some(paths), _) => paths.first().map(|p| paths::file_name(p).to_string()).unwrap_or_default(),
            (None, Some(vt)) => return (vt.get_file_name)(d.real, out),
            (None, None) => d.state().file_name.clone(),
        };
        *out = co_string(&name);
        if (*out).is_null() {
            E_OUTOFMEMORY
        } else {
            S_OK
        }
    })
}

// ---- IFileSaveDialog（只在另存新檔視窗的物件上）

/// 一開始的資料夾與檔名（例如「另存新檔」時目前檔案的位置）
unsafe extern "system" fn set_save_as_item(this: *mut c_void, item: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        if let Some(path) = shell_item_path(item) {
            let st = d.state();
            if let Some(dir) = paths::parent(&path) {
                st.folder = dir;
            }
            st.file_name = paths::file_name(&path).to_string();
        }
        match d.real_save_vtbl() {
            Some(vt) => (vt.set_save_as_item)(d.real, item),
            None => S_OK,
        }
    })
}

unsafe extern "system" fn set_properties(this: *mut c_void, store: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        let hr = match d.real_save_vtbl() {
            Some(vt) => (vt.set_properties)(d.real, store),
            None => E_NOTIMPL,
        };
        if hr >= 0 {
            d.state().properties = true;
        }
        hr
    })
}

unsafe extern "system" fn set_collected_properties(this: *mut c_void, list: *mut c_void, append_default: BOOL) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        let hr = match d.real_save_vtbl() {
            Some(vt) => (vt.set_collected_properties)(d.real, list, append_default),
            None => E_NOTIMPL,
        };
        if hr >= 0 {
            d.state().properties = true;
        }
        hr
    })
}

unsafe extern "system" fn get_properties(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        let d = dialog(this);
        match d.real_save_vtbl() {
            Some(vt) => (vt.get_properties)(d.real, out),
            None => E_UNEXPECTED,
        }
    })
}

unsafe extern "system" fn apply_properties(this: *mut c_void, item: *mut c_void, store: *mut c_void, owner: HWND, sink: *mut c_void) -> HRESULT {
    guard(|| {
        let d = dialog(this);
        match d.real_save_vtbl() {
            Some(vt) => (vt.apply_properties)(d.real, item, store, owner, sink),
            None => E_NOTIMPL,
        }
    })
}

// ---------------------------------------------------------------- IFileDialogCustomize：原封轉交

/// 這個介面的 27 個方法都只有最多三個整數或指標參數；在 x64 的呼叫慣例下，
/// 以三個 usize 接收再原封交給 Windows 原本的物件是正確的（多出來的暫存器內容對方不會讀）。
type Forward3 = unsafe extern "system" fn(*mut c_void, usize, usize, usize) -> HRESULT;

#[repr(C)]
struct CustomizeVtbl {
    base: UnknownVtbl,
    methods: [Forward3; 27],
}

/// 這個方法會不會在視窗裡加一個使用者要操作的選項（照 IFileDialogCustomize 的方法順序）：
/// 0 EnableOpenDropDown、1 AddMenu、2 AddPushButton、3 AddComboBox、4 AddRadioButtonList、5 AddCheckButton、6 AddEditBox。
/// 7 AddSeparator、8 AddText 之後是只有外觀或讀寫狀態的方法，不算。
fn adds_input_control(index: usize) -> bool {
    index <= 6
}

unsafe fn forward_custom(this: *mut c_void, index: usize, a: usize, b: usize, c: usize) -> HRESULT {
    guard(|| {
        let d = dialog_from_custom(this);
        if d.real_custom.is_null() {
            return E_NOTIMPL;
        }
        let f: Forward3 = std::mem::transmute::<*const c_void, Forward3>(slot(d.real_custom, 3 + index));
        let hr = f(d.real_custom, a, b, c);
        if hr >= 0 && adds_input_control(index) {
            // Stillfeather 的視窗沒有地方放程式自己的選項；使用者選不到的話，程式會一直用預設值（例如編碼、唯讀）
            d.state().custom_controls = true;
        }
        hr
    })
}

macro_rules! forwarders {
    ($($name:ident = $n:expr),* $(,)?) => {
        $(unsafe extern "system" fn $name(this: *mut c_void, a: usize, b: usize, c: usize) -> HRESULT {
            forward_custom(this, $n, a, b, c)
        })*
    };
}

forwarders!(
    c0 = 0, c1 = 1, c2 = 2, c3 = 3, c4 = 4, c5 = 5, c6 = 6, c7 = 7, c8 = 8, c9 = 9, c10 = 10, c11 = 11, c12 = 12, c13 = 13,
    c14 = 14, c15 = 15, c16 = 16, c17 = 17, c18 = 18, c19 = 19, c20 = 20, c21 = 21, c22 = 22, c23 = 23, c24 = 24, c25 = 25, c26 = 26,
);

unsafe extern "system" fn custom_query_interface(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let d = dialog_from_custom(this);
    query_interface(d as *const Dialog as *mut c_void, riid, out)
}

unsafe extern "system" fn custom_add_ref(this: *mut c_void) -> u32 {
    let d = dialog_from_custom(this);
    dialog_add_ref(d as *const Dialog as *mut c_void)
}

unsafe extern "system" fn custom_release(this: *mut c_void) -> u32 {
    let d = dialog_from_custom(this);
    dialog_release(d as *const Dialog as *mut c_void)
}

static CUSTOMIZE_VTBL: CustomizeVtbl = CustomizeVtbl {
    base: UnknownVtbl { query_interface: custom_query_interface, add_ref: custom_add_ref, release: custom_release },
    methods: [c0, c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18, c19, c20, c21, c22, c23, c24, c25, c26],
};

// ---------------------------------------------------------------- 類別工廠與 DLL 進入點

#[repr(C)]
struct Factory {
    vtbl: *const ClassFactoryVtbl,
}

unsafe impl Sync for Factory {}

static OPEN_FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    base: UnknownVtbl { query_interface: factory_query_interface, add_ref: factory_add_ref, release: factory_release },
    create_instance: create_open_instance,
    lock_server,
};

static SAVE_FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    base: UnknownVtbl { query_interface: factory_query_interface, add_ref: factory_add_ref, release: factory_release },
    create_instance: create_save_instance,
    lock_server,
};

static OPEN_FACTORY: Factory = Factory { vtbl: &OPEN_FACTORY_VTBL };
static SAVE_FACTORY: Factory = Factory { vtbl: &SAVE_FACTORY_VTBL };

unsafe extern "system" fn factory_query_interface(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    if *riid == IID_IUNKNOWN || *riid == IID_ICLASSFACTORY {
        *out = this;
        S_OK
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

// 工廠是靜態的，參考計數沒有意義
unsafe extern "system" fn factory_add_ref(_: *mut c_void) -> u32 {
    2
}

unsafe extern "system" fn factory_release(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn lock_server(_: *mut c_void, _: BOOL) -> HRESULT {
    S_OK
}

unsafe extern "system" fn create_open_instance(_: *mut c_void, outer: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    create_instance(Kind::Open, outer, riid, out)
}

unsafe extern "system" fn create_save_instance(_: *mut c_void, outer: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    create_instance(Kind::Save, outer, riid, out)
}

unsafe fn create_instance(kind: Kind, outer: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    guard(|| {
        if out.is_null() || riid.is_null() {
            return E_POINTER;
        }
        *out = std::ptr::null_mut();
        if !outer.is_null() {
            return CLASS_E_NOAGGREGATION;
        }
        let real = create_system_dialog(kind);
        let mut real_custom: *mut c_void = std::ptr::null_mut();
        let mut options = if kind == Kind::Save { FOS_SAVE_DEFAULT } else { FOS_OPEN_DEFAULT };
        if !real.is_null() {
            let vt = &**(real as *const *const DialogVtbl);
            if (vt.base.query_interface)(real, &IID_IFILEDIALOGCUSTOMIZE, &mut real_custom) < 0 {
                real_custom = std::ptr::null_mut();
            }
            let mut o = 0u32;
            if (vt.get_options)(real, &mut o) >= 0 {
                options = o;
            }
        }
        let vtbl: *const DialogVtbl = match kind {
            Kind::Open => &OPEN_VTBL.dialog,
            Kind::Save => &SAVE_VTBL.dialog,
        };
        let d = Box::new(Dialog {
            vtbl,
            custom_vtbl: &CUSTOMIZE_VTBL,
            refs: AtomicU32::new(1),
            kind,
            real,
            real_custom,
            state: UnsafeCell::new(State { options, ..Default::default() }),
        });
        let this = Box::into_raw(d) as *mut c_void;
        let hr = query_interface(this, riid, out);
        dialog_release(this);
        hr
    })
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(rclsid: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || out.is_null() {
        return E_POINTER;
    }
    *out = std::ptr::null_mut();
    // 兩個檔名的 DLL 內容相同，兩種視窗都能建立（登錄決定哪一個檔名負責哪一種）
    let factory = if *rclsid == CLSID_FILE_OPEN_DIALOG {
        &OPEN_FACTORY
    } else if *rclsid == CLSID_FILE_SAVE_DIALOG {
        &SAVE_FACTORY
    } else {
        return CLASS_E_CLASSNOTAVAILABLE;
    };
    factory_query_interface(factory as *const Factory as *mut c_void, riid, out)
}

/// 一律不卸載：載入在別人的程式裡，卸載 Rust 的執行環境沒有好處，風險卻不小
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        assert_eq!(std::mem::size_of::<Msg>(), 48);
        assert_eq!(std::mem::size_of::<StartupInfo>(), 104);
        assert_eq!(std::mem::size_of::<ProcessInformation>(), 24);
        assert_eq!(std::mem::size_of::<FilterSpec>(), 16);
        assert_eq!(std::mem::size_of::<GUID>(), 16);
        // IFileDialog：IUnknown 3 + IModalWindow 1 + IFileDialog 23
        assert_eq!(std::mem::size_of::<DialogVtbl>(), 27 * 8);
        // IFileOpenDialog：再加 GetResults、GetSelectedItems
        assert_eq!(std::mem::size_of::<OpenVtbl>(), 29 * 8);
        // IFileSaveDialog：再加 SetSaveAsItem、SetProperties、SetCollectedProperties、GetProperties、ApplyProperties
        assert_eq!(std::mem::size_of::<SaveVtbl>(), 32 * 8);
        assert_eq!(std::mem::offset_of!(SaveVtbl, set_save_as_item), 27 * 8);
        // IFileDialogCustomize：IUnknown 3 + 27
        assert_eq!(std::mem::size_of::<CustomizeVtbl>(), 30 * 8);
        assert_eq!(std::mem::offset_of!(Dialog, custom_vtbl), 8);
    }

    #[test]
    fn guids() {
        // 和登錄用的文字（src/picker.rs）是同一個
        let text = |g: GUID| {
            format!(
                "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}}}",
                g.d1,
                g.d2,
                g.d3,
                g.d4[0],
                g.d4[1],
                g.d4[2..].iter().map(|b| format!("{b:02X}")).collect::<String>()
            )
        };
        assert!(text(CLSID_FILE_OPEN_DIALOG).eq_ignore_ascii_case(protocol::CLSID_FILE_OPEN_DIALOG));
        assert!(text(CLSID_FILE_SAVE_DIALOG).eq_ignore_ascii_case(protocol::CLSID_FILE_SAVE_DIALOG));
        assert_eq!(text(IID_IFILESAVEDIALOG), "{84BCCD23-5FDE-4CDB-AEA4-AF64B83D78AB}");
        assert_eq!(Kind::Save.iid(), IID_IFILESAVEDIALOG);
        assert_eq!(Kind::Open.clsid(), CLSID_FILE_OPEN_DIALOG);
        assert_eq!(FOS_SAVE_DEFAULT, FOS_OVERWRITEPROMPT | 0x8 | 0x800 | FOS_NOREADONLYRETURN);
    }

    #[test]
    fn hresults() {
        assert_eq!(E_CANCELLED as u32, 0x8007_04C7);
        assert_eq!(HKEY_LOCAL_MACHINE as usize, 0xFFFF_FFFF_8000_0002);
        assert_eq!(HKEY_CURRENT_USER as usize, 0xFFFF_FFFF_8000_0001);
    }

    #[test]
    fn file_ok_only_s_false_refuses() {
        assert!(file_ok_accepts(S_OK));
        // Chrome／Edge 的 AutoCloseDialogEventHandler::OnFileOk 回傳 E_NOTIMPL：要當成接受
        assert!(file_ok_accepts(E_NOTIMPL));
        assert!(file_ok_accepts(E_FAIL));
        assert!(!file_ok_accepts(S_FALSE));
    }

    #[test]
    fn custom_controls_that_need_the_windows_dialog() {
        // EnableOpenDropDown、AddMenu、AddPushButton、AddComboBox、AddRadioButtonList、AddCheckButton、AddEditBox
        for i in 0..=6 {
            assert!(adds_input_control(i), "{i}");
        }
        // AddSeparator、AddText、SetControlLabel…：只改外觀或讀寫狀態
        for i in 7..27 {
            assert!(!adds_input_control(i), "{i}");
        }
    }
}

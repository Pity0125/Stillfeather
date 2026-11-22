//! 手寫的 Win32 FFI 宣告（x64）。
//!
//! 不依賴 windows-sys 等 crate：以 `raw-dylib` 直接匯入 DLL，建置時不需要 Windows SDK 的
//! 匯入程式庫。comctl32 改在執行期載入（見 `comctl`），以確保啟用 v6 視覺樣式。
//! 所有 handle 一律以 `isize` 表示，方便在執行緒間傳遞；在 x64 呼叫慣例上與指標等價。
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms, dead_code)]

use std::ffi::c_void;

pub type HWND = isize;
pub type HANDLE = isize;
pub type HINSTANCE = isize;
pub type HMENU = isize;
pub type HDC = isize;
pub type HFONT = isize;
pub type HBRUSH = isize;
pub type HPEN = isize;
pub type HICON = isize;
pub type HBITMAP = isize;
pub type HGDIOBJ = isize;
pub type HCURSOR = isize;
pub type HMONITOR = isize;
pub type HIMAGELIST = isize;
pub type HTREEITEM = isize;
pub type HGLOBAL = isize;
pub type HKEY = isize;
pub type HDWP = isize;
pub type HMODULE = isize;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type BOOL = i32;
pub type HRESULT = i32;
pub type COLORREF = u32;

pub type WNDPROC = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
pub type SUBCLASSPROC = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SIZE {
    pub cx: i32,
    pub cy: i32,
}

/// 屬性鍵（例如影片長度、圖片尺寸）
#[repr(C)]
pub struct PROPERTYKEY {
    pub fmtid: GUID,
    pub pid: u32,
}

/// 只讀取數值型別（VT_UI4、VT_UI8）時使用；其他型別交給 PropVariantClear 釋放。
#[repr(C)]
pub struct PROPVARIANT {
    pub vt: u16,
    pub reserved: [u16; 3],
    pub data: [u64; 2],
}

pub const VT_UI4: u16 = 19;
pub const VT_UI8: u16 = 21;
/// 未處理例外的資訊（記錄當機原因用）
#[repr(C)]
pub struct EXCEPTION_RECORD {
    pub code: u32,
    pub flags: u32,
    pub record: *mut EXCEPTION_RECORD,
    pub address: *mut c_void,
    pub count: u32,
    pub info: [usize; 15],
}

#[repr(C)]
pub struct EXCEPTION_POINTERS {
    pub record: *mut EXCEPTION_RECORD,
    pub context: *mut c_void,
}

pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ_WRITE: u32 = 0x3;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_ALWAYS: u32 = 4;
pub const FILE_APPEND_DATA: u32 = 0x0004;
pub const STD_ERROR_HANDLE: u32 = -12i32 as u32;
pub const HALFTONE: i32 = 4;
pub const DI_NORMAL: u32 = 3;
pub const WM_MOVE: u32 = 0x0003;
pub const DT_TOP: u32 = 0;
pub const CP_ACP: u32 = 0;

#[repr(C)]
pub struct LVTILEVIEWINFO {
    pub cbSize: u32,
    pub dwMask: u32,
    pub dwFlags: u32,
    pub sizeTile: SIZE,
    pub cLines: i32,
    pub rcLabelMargin: RECT,
}

#[repr(C)]
pub struct MENUITEMINFOW {
    pub cbSize: u32,
    pub fMask: u32,
    pub fType: u32,
    pub fState: u32,
    pub wID: u32,
    pub hSubMenu: HMENU,
    pub hbmpChecked: HBITMAP,
    pub hbmpUnchecked: HBITMAP,
    pub dwItemData: usize,
    pub dwTypeData: *mut u16,
    pub cch: u32,
    pub hbmpItem: HBITMAP,
}

/// TrackPopupMenuEx 的參數：選單不要蓋住的範圍（例如按鈕本身）
#[repr(C)]
pub struct TPMPARAMS {
    pub cbSize: u32,
    pub rcExclude: RECT,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RECT {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> RECT {
        RECT { left: x, top: y, right: x + w, bottom: y + h }
    }
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    pub fn inflate(&self, dx: i32, dy: i32) -> RECT {
        RECT { left: self.left - dx, top: self.top - dy, right: self.right + dx, bottom: self.bottom + dy }
    }
}

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: WNDPROC,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HICON,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
    pub lPrivate: u32,
}

/// 低階鍵盤攔截收到的按鍵（WH_KEYBOARD_LL）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KBDLLHOOKSTRUCT {
    pub vkCode: u32,
    pub scanCode: u32,
    pub flags: u32,
    pub time: u32,
    pub dwExtraInfo: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KEYBDINPUT {
    pub wVk: u16,
    pub wScan: u16,
    pub dwFlags: u32,
    pub time: u32,
    pub dwExtraInfo: usize,
}

/// SendInput 的 INPUT（只用到鍵盤；補到與 MOUSEINPUT 聯集相同的大小）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KEYBOARD_INPUT {
    pub kind: u32,
    pub ki: KEYBDINPUT,
    pub _pad: [u8; 8],
}

pub type HOOKPROC = unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
pub struct PAINTSTRUCT {
    pub hdc: HDC,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}

#[repr(C)]
pub struct TRACKMOUSEEVENT {
    pub cbSize: u32,
    pub dwFlags: u32,
    pub hwndTrack: HWND,
    pub dwHoverTime: u32,
}

#[repr(C)]
pub struct CMINVOKECOMMANDINFOEX {
    pub cbSize: u32,
    pub fMask: u32,
    pub hwnd: HWND,
    pub lpVerb: *const u8,
    pub lpParameters: *const u8,
    pub lpDirectory: *const u8,
    pub nShow: i32,
    pub dwHotKey: u32,
    pub hIcon: HANDLE,
    pub lpTitle: *const u8,
    pub lpVerbW: *const u16,
    pub lpParametersW: *const u16,
    pub lpDirectoryW: *const u16,
    pub lpTitleW: *const u16,
    pub ptInvoke: POINT,
}

/// IDragSourceHelper::InitializeFromBitmap 的拖曳圖示
#[repr(C)]
pub struct SHDRAGIMAGE {
    pub sizeDragImage: SIZE,
    pub ptOffset: POINT,
    pub hbmpDragImage: HBITMAP,
    pub crColorKey: COLORREF,
}

#[repr(C)]
pub struct FORMATETC {
    pub cfFormat: u16,
    pub ptd: *mut c_void,
    pub dwAspect: u32,
    pub lindex: i32,
    pub tymed: u32,
}

#[repr(C)]
pub struct STGMEDIUM {
    pub tymed: u32,
    /// 聯集（hGlobal、pstm…）；這裡只用到 hGlobal
    pub hGlobal: HGLOBAL,
    pub pUnkForRelease: *mut c_void,
}

/// 拖曳圖示下方的說明文字（例如「釘選到 常用位置」）
#[repr(C)]
pub struct DROPDESCRIPTION {
    pub kind: i32,
    pub szMessage: [u16; 260],
    pub szInsert: [u16; 260],
}

#[repr(C)]
pub struct FLASHWINFO {
    pub cbSize: u32,
    pub hwnd: HWND,
    pub dwFlags: u32,
    pub uCount: u32,
    pub dwTimeout: u32,
}

#[repr(C)]
pub struct MINMAXINFO {
    pub ptReserved: POINT,
    pub ptMaxSize: POINT,
    pub ptMaxPosition: POINT,
    pub ptMinTrackSize: POINT,
    pub ptMaxTrackSize: POINT,
}

#[repr(C)]
pub struct NMHDR {
    pub hwndFrom: HWND,
    pub idFrom: usize,
    pub code: u32,
}

#[repr(C)]
pub struct NMCUSTOMDRAW {
    pub hdr: NMHDR,
    pub dwDrawStage: u32,
    pub hdc: HDC,
    pub rc: RECT,
    pub dwItemSpec: usize,
    pub uItemState: u32,
    pub lItemlParam: LPARAM,
}

/// 清單自訂繪製（含每列文字顏色）。
#[repr(C)]
pub struct NMLVCUSTOMDRAW {
    pub nmcd: NMCUSTOMDRAW,
    pub clrText: COLORREF,
    pub clrTextBk: COLORREF,
    pub iSubItem: i32,
    pub dwItemType: u32,
    pub clrFace: COLORREF,
    pub iIconEffect: i32,
    pub iIconPhase: i32,
    pub iPartId: i32,
    pub iStateId: i32,
    pub rcText: RECT,
    pub uAlign: u32,
}

/// 樹狀圖自訂繪製。
#[repr(C)]
pub struct NMTVCUSTOMDRAW {
    pub nmcd: NMCUSTOMDRAW,
    pub clrText: COLORREF,
    pub clrTextBk: COLORREF,
    pub iLevel: i32,
}

#[repr(C)]
pub struct LVCOLUMNW {
    pub mask: u32,
    pub fmt: i32,
    pub cx: i32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iSubItem: i32,
    pub iImage: i32,
    pub iOrder: i32,
    pub cxMin: i32,
    pub cxDefault: i32,
    pub cxIdeal: i32,
}

#[repr(C)]
pub struct LVITEMW {
    pub mask: u32,
    pub iItem: i32,
    pub iSubItem: i32,
    pub state: u32,
    pub stateMask: u32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iImage: i32,
    pub lParam: LPARAM,
    pub iIndent: i32,
    pub iGroupId: i32,
    pub cColumns: u32,
    pub puColumns: *mut u32,
    pub piColFmt: *mut i32,
    pub iGroup: i32,
}

#[repr(C)]
pub struct NMLVDISPINFOW {
    pub hdr: NMHDR,
    pub item: LVITEMW,
}

#[repr(C)]
pub struct NMLISTVIEW {
    pub hdr: NMHDR,
    pub iItem: i32,
    pub iSubItem: i32,
    pub uNewState: u32,
    pub uOldState: u32,
    pub uChanged: u32,
    pub ptAction: POINT,
    pub lParam: LPARAM,
}

#[repr(C)]
pub struct LVFINDINFOW {
    pub flags: u32,
    pub psz: *const u16,
    pub lParam: LPARAM,
    pub pt: POINT,
    pub vkDirection: u32,
}

#[repr(C)]
pub struct NMLVFINDITEMW {
    pub hdr: NMHDR,
    pub iStart: i32,
    pub lvfi: LVFINDINFOW,
}

#[repr(C)]
pub struct LVHITTESTINFO {
    pub pt: POINT,
    pub flags: u32,
    pub iItem: i32,
    pub iSubItem: i32,
    pub iGroup: i32,
}

#[repr(C)]
pub struct NMLVGETINFOTIPW {
    pub hdr: NMHDR,
    pub dwFlags: u32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iItem: i32,
    pub iSubItem: i32,
    pub lParam: LPARAM,
}

#[repr(C)]
pub struct HDITEMW {
    pub mask: u32,
    pub cxy: i32,
    pub pszText: *mut u16,
    pub hbm: HBITMAP,
    pub cchTextMax: i32,
    pub fmt: i32,
    pub lParam: LPARAM,
    pub iImage: i32,
    pub iOrder: i32,
    pub type_: u32,
    pub pvFilter: *mut c_void,
    pub state: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TVITEMW {
    pub mask: u32,
    pub hItem: HTREEITEM,
    pub state: u32,
    pub stateMask: u32,
    pub pszText: *mut u16,
    pub cchTextMax: i32,
    pub iImage: i32,
    pub iSelectedImage: i32,
    pub cChildren: i32,
    pub lParam: LPARAM,
}

/// 實際 Windows 定義中 item 為 TVITEMEXW（較大的 union），補足長度。
#[repr(C)]
pub struct TVINSERTSTRUCTW {
    pub hParent: HTREEITEM,
    pub hInsertAfter: HTREEITEM,
    pub item: TVITEMW,
    pub _ex: [u8; 24],
}

#[repr(C)]
pub struct NMTREEVIEWW {
    pub hdr: NMHDR,
    pub action: u32,
    pub itemOld: TVITEMW,
    pub itemNew: TVITEMW,
    pub ptDrag: POINT,
}

#[repr(C)]
pub struct NMTVKEYDOWN {
    pub hdr: NMHDR,
    pub wVKey: u16,
    pub flags: u32,
}

#[repr(C)]
pub struct TVHITTESTINFO {
    pub pt: POINT,
    pub flags: u32,
    pub hItem: HTREEITEM,
}

#[repr(C)]
pub struct TOOLINFOW {
    pub cbSize: u32,
    pub uFlags: u32,
    pub hwnd: HWND,
    pub uId: usize,
    pub rect: RECT,
    pub hinst: HINSTANCE,
    pub lpszText: *mut u16,
    pub lParam: LPARAM,
    pub lpReserved: *mut c_void,
}

#[repr(C)]
pub struct INITCOMMONCONTROLSEX {
    pub dwSize: u32,
    pub dwICC: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FILETIME {
    pub dwLowDateTime: u32,
    pub dwHighDateTime: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SYSTEMTIME {
    pub wYear: u16,
    pub wMonth: u16,
    pub wDayOfWeek: u16,
    pub wDay: u16,
    pub wHour: u16,
    pub wMinute: u16,
    pub wSecond: u16,
    pub wMilliseconds: u16,
}

#[repr(C)]
pub struct WIN32_FIND_DATAW {
    pub dwFileAttributes: u32,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
    pub dwReserved0: u32,
    pub dwReserved1: u32,
    pub cFileName: [u16; 260],
    pub cAlternateFileName: [u16; 14],
}

#[repr(C)]
pub struct WIN32_FILE_ATTRIBUTE_DATA {
    pub dwFileAttributes: u32,
    pub ftCreationTime: FILETIME,
    pub ftLastAccessTime: FILETIME,
    pub ftLastWriteTime: FILETIME,
    pub nFileSizeHigh: u32,
    pub nFileSizeLow: u32,
}

#[repr(C)]
pub struct SHFILEOPSTRUCTW {
    pub hwnd: HWND,
    pub wFunc: u32,
    pub pFrom: *const u16,
    pub pTo: *const u16,
    pub fFlags: u16,
    pub fAnyOperationsAborted: BOOL,
    pub hNameMappings: *mut c_void,
    pub lpszProgressTitle: *const u16,
}

#[repr(C)]
pub struct SHFILEINFOW {
    pub hIcon: HICON,
    pub iIcon: i32,
    pub dwAttributes: u32,
    pub szDisplayName: [u16; 260],
    pub szTypeName: [u16; 80],
}

#[repr(C)]
pub struct SHSTOCKICONINFO {
    pub cbSize: u32,
    pub hIcon: HICON,
    pub iSysImageIndex: i32,
    pub iIcon: i32,
    pub szPath: [u16; 260],
}

#[repr(C)]
pub struct SHELLEXECUTEINFOW {
    pub cbSize: u32,
    pub fMask: u32,
    pub hwnd: HWND,
    pub lpVerb: *const u16,
    pub lpFile: *const u16,
    pub lpParameters: *const u16,
    pub lpDirectory: *const u16,
    pub nShow: i32,
    pub hInstApp: HINSTANCE,
    pub lpIDList: *mut c_void,
    pub lpClass: *const u16,
    pub hkeyClass: HKEY,
    pub dwHotKey: u32,
    pub hIconOrMonitor: HANDLE,
    pub hProcess: HANDLE,
}

#[repr(C)]
pub struct DROPFILES {
    pub pFiles: u32,
    pub pt: POINT,
    pub fNC: BOOL,
    pub fWide: BOOL,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BITMAPINFOHEADER {
    pub biSize: u32,
    pub biWidth: i32,
    pub biHeight: i32,
    pub biPlanes: u16,
    pub biBitCount: u16,
    pub biCompression: u32,
    pub biSizeImage: u32,
    pub biXPelsPerMeter: i32,
    pub biYPelsPerMeter: i32,
    pub biClrUsed: u32,
    pub biClrImportant: u32,
}

/// 色盤留三格，GetDIBits 遇到 BI_BITFIELDS 時才不會寫出界。
#[repr(C)]
pub struct BITMAPINFO {
    pub bmiHeader: BITMAPINFOHEADER,
    pub bmiColors: [u32; 3],
}

impl BITMAPINFO {
    pub fn top_down_32(width: i32, height: i32) -> BITMAPINFO {
        BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [0; 3],
        }
    }
}

#[repr(C)]
pub struct BITMAP {
    pub bmType: i32,
    pub bmWidth: i32,
    pub bmHeight: i32,
    pub bmWidthBytes: i32,
    pub bmPlanes: u16,
    pub bmBitsPixel: u16,
    pub bmBits: *mut c_void,
}

#[repr(C)]
pub struct ICONINFO {
    pub fIcon: BOOL,
    pub xHotspot: u32,
    pub yHotspot: u32,
    pub hbmMask: HBITMAP,
    pub hbmColor: HBITMAP,
}

#[repr(C)]
pub struct MONITORINFO {
    pub cbSize: u32,
    pub rcMonitor: RECT,
    pub rcWork: RECT,
    pub dwFlags: u32,
}

#[repr(C)]
pub struct CHOOSECOLORW {
    pub lStructSize: u32,
    pub hwndOwner: HWND,
    pub hInstance: HWND,
    pub rgbResult: COLORREF,
    pub lpCustColors: *mut COLORREF,
    pub Flags: u32,
    pub lCustData: LPARAM,
    pub lpfnHook: *const c_void,
    pub lpTemplateName: *const u16,
}

#[repr(C)]
pub struct COPYDATASTRUCT {
    pub dwData: usize,
    pub cbData: u32,
    pub lpData: *const c_void,
}

#[repr(C)]
pub struct ACTCTXW {
    pub cbSize: u32,
    pub dwFlags: u32,
    pub lpSource: *const u16,
    pub wProcessorArchitecture: u16,
    pub wLangId: u16,
    pub lpAssemblyDirectory: *const u16,
    pub lpResourceName: *const u16,
    pub lpApplicationName: *const u16,
    pub hModule: HMODULE,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub const fn from_u128(v: u128) -> GUID {
        GUID {
            data1: (v >> 96) as u32,
            data2: (v >> 80) as u16,
            data3: (v >> 64) as u16,
            data4: (v as u64).to_be_bytes(),
        }
    }
}

// ---------------------------------------------------------------- 常數

pub const WM_CREATE: u32 = 0x0001;
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_SIZE: u32 = 0x0005;
pub const WM_ACTIVATE: u32 = 0x0006;
pub const WM_SETFOCUS: u32 = 0x0007;
pub const WM_KILLFOCUS: u32 = 0x0008;
pub const WM_ENABLE: u32 = 0x000A;
pub const WM_SETTEXT: u32 = 0x000C;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SHOWWINDOW: u32 = 0x0018;
pub const WM_SETTINGCHANGE: u32 = 0x001A;
pub const WM_ACTIVATEAPP: u32 = 0x001C;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_MOUSEACTIVATE: u32 = 0x0021;
pub const WM_GETMINMAXINFO: u32 = 0x0024;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_COPYDATA: u32 = 0x004A;
pub const WM_NOTIFY: u32 = 0x004E;
pub const WM_CONTEXTMENU: u32 = 0x007B;
pub const WM_NCCREATE: u32 = 0x0081;
pub const WM_NCDESTROY: u32 = 0x0082;
pub const WM_NCHITTEST: u32 = 0x0084;
pub const WM_GETDLGCODE: u32 = 0x0087;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_CHAR: u32 = 0x0102;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSCHAR: u32 = 0x0106;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_CTLCOLOREDIT: u32 = 0x0133;
pub const WM_CTLCOLORLISTBOX: u32 = 0x0134;
pub const WM_CTLCOLORSTATIC: u32 = 0x0138;
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;
pub const WM_CAPTURECHANGED: u32 = 0x0215;
pub const WM_DEVICECHANGE: u32 = 0x0219;
pub const WM_DROPFILES: u32 = 0x0233;
pub const WM_MOUSELEAVE: u32 = 0x02A3;
pub const WM_DPICHANGED: u32 = 0x02E0;
pub const WM_THEMECHANGED: u32 = 0x031A;
pub const WM_USER: u32 = 0x0400;
pub const WM_APP: u32 = 0x8000;

pub const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CHILD: u32 = 0x4000_0000;
pub const WS_VISIBLE: u32 = 0x1000_0000;
pub const WS_DISABLED: u32 = 0x0800_0000;
pub const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
pub const WS_CLIPCHILDREN: u32 = 0x0200_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_BORDER: u32 = 0x0080_0000;
pub const WS_VSCROLL: u32 = 0x0020_0000;
pub const WS_HSCROLL: u32 = 0x0010_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const WS_MINIMIZEBOX: u32 = 0x0002_0000;
pub const WS_GROUP: u32 = 0x0002_0000;
pub const WS_TABSTOP: u32 = 0x0001_0000;

pub const WS_EX_DLGMODALFRAME: u32 = 0x0001;
pub const WS_EX_TOPMOST: u32 = 0x0008;
pub const WS_EX_ACCEPTFILES: u32 = 0x0010;
pub const WS_EX_TRANSPARENT: u32 = 0x0020;
pub const WS_EX_TOOLWINDOW: u32 = 0x0080;
pub const WS_EX_CLIENTEDGE: u32 = 0x0200;
pub const WS_EX_CONTROLPARENT: u32 = 0x0001_0000;
pub const WS_EX_LAYERED: u32 = 0x0008_0000;
pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

pub const CS_VREDRAW: u32 = 0x0001;
pub const CS_HREDRAW: u32 = 0x0002;
pub const CS_DBLCLKS: u32 = 0x0008;
pub const CS_DROPSHADOW: u32 = 0x0002_0000;

pub const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;

pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWNOACTIVATE: i32 = 4;
pub const SW_SHOW: i32 = 5;
pub const SW_SHOWNA: i32 = 8;
pub const SW_RESTORE: i32 = 9;

pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const SWP_SHOWWINDOW: u32 = 0x0040;
pub const SWP_HIDEWINDOW: u32 = 0x0080;
pub const HWND_TOPMOST: HWND = -1;

pub const GWLP_USERDATA: i32 = -21;
pub const GWL_STYLE: i32 = -16;
pub const GA_ROOT: u32 = 2;

pub const MB_OK: u32 = 0x0;
pub const MB_OKCANCEL: u32 = 0x1;
pub const MB_YESNO: u32 = 0x4;
pub const MB_ICONERROR: u32 = 0x10;
pub const MB_ICONQUESTION: u32 = 0x20;
pub const MB_ICONWARNING: u32 = 0x30;
pub const MB_ICONINFORMATION: u32 = 0x40;
pub const MB_DEFBUTTON2: u32 = 0x100;
pub const IDOK: i32 = 1;
pub const IDCANCEL: i32 = 2;
pub const IDYES: i32 = 6;

pub const DT_LEFT: u32 = 0x0;
pub const DT_CENTER: u32 = 0x1;
pub const DT_RIGHT: u32 = 0x2;
pub const DT_VCENTER: u32 = 0x4;
pub const DT_WORDBREAK: u32 = 0x10;
pub const DT_SINGLELINE: u32 = 0x20;
pub const DT_CALCRECT: u32 = 0x400;
pub const DT_EXPANDTABS: u32 = 0x40;
pub const DT_EDITCONTROL: u32 = 0x2000;
pub const DT_NOPREFIX: u32 = 0x800;
pub const DT_PATH_ELLIPSIS: u32 = 0x4000;
pub const DT_END_ELLIPSIS: u32 = 0x8000;

pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;
pub const VK_MBUTTON: i32 = 0x04;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_MENU: i32 = 0x12;
pub const VK_LWIN: i32 = 0x5B;
pub const VK_RWIN: i32 = 0x5C;
pub const WH_KEYBOARD_LL: i32 = 13;
pub const HC_ACTION: i32 = 0;
pub const LLKHF_INJECTED: u32 = 0x10;
pub const INPUT_KEYBOARD: u32 = 1;
pub const KEYEVENTF_KEYUP: u32 = 0x0002;
/// 訊息視窗（不顯示、不收廣播）的父視窗
pub const HWND_MESSAGE: HWND = -3;
pub const ASFW_ANY: u32 = u32::MAX;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
/// 輸入法（例如注音）先處理了這個按鍵；原本的按鍵用 ImmGetVirtualKey 取回
pub const VK_PROCESSKEY: u32 = 0xE5;
pub const GCS_COMPSTR: u32 = 0x0008;
pub const VK_PRIOR: u32 = 0x21;
pub const VK_NEXT: u32 = 0x22;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_APPS: u32 = 0x5D;
pub const VK_F2: u32 = 0x71;
pub const VK_F5: u32 = 0x74;
/// 鍵盤上的瀏覽器「上一頁」「下一頁」鍵
pub const VK_BROWSER_BACK: u32 = 0xA6;
pub const VK_BROWSER_FORWARD: u32 = 0xA7;
pub const VK_F10: u32 = 0x79;
pub const VK_F12: u32 = 0x7B;
pub const VK_OEM_COMMA: u32 = 0xBC;

pub const TPM_RIGHTBUTTON: u32 = 0x0002;
pub const TPM_VERTICAL: u32 = 0x0040;
pub const MIIM_STATE: u32 = 0x0001;
pub const MIIM_ID: u32 = 0x0002;
pub const MIIM_SUBMENU: u32 = 0x0004;
pub const MIIM_STRING: u32 = 0x0040;
pub const MIIM_BITMAP: u32 = 0x0080;
pub const MIIM_FTYPE: u32 = 0x0100;
pub const MFT_STRING: u32 = 0x0000;
pub const MFT_RADIOCHECK: u32 = 0x0200;
pub const MFT_SEPARATOR: u32 = 0x0800;
pub const MFS_GRAYED: u32 = 0x0003;
pub const MFS_CHECKED: u32 = 0x0008;
pub const TPM_RETURNCMD: u32 = 0x0100;
pub const MF_STRING: u32 = 0x0000;
pub const MF_GRAYED: u32 = 0x0001;
pub const MF_CHECKED: u32 = 0x0008;
pub const MF_SEPARATOR: u32 = 0x0800;
pub const MF_POPUP: u32 = 0x0010;
pub const MF_BYPOSITION: u32 = 0x0400;
pub const WM_DRAWITEM: u32 = 0x002B;
pub const WM_MEASUREITEM: u32 = 0x002C;
pub const WM_INITMENUPOPUP: u32 = 0x0117;
pub const WM_MENUCHAR: u32 = 0x0120;
pub const CMF_NORMAL: u32 = 0x0;
pub const CMF_ITEMMENU: u32 = 0x80;
pub const CMF_EXTENDEDVERBS: u32 = 0x100;
pub const GCS_VERBW: u32 = 0x4;
pub const CMIC_MASK_UNICODE: u32 = 0x4000;
pub const CMIC_MASK_SHIFT_DOWN: u32 = 0x1000_0000;
pub const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;
pub const CMIC_MASK_CONTROL_DOWN: u32 = 0x4000_0000;

pub const TME_LEAVE: u32 = 0x0002;

pub const IDC_ARROW: usize = 32512;
pub const IDC_IBEAM: usize = 32513;
pub const IDC_SIZEWE: usize = 32644;
pub const IDC_HAND: usize = 32649;

pub const SM_CXSMICON: i32 = 49;
pub const SM_CXICON: i32 = 11;
pub const IMAGE_ICON: u32 = 1;
pub const LR_DEFAULTCOLOR: u32 = 0;
pub const WM_SETICON: u32 = 0x0080;
pub const ICON_SMALL: usize = 0;
pub const ICON_BIG: usize = 1;
pub const SM_CYSMICON: i32 = 50;
pub const SM_CXVSCROLL: i32 = 2;

pub const MONITOR_DEFAULTTONEAREST: u32 = 2;
pub const LWA_ALPHA: u32 = 0x2;
pub const SMTO_ABORTIFHUNG: u32 = 0x2;

pub const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4;

pub const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
pub const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
pub const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
pub const DWMWA_BORDER_COLOR: u32 = 34;
pub const DWMWA_CAPTION_COLOR: u32 = 35;
pub const DWMWA_TEXT_COLOR: u32 = 36;
pub const DWMWCP_ROUND: u32 = 2;
pub const WM_SYSCOMMAND: u32 = 0x0112;
pub const SC_KEYMENU: usize = 0xF100;
pub const GWL_EXSTYLE: i32 = -20;
pub const HTERROR: i32 = -2;
pub const HTTRANSPARENT: LRESULT = -1;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const WM_XBUTTONUP: u32 = 0x020C;
/// 滑鼠側鍵：上一頁（XBUTTON1）、下一頁（XBUTTON2），在 wParam 的高位字組
pub const XBUTTON1: u16 = 1;
pub const XBUTTON2: u16 = 2;
pub const VK_F4: u32 = 0x73;
pub const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
pub const PW_CLIENTONLY: u32 = 0x1;
pub const PW_RENDERFULLCONTENT: u32 = 0x2;
pub const FLASHW_CAPTION: u32 = 0x1;
pub const MA_NOACTIVATE: LRESULT = 3;
pub const TVGN_NEXTVISIBLE: usize = 0x6;
pub const TVGN_FIRSTVISIBLE: usize = 0x5;
pub const TVM_GETITEMRECT: u32 = 0x1104;
pub const TVM_ENSUREVISIBLE: u32 = 0x1114;

pub const CF_UNICODETEXT: u32 = 13;
pub const CF_HDROP: u32 = 15;
pub const GMEM_MOVEABLE: u32 = 0x0002;
pub const GMEM_ZEROINIT: u32 = 0x0040;
pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;
pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_LINK: u32 = 4;
pub const DVASPECT_CONTENT: u32 = 1;
pub const TYMED_HGLOBAL: u32 = 1;
pub const CLSCTX_INPROC_SERVER: u32 = 1;
pub const MK_SHIFT: u32 = 0x4;
pub const MK_CONTROL: u32 = 0x8;
pub const MK_ALT: u32 = 0x20;
pub const WM_VSCROLL: u32 = 0x0115;
pub const SB_LINEUP: usize = 0;
pub const SB_LINEDOWN: usize = 1;
pub const LVIS_DROPHILITED: u32 = 0x8;
pub const TVGN_DROPHILITE: usize = 0x8;
pub const TVIS_DROPHILITED: u32 = 0x8;
/// DROPDESCRIPTION.type：交給系統顯示預設說明
pub const DROPIMAGE_INVALID: i32 = -1;
pub const DROPIMAGE_LINK: i32 = 4;

pub const FO_MOVE: u32 = 1;
pub const FO_COPY: u32 = 2;
pub const FO_DELETE: u32 = 3;
pub const FOF_RENAMEONCOLLISION: u16 = 0x0008;
pub const FOF_NOCONFIRMATION: u16 = 0x0010;
pub const FOF_ALLOWUNDO: u16 = 0x0040;
pub const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
pub const FOF_WANTNUKEWARNING: u16 = 0x4000;

pub const SHGFI_SMALLICON: u32 = 0x0001;
pub const SHGFI_USEFILEATTRIBUTES: u32 = 0x0010;
pub const SHGFI_ICON: u32 = 0x0100;
pub const SHGFI_SYSICONINDEX: u32 = 0x4000;
pub const SHCNE_UPDATEDIR: i32 = 0x0000_1000;
pub const SHCNF_PATHW: u32 = 0x0005;
pub const ERROR_NOT_SAME_DEVICE: u32 = 17;
pub const SHIL_LARGE: i32 = 0;
pub const SHIL_SMALL: i32 = 1;
pub const SHIL_EXTRALARGE: i32 = 2;
pub const SHIL_JUMBO: i32 = 4;
pub const MK_LBUTTON: u32 = 0x1;
pub const MK_RBUTTON: u32 = 0x2;
pub const DRAGDROP_S_DROP: HRESULT = 0x0004_0100;
pub const DRAGDROP_S_CANCEL: HRESULT = 0x0004_0101;
pub const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = 0x0004_0102;
pub const DSH_ALLOWDROPDESCRIPTIONTEXT: u32 = 0x1;
pub const SHGSI_SMALLICON: u32 = 0x0001;
pub const SHGSI_ICON: u32 = 0x0100;
pub const SIID_FOLDER: i32 = 3;
pub const SIID_DRIVEREMOVE: i32 = 7;
pub const SIID_DRIVEFIXED: i32 = 8;
pub const SIID_DRIVENET: i32 = 9;
pub const SIID_DRIVECD: i32 = 11;

pub const SEE_MASK_INVOKEIDLIST: u32 = 0x0000_000C;
pub const SEE_MASK_FLAG_NO_UI: u32 = 0x0000_0400;

pub const DRIVE_REMOVABLE: u32 = 2;
pub const I_IMAGENONE: i32 = -2;
pub const DRIVE_FIXED: u32 = 3;
pub const DRIVE_REMOTE: u32 = 4;
pub const DRIVE_CDROM: u32 = 5;

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
pub const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
pub const INVALID_HANDLE_VALUE: HANDLE = -1;
pub const FIND_EX_INFO_BASIC: i32 = 1;
pub const FIND_EX_SEARCH_NAME_MATCH: i32 = 0;
pub const FIND_FIRST_EX_LARGE_FETCH: u32 = 2;
pub const GET_FILE_EX_INFO_STANDARD: i32 = 0;

pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_NO_MORE_FILES: u32 = 18;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_ALREADY_EXISTS_FILE: u32 = 80; // ERROR_FILE_EXISTS

pub const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x0200;
pub const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x1000;

pub const SEM_FAILCRITICALERRORS: u32 = 0x0001;
pub const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;

pub const COINIT_APARTMENTTHREADED: u32 = 0x2;
pub const COINIT_DISABLE_OLE1DDE: u32 = 0x4;

pub const SIIGBF_RESIZETOFIT: u32 = 0x0;
pub const SIIGBF_THUMBNAILONLY: u32 = 0x8;

pub const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as i32 as isize;
pub const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
pub const RRF_RT_REG_SZ: u32 = 0x0000_0002;
pub const REG_SZ: u32 = 1;
pub const REG_DWORD: u32 = 4;
pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_SET_VALUE: u32 = 0x0002;
pub const KEY_READ: u32 = 0x0002_0019;
pub const KEY_WRITE: u32 = 0x0002_0006;
pub const REG_OPTION_NON_VOLATILE: u32 = 0;
pub const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
pub const SHCNF_IDLIST: u32 = 0;

pub const DIB_RGB_COLORS: u32 = 0;
pub const BI_RGB: u32 = 0;
pub const SRCCOPY: u32 = 0x00CC_0020;
pub const TRANSPARENT: i32 = 1;
pub const NULL_BRUSH: i32 = 5;
pub const NULL_PEN: i32 = 8;
pub const DC_BRUSH: i32 = 18;
pub const DC_PEN: i32 = 19;
pub const PS_SOLID: i32 = 0;
pub const FW_NORMAL: i32 = 400;
pub const FW_SEMIBOLD: i32 = 600;
pub const DEFAULT_CHARSET: u32 = 1;
pub const CLEARTYPE_QUALITY: u32 = 5;

pub const CC_RGBINIT: u32 = 0x1;
pub const CC_FULLOPEN: u32 = 0x2;

pub const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;

pub const ICC_LISTVIEW_CLASSES: u32 = 0x1;
pub const ICC_TREEVIEW_CLASSES: u32 = 0x2;
pub const ICC_BAR_CLASSES: u32 = 0x4;
pub const ICC_TAB_CLASSES: u32 = 0x8;
pub const ICC_STANDARD_CLASSES: u32 = 0x4000;

pub const ILC_MASK: u32 = 0x1;
pub const ILC_COLOR32: u32 = 0x20;

// Edit
pub const ES_AUTOHSCROLL: u32 = 0x0080;
pub const EM_SETSEL: u32 = 0x00B1;
pub const EM_LIMITTEXT: u32 = 0x00C5;
pub const EM_SETCUEBANNER: u32 = 0x1501;
pub const EN_SETFOCUS: u32 = 0x0100;
pub const EN_KILLFOCUS: u32 = 0x0200;
pub const EN_CHANGE: u32 = 0x0300;

// Static
pub const SS_LEFTNOWORDWRAP: u32 = 0x000C;
pub const SS_NOPREFIX: u32 = 0x0080;
pub const SS_ENDELLIPSIS: u32 = 0x4000;

// Dialog codes
pub const DLGC_WANTARROWS: LRESULT = 0x0001;
pub const DLGC_WANTALLKEYS: LRESULT = 0x0004;
pub const DLGC_BUTTON: LRESULT = 0x2000;

// List-view
pub const LVS_REPORT: u32 = 0x0001;
pub const LVS_SHOWSELALWAYS: u32 = 0x0008;
pub const LVS_SINGLESEL: u32 = 0x0004;
pub const LVS_SHAREIMAGELISTS: u32 = 0x0040;
pub const LVS_EDITLABELS: u32 = 0x0200;
pub const LVS_OWNERDATA: u32 = 0x1000;
pub const LVS_EX_FULLROWSELECT: u32 = 0x0020;
pub const LVS_EX_INFOTIP: u32 = 0x0400;
pub const LVS_EX_LABELTIP: u32 = 0x4000;
pub const LVS_EX_DOUBLEBUFFER: u32 = 0x0001_0000;
pub const LVM_SETBKCOLOR: u32 = 0x1001;
pub const LVM_SETIMAGELIST: u32 = 0x1003;
pub const LVM_GETITEMCOUNT: u32 = 0x1004;
pub const LVM_GETNEXTITEM: u32 = 0x100C;
pub const LVM_GETITEMRECT: u32 = 0x100E;
pub const LVM_HITTEST: u32 = 0x1012;
pub const LVM_SUBITEMHITTEST: u32 = 0x1039;
pub const LVM_ENSUREVISIBLE: u32 = 0x1013;
pub const LVM_SCROLL: u32 = 0x1014;
pub const LVM_REDRAWITEMS: u32 = 0x1015;
pub const LVM_GETEDITCONTROL: u32 = 0x1018;
pub const LVM_GETCOLUMNWIDTH: u32 = 0x101D;
pub const LVM_SETCOLUMNWIDTH: u32 = 0x101E;
pub const LVM_GETHEADER: u32 = 0x101F;
pub const LVM_SETTEXTCOLOR: u32 = 0x1024;
pub const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
pub const LVM_GETTOPINDEX: u32 = 0x1027;
pub const LVM_GETCOUNTPERPAGE: u32 = 0x1028;
pub const LVM_SETITEMSTATE: u32 = 0x102B;
pub const LVM_GETITEMSTATE: u32 = 0x102C;
pub const LVM_SETITEMCOUNT: u32 = 0x102F;
pub const LVM_GETSELECTEDCOUNT: u32 = 0x1032;
pub const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
/// 清單自己的提示視窗（截斷的名稱滑鼠停上去時顯示完整名稱）
pub const LVM_GETTOOLTIPS: u32 = 0x104E;
pub const LVM_SETSELECTIONMARK: u32 = 0x1043;
pub const LVM_INSERTCOLUMNW: u32 = 0x1061;
pub const LVM_EDITLABELW: u32 = 0x1076;
pub const LVM_CANCELEDITLABEL: u32 = 0x10B3;
pub const LVSICF_NOINVALIDATEALL: usize = 0x1;
pub const LVSICF_NOSCROLL: usize = 0x2;
pub const LVSIL_NORMAL: usize = 0;
pub const LVSIL_SMALL: usize = 1;
pub const LVM_SETICONSPACING: u32 = 0x1035;
pub const LVM_SETVIEW: u32 = 0x108E;
pub const LVM_SETTILEVIEWINFO: u32 = 0x10A2;
pub const LV_VIEW_ICON: usize = 0;
pub const LV_VIEW_DETAILS: usize = 1;
pub const LV_VIEW_SMALLICON: usize = 2;
pub const LV_VIEW_LIST: usize = 3;
pub const LV_VIEW_TILE: usize = 4;
pub const LVIF_COLUMNS: u32 = 0x200;
pub const LVIF_COLFMT: u32 = 0x10000;
pub const LVTVIM_TILESIZE: u32 = 0x1;
pub const LVTVIM_COLUMNS: u32 = 0x2;
pub const LVTVIM_LABELMARGIN: u32 = 0x4;
pub const LVTVIF_AUTOSIZE: u32 = 0x0;
pub const LVTVIF_FIXEDWIDTH: u32 = 0x1;
pub const LVIF_TEXT: u32 = 0x1;
pub const LVIF_IMAGE: u32 = 0x2;
pub const LVIF_STATE: u32 = 0x8;
pub const LVIS_FOCUSED: u32 = 0x1;
pub const LVIS_SELECTED: u32 = 0x2;
pub const LVNI_ALL: u32 = 0x0;
pub const LVNI_FOCUSED: u32 = 0x1;
pub const LVNI_SELECTED: u32 = 0x2;
pub const LVCF_FMT: u32 = 0x1;
pub const LVCF_WIDTH: u32 = 0x2;
pub const LVCF_TEXT: u32 = 0x4;
pub const LVCF_SUBITEM: u32 = 0x8;
pub const LVCFMT_LEFT: i32 = 0;
pub const LVCFMT_RIGHT: i32 = 1;
pub const LVHT_ONITEM: u32 = 0x000E;
pub const LVFI_STRING: u32 = 0x2;
pub const LVFI_PARTIAL: u32 = 0x8;
pub const LVFI_WRAP: u32 = 0x20;
pub const LVIR_BOUNDS: i32 = 0;

pub const HDM_GETITEMW: u32 = 0x120B;
pub const HDM_SETITEMW: u32 = 0x120C;
pub const HDI_FORMAT: u32 = 0x4;
pub const HDF_SORTDOWN: i32 = 0x0200;
pub const HDF_SORTUP: i32 = 0x0400;

// Notifications（以 u32 表示，比較時直接用 wrapping 值）
const fn n(v: i32) -> u32 {
    v as u32
}
pub const NM_CLICK: u32 = n(-2);
pub const NM_DBLCLK: u32 = n(-3);
pub const NM_RETURN: u32 = n(-4);
pub const NM_RCLICK: u32 = n(-5);
pub const NM_SETFOCUS: u32 = n(-7);
pub const NM_CUSTOMDRAW: u32 = n(-12);
pub const LVN_ITEMCHANGED: u32 = n(-101);
pub const LVN_COLUMNCLICK: u32 = n(-108);
pub const LVN_BEGINDRAG: u32 = n(-109);
pub const LVN_BEGINRDRAG: u32 = n(-111);
pub const LVN_ITEMACTIVATE: u32 = n(-114);
pub const LVN_ODSTATECHANGED: u32 = n(-115);
pub const LVN_KEYDOWN: u32 = n(-155);
pub const LVN_GETINFOTIPW: u32 = n(-158);
pub const LVN_BEGINLABELEDITW: u32 = n(-175);
pub const LVN_ENDLABELEDITW: u32 = n(-176);
pub const LVN_GETDISPINFOW: u32 = n(-177);
pub const LVN_ODFINDITEMW: u32 = n(-179);
pub const TVN_KEYDOWN: u32 = n(-412);
pub const TVN_SELCHANGEDW: u32 = n(-451);
pub const TVN_ITEMEXPANDINGW: u32 = n(-454);
pub const TVN_ITEMEXPANDEDW: u32 = n(-455);
pub const TVN_DELETEITEMW: u32 = n(-458);

pub const CDDS_PREPAINT: u32 = 0x1;
pub const CDDS_POSTPAINT: u32 = 0x2;
pub const CDDS_ITEM: u32 = 0x1_0000;
pub const CDDS_ITEMPREPAINT: u32 = CDDS_ITEM | CDDS_PREPAINT;
pub const CDRF_DODEFAULT: LRESULT = 0x0;
pub const CDRF_NEWFONT: LRESULT = 0x2;
pub const CDRF_SKIPDEFAULT: LRESULT = 0x4;
pub const CDRF_NOTIFYPOSTPAINT: LRESULT = 0x10;
pub const CDRF_NOTIFYITEMDRAW: LRESULT = 0x20;
pub const CDIS_SELECTED: u32 = 0x1;
pub const CDIS_FOCUS: u32 = 0x10;
pub const CDIS_HOT: u32 = 0x40;
pub const CLR_NONE: COLORREF = 0xFFFF_FFFF;
pub const ILD_TRANSPARENT: u32 = 0x1;
pub const HDM_GETITEMCOUNT: u32 = 0x1200;
pub const HDM_GETITEMRECT: u32 = 0x1207;

// Tree-view
pub const TVS_HASBUTTONS: u32 = 0x0001;
pub const TVS_LINESATROOT: u32 = 0x0004;
pub const TVS_SHOWSELALWAYS: u32 = 0x0020;
pub const TVS_TRACKSELECT: u32 = 0x0200;
pub const TVS_INFOTIP: u32 = 0x0800;
pub const TVS_FULLROWSELECT: u32 = 0x1000;
pub const TVS_NOHSCROLL: u32 = 0x8000;
pub const TVS_EX_DOUBLEBUFFER: usize = 0x0004;
pub const TVS_EX_FADEINOUTEXPANDOS: usize = 0x0040;
pub const TVM_DELETEITEM: u32 = 0x1101;
pub const TVM_EXPAND: u32 = 0x1102;
pub const TVM_SETIMAGELIST: u32 = 0x1109;
pub const TVM_GETNEXTITEM: u32 = 0x110A;
pub const TVM_SELECTITEM: u32 = 0x110B;
pub const TVM_HITTEST: u32 = 0x1111;
pub const TVM_SETITEMHEIGHT: u32 = 0x111B;
pub const TVM_SETBKCOLOR: u32 = 0x111D;
pub const TVM_SETTEXTCOLOR: u32 = 0x111E;
pub const TVM_SETLINECOLOR: u32 = 0x1128;
pub const TVM_SETEXTENDEDSTYLE: u32 = 0x112C;
pub const TVM_INSERTITEMW: u32 = 0x1132;
pub const TVM_GETITEMSTATE: u32 = 0x1127;
pub const TVM_GETITEMW: u32 = 0x113E;
pub const TVM_GETINDENT: u32 = 0x1106;
pub const TVM_SETITEMW: u32 = 0x113F;
pub const TVGN_ROOT: usize = 0x0;
pub const TVGN_NEXT: usize = 0x1;
pub const TVGN_PARENT: usize = 0x3;
pub const TVGN_CHILD: usize = 0x4;
pub const TVGN_CARET: usize = 0x9;
pub const TVE_COLLAPSE: usize = 0x1;
pub const TVE_EXPAND: usize = 0x2;
pub const TVE_COLLAPSERESET: usize = 0x8000;
pub const TVIF_TEXT: u32 = 0x1;
pub const TVIF_IMAGE: u32 = 0x2;
pub const TVIF_PARAM: u32 = 0x4;
pub const TVIF_STATE: u32 = 0x8;
pub const TVIF_HANDLE: u32 = 0x10;
pub const TVIF_SELECTEDIMAGE: u32 = 0x20;
pub const TVIF_CHILDREN: u32 = 0x40;
pub const TVIS_SELECTED: u32 = 0x2;
pub const TVIS_BOLD: u32 = 0x10;
pub const TVIS_EXPANDED: u32 = 0x20;
pub const TVI_ROOT: HTREEITEM = -0x10000;
pub const TVI_FIRST: HTREEITEM = -0xFFFF;
pub const TVI_LAST: HTREEITEM = -0xFFFE;
pub const TVHT_ONITEMICON: u32 = 0x2;
pub const TVHT_ONITEMLABEL: u32 = 0x4;
pub const TVHT_ONITEMBUTTON: u32 = 0x10;
pub const TVHT_ONITEMRIGHT: u32 = 0x20;
pub const TVSIL_NORMAL: usize = 0;

// Tooltip
pub const TTS_ALWAYSTIP: u32 = 0x01;
pub const TTS_NOPREFIX: u32 = 0x02;
pub const TTF_IDISHWND: u32 = 0x0001;
pub const TTF_SUBCLASS: u32 = 0x0010;
pub const TTM_SETMAXTIPWIDTH: u32 = WM_USER + 24;
/// 立刻收起正在顯示的提示
pub const TTM_POP: u32 = WM_USER + 28;
pub const TTM_ADDTOOLW: u32 = WM_USER + 50;
pub const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;

pub const DBT_DEVICEARRIVAL: usize = 0x8000;
/// OpenProcess：只用來等它結束
pub const SYNCHRONIZE: u32 = 0x0010_0000;
pub const WAIT_OBJECT_0: u32 = 0;
pub const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;

// ---------------------------------------------------------------- 函式

#[cfg_attr(windows, link(name = "kernel32", kind = "raw-dylib"))]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> HMODULE;
    pub fn LoadLibraryW(name: *const u16) -> HMODULE;
    pub fn GetProcAddress(module: HMODULE, name: *const u8) -> *const c_void;
    pub fn GetLastError() -> u32;
    pub fn SetLastError(code: u32);
    pub fn FormatMessageW(flags: u32, source: *const c_void, id: u32, lang: u32, buf: *mut u16, size: u32, args: *const c_void) -> u32;
    pub fn CreateMutexW(attrs: *const c_void, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn ReleaseMutex(h: HANDLE) -> BOOL;
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn FindFirstFileExW(name: *const u16, level: i32, data: *mut WIN32_FIND_DATAW, op: i32, filter: *const c_void, flags: u32) -> HANDLE;
    pub fn FindNextFileW(h: HANDLE, data: *mut WIN32_FIND_DATAW) -> BOOL;
    pub fn FindClose(h: HANDLE) -> BOOL;
    pub fn GetFileAttributesW(name: *const u16) -> u32;
    pub fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn QueryFullProcessImageNameW(process: HANDLE, flags: u32, name: *mut u16, size: *mut u32) -> BOOL;
    pub fn GetFileAttributesExW(name: *const u16, level: i32, info: *mut WIN32_FILE_ATTRIBUTE_DATA) -> BOOL;
    pub fn GetFullPathNameW(name: *const u16, len: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
    pub fn ExpandEnvironmentStringsW(src: *const u16, dst: *mut u16, size: u32) -> u32;
    pub fn GetLogicalDriveStringsW(len: u32, buf: *mut u16) -> u32;
    pub fn GetDriveTypeW(root: *const u16) -> u32;
    pub fn CreateDirectoryW(path: *const u16, attrs: *const c_void) -> BOOL;
    pub fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> BOOL;
    pub fn GlobalAlloc(flags: u32, bytes: usize) -> HGLOBAL;
    pub fn GlobalLock(h: HGLOBAL) -> *mut c_void;
    pub fn GlobalUnlock(h: HGLOBAL) -> BOOL;
    pub fn GlobalFree(h: HGLOBAL) -> HGLOBAL;
    pub fn GlobalSize(h: HGLOBAL) -> usize;
    pub fn MulDiv(a: i32, b: i32, c: i32) -> i32;
    pub fn FileTimeToSystemTime(ft: *const FILETIME, st: *mut SYSTEMTIME) -> BOOL;
    pub fn SystemTimeToTzSpecificLocalTime(tz: *const c_void, utc: *const SYSTEMTIME, local: *mut SYSTEMTIME) -> BOOL;
    pub fn GetCurrentProcessId() -> u32;
    pub fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> BOOL;
    pub fn GetTickCount64() -> u64;
    pub fn SetUnhandledExceptionFilter(filter: Option<unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32>) -> *const c_void;
    pub fn CreateFileW(name: *const u16, access: u32, share: u32, security: *const c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    pub fn WriteFile(file: HANDLE, buf: *const u8, len: u32, written: *mut u32, overlapped: *mut c_void) -> BOOL;
    pub fn FlushFileBuffers(file: HANDLE) -> BOOL;
    pub fn SetStdHandle(which: u32, handle: HANDLE) -> BOOL;
    pub fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32) -> *mut c_void;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetModuleFileNameW(module: HMODULE, buf: *mut u16, len: u32) -> u32;
    pub fn GetLocalTime(st: *mut SYSTEMTIME);
    pub fn MultiByteToWideChar(code_page: u32, flags: u32, src: *const u8, src_len: i32, dst: *mut u16, dst_len: i32) -> i32;
    pub fn QueryPerformanceCounter(count: *mut i64) -> BOOL;
    pub fn QueryPerformanceFrequency(freq: *mut i64) -> BOOL;
    pub fn SetErrorMode(mode: u32) -> u32;
    pub fn CreateActCtxW(ctx: *const ACTCTXW) -> HANDLE;
    pub fn ActivateActCtx(ctx: HANDLE, cookie: *mut usize) -> BOOL;
}

#[cfg_attr(windows, link(name = "imm32", kind = "raw-dylib"))]
extern "system" {
    pub fn ImmGetVirtualKey(hwnd: HWND) -> u32;
    pub fn ImmGetContext(hwnd: HWND) -> isize;
    pub fn ImmReleaseContext(hwnd: HWND, himc: isize) -> BOOL;
    pub fn ImmGetCompositionStringW(himc: isize, index: u32, buf: *mut c_void, len: u32) -> i32;
}

#[cfg_attr(windows, link(name = "user32", kind = "raw-dylib"))]
extern "system" {
    pub fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        inst: HINSTANCE,
        param: *const c_void,
    ) -> HWND;
    pub fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    pub fn GetMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32) -> BOOL;
    pub fn TranslateMessage(msg: *const MSG) -> BOOL;
    pub fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    pub fn IsDialogMessageW(dlg: HWND, msg: *mut MSG) -> BOOL;
    pub fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    pub fn SendMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    pub fn SendMessageTimeoutW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM, flags: u32, timeout: u32, result: *mut usize) -> LRESULT;
    pub fn PostQuitMessage(code: i32);
    pub fn ShowWindow(hwnd: HWND, cmd: i32) -> BOOL;
    pub fn UpdateWindow(hwnd: HWND) -> BOOL;
    pub fn DestroyWindow(hwnd: HWND) -> BOOL;
    pub fn SetWindowPos(hwnd: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> BOOL;
    pub fn BeginDeferWindowPos(n: i32) -> HDWP;
    pub fn DeferWindowPos(h: HDWP, hwnd: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> HDWP;
    pub fn EndDeferWindowPos(h: HDWP) -> BOOL;
    pub fn GetClientRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn GetWindowRect(hwnd: HWND, rect: *mut RECT) -> BOOL;
    pub fn InvalidateRect(hwnd: HWND, rect: *const RECT, erase: BOOL) -> BOOL;
    pub fn BeginPaint(hwnd: HWND, ps: *mut PAINTSTRUCT) -> HDC;
    pub fn EndPaint(hwnd: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    pub fn GetDC(hwnd: HWND) -> HDC;
    pub fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    pub fn FillRect(hdc: HDC, rect: *const RECT, brush: HBRUSH) -> i32;
    pub fn DrawTextW(hdc: HDC, text: *const u16, len: i32, rect: *mut RECT, format: u32) -> i32;
    pub fn DrawIconEx(hdc: HDC, x: i32, y: i32, icon: HICON, cx: i32, cy: i32, step: u32, brush: HBRUSH, flags: u32) -> BOOL;
    pub fn SetWindowTextW(hwnd: HWND, text: *const u16) -> BOOL;
    pub fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    pub fn GetWindowTextLengthW(hwnd: HWND) -> i32;
    pub fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    pub fn SetFocus(hwnd: HWND) -> HWND;
    pub fn GetFocus() -> HWND;
    pub fn EnableWindow(hwnd: HWND, enable: BOOL) -> BOOL;
    pub fn IsWindowEnabled(hwnd: HWND) -> BOOL;
    pub fn IsWindowVisible(hwnd: HWND) -> BOOL;
    pub fn IsWindow(hwnd: HWND) -> BOOL;
    pub fn IsChild(parent: HWND, hwnd: HWND) -> BOOL;
    pub fn IsIconic(hwnd: HWND) -> BOOL;
    pub fn GetParent(hwnd: HWND) -> HWND;
    pub fn GetAncestor(hwnd: HWND, flags: u32) -> HWND;
    pub fn SetCapture(hwnd: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;
    pub fn GetCapture() -> HWND;
    pub fn SetCursor(cursor: HCURSOR) -> HCURSOR;
    pub fn LoadCursorW(inst: HINSTANCE, name: *const u16) -> HCURSOR;
    pub fn LoadImageW(inst: HINSTANCE, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> HANDLE;
    pub fn CreateIconFromResourceEx(bits: *const u8, size: u32, icon: BOOL, version: u32, cx: i32, cy: i32, flags: u32) -> HICON;
    pub fn DestroyIcon(icon: HICON) -> BOOL;
    pub fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    pub fn CreateIconIndirect(info: *const ICONINFO) -> HICON;
    pub fn GetDpiForWindow(hwnd: HWND) -> u32;
    pub fn GetDpiForSystem() -> u32;
    pub fn GetSystemMetrics(index: i32) -> i32;
    pub fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    pub fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    pub fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    pub fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    pub fn FindWindowW(class: *const u16, name: *const u16) -> HWND;
    pub fn FindWindowExW(parent: HWND, after: HWND, class: *const u16, name: *const u16) -> HWND;
    pub fn SetWindowsHookExW(id: i32, proc: HOOKPROC, module: HMODULE, thread: u32) -> isize;
    pub fn UnhookWindowsHookEx(hook: isize) -> BOOL;
    pub fn CallNextHookEx(hook: isize, code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;
    pub fn SendInput(count: u32, inputs: *const KEYBOARD_INPUT, size: i32) -> u32;
    pub fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    pub fn AllowSetForegroundWindow(pid: u32) -> BOOL;
    pub fn SetForegroundWindow(hwnd: HWND) -> BOOL;
    pub fn GetForegroundWindow() -> HWND;
    pub fn WindowFromPoint(pt: POINT) -> HWND;
    pub fn FlashWindow(hwnd: HWND, invert: BOOL) -> BOOL;
    pub fn SetTimer(hwnd: HWND, id: usize, ms: u32, func: *const c_void) -> usize;
    pub fn KillTimer(hwnd: HWND, id: usize) -> BOOL;
    pub fn TrackMouseEvent(tme: *mut TRACKMOUSEEVENT) -> BOOL;
    pub fn GetCursorPos(pt: *mut POINT) -> BOOL;
    pub fn ScreenToClient(hwnd: HWND, pt: *mut POINT) -> BOOL;
    pub fn ClientToScreen(hwnd: HWND, pt: *mut POINT) -> BOOL;
    pub fn CreatePopupMenu() -> HMENU;
    pub fn AppendMenuW(menu: HMENU, flags: u32, id: usize, text: *const u16) -> BOOL;
    pub fn TrackPopupMenu(menu: HMENU, flags: u32, x: i32, y: i32, reserved: i32, hwnd: HWND, rect: *const RECT) -> BOOL;
    pub fn TrackPopupMenuEx(menu: HMENU, flags: u32, x: i32, y: i32, hwnd: HWND, params: *const c_void) -> BOOL;
    pub fn InsertMenuItemW(menu: HMENU, item: u32, by_position: BOOL, info: *const MENUITEMINFOW) -> BOOL;
    pub fn GetMenuItemCount(menu: HMENU) -> i32;
    pub fn GetMenuItemID(menu: HMENU, pos: i32) -> u32;
    pub fn GetMenuState(menu: HMENU, id: u32, flags: u32) -> u32;
    pub fn DeleteMenu(menu: HMENU, position: u32, flags: u32) -> BOOL;
    pub fn SetMenuDefaultItem(menu: HMENU, item: u32, by_position: u32) -> BOOL;
    pub fn DestroyMenu(menu: HMENU) -> BOOL;
    pub fn MessageBoxW(hwnd: HWND, text: *const u16, caption: *const u16, kind: u32) -> i32;
    pub fn OpenClipboard(hwnd: HWND) -> BOOL;
    pub fn CloseClipboard() -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn SetClipboardData(format: u32, mem: HANDLE) -> HANDLE;
    pub fn GetClipboardData(format: u32) -> HANDLE;
    pub fn IsClipboardFormatAvailable(format: u32) -> BOOL;
    pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
    pub fn GetKeyState(vk: i32) -> i16;
    pub fn GetAsyncKeyState(vk: i32) -> i16;
    pub fn GetNextDlgTabItem(dlg: HWND, ctl: HWND, previous: BOOL) -> HWND;
    pub fn MonitorFromPoint(pt: POINT, flags: u32) -> HMONITOR;
    pub fn MonitorFromWindow(hwnd: HWND, flags: u32) -> HMONITOR;
    pub fn GetMonitorInfoW(monitor: HMONITOR, info: *mut MONITORINFO) -> BOOL;
    pub fn SetLayeredWindowAttributes(hwnd: HWND, key: COLORREF, alpha: u8, flags: u32) -> BOOL;
    pub fn DrawFocusRect(hdc: HDC, rect: *const RECT) -> BOOL;
    pub fn SystemParametersInfoW(action: u32, param: u32, pv: *mut c_void, winini: u32) -> BOOL;
    pub fn PrintWindow(hwnd: HWND, hdc: HDC, flags: u32) -> BOOL;
    pub fn GetLastActivePopup(hwnd: HWND) -> HWND;
    pub fn FlashWindowEx(info: *const FLASHWINFO) -> BOOL;
    pub fn SetActiveWindow(hwnd: HWND) -> HWND;
}

#[cfg_attr(windows, link(name = "gdi32", kind = "raw-dylib"))]
extern "system" {
    pub fn CreateFontW(
        h: i32,
        w: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> HFONT;
    pub fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    pub fn CreatePen(style: i32, width: i32, color: COLORREF) -> HPEN;
    pub fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    pub fn SelectObject(hdc: HDC, obj: HGDIOBJ) -> HGDIOBJ;
    pub fn GetStockObject(index: i32) -> HGDIOBJ;
    pub fn SetDCBrushColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetDCPenColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetTextColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetBkColor(hdc: HDC, color: COLORREF) -> COLORREF;
    pub fn SetBkMode(hdc: HDC, mode: i32) -> i32;
    pub fn RoundRect(hdc: HDC, l: i32, t: i32, r: i32, b: i32, w: i32, h: i32) -> BOOL;
    pub fn GetTextExtentPoint32W(hdc: HDC, text: *const u16, len: i32, size: *mut SIZE) -> BOOL;
    pub fn GetTextFaceW(hdc: HDC, len: i32, buf: *mut u16) -> i32;
    pub fn CreateCompatibleDC(hdc: HDC) -> HDC;
    pub fn GdiFlush() -> BOOL;
    pub fn DeleteDC(hdc: HDC) -> BOOL;
    pub fn CreateCompatibleBitmap(hdc: HDC, w: i32, h: i32) -> HBITMAP;
    pub fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const c_void) -> HBITMAP;
    pub fn BitBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, rop: u32) -> BOOL;
    pub fn StretchBlt(hdc: HDC, x: i32, y: i32, w: i32, h: i32, src: HDC, sx: i32, sy: i32, sw: i32, sh: i32, rop: u32) -> BOOL;
    pub fn SetStretchBltMode(hdc: HDC, mode: i32) -> i32;
    pub fn SetBrushOrgEx(hdc: HDC, x: i32, y: i32, old: *mut POINT) -> BOOL;
    pub fn TextOutW(hdc: HDC, x: i32, y: i32, text: *const u16, len: i32) -> BOOL;
    pub fn CreateDIBSection(hdc: HDC, info: *const BITMAPINFO, usage: u32, bits: *mut *mut c_void, section: HANDLE, offset: u32) -> HBITMAP;
    pub fn GetDIBits(hdc: HDC, bm: HBITMAP, start: u32, lines: u32, bits: *mut c_void, info: *mut BITMAPINFO, usage: u32) -> i32;
    pub fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
    pub fn GetPixel(hdc: HDC, x: i32, y: i32) -> COLORREF;
    pub fn SaveDC(hdc: HDC) -> i32;
    pub fn RestoreDC(hdc: HDC, saved: i32) -> BOOL;
    pub fn IntersectClipRect(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) -> i32;
}

#[cfg_attr(windows, link(name = "shell32", kind = "raw-dylib"))]
extern "system" {
    pub fn ShellExecuteW(hwnd: HWND, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> HINSTANCE;
    pub fn ShellExecuteExW(info: *mut SHELLEXECUTEINFOW) -> BOOL;
    pub fn SHFileOperationW(op: *mut SHFILEOPSTRUCTW) -> i32;
    pub fn SHGetFileInfoW(path: *const u16, attrs: u32, info: *mut SHFILEINFOW, size: u32, flags: u32) -> usize;
    pub fn SHGetStockIconInfo(id: i32, flags: u32, info: *mut SHSTOCKICONINFO) -> HRESULT;
    pub fn SHGetKnownFolderPath(id: *const GUID, flags: u32, token: HANDLE, path: *mut *mut u16) -> HRESULT;
    pub fn SHCreateItemFromParsingName(path: *const u16, bc: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn SHParseDisplayName(name: *const u16, bc: *mut c_void, pidl: *mut *mut c_void, sfgao_in: u32, sfgao_out: *mut u32) -> HRESULT;
    pub fn SHBindToParent(pidl: *const c_void, riid: *const GUID, out: *mut *mut c_void, last: *mut *const c_void) -> HRESULT;
    pub fn SHBindToObject(folder: *mut c_void, pidl: *const c_void, bc: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn SHChangeNotify(event: i32, flags: u32, item1: *const c_void, item2: *const c_void);
    pub fn SHGetPropertyStoreFromParsingName(path: *const u16, bind: *mut c_void, flags: u32, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn SHGetImageList(kind: i32, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn SHCreateDataObject(folder: *const c_void, count: u32, items: *const *const c_void, inner: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn DragAcceptFiles(hwnd: HWND, accept: BOOL);
    pub fn DragQueryFileW(drop: HANDLE, index: u32, buf: *mut u16, len: u32) -> u32;
    pub fn DragFinish(drop: HANDLE);
}

#[cfg_attr(windows, link(name = "ole32", kind = "raw-dylib"))]
extern "system" {
    pub fn PropVariantClear(pv: *mut PROPVARIANT) -> HRESULT;
    pub fn CoInitializeEx(reserved: *const c_void, flags: u32) -> HRESULT;
    pub fn CoUninitialize();
    pub fn CoTaskMemFree(p: *mut c_void);
    pub fn CoCreateInstance(clsid: *const GUID, outer: *mut c_void, ctx: u32, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn OleInitialize(reserved: *mut c_void) -> HRESULT;
    pub fn RegisterDragDrop(hwnd: HWND, target: *mut c_void) -> HRESULT;
    pub fn RevokeDragDrop(hwnd: HWND) -> HRESULT;
    pub fn ReleaseStgMedium(medium: *mut STGMEDIUM);
    pub fn DoDragDrop(data: *mut c_void, source: *mut c_void, effects: u32, effect: *mut u32) -> HRESULT;
}

#[cfg_attr(windows, link(name = "uxtheme", kind = "raw-dylib"))]
extern "system" {
    pub fn SetWindowTheme(hwnd: HWND, app: *const u16, idlist: *const u16) -> HRESULT;
}

#[cfg_attr(windows, link(name = "dwmapi", kind = "raw-dylib"))]
extern "system" {
    pub fn DwmSetWindowAttribute(hwnd: HWND, attr: u32, value: *const c_void, size: u32) -> HRESULT;
}

#[cfg_attr(windows, link(name = "comdlg32", kind = "raw-dylib"))]
extern "system" {
    pub fn ChooseColorW(cc: *mut CHOOSECOLORW) -> BOOL;
}

#[cfg_attr(windows, link(name = "advapi32", kind = "raw-dylib"))]
extern "system" {
    pub fn GetUserNameW(buf: *mut u16, len: *mut u32) -> BOOL;
    pub fn RegGetValueW(key: HKEY, sub: *const u16, value: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
    pub fn RegCreateKeyExW(key: HKEY, sub: *const u16, reserved: u32, class: *const u16, options: u32, sam: u32, security: *const c_void, result: *mut HKEY, disposition: *mut u32) -> i32;
    pub fn RegOpenKeyExW(key: HKEY, sub: *const u16, options: u32, sam: u32, result: *mut HKEY) -> i32;
    pub fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    pub fn RegDeleteValueW(key: HKEY, name: *const u16) -> i32;
    pub fn RegDeleteTreeW(key: HKEY, sub: *const u16) -> i32;
    pub fn RegDeleteKeyW(key: HKEY, sub: *const u16) -> i32;
    pub fn RegQueryInfoKeyW(
        key: HKEY,
        class: *mut u16,
        class_len: *mut u32,
        reserved: *mut u32,
        subkeys: *mut u32,
        max_subkey_len: *mut u32,
        max_class_len: *mut u32,
        values: *mut u32,
        max_value_name_len: *mut u32,
        max_value_len: *mut u32,
        security: *mut u32,
        last_write: *mut FILETIME,
    ) -> i32;
    pub fn RegCloseKey(key: HKEY) -> i32;
}

// ---------------------------------------------------------------- comctl32 v6（執行期載入）

pub struct ComCtl {
    pub init_common_controls_ex: unsafe extern "system" fn(*const INITCOMMONCONTROLSEX) -> BOOL,
    pub set_window_subclass: unsafe extern "system" fn(HWND, SUBCLASSPROC, usize, usize) -> BOOL,
    pub def_subclass_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    pub image_list_create: unsafe extern "system" fn(i32, i32, u32, i32, i32) -> HIMAGELIST,
    pub image_list_replace_icon: unsafe extern "system" fn(HIMAGELIST, i32, HICON) -> i32,
    pub image_list_destroy: unsafe extern "system" fn(HIMAGELIST) -> BOOL,
    pub image_list_draw: unsafe extern "system" fn(HIMAGELIST, i32, HDC, i32, i32, u32) -> BOOL,
    pub image_list_get_icon: unsafe extern "system" fn(HIMAGELIST, i32, u32) -> HICON,
}

static COMCTL: std::sync::OnceLock<ComCtl> = std::sync::OnceLock::new();

pub fn comctl() -> &'static ComCtl {
    COMCTL.get().expect("comctl32 尚未載入")
}

/// 依名稱取得函式，失敗時改用序數（SetWindowSubclass 等在舊版只以序數匯出）。
pub unsafe fn proc_address(module: HMODULE, name: &str, ordinal: usize) -> *const c_void {
    let mut z: Vec<u8> = name.bytes().collect();
    z.push(0);
    let p = GetProcAddress(module, z.as_ptr());
    if !p.is_null() || ordinal == 0 {
        return p;
    }
    GetProcAddress(module, ordinal as *const u8)
}

/// 在已啟用 v6 啟用內容（activation context）後呼叫，載入 comctl32 v6 的函式。
pub unsafe fn load_comctl() -> Result<(), String> {
    let name: Vec<u16> = "comctl32.dll\0".encode_utf16().collect();
    let module = LoadLibraryW(name.as_ptr());
    if module == 0 {
        return Err("無法載入 comctl32.dll".into());
    }
    macro_rules! load {
        ($name:expr, $ord:expr) => {{
            let p = proc_address(module, $name, $ord);
            if p.is_null() {
                return Err(format!("comctl32.dll 缺少 {}", $name));
            }
            std::mem::transmute::<*const c_void, _>(p)
        }};
    }
    let table = ComCtl {
        init_common_controls_ex: load!("InitCommonControlsEx", 0),
        set_window_subclass: load!("SetWindowSubclass", 410),
        def_subclass_proc: load!("DefSubclassProc", 413),
        image_list_create: load!("ImageList_Create", 0),
        image_list_replace_icon: load!("ImageList_ReplaceIcon", 0),
        image_list_destroy: load!("ImageList_Destroy", 0),
        image_list_draw: load!("ImageList_Draw", 0),
        image_list_get_icon: load!("ImageList_GetIcon", 0),
    };
    let _ = COMCTL.set(table);
    Ok(())
}

// ---------------------------------------------------------------- 未公開的深色模式輔助（uxtheme 序數）

/// uxtheme #135 SetPreferredAppMode(int)：讓右鍵選單等系統元件跟隨深色。找不到時略過。
pub unsafe fn set_preferred_app_mode(mode: i32) {
    let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
    let module = LoadLibraryW(name.as_ptr());
    if module == 0 {
        return;
    }
    let p = GetProcAddress(module, 135usize as *const u8);
    if !p.is_null() {
        let f: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(p);
        f(mode);
    }
    let flush = GetProcAddress(module, 136usize as *const u8);
    if !flush.is_null() {
        let f: unsafe extern "system" fn() = std::mem::transmute(flush);
        f();
    }
}

/// uxtheme #133 AllowDarkModeForWindow(HWND, BOOL)。
pub unsafe fn allow_dark_mode_for_window(hwnd: HWND, allow: bool) {
    let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
    let module = LoadLibraryW(name.as_ptr());
    if module == 0 {
        return;
    }
    let p = GetProcAddress(module, 133usize as *const u8);
    if !p.is_null() {
        let f: unsafe extern "system" fn(HWND, BOOL) -> BOOL = std::mem::transmute(p);
        f(hwnd, allow as BOOL);
    }
}

// ---------------------------------------------------------------- 版面自我檢查

#[cfg(test)]
mod layout_tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn x64_struct_sizes() {
        // 這些大小對應 x64 Windows SDK；在 64 位元的任何平台上檢查都成立。
        if size_of::<usize>() != 8 {
            return;
        }
        assert_eq!(size_of::<WNDCLASSEXW>(), 80);
        assert_eq!(size_of::<MSG>(), 48);
        assert_eq!(size_of::<KBDLLHOOKSTRUCT>(), 24);
        assert_eq!(size_of::<KEYBOARD_INPUT>(), 40);
        assert_eq!(std::mem::offset_of!(KEYBOARD_INPUT, ki), 8);
        assert_eq!(size_of::<PAINTSTRUCT>(), 72);
        assert_eq!(size_of::<NMHDR>(), 24);
        assert_eq!(size_of::<LVITEMW>(), 88);
        assert_eq!(size_of::<LVCOLUMNW>(), 56);
        assert_eq!(size_of::<NMLISTVIEW>(), 64);
        assert_eq!(size_of::<NMLVFINDITEMW>(), 72);
        assert_eq!(size_of::<HDITEMW>(), 72);
        assert_eq!(size_of::<TVITEMW>(), 56);
        assert_eq!(size_of::<TVINSERTSTRUCTW>(), 96);
        assert_eq!(size_of::<NMTREEVIEWW>(), 152);
        assert_eq!(size_of::<TOOLINFOW>(), 72);
        assert_eq!(size_of::<WIN32_FIND_DATAW>(), 592);
        assert_eq!(size_of::<SHFILEOPSTRUCTW>(), 56);
        assert_eq!(size_of::<SHFILEINFOW>(), 696);
        assert_eq!(size_of::<SHSTOCKICONINFO>(), 544);
        assert_eq!(size_of::<SHELLEXECUTEINFOW>(), 112);
        assert_eq!(size_of::<DROPFILES>(), 20);
        assert_eq!(size_of::<BITMAPINFOHEADER>(), 40);
        assert_eq!(size_of::<ICONINFO>(), 32);
        assert_eq!(size_of::<MONITORINFO>(), 40);
        assert_eq!(size_of::<CHOOSECOLORW>(), 72);
        assert_eq!(size_of::<COPYDATASTRUCT>(), 24);
        assert_eq!(size_of::<ACTCTXW>(), 56);
        assert_eq!(size_of::<NMCUSTOMDRAW>(), 80);
        assert_eq!(size_of::<NMLVCUSTOMDRAW>(), 136);
        assert_eq!(size_of::<NMTVCUSTOMDRAW>(), 96);
        assert_eq!(size_of::<TRACKMOUSEEVENT>(), 24);
        assert_eq!(size_of::<NMLVGETINFOTIPW>(), 64);
        assert_eq!(size_of::<LVHITTESTINFO>(), 24);
        assert_eq!(size_of::<TVHITTESTINFO>(), 24);
        assert_eq!(size_of::<FLASHWINFO>(), 32);
        assert_eq!(size_of::<CMINVOKECOMMANDINFOEX>(), 104);
        assert_eq!(size_of::<FORMATETC>(), 32);
        assert_eq!(size_of::<SHDRAGIMAGE>(), 32);
        assert_eq!(size_of::<STGMEDIUM>(), 24);
        assert_eq!(size_of::<DROPDESCRIPTION>(), 1044);
        assert_eq!(size_of::<LVTILEVIEWINFO>(), 40);
        assert_eq!(size_of::<MENUITEMINFOW>(), 80);
        assert_eq!(size_of::<TPMPARAMS>(), 20);
        assert_eq!(size_of::<PROPERTYKEY>(), 20);
        assert_eq!(size_of::<PROPVARIANT>(), 24);
        assert_eq!(size_of::<EXCEPTION_RECORD>(), 152);
        assert_eq!(size_of::<EXCEPTION_POINTERS>(), 16);
    }

    #[test]
    fn guid_layout() {
        let g = GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);
        assert_eq!(g.data1, 0xbcc18b79);
        assert_eq!(g.data2, 0xba16);
        assert_eq!(g.data3, 0x442f);
        assert_eq!(g.data4, [0x80, 0xc4, 0x8a, 0x59, 0xc3, 0x0c, 0x46, 0x3b]);
    }
}

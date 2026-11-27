//! 「在資料夾中顯示」：讓其他程式找得到 Stillfeather 的視窗，並選取要顯示的檔案。
//!
//! 瀏覽器的下載「在資料夾中顯示」、各種「開啟所在資料夾」用的是 `SHOpenFolderAndSelectItems`。它先在
//! Windows 的殼層視窗清單（`IShellWindows`，檔案總管的視窗都登記在這裡）找正在顯示那個資料夾的視窗；
//! 找不到就用預設動詞開啟資料夾（Stillfeather 是預設檔案總管時開的是 Stillfeather，但只收到資料夾、沒有檔案），
//! 再一直等清單裡出現那個資料夾的視窗（約 15 秒）。Stillfeather 不在清單裡，所以：
//! - 只開資料夾，不會選取檔案；
//! - 每次都等到逾時才結束（越點越慢），排隊中的要求一個接一個重新開啟視窗（關掉又跳出來）。
//!
//! 解法和 Files（files-community/Files 的 `Files.App.Launcher/OpenInFolder.cpp`，實際發行、在 Windows 10／11 可用）相同：
//! - 登記一個隱藏的工具視窗（`WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`、隱形、不顯示），位置是作用中分頁的資料夾：
//!   `RegisterPending` → `Register` → `OnNavigate`，`SWC_BROWSER`。**視窗、物件與登記都在同一個執行緒**（同 Files）：
//!   登記主視窗（屬於介面執行緒）時，登記看起來成功、編號也配對，但 Windows 從來沒有找上它。
//! - **位置以 PIDL 的位元組陣列（`VT_ARRAY | VT_UI1`，同 `InitVariantFromBuffer`）傳入**；以路徑字串傳入時
//!   Windows 也找不到。換資料夾時以 `OnNavigate` 更新；到首頁、搜尋結果時保留上一個資料夾（要選取的檔案
//!   不在作用中的資料夾時，介面執行緒照一般的開啟要求處理）。
//! - 登記的物件提供 `IWebBrowserApp`（也是 `IDispatch`／`IWebBrowser`）、`IServiceProvider`、`IShellView`（也是 `IOleWindow`）。
//!   `QueryService` 不管要哪一個服務，都照要的介面回答。Windows 最後呼叫 `IShellView::SelectItem`，
//!   給的是相對於資料夾的子項目識別碼；轉成完整路徑交給介面執行緒（[`WM_SHELL_SELECT`]），在作用中的那一欄選取並把視窗拉到前面。
//!
//! `RegisterPending` 與 `Register` 是同一次登記的前後兩半：`Register` 以視窗所屬的執行緒（`GetWindowThreadProcessId`）
//! 找到先前的 `RegisterPending`。兩次拿到的編號相同就是配對成功。每次登記後以 `FindWindowSW`（和 Windows 自己找視窗的
//! 參數相同：`SWC_BROWSER`、`SWFO_INCLUDEPENDING | SWFO_NEEDDISPATCH`）自我檢查，結果寫進記錄。
//!
//! 隱藏視窗被啟用時（Windows 把找到的視窗拉到前面）改成把主視窗拉到前面（[`WM_SHELL_FRONT`]）。
//!
//! **啟動時的交接**（[`Handshake`]，同 Files 的啟動程式）：資料夾還沒有人登記時，Windows 用預設動詞啟動
//! Stillfeather，等這個新程序閒下來（開始等候輸入）後找視窗。主視窗的登記要等資料夾讀好才做，已經開著 Stillfeather 時
//! 新程序把位置交給原本的程序就結束，都趕不上。所以由其他程式（不是檔案總管）以單一資料夾啟動時，新程序在開始處理
//! 訊息之前先在主執行緒登記一個隱藏視窗，最多等 0.5 秒選取；收到就改成開啟那個檔案（開它所在的資料夾並選取），
//! 交給原本的 Stillfeather 時再多等 10 秒，晚到的選取也轉過去。由檔案總管啟動（雙擊資料夾）時不必等。
//!
//! 登記與這些物件都在自己的執行緒（STA）：呼叫 explorer.exe 的殼層視窗清單可能要等，檔案總管沒有回應時也不會卡住介面。
//! 只有 Stillfeather 是預設檔案總管時才登記（不是的話，「在資料夾中顯示」照常用檔案總管）。
//! 環境變數 `STILLFEATHER_SHELLWINDOWS=off` 可以關掉（偵錯用）。
//!
//! 每一步與其他程式呼叫了哪些方法都寫進記錄資料夾的 `shell-windows.log`（見 [`log`]）：選取沒有作用時看這個檔案。

use super::shellreg;
use super::sys::*;
use super::util::wide;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 送到主視窗：選取 `lparam`（`Box<Select>`）
pub const WM_SHELL_SELECT: u32 = WM_APP + 14;
/// 送到主視窗：登記的隱藏視窗被啟用了，把主視窗拉到前面
pub const WM_SHELL_FRONT: u32 = WM_APP + 15;
/// 送到登記執行緒：作用中的資料夾變了
const WM_SYNC: u32 = WM_APP + 1;
/// 送到登記執行緒：檔案總管重新啟動了（清單跟著重建，要重新登記）
const WM_RESET: u32 = WM_APP + 2;

/// 要選取的項目（交給介面執行緒）
pub struct Select {
    pub path: String,
    /// 先取消其他的選取（SVSI_DESELECTOTHERS）
    pub replace: bool,
}

const CLSID_SHELL_WINDOWS: GUID = GUID::from_u128(0x9BA05972_F6A8_11CF_A442_00A0C90A8F39);
const IID_ISHELL_WINDOWS: GUID = GUID::from_u128(0x85CB6900_4D95_11CF_960C_0080C7F4EE85);
const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
const IID_IDISPATCH: GUID = GUID::from_u128(0x00020400_0000_0000_C000_000000000046);
const IID_ISERVICE_PROVIDER: GUID = GUID::from_u128(0x6D5140C1_7436_11CE_8034_00AA006009FA);
const IID_IOLE_WINDOW: GUID = GUID::from_u128(0x00000114_0000_0000_C000_000000000046);
const IID_ISHELL_VIEW: GUID = GUID::from_u128(0x000214E3_0000_0000_C000_000000000046);
const IID_IWEB_BROWSER: GUID = GUID::from_u128(0xEAB22AC1_30C1_11CF_A7EB_0000C05BAE0B);
const IID_IWEB_BROWSER_APP: GUID = GUID::from_u128(0x0002DF05_0000_0000_C000_000000000046);

/// 記錄裡用名稱顯示的介面與服務（其餘顯示 GUID）
const KNOWN_GUIDS: &[(u128, &str)] = &[
    (0x00000000_0000_0000_C000_000000000046, "IUnknown"),
    (0x00020400_0000_0000_C000_000000000046, "IDispatch"),
    (0x6D5140C1_7436_11CE_8034_00AA006009FA, "IServiceProvider"),
    (0x00000114_0000_0000_C000_000000000046, "IOleWindow"),
    (0x000214E2_0000_0000_C000_000000000046, "IShellBrowser/SID_SShellBrowser"),
    (0x000214E3_0000_0000_C000_000000000046, "IShellView"),
    (0xCDE725B0_CCC9_4519_917E_325D72FAB4CE, "IFolderView/SID_SFolderView"),
    (0x1AF3A467_214F_4298_908E_06B03E0B39F9, "IFolderView2"),
    (0xE7A1AF80_4D96_11CF_960C_0080C7F4EE85, "IShellFolderViewDual"),
    (0xEAB22AC1_30C1_11CF_A7EB_0000C05BAE0B, "IWebBrowser"),
    (0x0002DF05_0000_0000_C000_000000000046, "IWebBrowserApp/SID_SWebBrowserApp"),
    (0xD30C1661_CDAF_11D0_8A3E_00C04FC9E26E, "IWebBrowser2"),
    (0x4C96BE40_915C_11CF_99D3_00AA004AE837, "SID_STopLevelBrowser"),
    (0x00000003_0000_0000_C000_000000000046, "IMarshal"),
    (0x00000018_0000_0000_C000_000000000046, "IStdMarshalInfo"),
    (0x00000019_0000_0000_C000_000000000046, "IExternalConnection"),
    (0x0000001B_0000_0000_C000_000000000046, "IIdentityUnmarshal"),
    (0x94EA2B94_E9CC_49E0_C0FF_EE64CA8F5B90, "IAgileObject"),
    (0xECC8691B_C1DB_4DC0_855E_65F6C551AF49, "INoMarshal"),
    (0x1079ACFC_29BD_11D3_8E0D_00C04F6837D5, "IPersistIDList"),
    (0x1AC3D9F0_175C_11D1_95BE_00609797EA4F, "IPersistFolder2"),
    (0xB196B283_BAB4_101A_B69C_00AA00341D07, "IProvideClassInfo"),
    (0xB196B284_BAB4_101A_B69C_00AA00341D07, "IConnectionPointContainer"),
];

const SWC_BROWSER: i32 = 1;
const SWFO_NEEDDISPATCH: i32 = 0x1;
const SWFO_INCLUDEPENDING: i32 = 0x2;
const DWMWA_CLOAK: u32 = 13;
const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;
const CLSCTX_ALL: u32 = 0x17;
const VT_EMPTY: u16 = 0;
const VT_UI1: u16 = 17;
const VT_ARRAY: u16 = 0x2000;
const PM_NOREMOVE: u32 = 0;
const WM_QUIT: u32 = 0x0012;
const VARIANT_TRUE: i16 = -1;
const VARIANT_FALSE: i16 = 0;

const S_OK: HRESULT = 0;
const E_NOTIMPL: HRESULT = 0x8000_4001_u32 as HRESULT;
const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
const E_POINTER: HRESULT = 0x8000_4003_u32 as HRESULT;
const E_FAIL: HRESULT = 0x8000_4005_u32 as HRESULT;
const E_OUTOFMEMORY: HRESULT = 0x8007_000E_u32 as HRESULT;
const DISP_E_MEMBERNOTFOUND: HRESULT = 0x8002_0003_u32 as HRESULT;
const DISP_E_UNKNOWNNAME: HRESULT = 0x8002_0006_u32 as HRESULT;

/// IShellView::SelectItem 的旗標
const SVSI_SELECT: u32 = 0x1;
const SVSI_DESELECTOTHERS: u32 = 0x4;
const SVSI_ENSUREVISIBLE: u32 = 0x8;
const SVSI_FOCUSED: u32 = 0x10;

// ---------------------------------------------------------------- 記錄

/// 寫一行到 `shell-windows.log`（記錄資料夾：開發版在 exe 旁；安裝版在 %LOCALAPPDATA%\Stillfeather\logs）。
/// 超過 128 KB 時從頭寫。任何執行緒都可以呼叫；寫不進去就算了。
pub fn log(msg: &str) {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    let Some(path) = PATH.get_or_init(log_path) else { return };
    let mut st: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let line = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{}] {}\r\n",
        st.wYear,
        st.wMonth,
        st.wDay,
        st.wHour,
        st.wMinute,
        st.wSecond,
        st.wMilliseconds,
        unsafe { GetCurrentThreadId() },
        msg
    );
    let restart = std::fs::metadata(path).is_ok_and(|m| m.len() > 128 * 1024);
    let mut options = std::fs::OpenOptions::new();
    if restart {
        options.write(true).create(true).truncate(true);
    } else {
        options.append(true).create(true);
    }
    if let Ok(mut f) = options.open(path) {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

fn log_path() -> Option<String> {
    super::crashlog::log_dir().map(|d| feather_core::paths::join(&d, "shell-windows.log"))
}

fn hr_text(hr: HRESULT) -> String {
    format!("0x{:08X}", hr as u32)
}

fn guid_value(g: &GUID) -> u128 {
    ((g.data1 as u128) << 96) | ((g.data2 as u128) << 80) | ((g.data3 as u128) << 64) | u64::from_be_bytes(g.data4) as u128
}

fn guid_text(g: *const GUID) -> String {
    if g.is_null() {
        return "(null)".into();
    }
    let v = guid_value(unsafe { &*g });
    if let Some((_, name)) = KNOWN_GUIDS.iter().find(|(k, _)| *k == v) {
        return (*name).to_string();
    }
    format!("{{{:08X}-{:04X}-{:04X}-{:04X}-{:012X}}}", (v >> 96) as u32, (v >> 80) as u16, (v >> 64) as u16, (v >> 48) as u16, v & 0xFFFF_FFFF_FFFF)
}

thread_local! {
    /// 每種呼叫記錄過幾次（列舉殼層視窗的工具可能一直呼叫同一個方法，每種最多記 5 次）
    static LOGGED: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
}

/// 記錄其他程式的呼叫，每種最多 5 次
fn log_call(what: &str) {
    let first = LOGGED.with(|l| {
        let mut l = l.borrow_mut();
        let n = l.entry(what.to_string()).or_insert(0);
        *n += 1;
        *n <= 5
    });
    if first {
        log(&format!("呼叫：{what}"));
    }
}

// ---------------------------------------------------------------- 介面執行緒這一邊

/// 想登記的位置（None：不登記）。介面執行緒寫，登記執行緒讀。
static DESIRED: Mutex<Option<String>> = Mutex::new(None);
/// 登記執行緒（0：沒有啟動）
static THREAD: AtomicU32 = AtomicU32::new(0);
static DONE: Mutex<Option<Receiver<()>>> = Mutex::new(None);

/// 啟動登記執行緒（主視窗建立後呼叫一次；選擇器不呼叫）
pub fn start(main: HWND) {
    if std::env::var("STILLFEATHER_SHELLWINDOWS").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
        log("STILLFEATHER_SHELLWINDOWS=off：不登記");
        return;
    }
    let (ready_tx, ready_rx) = channel();
    let (done_tx, done_rx) = channel();
    let spawned = std::thread::Builder::new().name("shell-windows".into()).spawn(move || {
        thread_main(main, ready_tx);
        let _ = done_tx.send(());
    });
    if spawned.is_err() {
        log("無法建立登記執行緒");
        return;
    }
    taskbar_created_msg();
    if let Ok(id) = ready_rx.recv_timeout(Duration::from_secs(2)) {
        THREAD.store(id, Ordering::Relaxed);
        if let Ok(mut d) = DONE.lock() {
            *d = Some(done_rx);
        }
        post_sync();
    } else {
        log("登記執行緒沒有回應");
    }
}

/// 作用中的資料夾。None（首頁、搜尋結果等不是單一資料夾的畫面）時保留上一個資料夾的登記：
/// 換來換去時不必一直取消又重新登記；要選取的檔案不在作用中的資料夾時，介面執行緒照一般的開啟要求處理。沒變時不做事。
pub fn set_location(path: Option<&str>) {
    let Some(path) = path else { return };
    let Ok(mut d) = DESIRED.lock() else { return };
    if d.as_deref() == Some(path) {
        return;
    }
    *d = Some(path.to_string());
    drop(d);
    post_sync();
}

/// 預設檔案總管換了（只有 Stillfeather 是預設檔案總管時才登記）：重新確認要不要登記
pub fn refresh() {
    post_sync();
}

/// 主視窗收到 `TaskbarCreated`（檔案總管重新啟動）時呼叫：舊的殼層視窗清單跟著不見了，重新登記
pub fn on_taskbar_created() {
    post(WM_RESET);
}

/// `TaskbarCreated` 的訊息編號（檔案總管重新啟動時廣播給所有頂層視窗；0：還沒取得）
pub fn taskbar_created_msg() -> u32 {
    static MSG: AtomicU32 = AtomicU32::new(0);
    let m = MSG.load(Ordering::Relaxed);
    if m != 0 {
        return m;
    }
    let m = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    MSG.store(m, Ordering::Relaxed);
    m
}

fn post_sync() {
    post(WM_SYNC);
}

fn post(msg: u32) {
    let id = THREAD.load(Ordering::Relaxed);
    if id != 0 {
        unsafe {
            PostThreadMessageW(id, msg, 0, 0);
        }
    }
}

/// 主視窗關閉時：取消登記（最多等 0.5 秒，檔案總管沒有回應時不等它）
pub fn stop() {
    let id = THREAD.swap(0, Ordering::Relaxed);
    if id == 0 {
        return;
    }
    unsafe {
        PostThreadMessageW(id, WM_QUIT, 0, 0);
    }
    if let Some(done) = DONE.lock().ok().and_then(|mut d| d.take()) {
        let _ = done.recv_timeout(Duration::from_millis(500));
    }
}

// ---------------------------------------------------------------- 登記執行緒

#[repr(C)]
struct ShellWindowsVtbl {
    _unknown: [usize; 3],
    _dispatch: [usize; 4],
    _get_count: usize,
    _item: usize,
    _new_enum: usize,
    register: unsafe extern "system" fn(*mut c_void, *mut c_void, i32, i32, *mut i32) -> HRESULT,
    register_pending: unsafe extern "system" fn(*mut c_void, i32, *const PROPVARIANT, *const PROPVARIANT, i32, *mut i32) -> HRESULT,
    revoke: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    on_navigate: unsafe extern "system" fn(*mut c_void, i32, *const PROPVARIANT) -> HRESULT,
    _on_activated: usize,
    find_window_sw: unsafe extern "system" fn(*mut c_void, *const PROPVARIANT, *const PROPVARIANT, i32, *mut i32, i32, *mut *mut c_void) -> HRESULT,
    _on_created: usize,
    _process_attach_detach: usize,
}

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// 資料夾識別碼（PIDL）的取得方式。同一個路徑兩種方式得到的識別碼可能不同（例如使用者資料夾下的「下載」），
/// Windows 比對時把它們當成不同的資料夾，所以兩種不同時各登記一個視窗。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    /// `SHParseDisplayName`（PowerShell、多數程式）
    Parse,
    /// 桌面資料夾的 `IShellFolder::ParseDisplayName`、路徑結尾加 `\`（Chrome、Edge、Electron 的「在資料夾中顯示」）
    Desktop,
}

impl Form {
    fn label(self) -> &'static str {
        match self {
            Form::Parse => "",
            Form::Desktop => "〔Chrome 格式〕",
        }
    }
}

struct Registration {
    form: Form,
    /// 殼層視窗清單（在 explorer.exe 裡；null：沒能取得）
    windows: *mut c_void,
    browser: *mut Browser,
    /// 登記的視窗：這個執行緒的隱藏視窗（沒能建立時用主視窗）
    window: HWND,
    cookie: Option<i32>,
    location: Option<String>,
    /// 記過「不是預設檔案總管」了（狀態改變時才再記）
    logged_not_default: bool,
    /// 上次比較兩種識別碼的路徑與結果（Chrome 格式的那一份用：相同時不必另外登記）
    compared: Option<(String, bool)>,
}

impl Registration {
    unsafe fn new(form: Form, main: HWND, window: HWND) -> Registration {
        Registration { form, windows: std::ptr::null_mut(), browser: Browser::create(main, window), window, cookie: None, location: None, logged_not_default: false, compared: None }
    }

    /// 取消登記並釋放（隱藏視窗由呼叫端關閉）
    unsafe fn close(&mut self) {
        self.sync(None);
        let unknown = &**(self.browser as *const *const UnknownVtbl);
        (unknown.release)(self.browser as *mut c_void);
        if !self.windows.is_null() {
            let unknown = &**(self.windows as *const *const UnknownVtbl);
            (unknown.release)(self.windows);
            self.windows = std::ptr::null_mut();
        }
    }

    unsafe fn vtbl(&self) -> &'static ShellWindowsVtbl {
        &**(self.windows as *const *const ShellWindowsVtbl)
    }

    /// 放掉手上的殼層視窗清單與登記（檔案總管重新啟動後，舊的已經不能用，也不必取消登記）
    unsafe fn disconnect(&mut self) {
        if !self.windows.is_null() {
            let unknown = &**(self.windows as *const *const UnknownVtbl);
            (unknown.release)(self.windows);
            self.windows = std::ptr::null_mut();
        }
        self.cookie = None;
        self.set_location(None);
    }

    /// 重新取得殼層視窗清單
    unsafe fn reconnect(&mut self) {
        self.disconnect();
        let mut windows: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(&CLSID_SHELL_WINDOWS, std::ptr::null_mut(), CLSCTX_ALL, &IID_ISHELL_WINDOWS, &mut windows);
        if hr >= 0 && !windows.is_null() {
            self.windows = windows;
            log("取得殼層視窗清單");
        } else {
            log(&format!("無法取得殼層視窗清單：{}", hr_text(hr)));
        }
    }

    /// 照目前想要的位置登記；登記途中位置又變了（訊息可能在 COM 呼叫的等待中被丟掉）就再做，最多三次
    unsafe fn sync_wanted(&mut self) {
        for _ in 0..3 {
            let want = self.wanted();
            if want == self.location {
                return;
            }
            self.sync(want);
        }
    }

    /// 要登記的位置：只有 Stillfeather 是預設檔案總管時才登記（不是的話「在資料夾中顯示」照常用檔案總管）
    fn wanted(&mut self) -> Option<String> {
        let desired = DESIRED.lock().ok().and_then(|d| d.clone())?;
        if shellreg::default_fm_program().is_some() {
            self.logged_not_default = false;
            return self.needed(desired);
        }
        if !self.logged_not_default {
            self.logged_not_default = true;
            log("Stillfeather 不是預設檔案總管：不登記");
        }
        None
    }

    /// Chrome 格式的那一份只在兩種識別碼不同時登記（相同時另一份已經涵蓋）
    fn needed(&mut self, path: String) -> Option<String> {
        if self.form == Form::Parse {
            return Some(path);
        }
        let differ = match &self.compared {
            Some((p, d)) if *p == path => *d,
            _ => {
                let d = unsafe { forms_differ(&path) };
                log(&format!("{path} 的兩種識別碼{}", if d { "不同：Chrome 格式另外登記一個視窗" } else { "相同" }));
                self.compared = Some((path.clone(), d));
                d
            }
        };
        differ.then_some(path)
    }

    /// 改成登記 `want`（None：取消登記）
    unsafe fn sync(&mut self, want: Option<String>) {
        if want == self.location {
            return;
        }
        if self.windows.is_null() {
            if want.is_none() {
                return;
            }
            self.reconnect();
            if self.windows.is_null() {
                return;
            }
        }
        let vt = self.vtbl();
        match (self.cookie, want) {
            (Some(c), None) => {
                let hr = (vt.revoke)(self.windows, c);
                log(&format!("{}取消登記 {c}：{}", self.form.label(), hr_text(hr)));
                self.cookie = None;
                self.set_location(None);
            }
            (Some(c), Some(p)) => {
                let hr = with_location(&p, self.form, |v| (vt.on_navigate)(self.windows, c, v));
                log(&format!("{}OnNavigate {c} {p}：{}", self.form.label(), hr_text(hr)));
                if hr >= 0 {
                    self.self_test(&p);
                    self.set_location(Some(p));
                } else {
                    (vt.revoke)(self.windows, c);
                    self.cookie = None;
                    self.set_location(None);
                    self.register(p);
                }
            }
            (None, Some(p)) => self.register(p),
            (None, None) => {}
        }
    }

    /// 登記（失敗時多半是檔案總管重新啟動過，手上的清單已經不能用：重新取得清單再試一次）
    unsafe fn register(&mut self, path: String) {
        let mut ok = self.try_register(&path);
        if !ok {
            self.reconnect();
            ok = !self.windows.is_null() && self.try_register(&path);
        }
        if ok {
            self.self_test(&path);
            self.set_location(Some(path));
        }
    }

    /// 和 Windows 自己找視窗的方式相同（`SWC_BROWSER`、`SWFO_INCLUDEPENDING | SWFO_NEEDDISPATCH`）找一次，結果寫進記錄
    unsafe fn self_test(&self, path: &str) {
        let vt = self.vtbl();
        let empty = PROPVARIANT { vt: VT_EMPTY, reserved: [0; 3], data: [0; 2] };
        let mut found = 0i32;
        let mut disp: *mut c_void = std::ptr::null_mut();
        let hr = with_location(path, self.form, |v| (vt.find_window_sw)(self.windows, v, &empty, SWC_BROWSER, &mut found, SWFO_INCLUDEPENDING | SWFO_NEEDDISPATCH, &mut disp));
        let ours = !disp.is_null() && disp == self.browser as *mut c_void;
        if !disp.is_null() {
            let unknown = &**(disp as *const *const UnknownVtbl);
            (unknown.release)(disp);
        }
        let what = if found == 0 {
            "沒找到"
        } else if found == self.window as i32 {
            "找到登記的視窗"
        } else {
            "找到別的視窗"
        };
        log(&format!("{}自我檢查 FindWindowSW {path}：{}，{what}（0x{found:X}），物件{}", self.form.label(), hr_text(hr), if ours { "是 Stillfeather 的" } else if disp.is_null() { "沒有" } else { "不是 Stillfeather 的" }));
    }

    /// 和 Files 的順序相同：RegisterPending（位置）→ Register（視窗與物件）→ OnNavigate（位置）
    unsafe fn try_register(&mut self, path: &str) -> bool {
        let vt = self.vtbl();
        let empty = PROPVARIANT { vt: VT_EMPTY, reserved: [0; 3], data: [0; 2] };
        // 視窗所屬的執行緒：Register 以它找到這筆準備中的登記（隱藏視窗就是這個執行緒）
        let thread = GetWindowThreadProcessId(self.window, std::ptr::null_mut());
        let mut pending = 0i32;
        let hr = with_location(path, self.form, |v| (vt.register_pending)(self.windows, thread as i32, v, &empty, SWC_BROWSER, &mut pending));
        log(&format!("{}RegisterPending {path}（執行緒 {thread}）：{}，編號 {pending}", self.form.label(), hr_text(hr)));
        if hr < 0 {
            return false;
        }
        let mut cookie = 0i32;
        let disp = self.browser as *mut c_void;
        let hr = (vt.register)(self.windows, disp, self.window as i32, SWC_BROWSER, &mut cookie);
        log(&format!("Register（視窗 0x{:X}）：{}，編號 {cookie}", self.window, hr_text(hr)));
        if hr < 0 {
            (vt.revoke)(self.windows, pending);
            return false;
        }
        let hr = with_location(path, self.form, |v| (vt.on_navigate)(self.windows, cookie, v));
        log(&format!("OnNavigate {cookie}：{}", hr_text(hr)));
        let paired = pending == cookie;
        if !paired {
            // Register 沒有接上準備中的那一筆：位置只靠 OnNavigate；準備中的那一筆取消
            log("RegisterPending 與 Register 的編號不同（沒有配對）");
            (vt.revoke)(self.windows, pending);
            if hr < 0 {
                (vt.revoke)(self.windows, cookie);
                return false;
            }
        }
        self.cookie = Some(cookie);
        true
    }

    fn set_location(&mut self, path: Option<String>) {
        // 選取時把子項目的識別碼換成路徑用
        unsafe {
            *(*self.browser).folder.borrow_mut() = path.clone();
        }
        self.location = path;
    }
}

/// PIDL 的大小（含結尾的兩個 0）
unsafe fn pidl_size(pidl: *const c_void) -> usize {
    let p = pidl as *const u8;
    let mut off = 0usize;
    loop {
        let cb = std::ptr::read_unaligned(p.add(off) as *const u16) as usize;
        if cb == 0 || off + cb > 0x10000 {
            break;
        }
        off += cb;
    }
    off + 2
}

#[repr(C)]
struct ShellFolderParseVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    parse_display_name: unsafe extern "system" fn(*mut c_void, HWND, *mut c_void, *mut u16, *mut u32, *mut *mut c_void, *mut u32) -> HRESULT,
}

/// 以 `form` 的方式取得 `path` 的識別碼（CoTaskMemFree 釋放）
unsafe fn make_pidl(path: &str, form: Form) -> Result<*mut c_void, HRESULT> {
    let mut pidl: *mut c_void = std::ptr::null_mut();
    let hr = match form {
        Form::Parse => {
            let w = wide(path);
            SHParseDisplayName(w.as_ptr(), std::ptr::null_mut(), &mut pidl, 0, std::ptr::null_mut())
        }
        Form::Desktop => {
            let mut desktop: *mut c_void = std::ptr::null_mut();
            let hr = SHGetDesktopFolder(&mut desktop);
            if hr < 0 || desktop.is_null() {
                return Err(if hr < 0 { hr } else { E_FAIL });
            }
            // 和 Chrome 相同：資料夾路徑結尾加上 \（磁碟根目錄本來就有）
            let mut text = path.to_string();
            if !text.ends_with('\\') {
                text.push('\\');
            }
            let mut w = wide(&text);
            let vt = &**(desktop as *const *const ShellFolderParseVtbl);
            let hr = (vt.parse_display_name)(desktop, 0, std::ptr::null_mut(), w.as_mut_ptr(), std::ptr::null_mut(), &mut pidl, std::ptr::null_mut());
            (vt.release)(desktop);
            hr
        }
    };
    if hr < 0 || pidl.is_null() {
        return Err(if hr < 0 { hr } else { E_FAIL });
    }
    Ok(pidl)
}

/// 兩種方式得到的識別碼是不是不同（位元組比較）
unsafe fn forms_differ(path: &str) -> bool {
    let (Ok(a), Ok(b)) = (make_pidl(path, Form::Parse), make_pidl(path, Form::Desktop)) else { return false };
    let (na, nb) = (pidl_size(a), pidl_size(b));
    let differ = na != nb || std::slice::from_raw_parts(a as *const u8, na) != std::slice::from_raw_parts(b as *const u8, nb);
    CoTaskMemFree(a);
    CoTaskMemFree(b);
    differ
}

/// 以資料夾的 PIDL 位元組陣列（`VT_ARRAY | VT_UI1`，同 `InitVariantFromBuffer`，Windows 自己用的格式）呼叫 `f`
unsafe fn with_location(path: &str, form: Form, f: impl FnOnce(*const PROPVARIANT) -> HRESULT) -> HRESULT {
    let pidl = match make_pidl(path, form) {
        Ok(p) => p,
        Err(hr) => {
            log(&format!("{}無法取得 {path} 的識別碼：{}", form.label(), hr_text(hr)));
            return hr;
        }
    };
    let size = pidl_size(pidl);
    let psa = SafeArrayCreateVector(VT_UI1, 0, size as u32);
    if psa.is_null() {
        CoTaskMemFree(pidl);
        return E_OUTOFMEMORY;
    }
    let mut data: *mut c_void = std::ptr::null_mut();
    if SafeArrayAccessData(psa, &mut data) < 0 || data.is_null() {
        SafeArrayDestroy(psa);
        CoTaskMemFree(pidl);
        return E_FAIL;
    }
    std::ptr::copy_nonoverlapping(pidl as *const u8, data as *mut u8, size);
    SafeArrayUnaccessData(psa);
    CoTaskMemFree(pidl);
    let v = PROPVARIANT { vt: VT_ARRAY | VT_UI1, reserved: [0; 3], data: [psa as u64, 0] };
    let hr = f(&v);
    SafeArrayDestroy(psa);
    hr
}

fn thread_main(main: HWND, ready: std::sync::mpsc::Sender<u32>) {
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // 先建好訊息佇列，介面執行緒才能 PostThreadMessage
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, 0, WM_USER, WM_USER, PM_NOREMOVE);
        if ready.send(GetCurrentThreadId()).is_err() {
            CoUninitialize();
            return;
        }
        log(&format!("登記執行緒開始（系統管理員權限：{}）", if IsUserAnAdmin() != 0 { "是" } else { "否" }));
        MAIN.store(main, Ordering::Relaxed);
        // 兩種識別碼各一個隱藏視窗（相同時只用第一個）
        let mut regs = Vec::new();
        let mut helpers = Vec::new();
        for form in [Form::Parse, Form::Desktop] {
            let helper = create_helper();
            if helper != 0 {
                helpers.push(helper);
                regs.push(Registration::new(form, main, helper));
            } else if form == Form::Parse {
                log("無法建立隱藏視窗：改登記主視窗");
                regs.push(Registration::new(form, main, main));
            }
        }
        // 殼層視窗清單等到真的要登記時才取得（sync 會自己連上）：不是預設檔案總管時就不必跨行程向檔案總管要
        for reg in regs.iter_mut() {
            reg.sync_wanted();
        }
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            if msg.hwnd == 0 && msg.message == WM_SYNC {
                for reg in regs.iter_mut() {
                    reg.sync_wanted();
                }
                continue;
            }
            if msg.hwnd == 0 && msg.message == WM_RESET {
                // 舊的清單已經跟著檔案總管不見了：不必取消登記，直接重新取得再登記
                log("檔案總管重新啟動：重新登記");
                for reg in regs.iter_mut() {
                    reg.disconnect();
                    reg.sync_wanted();
                }
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for reg in regs.iter_mut() {
            reg.close();
        }
        for helper in helpers {
            DestroyWindow(helper);
        }
        log("登記執行緒結束");
        CoUninitialize();
    }
}

// ---------------------------------------------------------------- 啟動時的交接

thread_local! {
    /// 交接期間收到的選取（主執行緒）
    static HANDSHAKE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

const HANDSHAKE_TIMER: usize = 0x5357;

/// 啟動時的交接（見模組說明）：在呼叫的執行緒上登記隱藏視窗，等候期間處理訊息。結束（drop）時取消登記。
pub struct Handshake {
    regs: Vec<Registration>,
    helpers: Vec<HWND>,
}

impl Handshake {
    /// 這次啟動要不要交接：其他程式（不是檔案總管）以本機磁碟上的單一資料夾啟動、Stillfeather 是預設檔案總管。
    ///
    /// 這時還沒把位置交給已經開著的 Stillfeather，所以碰那個位置（`is_dir`）放在最後：先做不碰磁碟的檢查
    /// （由檔案總管啟動，例如在桌面雙擊，就不交接），網路位置與光碟機也不交接（只看路徑與磁碟機類型，見 [`local_drive`]）。
    pub fn wanted(paths: &[String]) -> Option<String> {
        let [path] = paths else { return None };
        if std::env::var("STILLFEATHER_SHELLWINDOWS").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
            return None;
        }
        if launched_by_explorer() || shellreg::default_fm_program().is_none() {
            return None;
        }
        // 網路位置（UNC、對應的網路磁碟機）離線時，連檢查是不是資料夾都要等到 SMB 逾時；這段時間還沒把位置交給已經開著的 Stillfeather，
        // 使用者什麼都看不到，所以不交接，照常交給主視窗在背景讀取（見 fs::on_local_drive）
        if !super::fs::on_local_drive(path) {
            log(&format!("啟動時的交接：{path} 不在本機硬碟或隨身碟上，不交接（直接交給主視窗在背景讀取）"));
            return None;
        }
        super::fs::is_dir(path).then(|| path.clone())
    }

    /// 登記 `folder`（在開始處理訊息之前呼叫）
    pub fn begin(folder: &str) -> Option<Handshake> {
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let parent = parent_process().map(|(pid, name)| format!("{name}（{pid}）")).unwrap_or_else(|| "?".into());
            log(&format!("啟動時的交接：{folder}（由 {parent} 啟動）"));
            let mut h = Handshake { regs: Vec::new(), helpers: Vec::new() };
            for form in [Form::Parse, Form::Desktop] {
                let helper = create_helper();
                if helper == 0 {
                    continue;
                }
                h.helpers.push(helper);
                let mut reg = Registration::new(form, 0, helper);
                reg.reconnect();
                if let Some(path) = reg.needed(folder.to_string()) {
                    if !reg.windows.is_null() {
                        reg.register(path);
                    }
                }
                h.regs.push(reg);
            }
            // 一個都沒登記上：drop 會清理
            h.regs.iter().any(|r| r.cookie.is_some()).then_some(h)
        }
    }

    /// 處理訊息，最多 `timeout`，收到選取就（再等 0.1 秒收齊後）結束。回傳收到的完整路徑。
    pub fn wait(&mut self, timeout: Duration) -> Vec<String> {
        let Some(&timer_window) = self.helpers.first() else { return Vec::new() };
        unsafe {
            SetTimer(timer_window, HANDSHAKE_TIMER, timeout.as_millis().min(60_000) as u32, std::ptr::null());
            let mut msg: MSG = std::mem::zeroed();
            let mut finishing = false;
            loop {
                if GetMessageW(&mut msg, 0, 0, 0) <= 0 {
                    // WM_QUIT：放回去給之後的訊息迴圈
                    PostQuitMessage(msg.wParam as i32);
                    break;
                }
                if msg.message == WM_TIMER && msg.hwnd == timer_window && msg.wParam == HANDSHAKE_TIMER {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if !finishing && HANDSHAKE.with(|h| !h.borrow().is_empty()) {
                    finishing = true;
                    SetTimer(timer_window, HANDSHAKE_TIMER, 100, std::ptr::null());
                }
            }
            KillTimer(timer_window, HANDSHAKE_TIMER);
            let got = HANDSHAKE.with(|h| std::mem::take(&mut *h.borrow_mut()));
            log(&format!("啟動時的交接：{}", if got.is_empty() { "沒有收到選取".to_string() } else { format!("收到 {}", got.join("、")) }));
            got
        }
    }
}

impl Drop for Handshake {
    fn drop(&mut self) {
        unsafe {
            for reg in self.regs.iter_mut() {
                reg.close();
            }
            for &helper in &self.helpers {
                DestroyWindow(helper);
            }
            CoUninitialize();
        }
    }
}

/// 這個程序是不是由檔案總管啟動的（使用者雙擊資料夾等；同 Files 的 IsLaunchedByExplorer）
fn launched_by_explorer() -> bool {
    let mut explorer = 0u32;
    unsafe {
        GetWindowThreadProcessId(GetShellWindow(), &mut explorer);
    }
    explorer != 0 && parent_process().is_some_and(|(pid, _)| pid == explorer)
}

/// 啟動這個程序的程序（編號與執行檔名稱）
fn parent_process() -> Option<(u32, String)> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
            return None;
        }
        let me = GetCurrentProcessId();
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        // 先找到自己的父程序編號，再找它的名稱
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut parent = None;
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let n = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                names.insert(entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..n]));
                if entry.th32ProcessID == me {
                    parent = Some(entry.th32ParentProcessID);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        let pid = parent?;
        Some((pid, names.remove(&pid).unwrap_or_else(|| "（已結束）".into())))
    }
}

/// 呼叫進來的程序（每個程序記一次；同一個程序裡的呼叫不記）
unsafe fn log_caller() {
    let mut tid = 0u32;
    if CoGetCallerTID(&mut tid) != S_OK || tid == 0 {
        return;
    }
    let mut pid = 0u32;
    let thread = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid);
    if thread != 0 {
        pid = GetProcessIdOfThread(thread);
        CloseHandle(thread);
    }
    let mut name = String::from("?");
    if pid != 0 {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process != 0 {
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            if QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) != 0 {
                let full = String::from_utf16_lossy(&buf[..size as usize]);
                name = full.rsplit('\\').next().unwrap_or(&full).to_string();
            }
            CloseHandle(process);
        }
    }
    log_call(&format!("呼叫者：{name}（程序 {pid}）"));
}

/// 主視窗（隱藏視窗被啟用時通知它）
static MAIN: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 登記用的隱藏視窗（同 Files：工具視窗、不會被啟用、隱形，從不顯示；沒有父視窗，屬於這個執行緒）
unsafe fn create_helper() -> HWND {
    let class = wide("StillfeatherShellWindow");
    let instance = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: helper_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: 0,
        hCursor: 0,
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: 0,
    };
    // 已經登記過（主視窗關掉又開、同一個程序）也沒關係
    RegisterClassExW(&wc);
    let title = wide("Stillfeather");
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class.as_ptr(), title.as_ptr(), 0, 0, 0, 0, 0, 0, 0, instance, std::ptr::null());
    if hwnd != 0 {
        let cloak: BOOL = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const BOOL as *const c_void, std::mem::size_of::<BOOL>() as u32);
    }
    hwnd
}

unsafe extern "system" fn helper_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // 被啟用（Windows 把找到的視窗拉到前面）：改成主視窗。這時這個程序是前景，主視窗拉得上來。
    if msg == WM_ACTIVATE && (wparam & 0xFFFF) != 0 {
        log("隱藏視窗被啟用：把主視窗拉到前面");
        let main = MAIN.load(Ordering::Relaxed);
        if main != 0 {
            PostMessageW(main, WM_SHELL_FRONT, 0, 0);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------- 登記的物件

/// 一個物件提供所有介面；每個介面各有一個虛擬函式表指標，方法收到的 `this` 指向對應的那一個欄位。
/// 第一個（`IWebBrowserApp`）同時是 `IUnknown`、`IDispatch`、`IWebBrowser`（它們是 `IWebBrowserApp` 的前半）。
/// 介面組合和 Files 相同（沒有 `IShellBrowser`、`IFolderView`）。
#[repr(C)]
struct Browser {
    wb: *const WebBrowserAppVtbl,
    sp: *const ServiceProviderVtbl,
    sv: *const ShellViewVtbl,
    refs: AtomicU32,
    /// 主視窗（選取交給它；0：啟動時的交接，選取存起來）
    main: HWND,
    /// 登記的視窗（沒有主視窗時，回答視窗的方法用它）
    window: HWND,
    /// 目前登記的資料夾（選取時把子項目的識別碼換成路徑用）
    folder: RefCell<Option<String>>,
}

const OFF_WB: usize = 0;
const OFF_SP: usize = 8;
const OFF_SV: usize = 16;

impl Browser {
    fn create(main: HWND, window: HWND) -> *mut Browser {
        Box::into_raw(Box::new(Browser { wb: &WEB_BROWSER_APP_VTBL, sp: &SERVICE_PROVIDER_VTBL, sv: &SHELL_VIEW_VTBL, refs: AtomicU32::new(1), main, window, folder: RefCell::new(None) }))
    }

    /// 回答「視窗是哪一個」的方法用：主視窗，沒有時用登記的隱藏視窗
    unsafe fn hwnd(b: *mut Browser) -> HWND {
        if (*b).main != 0 {
            (*b).main
        } else {
            (*b).window
        }
    }
}

/// 某個介面的 `this` 換回物件
unsafe fn base(this: *mut c_void, offset: usize) -> *mut Browser {
    (this as *mut u8).sub(offset) as *mut Browser
}

unsafe fn interface(b: *mut Browser, offset: usize) -> *mut c_void {
    (b as *mut u8).add(offset) as *mut c_void
}

fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe fn query(b: *mut Browser, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    log_caller();
    if out.is_null() {
        return E_POINTER;
    }
    *out = std::ptr::null_mut();
    if riid.is_null() {
        return E_POINTER;
    }
    let iid = &*riid;
    let offset = if [IID_IUNKNOWN, IID_IDISPATCH, IID_IWEB_BROWSER, IID_IWEB_BROWSER_APP].iter().any(|g| same(iid, g)) {
        OFF_WB
    } else if same(iid, &IID_ISERVICE_PROVIDER) {
        OFF_SP
    } else if same(iid, &IID_ISHELL_VIEW) || same(iid, &IID_IOLE_WINDOW) {
        OFF_SV
    } else {
        log_call(&format!("QueryInterface {}：沒有", guid_text(riid)));
        return E_NOINTERFACE;
    };
    if !same(iid, &IID_IUNKNOWN) {
        log_call(&format!("QueryInterface {}", guid_text(riid)));
    }
    (*b).refs.fetch_add(1, Ordering::Relaxed);
    *out = interface(b, offset);
    S_OK
}

unsafe fn add_ref(b: *mut Browser) -> u32 {
    (*b).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe fn release(b: *mut Browser) -> u32 {
    let left = (*b).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if left == 0 {
        drop(Box::from_raw(b));
    }
    left
}

/// 每個介面自己的 QueryInterface／AddRef／Release（換回物件再處理）
macro_rules! unknown_methods {
    ($qi:ident, $ar:ident, $rel:ident, $off:expr) => {
        unsafe extern "system" fn $qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
            query(base(this, $off), riid, out)
        }
        unsafe extern "system" fn $ar(this: *mut c_void) -> u32 {
            add_ref(base(this, $off))
        }
        unsafe extern "system" fn $rel(this: *mut c_void) -> u32 {
            release(base(this, $off))
        }
    };
}

unknown_methods!(wb_qi, wb_add_ref, wb_release, OFF_WB);
unknown_methods!(sp_qi, sp_add_ref, sp_release, OFF_SP);
unknown_methods!(sv_qi, sv_add_ref, sv_release, OFF_SV);

type QueryFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;
/// 沒有輸出參數的方法。x64 的呼叫慣例由呼叫端清理參數，所以宣告的參數比實際少也沒關係。
type Plain = unsafe extern "system" fn(*mut c_void) -> HRESULT;
type OutBool = unsafe extern "system" fn(*mut c_void, *mut i16) -> HRESULT;
type OutLong = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
type OutBstr = unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HRESULT;
type OutDisp = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;

/// 沒有輸出參數、照記錄回答固定結果的方法（記下名稱，看得出其他程式呼叫了哪些）
macro_rules! plain_methods {
    ($($name:ident = $label:literal => $hr:expr;)*) => {
        $(unsafe extern "system" fn $name(_this: *mut c_void) -> HRESULT {
            log_call($label);
            $hr
        })*
    };
}

unsafe fn put_bool(out: *mut i16, v: bool, label: &str) -> HRESULT {
    log_call(label);
    if out.is_null() {
        return E_POINTER;
    }
    *out = if v { VARIANT_TRUE } else { VARIANT_FALSE };
    S_OK
}

unsafe fn put_long(out: *mut i32, v: i32, label: &str) -> HRESULT {
    log_call(label);
    if out.is_null() {
        return E_POINTER;
    }
    *out = v;
    S_OK
}

unsafe fn put_bstr(out: *mut *mut u16, text: &str, label: &str) -> HRESULT {
    log_call(label);
    if out.is_null() {
        return E_POINTER;
    }
    let w = wide(text);
    *out = SysAllocString(w.as_ptr());
    if (*out).is_null() {
        E_OUTOFMEMORY
    } else {
        S_OK
    }
}

// ---- IDispatch／IWebBrowser／IWebBrowserApp（和 Files 的回答相同；沒有以名稱呼叫的成員）

/// IUnknown 3 + IDispatch 4 + IWebBrowser 25 + IWebBrowserApp 20 = 52 格
#[repr(C)]
struct WebBrowserAppVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    // IDispatch
    get_type_info_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_type_info: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> HRESULT,
    get_ids_of_names: unsafe extern "system" fn(*mut c_void, *const GUID, *const *const u16, u32) -> HRESULT,
    invoke: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    // IWebBrowser
    go_back: Plain,
    go_forward: Plain,
    go_home: Plain,
    go_search: Plain,
    navigate: Plain,
    refresh: Plain,
    refresh2: Plain,
    stop: Plain,
    get_application: OutDisp,
    get_parent: OutDisp,
    get_container: OutDisp,
    get_document: OutDisp,
    get_top_level_container: OutBool,
    get_type: OutBstr,
    get_left: OutLong,
    put_left: Plain,
    get_top: OutLong,
    put_top: Plain,
    get_width: OutLong,
    put_width: Plain,
    get_height: OutLong,
    put_height: Plain,
    get_location_name: OutBstr,
    get_location_url: OutBstr,
    get_busy: OutBool,
    // IWebBrowserApp
    quit: Plain,
    client_to_window: unsafe extern "system" fn(*mut c_void, *mut i32, *mut i32) -> HRESULT,
    put_property: Plain,
    get_property: unsafe extern "system" fn(*mut c_void, *mut u16, *mut PROPVARIANT) -> HRESULT,
    get_name: OutBstr,
    get_hwnd: unsafe extern "system" fn(*mut c_void, *mut isize) -> HRESULT,
    get_full_name: OutBstr,
    get_path: OutBstr,
    get_visible: OutBool,
    put_visible: Plain,
    get_status_bar: OutBool,
    put_status_bar: Plain,
    get_status_text: OutBstr,
    put_status_text: Plain,
    get_tool_bar: OutLong,
    put_tool_bar: Plain,
    get_menu_bar: OutBool,
    put_menu_bar: Plain,
    get_full_screen: OutBool,
    put_full_screen: Plain,
}

unsafe extern "system" fn get_type_info_count(_this: *mut c_void, count: *mut u32) -> HRESULT {
    log_call("IDispatch::GetTypeInfoCount");
    if count.is_null() {
        return E_POINTER;
    }
    *count = 0;
    S_OK
}

unsafe extern "system" fn get_type_info(_this: *mut c_void, _index: u32, _lcid: u32, out: *mut *mut c_void) -> HRESULT {
    log_call("IDispatch::GetTypeInfo");
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    E_NOTIMPL
}

unsafe extern "system" fn get_ids_of_names(_this: *mut c_void, _riid: *const GUID, names: *const *const u16, count: u32) -> HRESULT {
    let name = if names.is_null() || count == 0 || (*names).is_null() {
        String::new()
    } else {
        let p = *names;
        let mut n = 0;
        while n < 256 && *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    };
    log_call(&format!("IDispatch::GetIDsOfNames {name}"));
    DISP_E_UNKNOWNNAME
}

unsafe extern "system" fn invoke(_this: *mut c_void, dispid: i32) -> HRESULT {
    log_call(&format!("IDispatch::Invoke {dispid}"));
    DISP_E_MEMBERNOTFOUND
}

plain_methods! {
    wb_go_back = "IWebBrowser::GoBack" => E_NOTIMPL;
    wb_go_forward = "IWebBrowser::GoForward" => E_NOTIMPL;
    wb_go_home = "IWebBrowser::GoHome" => E_NOTIMPL;
    wb_go_search = "IWebBrowser::GoSearch" => E_NOTIMPL;
    wb_navigate = "IWebBrowser::Navigate" => E_NOTIMPL;
    wb_refresh = "IWebBrowser::Refresh" => S_OK;
    wb_refresh2 = "IWebBrowser::Refresh2" => S_OK;
    wb_stop = "IWebBrowser::Stop" => S_OK;
    wb_put_left = "IWebBrowser::put_Left" => E_NOTIMPL;
    wb_put_top = "IWebBrowser::put_Top" => E_NOTIMPL;
    wb_put_width = "IWebBrowser::put_Width" => E_NOTIMPL;
    wb_put_height = "IWebBrowser::put_Height" => E_NOTIMPL;
    // 不關閉主視窗（列舉殼層視窗、關掉所有檔案總管視窗的工具會呼叫）
    wb_quit = "IWebBrowserApp::Quit" => E_NOTIMPL;
    wb_put_property = "IWebBrowserApp::PutProperty" => E_NOTIMPL;
    wb_put_visible = "IWebBrowserApp::put_Visible" => E_NOTIMPL;
    wb_put_status_bar = "IWebBrowserApp::put_StatusBar" => E_NOTIMPL;
    wb_put_status_text = "IWebBrowserApp::put_StatusText" => E_NOTIMPL;
    wb_put_tool_bar = "IWebBrowserApp::put_ToolBar" => E_NOTIMPL;
    wb_put_menu_bar = "IWebBrowserApp::put_MenuBar" => E_NOTIMPL;
    wb_put_full_screen = "IWebBrowserApp::put_FullScreen" => E_NOTIMPL;
}

/// Application、Parent、Container、Document 都是這個物件本身（同 Files）
macro_rules! self_getters {
    ($($name:ident = $label:literal;)*) => {
        $(unsafe extern "system" fn $name(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
            log_call($label);
            query(base(this, OFF_WB), &IID_IDISPATCH, out)
        })*
    };
}

self_getters! {
    wb_get_application = "IWebBrowser::get_Application";
    wb_get_parent = "IWebBrowser::get_Parent";
    wb_get_container = "IWebBrowser::get_Container";
    wb_get_document = "IWebBrowser::get_Document";
}

unsafe extern "system" fn wb_get_top_level_container(_this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, true, "IWebBrowser::get_TopLevelContainer")
}

unsafe extern "system" fn wb_get_type(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, "Stillfeather", "IWebBrowser::get_Type")
}

unsafe extern "system" fn wb_get_left(_this: *mut c_void, out: *mut i32) -> HRESULT {
    put_long(out, 0, "IWebBrowser::get_Left")
}

unsafe extern "system" fn wb_get_top(_this: *mut c_void, out: *mut i32) -> HRESULT {
    put_long(out, 0, "IWebBrowser::get_Top")
}

unsafe extern "system" fn wb_get_width(_this: *mut c_void, out: *mut i32) -> HRESULT {
    put_long(out, 0, "IWebBrowser::get_Width")
}

unsafe extern "system" fn wb_get_height(_this: *mut c_void, out: *mut i32) -> HRESULT {
    put_long(out, 0, "IWebBrowser::get_Height")
}

unsafe extern "system" fn wb_get_location_name(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, "", "IWebBrowser::get_LocationName")
}

unsafe extern "system" fn wb_get_location_url(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, "", "IWebBrowser::get_LocationURL")
}

unsafe extern "system" fn wb_get_busy(_this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, false, "IWebBrowser::get_Busy")
}

unsafe extern "system" fn wb_client_to_window(_this: *mut c_void, width: *mut i32, height: *mut i32) -> HRESULT {
    log_call("IWebBrowserApp::ClientToWindow");
    if width.is_null() || height.is_null() {
        E_POINTER
    } else {
        S_OK
    }
}

unsafe extern "system" fn wb_get_property(_this: *mut c_void, _name: *mut u16, value: *mut PROPVARIANT) -> HRESULT {
    log_call("IWebBrowserApp::GetProperty");
    if value.is_null() {
        return E_POINTER;
    }
    *value = PROPVARIANT { vt: VT_EMPTY, reserved: [0; 3], data: [0; 2] };
    E_NOTIMPL
}

unsafe extern "system" fn wb_get_name(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, "Stillfeather", "IWebBrowserApp::get_Name")
}

unsafe extern "system" fn wb_get_hwnd(this: *mut c_void, out: *mut isize) -> HRESULT {
    log_call("IWebBrowserApp::get_HWND");
    if out.is_null() {
        return E_POINTER;
    }
    *out = Browser::hwnd(base(this, OFF_WB));
    S_OK
}

unsafe extern "system" fn wb_get_full_name(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, &super::util::current_exe_path().unwrap_or_default(), "IWebBrowserApp::get_FullName")
}

unsafe extern "system" fn wb_get_path(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, &super::util::current_exe_path().unwrap_or_default(), "IWebBrowserApp::get_Path")
}

unsafe extern "system" fn wb_get_visible(this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, IsWindowVisible(Browser::hwnd(base(this, OFF_WB))) != 0, "IWebBrowserApp::get_Visible")
}

unsafe extern "system" fn wb_get_status_bar(_this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, false, "IWebBrowserApp::get_StatusBar")
}

unsafe extern "system" fn wb_get_status_text(_this: *mut c_void, out: *mut *mut u16) -> HRESULT {
    put_bstr(out, "", "IWebBrowserApp::get_StatusText")
}

unsafe extern "system" fn wb_get_tool_bar(_this: *mut c_void, out: *mut i32) -> HRESULT {
    put_long(out, 0, "IWebBrowserApp::get_ToolBar")
}

unsafe extern "system" fn wb_get_menu_bar(_this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, false, "IWebBrowserApp::get_MenuBar")
}

unsafe extern "system" fn wb_get_full_screen(_this: *mut c_void, out: *mut i16) -> HRESULT {
    put_bool(out, false, "IWebBrowserApp::get_FullScreen")
}

static WEB_BROWSER_APP_VTBL: WebBrowserAppVtbl = WebBrowserAppVtbl {
    query_interface: wb_qi,
    add_ref: wb_add_ref,
    release: wb_release,
    get_type_info_count,
    get_type_info,
    get_ids_of_names,
    invoke,
    go_back: wb_go_back,
    go_forward: wb_go_forward,
    go_home: wb_go_home,
    go_search: wb_go_search,
    navigate: wb_navigate,
    refresh: wb_refresh,
    refresh2: wb_refresh2,
    stop: wb_stop,
    get_application: wb_get_application,
    get_parent: wb_get_parent,
    get_container: wb_get_container,
    get_document: wb_get_document,
    get_top_level_container: wb_get_top_level_container,
    get_type: wb_get_type,
    get_left: wb_get_left,
    put_left: wb_put_left,
    get_top: wb_get_top,
    put_top: wb_put_top,
    get_width: wb_get_width,
    put_width: wb_put_width,
    get_height: wb_get_height,
    put_height: wb_put_height,
    get_location_name: wb_get_location_name,
    get_location_url: wb_get_location_url,
    get_busy: wb_get_busy,
    quit: wb_quit,
    client_to_window: wb_client_to_window,
    put_property: wb_put_property,
    get_property: wb_get_property,
    get_name: wb_get_name,
    get_hwnd: wb_get_hwnd,
    get_full_name: wb_get_full_name,
    get_path: wb_get_path,
    get_visible: wb_get_visible,
    put_visible: wb_put_visible,
    get_status_bar: wb_get_status_bar,
    put_status_bar: wb_put_status_bar,
    get_status_text: wb_get_status_text,
    put_status_text: wb_put_status_text,
    get_tool_bar: wb_get_tool_bar,
    put_tool_bar: wb_put_tool_bar,
    get_menu_bar: wb_get_menu_bar,
    put_menu_bar: wb_put_menu_bar,
    get_full_screen: wb_get_full_screen,
    put_full_screen: wb_put_full_screen,
};

// ---- IServiceProvider：不管要哪一個服務，都照要的介面回答（同 Files）

#[repr(C)]
struct ServiceProviderVtbl {
    query_interface: QueryFn,
    add_ref: RefFn,
    release: RefFn,
    query_service: unsafe extern "system" fn(*mut c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT,
}

unsafe extern "system" fn query_service(this: *mut c_void, service: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let hr = query(base(this, OFF_SP), riid, out);
    log_call(&format!("QueryService {} → {}：{}", guid_text(service), guid_text(riid), hr_text(hr)));
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

unsafe extern "system" fn sv_get_window(this: *mut c_void, out: *mut HWND) -> HRESULT {
    log_call("IOleWindow::GetWindow");
    if out.is_null() {
        return E_POINTER;
    }
    *out = Browser::hwnd(base(this, OFF_SV));
    S_OK
}

plain_methods! {
    sv_context_sensitive_help = "IOleWindow::ContextSensitiveHelp" => E_NOTIMPL;
    sv_translate_accelerator = "IShellView::TranslateAccelerator" => E_NOTIMPL;
    sv_enable_modeless = "IShellView::EnableModeless" => S_OK;
    sv_ui_activate = "IShellView::UIActivate" => S_OK;
    sv_refresh = "IShellView::Refresh" => S_OK;
    sv_destroy_view_window = "IShellView::DestroyViewWindow" => S_OK;
    sv_add_property_sheet_pages = "IShellView::AddPropertySheetPages" => E_NOTIMPL;
    sv_save_view_state = "IShellView::SaveViewState" => S_OK;
}

unsafe extern "system" fn sv_create_view_window(_this: *mut c_void, _prev: *mut c_void, _fs: *const c_void, _browser: *mut c_void, _rect: *const c_void, out: *mut HWND) -> HRESULT {
    log_call("IShellView::CreateViewWindow");
    if !out.is_null() {
        *out = 0;
    }
    E_NOTIMPL
}

unsafe extern "system" fn sv_get_current_info(_this: *mut c_void, out: *mut [u32; 2]) -> HRESULT {
    log_call("IShellView::GetCurrentInfo");
    if out.is_null() {
        return E_POINTER;
    }
    *out = [0, 0];
    S_OK
}

unsafe extern "system" fn sv_select_item(this: *mut c_void, child: *const c_void, flags: u32) -> HRESULT {
    select(base(this, OFF_SV), child, flags)
}

unsafe extern "system" fn sv_get_item_object(_this: *mut c_void, _item: u32, _riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    log_call("IShellView::GetItemObject");
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    E_NOINTERFACE
}

static SHELL_VIEW_VTBL: ShellViewVtbl = ShellViewVtbl {
    query_interface: sv_qi,
    add_ref: sv_add_ref,
    release: sv_release,
    get_window: sv_get_window,
    context_sensitive_help: sv_context_sensitive_help,
    translate_accelerator: sv_translate_accelerator,
    enable_modeless: sv_enable_modeless,
    ui_activate: sv_ui_activate,
    refresh: sv_refresh,
    create_view_window: sv_create_view_window,
    destroy_view_window: sv_destroy_view_window,
    get_current_info: sv_get_current_info,
    add_property_sheet_pages: sv_add_property_sheet_pages,
    save_view_state: sv_save_view_state,
    select_item: sv_select_item,
    get_item_object: sv_get_item_object,
};

// ---- 選取

/// 選取登記的資料夾裡的 `child`（相對於資料夾的項目識別碼）：換成完整路徑交給介面執行緒
unsafe fn select(b: *mut Browser, child: *const c_void, flags: u32) -> HRESULT {
    // 只是取消選取（例如先清掉其他的選取）：不必做事（同 Files）
    if child.is_null() || flags & (SVSI_SELECT | SVSI_FOCUSED | SVSI_ENSUREVISIBLE) == 0 {
        log(&format!("IShellView::SelectItem 旗標 0x{flags:X}：略過"));
        return S_OK;
    }
    let Some(folder) = (*b).folder.borrow().clone() else {
        log("IShellView::SelectItem：沒有登記的資料夾");
        return E_FAIL;
    };
    let Some(path) = item_path(&folder, child) else {
        log(&format!("IShellView::SelectItem：無法取得 {folder} 裡項目的路徑"));
        return E_FAIL;
    };
    log(&format!("IShellView::SelectItem {path}（旗標 0x{flags:X}）"));
    if (*b).main == 0 {
        // 啟動時的交接：存起來，等候的迴圈看到就結束
        HANDSHAKE.with(|h| h.borrow_mut().push(path));
        return S_OK;
    }
    let msg = Box::into_raw(Box::new(Select { path, replace: flags & SVSI_DESELECTOTHERS != 0 }));
    if PostMessageW((*b).main, WM_SHELL_SELECT, 0, msg as LPARAM) == 0 {
        drop(Box::from_raw(msg));
        return E_FAIL;
    }
    S_OK
}

/// 子項目識別碼 → 完整路徑。照文件是相對於資料夾的子項目；也接受完整的識別碼（有些呼叫端直接傳完整的）。
pub(super) unsafe fn item_path(folder: &str, child: *const c_void) -> Option<String> {
    let combined = combined_path(folder, child);
    if combined.as_deref().is_some_and(|p| std::path::Path::new(p).exists()) {
        return combined;
    }
    match pidl_path(child) {
        Some(p) if std::path::Path::new(&p).exists() => Some(p),
        _ => combined,
    }
}

unsafe fn combined_path(folder: &str, child: *const c_void) -> Option<String> {
    let w = wide(folder);
    let mut parent: *mut c_void = std::ptr::null_mut();
    if SHParseDisplayName(w.as_ptr(), std::ptr::null_mut(), &mut parent, 0, std::ptr::null_mut()) < 0 || parent.is_null() {
        return None;
    }
    let full = ILCombine(parent, child);
    CoTaskMemFree(parent);
    if full.is_null() {
        return None;
    }
    let path = pidl_path(full);
    CoTaskMemFree(full);
    path
}

unsafe fn pidl_path(pidl: *const c_void) -> Option<String> {
    let mut buf = vec![0u16; 32768];
    if SHGetPathFromIDListEx(pidl, buf.as_mut_ptr(), buf.len() as u32, 0) == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts() {
        // IUnknown 3 + IDispatch 4 + IShellWindows 11
        assert_eq!(std::mem::size_of::<ShellWindowsVtbl>(), 18 * 8);
        assert_eq!(std::mem::offset_of!(ShellWindowsVtbl, register), 10 * 8);
        assert_eq!(std::mem::offset_of!(ShellWindowsVtbl, register_pending), 11 * 8);
        assert_eq!(std::mem::offset_of!(ShellWindowsVtbl, revoke), 12 * 8);
        assert_eq!(std::mem::offset_of!(ShellWindowsVtbl, on_navigate), 13 * 8);
        assert_eq!(std::mem::offset_of!(ShellWindowsVtbl, find_window_sw), 15 * 8);
        // IWebBrowserApp：IUnknown 3 + IDispatch 4 + IWebBrowser 25 + IWebBrowserApp 20
        assert_eq!(std::mem::size_of::<WebBrowserAppVtbl>(), 52 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, go_back), 7 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, get_document), 18 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, get_busy), 31 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, quit), 32 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, get_hwnd), 37 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, get_visible), 40 * 8);
        assert_eq!(std::mem::offset_of!(WebBrowserAppVtbl, put_full_screen), 51 * 8);
        assert_eq!(std::mem::size_of::<ServiceProviderVtbl>(), 4 * 8);
        // IShellView：IUnknown 3 + IOleWindow 2 + 11；SelectItem 是第 14 個
        assert_eq!(std::mem::size_of::<ShellViewVtbl>(), 16 * 8);
        assert_eq!(std::mem::offset_of!(ShellViewVtbl, create_view_window), 9 * 8);
        assert_eq!(std::mem::offset_of!(ShellViewVtbl, select_item), 14 * 8);
        assert_eq!(std::mem::size_of::<PROCESSENTRY32W>(), 568);
        assert_eq!(std::mem::offset_of!(PROCESSENTRY32W, th32ParentProcessID), 32);
        // 物件裡每個介面指標的位置
        assert_eq!(std::mem::offset_of!(Browser, wb), OFF_WB);
        assert_eq!(std::mem::offset_of!(Browser, sp), OFF_SP);
        assert_eq!(std::mem::offset_of!(Browser, sv), OFF_SV);
    }

    #[test]
    fn pidl_sizes() {
        // 兩層：4 + 3 個位元組，再加結尾的兩個 0
        let pidl: [u8; 9] = [4, 0, 1, 2, 3, 0, 9, 0, 0];
        assert_eq!(unsafe { pidl_size(pidl.as_ptr() as *const c_void) }, 9);
        let empty: [u8; 2] = [0, 0];
        assert_eq!(unsafe { pidl_size(empty.as_ptr() as *const c_void) }, 2);
    }

    #[test]
    fn guid_names() {
        assert_eq!(guid_text(&IID_ISHELL_VIEW), "IShellView");
        assert_eq!(guid_text(&GUID::from_u128(0x12345678_9ABC_DEF0_1122_334455667788)), "{12345678-9ABC-DEF0-1122-334455667788}");
        assert_eq!(guid_text(std::ptr::null()), "(null)");
    }

    // 物件的虛擬函式表會連到 Win32 的函式：只在 Windows 上連結得起來（test.ps1 會跑）
    #[cfg(windows)]
    #[test]
    fn interfaces_share_one_identity() {
        unsafe {
            let b = Browser::create(0, 0);
            let mut sp: *mut c_void = std::ptr::null_mut();
            assert_eq!(query(b, &IID_ISERVICE_PROVIDER, &mut sp), S_OK);
            // 任何服務都照要的介面回答
            let mut sv: *mut c_void = std::ptr::null_mut();
            assert_eq!(query_service(sp, &CLSID_SHELL_WINDOWS, &IID_ISHELL_VIEW, &mut sv), S_OK);
            let mut unk: *mut c_void = std::ptr::null_mut();
            assert_eq!(sv_qi(sv, &IID_IUNKNOWN, &mut unk), S_OK);
            assert_eq!(unk, b as *mut c_void);
            let mut none: *mut c_void = 1 as *mut c_void;
            assert_eq!(query(b, &CLSID_SHELL_WINDOWS, &mut none), E_NOINTERFACE);
            assert!(none.is_null());
            // IWebBrowserApp → get_Document → IShellView
            let mut wb: *mut c_void = std::ptr::null_mut();
            assert_eq!(query(b, &IID_IWEB_BROWSER_APP, &mut wb), S_OK);
            assert_eq!(wb, b as *mut c_void);
            let mut doc: *mut c_void = std::ptr::null_mut();
            assert_eq!(wb_get_document(wb, &mut doc), S_OK);
            let mut sv2: *mut c_void = std::ptr::null_mut();
            assert_eq!(wb_qi(doc, &IID_ISHELL_VIEW, &mut sv2), S_OK);
            assert_eq!(sv2, sv);
            // 1（建立）+ 6（sp、sv、unk、wb、doc、sv2）
            assert_eq!((*b).refs.load(Ordering::Relaxed), 7);
            for p in [sv2, doc, wb, unk, sv, sp] {
                let vt = &**(p as *const *const UnknownVtbl);
                (vt.release)(p);
            }
            assert_eq!(release(b), 0);
        }
    }
}

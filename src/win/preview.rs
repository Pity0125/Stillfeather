//! 快速預覽：圖片、文字、程式碼（語法上色）、PDF 第 1 頁、影片縮圖，以及 Word、Excel、PowerPoint 與 OpenDocument 文件。
//!
//! - 懸停（[`Mode::Hover`]）：滑鼠停在清單的項目上就在游標旁顯示小視窗，內容載好才出現；
//!   視窗不接收滑鼠（滑鼠穿透到清單），移到別的項目就跟著換，移開、點擊、捲動或按鍵就關閉。
//! - 按住空白鍵（[`Mode::Hold`]）：預覽選取的項目（懸停中則放大懸停的項目），置中顯示，放開就關閉；
//!   按住時的自動重複會被忽略，用方向鍵換選取，預覽會跟著換。
//! - 右鍵選單（[`Mode::Pinned`]）：置中並維持開著，直到按空白鍵、Esc 或 ×。
//! - 不搶走鍵盤焦點（清單仍可操作）；置中時文字與程式碼可用滾輪捲動。
//! - 圖片、影片與 PDF 在短命的子程序解碼（本程式以 `--preview-worker` 模式執行），
//!   透過 Windows Shell 的縮圖介面取得影像，並讀出原始尺寸與影片長度；程式本身不常駐解碼器。
//! - PDF 先試 Windows 的縮圖介面（裝了 PDF 縮圖處理程式時很快）；沒有時由系統內建的 Windows PowerShell 5.1 + Windows.Data.Pdf 輸出第 1 頁 PNG。
//! - 文字讀取前 1 MiB，在背景執行緒解碼（UTF-8／UTF-16／系統字碼頁）並上色。
//! - Office 文件在背景執行緒直接讀 ZIP 裡的 XML（[`office`]），不需要安裝 Office：文件與簡報排成段落、標題、
//!   清單與表格，試算表顯示成有欄名與列號的格線；簡報另外請 Windows 產生第 1 張投影片的縮圖當封面（有縮圖處理程式時）。
//! - 雲端尚未下載或離線的檔案不讀取（避免觸發下載）；子程序超過 8 秒會被終止。

use super::anim;
use super::crashlog::{crumb, mark};
use super::app::{self, WorkerMsg};
use super::fs::{self, FsError};
use super::sys::*;
use super::theme::{self, Buffered};
use super::util::*;
use feather_core::lru::ByteLru;
use feather_core::office::{self, Block, Document, Format, Office, Table, Workbook};
use feather_core::paths;
use feather_core::quicklook::{self, Decoded, Doc, Kind, Lang, Tok};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const POPUP_CLASS: &str = "FeatherFiles.Preview";
/// 選取改變後稍等一下再載入（按住方向鍵時不會每一項都解碼）
pub const TIMER_ID: usize = 1;
const FOLLOW_DELAY_MS: u32 = 90;
/// 按住空白鍵預覽時，定期確認空白鍵還按著（放開的訊息被輸入法吃掉時也能關閉）
pub const HOLD_TIMER: usize = 6;
const HOLD_POLL_MS: u32 = 60;
/// 懸停預覽載入較久（PDF、影片）時，過這麼久才先顯示「正在載入」
pub const SHOW_TIMER: usize = 7;
const SHOW_DELAY_MS: u32 = 350;
/// 懸停中換到別的項目：上一個的畫面最多留這麼久，新內容還沒好就改顯示「正在載入」
const SWITCH_LOADING_MS: u32 = 140;
/// 縮圖最長邊的上限（像素）
const MAX_EDGE: i32 = 1280;
const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// PDF 的大小上限（PowerShell 轉檔時整個讀進記憶體；系統縮圖也套用同樣的上限）
const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
/// Office 文件的大小上限（整個讀進記憶體再解壓）
const MAX_OFFICE_BYTES: u64 = 64 * 1024 * 1024;
/// 簡報封面縮圖的最長邊
const COVER_EDGE: i32 = 960;
const TIMEOUT: Duration = Duration::from_secs(8);
const MAGIC: &[u8; 4] = b"FFP2";
const IID_IIMAGELIST: GUID = GUID::from_u128(0x46EB5926_582E_4017_9FDF_E8998DAA0950);

/// 要預覽的項目（由清單的選取產生）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    /// 標題下方的說明：類型 · 大小 · 修改日期
    pub meta: String,
}

/// 預覽的開法
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 滑鼠停在項目上：游標旁的小視窗，滑鼠穿透
    Hover,
    /// 按住空白鍵：置中，放開就關閉
    Hold,
    /// 右鍵選單：置中，維持開著
    Pinned,
}

/// 設定中的開關
#[derive(Clone, Copy, Debug)]
pub struct Opts {
    pub images: bool,
    pub pdf: bool,
    /// 「按空白鍵預覽」：懸停的預覽是按住空白鍵才出現的
    pub space: bool,
}

/// 背景載入的結果
#[derive(Clone)]
pub enum Loaded {
    Image(Image),
    Text { doc: Doc, caption: String },
    /// Office 文件；簡報可能附上第 1 張投影片的縮圖
    Office { office: Office, cover: Option<Image>, caption: String },
    Message(String),
}

#[derive(Clone)]
pub struct Image {
    pub width: i32,
    pub height: i32,
    /// BGRA，由上而下。
    pub pixels: Vec<u32>,
    pub caption: String,
}

enum Content {
    Loading,
    Image { bitmap: HBITMAP, w: i32, h: i32, caption: String },
    Text { doc: Doc, caption: String },
    /// Word、PowerPoint 與 OpenDocument 文字／簡報：段落、標題、清單與表格（排版結果依寬度快取）
    Document { doc: Document, cover: Option<Cover>, caption: String, layout: DocLayout },
    /// Excel 與 OpenDocument 試算表：儲存格格線（欄寬依內容、建立時的 DPI）
    Sheet { book: Workbook, caption: String, cols: Vec<i32>, gutter: i32, dpi: u32 },
    Message(String),
}

/// 簡報封面（第 1 張投影片的縮圖）
struct Cover {
    bitmap: HBITMAP,
    w: i32,
    h: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hot {
    None,
    Open,
    Close,
}

pub struct State {
    pub popup: HWND,
    target: Option<Target>,
    /// 選取改變後等待載入的項目
    pending: Option<Target>,
    opts: Opts,
    /// 置中的範圍（Stillfeather 視窗，螢幕座標）
    area: RECT,
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    content: Content,
    /// 資訊卡用的大圖示
    icon: HICON,
    /// 文字：第一個可見行；文件：捲動的像素；試算表：第一個可見列
    scroll: usize,
    /// 試算表：第一個可見欄
    hscroll: usize,
    /// 等寬字型與建立時的 DPI
    mono: (HFONT, u32),
    /// 文件預覽的字型
    doc_fonts: Option<DocFonts>,
    hot: Hot,
    tracking: bool,
    mode: Mode,
    /// 懸停時的游標位置與所在螢幕的工作區（螢幕座標）
    cursor: POINT,
    work: RECT,
}

impl State {
    pub fn new() -> State {
        State {
            popup: 0,
            target: None,
            pending: None,
            opts: Opts { images: true, pdf: true, space: false },
            area: RECT::default(),
            generation: 0,
            cancel: None,
            content: Content::Loading,
            icon: 0,
            scroll: 0,
            hscroll: 0,
            mono: (0, 0),
            doc_fonts: None,
            hot: Hot::None,
            tracking: false,
            mode: Mode::Pinned,
            cursor: POINT::default(),
            work: RECT::default(),
        }
    }
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(POPUP_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DROPSHADOW,
        lpfnWndProc: popup_proc,
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
    let class = wide(POPUP_CLASS);
    let name = wide(tr!("快速預覽", "Quick preview"));
    unsafe {
        // 不搶焦點：鍵盤仍留在清單，方向鍵換項目、空白鍵或 Esc 關閉
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED,
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
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        // 顯示時由我們自己淡入，關掉系統的視窗開關動畫以免重複；Windows 11 上用圓角。
        let off: BOOL = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const BOOL as *const std::ffi::c_void, 4);
        let round: u32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const u32 as *const std::ffi::c_void, 4);
        hwnd
    }
}

pub fn is_open(state: &State) -> bool {
    state.target.is_some()
}

/// 是否是「按住空白鍵」的預覽（放開就關閉）。
pub fn is_holding(state: &State) -> bool {
    is_open(state) && state.mode == Mode::Hold
}

/// 開著時是哪一種預覽。
pub fn mode(state: &State) -> Option<Mode> {
    is_open(state).then_some(state.mode)
}

/// 正在預覽的項目。
pub fn target(state: &State) -> Option<&Target> {
    state.target.as_ref()
}

/// 空白鍵實際上是否還按著。
pub fn space_held() -> bool {
    unsafe { GetAsyncKeyState(VK_SPACE as i32) < 0 }
}

/// 開啟（或換成另一個項目、換成另一種開法）。`area` 是 Stillfeather 視窗的螢幕範圍，置中時顯示在其中；
/// `cursor` 是懸停時的游標位置（螢幕座標），其他開法不使用。
pub fn open(state: &mut State, owner: HWND, target: Target, area: RECT, cursor: POINT, opts: Opts, mode: Mode) {
    crumb("預覽：開啟");
    let was_open = is_open(state);
    let visible = unsafe { IsWindowVisible(state.popup) } != 0;
    state.area = area;
    state.cursor = cursor;
    if mode == Mode::Hover {
        state.work = work_area(cursor);
    }
    state.opts = opts;
    state.pending = None;
    state.mode = mode;
    state.hot = Hot::None;
    unsafe {
        KillTimer(owner, TIMER_ID);
        KillTimer(owner, SHOW_TIMER);
        if mode == Mode::Hold {
            SetTimer(owner, HOLD_TIMER, HOLD_POLL_MS, std::ptr::null());
        } else {
            KillTimer(owner, HOLD_TIMER);
        }
    }
    set_click_through(state.popup, mode == Mode::Hover);
    // 同一個項目（例如懸停時按住空白鍵放大）：已載入或正在載入就不重新載入
    if !(was_open && state.target.as_ref() == Some(&target)) {
        start(state, target);
    }
    if mode == Mode::Hover && matches!(state.content, Content::Loading) {
        // 懸停：內容載好才出現，不閃「正在載入」；載入較久時才先顯示載入中。
        // 已經顯示著上一個項目（滑鼠換到下一個）時不先隱藏：隱藏再淡入每換一次就閃一下，用起來卡卡的。
        // 先留著上一個的畫面，新的內容載好就直接換上；載入比較久才換成這個項目的「正在載入」。
        let delay = if visible { SWITCH_LOADING_MS } else { SHOW_DELAY_MS };
        unsafe {
            SetTimer(owner, SHOW_TIMER, delay, std::ptr::null());
        }
        return;
    }
    reveal(state, visible);
}

/// 調整位置並顯示（原本沒顯示就淡入）。
fn reveal(state: &mut State, visible: bool) {
    mark("預覽：顯示");
    place(state);
    if !visible {
        anim::fade_in(state.popup, if state.mode == Mode::Hover { 110.0 } else { 140.0 });
    }
    unsafe {
        SetWindowPos(state.popup, 0, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// 懸停預覽載入較久：先顯示「正在載入」。
pub fn on_show_timer(state: &mut State, owner: HWND) {
    unsafe {
        KillTimer(owner, SHOW_TIMER);
    }
    if !is_open(state) {
        return;
    }
    if unsafe { IsWindowVisible(state.popup) } == 0 {
        reveal(state, false);
    } else if matches!(state.content, Content::Loading) {
        // 還留著上一個項目的畫面：換成這個項目的「正在載入」，並移到游標旁
        place(state);
    }
}

/// 懸停時滑鼠要穿透預覽視窗，才不會擋住清單（移到預覽底下的項目也能換）。
fn set_click_through(hwnd: HWND, on: bool) {
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let next = if on { ex | WS_EX_TRANSPARENT } else { ex & !WS_EX_TRANSPARENT };
        if next != ex {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next as isize);
        }
    }
}

/// 游標所在螢幕的工作區（不含工作列）。
fn work_area(pt: POINT) -> RECT {
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: RECT::default(), rcWork: RECT::default(), dwFlags: 0 };
        if GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi) != 0 {
            mi.rcWork
        } else {
            RECT { left: pt.x - 800, top: pt.y - 600, right: pt.x + 800, bottom: pt.y + 600 }
        }
    }
}

/// 預覽開著時清單的選取改變：稍等一下再換（連續按方向鍵時只載入最後一個）。
pub fn follow(state: &mut State, owner: HWND, target: Option<Target>) {
    // 懸停的預覽跟著滑鼠，不跟著選取
    if !is_open(state) || state.mode == Mode::Hover {
        return;
    }
    let Some(t) = target else { return };
    if state.target.as_ref() == Some(&t) && state.pending.is_none() {
        return;
    }
    state.pending = Some(t);
    unsafe {
        SetTimer(owner, TIMER_ID, FOLLOW_DELAY_MS, std::ptr::null());
    }
}

pub fn on_timer(state: &mut State, owner: HWND) {
    unsafe {
        KillTimer(owner, TIMER_ID);
    }
    if let Some(t) = state.pending.take() {
        if is_open(state) {
            start(state, t);
            invalidate(state.popup);
        }
    }
}

pub fn close(state: &mut State, owner: HWND) {
    if !is_open(state) && state.cancel.is_none() {
        return;
    }
    crumb("預覽：關閉");
    unsafe {
        KillTimer(owner, TIMER_ID);
        KillTimer(owner, HOLD_TIMER);
        KillTimer(owner, SHOW_TIMER);
    }
    if let Some(c) = state.cancel.take() {
        c.store(true, Ordering::Relaxed);
    }
    state.target = None;
    state.pending = None;
    state.generation += 1;
    set_content(state, Content::Loading);
    if state.popup != 0 {
        show(state.popup, false);
    }
}

/// Stillfeather 視窗移動或改變大小：預覽跟著置中。
pub fn reposition(state: &mut State, area: RECT) {
    if is_open(state) && state.mode != Mode::Hover {
        state.area = area;
        place(state);
    }
}

fn set_content(state: &mut State, content: Content) {
    let old = std::mem::replace(&mut state.content, content);
    if let Content::Image { bitmap, .. } | Content::Document { cover: Some(Cover { bitmap, .. }), .. } = old {
        if bitmap != 0 {
            unsafe {
                DeleteObject(bitmap);
            }
        }
    }
    if state.icon != 0 {
        unsafe {
            DestroyIcon(state.icon);
        }
        state.icon = 0;
    }
    state.scroll = 0;
    state.hscroll = 0;
}

/// 開始載入某個項目（取消前一個）。
fn start(state: &mut State, target: Target) {
    if let Some(c) = state.cancel.take() {
        c.store(true, Ordering::Relaxed);
    }
    state.generation += 1;
    let generation = state.generation;
    // 換項目時保留目前的視窗大小，內容載好再調整，避免閃動
    set_content(state, Content::Loading);
    state.target = Some(target.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    state.cancel = Some(cancel.clone());
    // 依置中時的大小產生影像，懸停後按住空白鍵放大時不必重新載入
    let (cw, ch) = center_box(state);
    let edge = cw.max(ch).min(MAX_EDGE);
    let opts = state.opts;
    mark("預覽：開始載入");
    std::thread::spawn(move || {
        mark("背景：載入開始");
        let result = load(&target, edge, opts, &cancel);
        mark("背景：載入結束");
        if !cancel.load(Ordering::Relaxed) {
            app::post(WorkerMsg::Preview { generation, path: target.path, result });
        }
    });
}

pub fn on_result(state: &mut State, generation: u64, path: &str, result: Result<Loaded, FsError>) {
    if generation != state.generation || state.target.as_ref().map(|t| t.path.as_str()) != Some(path) {
        return;
    }
    state.cancel = None;
    crumb("預覽：收到結果");
    let visible = unsafe { IsWindowVisible(state.popup) } != 0;
    let t = app::theme();
    let content = match result {
        Err(FsError::Cancelled) => return,
        Err(FsError::Message(m)) | Ok(Loaded::Message(m)) => Content::Message(m),
        Ok(Loaded::Image(img)) => Content::Image { bitmap: make_bitmap(&img, t.pal.surface), w: img.width, h: img.height, caption: img.caption },
        Ok(Loaded::Text { doc, caption }) => Content::Text { doc, caption },
        Ok(Loaded::Office { office: Office::Document(doc), cover, caption }) => Content::Document {
            doc,
            cover: cover.filter(|c| c.width > 0 && c.height > 0).map(|c| Cover { bitmap: make_bitmap(&c, t.pal.surface), w: c.width, h: c.height }),
            caption,
            layout: DocLayout::default(),
        },
        Ok(Loaded::Office { office: Office::Workbook(book), caption, .. }) => sheet_content(book, caption),
    };
    set_content(state, content);
    if matches!(state.content, Content::Message(_)) {
        mark("預覽：取得圖示");
        state.icon = file_icon(path);
    }
    if visible {
        place(state);
    } else {
        // 懸停的預覽在內容載好時才出現
        reveal(state, false);
    }
    mark("預覽：結果處理完成");
}

// ---------------------------------------------------------------- 版面

const HEADER: i32 = 60;
const FOOTER: i32 = 34;
const PAD: i32 = 14;

fn s(v: i32) -> i32 {
    app::theme().s(v)
}

/// 預覽視窗最大的寬高：懸停時是游標旁的小視窗（依螢幕大小），其他時候依 Stillfeather 視窗大小。
fn max_box(state: &State) -> (i32, i32) {
    if state.mode == Mode::Hover {
        let w = state.work;
        return ((w.width() * 40 / 100).clamp(s(320), s(560)), (w.height() * 55 / 100).clamp(s(240), s(500)));
    }
    center_box(state)
}

/// 置中時的最大寬高
fn center_box(state: &State) -> (i32, i32) {
    let a = state.area;
    let w = (a.width() * 72 / 100).clamp(s(440), s(1100)).min((a.width() - s(40)).max(s(320)));
    let h = (a.height() * 84 / 100).clamp(s(320), s(820)).min((a.height() - s(40)).max(s(240)));
    (w, h)
}

/// 依內容決定視窗大小：圖片貼合比例，文字用最大範圍，訊息用小卡片。
fn desired_size(state: &State) -> (i32, i32) {
    let (mw, mh) = max_box(state);
    match &state.content {
        Content::Image { w, h, .. } => {
            let bw = (mw - 2 * s(PAD)) as f32;
            let bh = (mh - s(HEADER) - s(FOOTER) - s(PAD)) as f32;
            let scale = (bw / *w as f32).min(bh / *h as f32).min(1.0);
            let iw = (*w as f32 * scale).round() as i32;
            let ih = (*h as f32 * scale).round() as i32;
            ((iw + 2 * s(PAD)).max(s(440)), ih + s(HEADER) + s(FOOTER) + s(PAD))
        }
        Content::Text { .. } | Content::Document { .. } => (mw, mh),
        // 試算表：小的表格用剛好的大小
        Content::Sheet { book, cols, gutter, .. } if !book.table.rows.is_empty() => {
            let rh = sheet_row_height();
            let w = 2 * s(PAD) + gutter + cols.iter().sum::<i32>() + s(4);
            let h = s(HEADER) + s(FOOTER) + s(SHEET_TOP) + rh * (book.table.rows.len() as i32 + 1) + s(10);
            (w.clamp(s(440).min(mw), mw), h.clamp(s(220).min(mh), mh))
        }
        Content::Sheet { .. } | Content::Message(_) => card_size(state, mw, mh),
        Content::Loading => {
            let r = window_rect(state.popup);
            if unsafe { IsWindowVisible(state.popup) } != 0 && r.width() > 0 {
                (r.width(), r.height())
            } else {
                card_size(state, mw, mh)
            }
        }
    }
}

/// 訊息與載入中的小卡片
fn card_size(state: &State, mw: i32, mh: i32) -> (i32, i32) {
    if state.mode == Mode::Hover {
        (s(400).min(mw), s(240).min(mh))
    } else {
        (s(480).min(mw), s(300).min(mh))
    }
}

fn place(state: &State) {
    let (w, h) = desired_size(state);
    let (x, y) = if state.mode == Mode::Hover {
        hover_origin(state, w, h)
    } else {
        let a = state.area;
        (a.left + (a.width() - w) / 2, a.top + (a.height() - h) / 2)
    };
    unsafe {
        SetWindowPos(state.popup, 0, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    invalidate(state.popup);
}

/// 懸停：放在游標右邊（放不下就左邊），垂直方向略高於游標，並留在螢幕工作區內。
fn hover_origin(state: &State, w: i32, h: i32) -> (i32, i32) {
    let c = state.cursor;
    let wk = state.work;
    let gap = s(24);
    let x = if c.x + gap + w <= wk.right {
        c.x + gap
    } else if c.x - gap - w >= wk.left {
        c.x - gap - w
    } else {
        (wk.right - w).max(wk.left)
    };
    let y = (c.y - h / 3).clamp(wk.top, (wk.bottom - h).max(wk.top));
    (x, y)
}

fn header_buttons(rect: &RECT) -> (RECT, RECT) {
    let b = s(32);
    let top = (s(HEADER) - b) / 2;
    let close = RECT::new(rect.right - s(PAD) - b, top, b, b);
    let open = RECT::new(close.left - s(8) - s(72), top, s(72), b);
    (open, close)
}

fn body_rect(rect: &RECT) -> RECT {
    RECT { left: rect.left + s(PAD), top: rect.top + s(HEADER), right: rect.right - s(PAD), bottom: rect.bottom - s(FOOTER) }
}

fn window_rect(hwnd: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut r);
    }
    r
}

// ---------------------------------------------------------------- 繪製

/// 程式碼的顏色：關鍵字用重點色，其餘是不含綠色的柔和色系；深淺色各一組。
fn tok_color(tok: Tok, pal: &theme::Palette) -> COLORREF {
    let dark = theme::is_dark_color(pal.surface);
    let rgb = |r: u32, g: u32, b: u32| r | (g << 8) | (b << 16);
    match (tok, dark) {
        (Tok::Keyword | Tok::Heading, _) => pal.accent,
        (Tok::Comment, _) => pal.muted,
        (Tok::Type | Tok::Tag, true) => rgb(0x8F, 0xB8, 0xE8),
        (Tok::Type | Tok::Tag, false) => rgb(0x2F, 0x5F, 0x9E),
        (Tok::Function | Tok::Attr, true) => rgb(0xB9, 0xA6, 0xE8),
        (Tok::Function | Tok::Attr, false) => rgb(0x6A, 0x4F, 0xB0),
        (Tok::String, true) => rgb(0xD9, 0xB6, 0x7C),
        (Tok::String, false) => rgb(0x9A, 0x6A, 0x12),
        (Tok::Number | Tok::Constant, true) => rgb(0xE3, 0xA0, 0xB5),
        (Tok::Number | Tok::Constant, false) => rgb(0xB0, 0x3A, 0x62),
        (Tok::Variable, true) => rgb(0x9C, 0xC9, 0xE0),
        (Tok::Variable, false) => rgb(0x1E, 0x6E, 0x8C),
    }
}

/// 等寬字型：優先 Cascadia Mono，沒有安裝時用 Consolas。
fn mono_font(state: &mut State) -> HFONT {
    mark("預覽：等寬字型");
    let dpi = app::theme().dpi;
    if state.mono.0 != 0 && state.mono.1 == dpi {
        return state.mono.0;
    }
    unsafe {
        if state.mono.0 != 0 {
            DeleteObject(state.mono.0);
        }
        let make = |face: &str| {
            let f = wide(face);
            CreateFontW(-MulDiv(10, dpi as i32, 72), 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, f.as_ptr())
        };
        let mut font = make("Cascadia Mono");
        let dc = GetDC(0);
        let old = SelectObject(dc, font);
        let mut name = [0u16; 64];
        let n = GetTextFaceW(dc, name.len() as i32, name.as_mut_ptr());
        SelectObject(dc, old);
        ReleaseDC(0, dc);
        if !String::from_utf16_lossy(&name[..n.max(0) as usize]).trim_end_matches('\0').eq_ignore_ascii_case("Cascadia Mono") {
            DeleteObject(font);
            font = make("Consolas");
        }
        state.mono = (font, dpi);
        font
    }
}

fn text_size(dc: HDC, text: &[u16]) -> SIZE {
    let mut sz = SIZE::default();
    if text.is_empty() {
        return sz;
    }
    unsafe {
        GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut sz);
    }
    sz
}

/// 文字內容的行高與一屏可見行數
fn text_metrics(state: &mut State) -> (i32, i32) {
    let font = mono_font(state);
    unsafe {
        let dc = GetDC(0);
        let old = SelectObject(dc, font);
        let sz = text_size(dc, &[b'0' as u16]);
        SelectObject(dc, old);
        ReleaseDC(0, dc);
        (sz.cy + s(3), sz.cx)
    }
}

fn paint_text(state: &mut State, dc: HDC, body: &RECT) {
    let font = mono_font(state);
    let (lh, cw) = text_metrics(state);
    let t = app::theme();
    let p = &t.pal;
    let Content::Text { doc, .. } = &state.content else { return };
    if doc.lines.is_empty() {
        theme::draw_text(dc, tr!("（空白檔案）", "(Empty file)"), body, t.fonts.ui, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        return;
    }
    let digits = doc.lines.len().to_string().len() as i32;
    let gutter = digits * cw + s(18);
    unsafe {
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT);
        SaveDC(dc);
        IntersectClipRect(dc, body.left, body.top, body.right, body.bottom);
        // 行號欄的底色
        theme::fill(dc, &RECT { left: body.left, top: body.top, right: body.left + gutter - s(8), bottom: body.bottom }, theme::blend(p.ink, p.surface, 0.035));
        let visible = (body.height() / lh + 1).max(1) as usize;
        for (row, line) in doc.lines.iter().enumerate().skip(state.scroll).take(visible) {
            let y = body.top + s(4) + (row - state.scroll) as i32 * lh;
            let num: Vec<u16> = (row + 1).to_string().encode_utf16().collect();
            let nw = text_size(dc, &num).cx;
            SetTextColor(dc, theme::blend(p.muted, p.surface, 0.75));
            TextOutW(dc, body.left + gutter - s(16) - nw, y, num.as_ptr(), num.len() as i32);
            // 片段之間的空隙是一般文字
            let mut x = body.left + gutter;
            let mut at = 0usize;
            let mut segments: Vec<(usize, usize, COLORREF)> = Vec::new();
            for sp in &line.spans {
                if sp.start > at {
                    segments.push((at, sp.start, p.ink));
                }
                segments.push((sp.start, sp.end, tok_color(sp.tok, p)));
                at = sp.end;
            }
            if at < line.text.len() {
                segments.push((at, line.text.len(), p.ink));
            }
            for (a, b, color) in segments {
                if x > body.right {
                    break;
                }
                if a >= b {
                    continue;
                }
                let w: Vec<u16> = line.text[a..b].encode_utf16().collect();
                SetTextColor(dc, color);
                TextOutW(dc, x, y, w.as_ptr(), w.len() as i32);
                x += text_size(dc, &w).cx;
            }
        }
        RestoreDC(dc, -1);
        SelectObject(dc, old);
    }
}

fn paint(state: &mut State, hwnd: HWND, dc: HDC) {
    mark("預覽：paint 開始");
    let rect = client_rect(hwnd);
    let t = app::theme();
    let p = &t.pal;
    let f = &t.fonts;
    theme::fill(dc, &rect, p.surface);
    theme::round_rect(dc, &rect, 0, None, Some(p.line));
    let Some(target) = state.target.clone() else { return };
    // 標題：名稱與說明
    let (open_r, close_r) = header_buttons(&rect);
    let buttons = state.mode != Mode::Hover;
    let title_right = if buttons { open_r.left - s(12) } else { rect.right - s(PAD) - s(4) };
    let title = RECT { left: rect.left + s(PAD) + s(4), top: s(10), right: title_right, bottom: s(10) + f.ui_height + s(4) };
    theme::draw_text(dc, &target.name, &title, f.ui_bold, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    let meta = RECT { top: title.bottom, bottom: s(HEADER) - s(6), ..title };
    theme::draw_text(dc, &target.meta, &meta, f.small, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    // 開啟、關閉按鈕（懸停時滑鼠穿透，不顯示）
    if buttons {
        let hot_fill = |on: bool| if on { p.hover } else { p.surface };
        theme::round_rect(dc, &open_r, open_r.height(), Some(hot_fill(state.hot == Hot::Open)), Some(p.line));
        theme::draw_text(dc, tr!("開啟", "Open"), &open_r, f.ui, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        theme::round_rect(dc, &close_r, close_r.height(), Some(hot_fill(state.hot == Hot::Close)), None);
        theme::draw_text(dc, &theme::glyph::CLOSE.to_string(), &close_r, f.icon_small, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let line = RECT { left: rect.left + s(PAD), top: s(HEADER) - 1, right: rect.right - s(PAD), bottom: s(HEADER) };
    theme::fill(dc, &line, theme::blend(p.ink, p.surface, 0.08));
    // 內容
    let body = body_rect(&rect);
    mark("預覽：paint 內容");
    let caption = match &state.content {
        Content::Loading => {
            theme::draw_text(dc, tr!("正在載入預覽…", "Loading preview…"), &body, f.ui, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            String::new()
        }
        Content::Message(m) => {
            mark("預覽：paint 訊息");
            let icon_size = s(48);
            let mut text_top = body.top + body.height() / 2 - s(10);
            if state.icon != 0 {
                let iy = body.top + body.height() / 2 - icon_size - s(14);
                unsafe {
                    DrawIconEx(dc, body.left + (body.width() - icon_size) / 2, iy, state.icon, icon_size, icon_size, 0, 0, DI_NORMAL);
                }
                text_top = iy + icon_size + s(14);
            }
            let tr = RECT { left: body.left + s(24), top: text_top, right: body.right - s(24), bottom: body.bottom };
            theme::draw_text(dc, m, &tr, f.ui, p.muted, DT_CENTER | DT_TOP | DT_WORDBREAK);
            String::new()
        }
        Content::Image { bitmap, w, h, caption } => {
            mark("預覽：paint 圖片");
            let ib = RECT { top: body.top + s(PAD) / 2, ..body };
            let scale = (ib.width() as f32 / *w as f32).min(ib.height() as f32 / *h as f32).min(1.0);
            let (dw, dh) = ((*w as f32 * scale).round() as i32, (*h as f32 * scale).round() as i32);
            let x = ib.left + (ib.width() - dw) / 2;
            let y = ib.top + (ib.height() - dh) / 2;
            blit(dc, *bitmap, *w, *h, x, y, dw, dh);
            caption.clone()
        }
        Content::Document { caption, .. } => {
            let caption = caption.clone();
            mark("預覽：paint 文件");
            paint_document(state, dc, &body);
            caption
        }
        Content::Sheet { caption, .. } => {
            let caption = caption.clone();
            mark("預覽：paint 試算表");
            paint_sheet(state, dc, &body);
            caption
        }
        Content::Text { caption, .. } => {
            let caption = caption.clone();
            mark("預覽：paint 文字");
            paint_text(state, dc, &body);
            mark("預覽：paint 文字完成");
            caption
        }
    };
    mark("預覽：paint 底部");
    // 底部：操作提示與內容說明
    let foot = RECT { left: rect.left + s(PAD) + s(4), top: rect.bottom - s(FOOTER), right: rect.right - s(PAD) - s(4), bottom: rect.bottom };
    let sep = RECT { left: rect.left + s(PAD), top: foot.top, right: rect.right - s(PAD), bottom: foot.top + 1 };
    theme::fill(dc, &sep, theme::blend(p.ink, p.surface, 0.08));
    let scroll_hint = match state.content {
        Content::Text { .. } | Content::Document { .. } => Some(tr!("滾輪捲動", "Scroll with the mouse wheel")),
        Content::Sheet { .. } => Some(tr!("滾輪捲動（按住 Shift 左右）", "Scroll with the mouse wheel (hold Shift for left/right)")),
        _ => None,
    };
    let hint = match (state.mode, scroll_hint) {
        (Mode::Hover, _) if state.opts.space => tr!("放開空白鍵關閉 · 按住時移到別的項目就換", "Release Space to close · move to another item while holding to switch").to_string(),
        (Mode::Hover, _) => tr!("移開滑鼠就關閉 · 按住空白鍵放大", "Move the mouse away to close · hold Space to enlarge").to_string(),
        (Mode::Hold, Some(h)) => trf!("放開空白鍵關閉 · 按住時 ↑↓ 換項目 · {h}", "Release Space to close · ↑↓ to switch items while holding · {h}"),
        (Mode::Hold, None) => tr!("放開空白鍵關閉 · 按住時 ↑↓ 換項目", "Release Space to close · ↑↓ to switch items while holding").to_string(),
        (Mode::Pinned, Some(h)) => trf!("空白鍵或 Esc 關閉 · ↑↓ 換項目 · {h} · Enter 開啟", "Space or Esc to close · ↑↓ to switch items · {h} · Enter to open"),
        (Mode::Pinned, None) => tr!("空白鍵或 Esc 關閉 · ↑↓ 換項目 · Enter 開啟", "Space or Esc to close · ↑↓ to switch items · Enter to open").to_string(),
    };
    // 說明靠右（最多占一半多一點），操作提示在左邊剩下的空間，兩者不重疊
    let cap_w = if caption.is_empty() { 0 } else { (theme::text_width(dc, &caption, f.small) + s(2)).min(foot.width() * 55 / 100) };
    let hint_r = RECT { right: foot.right - cap_w - if cap_w > 0 { s(16) } else { 0 }, ..foot };
    theme::draw_text(dc, &hint, &hint_r, f.small, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    if cap_w > 0 {
        let cap_r = RECT { left: foot.right - cap_w, ..foot };
        theme::draw_text(dc, &caption, &cap_r, f.small, p.muted, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }
    mark("預覽：paint 完成");
}

fn scroll_by(state: &mut State, hwnd: HWND, lines: i64) {
    let total = match &state.content {
        Content::Text { doc, .. } => doc.lines.len(),
        _ => return,
    };
    let body = body_rect(&client_rect(hwnd));
    let (lh, _) = text_metrics(state);
    let page = (body.height() / lh.max(1)).max(1) as usize;
    let max_top = total.saturating_sub(page.saturating_sub(1));
    let next = (state.scroll as i64 + lines).clamp(0, max_top as i64) as usize;
    if next != state.scroll {
        state.scroll = next;
        invalidate(hwnd);
    }
}

/// 滑鼠滾輪（由主視窗轉來，或直接送到預覽視窗）。試算表按住 Shift 時左右捲動。
pub fn wheel(state: &mut State, delta: i32) {
    if !is_open(state) {
        return;
    }
    let popup = state.popup;
    let lines = -(delta as i64) * 3 / 120;
    match state.content {
        Content::Sheet { .. } if unsafe { GetKeyState(VK_SHIFT) } < 0 => scroll_sheet(state, popup, 0, -notches(delta)),
        Content::Sheet { .. } => scroll_sheet(state, popup, lines, 0),
        // 文件依像素捲動（觸控板的細微捲動也有反應），一格約三行
        Content::Document { .. } => {
            let line = app::theme().fonts.ui_height + s(7);
            scroll_document(state, popup, -(delta as i64) * 3 * line as i64 / 120);
        }
        _ => scroll_by(state, popup, lines),
    }
}

/// 水平滾輪（觸控板左右滑動、滾輪左右撥）：試算表左右捲動
pub fn hwheel(state: &mut State, delta: i32) {
    if is_open(state) && matches!(state.content, Content::Sheet { .. }) {
        let popup = state.popup;
        scroll_sheet(state, popup, 0, notches(delta));
    }
}

/// 滾輪的格數（不足一格也算一格）
fn notches(delta: i32) -> i64 {
    match delta as i64 / 120 {
        0 => delta.signum() as i64,
        n => n,
    }
}

fn hit(state: &State, hwnd: HWND, x: i32, y: i32) -> Hot {
    let (open_r, close_r) = header_buttons(&client_rect(hwnd));
    let pt = POINT { x, y };
    let inside = |r: &RECT| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom;
    if state.target.is_none() || state.mode == Mode::Hover {
        Hot::None
    } else if inside(&close_r) {
        Hot::Close
    } else if inside(&open_r) {
        Hot::Open
    } else {
        Hot::None
    }
}

unsafe extern "system" fn popup_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(a) = app::app_opt() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    if msg == WM_PAINT {
        mark("預覽視窗：WM_PAINT");
    }
    match msg {
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => 3, // MA_NOACTIVATE：點預覽視窗不搶走清單的焦點
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let rect = client_rect(hwnd);
            {
                let buf = Buffered::new(hdc, rect);
                paint(&mut a.preview, hwnd, buf.hdc);
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEWHEEL => {
            wheel(&mut a.preview, ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32);
            0
        }
        WM_MOUSEHWHEEL => {
            hwheel(&mut a.preview, ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32);
            0
        }
        WM_MOUSEMOVE => {
            let st = &mut a.preview;
            if !st.tracking {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                st.tracking = true;
            }
            let h = hit(st, hwnd, get_x_lparam(lparam), get_y_lparam(lparam));
            if h != st.hot {
                st.hot = h;
                invalidate(hwnd);
            }
            0
        }
        WM_MOUSELEAVE => {
            let st = &mut a.preview;
            st.tracking = false;
            if st.hot != Hot::None {
                st.hot = Hot::None;
                invalidate(hwnd);
            }
            0
        }
        WM_LBUTTONDOWN => {
            match hit(&a.preview, hwnd, get_x_lparam(lparam), get_y_lparam(lparam)) {
                Hot::Close => a.hide_preview(),
                Hot::Open => {
                    if let Some(t) = a.preview.target.clone() {
                        a.hide_preview();
                        a.open_path(&t.path, t.is_dir);
                    }
                }
                Hot::None => {}
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 把影像合成到底色上，建立可直接貼上的 DIB。
fn make_bitmap(img: &Image, bg: COLORREF) -> HBITMAP {
    let (br, bgc, bb) = ((bg & 0xFF), ((bg >> 8) & 0xFF), ((bg >> 16) & 0xFF));
    let any_alpha = img.pixels.iter().any(|p| p >> 24 != 0);
    let premultiplied = any_alpha
        && img.pixels.iter().all(|p| {
            let a = p >> 24;
            (p & 0xFF) <= a && ((p >> 8) & 0xFF) <= a && ((p >> 16) & 0xFF) <= a
        });
    let out: Vec<u32> = img
        .pixels
        .iter()
        .map(|p| {
            if !any_alpha {
                return p | 0xFF00_0000;
            }
            let a = p >> 24;
            let (b, g, r) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
            let mix = |c: u32, back: u32| {
                if premultiplied {
                    (c + back * (255 - a) / 255).min(255)
                } else {
                    (c * a + back * (255 - a)) / 255
                }
            };
            0xFF00_0000 | mix(b, bb) | (mix(g, bgc) << 8) | (mix(r, br) << 16)
        })
        .collect();
    unsafe {
        let bi = BITMAPINFO::top_down_32(img.width, img.height);
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hdc = GetDC(0);
        let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
        ReleaseDC(0, hdc);
        if hbm != 0 && !bits.is_null() {
            std::ptr::copy_nonoverlapping(out.as_ptr(), bits as *mut u32, out.len());
        }
        hbm
    }
}

/// 資訊卡用的 48 像素系統圖示。
fn file_icon(path: &str) -> HICON {
    unsafe {
        let w = wide(path);
        let mut info: SHFILEINFOW = std::mem::zeroed();
        if SHGetFileInfoW(w.as_ptr(), 0, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) == 0 {
            return 0;
        }
        let mut list: *mut std::ffi::c_void = std::ptr::null_mut();
        if SHGetImageList(SHIL_EXTRALARGE, &IID_IIMAGELIST, &mut list) < 0 || list.is_null() {
            return 0;
        }
        (comctl().image_list_get_icon)(list as HIMAGELIST, info.iIcon, ILD_TRANSPARENT)
    }
}

// ---------------------------------------------------------------- Office 文件的繪製

/// 文件預覽的字型（依 DPI 建立）
#[derive(Clone, Copy)]
struct DocFonts {
    dpi: u32,
    body: HFONT,
    h1: HFONT,
    h2: HFONT,
    h3: HFONT,
    /// 投影片標籤
    label: HFONT,
}

fn create_doc_font(tenths_pt: i32, weight: i32, dpi: u32) -> HFONT {
    let face = wide("Microsoft JhengHei UI");
    unsafe { CreateFontW(-MulDiv(tenths_pt, dpi as i32, 720), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, face.as_ptr()) }
}

fn doc_fonts(state: &mut State) -> DocFonts {
    let dpi = app::theme().dpi;
    if let Some(f) = state.doc_fonts {
        if f.dpi == dpi {
            return f;
        }
        unsafe {
            for h in [f.body, f.h1, f.h2, f.h3, f.label] {
                DeleteObject(h);
            }
        }
    }
    let f = DocFonts {
        dpi,
        body: create_doc_font(100, FW_NORMAL, dpi),
        h1: create_doc_font(160, 700, dpi),
        h2: create_doc_font(130, FW_SEMIBOLD, dpi),
        h3: create_doc_font(110, FW_SEMIBOLD, dpi),
        label: create_doc_font(85, FW_SEMIBOLD, dpi),
    };
    state.doc_fonts = Some(f);
    f
}

/// 文件段落：自動換行（沒有空格的長字串也會斷開），展開定位點
const PARA_FMT: u32 = DT_LEFT | DT_TOP | DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL | DT_NOPREFIX;
const CELL_PAD_X: i32 = 8;
const CELL_PAD_Y: i32 = 5;
/// 表格下方「還有更多內容」說明的高度
const TABLE_NOTE: i32 = 22;
/// 試算表格線上方的空間
const SHEET_TOP: i32 = 6;

/// 文件的排版結果：依內容寬度、高度與 DPI，從頭往下排到需要的位置為止（長文件不必一次排完）
#[derive(Default)]
struct DocLayout {
    key: (i32, i32, u32),
    /// 封面的顯示大小（沒有封面時是 0）
    cover: (i32, i32),
    items: Vec<Laid>,
    /// 下一個區塊的位置；全部排完時就是總高度
    next: i32,
}

struct Laid {
    y: i32,
    h: i32,
    /// 表格的欄寬與列高
    grid: Option<(Vec<i32>, Vec<i32>)>,
}

fn block_font(b: &Block, f: &DocFonts) -> HFONT {
    match b {
        Block::Heading { level: 1, .. } => f.h1,
        Block::Heading { level: 2, .. } => f.h2,
        Block::Heading { .. } => f.h3,
        _ => f.body,
    }
}

/// 區塊前後的間距（未縮放）
fn block_spacing(b: &Block) -> (i32, i32) {
    match b {
        Block::Heading { level: 1, .. } => (18, 8),
        Block::Heading { .. } => (14, 6),
        Block::Paragraph(_) => (0, 9),
        Block::Item { .. } => (0, 4),
        Block::Slide { .. } => (24, 10),
        Block::Table(_) => (4, 14),
    }
}

fn item_indent(level: u8) -> i32 {
    s(22) * (level as i32 + 1)
}

/// 文件內容的範圍（內文區再往內縮一點）
fn doc_inner(body: &RECT) -> RECT {
    RECT { left: body.left + s(8), top: body.top + s(10), right: body.right - s(8), bottom: body.bottom }
}

/// 表格的欄寬（依內容，放不下時寬的欄平分空間）與列高（依換行後的文字高度）
fn table_grid(dc: HDC, t: &Table, width: i32) -> (Vec<i32>, Vec<i32>) {
    let f = &app::theme().fonts;
    let (px, py) = (s(CELL_PAD_X), s(CELL_PAD_Y));
    let font = |r: usize| if r == 0 { f.ui_bold } else { f.ui };
    let mut natural = vec![s(28); t.cols];
    for (r, row) in t.rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate().take(t.cols) {
            let w = cell.lines().map(|l| theme::text_width(dc, l, font(r))).max().unwrap_or(0);
            natural[c] = natural[c].max(w.min(s(360)) + 2 * px);
        }
    }
    let widths = office::fit_columns(&natural, width, s(56));
    let rows = t
        .rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let text_h = row.iter().zip(&widths).map(|(cell, w)| theme::text_height(dc, cell, font(r), w - 2 * px, PARA_FMT)).max().unwrap_or(0);
            text_h.max(f.ui_height) + 2 * py
        })
        .collect();
    (widths, rows)
}

/// 排版到 `until`（相對於內容頂端的像素）為止。
#[allow(clippy::too_many_arguments)]
fn extend_layout(dc: HDC, fonts: &DocFonts, doc: &Document, cover: Option<&Cover>, layout: &mut DocLayout, width: i32, height: i32, until: i32) {
    let key = (width, height, fonts.dpi);
    if layout.key != key {
        *layout = DocLayout { key, ..Default::default() };
        if let Some(c) = cover {
            // 封面最多占可見高度的一半多一點，不放大
            let max_h = (height * 55 / 100).max(s(120));
            let scale = (width as f32 / c.w as f32).min(max_h as f32 / c.h as f32).min(1.0);
            let (w, h) = (((c.w as f32 * scale).round() as i32).max(1), ((c.h as f32 * scale).round() as i32).max(1));
            layout.cover = (w, h);
            layout.next = h + s(6);
        }
    }
    while layout.items.len() < doc.blocks.len() && layout.next <= until {
        let i = layout.items.len();
        let b = &doc.blocks[i];
        let (before, after) = block_spacing(b);
        let y = layout.next + if i == 0 && layout.cover.1 == 0 { 0 } else { s(before) };
        let (h, grid) = match b {
            Block::Heading { text, .. } | Block::Paragraph(text) => (theme::text_height(dc, text, block_font(b, fonts), width, PARA_FMT), None),
            Block::Item { level, text } => (theme::text_height(dc, text, fonts.body, width - item_indent(*level), PARA_FMT), None),
            Block::Slide { .. } => (s(24), None),
            Block::Table(t) => {
                let (cols, rows) = table_grid(dc, t, width);
                let mut h = rows.iter().sum::<i32>() + 1;
                if t.more_rows || t.more_cols {
                    h += s(TABLE_NOTE);
                }
                (h, Some((cols, rows)))
            }
        };
        layout.items.push(Laid { y, h, grid });
        layout.next = y + h + s(after);
    }
}

/// 把點陣圖貼到 (x, y)，大小不同時以 HALFTONE 縮放。
#[allow(clippy::too_many_arguments)]
fn blit(dc: HDC, bitmap: HBITMAP, sw: i32, sh: i32, x: i32, y: i32, dw: i32, dh: i32) {
    unsafe {
        let mem = CreateCompatibleDC(dc);
        let old = SelectObject(mem, bitmap);
        if dw == sw && dh == sh {
            BitBlt(dc, x, y, dw, dh, mem, 0, 0, SRCCOPY);
        } else {
            SetStretchBltMode(dc, HALFTONE);
            SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
            StretchBlt(dc, x, y, dw, dh, mem, 0, 0, sw, sh, SRCCOPY);
        }
        SelectObject(mem, old);
        DeleteDC(mem);
    }
}

fn draw_table(dc: HDC, t: &Table, cols: &[i32], rows: &[i32], x: i32, y: i32) {
    let th = app::theme();
    let (p, f) = (&th.pal, &th.fonts);
    let (px, py) = (s(CELL_PAD_X), s(CELL_PAD_Y));
    let width: i32 = cols.iter().sum();
    let height: i32 = rows.iter().sum();
    let line = theme::blend(p.ink, p.surface, 0.16);
    // 第一列當作標題列
    if let Some(h0) = rows.first() {
        theme::fill(dc, &RECT { left: x, top: y, right: x + width, bottom: y + h0 }, theme::blend(p.ink, p.surface, 0.05));
    }
    let mut cy = y;
    for (r, (row, rh)) in t.rows.iter().zip(rows).enumerate() {
        let font = if r == 0 { f.ui_bold } else { f.ui };
        let mut cx = x;
        for (cell, cw) in row.iter().zip(cols) {
            if !cell.is_empty() {
                let cr = RECT { left: cx + px, top: cy + py, right: cx + cw - px, bottom: cy + rh - py + s(2) };
                theme::draw_text(dc, cell, &cr, font, p.ink, PARA_FMT);
            }
            cx += cw;
        }
        cy += rh;
        theme::fill(dc, &RECT { left: x, top: cy, right: x + width, bottom: cy + 1 }, line);
    }
    theme::fill(dc, &RECT { left: x, top: y, right: x + width, bottom: y + 1 }, line);
    let mut cx = x;
    theme::fill(dc, &RECT { left: cx, top: y, right: cx + 1, bottom: y + height + 1 }, line);
    for cw in cols {
        cx += cw;
        theme::fill(dc, &RECT { left: cx - 1, top: y, right: cx, bottom: y + height + 1 }, line);
    }
    if t.more_rows || t.more_cols {
        let note = RECT { left: x, top: y + height + s(4), right: x + width.max(s(320)), bottom: y + height + s(TABLE_NOTE) };
        theme::draw_text(dc, tr!("（表格還有更多內容沒有顯示）", "(The table has more content that isn't shown)"), &note, f.small, p.muted, DT_LEFT | DT_TOP | DT_SINGLELINE);
    }
}

fn draw_block(dc: HDC, b: &Block, l: &Laid, x: i32, y: i32, width: i32, fonts: &DocFonts) {
    let p = &app::theme().pal;
    let r = RECT { left: x, top: y, right: x + width, bottom: y + l.h };
    match b {
        Block::Heading { text, .. } => theme::draw_text(dc, text, &r, block_font(b, fonts), p.ink, PARA_FMT),
        Block::Paragraph(text) => theme::draw_text(dc, text, &r, fonts.body, p.ink, PARA_FMT),
        Block::Item { level, text } => {
            let indent = item_indent(*level);
            let bullet = match level % 3 {
                0 => "•",
                1 => "–",
                _ => "·",
            };
            let br = RECT { left: x + indent - s(16), right: x + indent, ..r };
            theme::draw_text(dc, bullet, &br, fonts.body, p.muted, DT_LEFT | DT_TOP | DT_SINGLELINE);
            theme::draw_text(dc, text, &RECT { left: x + indent, ..r }, fonts.body, p.ink, PARA_FMT);
        }
        Block::Slide { number } => {
            let label = trf!("投影片 {number}", "Slide {number}");
            let pill = RECT { left: x, top: y, right: x + theme::text_width(dc, &label, fonts.label) + s(22), bottom: y + l.h };
            theme::round_rect(dc, &pill, pill.height(), Some(theme::blend(p.accent, p.surface, 0.14)), None);
            theme::draw_text(dc, &label, &pill, fonts.label, p.accent, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let mid = y + l.h / 2;
            theme::fill(dc, &RECT { left: pill.right + s(10), top: mid, right: x + width, bottom: mid + 1 }, theme::blend(p.ink, p.surface, 0.12));
        }
        Block::Table(t) => {
            if let Some((cols, rows)) = &l.grid {
                draw_table(dc, t, cols, rows, x, y);
            }
        }
    }
}

fn paint_document(state: &mut State, dc: HDC, body: &RECT) {
    let fonts = doc_fonts(state);
    let th = app::theme();
    let p = &th.pal;
    let inner = doc_inner(body);
    let scroll = state.scroll as i32;
    let Content::Document { doc, cover, layout, .. } = &mut state.content else { return };
    extend_layout(dc, &fonts, doc, cover.as_ref(), layout, inner.width(), inner.height(), scroll + inner.height());
    if doc.blocks.is_empty() && cover.is_none() {
        theme::draw_text(dc, tr!("（文件裡沒有文字內容）", "(The document has no text)"), body, th.fonts.ui, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        return;
    }
    unsafe {
        SaveDC(dc);
        IntersectClipRect(dc, body.left, body.top, body.right, body.bottom);
    }
    if let Some(c) = cover {
        let (w, h) = layout.cover;
        let x = inner.left + (inner.width() - w) / 2;
        let y = inner.top - scroll;
        if y + h > body.top {
            blit(dc, c.bitmap, c.w, c.h, x, y, w, h);
            theme::round_rect(dc, &RECT::new(x - 1, y - 1, w + 2, h + 2), 0, None, Some(theme::blend(p.ink, p.surface, 0.14)));
        }
    }
    let first = layout.items.partition_point(|l| l.y + l.h < scroll);
    for (b, l) in doc.blocks.iter().zip(&layout.items).skip(first) {
        let y = inner.top + l.y - scroll;
        if y > body.bottom {
            break;
        }
        draw_block(dc, b, l, inner.left, y, inner.width(), &fonts);
    }
    unsafe {
        RestoreDC(dc, -1);
    }
}

/// 文件捲動 `delta` 像素（需要時往下排版）。
fn scroll_document(state: &mut State, hwnd: HWND, delta: i64) {
    let fonts = doc_fonts(state);
    let inner = doc_inner(&body_rect(&client_rect(hwnd)));
    let want = (state.scroll as i64 + delta).clamp(0, i32::MAX as i64 / 2) as i32;
    let Content::Document { doc, cover, layout, .. } = &mut state.content else { return };
    unsafe {
        let dc = GetDC(0);
        extend_layout(dc, &fonts, doc, cover.as_ref(), layout, inner.width(), inner.height(), want + inner.height() * 2);
        ReleaseDC(0, dc);
    }
    let max = (layout.next - inner.height() + s(12)).max(0);
    let next = want.min(max) as usize;
    if next != state.scroll {
        state.scroll = next;
        invalidate(hwnd);
    }
}

fn sheet_row_height() -> i32 {
    app::theme().fonts.ui_height + s(10)
}

/// 試算表的欄寬（依前 200 列的內容）與列號欄的寬度
fn sheet_columns(dc: HDC, book: &Workbook) -> (Vec<i32>, i32) {
    let f = &app::theme().fonts;
    let t = &book.table;
    let pad = s(CELL_PAD_X);
    let mut cols: Vec<i32> = (0..t.cols).map(|c| (theme::text_width(dc, &office::column_name(c), f.small) + 2 * pad).max(s(44))).collect();
    for row in t.rows.iter().take(200) {
        for (c, cell) in row.iter().enumerate().take(t.cols) {
            if !cell.is_empty() {
                let w = cell.lines().map(|l| theme::text_width(dc, l, f.ui)).max().unwrap_or(0);
                cols[c] = cols[c].max(w.min(s(260)) + 2 * pad);
            }
        }
    }
    let gutter = theme::text_width(dc, &t.rows.len().max(1).to_string(), f.small) + 2 * pad;
    (cols, gutter.max(s(36)))
}

fn sheet_content(book: Workbook, caption: String) -> Content {
    let dpi = app::theme().dpi;
    let (cols, gutter) = unsafe {
        let dc = GetDC(0);
        let r = sheet_columns(dc, &book);
        ReleaseDC(0, dc);
        r
    };
    Content::Sheet { book, caption, cols, gutter, dpi }
}

fn sheet_grid(body: &RECT) -> RECT {
    RECT { top: body.top + s(SHEET_TOP), bottom: body.bottom - s(2), ..*body }
}

fn paint_sheet(state: &mut State, dc: HDC, body: &RECT) {
    let th = app::theme();
    let (p, f) = (&th.pal, &th.fonts);
    let (top, left) = (state.scroll, state.hscroll);
    let Content::Sheet { book, cols, gutter, dpi, .. } = &mut state.content else { return };
    if *dpi != th.dpi {
        (*cols, *gutter) = sheet_columns(dc, book);
        *dpi = th.dpi;
    }
    let t = &book.table;
    if t.rows.is_empty() {
        theme::draw_text(dc, tr!("（這個工作表是空的）", "(This sheet is empty)"), body, f.ui, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        return;
    }
    let grid = sheet_grid(body);
    let rh = sheet_row_height();
    let pad = s(CELL_PAD_X);
    let line = theme::blend(p.ink, p.surface, 0.12);
    let head_bg = theme::blend(p.ink, p.surface, 0.045);
    unsafe {
        SaveDC(dc);
        IntersectClipRect(dc, grid.left, grid.top, grid.right, grid.bottom);
    }
    // 看得到的欄
    let mut visible: Vec<(usize, i32, i32)> = Vec::new();
    let mut x = grid.left + *gutter;
    for (c, w) in cols.iter().enumerate().skip(left) {
        if x >= grid.right {
            break;
        }
        visible.push((c, x, *w));
        x += w;
    }
    let right = x.min(grid.right);
    let rows_bottom = (grid.top + rh * (t.rows.len().saturating_sub(top) as i32 + 1)).min(grid.bottom);
    // 欄名列與列號欄的底色
    theme::fill(dc, &RECT { left: grid.left, top: grid.top, right, bottom: grid.top + rh }, head_bg);
    theme::fill(dc, &RECT { left: grid.left, top: grid.top, right: grid.left + *gutter, bottom: rows_bottom }, head_bg);
    for &(c, x, w) in &visible {
        let r = RECT { left: x, top: grid.top, right: x + w, bottom: grid.top + rh };
        theme::draw_text(dc, &office::column_name(c), &r, f.small, p.muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let mut y = grid.top + rh;
    for (ri, row) in t.rows.iter().enumerate().skip(top) {
        if y >= grid.bottom {
            break;
        }
        let num = RECT { left: grid.left, top: y, right: grid.left + *gutter - pad, bottom: y + rh };
        theme::draw_text(dc, &(ri + 1).to_string(), &num, f.small, p.muted, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
        for &(c, x, w) in &visible {
            let Some(cell) = row.get(c).filter(|c| !c.is_empty()) else { continue };
            let text = if cell.contains('\n') { cell.replace('\n', " ") } else { cell.clone() };
            let align = match office::cell_align(cell) {
                office::Align::Left => DT_LEFT,
                office::Align::Center => DT_CENTER,
                office::Align::Right => DT_RIGHT,
            };
            let r = RECT { left: x + pad, top: y, right: x + w - pad, bottom: y + rh };
            theme::draw_text(dc, &text, &r, f.ui, p.ink, align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        y += rh;
        theme::fill(dc, &RECT { left: grid.left, top: y - 1, right, bottom: y }, line);
    }
    // 格線
    theme::fill(dc, &RECT { left: grid.left, top: grid.top + rh - 1, right, bottom: grid.top + rh }, line);
    theme::fill(dc, &RECT { left: grid.left + *gutter - 1, top: grid.top, right: grid.left + *gutter, bottom: rows_bottom }, line);
    for &(_, x, w) in &visible {
        theme::fill(dc, &RECT { left: x + w - 1, top: grid.top, right: x + w, bottom: rows_bottom }, line);
    }
    unsafe {
        RestoreDC(dc, -1);
    }
}

/// 試算表捲動 `rows` 列、`cols` 欄（最後一列、最右邊的欄完整顯示時就停）。
fn scroll_sheet(state: &mut State, hwnd: HWND, rows: i64, cols: i64) {
    let Content::Sheet { book, cols: widths, gutter, .. } = &state.content else { return };
    let grid = sheet_grid(&body_rect(&client_rect(hwnd)));
    let rh = sheet_row_height().max(1);
    let visible = ((grid.height() - rh) / rh).max(1) as usize;
    let max_top = book.table.rows.len().saturating_sub(visible);
    let avail = grid.width() - gutter;
    let mut max_left = widths.len().saturating_sub(1);
    let mut sum = 0;
    for i in (0..widths.len()).rev() {
        sum += widths[i];
        if sum > avail {
            break;
        }
        max_left = i;
    }
    let top = (state.scroll as i64 + rows).clamp(0, max_top as i64) as usize;
    let left = (state.hscroll as i64 + cols).clamp(0, max_left as i64) as usize;
    if (top, left) != (state.scroll, state.hscroll) {
        state.scroll = top;
        state.hscroll = left;
        invalidate(hwnd);
    }
}

// ---------------------------------------------------------------- 背景載入

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn msg(text: &str) -> Result<Loaded, FsError> {
    Ok(Loaded::Message(text.to_string()))
}

/// 最近產生過的預覽：同一個檔案（路徑、大小、修改時間都相同）以同樣的設定再預覽時直接重用。
/// 懸停時滑鼠在幾個影片或圖片之間來回，就不必每次重新啟動縮圖工作程序。
#[derive(PartialEq, Eq)]
struct CacheKey {
    path: String,
    size: u64,
    modified: u64,
    edge: i32,
    images: bool,
    pdf: bool,
}

/// 總共最多約 32 MiB、24 個項目（一張置中大小的影片縮圖約 2～3 MiB）。
static CACHE: Mutex<ByteLru<CacheKey, Loaded>> = Mutex::new(ByteLru::new(32 * 1024 * 1024, 24));

/// 快取項目大約占用的記憶體；訊息不快取（本來就很快，錯誤也要能重試）。
fn cache_cost(loaded: &Loaded) -> Option<usize> {
    match loaded {
        Loaded::Image(img) => Some(img.pixels.len() * 4 + img.caption.len() + 64),
        Loaded::Text { doc, caption } => Some(
            doc.lines.iter().map(|l| l.text.len() + l.spans.len() * std::mem::size_of::<quicklook::Span>() + 48).sum::<usize>() + caption.len() + 64,
        ),
        Loaded::Office { office, cover, caption } => Some(office.approx_bytes() + cover.as_ref().map_or(0, |c| c.pixels.len() * 4) + caption.len() + 64),
        Loaded::Message(_) => None,
    }
}

fn load(target: &Target, edge: i32, opts: Opts, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    let path = target.path.as_str();
    mark("背景：讀取屬性");
    let Some((attrs, size, modified)) = fs::attributes_size_modified(path) else {
        return Err(FsError::Message(trf!("無法讀取：{}", "Couldn't read: {}", last_error_text())));
    };
    if attrs & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS) != 0 {
        return msg(tr!("這個檔案在雲端，還沒下載到這台電腦；為了不觸發下載，先不讀取內容。按 Enter 開啟。", "This file is in the cloud and hasn't been downloaded to this PC; its contents aren't read so the download isn't triggered. Press Enter to open it."));
    }
    if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
        let count = std::fs::read_dir(paths::to_long_path(path)).map(|d| d.take(100_000).count()).unwrap_or(0);
        return Ok(Loaded::Message(if count == 0 { tr!("空的資料夾", "Empty folder").to_string() } else { trn!(count; "這個資料夾有 {count} 個項目。按 Enter 開啟。", "This folder has {count} item. Press Enter to open it.", "This folder has {count} items. Press Enter to open it.") }));
    }
    let key = CacheKey { path: path.to_string(), size, modified, edge, images: opts.images, pdf: opts.pdf };
    if let Some(hit) = CACHE.lock().ok().and_then(|mut c| c.get(&key)) {
        mark("背景：使用快取");
        return Ok(hit);
    }
    let result = load_content(target, size, edge, opts, cancel);
    if let Ok(loaded) = &result {
        if let Some(cost) = cache_cost(loaded) {
            if let Ok(mut c) = CACHE.lock() {
                c.put(key, loaded.clone(), cost);
            }
        }
    }
    result
}

/// 效能量測（`--perf`）：用與懸停預覽相同的程式路徑載入一個檔案，回傳耗時（毫秒）與結果說明。
/// `use_cache` 為 false 時先清空 Stillfeather 自己的預覽快取（量測真正的解碼或縮圖工作程序）。
pub fn perf_load(path: &str, edge: i32, use_cache: bool) -> (f64, String) {
    if !use_cache {
        if let Ok(mut c) = CACHE.lock() {
            c.clear();
        }
    }
    let target = Target { path: path.to_string(), name: paths::file_name(path).to_string(), is_dir: false, meta: String::new() };
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let result = load(&target, edge, Opts { images: true, pdf: true, space: false }, &cancel);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let what = match result {
        Ok(Loaded::Image(img)) => trf!("影像 {}×{}（{}）", "Image {}×{} ({})", img.width, img.height, img.caption),
        Ok(Loaded::Text { doc, caption }) => trf!("文字 {} 行（{}）", "Text, {} lines ({})", doc.lines.len(), caption),
        Ok(Loaded::Office { cover, caption, .. }) => trf!("Office（{caption}{}）", "Office ({caption}{})", if cover.is_some() { tr!(" · 含封面縮圖", " · with cover thumbnail") } else { "" }),
        Ok(Loaded::Message(m)) => trf!("訊息：{m}", "Message: {m}"),
        Err(FsError::Message(m)) => trf!("錯誤：{m}", "Error: {m}"),
        Err(FsError::Cancelled) => tr!("已取消", "Cancelled").into(),
    };
    (ms, what)
}

fn load_content(target: &Target, size: u64, edge: i32, opts: Opts, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    let path = target.path.as_str();
    match quicklook::classify(&target.name) {
        Kind::Image | Kind::Video if !opts.images => msg(tr!("設定中已關閉圖片與影片的快速預覽。", "Quick preview of images and videos is turned off in Settings.")),
        Kind::Pdf if !opts.pdf => msg(tr!("設定中已關閉 PDF 的快速預覽。", "Quick preview of PDFs is turned off in Settings.")),
        Kind::Image if size > MAX_IMAGE_BYTES => msg(tr!("圖片超過 256 MiB，不產生預覽。", "The image is larger than 256 MiB, so no preview is made.")),
        Kind::Image => thumbnail(path, edge, false, cancel),
        Kind::Video => thumbnail(path, edge, true, cancel),
        Kind::Pdf if size > MAX_PDF_BYTES => msg(tr!("PDF 超過 64 MiB，不產生預覽。", "The PDF is larger than 64 MiB, so no preview is made.")),
        Kind::Pdf => pdf(path, edge, cancel),
        Kind::Text(lang) => text(path, size, lang),
        Kind::Office(_) if size > MAX_OFFICE_BYTES => msg(tr!("文件超過 64 MiB，不產生預覽。", "The document is larger than 64 MiB, so no preview is made.")),
        Kind::Office(format) => office_doc(path, format, edge, opts, cancel),
        Kind::LegacyOffice => msg(tr!("舊版 Office 格式（.doc、.xls、.ppt）沒有內容預覽；用 Office 另存成 .docx、.xlsx 或 .pptx 就能預覽。按 Enter 開啟。", "Old Office formats (.doc, .xls, .ppt) have no content preview; save them as .docx, .xlsx or .pptx in Office to preview them. Press Enter to open.")),
        Kind::Other => {
            // 沒有副檔名的小檔案（LICENSE、README）試著當純文字
            if paths::extension_lower(&target.name).is_empty() && size <= MAX_TEXT_BYTES {
                if let Ok(Loaded::Text { doc, caption }) = text(path, size, Lang::Plain) {
                    return Ok(Loaded::Text { doc, caption });
                }
            }
            msg(tr!("這種檔案沒有內容預覽。按 Enter 用預設程式開啟。", "There's no content preview for this type of file. Press Enter to open it in its default app."))
        }
    }
}

/// 這台電腦能不能產生簡報的縮圖（裝了 Office 或 LibreOffice 的縮圖處理程式時可以）：
/// 0 = 還不知道、1 = 可以、2 = 不行（之後不再啟動縮圖工作程序，只顯示文字）。
static SHELL_OFFICE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn office_doc(path: &str, format: Format, edge: i32, opts: Opts, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    mark("背景：讀取 Office 文件");
    let bytes = std::fs::read(paths::to_long_path(path)).map_err(|e| FsError::Message(trf!("無法讀取：{e}", "Couldn't read: {e}")))?;
    if cancel.load(Ordering::Relaxed) {
        return Err(FsError::Cancelled);
    }
    // 簡報：文件裡存了縮圖時，同時請 Windows 產生第 1 張投影片的影像當封面（和讀文字同時進行）
    let want_cover = format.is_presentation() && opts.images && SHELL_OFFICE.load(Ordering::Relaxed) != 2 && office::has_thumbnail(&bytes);
    std::thread::scope(|scope| {
        let cover = want_cover.then(|| scope.spawn(move || thumbnail(path, edge.min(COVER_EDGE), false, cancel)));
        mark("背景：解析 Office 文件");
        let extracted = office::extract(format, &bytes);
        mark("背景：解析 Office 文件完成");
        let cover = match cover.map(|h| h.join()) {
            Some(Ok(Ok(Loaded::Image(img)))) => {
                SHELL_OFFICE.store(1, Ordering::Relaxed);
                Some(img)
            }
            Some(Ok(Err(FsError::Cancelled))) | None => None,
            Some(_) => {
                // 第一次就失敗：這台電腦沒有簡報的縮圖處理程式
                let _ = SHELL_OFFICE.compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed);
                None
            }
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(FsError::Cancelled);
        }
        match (extracted, cover) {
            (Ok(office), cover) => {
                let caption = office::caption(format, &office);
                Ok(Loaded::Office { office, cover, caption })
            }
            // 文字讀不出來但有縮圖：至少顯示第 1 張投影片
            (Err(_), Some(mut img)) => {
                img.caption = trf!("{} · 第 1 張投影片", "{} · slide 1", format.label());
                Ok(Loaded::Image(img))
            }
            (Err(e), None) => msg(&e),
        }
    })
}

fn text(path: &str, size: u64, lang: Lang) -> Result<Loaded, FsError> {
    mark("背景：讀取文字");
    let mut bytes = Vec::new();
    std::fs::File::open(paths::to_long_path(path))
        .and_then(|f| f.take(MAX_TEXT_BYTES).read_to_end(&mut bytes))
        .map_err(|e| FsError::Message(trf!("無法讀取：{e}", "Couldn't read: {e}")))?;
    let (content, encoding) = match quicklook::decode(&bytes) {
        Decoded::Text(t) => (t, ""),
        Decoded::Binary => return msg(tr!("看起來是二進位檔案，沒有文字預覽。", "This looks like a binary file, so there's no text preview.")),
        Decoded::Legacy => (legacy_decode(&bytes), tr!("系統字碼頁 · ", "System code page · ")),
    };
    mark("背景：語法上色");
    let doc = quicklook::highlight(&content, lang);
    mark("背景：語法上色完成");
    let mut caption = trf!("{}{} · {} 行", "{}{} · {} lines", encoding, lang.label(), doc.lines.len());
    if size > MAX_TEXT_BYTES {
        caption.push_str(tr!(" · 只顯示前 1 MiB", " · only the first 1 MiB is shown"));
    } else if doc.truncated_lines {
        caption.push_str(&trf!(" · 只顯示前 {} 行", " · only the first {} lines are shown", quicklook::MAX_LINES));
    }
    if doc.long_lines {
        caption.push_str(tr!(" · 過長的行已截斷", " · long lines are cut off"));
    }
    Ok(Loaded::Text { doc, caption })
}

/// 不是 UTF-8 的文字以系統預設字碼頁（繁體中文 Windows 為 Big5）解碼。
fn legacy_decode(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    unsafe {
        let n = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut out = vec![0u16; n as usize];
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, out.as_mut_ptr(), n);
        String::from_utf16_lossy(&out)
    }
}

fn duration_text(hundred_ns: u64) -> String {
    let secs = hundred_ns / 10_000_000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn thumbnail(path: &str, edge: i32, video: bool, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    mark("背景：縮圖工作程序");
    let exe = std::env::current_exe().map_err(|e| FsError::Message(trf!("找不到預覽工作程式：{e}", "Can't find the preview worker: {e}")))?;
    let edge_arg = edge.to_string();
    let out = run_child(exe.as_os_str(), &["--preview-worker".as_ref(), path.as_ref(), edge_arg.as_ref()], &[], cancel)?;
    let (mut img, ow, oh, dur) = parse_worker_output(out, video)?;
    let mut parts = vec![if video { tr!("影片縮圖", "Video thumbnail").to_string() } else { tr!("圖片", "Image").to_string() }];
    if ow > 0 && oh > 0 {
        parts.push(format!("{ow} × {oh}"));
    }
    if dur > 0 {
        parts.push(duration_text(dur));
    }
    let ext = paths::extension_lower(path);
    if !video && (ext == "gif" || ext.starts_with("tif")) {
        parts.push(tr!("顯示第 1 幀", "Showing frame 1").into());
    }
    img.caption = parts.join(" · ");
    Ok(Loaded::Image(img))
}

/// 這台電腦的 Windows 能不能直接產生 PDF 縮圖（裝了 Acrobat 等 PDF 縮圖處理程式時可以）：
/// 0 = 還不知道、1 = 可以、2 = 不行。只在第一次預覽 PDF 時試一次。
static SHELL_PDF: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn pdf(path: &str, edge: i32, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    // 先用 Windows 的縮圖介面（約 0.1 秒）；沒有 PDF 縮圖處理程式時才改用 PowerShell 轉第 1 頁（啟動就要約 0.4 秒）。
    if SHELL_PDF.load(Ordering::Relaxed) != 2 {
        match thumbnail(path, edge, false, cancel) {
            Ok(Loaded::Image(mut img)) => {
                SHELL_PDF.store(1, Ordering::Relaxed);
                img.caption = tr!("PDF 第 1 頁", "PDF page 1").to_string();
                return Ok(Loaded::Image(img));
            }
            Err(FsError::Cancelled) => return Err(FsError::Cancelled),
            _ => {
                // 第一次就失敗：這台電腦沒有 PDF 縮圖處理程式，之後直接用 PowerShell
                if SHELL_PDF.load(Ordering::Relaxed) == 0 {
                    SHELL_PDF.store(2, Ordering::Relaxed);
                }
            }
        }
    }
    pdf_powershell(path, edge, cancel)
}

fn pdf_powershell(path: &str, edge: i32, cancel: &AtomicBool) -> Result<Loaded, FsError> {
    let exe = std::env::current_exe().map_err(|e| FsError::Message(trf!("找不到預覽工作程式：{e}", "Can't find the preview worker: {e}")))?;
    let dir = std::env::temp_dir().join(format!(
        "FeatherFiles-preview-{}-{}-{}",
        std::process::id(),
        unsafe { GetTickCount64() },
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| FsError::Message(trf!("無法建立暫存資料夾：{e}", "Couldn't create the temporary folder: {e}")))?;
    let result = (|| {
        let png = dir.join("preview.png");
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let ps = std::path::Path::new(&system_root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        let script = feather_core::base64::encode_utf16le(PDF_SCRIPT);
        let png_str = png.to_string_lossy().to_string();
        let edge_str = edge.to_string();
        let out = run_child(
            ps.as_os_str(),
            &[
                "-NoProfile".as_ref(),
                "-NonInteractive".as_ref(),
                "-WindowStyle".as_ref(),
                "Hidden".as_ref(),
                "-OutputFormat".as_ref(),
                "Text".as_ref(),
                "-EncodedCommand".as_ref(),
                script.as_ref(),
            ],
            &[("FEATHER_PREVIEW_INPUT", path), ("FEATHER_PREVIEW_OUTPUT", &png_str), ("FEATHER_PREVIEW_EDGE", &edge_str)],
            cancel,
        )?;
        if out.code != 0 || !png.exists() {
            return Err(FsError::Message(simplify_error(&out.stderr, true)));
        }
        let pages: Option<u32> = String::from_utf8_lossy(&out.stdout).trim().parse().ok();
        let png_os = png.as_os_str().to_owned();
        let out = run_child(exe.as_os_str(), &["--preview-worker".as_ref(), png_os.as_ref(), edge_str.as_ref()], &[], cancel)?;
        let (mut img, _, _, _) = parse_worker_output(out, false)?;
        img.caption = match pages {
            Some(n) if n > 1 => trf!("PDF 第 1 頁，共 {n} 頁", "PDF page 1 of {n}"),
            _ => tr!("PDF 第 1 頁", "PDF page 1").to_string(),
        };
        Ok(Loaded::Image(img))
    })();
    // 只刪除這次建立的唯一暫存資料夾。
    let _ = std::fs::remove_dir_all(&dir);
    result
}

struct ChildOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_child(program: &std::ffi::OsStr, args: &[&std::ffi::OsStr], envs: &[(&str, &str)], cancel: &AtomicBool) -> Result<ChildOutput, FsError> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| FsError::Message(trf!("無法啟動預覽工作程式：{e}", "Couldn't start the preview worker: {e}")))?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.take(16 * 1024 * 1024).read_to_end(&mut v);
        }
        v
    });
    let err_reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.take(64 * 1024).read_to_end(&mut v);
        }
        v
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                return Err(FsError::Message(e.to_string()));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FsError::Cancelled);
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FsError::Message(tr!("預覽超過 8 秒，已停止工作程式。", "The preview took longer than 8 seconds, so the worker was stopped.").into()));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(ChildOutput { code: status.code().unwrap_or(-1), stdout, stderr })
}

fn simplify_error(stderr: &[u8], pdf: bool) -> String {
    let text = String::from_utf8_lossy(stderr);
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if first.is_empty() {
        return if pdf {
            tr!("PDF 無法預覽；檔案可能損壞、受密碼保護，或系統 PDF 元件不可用。", "The PDF can't be previewed; the file may be damaged or password-protected, or the system PDF component isn't available.").into()
        } else {
            tr!("沒有可用的預覽；檔案可能損壞，或系統缺少對應的解碼器。", "No preview available; the file may be damaged, or the system is missing a suitable decoder.").into()
        };
    }
    let short: String = first.chars().take(220).collect();
    trf!("無法預覽：{short}", "Can't preview: {short}")
}

/// 解析工作程序的輸出："FFP2"、縮圖寬高、原始寬高、長度（100 奈秒）、BGRA 像素。
fn parse_worker_output(out: ChildOutput, video: bool) -> Result<(Image, u32, u32, u64), FsError> {
    if out.code != 0 {
        let mut e = simplify_error(&out.stderr, false);
        if video && out.stderr.is_empty() {
            e = tr!("沒有可用的影片縮圖；系統可能缺少這種影片格式的解碼器。", "No video thumbnail available; the system may be missing a decoder for this video format.").into();
        }
        return Err(FsError::Message(e));
    }
    let b = &out.stdout;
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    if b.len() < 28 || &b[0..4] != MAGIC {
        return Err(FsError::Message(tr!("預覽輸出無效。", "The preview output is invalid.").into()));
    }
    let w = u32_at(4) as i32;
    let h = u32_at(8) as i32;
    let (ow, oh) = (u32_at(12), u32_at(16));
    let dur = u64::from_le_bytes(b[20..28].try_into().unwrap_or([0; 8]));
    if w < 1 || h < 1 || w > 2 * MAX_EDGE || h > 2 * MAX_EDGE || b.len() != 28 + (w * h * 4) as usize {
        return Err(FsError::Message(tr!("預覽尺寸超過上限。", "The preview is larger than the size limit.").into()));
    }
    let pixels = b[28..].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    Ok((Image { width: w, height: h, pixels, caption: String::new() }, ow, oh, dur))
}

// ---------------------------------------------------------------- 工作程序（--preview-worker）

#[repr(C)]
struct ImageFactoryVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    get_image: unsafe extern "system" fn(*mut std::ffi::c_void, SIZE, u32, *mut HBITMAP) -> HRESULT,
}

#[repr(C)]
struct ComObject<V> {
    vtbl: *const V,
}

#[repr(C)]
struct PropertyStoreVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    get_count: usize,
    get_at: usize,
    get_value: unsafe extern "system" fn(*mut std::ffi::c_void, *const PROPERTYKEY, *mut PROPVARIANT) -> HRESULT,
}

const IID_ISHELLITEMIMAGEFACTORY: GUID = GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);
const IID_IPROPERTYSTORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
/// System.Image.HorizontalSize／VerticalSize
const FMTID_IMAGE: GUID = GUID::from_u128(0x6444048f_4c8b_11d1_8b70_080036b11a03);
/// System.Video.FrameWidth／FrameHeight
const FMTID_VIDEO: GUID = GUID::from_u128(0x64440491_4c8b_11d1_8b70_080036b11a03);
/// System.Media.Duration
const FMTID_MEDIA: GUID = GUID::from_u128(0x64440490_4c8b_11d1_8b70_080036b11a03);

/// 子程序入口：輸出 "FFP2" + 縮圖寬高 + 原始寬高 + 長度 + BGRA 像素到標準輸出。
pub fn worker_main(path: &str, edge: Option<&str>) -> i32 {
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let edge = edge.and_then(|e| e.parse::<i32>().ok()).unwrap_or(640).clamp(64, MAX_EDGE);
    let result = unsafe { extract(path, edge) };
    match result {
        Ok((w, h, pixels)) => {
            let (ow, oh, dur) = unsafe { properties(path) };
            let mut out = Vec::with_capacity(28 + pixels.len() * 4);
            out.extend_from_slice(MAGIC);
            out.extend_from_slice(&(w as u32).to_le_bytes());
            out.extend_from_slice(&(h as u32).to_le_bytes());
            out.extend_from_slice(&ow.to_le_bytes());
            out.extend_from_slice(&oh.to_le_bytes());
            out.extend_from_slice(&dur.to_le_bytes());
            for p in pixels {
                out.extend_from_slice(&p.to_le_bytes());
            }
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&out).and_then(|_| stdout.flush()).is_err() {
                return 3;
            }
            0
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "{e}");
            2
        }
    }
}

/// 原始寬高與影片長度（讀不到就是 0）。
unsafe fn properties(path: &str) -> (u32, u32, u64) {
    let w = wide(path);
    let mut obj: *mut std::ffi::c_void = std::ptr::null_mut();
    if SHGetPropertyStoreFromParsingName(w.as_ptr(), std::ptr::null_mut(), 0, &IID_IPROPERTYSTORE, &mut obj) < 0 || obj.is_null() {
        return (0, 0, 0);
    }
    let vt = &*(*(obj as *mut ComObject<PropertyStoreVtbl>)).vtbl;
    let get = |fmtid: GUID, pid: u32| -> u64 {
        let key = PROPERTYKEY { fmtid, pid };
        let mut pv: PROPVARIANT = std::mem::zeroed();
        if (vt.get_value)(obj, &key, &mut pv) < 0 {
            return 0;
        }
        let v = match pv.vt {
            VT_UI4 => pv.data[0] & 0xFFFF_FFFF,
            VT_UI8 => pv.data[0],
            _ => 0,
        };
        PropVariantClear(&mut pv);
        v
    };
    let mut width = get(FMTID_IMAGE, 3) as u32;
    let mut height = get(FMTID_IMAGE, 4) as u32;
    if width == 0 || height == 0 {
        width = get(FMTID_VIDEO, 3) as u32;
        height = get(FMTID_VIDEO, 4) as u32;
    }
    let dur = get(FMTID_MEDIA, 3);
    (vt.release)(obj);
    (width, height, dur)
}

unsafe fn extract(path: &str, edge: i32) -> Result<(i32, i32, Vec<u32>), String> {
    extract_with(path, edge, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY)
}

/// 用 Windows Shell 的縮圖介面取得 `path` 縮到邊長 `edge` 以內的影像（BGRA 像素）。清單縮圖的工作程序也用它。
pub(super) unsafe fn extract_with(path: &str, edge: i32, flags: u32) -> Result<(i32, i32, Vec<u32>), String> {
    let w = wide(path);
    let mut obj: *mut std::ffi::c_void = std::ptr::null_mut();
    let hr = SHCreateItemFromParsingName(w.as_ptr(), std::ptr::null_mut(), &IID_ISHELLITEMIMAGEFACTORY, &mut obj);
    if hr < 0 || obj.is_null() {
        return Err(trf!("無法開啟檔案（0x{:08X}）。", "Couldn't open the file (0x{:08X}).", hr as u32));
    }
    let vt = &*(*(obj as *mut ComObject<ImageFactoryVtbl>)).vtbl;
    let mut hbm: HBITMAP = 0;
    let hr = (vt.get_image)(obj, SIZE { cx: edge, cy: edge }, flags, &mut hbm);
    (vt.release)(obj);
    if hr < 0 || hbm == 0 {
        return Err(tr!("沒有可用的縮圖：格式可能不支援、檔案損壞，或系統缺少對應的解碼器。", "No thumbnail available: the format may not be supported, the file may be damaged, or the system is missing a suitable decoder.").into());
    }
    let mut bm: BITMAP = std::mem::zeroed();
    GetObjectW(hbm, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut std::ffi::c_void);
    let (bw, bh) = (bm.bmWidth, bm.bmHeight.abs());
    if bw < 1 || bh < 1 || bw > 2 * MAX_EDGE || bh > 2 * MAX_EDGE {
        DeleteObject(hbm);
        return Err(tr!("縮圖尺寸無效。", "Invalid thumbnail size.").into());
    }
    let mut pixels = vec![0u32; (bw * bh) as usize];
    let mut bi = BITMAPINFO::top_down_32(bw, bh);
    let hdc = GetDC(0);
    let lines = GetDIBits(hdc, hbm, 0, bh as u32, pixels.as_mut_ptr() as *mut std::ffi::c_void, &mut bi, DIB_RGB_COLORS);
    ReleaseDC(0, hdc);
    DeleteObject(hbm);
    if lines == 0 {
        return Err(tr!("無法讀取縮圖像素。", "Couldn't read the thumbnail pixels.").into());
    }
    Ok((bw, bh, pixels))
}

// Windows PowerShell 5.1 提供 .NET Framework 的 WinRT 投影；不覆寫執行原則、不載入模組或下載任何東西。
// 使用者資料只透過環境變數傳入，不會被當成腳本文字或殼層參數解讀。
const PDF_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
$previewMemory = $null
$previewRandom = $null
$previewPage = $null
$previewOutput = $null
$previewManaged = $null
try {
    $previewPath = [IO.Path]::GetFullPath($env:FEATHER_PREVIEW_INPUT)
    $previewCurrent = $previewPath
    $previewLeaf = $true
    while ($null -ne $previewCurrent) {
        $previewAttributes = [int][IO.File]::GetAttributes($previewCurrent)
        if (($previewAttributes -band (0x1000 -bor 0x40000 -bor 0x400000)) -ne 0) { throw '雲端尚未下載或離線的檔案不讀取內容。' }
        if ($previewLeaf -and (($previewAttributes -band 0x10) -ne 0)) { throw '資料夾不提供內容預覽。' }
        $previewLeaf = $false
        $previewCurrent = [IO.Path]::GetDirectoryName($previewCurrent)
    }
    $previewSource = [IO.File]::Open($previewPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
    try {
        if ($previewSource.Length -gt 67108864) { throw '檔案超過 64 MiB，不產生預覽。' }
        $previewMemory = New-Object IO.MemoryStream
        $previewBuffer = New-Object byte[] 65536
        while (($previewRead = $previewSource.Read($previewBuffer, 0, $previewBuffer.Length)) -gt 0) {
            if (($previewMemory.Length + $previewRead) -gt 67108864) { throw '檔案超過 64 MiB，不產生預覽。' }
            $previewMemory.Write($previewBuffer, 0, $previewRead)
        }
    } finally { $previewSource.Dispose() }
    $previewMemory.Position = 0
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    [Windows.Data.Pdf.PdfDocument,Windows.Data.Pdf,ContentType=WindowsRuntime] | Out-Null
    [Windows.Data.Pdf.PdfPageRenderOptions,Windows.Data.Pdf,ContentType=WindowsRuntime] | Out-Null
    [Windows.Storage.Streams.InMemoryRandomAccessStream,Windows.Storage.Streams,ContentType=WindowsRuntime] | Out-Null
    $previewAsTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and $_.GetParameters().Length -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
    $previewRandom = [IO.WindowsRuntimeStreamExtensions]::AsRandomAccessStream($previewMemory)
    $previewOperation = [Windows.Data.Pdf.PdfDocument]::LoadFromStreamAsync($previewRandom)
    $previewTask = $previewAsTask.MakeGenericMethod([Windows.Data.Pdf.PdfDocument]).Invoke($null, @($previewOperation))
    $previewDocument = $previewTask.GetAwaiter().GetResult()
    if ($previewDocument.PageCount -lt 1) { throw 'PDF 沒有可顯示的頁面。' }
    [Console]::Out.Write([string]$previewDocument.PageCount)
    $previewPage = $previewDocument.GetPage(0)
    $previewSize = $previewPage.Size
    if ($previewSize.Width -le 0 -or $previewSize.Height -le 0) { throw 'PDF 頁面尺寸無效。' }
    $previewEdge = 640.0
    [double]::TryParse($env:FEATHER_PREVIEW_EDGE, [ref]$previewEdge) | Out-Null
    $previewScale = [Math]::Min(1280.0, [Math]::Max(64.0, $previewEdge)) / [Math]::Max($previewSize.Width, $previewSize.Height)
    $previewOptions = New-Object Windows.Data.Pdf.PdfPageRenderOptions
    $previewOptions.DestinationWidth = [uint32][Math]::Max(1, [Math]::Round($previewSize.Width * $previewScale))
    $previewOptions.DestinationHeight = [uint32][Math]::Max(1, [Math]::Round($previewSize.Height * $previewScale))
    $previewOutput = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
    $previewAsAction = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and -not $_.IsGenericMethod -and $_.GetParameters().Length -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' } | Select-Object -First 1
    $previewRenderTask = $previewAsAction.Invoke($null, @($previewPage.RenderToStreamAsync($previewOutput, $previewOptions)))
    $previewRenderTask.GetAwaiter().GetResult()
    $previewOutput.Seek(0)
    $previewManaged = [IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($previewOutput)
    $previewTarget = [IO.File]::Create($env:FEATHER_PREVIEW_OUTPUT)
    try { $previewManaged.CopyTo($previewTarget) } finally { $previewTarget.Dispose() }
} catch {
    [Console]::Error.WriteLine($_.Exception.GetBaseException().Message)
    exit 2
} finally {
    if ($null -ne $previewManaged) { $previewManaged.Dispose() }
    if ($null -ne $previewPage) { $previewPage.Dispose() }
    if ($null -ne $previewOutput) { $previewOutput.Dispose() }
    if ($null -ne $previewRandom) { $previewRandom.Dispose() }
    if ($null -ne $previewMemory) { $previewMemory.Dispose() }
}
"#;

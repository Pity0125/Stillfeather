//! 主視窗：左側全高的側欄卡片（程式名稱、資料夾樹、個人化提示卡），
//! 右側依序是麵包屑與搜尋膠囊、大標題與摘要、底線分頁、類型篩選籤、檔案清單卡片、浮動狀態列。
//! 這裡也處理分頁與雙欄、鍵盤操作、命令與背景工作結果。

use super::anim;
use super::chips;
use super::controls::{self, Kind, Surface};
use super::crumbs;
use super::dialog::{self, Icon};
use super::dragdrop;
use super::fileops::{self, OpReport};
use super::fs::{self, FolderListing, FsError, Listing, SearchSummary};
use super::icons::{self, Icons};
use super::instance;
use super::preview;
use super::settings_dialog;
use super::shellreg;
use super::sys::*;
use super::tabs;
use super::theme::{self, glyph, Theme};
use super::util::*;
use feather_core::anim::{RowFx, Tween};
use feather_core::category::{self, filter_rows, Category, Summary};
use feather_core::entry::Entry;
use feather_core::folderviews::{self, FolderViews};
use feather_core::format::{format_size, group_digits};
use feather_core::install;
use feather_core::paths;
use feather_core::plan::OpKind;
use feather_core::session::{Session, SideTabs};
use feather_core::typeahead::TypeAhead;
use feather_core::settings::{Settings, ThemeMode, SIDEBAR_MAX, SIDEBAR_MIN, SPLIT_MAX, SPLIT_MIN};
use feather_core::view::ViewMode;
use feather_core::workspace::{CloseResult, DualChange, Side, TabId, Workspace};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, Ordering};
use std::sync::Arc;

pub const WM_APP_WORKER: u32 = WM_APP + 1;
pub const WM_APP_OPEN: u32 = WM_APP + 2;
pub const WM_APP_TREE_RESET: u32 = WM_APP + 3;
pub const WM_APP_RENAME: u32 = WM_APP + 4;
/// 搜尋框得到或失去焦點後，焦點轉移完成時再決定要不要收合。
pub const WM_APP_SEARCH: u32 = WM_APP + 5;
pub const TIMER_DRIVES: usize = 2;
/// 執行 Windows 右鍵選單的命令後，稍後再重新整理一次（有些命令在背景完成）。
pub const TIMER_SHELL_REFRESH: usize = 3;
/// 個人化卡片高度動畫
pub const TIMER_PROMO: usize = 4;
/// 搜尋按鈕展開／收合動畫
pub const TIMER_SEARCH: usize = 5;

pub mod id {
    pub const BACK: u16 = 100;
    pub const FORWARD: u16 = 101;
    pub const UP: u16 = 102;
    pub const REFRESH: u16 = 103;
    pub const ADDRESS: u16 = 104;
    pub const QUERY: u16 = 105;
    pub const CLEAR: u16 = 108;
    pub const SEARCH: u16 = 109;
    pub const NEW_FOLDER: u16 = 120;
    pub const CUT: u16 = 121;
    pub const COPY: u16 = 122;
    pub const PASTE: u16 = 123;
    pub const RENAME: u16 = 124;
    pub const DELETE: u16 = 125;
    pub const DUAL: u16 = 126;
    pub const COPY_ACROSS: u16 = 127;
    pub const HIDDEN: u16 = 128;
    pub const PROPERTIES: u16 = 129;
    pub const SETTINGS: u16 = 130;
    pub const HELP: u16 = 131;
    pub const THEME: u16 = 132;
    /// 標題列右側的「排序」「檢視」下拉選單按鈕
    pub const SORT_MENU: u16 = 133;
    pub const VIEW_MENU: u16 = 134;
    /// 復原（Ctrl+Z）
    pub const UNDO: u16 = 135;
    /// 快速預覽（空白鍵）
    pub const QUICKLOOK: u16 = 136;
    /// 搜尋膠囊裡的「全域搜尋」勾選
    pub const SEARCH_GLOBAL: u16 = 137;
    pub const LIST_LEFT: u16 = 150;
    pub const LIST_RIGHT: u16 = 151;
    pub const TREE: u16 = 152;
    pub const OPEN: u16 = 200;
    pub const OPEN_NEW_TAB: u16 = 201;
    pub const OPEN_LOCATION: u16 = 202;
    pub const COPY_PATH: u16 = 203;
    pub const ADD_FAVORITE: u16 = 204;
    pub const REMOVE_FAVORITE: u16 = 205;
    pub const SELECT_ALL: u16 = 206;
    /// 拖放到側欄「常用位置」後，稍後加入（見 dragdrop.rs）
    pub const PIN_DROPPED: u16 = 207;
    /// 右鍵選單的「在這裡開啟 PowerShell 視窗」
    pub const OPEN_POWERSHELL: u16 = 208;
    pub const TAB_NEW: u16 = 220;
    pub const TAB_DUPLICATE: u16 = 221;
    pub const TAB_MOVE_LEFT: u16 = 222;
    pub const TAB_MOVE_RIGHT: u16 = 223;
    pub const TAB_CLOSE: u16 = 224;
    /// 檢視模式：VIEW_FIRST + 0～7（順序同 feather_core::view::ALL）
    pub const VIEW_FIRST: u16 = 240;
    pub const SORT_NAME: u16 = 250;
    pub const SORT_SIZE: u16 = 251;
    pub const SORT_TYPE: u16 = 252;
    pub const SORT_DATE: u16 = 253;
    pub const SORT_LOCATION: u16 = 254;
    pub const SORT_ASC: u16 = 255;
    pub const SORT_DESC: u16 = 256;
    /// 排序選單的「資料夾一律排在最前面」
    pub const SORT_FOLDERS_ON_TOP: u16 = 257;
    pub const SHOW_COMPACT: u16 = 260;
    pub const SHOW_EXTENSIONS: u16 = 261;
    /// 把目前的檢視設為所有資料夾的檢視（清掉個別資料夾的記錄）
    pub const VIEW_APPLY_ALL: u16 = 262;
    /// 「檢視 › 顯示 › 相片資料夾用大圖示」
    pub const SHOW_MEDIA_ICONS: u16 = 263;
    /// 選擇器（見 picker.rs）：確定、取消、改用 Windows 視窗、檔案類型選單、檔名欄
    pub const PICK_OK: u16 = 270;
    pub const PICK_CANCEL: u16 = 271;
    pub const PICK_SYSTEM: u16 = 272;
    pub const PICK_TYPE: u16 = 273;
    pub const PICK_NAME: u16 = 274;
    /// 檔案類型選單的項目：PICK_TYPE_FIRST + 第幾個類型
    pub const PICK_TYPE_FIRST: u16 = 300;
}

// ---------------------------------------------------------------- 全域存取

static APP: AtomicPtr<App> = AtomicPtr::new(std::ptr::null_mut());
/// 第一個資料夾已經顯示（啟動時間只記一次）
static FIRST_FOLDER_SHOWN: AtomicBool = AtomicBool::new(false);
static MAIN: AtomicIsize = AtomicIsize::new(0);

/// UI 執行緒專用。Win32 訊息會重入，所以以原始指標保存唯一的 App。
pub(super) fn app_opt() -> Option<&'static mut App> {
    unsafe { APP.load(Ordering::Acquire).as_mut() }
}

pub(super) fn app() -> &'static mut App {
    app_opt().expect("App 尚未建立")
}

pub fn main_hwnd() -> HWND {
    MAIN.load(Ordering::Acquire)
}

pub fn theme() -> &'static Theme {
    &app().theme
}

/// 主視窗建立前（例如啟動失敗）沒有主題可用。
pub fn theme_opt() -> Option<&'static Theme> {
    app_opt().map(|a| &a.theme)
}

/// 資料夾清單的狀態文字（項目數、無法讀取的項目數）。
pub(super) fn folder_status(count: usize, skipped: usize) -> String {
    trn!(count; "{} 個項目", "{} item", "{} items", group_digits(count as u64)) + &if skipped > 0 { trf!("  ·  {} 個無法讀取", "  ·  {} unreadable", skipped) } else { String::new() }
}

/// 背景工作把結果交回 UI 執行緒。
pub enum WorkerMsg {
    Dir { tab: TabId, generation: u64, path: String, kind: NavKind, result: Result<Listing, FsError> },
    /// [`App::navigate_or_reveal`] 要開的位置其實是檔案：改開它所在的資料夾並選取它
    NotFolder { tab: TabId, generation: u64, path: String, kind: NavKind },
    /// 自動重新整理的重新讀取（見 watch.rs）；`gone_to`：資料夾已不在時，還在的上一層
    AutoDir { tab: TabId, generation: u64, path: String, result: Result<Listing, FsError>, gone_to: Option<String> },
    SearchBatch { tab: TabId, generation: u64, entries: Vec<Entry>, scanned: usize, skipped: usize },
    SearchDone { tab: TabId, generation: u64, result: Result<SearchSummary, FsError> },
    Tree { item: HTREEITEM, token: u64, result: Result<FolderListing, FsError> },
    Preview { generation: u64, path: String, result: Result<preview::Loaded, FsError> },
    FileOp(OpReport),
    Update(super::update::Event),
}

pub fn post(msg: WorkerMsg) {
    let hwnd = main_hwnd();
    let p = Box::into_raw(Box::new(msg));
    unsafe {
        if hwnd == 0 || PostMessageW(hwnd, WM_APP_WORKER, 0, p as LPARAM) == 0 {
            drop(Box::from_raw(p));
        }
    }
}

pub fn tab_model(side: Side) -> tabs::Model {
    let a = app();
    let g = a.ws.group(side);
    tabs::Model {
        titles: g.tabs.iter().map(|id| a.tabs.get(id).map(|t| paths::tab_title(&t.display_path())).unwrap_or_default()).collect(),
        selected: g.selected,
        active: a.active_side() == side,
    }
}

pub fn chip_model() -> chips::Model {
    match app().active_tab() {
        Some(t) => chips::Model {
            counts: t.summary.counts,
            filter: t.filter,
            sort: chips::Sort::from_column(t.sort_col, t.desc),
            recursive: t.recursive,
        },
        None => chips::Model { counts: [0; category::COUNT], filter: Category::All, sort: Some(chips::Sort::Name), recursive: true },
    }
}

pub fn crumb_model() -> Vec<(String, String)> {
    app().active_tab().map(|t| paths::segments(&t.display_path())).unwrap_or_default()
}

// ---------------------------------------------------------------- 分頁狀態

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavKind {
    Normal,
    NoHistory,
    Back,
    Forward,
}

#[derive(Default)]
pub(super) struct ViewState {
    pub selected: Vec<usize>,
    pub focused: i32,
    pub top: i32,
}

pub(super) struct Tab {
    pub path: String,
    pub requested: String,
    pub back: Vec<String>,
    pub forward: Vec<String>,
    /// 目前資料夾（或搜尋結果）的全部項目
    pub entries: Vec<Entry>,
    /// 套用類型篩選後顯示的列；None 表示全部顯示
    pub rows: Option<Vec<u32>>,
    pub filter: Category,
    pub summary: Summary,
    pub sort_col: usize,
    pub desc: bool,
    pub search_mode: bool,
    pub searching: bool,
    pub loading: bool,
    pub query: String,
    pub recursive: bool,
    /// 這次搜尋是整台電腦（全域搜尋）
    pub search_global: bool,
    pub status: String,
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
    pub view: ViewState,
    pub pending_select: Vec<String>,
    pub pending_rename: bool,
    /// 隱藏中的分頁設定變更後需要重新讀取（切回時才讀，背景分頁不掃描）。
    pub stale: bool,
    /// 這次讀取資料夾或搜尋開始的時間（寫進 trace.log 供效能量測）
    pub work_started: Option<std::time::Instant>,
    /// 選擇器的檔案類型篩選（一般視窗是 None）
    pub pick: Option<feather_core::picker::Filter>,
    /// 相片資料夾（圖片與影片佔一半以上）：沒記住檢視時用大圖示。換資料夾時判斷一次，自動重新整理不會讓檢視跳來跳去
    pub media: bool,
    /// 還原的分頁還沒讀取：第一次顯示時才讀（`requested` 是要開的資料夾）
    pub restored: bool,
}

impl Tab {
    fn new() -> Tab {
        Tab {
            path: String::new(),
            requested: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            rows: None,
            filter: Category::All,
            summary: Summary::default(),
            sort_col: 0,
            desc: false,
            search_mode: false,
            searching: false,
            loading: false,
            query: String::new(),
            recursive: true,
            search_global: false,
            status: tr!("選擇資料夾開始瀏覽", "Select a folder to start browsing").into(),
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            view: ViewState::default(),
            pending_select: Vec::new(),
            pending_rename: false,
            stale: false,
            work_started: None,
            pick: None,
            media: false,
            restored: false,
        }
    }

    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            self.requested.clone()
        } else {
            self.path.clone()
        }
    }

    pub fn working(&self) -> bool {
        self.loading || self.searching
    }

    /// 清單上顯示的列數（已套用篩選）。
    pub fn row_count(&self) -> usize {
        self.rows.as_ref().map(Vec::len).unwrap_or(self.entries.len())
    }

    /// 第 i 個顯示列對應的項目。
    pub fn row(&self, i: usize) -> Option<&Entry> {
        match &self.rows {
            Some(r) => r.get(i).and_then(|j| self.entries.get(*j as usize)),
            None => self.entries.get(i),
        }
    }

    pub fn refilter(&mut self) {
        self.rows = match &self.pick {
            Some(pick) if !pick.is_all() => {
                Some(self.entries.iter().enumerate().filter(|(_, e)| self.filter.matches(e) && pick.matches(&e.name, e.is_dir)).map(|(i, _)| i as u32).collect())
            }
            _ => filter_rows(&self.entries, self.filter),
        };
    }

    /// 這個項目在清單上顯示嗎（類型篩選籤，以及選擇器的檔案類型）
    pub fn shows(&self, e: &Entry) -> bool {
        self.filter.matches(e) && self.pick.as_ref().is_none_or(|p| p.matches(&e.name, e.is_dir))
    }

    pub fn resummarize(&mut self) {
        self.summary = Summary::of(&self.entries);
    }

    /// 取消目前工作並開始新的一輪；舊結果會因世代不符而被忽略。
    pub(super) fn begin_work(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        (self.generation, self.cancel.clone())
    }
}

// ---------------------------------------------------------------- 樹狀節點

struct PromoGeometry {
    promo: Option<RECT>,
    toggle: Option<RECT>,
    button: RECT,
    button_visible: bool,
    tree_bottom: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NodeKind {
    Header,
    Favorite { custom: bool },
    Drive,
    Folder,
    Info,
}

pub(super) struct Node {
    pub path: String,
    pub kind: NodeKind,
    pub loaded: bool,
    pub token: u64,
}

// ---------------------------------------------------------------- 布局

#[derive(Default, Clone)]
pub(super) struct Layout {
    /// 左側全高的側欄卡片
    pub sidebar: RECT,
    pub brand: RECT,
    pub promo: Option<RECT>,
    /// 個人化卡片的收合／展開感應範圍（展開時是右上角的小圓鈕，收合時是整張卡片）
    pub promo_toggle: Option<RECT>,
    pub sidebar_splitter: RECT,
    pub content_left: i32,
    pub content_right: i32,
    /// 第一列中段：路徑列與搜尋共用的範圍
    pub top_row: RECT,
    pub crumbs: RECT,
    /// 搜尋膠囊目前的外框（收合時是圓形按鈕的位置）
    pub search: RECT,
    pub title: RECT,
    pub subtitle: RECT,
    pub strips: [RECT; 2],
    pub chips: RECT,
    /// 檔案清單卡片
    pub panes: [RECT; 2],
    pub pane_splitter: Option<RECT>,
    /// 底部浮動狀態列
    pub footer: RECT,
}

/// 第一列中段的計算結果：路徑列、搜尋外框，以及要搬動的子視窗。
struct TopRow {
    crumbs: RECT,
    search: RECT,
    windows: Vec<(HWND, RECT, bool)>,
}

/// 一次搬動多個子視窗（同時決定顯示或隱藏）。
fn move_windows(pos: &[(HWND, RECT, bool)]) {
    unsafe {
        let mut hdwp = BeginDeferWindowPos(pos.len() as i32);
        for (hwnd, r, visible) in pos {
            if *hwnd == 0 {
                continue;
            }
            let flags = SWP_NOZORDER | SWP_NOACTIVATE | if *visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
            if hdwp != 0 {
                hdwp = DeferWindowPos(hdwp, *hwnd, 0, r.left, r.top, r.width().max(0), r.height().max(0), flags);
            } else {
                SetWindowPos(*hwnd, 0, r.left, r.top, r.width().max(0), r.height().max(0), flags);
            }
        }
        if hdwp != 0 {
            EndDeferWindowPos(hdwp);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    Sidebar,
    Split,
}

/// 浮動狀態列左側：選取項目的圖示、名稱與細節。
#[derive(Default)]
struct FooterInfo {
    icon: i32,
    title: String,
    detail: String,
}

// ---------------------------------------------------------------- App

pub struct App {
    pub hwnd: HWND,
    pub theme: Theme,
    pub settings: Settings,
    pub(super) ws: Workspace,
    pub(super) tabs: HashMap<TabId, Tab>,
    pub(super) bound: [Option<TabId>; 2],
    pub(super) lists: [HWND; 2],
    pub(super) pane_status: [HWND; 2],
    pub(super) strips: [HWND; 2],
    pub(super) crumbs: HWND,
    chips: HWND,
    pub(super) tree: HWND,
    pub(super) address: HWND,
    pub(super) query: HWND,
    back_btn: HWND,
    forward_btn: HWND,
    up_btn: HWND,
    refresh_btn: HWND,
    clear_btn: HWND,
    /// 搜尋膠囊裡的「全域搜尋」勾選方塊
    global_btn: HWND,
    /// 全域搜尋是否勾選（新的搜尋都套用）
    search_global: bool,
    /// 收合時代表搜尋的圓形按鈕
    search_btn: HWND,
    /// 右上角的圓形按鈕（雙欄、隱藏項目、深淺色、設定）
    top_buttons: Vec<HWND>,
    /// 標題列右側的動作（新增資料夾、剪下、複製…）
    action_buttons: Vec<HWND>,
    dual_btn: HWND,
    theme_btn: HWND,
    /// 「排序」「檢視」下拉選單按鈕
    pub(super) sort_btn: HWND,
    pub(super) view_btn: HWND,
    help_btn: HWND,
    promo_btn: HWND,
    /// 側欄標頭的羽毛圖示與其邊長（DPI 改變時重建）
    brand_icon: HICON,
    brand_icon_px: i32,
    /// 側欄標頭的草書字樣（圖示）、它的寬高，以及產生時的（可用寬, 可用高, 墨色）
    wordmark: (HICON, i32, i32),
    wordmark_key: (i32, i32, u32),
    tooltip: HWND,
    pub(super) icons: Icons,
    pub(super) nodes: HashMap<HTREEITEM, Node>,
    pub(super) node_token: u64,
    pub(super) show_hidden: bool,
    /// 每個資料夾記住的檢視模式
    pub(super) views: FolderViews,
    /// 兩欄目前套用的檢視模式（依各自顯示的資料夾）
    pub(super) view: [ViewMode; 2],
    pub(super) op_busy: bool,
    footer: String,
    footer_sticky: bool,
    footer_info: FooterInfo,
    pub(super) layout: Layout,
    drag: Option<Drag>,
    pub(super) preview: preview::State,
    /// 更新提醒（上次檢查、略過的版本、等待提醒的新版本）
    pub(super) updates: super::update::Ui,
    /// 滑鼠停在清單項目上的預覽
    pub(super) hover: super::pane::Hover,
    pub(super) default_path: String,
    suppress_query_change: bool,
    pub(super) updating_list: bool,
    /// 視窗失去作用時記住焦點，切回來時還原（一般視窗沒有對話方塊管理員代勞）。
    last_focus: HWND,
    /// 路徑列目前是可輸入的文字框（否則顯示麵包屑）。
    address_editing: bool,
    /// 在清單或樹狀圖打字跳到項目（取代內建搜尋，找不到時不發出提示音）。
    typeahead: TypeAhead,
    /// 清單列的選取與懸停動畫；捲動位置變了就不替新出現的列播放動畫。
    pub(super) list_fx: [RowFx<usize>; 2],
    pub(super) list_top: [isize; 2],
    pub(super) tree_fx: RowFx<HTREEITEM>,
    pub(super) tree_top: HTREEITEM,
    /// 底部狀態列內容更新時淡入
    footer_fx: Tween,
    footer_changed: f64,
    /// 個人化卡片：1 = 展開、0 = 收合；懸停在收合鈕上的程度
    promo_fx: Tween,
    promo_hover: Tween,
    promo_hot: bool,
    mouse_tracked: bool,
    /// 拖放中標示的放置位置：清單的資料夾列、整欄、側欄「常用位置」
    pub(super) drop_row: Option<(Side, usize)>,
    pub(super) drop_pane: Option<Side>,
    pub(super) drop_pin: bool,
    pub(super) pending_pins: Vec<String>,
    /// 從側欄刪除的資料夾節點：刪除完成後若資料夾已不在就移除節點
    pub(super) pending_tree_delete: Option<(HTREEITEM, String)>,
    /// 復原紀錄（Ctrl+Z）
    pub(super) history: feather_core::undo::History,
    /// 進行中的移動／刪除：完成後依結果寫入復原紀錄
    pub(super) op_pending: Option<super::undo::Pending>,
    /// 復原完成後要選取的項目（還原到的位置）
    pub(super) undo_select: Vec<String>,
    /// 正在就地重新命名的項目（開始編輯時記下完整路徑；清單在編輯中被重新整理或重排時，
    /// 列號會指到別的項目，結束時一律以這裡記下的路徑為準）
    pub(super) renaming: Option<(TabId, String)>,
    /// 由「剪下」貼上的檔案：移動完成後清空剪貼簿（與檔案總管相同），避免再貼一次全部失敗
    pub(super) paste_cut: Option<Vec<String>>,
    /// 刪除完成後要選取的項目（被刪項目的下一個，鍵盤可以接著操作）
    pub(super) after_delete: Option<(TabId, String)>,
    /// 進行中的移動／刪除的來源（完成後把搜尋結果裡已不在的項目拿掉）
    op_sources: Vec<String>,
    /// 搜尋框取得焦點時外框淡入重點色
    search_fx: Tween,
    /// 搜尋的展開程度：0 = 圓形按鈕、1 = 膠囊輸入框
    search_open: Tween,
    /// 自動重新整理：顯示中資料夾的變更通知與重新讀取排程
    pub(super) watch: super::watch::Watches,
    /// 清單縮圖（見 thumbs.rs）
    pub(super) thumbs: super::thumbs::Thumbs,
    /// 選擇器模式（其他程式要開啟檔案時）；一般視窗是 None
    pub(super) picker: Option<super::picker::Picker>,
}

pub fn settings_path() -> Result<String, String> {
    if let Ok(custom) = std::env::var("FEATHER_SETTINGS_PATH") {
        if !custom.trim().is_empty() {
            if !paths::is_fully_qualified(&custom) {
                return Err(tr!("設定檔位置必須是完整路徑。", "The settings file location must be a full path.").into());
            }
            return Ok(custom);
        }
    }
    let base = std::env::var("LOCALAPPDATA").map_err(|_| tr!("找不到 %LOCALAPPDATA%。", "Can't find %LOCALAPPDATA%.").to_string())?;
    Ok(paths::join(&feather_core::install::data_dir(&base), "settings.json"))
}

/// 啟動時（建立任何視窗之前）依設定的語言切換介面語言。讀不到設定檔時跟隨 Windows 的顯示語言。
pub fn apply_language() {
    let language = settings_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| Settings::from_json(&t).ok()).map(|s| s.language).unwrap_or_default();
    feather_core::i18n::set_english(language.english(system_ui_chinese()));
}

fn load_settings() -> (Settings, Option<String>) {
    let path = match settings_path() {
        Ok(p) => p,
        Err(e) => return (Settings::default(), Some(e)),
    };
    let error = match std::fs::read_to_string(&path) {
        Ok(text) => match Settings::from_json(&text) {
            Ok(s) => return (s, None),
            Err(e) => e,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => e.to_string(),
    };
    // 讀不懂的設定檔先備份：之後任何一次儲存都會用預設值覆寫它，常用位置等內容就找不回來了。
    let backup = format!("{path}.bak");
    let note = match std::fs::copy(&path, &backup) {
        Ok(_) => trf!("（原檔已備份為 {}）", " (the original file was backed up as {})", paths::file_name(&backup)),
        Err(_) => String::new(),
    };
    (Settings::default(), Some(trf!("設定讀取失敗，使用預設值{note}：{error}", "Couldn't read the settings, so the defaults are used{note}: {error}")))
}

/// 先寫入同資料夾的暫存檔並清空緩衝，再以改名取代，避免寫到一半損毀舊檔。
pub(super) fn write_atomic(path: &str, text: &str) -> Result<(), String> {
    use std::io::Write;
    let dir = paths::parent(path).ok_or(tr!("檔案位置無效。", "Invalid file location."))?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = paths::join(&dir, &format!(".feather-{}-{}.tmp", std::process::id(), unsafe { GetTickCount64() }));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

pub fn write_settings(settings: &Settings) -> Result<(), String> {
    // 選擇器是另一個程序：改了檢視、隱藏項目等只在這次有效，不寫回設定檔（主程式可能同時開著）
    if super::picker::active() {
        return Ok(());
    }
    let mut copy = settings.clone();
    copy.validate()?;
    write_atomic(&settings_path()?, &copy.to_json())
}

/// 上次關閉時的分頁（見 feather_core::session）：和設定檔放在同一個資料夾
fn session_path() -> Result<String, String> {
    let settings = settings_path()?;
    let dir = paths::parent(&settings).ok_or(tr!("設定檔位置無效。", "Invalid settings file location."))?;
    Ok(paths::join(&dir, feather_core::session::FILE_NAME))
}

/// 讀出上次的分頁並刪掉記錄：這次正常關閉時會再寫，當掉的話下次就不會再開回可能讓它當掉的分頁。
/// 本機硬碟上已經不在的資料夾不還原（網路與抽取式磁碟不檢查，避免啟動時等它們回應）。
fn take_session() -> Option<Session> {
    let path = session_path().ok()?;
    let text = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(&path);
    let mut session = Session::from_json(&text?).ok()?;
    for side in [&mut session.left, &mut session.right] {
        let selected = side.tabs.get(side.selected).cloned();
        side.tabs.retain(|p| !on_fixed_drive(p) || fs::is_dir(p));
        side.selected = selected.and_then(|s| side.tabs.iter().position(|p| *p == s)).unwrap_or(0);
    }
    let session = session.sanitize();
    (!session.is_empty()).then_some(session)
}

/// 本機硬碟（不是網路、抽取式或光碟）上的路徑
fn on_fixed_drive(path: &str) -> bool {
    let b = path.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return false;
    }
    let root = wide(&path[..3]);
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_FIXED }
}

/// 每個資料夾的檢視記錄：和設定檔放在同一個資料夾（Rust 版專用，C# 版不讀）。
pub fn folder_views_path() -> Result<String, String> {
    let settings = settings_path()?;
    let dir = paths::parent(&settings).ok_or(tr!("設定檔位置無效。", "Invalid settings file location."))?;
    Ok(paths::join(&dir, "folder-views.json"))
}

fn load_folder_views() -> (FolderViews, Option<String>) {
    let Ok(path) = folder_views_path() else { return (FolderViews::default(), None) };
    match std::fs::metadata(&path) {
        Ok(m) if m.len() > folderviews::MAX_FILE_BYTES => return (FolderViews::default(), Some(tr!("資料夾檢視記錄太大，已改用預設檢視。", "The folder view record is too large, so the default views are used.").into())),
        Err(_) => return (FolderViews::default(), None),
        Ok(_) => {}
    }
    match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| FolderViews::from_json(&t)) {
        Ok(v) => (v, None),
        Err(e) => (FolderViews::default(), Some(trf!("資料夾檢視記錄讀取失敗，改用預設檢視：{e}", "Couldn't read the folder view record, so the default views are used: {e}"))),
    }
}

pub fn write_folder_views(views: &FolderViews) -> Result<(), String> {
    if super::picker::active() {
        return Ok(());
    }
    write_atomic(&folder_views_path()?, &views.to_json())
}

/// 建立主視窗並執行訊息迴圈。`pick` 有值時是選擇器模式（見 picker.rs）：精簡的介面、蓋在要求的程式上，
/// 回傳值是交給取代元件的結束代碼。
pub(super) fn run(paths_in: Vec<String>, pick: Option<super::picker::Picker>) -> i32 {
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    // 選擇器用不同的視窗類別：單一實例與安裝程式找主視窗時不會找到它
    let class_name = if pick.is_some() { format!("{}.Picker", instance::main_class()) } else { instance::main_class() };
    let class = wide(&class_name);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: main_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: icons::app_icon(unsafe { GetSystemMetrics(SM_CXICON) }),
        hCursor: load_cursor(IDC_ARROW),
        hbrBackground: 0,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: icons::app_icon(unsafe { GetSystemMetrics(SM_CXSMICON) }),
    };
    unsafe {
        RegisterClassExW(&wc);
    }
    controls::register(hinst);
    anim::register(hinst);
    dialog::register(hinst);
    tabs::register(hinst);
    chips::register(hinst);
    crumbs::register(hinst);
    preview::register(hinst);
    settings_dialog::register(hinst);

    let (settings, settings_error) = load_settings();
    let promo_collapsed = settings.personalize_collapsed;
    let dpi = unsafe { GetDpiForSystem() };
    let theme = Theme::new(&settings, dpi);
    theme::apply_app_mode(theme.pal.dark);

    // 選擇器：蓋在要求的程式的視窗上（它的視窗是擁有者），置中在那個視窗上
    let owner = pick.as_ref().map(|p| p.req.owner).filter(|o| *o != 0 && unsafe { IsWindow(*o) } != 0).unwrap_or(0);
    // 視窗大小：預設 1280×800（選擇器 1100×720，依 DPI 縮放），不超過游標（或擁有者）所在螢幕的工作區。
    let (x, y, w, h) = unsafe {
        let mut or = RECT::default();
        let anchor = if owner != 0 && GetWindowRect(owner, &mut or) != 0 { POINT { x: or.left + or.width() / 2, y: or.top + or.height() / 2 } } else { cursor_pos() };
        let mon = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let (base_w, base_h) = if pick.is_some() { (1100, 720) } else { (1280, 800) };
        let w = scale(base_w, dpi).min(work.width() - 40).max(400);
        let h = scale(base_h, dpi).min(work.height() - 40).max(300);
        let x = (anchor.x - w / 2).clamp(work.left, (work.right - w).max(work.left));
        let y = (anchor.y - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
        (x, y, w, h)
    };
    let title = wide(&pick.as_ref().map(|p| p.req.window_title()).unwrap_or_else(|| "Stillfeather".into()));
    // 選擇器像對話方塊：沒有最小化（擁有者在它關閉前不能操作）
    let style = if pick.is_some() { (WS_OVERLAPPEDWINDOW & !WS_MINIMIZEBOX) | WS_CLIPCHILDREN } else { WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN };
    let hwnd = unsafe { CreateWindowExW(WS_EX_CONTROLPARENT, class.as_ptr(), title.as_ptr(), style, x, y, w, h, owner, 0, hinst, std::ptr::null()) };
    if hwnd == 0 {
        message_box(0, &trf!("無法建立主視窗：{}", "Couldn't create the main window: {}", last_error_text()), MB_OK | MB_ICONERROR);
        return 1;
    }
    MAIN.store(hwnd, Ordering::Release);
    let real_dpi = unsafe { GetDpiForWindow(hwnd) };
    let mut theme = theme;
    if real_dpi != dpi && real_dpi != 0 {
        theme.set_dpi(real_dpi);
    }
    theme::style_caption(hwnd, &theme.pal, theme.pal.background);
    // 依視窗所在螢幕的縮放比例設定清晰的圖示（類別圖示是以系統 DPI 建立的）
    unsafe {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG, icons::app_icon(scale(32, theme.dpi)) as LPARAM);
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL, icons::app_icon(scale(16, theme.dpi)) as LPARAM);
    }

    let (ws, first) = Workspace::new();
    let mut tabs_map = HashMap::new();
    tabs_map.insert(first, Tab::new());
    let icons = Icons::new(theme.dpi, settings.compact_rows);
    // 程式要求一律顯示隱藏的項目（FOS_FORCESHOWHIDDEN）
    let show_hidden = settings.show_hidden || pick.as_ref().is_some_and(|p| p.req.show_hidden);
    let default_view = settings.view_mode;
    let (views, views_error) = load_folder_views();
    let boxed = Box::new(App {
        hwnd,
        theme,
        settings,
        ws,
        tabs: tabs_map,
        bound: [None, None],
        lists: [0, 0],
        pane_status: [0, 0],
        strips: [0, 0],
        crumbs: 0,
        chips: 0,
        tree: 0,
        address: 0,
        query: 0,
        back_btn: 0,
        forward_btn: 0,
        up_btn: 0,
        refresh_btn: 0,
        clear_btn: 0,
        global_btn: 0,
        search_global: false,
        search_btn: 0,
        top_buttons: Vec::new(),
        action_buttons: Vec::new(),
        dual_btn: 0,
        sort_btn: 0,
        view_btn: 0,
        theme_btn: 0,
        help_btn: 0,
        promo_btn: 0,
        brand_icon: 0,
        brand_icon_px: 0,
        wordmark: (0, 0, 0),
        wordmark_key: (0, 0, 0),
        tooltip: 0,
        icons,
        nodes: HashMap::new(),
        node_token: 0,
        show_hidden,
        views,
        view: [default_view; 2],
        op_busy: false,
        footer: tr!("就緒", "Ready").into(),
        footer_sticky: false,
        footer_info: FooterInfo::default(),
        layout: Layout::default(),
        drag: None,
        preview: preview::State::new(),
        updates: super::update::Ui::load(),
        hover: Default::default(),
        default_path: fs::profile_dir(),
        suppress_query_change: false,
        updating_list: false,
        last_focus: 0,
        address_editing: false,
        typeahead: TypeAhead::default(),
        list_fx: [RowFx::default(), RowFx::default()],
        list_top: [-1, -1],
        tree_fx: RowFx::default(),
        tree_top: 0,
        footer_fx: Tween::new(1.0),
        footer_changed: 0.0,
        promo_fx: Tween::new(if promo_collapsed { 0.0 } else { 1.0 }),
        promo_hover: Tween::new(0.0),
        promo_hot: false,
        mouse_tracked: false,
        drop_row: None,
        drop_pane: None,
        drop_pin: false,
        pending_pins: Vec::new(),
        pending_tree_delete: None,
        history: feather_core::undo::History::default(),
        op_pending: None,
        undo_select: Vec::new(),
        renaming: None,
        paste_cut: None,
        after_delete: None,
        op_sources: Vec::new(),
        search_fx: Tween::new(0.0),
        search_open: Tween::new(0.0),
        watch: Default::default(),
        thumbs: Default::default(),
        picker: pick,
    });
    APP.store(Box::into_raw(boxed), Ordering::Release);
    let a = app();
    a.create_children();
    a.create_picker_controls();
    dragdrop::register(a.lists[0], dragdrop::DropWindow::List(Side::Left));
    dragdrop::register(a.lists[1], dragdrop::DropWindow::List(Side::Right));
    dragdrop::register(a.tree, dragdrop::DropWindow::Tree);
    dragdrop::register(a.crumbs, dragdrop::DropWindow::Crumbs);
    dragdrop::register(a.strips[0], dragdrop::DropWindow::Tabs(Side::Left));
    dragdrop::register(a.strips[1], dragdrop::DropWindow::Tabs(Side::Right));
    a.apply_view();
    a.sync_label_tips();
    a.apply_theme_to_controls();
    a.build_tree();
    a.relayout();
    a.bind(Side::Left);
    // 還原上次的分頁（設定開著時；選擇器不還原）：在顯示視窗之前，畫面一出現就是上次的樣子
    let restored = !a.picker_mode() && a.restore_session();
    a.update_chrome();
    unsafe {
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);
    }
    super::crashlog::crumb_value("效能：主視窗顯示（啟動後毫秒）", super::since_start_ms());
    a.focus_list(Side::Left);
    if a.picker_mode() {
        // 選擇器：不開指定的路徑、不檢查更新；從程式指定（或上次用）的資料夾開始
        let _ = (settings_error, views_error);
        a.picker_start();
    } else {
        a.open_paths(paths_in, true, restored);
        if let Some(e) = settings_error.or(views_error) {
            a.set_footer(e);
        }
        a.schedule_update_check();
        // 「設為預設檔案總管」（預設開啟）：沒選過時設為預設
        maintain_default_file_manager();
        // 「在資料夾中顯示」（瀏覽器的下載等）找得到這個視窗並選取檔案（見 shellwin.rs）；
        // 是不是預設檔案總管會影響要不要登記，所以放在上一行之後
        super::shellwin::start(a.hwnd);
        // 「攔截 Win+E」（預設開啟）：新版 Windows 11 的 Win+E 不看登錄，在背景攔截按鍵改開 Stillfeather
        super::wine::maintain();
        // 「其他程式開啟或儲存檔案時也用 Stillfeather」（預設關閉）：依使用者的選擇登記或取消，並更新登記的取代元件
        super::picker::maintain();
        // 舊版的資料夾（%LOCALAPPDATA%\FeatherFiles）：設定已經搬走、取代元件也換到新資料夾之後刪掉
        super::rescue::remove_old_data_dir();
        // 更新時還在執行而改名讓位的舊 exe（見 install::old_exe_name）：結束後就刪得掉
        std::thread::spawn(remove_old_exes);
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            if pretranslate(&msg) {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    msg.wParam as i32
}

/// 刪掉 exe 旁邊改名讓位的舊 exe（還在執行的刪不掉，下次啟動再試）
fn remove_old_exes() {
    let Some(dir) = current_exe_path().and_then(|p| paths::parent(&p)) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for e in entries.flatten() {
        if feather_core::install::is_old_exe_name(&e.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 主程式啟動時依使用者對「設為預設檔案總管」的選擇處理（沒選過時預設開啟，見 install::default_fm_on_start）。
/// 失敗時不記下選擇，下次啟動再試。
fn maintain_default_file_manager() {
    let registered = shellreg::default_fm_program().map(|p| std::path::Path::new(&p).is_file());
    match install::default_fm_on_start(shellreg::default_fm_choice().as_deref(), registered) {
        install::DefaultFmStart::Keep => {}
        install::DefaultFmStart::Remember => {
            let _ = shellreg::set_default_fm_choice(true);
        }
        install::DefaultFmStart::Take => {
            if current_exe_path().is_some_and(|exe| shellreg::set_default_file_manager(&exe).is_ok()) {
                let _ = shellreg::set_default_fm_choice(true);
            }
        }
    }
}

impl App {
    pub fn s(&self, v: i32) -> i32 {
        self.theme.s(v)
    }

    pub(super) fn active_side(&self) -> Side {
        if self.ws.dual {
            self.ws.active
        } else {
            Side::Left
        }
    }

    pub(super) fn active_tab_id(&self) -> TabId {
        self.ws.current(self.active_side()).unwrap_or_else(|| self.ws.active_tab())
    }

    pub(super) fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(&self.active_tab_id())
    }

    pub(super) fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let id = self.active_tab_id();
        self.tabs.get_mut(&id)
    }

    pub(super) fn side_of_list(&self, hwnd: HWND) -> Option<Side> {
        if hwnd == self.lists[0] {
            Some(Side::Left)
        } else if hwnd == self.lists[1] {
            Some(Side::Right)
        } else {
            None
        }
    }

    // ------------------------------------------------------------ 建立控制項

    fn create_children(&mut self) {
        let hwnd = self.hwnd;
        self.strips = [tabs::create(hwnd, Side::Left), tabs::create(hwnd, Side::Right)];
        self.crumbs = crumbs::create(hwnd);
        self.chips = chips::create(hwnd);
        let icon = |id: u16, g: char, text: &str, surface: Surface| controls::create(hwnd, id, Kind::Icon(g), text, surface, false);
        let round = |id: u16, g: char, text: &str| controls::create(hwnd, id, Kind::Round(g), text, Surface::Page, false);
        self.back_btn = icon(id::BACK, glyph::BACK, tr!("上一頁 (Alt+←)", "Back (Alt+←)"), Surface::Page);
        self.forward_btn = icon(id::FORWARD, glyph::FORWARD, tr!("下一頁 (Alt+→)", "Forward (Alt+→)"), Surface::Page);
        self.up_btn = icon(id::UP, glyph::UP, tr!("上一層 (Alt+↑)", "Up (Alt+↑)"), Surface::Page);
        self.refresh_btn = icon(id::REFRESH, glyph::REFRESH, tr!("重新整理 (F5)", "Refresh (F5)"), Surface::Page);
        self.address = create_edit(hwnd, id::ADDRESS, tr!("資料夾路徑", "Folder path"));
        self.query = create_edit(hwnd, id::QUERY, tr!("檔名搜尋", "File name search"));
        let cue = wide(tr!("搜尋檔名…", "Search file names…"));
        send_ptr(self.query, EM_SETCUEBANNER, 1, cue.as_ptr());
        self.clear_btn = icon(id::CLEAR, glyph::CLOSE, tr!("清除或取消搜尋", "Clear or cancel search"), Surface::Card);
        // 不搶焦點：勾選後游標仍在搜尋框裡
        self.global_btn = controls::create(hwnd, id::SEARCH_GLOBAL, Kind::Toggle, tr!("全域搜尋", "Whole PC"), Surface::Card, false);
        self.search_btn = round(id::SEARCH, glyph::SEARCH, tr!("搜尋檔名 (Ctrl+F)", "Search file names (Ctrl+F)"));

        // 右上角：像參考圖的月亮、鈴鐺那樣的圓形按鈕
        self.dual_btn = round(id::DUAL, glyph::DUAL, tr!("切換雙欄 (Ctrl+Shift+D)", "Toggle dual pane (Ctrl+Shift+D)"));
        self.theme_btn = round(id::THEME, glyph::MOON, tr!("切換淺色／深色", "Switch light/dark"));
        let settings = round(id::SETTINGS, glyph::SETTINGS, tr!("設定 (Ctrl+,)", "Settings (Ctrl+,)"));
        self.top_buttons = vec![self.dual_btn, self.theme_btn, settings];

        // 標題列右側：主要動作是膠囊按鈕，其餘是圓形圖示按鈕
        let new_folder = controls::create(hwnd, id::NEW_FOLDER, Kind::Pill(glyph::NEW_FOLDER), tr!("新增資料夾", "New folder"), Surface::Page, false);
        self.action_buttons = vec![new_folder];
        for (cid, g, text) in [
            (id::CUT, glyph::CUT, tr!("剪下 (Ctrl+X)", "Cut (Ctrl+X)")),
            (id::COPY, glyph::COPY, tr!("複製 (Ctrl+C)", "Copy (Ctrl+C)")),
            (id::PASTE, glyph::PASTE, tr!("貼上 (Ctrl+V)", "Paste (Ctrl+V)")),
            (id::RENAME, glyph::RENAME, tr!("重新命名 (F2)", "Rename (F2)")),
            (id::DELETE, glyph::DELETE, tr!("刪除到資源回收筒 (Delete、Ctrl+D)", "Delete to Recycle Bin (Delete, Ctrl+D)")),
            (id::COPY_ACROSS, glyph::COPY_ACROSS, tr!("複製到另一欄", "Copy to the other pane")),
            (id::PROPERTIES, glyph::PROPERTIES, tr!("內容 (Alt+Enter)", "Properties (Alt+Enter)")),
        ] {
            self.action_buttons.push(round(cid, g, text));
        }
        // 最右邊：排序與檢視（顯示隱藏項目、副檔名在「檢視 › 顯示」裡）
        self.sort_btn = controls::create(hwnd, id::SORT_MENU, Kind::Menu(glyph::SORT), tr!("排序", "Sort"), Surface::Page, false);
        self.view_btn = controls::create(hwnd, id::VIEW_MENU, Kind::Menu(glyph::VIEW_DETAILS), tr!("檢視", "View"), Surface::Page, false);

        // 側欄
        self.help_btn = icon(id::HELP, glyph::HELP_CIRCLE, tr!("使用說明", "Help"), Surface::Sidebar);
        controls::set_subtle(self.help_btn);
        self.promo_btn = controls::create(hwnd, id::SETTINGS, Kind::Text, tr!("開啟設定", "Open Settings"), Surface::Peach, false);
        controls::set_primary(self.promo_btn);
        self.tree = self.create_tree();
        self.lists = [self.create_list(id::LIST_LEFT, Side::Left), self.create_list(id::LIST_RIGHT, Side::Right)];
        self.pane_status = [create_static(hwnd), create_static(hwnd)];
        self.preview.popup = preview::create_popup(hwnd);

        unsafe {
            let class = wide("tooltips_class32");
            self.tooltip = CreateWindowExW(
                WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                0,
                0,
                0,
                0,
                hwnd,
                0,
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            SendMessageW(self.tooltip, TTM_SETMAXTIPWIDTH, 0, self.s(400) as LPARAM);
        }
        let mut tipped: Vec<HWND> = vec![self.back_btn, self.forward_btn, self.up_btn, self.refresh_btn, self.clear_btn, self.search_btn, self.help_btn];
        tipped.extend(self.top_buttons.iter().copied());
        tipped.extend(self.action_buttons.iter().skip(1).copied());
        tipped.extend([self.sort_btn, self.view_btn]);
        for b in tipped {
            self.add_tooltip(b);
        }
        self.apply_fonts();
    }

    fn add_tooltip(&self, target: HWND) {
        let mut text = wide(&window_text(target));
        let info = TOOLINFOW {
            cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: self.hwnd,
            uId: target as usize,
            rect: RECT::default(),
            hinst: 0,
            lpszText: text.as_mut_ptr(),
            lParam: 0,
            lpReserved: std::ptr::null_mut(),
        };
        send_ptr(self.tooltip, TTM_ADDTOOLW, 0, &info);
    }

    fn apply_fonts(&self) {
        let ui = self.theme.fonts.ui as WPARAM;
        for h in [self.address, self.query, self.tree, self.lists[0], self.lists[1], self.tooltip] {
            if h != 0 {
                send(h, WM_SETFONT, ui, 1);
            }
        }
        for h in self.pane_status {
            if h != 0 {
                send(h, WM_SETFONT, self.theme.fonts.small as WPARAM, 1);
            }
        }
        if let Some(p) = self.picker.as_ref() {
            send(p.name_edit, WM_SETFONT, ui, 1);
        }
    }

    pub(super) fn apply_theme_to_controls(&mut self) {
        let p = self.theme.pal;
        theme::style_caption(self.hwnd, &p, p.background);
        theme::apply_app_mode(p.dark);
        for side in [Side::Left, Side::Right] {
            self.style_list(side);
        }
        self.style_tree();
        controls::set_glyph(self.theme_btn, if p.dark { glyph::SUN } else { glyph::MOON });
        unsafe {
            if p.dark {
                let name = wide("DarkMode_Explorer");
                SetWindowTheme(self.tooltip, name.as_ptr(), std::ptr::null());
            } else {
                SetWindowTheme(self.tooltip, std::ptr::null(), std::ptr::null());
            }
            InvalidateRect(self.hwnd, std::ptr::null(), 1);
        }
        let mut all: Vec<HWND> = vec![
            self.address,
            self.query,
            self.back_btn,
            self.forward_btn,
            self.up_btn,
            self.refresh_btn,
            self.clear_btn,
            self.global_btn,
            self.search_btn,
            self.help_btn,
            self.promo_btn,
            self.crumbs,
            self.chips,
            self.strips[0],
            self.strips[1],
            self.pane_status[0],
            self.pane_status[1],
        ];
        all.extend(self.top_buttons.iter().copied());
        all.extend(self.action_buttons.iter().copied());
        all.extend([self.sort_btn, self.view_btn]);
        if let Some(p) = self.picker.as_ref() {
            all.extend([p.name_edit, p.type_btn, p.ok_btn, p.cancel_btn, p.system_btn]);
        }
        for h in all {
            invalidate(h);
        }
    }

    // ------------------------------------------------------------ 布局

    fn clear_visible(&self) -> bool {
        self.active_tab().is_some_and(|t| t.working() || !t.query.is_empty())
    }

    /// 清除鈕只在搜尋膠囊（大致）展開後出現。
    fn clear_shown(&self) -> bool {
        self.search_open.target() > 0.5 && anim::value(&self.search_open) > 0.6 && self.clear_visible()
    }

    /// 第一列中段：左邊是路徑列，右邊是搜尋。搜尋平常收成一個圓形按鈕，
    /// 展開時往左長成膠囊輸入框，路徑列跟著讓出寬度。
    fn top_row(&self, area: RECT) -> TopRow {
        let s = |v: i32| self.theme.s(v);
        let v = anim::value(&self.search_open).clamp(0.0, 1.0);
        let opening = self.search_open.target() > 0.5;
        let round = s(40);
        let row_h = area.height();
        let full = s(360).min((area.width() - s(12)) * 2 / 5).max(round);
        let w = round + ((full - round) as f32 * v).round() as i32;
        let h = round + ((row_h - round) as f32 * v).round() as i32;
        let top = area.top + (row_h - h) / 2;
        let search = RECT { left: area.right - w, top, right: area.right, bottom: top + h };
        let crumbs = RECT { left: area.left, top: area.top, right: search.left - s(12), bottom: area.bottom };
        let edit_h = self.theme.fonts.ui_height + s(2);
        let center_y = |r: &RECT| r.top + (r.height() - edit_h) / 2;
        let query_left = search.left + s(44);
        let cb = s(28);
        // 「全域搜尋」放在清除鈕左邊；膠囊太窄時不顯示
        let gw = s(100);
        let global_left = search.right - s(40) - gw;
        let global_fits = search.width() >= s(260);
        let global_shown = opening && v > 0.6 && global_fits;
        let query_right = if global_fits && opening { global_left - s(4) } else { search.right - s(44) };
        let windows = vec![
            (self.crumbs, crumbs, !self.address_editing),
            (
                self.address,
                RECT { left: crumbs.left + s(16), top: center_y(&crumbs), right: crumbs.right - s(16), bottom: center_y(&crumbs) + edit_h },
                self.address_editing,
            ),
            // 展開途中輸入框跟著膠囊變寬，所以一按下就能打字
            (self.query, RECT { left: query_left, top: center_y(&search), right: query_right.max(query_left), bottom: center_y(&search) + edit_h }, opening),
            (self.global_btn, RECT::new(global_left, search.top + (h - s(30)) / 2, gw, s(30)), global_shown),
            (self.clear_btn, RECT::new(search.right - s(8) - cb, search.top + (h - cb) / 2, cb, cb), self.clear_shown()),
            // 收合動畫走完才換回真正的按鈕（途中由主視窗畫出變形中的外框）
            (self.search_btn, RECT::new(area.right - round, area.top + (row_h - round) / 2, round, round), !opening && v <= 0.001),
        ];
        TopRow { crumbs, search, windows }
    }

    /// 動畫途中只移動第一列的路徑列與搜尋，不重排整個視窗。
    fn place_top_row(&mut self) {
        let row = self.top_row(self.layout.top_row);
        let crumbs_changed = row.crumbs != self.layout.crumbs;
        self.layout.crumbs = row.crumbs;
        self.layout.search = row.search;
        move_windows(&row.windows);
        // 整段重畫：膠囊的陰影、編輯路徑時的外框都會跟著移動
        let dirty = self.layout.top_row.inflate(self.s(8), self.s(8));
        unsafe {
            InvalidateRect(self.hwnd, &dirty, 0);
        }
        if crumbs_changed {
            invalidate(self.crumbs);
        }
    }

    /// 搜尋該展開的時候：輸入框有焦點（視窗暫時不在前景時看離開前的焦點），或這個分頁正在搜尋、顯示搜尋結果。
    fn search_wanted(&self) -> bool {
        let focus = unsafe { GetFocus() };
        let focused = if focus == 0 { self.last_focus == self.query } else { focus == self.query };
        focused || self.active_tab().is_some_and(|t| t.searching || t.search_mode)
    }

    fn set_search_open(&mut self, open: bool) {
        if (self.search_open.target() > 0.5) == open {
            return;
        }
        anim::to(&mut self.search_open, open as u8 as f32, anim::SLIDE_MS);
        if anim::running(&self.search_open) {
            unsafe {
                SetTimer(self.hwnd, TIMER_SEARCH, 15, std::ptr::null());
            }
        }
        self.place_top_row();
    }

    /// 依目前狀態展開或收合搜尋。
    pub(super) fn sync_search(&mut self) {
        let want = self.search_wanted();
        self.set_search_open(want);
    }

    /// 點在不會自己取得焦點的地方（視窗底、標題、分頁列、篩選籤、麵包屑、狀態列）時，
    /// 讓路徑或搜尋輸入框失去焦點，就像在網頁上點空白處；搜尋沒在進行就跟著收回。
    fn blur_inputs_on_click(&mut self, target: HWND) {
        let focus = unsafe { GetFocus() };
        if focus != self.query && focus != self.address {
            return;
        }
        let passive = target == self.hwnd
            || target == self.chips
            || target == self.crumbs
            || self.strips.contains(&target)
            || self.pane_status.contains(&target);
        if passive {
            self.focus_list(self.active_side());
        }
    }

    /// 按下搜尋按鈕或 Ctrl+F：展開並把焦點放進輸入框。
    fn open_search(&mut self) {
        self.set_search_open(true);
        unsafe {
            SetFocus(self.query);
        }
    }

    fn on_search_timer(&mut self) {
        self.place_top_row();
        if !anim::running(&self.search_open) {
            unsafe {
                KillTimer(self.hwnd, TIMER_SEARCH);
            }
        }
    }

    pub(super) fn relayout(&mut self) {
        let rc = client_rect(self.hwnd);
        if rc.width() <= 0 || rc.height() <= 0 {
            return;
        }
        let s = |v: i32| self.theme.s(v);
        let w = rc.width();
        let h = rc.height();
        let m = s(14);
        let mut l = Layout::default();
        let mut pos: Vec<(HWND, RECT, bool)> = Vec::new();

        // ---- 側欄卡片（全高）
        let sidebar_w = s(self.settings.sidebar_width).min((w - s(640)).max(s(SIDEBAR_MIN)));
        l.sidebar = RECT { left: m, top: m, right: m + sidebar_w, bottom: h - m };
        l.brand = RECT { left: l.sidebar.left + s(16), top: l.sidebar.top + s(16), right: l.sidebar.right - s(52), bottom: l.sidebar.top + s(60) };
        // 說明按鈕與標頭垂直置中，靠右但不貼邊
        let help = s(30);
        let pick = self.picker_mode();
        pos.push((self.help_btn, RECT::new(l.sidebar.right - s(14) - help, (l.brand.top + l.brand.bottom - help) / 2, help, help), !pick));
        let g = self.promo_geometry(l.sidebar);
        l.promo = g.promo;
        l.promo_toggle = g.toggle;
        pos.push((self.promo_btn, g.button, g.button_visible));
        pos.push((self.tree, self.tree_rect(&l, g.tree_bottom), true));
        let gap = s(18);
        l.sidebar_splitter = RECT { left: l.sidebar.right, top: l.sidebar.top, right: l.sidebar.right + gap, bottom: l.sidebar.bottom };
        let cl = l.sidebar.right + gap;
        let cr = w - m;
        l.content_left = cl;
        l.content_right = cr;

        // ---- 第一列：導覽、麵包屑、搜尋膠囊、圓形按鈕
        let row_top = m;
        let row_h = s(42);
        let nav = s(34);
        let nav_y = row_top + (row_h - nav) / 2;
        for (i, b) in [self.back_btn, self.forward_btn, self.up_btn, self.refresh_btn].iter().enumerate() {
            pos.push((*b, RECT::new(cl + i as i32 * (nav + s(2)), nav_y, nav, nav), true));
        }
        let round = s(40);
        let mut x = cr - round;
        for b in self.top_buttons.iter().rev() {
            pos.push((*b, RECT::new(x, row_top + (row_h - round) / 2, round, round), !pick));
            if !pick {
                x -= round + s(8);
            }
        }
        let mut top_left = x + round + s(8);
        if let Some(p) = self.picker.as_ref() {
            // 選擇器：右上角只有「使用 Windows 視窗」（圓形按鈕都不需要）
            let sw = s(150);
            pos.push((p.system_btn, RECT::new(cr - sw, row_top + (row_h - s(34)) / 2, sw, s(34)), true));
            top_left = cr - sw;
        }
        l.top_row = RECT { left: cl + 4 * (nav + s(2)) + s(10), top: row_top, right: top_left - s(14), bottom: row_top + row_h };
        let row = self.top_row(l.top_row);
        l.crumbs = row.crumbs;
        l.search = row.search;
        pos.extend(row.windows);

        // ---- 大標題與摘要；右側是動作按鈕
        let title_top = row_top + row_h + s(20);
        let act = s(36);
        let pill_w = s(126);
        let round_actions = if pick { 0 } else { self.action_buttons.len() as i32 - 1 };
        let base_w = pill_w + s(10) + round_actions * (act + s(6));
        // 視窗窄到標題剩不到 240 時，排序、檢視只顯示圖示與箭頭
        let menu_w = if cr - cl - base_w - (s(14) + 2 * s(92)) - s(20) >= s(240) { s(92) } else { s(54) };
        let actions_w = base_w + s(8) + 2 * menu_w + s(6);
        let mut ax = cr - actions_w;
        for (i, b) in self.action_buttons.iter().enumerate() {
            if pick && i > 0 {
                pos.push((*b, RECT::default(), false));
                continue;
            }
            let bw = if i == 0 { pill_w } else { act };
            pos.push((*b, RECT::new(ax, title_top + s(6), bw, act), true));
            ax += bw + if i == 0 { s(10) } else { s(6) };
        }
        ax += s(8);
        for b in [self.sort_btn, self.view_btn] {
            pos.push((b, RECT::new(ax, title_top + s(6), menu_w, act), true));
            ax += menu_w + s(6);
        }
        l.title = RECT { left: cl, top: title_top, right: cr - actions_w - s(20), bottom: title_top + s(44) };
        l.subtitle = RECT { left: cl + s(2), top: l.title.bottom, right: l.title.right, bottom: l.title.bottom + s(22) };

        // ---- 分頁、篩選籤（選擇器沒有：清單卡片直接接在標題下）
        let strip_top = l.subtitle.bottom + s(14);
        let strip_h = s(40);
        l.chips = RECT { left: cl, top: strip_top + strip_h + s(10), right: cr, bottom: strip_top + strip_h + s(10) + s(36) };
        pos.push((self.chips, l.chips, !pick));

        // ---- 底部浮動狀態列（選擇器是檔名欄、類型與確定／取消）與清單卡片
        l.footer = RECT { left: cl, top: h - m - s(if pick { 62 } else { 54 }), right: cr, bottom: h - m };
        if pick {
            pos.extend(self.picker_bar_layout(l.footer));
        }
        let cards_top = if pick { strip_top } else { l.chips.bottom + s(14) };
        let cards_bottom = l.footer.top - s(14);
        if self.ws.dual {
            let gap = s(16);
            let left_w = ((cr - cl - gap) * self.settings.split_percent / 100).max(1);
            l.panes[0] = RECT { left: cl, top: cards_top, right: cl + left_w, bottom: cards_bottom };
            let split = RECT { left: l.panes[0].right, top: strip_top, right: l.panes[0].right + gap, bottom: cards_bottom };
            l.panes[1] = RECT { left: split.right, top: cards_top, right: cr, bottom: cards_bottom };
            l.pane_splitter = Some(split);
        } else {
            l.panes[0] = RECT { left: cl, top: cards_top, right: cr, bottom: cards_bottom };
        }
        l.strips[0] = RECT { left: cl, top: strip_top, right: l.panes[0].right, bottom: strip_top + strip_h };
        l.strips[1] = RECT { left: l.panes[1].left, top: strip_top, right: cr, bottom: strip_top + strip_h };
        pos.push((self.strips[0], l.strips[0], !pick));
        pos.push((self.strips[1], l.strips[1], self.ws.dual && !pick));
        for i in 0..2 {
            let visible = i == 0 || self.ws.dual;
            let pane = l.panes[i];
            let list = RECT { left: pane.left + s(10), top: pane.top + s(10), right: pane.right - s(10), bottom: pane.bottom - s(32) };
            let status = RECT { left: pane.left + s(20), top: list.bottom + s(4), right: pane.right - s(20), bottom: pane.bottom - s(6) };
            pos.push((self.lists[i], list, visible));
            pos.push((self.pane_status[i], status, visible));
        }
        self.layout = l;
        move_windows(&pos);
        unsafe {
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
        for side in [Side::Left, Side::Right] {
            self.fit_columns(side);
        }
        for h in [self.strips[0], self.strips[1], self.chips, self.crumbs] {
            invalidate(h);
        }
    }

    /// 顯示操作結果；直到下一次選取或導覽前都不會被摘要覆蓋。
    pub(super) fn set_footer(&mut self, text: String) {
        self.footer = text;
        self.footer_sticky = true;
        self.invalidate_footer();
    }

    pub(super) fn clear_footer_message(&mut self) {
        self.footer_sticky = false;
    }

    fn invalidate_footer(&self) {
        unsafe {
            InvalidateRect(self.hwnd, &self.layout.footer.inflate(self.s(4), self.s(8)), 0);
        }
    }

    /// 路徑列切換成可輸入的文字框。
    pub(super) fn begin_address_edit(&mut self) {
        let _fade = if self.address_editing {
            anim::Crossfade::none()
        } else {
            self.crossfade(vec![self.layout.crumbs.inflate(self.s(4), self.s(8))], anim::HOVER_MS)
        };
        let path = self.active_tab().map(|t| t.display_path()).unwrap_or_default();
        set_window_text(self.address, &path);
        self.address_editing = true;
        self.relayout();
        unsafe {
            SetFocus(self.address);
        }
    }

    /// 回到麵包屑顯示。
    pub(super) fn end_address_edit(&mut self) {
        if self.address_editing {
            let _fade = self.crossfade(vec![self.layout.crumbs.inflate(self.s(4), self.s(8))], anim::HOVER_MS);
            self.address_editing = false;
            self.relayout();
        }
    }

    /// 大標題與其下的摘要文字。
    fn title_texts(&self) -> (String, String) {
        let Some(t) = self.active_tab() else { return ("Stillfeather".into(), String::new()) };
        if t.search_mode {
            let title = if t.search_global { trf!("在整台電腦搜尋「{}」", "Searching the whole PC for “{}”", t.query.trim()) } else { trf!("搜尋「{}」", "Search “{}”", t.query.trim()) };
            return (title, t.status.clone());
        }
        let title = paths::tab_title(&t.display_path());
        if t.loading || t.status.starts_with(tr!("無法", "Couldn't")) || t.status.starts_with(tr!("已取消", "Cancelled")) {
            return (title, t.status.clone());
        }
        let s = &t.summary;
        let mut parts = vec![trn!(s.folders; "{} 個資料夾", "{} folder", "{} folders", group_digits(s.folders as u64)), trn!(s.files; "{} 個檔案", "{} file", "{} files", group_digits(s.files as u64))];
        if s.files > 0 {
            parts.push(format_size(Some(s.bytes)));
        }
        if t.filter != Category::All {
            parts.push(trf!("顯示{} {} 項", "{0}: showing {1}", t.filter.label(), group_digits(t.row_count() as u64)));
        } else if t.pick.as_ref().is_some_and(|p| !p.is_all()) {
            // 選擇器的檔案類型藏起了一些檔案
            parts.push(trf!("顯示 {} 項", "Showing {}", group_digits(t.row_count() as u64)));
        }
        (title, parts.join(" · "))
    }

    // ------------------------------------------------------------ 繪製主視窗

    fn paint(&mut self) {
        // 側欄標頭的圖示依 DPI 決定邊長，第一次畫或縮放改變時才重建
        let mark = self.theme.s(40);
        if self.brand_icon_px != mark {
            unsafe {
                if self.brand_icon != 0 {
                    DestroyIcon(self.brand_icon);
                }
            }
            self.brand_icon = icons::brand_icon(mark);
            self.brand_icon_px = mark;
        }
        let brand_icon = (self.brand_icon, self.brand_icon_px);
        // 草書字樣：可用寬度（圖示右側到說明按鈕前）或墨色改變時才重畫
        let b = self.layout.brand;
        let avail = (b.right - (b.left + mark + self.theme.s(10)), b.height() + self.theme.s(8), self.theme.pal.sidebar_ink);
        if self.wordmark_key != avail {
            unsafe {
                if self.wordmark.0 != 0 {
                    DestroyIcon(self.wordmark.0);
                }
            }
            self.wordmark = icons::wordmark_icon(avail.0, avail.1, avail.2, self.theme.dpi);
            self.wordmark_key = avail;
        }
        let wordmark = self.wordmark;
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let rect = client_rect(self.hwnd);
            {
                let buf = theme::Buffered::new(hdc, rect);
                let dc = buf.hdc;
                let p = self.theme.pal;
                let f = &self.theme.fonts;
                let dpi = self.theme.dpi;
                let l = &self.layout;
                let s = |v: i32| self.theme.s(v);
                theme::fill(dc, &rect, p.background);

                // ---- 側欄卡片
                theme::card(dc, &l.sidebar, s(24), p.sidebar, None, &p, dpi);
                // 程式名稱前面是 Stillfeather 自己的圖示（與工作列同一個），直接放在側欄上，不另外墊色塊
                let mark = brand_icon.1;
                let logo = RECT::new(l.brand.left, (l.brand.top + l.brand.bottom - mark) / 2, mark, mark);
                if brand_icon.0 != 0 {
                    DrawIconEx(dc, logo.left, logo.top, brand_icon.0, mark, mark, 0, 0, DI_NORMAL);
                }
                // 程式名稱：沾水筆草書字樣，與圖示垂直置中（產生失敗時退回一般文字）
                let name = RECT { left: logo.right + s(10), top: l.brand.top, right: l.brand.right, bottom: l.brand.bottom };
                if wordmark.0 != 0 {
                    let y = (l.brand.top + l.brand.bottom - wordmark.2) / 2;
                    DrawIconEx(dc, name.left, y, wordmark.0, wordmark.1, wordmark.2, 0, 0, DI_NORMAL);
                } else {
                    theme::draw_text(dc, "Stillfeather", &name, f.brand, p.sidebar_ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }
                if let Some(promo) = l.promo {
                    // 展開程度依目前高度換算（側欄太矮而強制收合時也正確）
                    let v = ((promo.height() - s(46)) as f32 / (s(150) - s(46)).max(1) as f32).clamp(0.0, 1.0);
                    let hv = anim::value(&self.promo_hover);
                    if anim::running(&self.promo_hover) {
                        anim::request_rects(self.hwnd, vec![promo]);
                    }
                    let hover_fill = theme::blend(p.ink, p.peach, 0.06);
                    // 收合時整張卡片都可以點，懸停時整張卡片淡淡加深
                    let fill = anim::mix(p.peach, hover_fill, hv * (1.0 - v));
                    theme::round_rect(dc, &promo, s(20).min(promo.height() / 2), Some(fill), None);
                    let saved = SaveDC(dc);
                    IntersectClipRect(dc, promo.left, promo.top, promo.right, promo.bottom);
                    // 展開的內容：展開後段才淡入
                    let a = ((v - 0.4) / 0.6).clamp(0.0, 1.0);
                    if a > 0.0 {
                        let g = RECT { top: promo.top + s(12), bottom: promo.top + s(40), ..promo };
                        theme::draw_text(dc, &glyph::PALETTE.to_string(), &g, f.icon, anim::mix(fill, p.accent, a), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                        let t1 = RECT { top: g.bottom, bottom: g.bottom + s(22), ..promo };
                        theme::draw_text(dc, tr!("個人化", "Personalize"), &t1, f.ui_bold, anim::mix(fill, p.ink, a), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                        let t2 = RECT { left: promo.left + s(10), right: promo.right - s(10), top: t1.bottom, bottom: t1.bottom + s(20) };
                        theme::draw_text(dc, tr!("主題、配色與快速預覽", "Theme, colors and quick preview"), &t2, f.small, anim::mix(fill, p.muted, a), DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                        // 右上角的收合鈕
                        if let Some(t) = l.promo_toggle {
                            if hv > 0.001 {
                                theme::circle(dc, &t, Some(anim::mix(p.peach, theme::blend(p.ink, p.peach, 0.1), hv * a)), None);
                            }
                            theme::draw_text(dc, &glyph::SORT_DOWN.to_string(), &t, f.icon_small, anim::mix(fill, anim::mix(p.muted, p.ink, hv), a), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                        }
                    }
                    // 收合的內容：一列「調色盤圖示＋個人化＋向上箭頭」，收合前段才淡入
                    let b = ((0.45 - v) / 0.45).clamp(0.0, 1.0);
                    if b > 0.0 {
                        let row = RECT { bottom: promo.top + s(46), ..promo };
                        let g = RECT { left: row.left + s(10), right: row.left + s(40), ..row };
                        theme::draw_text(dc, &glyph::PALETTE.to_string(), &g, f.icon, anim::mix(fill, p.accent, b), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                        let t = RECT { left: g.right + s(4), right: row.right - s(36), ..row };
                        theme::draw_text(dc, tr!("個人化", "Personalize"), &t, f.ui_bold, anim::mix(fill, p.ink, b), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                        let c = RECT { left: row.right - s(36), right: row.right - s(8), ..row };
                        theme::draw_text(dc, &glyph::SORT_UP.to_string(), &c, f.icon_small, anim::mix(fill, anim::mix(p.muted, p.ink, hv), b), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    }
                    RestoreDC(dc, saved);
                }

                // ---- 搜尋：收合時是圓形按鈕（子視窗自己畫），展開途中與展開後由這裡畫膠囊
                let focus = GetFocus();
                anim::to(&mut self.search_fx, (focus == self.query) as u8 as f32, anim::HOVER_MS);
                let sv = anim::value(&self.search_fx);
                if anim::running(&self.search_fx) {
                    anim::request_rects(self.hwnd, vec![l.search.inflate(s(4), s(8))]);
                }
                let ov = anim::value(&self.search_open).clamp(0.0, 1.0);
                if self.search_open.target() > 0.5 || ov > 0.001 {
                    // 圓形按鈕的細框隨展開淡去；有焦點時換成重點色
                    let border = anim::mix(anim::mix(p.line, p.surface, ov), p.accent, sv);
                    let shape = if ov < 0.5 { l.search.inflate(-1, -1) } else { l.search };
                    if ov > 0.2 {
                        theme::card(dc, &shape, shape.height(), p.surface, Some(border), &p, dpi);
                    } else {
                        theme::round_rect(dc, &shape, shape.height(), Some(p.surface), Some(border));
                    }
                    // 放大鏡從圓心滑到膠囊左側
                    let collapsed_cx = l.search.left + l.search.height() / 2;
                    let cx = collapsed_cx + ((l.search.left + s(27) - collapsed_cx) as f32 * ov).round() as i32;
                    let sg = RECT { left: cx - s(14), right: cx + s(14), ..l.search };
                    let font = if ov < 0.5 { f.icon } else { f.icon_small };
                    theme::draw_text(dc, &glyph::SEARCH.to_string(), &sg, font, anim::mix(p.ink, p.muted, ov), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                }
                if self.address_editing {
                    theme::card(dc, &l.crumbs, l.crumbs.height(), p.surface, Some(p.accent), &p, dpi);
                }

                // ---- 大標題
                let (title, subtitle) = self.title_texts();
                theme::draw_text(dc, &title, &l.title, f.title, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                theme::draw_text(dc, &subtitle, &l.subtitle, f.ui, p.muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);

                // ---- 清單卡片（雙欄時作用中的一欄以重點色描邊）
                let active = self.active_side();
                for i in 0..2 {
                    if i == 1 && !self.ws.dual {
                        continue;
                    }
                    let dropping = self.drop_pane.is_some_and(|d| d.index() == i);
                    let border = if dropping || (self.ws.dual && active.index() == i) { Some(p.accent) } else { Some(theme::blend(p.ink, p.surface, 0.05)) };
                    theme::card(dc, &l.panes[i], s(20), p.surface, border, &p, dpi);
                }
                if let Some(sp) = l.pane_splitter {
                    if self.drag == Some(Drag::Split) {
                        let c = sp.left + sp.width() / 2;
                        let bar = RECT { left: c - s(2), top: l.panes[0].top + s(20), right: c + s(2), bottom: l.panes[0].bottom - s(20) };
                        theme::round_rect(dc, &bar, s(4), Some(p.accent), None);
                    }
                }
                if self.drag == Some(Drag::Sidebar) {
                    let c = l.sidebar_splitter.left + l.sidebar_splitter.width() / 2;
                    let bar = RECT { left: c - s(2), top: l.sidebar.top + s(40), right: c + s(2), bottom: l.sidebar.bottom - s(40) };
                    theme::round_rect(dc, &bar, s(4), Some(p.accent), None);
                }

                // ---- 選擇器：檔名欄與按鈕（取代浮動狀態列）
                if self.picker_mode() {
                    self.paint_picker_bar(dc, l.footer);
                } else {
                // ---- 浮動狀態列（參考圖下方的播放列）
                let ft = l.footer;
                theme::card(dc, &ft, s(22), p.surface, Some(theme::blend(p.ink, p.surface, 0.05)), &p, dpi);
                let info = &self.footer_info;
                let mut x = ft.left + s(16);
                if info.icon >= 0 && self.icons.list != 0 {
                    let tile = RECT::new(x, ft.top + (ft.height() - s(36)) / 2, s(36), s(36));
                    theme::round_rect(dc, &tile, s(10), Some(p.chip), None);
                    let iy = tile.top + (tile.height() - self.icons.row_px) / 2;
                    let ix = tile.left + (tile.width() - self.icons.icon_px) / 2;
                    (comctl().image_list_draw)(self.icons.list, info.icon, dc, ix, iy, ILD_TRANSPARENT);
                    x = tile.right + s(12);
                }
                let right_w = (ft.width() / 3).min(s(360));
                let text_right = ft.right - s(16) - right_w - s(12);
                let fv = anim::value(&self.footer_fx).clamp(0.0, 1.0);
                let rise = anim::px((1.0 - fv) * s(5) as f32);
                let t1 = RECT { left: x, top: ft.top + s(8) + rise, right: text_right, bottom: ft.top + ft.height() / 2 + s(1) + rise };
                theme::draw_text(dc, &info.title, &t1, f.ui_bold, anim::mix(p.surface, p.ink, fv), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                let t2 = RECT { top: t1.bottom - s(1), bottom: ft.bottom - s(8) + rise, ..t1 };
                theme::draw_text(dc, &info.detail, &t2, f.small, anim::mix(p.surface, p.muted, fv), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                if anim::running(&self.footer_fx) {
                    anim::request_rects(self.hwnd, vec![ft]);
                }
                let rr = RECT { left: ft.right - s(16) - right_w, right: ft.right - s(18), ..ft };
                let color = if self.op_busy { p.accent } else { p.muted };
                theme::draw_text(dc, &self.footer, &rr, f.ui, color, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
            EndPaint(self.hwnd, &ps);
        }
    }

    // ------------------------------------------------------------ 狀態同步

    /// 讓共用的導覽、搜尋與動作列反映作用中欄位的分頁。
    pub(super) fn update_chrome(&mut self) {
        let side = self.active_side();
        let tab_id = self.active_tab_id();
        let (path, can_back, can_fwd, can_up, query) = match self.tabs.get(&tab_id) {
            Some(t) => (t.path.clone(), !t.back.is_empty(), !t.forward.is_empty(), paths::parent(&t.path).is_some(), t.query.clone()),
            None => (String::new(), false, false, false, String::new()),
        };
        if self.address_editing && unsafe { GetFocus() } != self.address {
            set_window_text(self.address, &path);
        }
        if window_text(self.query) != query {
            self.suppress_query_change = true;
            set_window_text(self.query, &query);
            self.suppress_query_change = false;
        }
        controls::set_enabled(self.back_btn, can_back);
        controls::set_enabled(self.forward_btn, can_fwd);
        controls::set_enabled(self.up_btn, can_up);
        controls::set_enabled(self.refresh_btn, !path.is_empty());
        // 殼層視窗清單登記的位置：作用中分頁顯示的資料夾（讀取新資料夾時 path 還是舊的，讀好才換）；搜尋結果、首頁不登記
        if self.picker.is_none() {
            let location = self.tabs.get(&tab_id).filter(|t| !t.search_mode && !t.path.is_empty()).map(|t| t.path.as_str());
            super::shellwin::set_location(location);
        }
        self.sync_search();
        show(self.clear_btn, self.clear_shown());
        controls::set_checked(self.dual_btn, self.ws.dual);
        if !self.op_busy && !self.footer_sticky {
            let where_ = if !self.ws.dual {
                tr!("單欄", "Single pane")
            } else if side == Side::Left {
                tr!("左欄", "Left pane")
            } else {
                tr!("右欄", "Right pane")
            };
            self.footer = trn!(self.ws.tab_count(); "{} 個分頁 · {}", "{} tab · {}", "{} tabs · {}", self.ws.tab_count(), where_);
        }
        let info = self.compute_footer_info(side);
        if info.title != self.footer_info.title || info.detail != self.footer_info.detail {
            // 狀態列內容換了：文字淡入並稍微上浮。連續快速變化（例如按住方向鍵）時不重播，避免文字一直閃。
            let now = anim::now();
            if now - self.footer_changed > 350.0 {
                self.footer_fx.restart(0.25, 1.0, now, anim::dur(anim::TOGGLE_MS));
            }
            self.footer_changed = now;
        }
        self.footer_info = info;
        for i in 0..2 {
            if let Some(t) = self.bound[i].and_then(|id| self.tabs.get(&id)) {
                set_window_text(self.pane_status[i], &t.status);
            }
        }
        for h in [self.strips[0], self.strips[1], self.chips, self.crumbs] {
            invalidate(h);
        }
        unsafe {
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
        // 顯示的資料夾換了（導覽、切換分頁、雙欄、搜尋）：自動重新整理改監看新的資料夾
        self.sync_watches();
    }

    /// 浮動狀態列左側：選取一個項目時顯示其圖示、名稱與細節；多選時顯示數量與合計大小。
    fn compute_footer_info(&mut self, side: Side) -> FooterInfo {
        let count = self.selected_count(side);
        let tab_title = self.active_tab().map(|t| paths::tab_title(&t.display_path())).unwrap_or_default();
        if count == 0 {
            let detail = self.active_tab().map(|t| trn!(t.row_count(); "{} 個項目", "{} item", "{} items", group_digits(t.row_count() as u64))).unwrap_or_default();
            let icon = self.icons.folder_index();
            return FooterInfo { icon, title: tab_title, detail };
        }
        if count == 1 {
            if let Some(e) = self.selected_entries(side).into_iter().next() {
                let mut parts = vec![e.type_label()];
                if !e.is_dir {
                    parts.push(format_size(e.size()));
                }
                let date = format_filetime(e.modified);
                if !date.is_empty() {
                    parts.push(date);
                }
                let icon = self.icons.index_for(&e);
                return FooterInfo { icon, title: e.name.clone(), detail: parts.join(" · ") };
            }
        }
        let detail = if count <= 5_000 {
            let entries = self.selected_entries(side);
            let bytes: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
            let folders = entries.iter().filter(|e| e.is_dir).count();
            trf!("{} 個資料夾 · {} 個檔案 · {}", "{} folders · {} files · {}", folders, entries.len() - folders, format_size(Some(bytes)))
        } else {
            tr!("大小未計算（選取項目過多）", "Size not calculated (too many items selected)").to_string()
        };
        FooterInfo { icon: -1, title: trn!(count; "已選取 {} 個項目", "{} item selected", "{} items selected", group_digits(count as u64)), detail }
    }

    pub(super) fn activate(&mut self, side: Side) {
        if side == Side::Right && !self.ws.dual {
            return;
        }
        if self.ws.active != side {
            self.ws.active = side;
            self.footer_sticky = false;
            self.update_chrome();
        }
    }

    // ------------------------------------------------------------ 個人化卡片

    /// 卡片、收合鈕、「開啟設定」按鈕與樹狀圖底端的位置。高度隨收合動畫在 46～150 之間變化；
    /// 側欄太矮時只顯示收合的樣子，再矮就不顯示。
    fn promo_geometry(&self, sidebar: RECT) -> PromoGeometry {
        let s = |v: i32| self.s(v);
        let none = PromoGeometry { promo: None, toggle: None, button: RECT::default(), button_visible: false, tree_bottom: sidebar.bottom - s(12) };
        // 選擇器沒有個人化卡片：資料夾樹一路到底
        if sidebar.height() <= s(420) || self.picker_mode() {
            return none;
        }
        let v = if sidebar.height() > s(600) { anim::value(&self.promo_fx).clamp(0.0, 1.0) } else { 0.0 };
        let (full, min) = (s(150), s(46));
        let h = min + ((full - min) as f32 * v).round() as i32;
        let promo = RECT { left: sidebar.left + s(12), top: sidebar.bottom - s(12) - h, right: sidebar.right - s(12), bottom: sidebar.bottom - s(12) };
        let bw = s(112);
        let button = RECT::new(promo.left + (promo.width() - bw) / 2, promo.bottom - s(16) - s(34), bw, s(34));
        let toggle = if v > 0.5 { RECT::new(promo.right - s(10) - s(28), promo.top + s(10), s(28), s(28)) } else { promo };
        PromoGeometry { promo: Some(promo), toggle: Some(toggle), button, button_visible: v >= 0.999, tree_bottom: promo.top - s(10) }
    }

    fn tree_rect(&self, l: &Layout, bottom: i32) -> RECT {
        RECT { left: l.sidebar.left + self.s(8), top: l.brand.bottom + self.s(16), right: l.sidebar.right - self.s(8), bottom }
    }

    /// 收合或展開個人化卡片（會記住）。
    fn toggle_personalize(&mut self) {
        let collapsed = !self.settings.personalize_collapsed;
        self.settings.personalize_collapsed = collapsed;
        if let Err(e) = write_settings(&self.settings) {
            self.set_footer(trf!("無法儲存設定：{e}", "Couldn't save settings: {e}"));
        }
        anim::to(&mut self.promo_fx, if collapsed { 0.0 } else { 1.0 }, anim::SLIDE_MS);
        if anim::running(&self.promo_fx) {
            unsafe {
                SetTimer(self.hwnd, TIMER_PROMO, 15, std::ptr::null());
            }
        }
        self.place_sidebar_bottom();
    }

    /// 動畫途中只調整側欄底部（卡片與樹狀圖），不重排整個視窗。
    fn place_sidebar_bottom(&mut self) {
        let g = self.promo_geometry(self.layout.sidebar);
        self.layout.promo = g.promo;
        self.layout.promo_toggle = g.toggle;
        let tree = self.tree_rect(&self.layout, g.tree_bottom);
        unsafe {
            SetWindowPos(self.tree, 0, tree.left, tree.top, tree.width().max(0), tree.height().max(0), SWP_NOZORDER | SWP_NOACTIVATE);
            let b = g.button;
            let show = if g.button_visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
            SetWindowPos(self.promo_btn, 0, b.left, b.top, b.width().max(0), b.height().max(0), SWP_NOZORDER | SWP_NOACTIVATE | show);
            let r = self.layout.sidebar;
            InvalidateRect(self.hwnd, &r, 0);
        }
    }

    fn on_promo_timer(&mut self) {
        self.place_sidebar_bottom();
        if !anim::running(&self.promo_fx) {
            unsafe {
                KillTimer(self.hwnd, TIMER_PROMO);
            }
        }
    }

    /// 滑鼠移到收合鈕（或收合後的整張卡片）上：手形游標與淡入的底色。
    fn promo_mouse(&mut self, x: i32, y: i32) {
        let hot = self.layout.promo_toggle.is_some_and(|r| r.contains(x, y));
        if hot != self.promo_hot {
            self.promo_hot = hot;
            anim::to(&mut self.promo_hover, hot as u8 as f32, anim::HOVER_MS);
            if let Some(r) = self.layout.promo {
                unsafe {
                    InvalidateRect(self.hwnd, &r, 0);
                }
            }
        }
        if hot && !self.mouse_tracked {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                TrackMouseEvent(&mut tme);
            }
            self.mouse_tracked = true;
        }
    }

    // ------------------------------------------------------------ 交叉淡化

    /// 某一欄內容切換時要交叉淡化的範圍：清單卡片；作用中的欄另外加上大標題與底部狀態列。
    pub(super) fn content_fade_areas(&self, side: Side) -> Vec<RECT> {
        let l = &self.layout;
        let mut v = vec![l.panes[side.index()].inflate(self.s(4), self.s(6))];
        if side == self.active_side() {
            v.push(RECT { left: l.title.left, top: l.title.top, right: l.title.right, bottom: l.subtitle.bottom + self.s(2) });
            v.push(l.footer.inflate(self.s(4), self.s(6)));
        }
        v
    }

    /// 分頁列以下整個內容區（雙欄切換、關閉分頁可能改變欄數）。
    fn whole_content_area(&self) -> RECT {
        let l = &self.layout;
        RECT { left: l.content_left, top: l.strips[0].top, right: l.content_right, bottom: l.footer.bottom }.inflate(self.s(6), self.s(8))
    }

    /// 擷取目前畫面並蓋上；回傳值離開作用域時淡出，呈現舊內容淡成新內容。
    pub(super) fn crossfade(&self, areas: Vec<RECT>, ms: f64) -> anim::Crossfade {
        anim::Crossfade::begin(self.hwnd, &areas, ms)
    }

    // ------------------------------------------------------------ 導覽

    pub(super) fn navigate(&mut self, tab_id: TabId, input: &str, kind: NavKind, select: Vec<String>) {
        self.navigate_with(tab_id, input, kind, select, false);
    }

    /// 開啟還不知道是檔案還是資料夾的位置（網路上的位置）：先當成資料夾在背景讀取，畫面顯示「正在讀取資料夾…」；
    /// 其實是檔案時改開它所在的資料夾並選取它（見 on_not_folder）。不在介面執行緒上先檢查，
    /// 因為網路位置離線時連檢查都要等到 SMB 逾時（約 20 秒），這段時間整個視窗會凍住。
    pub(super) fn navigate_or_reveal(&mut self, tab_id: TabId, input: &str, kind: NavKind) {
        self.navigate_with(tab_id, input, kind, vec![], true);
    }

    fn navigate_with(&mut self, tab_id: TabId, input: &str, kind: NavKind, select: Vec<String>, reveal: bool) {
        let show_hidden = self.show_hidden;
        let path = match full_path(input) {
            Ok(p) => p,
            Err(e) => {
                if let Some(t) = self.tabs.get_mut(&tab_id) {
                    t.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
                }
                self.update_chrome();
                return;
            }
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        let (generation, cancel) = tab.begin_work();
        tab.work_started = Some(std::time::Instant::now());
        tab.loading = true;
        tab.searching = false;
        tab.stale = false;
        tab.pending_select = select;
        if tab.path.is_empty() {
            tab.requested = path.clone();
        }
        tab.status = tr!("正在讀取資料夾…", "Reading folder…").into();
        let (sort_col, desc) = (tab.sort_col, tab.desc);
        let folders_on_top = self.settings.folders_on_top;
        let p = path.clone();
        std::thread::spawn(move || {
            let result = if reveal {
                match fs::read_dir_or_file(&p, show_hidden, sort_col, desc, folders_on_top, &cancel) {
                    Ok(Some(listing)) => Ok(listing),
                    Ok(None) => return post(WorkerMsg::NotFolder { tab: tab_id, generation, path: p, kind }),
                    Err(e) => Err(e),
                }
            } else {
                fs::read_dir(&p, show_hidden, sort_col, desc, folders_on_top, &cancel)
            };
            post(WorkerMsg::Dir { tab: tab_id, generation, path: p, kind, result });
        });
        if self.ws.side_of(tab_id).is_some_and(|s| self.bound[s.index()] == Some(tab_id)) {
            self.hide_preview();
        }
        self.update_chrome();
        invalidate(self.strips[0]);
        invalidate(self.strips[1]);
    }

    fn on_dir(&mut self, tab_id: TabId, generation: u64, path: String, kind: NavKind, result: Result<Listing, FsError>) {
        // 換到另一個資料夾、而且這個分頁正顯示在畫面上：舊清單淡成新清單。
        let _fade = match (self.tabs.get(&tab_id), self.ws.side_of(tab_id)) {
            (Some(tab), Some(side))
                if tab.generation == generation
                    && result.is_ok()
                    && !paths::same_path(&tab.path, &path)
                    && self.bound[side.index()] == Some(tab_id)
                    && (side == Side::Left || self.ws.dual) =>
            {
                self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
            }
            _ => anim::Crossfade::none(),
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        if tab.generation != generation {
            return;
        }
        tab.loading = false;
        let loaded_ok = result.is_ok();
        if !loaded_ok {
            tab.work_started = None;
        }
        // 重新讀取同一個資料夾（F5、Stillfeather 自己的檔案作業之後）：側欄裡展開的這個資料夾也跟著更新
        let mut tree_sync: Option<Vec<(String, bool)>> = None;
        match result {
            Ok(listing) => {
                let old = std::mem::take(&mut tab.path);
                // 只是重新整理同一個資料夾時保留類型篩選；換資料夾時回到「全部」。
                let folder_changed = !paths::same_path(&old, &path);
                match kind {
                    NavKind::Normal => {
                        if !old.is_empty() && old != path {
                            tab.back.push(old);
                            tab.forward.clear();
                        }
                    }
                    NavKind::Back => {
                        tab.back.pop();
                        if !old.is_empty() {
                            tab.forward.push(old);
                        }
                    }
                    NavKind::Forward => {
                        tab.forward.pop();
                        if !old.is_empty() {
                            tab.back.push(old);
                        }
                    }
                    NavKind::NoHistory => {}
                }
                if folder_changed {
                    tab.filter = Category::All;
                }
                tab.path = path.clone();
                tab.requested = path;
                tab.entries = listing.entries;
                if folder_changed {
                    tab.media = feather_core::thumbs::media_folder(&tab.entries);
                }
                tab.refilter();
                tab.resummarize();
                tab.search_mode = false;
                tab.view = ViewState::default();
                tab.status = folder_status(tab.entries.len(), listing.skipped);
                if !folder_changed {
                    tree_sync = Some(tab.entries.iter().filter(|e| e.is_dir).map(|e| (e.name.clone(), e.is_link)).collect());
                }
            }
            Err(FsError::Cancelled) => {
                tab.status = tr!("已取消讀取", "Cancelled reading").into();
                tab.pending_select.clear();
                tab.pending_rename = false;
            }
            Err(FsError::Message(e)) => {
                tab.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
                tab.pending_select.clear();
                tab.pending_rename = false;
            }
        }
        let shown = self.ws.side_of(tab_id).is_some_and(|side| self.bound[side.index()] == Some(tab_id));
        if let Some(side) = self.ws.side_of(tab_id) {
            if self.bound[side.index()] == Some(tab_id) {
                self.refresh_list(side, true);
            }
        }
        if tab_id == self.active_tab_id() {
            set_window_text(self.address, &self.tabs[&tab_id].path.clone());
        }
        if let Some(folders) = tree_sync {
            let path = self.tabs[&tab_id].path.clone();
            self.sync_tree_folder(&path, &folders);
        }
        self.update_chrome();
        // 效能記錄：從開始讀取到清單換好（含背景列舉、排序與畫面更新）
        if let Some(t) = self.tabs.get_mut(&tab_id) {
            if let Some(started) = t.work_started.take() {
                let count = t.entries.len() as u64;
                super::crashlog::crumb_value("效能：資料夾讀取完成（毫秒）", started.elapsed().as_millis() as u64);
                super::crashlog::crumb_value("效能：項目數", count);
                if shown && !FIRST_FOLDER_SHOWN.swap(true, Ordering::Relaxed) {
                    super::crashlog::crumb_value("效能：第一個資料夾顯示（啟動後毫秒）", super::since_start_ms());
                }
            }
        }
    }

    /// [`navigate_or_reveal`](Self::navigate_or_reveal) 的位置其實是檔案：同一個分頁改開它所在的資料夾並選取它
    fn on_not_folder(&mut self, tab_id: TabId, generation: u64, path: String, kind: NavKind) {
        if self.tabs.get(&tab_id).is_none_or(|t| t.generation != generation) {
            return;
        }
        let folder = paths::parent(&path).unwrap_or_else(|| path.clone());
        self.navigate(tab_id, &folder, kind, vec![path]);
    }

    pub(super) fn go_back(&mut self) {
        let id = self.active_tab_id();
        let target = self.tabs.get(&id).and_then(|t| if t.loading { None } else { t.back.last().cloned() });
        if let Some(t) = target {
            self.navigate(id, &t, NavKind::Back, vec![]);
        }
    }

    pub(super) fn go_forward(&mut self) {
        let id = self.active_tab_id();
        let target = self.tabs.get(&id).and_then(|t| if t.loading { None } else { t.forward.last().cloned() });
        if let Some(t) = target {
            self.navigate(id, &t, NavKind::Forward, vec![]);
        }
    }

    pub(super) fn go_up(&mut self) {
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get(&id) else { return };
        if let Some(parent) = paths::parent(&tab.path) {
            let select = vec![tab.path.clone()];
            self.navigate(id, &parent, NavKind::Normal, select);
        }
    }

    /// 重新整理：回到目前資料夾清單，保留選取。
    pub(super) fn refresh_tab(&mut self, id: TabId) {
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.path.is_empty() {
            return;
        }
        let path = tab.path.clone();
        let select = self.bound_selection_paths(id);
        self.navigate(id, &path, NavKind::NoHistory, select);
    }

    // ------------------------------------------------------------ 搜尋

    fn start_search(&mut self) {
        let id = self.active_tab_id();
        let text = window_text(self.query);
        let recursive = self.active_tab().map(|t| t.recursive).unwrap_or(true);
        let show_hidden = self.show_hidden;
        let global = self.search_global;
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if (tab.path.is_empty() && !global) || tab.loading {
            return;
        }
        if text.trim().is_empty() {
            tab.status = tr!("請輸入要搜尋的檔案名稱", "Enter a file name to search for").into();
            unsafe {
                SetFocus(self.query);
            }
            self.update_chrome();
            return;
        }
        tab.query = text.clone();
        tab.recursive = recursive;
        tab.search_global = global;
        let (generation, cancel) = tab.begin_work();
        tab.work_started = Some(std::time::Instant::now());
        tab.searching = true;
        tab.search_mode = true;
        tab.entries.clear();
        tab.refilter();
        tab.summary = Summary::default();
        tab.view = ViewState::default();
        let scope = if global { fs::SearchScope::Computer } else { fs::SearchScope::Folder(tab.path.clone()) };
        tab.status = if global {
            trf!("搜尋整台電腦（{}）· 只比對檔名，不讀取檔案內容", "Searching the whole PC ({}) · file names only, file contents aren't read", fs::computer_roots().iter().map(|r| r.trim_end_matches('\\')).collect::<Vec<_>>().join(tr!("、", ", ")))
        } else {
            tr!("搜尋中 · 只比對檔名，不讀取檔案內容", "Searching · file names only, file contents aren't read").into()
        };
        std::thread::spawn(move || {
            let result = fs::search(&scope, &text, recursive, show_hidden, &cancel, |entries, scanned, skipped| {
                post(WorkerMsg::SearchBatch { tab: id, generation, entries, scanned, skipped });
            });
            post(WorkerMsg::SearchDone { tab: id, generation, result });
        });
        self.hide_preview();
        if let Some(side) = self.ws.side_of(id) {
            if self.bound[side.index()] == Some(id) {
                self.refresh_list(side, true);
            }
        }
        self.update_chrome();
    }

    fn on_search_batch(&mut self, id: TabId, generation: u64, mut entries: Vec<Entry>, scanned: usize, skipped: usize) {
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.generation != generation {
            return;
        }
        let start = tab.entries.len();
        for e in &entries {
            tab.summary.add(e);
        }
        if tab.rows.is_some() {
            let shown: Vec<u32> = entries.iter().enumerate().filter(|(_, e)| tab.shows(e)).map(|(i, _)| (start + i) as u32).collect();
            if let Some(rows) = tab.rows.as_mut() {
                rows.extend(shown);
            }
        }
        tab.entries.append(&mut entries);
        tab.status = trn!(
            tab.entries.len();
            "搜尋中 · {} 個結果 · 已檢查 {} 項 · 略過 {} 項", "Searching · {} result · {} checked · {} skipped", "Searching · {} results · {} checked · {} skipped",
            group_digits(tab.entries.len() as u64),
            group_digits(scanned as u64),
            group_digits(skipped as u64)
        );
        if let Some(side) = self.ws.side_of(id) {
            if self.bound[side.index()] == Some(id) {
                self.refresh_list(side, false);
                set_window_text(self.pane_status[side.index()], &self.tabs[&id].status.clone());
            }
        }
    }

    fn on_search_done(&mut self, id: TabId, generation: u64, result: Result<SearchSummary, FsError>) {
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.generation != generation {
            return;
        }
        tab.searching = false;
        if let Some(started) = tab.work_started.take() {
            super::crashlog::crumb_value("效能：搜尋完成（毫秒）", started.elapsed().as_millis() as u64);
            super::crashlog::crumb_value("效能：搜尋結果數", tab.entries.len() as u64);
        }
        tab.status = match result {
            Ok(s) => trn!(
                s.matched;
                "{} · {} 個結果 · 已檢查 {} 項 · 略過 {} 項", "{} · {} result · {} checked · {} skipped", "{} · {} results · {} checked · {} skipped",
                if s.truncated {
                    tr!("結果已達上限（未完整搜尋）", "Result limit reached (search incomplete)")
                } else if s.skipped > 0 {
                    tr!("部分完成", "Partly done")
                } else {
                    tr!("搜尋完成", "Search complete")
                },
                group_digits(s.matched as u64),
                group_digits(s.scanned as u64),
                group_digits(s.skipped as u64)
            ),
            Err(FsError::Cancelled) => trf!("已取消 · 保留 {} 個結果（未完整搜尋）", "Cancelled · kept {} results (search incomplete)", group_digits(tab.entries.len() as u64)),
            Err(FsError::Message(e)) => trf!("無法完成：{e}", "Couldn't complete: {e}"),
        };
        if let Some(side) = self.ws.side_of(id) {
            if self.bound[side.index()] == Some(id) {
                self.refresh_list(side, false);
                self.fit_columns(side);
            }
        }
        self.update_chrome();
    }

    fn clear_or_cancel(&mut self) {
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.working() {
            tab.cancel.store(true, Ordering::Relaxed);
            tab.status = tr!("正在取消…", "Cancelling…").into();
            self.update_chrome();
        } else {
            tab.query.clear();
            self.suppress_query_change = true;
            set_window_text(self.query, "");
            self.suppress_query_change = false;
            self.refresh_tab(id);
        }
    }

    // ------------------------------------------------------------ 分頁與雙欄

    pub(super) fn new_tab(&mut self, side: Side, path: &str) -> TabId {
        self.new_tab_with(side, path, false)
    }

    /// `reveal`：`path` 還不知道是檔案還是資料夾（見 navigate_or_reveal）
    fn new_tab_with(&mut self, side: Side, path: &str, reveal: bool) -> TabId {
        // 選擇器沒有分頁：「在新分頁開啟」就是在原地開啟
        if self.picker_mode() {
            let id = self.active_tab_id();
            self.navigate_with(id, path, NavKind::Normal, vec![], reveal);
            return id;
        }
        let side = if self.ws.dual { side } else { Side::Left };
        let _fade = self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS);
        let id = self.ws.add_tab(side);
        self.tabs.insert(id, Tab::new());
        self.navigate_with(id, path, NavKind::NoHistory, vec![], reveal);
        self.bind(side);
        self.update_chrome();
        id
    }

    fn new_tab_active(&mut self) {
        let path = self.active_tab().map(|t| t.display_path()).filter(|p| !p.is_empty()).unwrap_or_else(|| self.default_path.clone());
        self.new_tab(self.active_side(), &path);
    }

    pub(super) fn close_tab(&mut self, side: Side, index: usize) {
        self.hide_preview();
        let _fade = self.crossfade(vec![self.whole_content_area()], anim::CROSSFADE_MS);
        let was_dual = self.ws.dual;
        match self.ws.close(side, index) {
            CloseResult::Nothing => {}
            CloseResult::CloseWindow => unsafe {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },
            CloseResult::Closed(removed) => {
                for id in removed {
                    if let Some(t) = self.tabs.remove(&id) {
                        t.cancel.store(true, Ordering::Relaxed);
                    }
                    for b in self.bound.iter_mut() {
                        if *b == Some(id) {
                            *b = None;
                        }
                    }
                }
                for side in [Side::Left, Side::Right] {
                    if let Some(b) = self.bound[side.index()] {
                        if self.ws.side_of(b) != Some(side) {
                            self.bound[side.index()] = None;
                        }
                    }
                }
                if was_dual != self.ws.dual {
                    self.relayout();
                }
                self.bind(Side::Left);
                if self.ws.dual {
                    self.bind(Side::Right);
                }
                self.update_chrome();
            }
        }
    }

    fn close_current_tab(&mut self) {
        let side = self.active_side();
        let index = self.ws.group(side).selected;
        self.close_tab(side, index);
    }

    pub(super) fn select_tab(&mut self, side: Side, index: usize) {
        if side == Side::Right && !self.ws.dual {
            return;
        }
        if index >= self.ws.group(side).tabs.len() {
            self.activate(side);
            return;
        }
        self.hide_preview();
        let _fade = if self.ws.group(side).selected != index {
            self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
        } else {
            anim::Crossfade::none()
        };
        self.footer_sticky = false;
        self.ws.select(side, index);
        self.bind(side);
        self.update_chrome();
    }

    fn switch_tab(&mut self, delta: isize) {
        let side = self.active_side();
        let _fade = if self.ws.group(side).tabs.len() > 1 {
            self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
        } else {
            anim::Crossfade::none()
        };
        self.ws.active = side;
        self.ws.switch(delta);
        self.footer_sticky = false;
        self.hide_preview();
        self.bind(side);
        self.update_chrome();
    }

    fn move_tab(&mut self, side: Side, index: usize, delta: isize) {
        if self.ws.move_tab(side, index, delta) {
            self.update_chrome();
        }
    }

    pub(super) fn set_dual(&mut self, enabled: bool) {
        if self.picker_mode() {
            return;
        }
        self.hide_preview();
        let _fade = if enabled != self.ws.dual { self.crossfade(vec![self.whole_content_area()], anim::CROSSFADE_MS + 40.0) } else { anim::Crossfade::none() };
        match self.ws.set_dual(enabled) {
            DualChange::Unchanged => return,
            DualChange::Shown { new_right_tab } => {
                if let Some(id) = new_right_tab {
                    self.tabs.insert(id, Tab::new());
                    let path = self.tabs.get(&self.ws.current(Side::Left).unwrap_or(0)).map(|t| t.display_path()).unwrap_or_default();
                    let path = if path.is_empty() { self.default_path.clone() } else { path };
                    self.navigate(id, &path, NavKind::NoHistory, vec![]);
                }
                self.relayout();
                self.bind(Side::Right);
                // 右欄收起期間沒有監看：補讀一次
                if let Some(id) = self.ws.current(Side::Right) {
                    self.watch_catch_up(id);
                }
            }
            DualChange::Hidden => {
                self.relayout();
            }
        }
        self.update_chrome();
    }

    fn tab_action(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let side = Side::from_index(wparam & 0xF);
        let Some(action) = tabs::Action::from_bits((wparam >> 4) & 0xF) else { return };
        let index = wparam >> 8;
        match action {
            tabs::Action::Select => self.select_tab(side, index),
            tabs::Action::Close => self.close_tab(side, index),
            tabs::Action::New => {
                self.activate(side);
                let path = self
                    .ws
                    .current(side)
                    .and_then(|id| self.tabs.get(&id))
                    .map(|t| t.display_path())
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| self.default_path.clone());
                self.new_tab(side, &path);
            }
            tabs::Action::Menu => {
                let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
                self.tab_menu(side, index, pt);
            }
        }
    }

    fn tab_menu(&mut self, side: Side, index: usize, pt: POINT) {
        let count = self.ws.group(side).tabs.len();
        let valid = index < count;
        if valid {
            self.select_tab(side, index);
        }
        let mut items: Vec<(u16, &str, bool)> = vec![(id::TAB_NEW, tr!("新增分頁\tCtrl+T", "New tab\tCtrl+T"), true)];
        if valid {
            items.push((id::TAB_DUPLICATE, tr!("複製分頁", "Duplicate tab"), true));
            items.push((id::TAB_MOVE_LEFT, tr!("向左移\tCtrl+Shift+PgUp", "Move left\tCtrl+Shift+PgUp"), index > 0));
            items.push((id::TAB_MOVE_RIGHT, tr!("向右移\tCtrl+Shift+PgDn", "Move right\tCtrl+Shift+PgDn"), index + 1 < count));
            items.push((0, "", true));
            items.push((id::TAB_CLOSE, tr!("關閉分頁\tCtrl+W", "Close tab\tCtrl+W"), true));
        }
        let Some(cmd) = popup_menu(self.hwnd, &items, pt) else { return };
        match cmd {
            id::TAB_NEW => self.tab_action(side.index() | ((tabs::Action::New as usize) << 4), 0),
            id::TAB_DUPLICATE => {
                if let Some(path) = self.ws.group(side).tabs.get(index).and_then(|id| self.tabs.get(id)).map(|t| t.display_path()) {
                    self.new_tab(side, &path);
                }
            }
            id::TAB_MOVE_LEFT => self.move_tab(side, index, -1),
            id::TAB_MOVE_RIGHT => self.move_tab(side, index, 1),
            id::TAB_CLOSE => self.close_tab(side, index),
            _ => {}
        }
    }

    // ------------------------------------------------------------ 外部開啟要求

    /// 與 C# 版相同：相同路徑字串的分頁直接聚焦；檔案路徑開啟父資料夾並選取。
    /// 切到這個分頁（在收起的右欄時先開雙欄）
    fn show_tab(&mut self, id: TabId) {
        if self.ws.side_of(id) == Some(Side::Right) && !self.ws.dual {
            self.set_dual(true);
        }
        self.ws.activate_tab(id);
        if let Some(side) = self.ws.side_of(id) {
            self.bind(side);
        }
    }

    // ------------------------------------------------------------ 還原上次的分頁（見 feather_core::session）

    /// 還原的分頁第一次顯示：現在才讀取
    pub(super) fn load_restored(&mut self, id: TabId) {
        let Some(t) = self.tabs.get_mut(&id) else { return };
        if !t.restored {
            return;
        }
        t.restored = false;
        let path = t.requested.clone();
        self.navigate(id, &path, NavKind::NoHistory, vec![]);
    }

    /// 啟動時（設定「還原上次的分頁」開著）照上次關閉時的樣子開回分頁。只有顯示中的分頁馬上讀取，其他的切過去時才讀。
    /// 回傳是否還原了。
    fn restore_session(&mut self) -> bool {
        if !self.settings.restore_tabs {
            return false;
        }
        let Some(session) = take_session() else { return false };
        // 左欄的第一個分頁沿用啟動時建立的那一個
        let first = self.ws.current(Side::Left).unwrap_or(0);
        for (side, saved) in [(Side::Left, &session.left), (Side::Right, &session.right)] {
            for (i, path) in saved.tabs.iter().enumerate() {
                let id = if side == Side::Left && i == 0 {
                    first
                } else {
                    let id = self.ws.add_tab(side);
                    self.tabs.insert(id, Tab::new());
                    id
                };
                if let Some(t) = self.tabs.get_mut(&id) {
                    t.requested = path.clone();
                    t.status = tr!("切換到這個分頁時讀取", "Loads when you switch to this tab").into();
                    t.restored = true;
                }
            }
        }
        if !session.right.tabs.is_empty() {
            self.ws.select(Side::Right, session.right.selected);
        }
        self.ws.select(Side::Left, session.left.selected);
        if session.dual {
            // 還沒顯示視窗：直接開雙欄，不播放切換動畫
            self.ws.set_dual(true);
            if session.active == 1 {
                self.ws.active = Side::Right;
            }
            self.relayout();
        }
        for side in [Side::Left, Side::Right] {
            if side == Side::Left || self.ws.dual {
                self.bind(side);
                if let Some(id) = self.ws.current(side) {
                    self.load_restored(id);
                }
            }
        }
        true
    }

    /// 主視窗關閉、Windows 登出或關機時：設定開著就記下分頁；關著就刪掉舊的記錄。選擇器不記。
    pub(super) fn save_session(&self) {
        if self.picker_mode() {
            return;
        }
        let Ok(path) = session_path() else { return };
        if !self.settings.restore_tabs {
            let _ = std::fs::remove_file(&path);
            return;
        }
        let side_tabs = |side: Side| {
            let g = self.ws.group(side);
            let mut saved = SideTabs::default();
            for (i, id) in g.tabs.iter().enumerate() {
                let Some(p) = self.tabs.get(id).map(Tab::display_path).filter(|p| !p.is_empty()) else { continue };
                if i == g.selected {
                    saved.selected = saved.tabs.len();
                }
                saved.tabs.push(p);
            }
            saved
        };
        let session = Session { dual: self.ws.dual, active: self.ws.active.index(), left: side_tabs(Side::Left), right: side_tabs(Side::Right) }.sanitize();
        if session.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if write_atomic(&path, &session.to_json()).is_err() {
            super::crashlog::crumb("分頁記錄寫入失敗");
        }
    }

    /// 開啟要求的路徑（命令列、另一個 Stillfeather 轉過來的、Win+E）。`restored`：啟動時已經還原了上次的分頁，
    /// 要求的路徑開在新分頁（已經開著同一個位置就切過去），沒有要求時就維持還原的分頁。
    pub(super) fn open_paths(&mut self, requested: Vec<String>, startup: bool, restored: bool) {
        if !startup {
            unsafe {
                if IsIconic(self.hwnd) != 0 {
                    ShowWindow(self.hwnd, SW_RESTORE);
                }
                SetForegroundWindow(self.hwnd);
            }
        }
        // Win+E：啟動時開首頁；已經開著就在新分頁開首頁（像檔案總管每次開新視窗）
        let home = requested.iter().any(|p| p == feather_core::install::HOME_MARKER);
        let requested: Vec<String> = requested.into_iter().filter(|p| p != feather_core::install::HOME_MARKER).collect();
        if home && requested.is_empty() && (!startup || restored) {
            let path = self.default_path.clone();
            // 剛還原了上次的分頁、其中已經有首頁：切過去就好，不再多開一個
            if restored {
                let home_tab = self.ws.all_tabs().find(|id| self.tabs.get(id).is_some_and(|t| paths::same_request_path(&t.display_path(), &path)));
                if let Some(id) = home_tab {
                    self.show_tab(id);
                    self.update_chrome();
                    return;
                }
            }
            let side = self.active_side();
            self.new_tab(side, &path);
            self.update_chrome();
            return;
        }
        if requested.is_empty() {
            if startup && !restored {
                let id = self.active_tab_id();
                let path = self.default_path.clone();
                self.navigate(id, &path, NavKind::NoHistory, vec![]);
            }
            return;
        }
        let mut first = startup && !restored;
        for raw in requested {
            let full = full_path(&raw).unwrap_or(raw);
            // 網路上的位置：在這裡（介面執行緒）判斷是不是檔案，離線時要等到 SMB 逾時（約 20 秒），整個視窗凍住；
            // 改成先當資料夾在背景讀取，其實是檔案時再改開它所在的資料夾並選取（見 navigate_or_reveal）
            let reveal = !fs::on_local_drive(&full);
            let (path, select) = if !reveal && fs::is_file(&full) {
                (paths::parent(&full).unwrap_or_else(|| full.clone()), vec![full.clone()])
            } else {
                (full.clone(), vec![])
            };
            let existing = self.ws.all_tabs().find(|id| self.tabs.get(id).is_some_and(|t| paths::same_request_path(&t.display_path(), &path)));
            if let Some(id) = existing {
                self.show_tab(id);
                if !select.is_empty() {
                    self.navigate(id, &path, NavKind::NoHistory, select);
                }
            } else if first {
                let id = self.ws.current(Side::Left).unwrap_or(0);
                self.navigate_with(id, &path, NavKind::NoHistory, select, reveal);
            } else {
                let side = self.active_side();
                let id = self.new_tab_with(side, &path, reveal);
                if !select.is_empty() {
                    self.navigate(id, &path, NavKind::NoHistory, select);
                }
            }
            first = false;
        }
        self.update_chrome();
    }

    // ------------------------------------------------------------ 命令

    fn command(&mut self, cmd: u16, code: u16, ctl: HWND) {
        if ctl == self.query && code as u32 == EN_CHANGE {
            if !self.suppress_query_change {
                let text = window_text(self.query);
                if let Some(t) = self.active_tab_mut() {
                    t.query = text;
                }
                show(self.clear_btn, self.clear_shown());
                unsafe {
                    InvalidateRect(self.hwnd, &self.layout.search, 0);
                }
            }
            return;
        }
        if (ctl == self.address || ctl == self.query) && (code as u32 == EN_SETFOCUS || code as u32 == EN_KILLFOCUS) {
            if code as u32 == EN_SETFOCUS {
                post_select_all(ctl);
            }
            if ctl == self.address && code as u32 == EN_KILLFOCUS {
                self.end_address_edit();
            }
            unsafe {
                if ctl == self.query {
                    // 焦點轉移完成後再判斷（此時 GetFocus 還不一定是新的視窗）
                    PostMessageW(self.hwnd, WM_APP_SEARCH, 0, 0);
                }
                InvalidateRect(self.hwnd, std::ptr::null(), 0);
            }
            return;
        }
        if ctl == self.address || ctl == self.query {
            return;
        }
        if self.picker_command(cmd, code, ctl) {
            return;
        }
        match cmd {
            id::BACK => self.go_back(),
            id::FORWARD => self.go_forward(),
            id::UP => self.go_up(),
            id::REFRESH => {
                let id = self.active_tab_id();
                self.refresh_tab(id);
            }
            id::CLEAR => {
                let cancelling = self.active_tab().is_some_and(|t| t.working());
                self.clear_or_cancel();
                if !cancelling {
                    // 清掉搜尋就回到清單，搜尋收回成按鈕
                    self.focus_list(self.active_side());
                    self.sync_search();
                }
            }
            id::SEARCH => self.open_search(),
            id::SORT_MENU => self.sort_menu(),
            id::UNDO => self.undo(),
            id::SEARCH_GLOBAL => {
                self.search_global = controls::is_checked(self.global_btn);
                // 已經有搜尋（或正在搜尋）就用新的範圍重新搜尋
                let rerun = self.active_tab().is_some_and(|t| (t.search_mode || t.searching) && !window_text(self.query).trim().is_empty());
                if rerun {
                    self.start_search();
                }
                self.update_chrome();
            }
            id::QUICKLOOK => {
                let side = self.active_side();
                self.toggle_quicklook(side);
            }
            id::VIEW_MENU => self.view_menu(),
            id::SHOW_COMPACT => self.toggle_compact(),
            id::SHOW_EXTENSIONS => self.toggle_extensions(),
            c if (id::VIEW_FIRST..id::VIEW_FIRST + 8).contains(&c) => {
                if let Some(mode) = feather_core::view::ViewMode::from_digit((c - id::VIEW_FIRST + 1) as u8) {
                    self.set_view_mode(mode);
                }
            }
            c if (id::SORT_NAME..=id::SORT_DESC).contains(&c) => self.sort_command(c),
            id::THEME => self.toggle_theme(),
            id::NEW_FOLDER => self.new_folder(),
            id::CUT => self.clipboard_files(true),
            id::COPY => self.clipboard_files(false),
            id::PASTE => self.paste(),
            id::RENAME => self.begin_rename(),
            id::DELETE => self.delete_selection(),
            id::DUAL => {
                let v = !self.ws.dual;
                self.set_dual(v);
            }
            id::COPY_ACROSS => self.copy_across(),
            id::HIDDEN => self.toggle_hidden(),
            id::PROPERTIES => self.properties(),
            id::SETTINGS => self.open_settings(),
            id::HELP => self.show_help(),
            id::OPEN => self.open_selection(self.active_side()),
            id::OPEN_NEW_TAB => self.open_in_new_tab(),
            id::OPEN_LOCATION => self.open_location(),
            id::OPEN_POWERSHELL => self.open_powershell_here(),
            id::COPY_PATH => self.copy_path(),
            id::SELECT_ALL => self.select_all(self.active_side()),
            id::PIN_DROPPED => {
                let dirs = std::mem::take(&mut self.pending_pins);
                self.add_favorites(dirs);
            }
            _ => {}
        }
        self.update_chrome();
    }

    fn new_folder(&mut self) {
        if self.op_busy {
            return;
        }
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.path.is_empty() || tab.working() {
            return;
        }
        let dir = tab.path.clone();
        match fileops::create_new_folder(&dir) {
            Ok(path) => {
                if let Some(t) = self.tabs.get_mut(&id) {
                    t.pending_rename = true;
                }
                self.navigate(id, &dir, NavKind::NoHistory, vec![path]);
            }
            Err(e) => self.show_error(&trf!("無法新增資料夾：{e}", "Couldn't create the folder: {e}")),
        }
    }

    fn clipboard_files(&mut self, cut: bool) {
        let side = self.active_side();
        let files = self.selected_paths(side);
        self.clipboard_paths(files, cut);
    }

    /// 把指定的檔案或資料夾放上剪貼簿（清單的選取或側欄的資料夾）。
    pub(super) fn clipboard_paths(&mut self, files: Vec<String>, cut: bool) {
        if files.is_empty() {
            return;
        }
        match super::clipboard::set_files(self.hwnd, &files, cut) {
            Ok(()) => {
                self.update_chrome();
                self.set_footer(trn!(files.len(); "已{} {} 個項目，請至目的地貼上", "{} {} item. Go to the destination and paste it.", "{} {} items. Go to the destination and paste them.", if cut { tr!("剪下", "Cut") } else { tr!("複製", "Copied") }, files.len()));
            }
            Err(e) => self.show_error(&e),
        }
    }

    fn paste(&mut self) {
        if self.op_busy {
            return;
        }
        let Some(dest) = self.active_tab().filter(|t| !t.working() && !t.path.is_empty()).map(|t| t.path.clone()) else { return };
        match super::clipboard::get_files(self.hwnd) {
            Ok(Some((files, moving))) if !files.is_empty() => {
                let cut = moving.then(|| files.clone());
                if self.run_operation(if moving { OpKind::Move } else { OpKind::Copy }, files, Some(dest)) {
                    self.paste_cut = cut;
                }
            }
            Ok(_) => {
                self.set_footer(tr!("剪貼簿中沒有檔案", "There are no files on the clipboard").into());
            }
            Err(e) => self.show_error(&e),
        }
    }

    fn copy_across(&mut self) {
        if self.op_busy {
            return;
        }
        let side = self.active_side();
        let files = self.selected_paths(side);
        if files.is_empty() {
            self.set_footer(tr!("請先選取要複製的檔案或資料夾", "First select the files or folders to copy").into());
            return;
        }
        if !self.ws.dual {
            self.set_dual(true);
            self.set_footer(tr!("已開啟雙欄；右欄讀取完成後請再按一次「複製到另一欄」", "Dual pane is on. Once the right pane finishes loading, click “Copy to the other pane” again").into());
            return;
        }
        let other = side.other();
        let Some(dest_tab) = self.ws.current(other).and_then(|id| self.tabs.get(&id)) else { return };
        if dest_tab.loading || dest_tab.path.is_empty() {
            return;
        }
        let dest = dest_tab.path.clone();
        self.run_operation(OpKind::Copy, files, Some(dest));
    }

    fn delete_selection(&mut self) {
        if self.op_busy {
            return;
        }
        let side = self.active_side();
        if self.active_tab().is_none_or(|t| t.working()) {
            return;
        }
        let files = self.selected_paths(side);
        // 刪除後選取原本的下一個項目（沒有下一個就選上一個），鍵盤可以接著操作，與檔案總管相同。
        let next = self.row_after_selection(side);
        let tab = self.active_tab_id();
        if self.delete_paths(files) {
            self.after_delete = next.map(|p| (tab, p));
        }
    }

    /// 確認後把指定的項目移到資源回收筒。回傳是否開始了刪除。
    pub(super) fn delete_paths(&mut self, files: Vec<String>) -> bool {
        if self.op_busy || files.is_empty() {
            return false;
        }
        let names: Vec<String> = files.iter().take(5).map(|f| paths::file_name(f).to_string()).collect();
        let (heading, body) = if files.len() == 1 {
            (trf!("要把「{}」移到資源回收筒嗎？", "Move “{}” to the Recycle Bin?", names[0]), tr!("之後仍可從資源回收筒還原。", "You can still restore it from the Recycle Bin later.").to_string())
        } else {
            let more = if files.len() > 5 { trf!("\n…等 {} 個項目", "\n…and {} more", files.len()) } else { String::new() };
            (trf!("要把 {} 個項目移到資源回收筒嗎？", "Move {} items to the Recycle Bin?", files.len()), trf!("{}{}\n\n之後仍可從資源回收筒還原。", "{}{}\n\nYou can still restore them from the Recycle Bin later.", names.join("\n"), more))
        };
        if !dialog::confirm(self.hwnd, Icon::Delete, &heading, &body, tr!("移到資源回收筒", "Move to Recycle Bin")) {
            return false;
        }
        self.run_operation(OpKind::Delete, files, None);
        true
    }

    /// 開始背景檔案作業；回傳是否真的開始了（移動要先確認，使用者可能取消）。
    fn run_operation(&mut self, kind: OpKind, sources: Vec<String>, dest: Option<String>) -> bool {
        if self.op_busy || sources.is_empty() {
            return false;
        }
        if kind == OpKind::Move {
            let heading = trn!(sources.len(); "要移動 {} 個項目嗎？", "Move {} item?", "Move {} items?", sources.len());
            let body = trf!("移動到：\n{}", "Move to:\n{}", dest.as_deref().unwrap_or(""));
            if !dialog::confirm(self.hwnd, Icon::Question, &heading, &body, tr!("移動", "Move")) {
                return false;
            }
        }
        self.op_busy = true;
        self.set_footer(trn!(sources.len(); "正在{} {} 個項目…", "Starting to {} {} item…", "Starting to {} {} items…", kind.verb(), sources.len()));
        self.prepare_undo(kind, &sources, dest.as_deref());
        if kind != OpKind::Copy {
            self.op_sources = sources.clone();
        }
        fileops::start(kind, sources, dest, self.hwnd);
        true
    }

    /// 搜尋結果中已被移走或刪除的項目：從結果拿掉（搜尋結果不會重新讀取資料夾）。
    fn drop_gone_search_results(&mut self, sources: &[String]) {
        if sources.is_empty() {
            return;
        }
        let gone: std::collections::HashSet<String> =
            sources.iter().filter(|p| fs::attributes(p).is_none()).map(|p| paths::trim_end_sep(p).to_lowercase()).collect();
        if gone.is_empty() {
            return;
        }
        for side in [Side::Left, Side::Right] {
            let Some(id) = self.bound[side.index()] else { continue };
            let Some(tab) = self.tabs.get_mut(&id) else { continue };
            if !tab.search_mode {
                continue;
            }
            let before = tab.entries.len();
            tab.entries.retain(|e| !gone.contains(&e.full_path().to_lowercase()));
            if tab.entries.len() != before {
                tab.resummarize();
                tab.refilter();
                self.refresh_list(side, true);
            }
        }
        // 沒顯示在畫面上的搜尋分頁也一併更新
        for tab in self.tabs.values_mut().filter(|t| t.search_mode) {
            let before = tab.entries.len();
            tab.entries.retain(|e| !gone.contains(&e.full_path().to_lowercase()));
            if tab.entries.len() != before {
                tab.resummarize();
                tab.refilter();
            }
        }
    }

    fn on_file_op(&mut self, report: OpReport) {
        self.op_busy = false;
        let sources = std::mem::take(&mut self.op_sources);
        let paste_cut = self.paste_cut.take();
        let after_delete = self.after_delete.take();
        if !report.undo {
            // 剪下貼上完成：清空剪貼簿，避免再貼一次
            if let Some(files) = paste_cut.filter(|_| report.kind == OpKind::Move && report.completed > 0) {
                super::clipboard::clear_cut_files(self.hwnd, &files);
            }
            self.drop_gone_search_results(&sources);
        }
        // 從側欄刪除的資料夾：不在了就把節點拿掉
        if let Some((item, path)) = self.pending_tree_delete.take() {
            if !fs::is_dir(&path) && self.nodes.get(&item).is_some_and(|n| paths::eq_ignore_case(&n.path, &path)) {
                send(self.tree, TVM_DELETEITEM, 0, item);
                self.nodes.remove(&item);
            }
        }
        let select = if report.undo {
            std::mem::take(&mut self.undo_select)
        } else {
            self.finish_undo_record();
            match after_delete {
                // 只在刪除時所在的分頁還顯示著同一個資料夾時選取（refresh_after_undo 依資料夾比對）
                Some((tab, path)) if report.kind == OpKind::Delete && self.tabs.contains_key(&tab) => vec![path],
                _ => Vec::new(),
            }
        };
        self.refresh_after_undo(&select);
        self.update_chrome();
        if report.undo {
            self.set_footer(trf!(
                "已復原{} · 還原 {} 項{}", "Undid {} · restored {}{}",
                report.kind.verb(),
                report.completed,
                if report.skipped > 0 { trf!(" · {} 項無法還原", " · {} couldn't be restored", report.skipped) } else { String::new() }
            ));
        } else {
            self.set_footer(trf!(
                "{}{} · 完成 {} 項 · 略過 {} 項 · 錯誤 {} 項{}", "{1} {0} · {2} done · {3} skipped · {4} errors{5}",
                report.kind.verb(),
                if report.cancelled { tr!("已取消", "Cancelled") } else { tr!("結束", "Finished") },
                report.completed,
                report.skipped,
                report.errors.len(),
                if report.completed > 0 && report.kind != OpKind::Copy { tr!(" · Ctrl+Z 可復原", " · Ctrl+Z to undo") } else { "" }
            ));
        }
        if !report.errors.is_empty() {
            let mut text: Vec<String> = report.errors.iter().take(8).cloned().collect();
            if report.errors.len() > 8 {
                text.push(trf!("…等 {} 項", "…and {} more", report.errors.len()));
            }
            let heading = if report.undo {
                trn!(report.errors.len(); "復原{}時有 {} 個項目沒有還原", "Undo {0}: {1} item wasn't restored", "Undo {0}: {1} items weren't restored", report.kind.verb(), report.errors.len())
            } else {
                trn!(report.errors.len(); "{}時有 {} 個項目沒有完成", "{1} item didn't finish ({0})", "{1} items didn't finish ({0})", report.kind.verb(), report.errors.len())
            };
            dialog::Spec::new(Icon::Warning, &heading).body(&text.join("\n")).width(500).show(self.hwnd);
        }
    }

    /// 檔案操作或復原之後重新讀取畫面上的分頁；`select` 所在資料夾的分頁會選取這些項目。
    pub(super) fn refresh_after_undo(&mut self, select: &[String]) {
        for side in self.ws.visible_sides().to_vec() {
            let Some(id) = self.ws.current(side) else { continue };
            let Some(tab) = self.tabs.get(&id) else { continue };
            if tab.search_mode {
                continue;
            }
            let here: Vec<String> = select
                .iter()
                .filter(|p| paths::parent(p).is_some_and(|d| paths::eq_ignore_case(paths::trim_end_sep(&d), paths::trim_end_sep(&tab.path))))
                .cloned()
                .collect();
            if here.is_empty() {
                self.refresh_tab(id);
            } else {
                let dir = tab.path.clone();
                self.navigate(id, &dir, NavKind::NoHistory, here);
            }
        }
        // 隱藏中的分頁切回時再讀取。
        let visible: Vec<TabId> = self.ws.visible_sides().iter().filter_map(|s| self.ws.current(*s)).collect();
        for (id, t) in self.tabs.iter_mut() {
            if !visible.contains(id) && !t.path.is_empty() {
                t.stale = true;
            }
        }
    }

    pub(super) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        // 整體設定：記到設定檔（不分資料夾）
        let mut new = self.settings.clone();
        new.show_hidden = self.show_hidden;
        match write_settings(&new) {
            Ok(()) => self.settings = new,
            Err(e) => {
                self.settings.show_hidden = self.show_hidden;
                self.set_footer(trf!("隱藏項目已切換，但無法儲存設定：{e}", "Hidden items were toggled, but the settings couldn't be saved: {e}"));
            }
        }
        let visible: Vec<TabId> = self.ws.visible_sides().iter().filter_map(|s| self.ws.current(*s)).collect();
        for (id, t) in self.tabs.iter_mut() {
            if !visible.contains(id) && !t.path.is_empty() {
                t.stale = true;
            }
        }
        for id in visible {
            if self.tabs.get(&id).is_some_and(|t| !t.search_mode) {
                self.refresh_tab(id);
            }
        }
        self.build_tree();
        self.update_chrome();
    }

    fn properties(&mut self) {
        let side = self.active_side();
        // 焦點在側欄（Alt+Enter）：顯示側欄選取的資料夾
        let tree = (unsafe { GetFocus() } == self.tree).then(|| self.tree_caret_path()).flatten();
        let target = tree
            .or_else(|| self.selected_paths(side).into_iter().next())
            .or_else(|| self.active_tab().map(|t| t.path.clone()).filter(|p| !p.is_empty()));
        if let Some(p) = target {
            fileops::show_properties(self.hwnd, &p);
        }
    }

    fn copy_path(&mut self) {
        let side = self.active_side();
        let mut files = self.selected_paths(side);
        if files.is_empty() {
            if let Some(p) = self.active_tab().map(|t| t.path.clone()).filter(|p| !p.is_empty()) {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        match super::clipboard::set_text(self.hwnd, &files.join("\r\n")) {
            Ok(()) => {
                self.update_chrome();
                self.set_footer(tr!("已複製路徑", "Path copied").into());
            }
            Err(e) => self.show_error(&e),
        }
    }

    fn open_in_new_tab(&mut self) {
        let side = self.active_side();
        let Some(entry) = self.selected_entries(side).into_iter().next() else { return };
        let path = if entry.is_dir { entry.full_path() } else { entry.dir.to_string() };
        self.new_tab(side, &path);
    }

    /// 清單右鍵選單的「在這裡開啟 PowerShell 視窗」：選了一個資料夾時開在它裡面，否則開在目前的資料夾
    fn open_powershell_here(&mut self) {
        let side = self.active_side();
        let entries = self.selected_entries(side);
        let folder = match entries.as_slice() {
            [e] if e.is_dir => Some(e.full_path()),
            _ => self.active_tab().filter(|t| !t.search_mode && !t.path.is_empty()).map(|t| t.path.clone()),
        };
        if let Some(folder) = folder {
            self.open_powershell(&folder);
        }
    }

    pub(super) fn open_powershell(&mut self, folder: &str) {
        if let Err(e) = super::fileops::open_powershell(self.hwnd, folder) {
            self.show_error(&e);
        }
    }

    fn open_location(&mut self) {
        let side = self.active_side();
        let Some(entry) = self.selected_entries(side).into_iter().next() else { return };
        let id = self.active_tab_id();
        self.navigate(id, &entry.dir.to_string(), NavKind::Normal, vec![entry.full_path()]);
    }

    pub(super) fn open_selection(&mut self, side: Side) {
        // 選擇器：資料夾就進去，檔案就選定（不用預設程式開啟）
        if self.picker_mode() {
            self.picker_activate(side);
            return;
        }
        let Some(tab) = self.ws.current(side).and_then(|id| self.tabs.get(&id)) else { return };
        if tab.loading {
            return;
        }
        let entries = self.selected_entries(side);
        let Some(entry) = entries.first().cloned() else { return };
        if entries.len() > 1 {
            self.open_many(side, entries);
            return;
        }
        if entry.is_dir {
            let id = self.ws.current(side).unwrap_or(0);
            self.navigate(id, &entry.full_path(), NavKind::Normal, vec![]);
        } else if let Err(e) = fileops::shell_open(self.hwnd, &entry.full_path()) {
            if let Some(t) = self.ws.current(side).and_then(|id| self.tabs.get_mut(&id)) {
                t.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
            }
            self.update_chrome();
        }
    }

    /// 選了多個項目按 Enter（或雙擊）：檔案全部用預設程式開啟，資料夾各開一個新分頁；
    /// 與檔案總管相同，超過 15 個先確認。
    fn open_many(&mut self, side: Side, entries: Vec<Entry>) {
        const ASK_ABOVE: usize = 15;
        if entries.len() > ASK_ABOVE {
            let heading = trf!("要開啟 {} 個項目嗎？", "Open {} items?", entries.len());
            let body = tr!("每個檔案都會用預設程式開啟，資料夾會各開一個新分頁。", "Each file opens in its default app, and each folder opens in a new tab.");
            if !dialog::confirm(self.hwnd, Icon::Question, &heading, body, tr!("全部開啟", "Open all")) {
                return;
            }
        }
        let mut failed: Vec<String> = Vec::new();
        for e in entries.iter().filter(|e| !e.is_dir) {
            if let Err(err) = fileops::shell_open(self.hwnd, &e.full_path()) {
                failed.push(trf!("{}：{err}", "{}: {err}", e.name));
            }
        }
        for e in entries.iter().filter(|e| e.is_dir) {
            self.new_tab(side, &e.full_path());
        }
        if !failed.is_empty() {
            let mut text: Vec<String> = failed.iter().take(8).cloned().collect();
            if failed.len() > 8 {
                text.push(trf!("…等 {} 項", "…and {} more", failed.len()));
            }
            dialog::Spec::new(Icon::Warning, &trn!(failed.len(); "有 {} 個項目無法開啟", "{} item couldn't be opened", "{} items couldn't be opened", failed.len())).body(&text.join("\n")).width(500).show(self.hwnd);
        }
        self.update_chrome();
    }

    pub(super) fn add_favorites(&mut self, candidates: Vec<String>) {
        let mut added = 0;
        let mut ignored = 0;
        for p in candidates {
            let p = paths::trim_end_sep(&p).to_string();
            if !fs::is_dir(&p) || !paths::is_fully_qualified(&p) {
                ignored += 1;
                continue;
            }
            if self.settings.favorites.iter().any(|f| paths::eq_ignore_case(f, &p)) {
                continue;
            }
            if self.settings.favorites.len() >= feather_core::settings::MAX_FAVORITES {
                ignored += 1;
                continue;
            }
            self.settings.favorites.push(p);
            added += 1;
        }
        if added > 0 {
            if let Err(e) = write_settings(&self.settings) {
                self.show_error(&trf!("常用位置未儲存：{e}", "Favorites weren't saved: {e}"));
            }
            self.build_tree();
        }
        self.update_chrome();
        self.set_footer(trn!(
            added;
            "已釘選 {added} 個資料夾到常用位置{}", "Pinned {added} folder to Favorites{}", "Pinned {added} folders to Favorites{}",
            if ignored > 0 { trf!("，{ignored} 個不是資料夾或已達上限", "; {ignored} weren't folders or the limit was reached") } else { String::new() }
        ));
    }

    pub(super) fn remove_favorite(&mut self, path: &str) {
        self.remove_favorites(&[path.to_string()]);
    }

    /// 從常用位置取消釘選（資料夾本身不動）。
    pub(super) fn remove_favorites(&mut self, targets: &[String]) {
        let before = self.settings.favorites.len();
        self.settings.favorites.retain(|f| !targets.iter().any(|t| paths::eq_ignore_case(f, paths::trim_end_sep(t))));
        let removed = before - self.settings.favorites.len();
        if removed > 0 {
            if let Err(e) = write_settings(&self.settings) {
                self.show_error(&trf!("常用位置未儲存：{e}", "Favorites weren't saved: {e}"));
            }
            self.build_tree();
            self.update_chrome();
            self.set_footer(trn!(removed; "已從常用位置取消釘選 {removed} 個資料夾（資料夾本身沒有刪除）", "Unpinned {removed} folder from Favorites (the folder itself wasn't deleted)", "Unpinned {removed} folders from Favorites (the folders themselves weren't deleted)"));
        }
    }

    /// 這個資料夾是否已釘選在常用位置。
    pub(super) fn is_pinned(&self, path: &str) -> bool {
        let p = paths::trim_end_sep(path);
        self.settings.favorites.iter().any(|f| paths::eq_ignore_case(f, p))
    }

    /// 右上角月亮／太陽：在淺色與深色之間切換並儲存。
    fn toggle_theme(&mut self) {
        let mut new = self.settings.clone();
        new.theme_mode = if self.theme.pal.dark { ThemeMode::Light } else { ThemeMode::Dark };
        match write_settings(&new) {
            Ok(()) => self.apply_settings(new),
            Err(e) => self.show_error(&trf!("設定未儲存：{e}", "Settings weren't saved: {e}")),
        }
    }

    /// 篩選籤列：類型篩選、排序、搜尋含子資料夾。
    fn chip_action(&mut self, wparam: WPARAM) {
        let side = self.active_side();
        let value = wparam >> 8;
        match wparam & 0xFF {
            chips::ACTION_FILTER => {
                let Some(c) = chips::category_from_index(value) else { return };
                let id = self.active_tab_id();
                let changes = self.tabs.get(&id).is_some_and(|t| t.filter != c) && self.bound[side.index()] == Some(id);
                let _fade = if changes { self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS) } else { anim::Crossfade::none() };
                let selected = self.selected_paths(side);
                let Some(tab) = self.tabs.get_mut(&id) else { return };
                tab.filter = c;
                tab.refilter();
                tab.view = ViewState::default();
                tab.pending_select = selected;
                self.hide_preview();
                if self.bound[side.index()] == Some(id) {
                    self.refresh_list(side, true);
                }
            }
            chips::ACTION_SORT => {
                if let Some(sort) = chips::sort_from_index(value) {
                    let (col, desc) = sort.column();
                    self.sort_to(side, col, desc);
                }
            }
            chips::ACTION_RECURSIVE => {
                if let Some(t) = self.active_tab_mut() {
                    t.recursive = !t.recursive;
                }
            }
            _ => {}
        }
        self.update_chrome();
    }

    /// 麵包屑：前往上層資料夾（並選取原本所在的子資料夾），或切換成輸入模式。
    fn crumb_action(&mut self, index: usize) {
        if index == crumbs::EDIT {
            self.begin_address_edit();
            return;
        }
        let segments = crumb_model();
        let Some((_, target)) = segments.get(index) else { return };
        let select: Vec<String> = segments.get(index + 1).map(|s| vec![s.1.clone()]).unwrap_or_default();
        let id = self.active_tab_id();
        self.navigate(id, &target.clone(), NavKind::Normal, select);
    }

    fn open_settings(&mut self) {
        self.hide_preview();
        let current = self.settings.clone();
        let exe = current_exe_path();
        // 「攔截 Win+E」與「其他程式開啟或儲存檔案時也用 Stillfeather」顯示使用者的選擇（沒選過時：攔截 Win+E 開啟、開檔視窗關閉）
        let was = settings_dialog::Integration {
            default_fm: exe.as_deref().is_some_and(shellreg::is_default_file_manager),
            win_e: shellreg::win_e_choice(),
            picker: shellreg::picker_choice(),
        };
        if let Some((new, want)) = settings_dialog::show(self.hwnd, &current, was) {
            let english = new.language.english(system_ui_chinese());
            let saved = match write_settings(&new) {
                Ok(()) => {
                    self.apply_settings(new);
                    true
                }
                Err(e) => {
                    self.show_error(&trf!("設定未儲存，保留先前設定：{e}", "Settings weren't saved; the previous settings are kept: {e}"));
                    false
                }
            };
            if want.win_e != was.win_e {
                let _ = shellreg::set_win_e_choice(want.win_e);
            }
            if want.default_fm != was.default_fm {
                self.set_default_file_manager(want.default_fm);
            } else if want.win_e != was.win_e && want.default_fm {
                self.set_footer(if want.win_e { tr!("Win+E 會開 Stillfeather（在背景攔截按鍵）", "Win+E opens Stillfeather (the key is intercepted in the background)").into() } else { tr!("已停止攔截 Win+E", "Stopped intercepting Win+E").into() });
            }
            if want.default_fm != was.default_fm || want.win_e != was.win_e {
                // 依「設為預設檔案總管」與「攔截 Win+E」啟動或停止背景攔截
                super::wine::maintain();
            }
            if want.picker != was.picker {
                self.set_picker_enabled(want.picker);
            }
            // 換了語言：重新開啟才套用（畫面上的文字都是建立時就決定的）
            if saved && english != feather_core::i18n::english() {
                self.offer_language_restart(english);
            }
        }
        // 在設定裡按「檢查更新」找到新版本：現在提醒
        self.after_settings_closed();
    }

    /// 其他程式的「在資料夾中顯示」要選取 `path`（見 shellwin.rs）：作用中的分頁就是它所在的資料夾時在這個分頁選取
    /// （清單上已經有它就直接選取；剛下載好、清單還沒更新到時重新讀取資料夾再選取），否則照一般的開啟要求
    /// （開它所在的資料夾並選取）。`replace`：取消其他的選取。視窗拉到前面。
    pub(super) fn shell_select(&mut self, path: String, replace: bool) {
        let side = self.active_side();
        let id = self.active_tab_id();
        let Some(folder) = paths::parent(&path) else { return };
        let here = self.tabs.get(&id).is_some_and(|t| !t.search_mode && paths::same_path(&folder, &t.path));
        if !here {
            super::shellwin::log(&format!("選取 {path}：不在作用中的資料夾，照一般的開啟要求"));
            self.open_paths(vec![path], false, false);
            return;
        }
        let front = unsafe {
            if IsIconic(self.hwnd) != 0 {
                ShowWindow(self.hwnd, SW_RESTORE);
            }
            SetForegroundWindow(self.hwnd) != 0
        };
        if !front {
            super::shellwin::log("無法把視窗拉到前面（Windows 不允許）");
        }
        let Some(t) = self.tabs.get(&id) else { return };
        // 同一次要求選取好幾個時會連續呼叫：讀取中就接在還沒套用的選取後面
        let mut selected = if replace {
            Vec::new()
        } else if t.loading {
            t.pending_select.clone()
        } else {
            self.selected_paths(side)
        };
        if !selected.iter().any(|p| paths::same_path(p, &path)) {
            selected.push(path.clone());
        }
        let shown = !t.loading && (0..t.row_count()).any(|i| t.row(i).is_some_and(|e| paths::same_path(&e.full_path(), &path)));
        super::shellwin::log(&format!("選取 {path}（{}）", if shown { "清單上已經有" } else { "重新讀取資料夾再選取" }));
        if shown {
            if let Some(t) = self.tabs.get_mut(&id) {
                t.pending_select = selected;
            }
            self.refresh_list(side, true);
        } else {
            let folder = self.tabs[&id].path.clone();
            self.navigate(id, &folder, NavKind::NoHistory, selected);
        }
        self.focus_list(side);
    }

    /// 問要不要現在重新開啟，套用新的語言。問題用新的語言寫（使用者剛選了它）。
    fn offer_language_restart(&mut self, english: bool) {
        let old = feather_core::i18n::english();
        feather_core::i18n::set_english(english);
        let body = if self.settings.restore_tabs {
            tr!("新的語言會在重新開啟 Stillfeather 後套用。分頁會照現在的樣子開回來。", "The new language applies after Stillfeather restarts. Your tabs will be reopened as they are now.")
        } else {
            tr!("新的語言會在重新開啟 Stillfeather 後套用。", "The new language applies after Stillfeather restarts.")
        };
        let choice = dialog::Spec::new(Icon::Question, tr!("要現在重新開啟嗎？", "Restart now?"))
            .body(body)
            .buttons(vec![dialog::Button::primary(1, tr!("立即重新開啟", "Restart now")), dialog::Button::normal(0, tr!("稍後", "Later"))], 0)
            .show(self.hwnd);
        feather_core::i18n::set_english(old);
        if choice == 1 {
            self.restart();
        } else {
            self.set_footer(tr!("新的語言會在下次開啟 Stillfeather 時套用", "The new language applies the next time Stillfeather starts").into());
        }
    }

    /// 重新開啟 Stillfeather：先啟動新的一份（等這一份結束才接手，見 win::entry 的 --wait-pid），再關閉這個視窗。
    /// 關閉時照常記下分頁，新的一份照「還原上次的分頁」開回來。
    fn restart(&mut self) {
        let Some(exe) = current_exe_path() else { return };
        let arg = format!("{}{}", super::WAIT_PID_ARG, unsafe { GetCurrentProcessId() });
        match std::process::Command::new(&exe).arg(arg).spawn() {
            Ok(_) => unsafe {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },
            Err(e) => self.show_error(&trf!("無法重新開啟 Stillfeather：{e}", "Couldn't restart Stillfeather: {e}")),
        }
    }

    /// 現在跳出更新提醒會不會打斷使用者：Stillfeather 要在前景，而且沒有對話方塊或選單開著、
    /// 沒有在拖曳、重新命名或輸入路徑
    pub(super) fn ready_for_prompt(&self) -> bool {
        let front = unsafe { GetForegroundWindow() == self.hwnd && IsWindowEnabled(self.hwnd) != 0 };
        front
            && self.renaming.is_none()
            && self.drag.is_none()
            && !self.address_editing
            && !super::update::ui_busy()
    }

    /// 設為／取消預設檔案總管（Win+E 與開啟資料夾）。只改目前使用者的登錄，不需要系統管理員權限。
    fn set_default_file_manager(&mut self, on: bool) {
        // 先記下選擇：之後啟動時照這個選擇處理（沒選過時預設開啟）
        let _ = shellreg::set_default_fm_choice(on);
        let result = if on {
            match current_exe_path() {
                Some(exe) => shellreg::set_default_file_manager(&exe),
                None => Err(tr!("找不到 Stillfeather 程式的位置。", "Can't find where the Stillfeather program is.").into()),
            }
        } else {
            shellreg::restore_default_file_manager()
        };
        super::shellwin::refresh();
        match result {
            Ok(()) if on => self.set_footer(tr!("已設為預設檔案總管：Win+E 與開啟資料夾都會使用 Stillfeather", "Set as the default file manager: Win+E and opening folders both use Stillfeather").into()),
            Ok(()) => self.set_footer(tr!("已改回 Windows 檔案總管", "Switched back to File Explorer").into()),
            Err(e) => self.show_error(&trf!("無法變更預設檔案總管：{e}", "Couldn't change the default file manager: {e}")),
        }
    }

    pub(super) fn apply_settings(&mut self, new: Settings) {
        let _fade = self.crossfade(vec![client_rect(self.hwnd)], anim::THEME_MS);
        let compact_changed = new.compact_rows != self.settings.compact_rows;
        let favorites_changed = new.favorites != self.settings.favorites;
        self.settings = new;
        self.theme.set_palette(&self.settings);
        if compact_changed {
            self.rebuild_icons();
        }
        self.apply_theme_to_controls();
        if favorites_changed {
            self.build_tree();
        }
        self.relayout();
        for side in [Side::Left, Side::Right] {
            self.sync_view(side);
            invalidate(self.lists[side.index()]);
        }
        // 「按空白鍵預覽」可能切換了：名稱提示跟著開關
        self.sync_label_tips();
        self.update_chrome();
    }

    fn rebuild_icons(&mut self) {
        let mut old = std::mem::replace(&mut self.icons, Icons::new(self.theme.dpi, self.settings.compact_rows));
        for list in self.lists {
            send(list, LVM_SETIMAGELIST, LVSIL_SMALL, self.icons.list);
        }
        send(self.tree, TVM_SETIMAGELIST, TVSIL_NORMAL, self.icons.tree);
        // 大圖示清單也跟著新的 DPI 重建；清單改用新的之後才銷毀舊的
        self.apply_view();
        old.destroy();
        self.build_tree();
    }

    fn on_dpi_changed(&mut self, dpi: u32, suggested: &RECT) {
        self.theme.set_dpi(dpi);
        self.apply_fonts();
        self.rebuild_icons();
        unsafe {
            SetWindowPos(self.hwnd, 0, suggested.left, suggested.top, suggested.width(), suggested.height(), SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.relayout();
        self.apply_theme_to_controls();
    }

    fn system_theme_changed(&mut self) {
        if self.settings.theme_mode == feather_core::settings::ThemeMode::System {
            let _fade = self.crossfade(vec![client_rect(self.hwnd)], anim::THEME_MS);
            self.theme.set_palette(&self.settings);
            self.apply_theme_to_controls();
            self.update_chrome();
        }
    }

    /// 執行了 Windows 右鍵選單的命令（新增、建立捷徑、解壓縮…）：檔案可能有變動，馬上重新整理一次，
    /// 稍後再整理一次給在背景完成的命令。
    pub(super) fn after_shell_command(&mut self) {
        self.refresh_after_shell();
        unsafe {
            SetTimer(self.hwnd, TIMER_SHELL_REFRESH, 1200, std::ptr::null());
        }
    }

    fn refresh_after_shell(&mut self) {
        for side in [Side::Left, Side::Right] {
            if side == Side::Right && !self.ws.dual {
                continue;
            }
            let Some(id) = self.bound[side.index()] else { continue };
            if self.tabs.get(&id).is_some_and(|t| !t.search_mode && !t.loading && !t.path.is_empty()) {
                self.refresh_tab(id);
            }
        }
    }

    /// 在清單或樹狀圖打字：跳到名稱開頭相符的項目；找不到就安靜地不動。
    fn type_to_find(&mut self, target: HWND, c: char) {
        let now = anim::now();
        if c == ' ' && !self.typeahead.active(now) {
            return;
        }
        let query = self.typeahead.push(c, now);
        if let Some(side) = self.side_of_list(target) {
            self.list_type_ahead(side, &query);
        } else if target == self.tree {
            self.tree_type_ahead(&query);
        }
    }

    fn show_help(&self) {
        let (hover_key, hover_text, space_key, space_text) = if self.settings.space_preview {
            (tr!("滑鼠停在項目上＋按住空白鍵", "Hover over an item + hold Space"), tr!("快速預覽滑鼠下的項目，放開就關閉（設定：按空白鍵預覽）", "Quick preview the item under the mouse; release to close (setting: Preview with Space)"), tr!("右鍵 › 快速預覽", "Right-click › Quick preview"), tr!("預覽選取的項目並維持開著", "Preview the selected item and keep it open"))
        } else {
            (tr!("滑鼠停在項目上", "Hover over an item"), tr!("快速預覽圖片、文字、程式碼、PDF 與影片，移開就關閉", "Quick preview images, text, code, PDFs and videos; move away to close"), tr!("按住空白鍵", "Hold Space"), tr!("放大預覽（懸停的項目或選取的項目），放開就關閉", "Enlarge the preview (the hovered or selected item); release to close"))
        };
        let rows: Vec<(String, String)> = [
            (tr!("Ctrl+T、Ctrl+N", "Ctrl+T, Ctrl+N"), tr!("新分頁", "New tab")),
            ("Ctrl+W", tr!("關閉分頁（分頁上按中鍵也可以）", "Close tab (or middle-click the tab)")),
            ("Ctrl+Tab", tr!("下一個分頁", "Next tab")),
            (tr!("Ctrl+Shift+PgUp／PgDn", "Ctrl+Shift+PgUp/PgDn"), tr!("調整分頁順序", "Reorder tabs")),
            (hover_key, hover_text),
            (space_key, space_text),
            ("Ctrl+Z", tr!("復原重新命名、移動或刪除", "Undo rename, move or delete")),
            ("Ctrl+Shift+D", tr!("單欄／雙欄", "Single/dual pane")),
            (tr!("Ctrl+Shift+1～8", "Ctrl+Shift+1–8"), tr!("超大圖示、大圖示…詳細資料、並排、內容", "Extra large icons, large icons… details, tiles, content")),
            (tr!("Ctrl+L、Alt+D", "Ctrl+L, Alt+D"), tr!("編輯路徑", "Edit path")),
            ("Ctrl+F", tr!("搜尋檔名", "Search file names")),
            (tr!("Alt+←／→／↑", "Alt+←/→/↑"), tr!("上一頁／下一頁／上一層", "Back/forward/up")),
            ("F5", tr!("重新整理", "Refresh")),
            ("F2", tr!("重新命名", "Rename")),
            (tr!("Delete、Ctrl+D", "Delete, Ctrl+D"), tr!("刪除到資源回收筒", "Delete to Recycle Bin")),
            (tr!("Ctrl+C／X／V", "Ctrl+C/X/V"), tr!("複製／剪下／貼上", "Copy/cut/paste")),
            ("Alt+Enter", tr!("內容", "Properties")),
            (tr!("F12、Ctrl+,", "F12, Ctrl+,"), tr!("設定", "Settings")),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        dialog::Spec::new(Icon::Info, "Stillfeather")
            .body(tr!("Rust／原生 Win32 版 · 常用快捷鍵", "Rust / native Win32 · Keyboard shortcuts"))
            .rows(rows)
            .footnote(
                tr!("在清單直接打字可以跳到開頭相符的項目。搜尋只比對檔名，不讀取內容；勾選搜尋框裡的「全域搜尋」會搜尋整台電腦的所有本機與卸除式磁碟。目錄連結不會遞迴搜尋，最多顯示 50,000 個結果。\n\
標題右側的「排序」「檢視」可以換排序方式與檢視模式；檢視模式會記在各個資料夾（「檢視 › 套用到所有資料夾」可統一）。顯示隱藏項目、副檔名與精簡檢視在「檢視 › 顯示」，是整體設定，會一直記住。\n\
檔案與資料夾可以直接拖曳：放到資料夾、另一欄、側欄的資料夾或磁碟就會移動或複製（同一磁碟移動、不同磁碟複製；Ctrl 複製、Shift 移動、右鍵拖曳有選單）。把資料夾拖到側欄的「常用位置」標題就會釘選。滑鼠停在項目上就會快速預覽；設定中開啟「按空白鍵預覽」則改成按住空白鍵才預覽。\n\
尚未提供：索引搜尋。", "Type in the list to jump to an item that starts with what you type. Search matches file names only and doesn't read contents; check “Whole PC” in the search box to search every local and removable drive. Directory links aren't searched recursively, and up to 50,000 results are shown.\n\
Use “Sort” and “View” to the right of the title to change the sort order and view. Each folder remembers its view (“View › Apply to all folders” makes them all the same). Hidden items, file extensions and compact view are under “View › Show”; they apply everywhere and are always remembered.\n\
Drag files and folders to move or copy them: drop them on a folder, the other pane, or a folder or drive in the sidebar (same drive moves, another drive copies; hold Ctrl to copy, Shift to move, or drag with the right button for a menu). Drag a folder onto the sidebar's “Favorites” heading to pin it. Hover over an item to quick preview it; turn on “Preview with Space” in Settings to preview only while holding Space.\n\
Not available yet: indexed search."),
            )
            .width(540)
            .show(self.hwnd);
    }

    pub(super) fn show_error(&mut self, text: &str) {
        self.set_footer(tr!("無法完成操作", "Couldn't complete the operation").into());
        dialog::error(self.hwnd, tr!("無法完成操作", "Couldn't complete the operation"), text);
    }

    fn restore_focus(&self) {
        let target = if self.last_focus != 0 && unsafe { IsWindow(self.last_focus) != 0 && IsWindowVisible(self.last_focus) != 0 && IsChild(self.hwnd, self.last_focus) != 0 } {
            self.last_focus
        } else {
            self.lists[self.active_side().index()]
        };
        unsafe {
            SetFocus(target);
        }
    }

    pub(super) fn hide_preview(&mut self) {
        let owner = self.hwnd;
        preview::close(&mut self.preview, owner);
        self.sync_label_tips();
    }

    // ------------------------------------------------------------ 分隔線拖曳

    fn splitter_at(&self, x: i32, y: i32) -> Option<Drag> {
        let grow = self.s(2);
        if self.layout.sidebar_splitter.inflate(grow, 0).contains(x, y) {
            return Some(Drag::Sidebar);
        }
        if self.layout.pane_splitter.is_some_and(|r| r.inflate(grow, 0).contains(x, y)) {
            return Some(Drag::Split);
        }
        None
    }

    fn drag_to(&mut self, x: i32) {
        match self.drag {
            Some(Drag::Sidebar) => {
                let w = ((x - self.layout.sidebar.left) * 96 / self.theme.dpi as i32).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                if w != self.settings.sidebar_width {
                    self.settings.sidebar_width = w;
                    self.relayout();
                }
            }
            Some(Drag::Split) => {
                let area_left = self.layout.content_left;
                let area_w = (self.layout.content_right - area_left).max(1);
                let pct = ((x - area_left) * 100 / area_w).clamp(SPLIT_MIN, SPLIT_MAX);
                if pct != self.settings.split_percent {
                    self.settings.split_percent = pct;
                    self.relayout();
                }
            }
            None => {}
        }
    }

    fn end_drag(&mut self) {
        if self.drag.take().is_some() {
            if let Err(e) = write_settings(&self.settings) {
                self.set_footer(trf!("布局未儲存：{e}", "The layout wasn't saved: {e}"));
            }
        }
    }

    // ------------------------------------------------------------ 鍵盤

    fn handle_key(&mut self, msg: &MSG) -> bool {
        let mut vk = msg.wParam as u32;
        if vk == VK_PROCESSKEY {
            // 中文輸入法（例如注音）開著時，按鍵先交給輸入法，這裡收到的是 VK_PROCESSKEY。
            // 正在組字就交給輸入法；沒有組字時取回原本的按鍵，讓空白鍵等快捷鍵照常作用。
            if ime_composing(msg.hwnd) {
                return false;
            }
            vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = msg.message == WM_SYSKEYDOWN || key_down(VK_MENU);
        let focus = unsafe { GetFocus() };
        let in_edit = focus == self.address || focus == self.query;
        // 選擇器：檔名欄的 Enter／Esc，以及分頁、雙欄、設定這些不存在的快捷鍵
        if let Some(handled) = self.picker_key(vk, ctrl, shift, alt, focus) {
            return handled;
        }
        if !in_edit && focus != 0 && class_name(focus).eq_ignore_ascii_case("Edit") {
            return false; // 清單的就地重新命名
        }
        let letter = |c: u8| vk == c as u32;
        // 按鍵（空白鍵與 Shift／Ctrl／Alt／Windows 鍵除外）關掉懸停的預覽，滑鼠移到別的項目前不再出現
        let modifier = matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
        if vk != VK_SPACE && !modifier {
            let hovering = preview::mode(&self.preview) == Some(preview::Mode::Hover);
            self.cancel_hover();
            if hovering && vk == VK_ESCAPE {
                return true;
            }
        }
        // 「按空白鍵預覽」：按住空白鍵時滑鼠停在哪個項目就預覽哪個，放開就關閉（不看選取）。
        // 置中的預覽（右鍵選單開啟）開著時照舊由下面處理；清單正在打字找檔案時空白是名稱的一部分。
        if self.settings.space_preview
            && vk == VK_SPACE
            && !ctrl
            && !alt
            && !shift
            && !in_edit
            && !preview::mode(&self.preview).is_some_and(|m| m != preview::Mode::Hover)
            && !(self.side_of_list(focus).is_some() && self.typeahead.active(anim::now()))
            && self.space_gate_applies(focus)
        {
            let repeat = msg.lParam & (1 << 30) != 0;
            if !repeat && !self.space_gate_open() {
                self.open_space_gate();
            }
            return true;
        }
        // 快速預覽開著時 Esc 先關閉預覽
        if vk == VK_ESCAPE && preview::is_open(&self.preview) {
            self.hide_preview();
            return true;
        }
        // 選擇器：Esc 取消（在路徑或搜尋輸入框裡時，照常先離開輸入框、取消搜尋）
        if vk == VK_ESCAPE && !in_edit && self.picker_mode() {
            self.picker_finish(feather_core::picker::EXIT_CANCEL);
            return true;
        }
        // Ctrl+Z 復原檔案操作；在路徑或搜尋輸入框裡則交給輸入框自己復原文字
        if ctrl && !shift && !alt && letter(b'Z') && !in_edit {
            self.undo();
            return true;
        }
        if ctrl && !alt && (letter(b'T') || letter(b'N')) {
            self.new_tab_active();
            return true;
        }
        if ctrl && !alt && letter(b'W') {
            self.close_current_tab();
            return true;
        }
        if ctrl && vk == VK_TAB {
            self.switch_tab(if shift { -1 } else { 1 });
            return true;
        }
        if ctrl && shift && (vk == VK_PRIOR || vk == VK_NEXT) {
            let side = self.active_side();
            let index = self.ws.group(side).selected;
            self.move_tab(side, index, if vk == VK_PRIOR { -1 } else { 1 });
            return true;
        }
        // Ctrl+Shift+1～8 切換檢視（與檔案總管相同）
        if ctrl && shift && !alt && (0x31..=0x38).contains(&vk) {
            if let Some(mode) = feather_core::view::ViewMode::from_digit((vk - 0x30) as u8) {
                self.set_view_mode(mode);
            }
            return true;
        }
        // Ctrl+Shift+D 切換雙欄；單獨的 Ctrl+D 與檔案總管相同是刪除（見下方清單按鍵）。
        if ctrl && shift && !alt && letter(b'D') {
            let v = !self.ws.dual;
            self.set_dual(v);
            return true;
        }
        if vk == VK_F12 || (ctrl && vk == VK_OEM_COMMA) {
            self.open_settings();
            return true;
        }
        if (ctrl && letter(b'L')) || (alt && letter(b'D')) {
            self.begin_address_edit();
            return true;
        }
        if ctrl && letter(b'F') {
            self.open_search();
            return true;
        }
        if vk == VK_F5 {
            let id = self.active_tab_id();
            self.refresh_tab(id);
            return true;
        }
        // 鍵盤上的「上一頁」「下一頁」鍵
        if vk == VK_BROWSER_BACK || vk == VK_BROWSER_FORWARD {
            if vk == VK_BROWSER_BACK {
                self.go_back();
            } else {
                self.go_forward();
            }
            return true;
        }
        if alt && !ctrl {
            match vk {
                VK_LEFT => {
                    self.go_back();
                    return true;
                }
                VK_RIGHT => {
                    self.go_forward();
                    return true;
                }
                VK_UP => {
                    self.go_up();
                    return true;
                }
                VK_RETURN if !in_edit => {
                    self.properties();
                    return true;
                }
                _ => {}
            }
        }
        if vk == VK_TAB && !ctrl && !alt {
            let next = unsafe { GetNextDlgTabItem(self.hwnd, focus, shift as BOOL) };
            if next != 0 {
                unsafe {
                    SetFocus(next);
                }
            }
            return true;
        }
        if in_edit {
            match vk {
                VK_RETURN => {
                    if focus == self.address {
                        let text = window_text(self.address);
                        let id = self.active_tab_id();
                        // 相對路徑（..、子資料夾名稱、\Windows）以目前的資料夾為準
                        let here = self.active_tab().map(|t| t.path.clone()).unwrap_or_default();
                        // 輸入的是檔案路徑：開啟所在資料夾並選取該檔案（不直接執行）。
                        // 網路上的位置不在這裡檢查（離線時要等 SMB 逾時、視窗凍住），交給背景讀取時再判斷（見 navigate_or_reveal）
                        match full_path_in(&text, Some(&here)) {
                            Ok(full) if !fs::on_local_drive(&full) => self.navigate_or_reveal(id, &full, NavKind::Normal),
                            Ok(full) if fs::is_file(&full) => {
                                let parent = paths::parent(&full).unwrap_or_else(|| full.clone());
                                self.navigate(id, &parent, NavKind::Normal, vec![full]);
                            }
                            Ok(full) => self.navigate(id, &full, NavKind::Normal, vec![]),
                            _ => self.navigate(id, &text, NavKind::Normal, vec![]),
                        }
                        self.end_address_edit();
                        self.focus_list(self.active_side());
                    } else {
                        self.start_search();
                    }
                    return true;
                }
                VK_ESCAPE => {
                    if focus == self.address {
                        let path = self.active_tab().map(|t| t.path.clone()).unwrap_or_default();
                        set_window_text(self.address, &path);
                        self.end_address_edit();
                    } else if self.active_tab().is_some_and(|t| t.working()) {
                        self.clear_or_cancel();
                        return true;
                    }
                    self.focus_list(self.active_side());
                    return true;
                }
                _ => return false,
            }
        }
        if let Some(side) = self.side_of_list(focus) {
            // 空白鍵：按住預覽、放開關閉（正在打字找檔案時，空白是名稱的一部分）。
            // 按住時的自動重複一律略過，否則預覽會一直開關閃爍。
            if vk == VK_SPACE && !ctrl && !alt && !shift && !self.typeahead.active(anim::now()) {
                let repeat = msg.lParam & (1 << 30) != 0;
                if !repeat {
                    super::crashlog::crumb("按鍵：空白鍵按下");
                    match preview::mode(&self.preview) {
                        // 懸停中：放大懸停的項目，放開就關閉
                        Some(preview::Mode::Hover) => self.enlarge_hover_preview(),
                        // 從選單開啟、維持開著的預覽：空白鍵關閉
                        Some(_) => self.hide_preview(),
                        None => {
                            self.cancel_hover();
                            self.open_quicklook(side, true);
                        }
                    }
                }
                return true;
            }
            match vk {
                VK_RETURN => {
                    self.hide_preview();
                    self.open_selection(side);
                    return true;
                }
                VK_BACK => {
                    self.go_back();
                    return true;
                }
                VK_DELETE => {
                    self.delete_selection();
                    return true;
                }
                _ if ctrl && !shift && !alt && letter(b'D') => {
                    self.delete_selection();
                    return true;
                }
                VK_F2 => {
                    self.begin_rename();
                    return true;
                }
                _ => {}
            }
            if ctrl && letter(b'A') {
                self.select_all(side);
                return true;
            }
        }
        if focus == self.tree && vk == VK_RETURN {
            self.tree_navigate_selected();
            return true;
        }
        // 焦點在側欄：複製、剪下、複製路徑針對側欄選取的資料夾（與右鍵選單相同），
        // 而不是清單裡剛好選著的項目。
        if focus == self.tree && !in_edit {
            let copy = ctrl && !alt && letter(b'C');
            let cut = ctrl && !alt && letter(b'X');
            if copy || cut {
                if let Some(path) = self.tree_caret_path() {
                    if copy && shift {
                        if let Err(e) = super::clipboard::set_text(self.hwnd, &path) {
                            self.show_error(&e);
                        } else {
                            self.set_footer(tr!("已複製路徑", "Path copied").into());
                        }
                    } else if cut && paths::parent(&path).is_none() {
                        self.set_footer(tr!("磁碟根目錄不能剪下", "A drive's root folder can't be cut").into());
                    } else {
                        self.clipboard_paths(vec![path], cut);
                    }
                }
                return true;
            }
        }
        if ctrl && !alt && !in_edit {
            if letter(b'C') {
                if shift {
                    self.copy_path();
                } else {
                    self.clipboard_files(false);
                }
                return true;
            }
            if letter(b'X') {
                self.clipboard_files(true);
                return true;
            }
            if letter(b'V') {
                self.paste();
                return true;
            }
        }
        false
    }
}

fn pretranslate(msg: &MSG) -> bool {
    if msg.message == WM_MOUSEWHEEL || msg.message == WM_MOUSEHWHEEL {
        let Some(a) = app_opt() else { return false };
        let horizontal = msg.message == WM_MOUSEHWHEEL;
        // 懸停的預覽：清單照常捲動，預覽先關掉，滑鼠再移動時依新的項目重新計時
        if preview::mode(&a.preview) == Some(preview::Mode::Hover) {
            a.reset_hover();
            return false;
        }
        // 滑鼠在置中的快速預覽上：滾輪捲動預覽的內容（試算表也可左右捲動），而不是清單
        if preview::is_open(&a.preview) {
            let mut pt = POINT::default();
            let mut r = RECT::default();
            unsafe {
                GetCursorPos(&mut pt);
                GetWindowRect(a.preview.popup, &mut r);
            }
            if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
                let delta = ((msg.wParam >> 16) & 0xFFFF) as u16 as i16 as i32;
                if horizontal {
                    preview::hwheel(&mut a.preview, delta);
                } else {
                    preview::wheel(&mut a.preview, delta);
                }
                return true;
            }
        }
        if horizontal {
            a.reset_hover();
        }
        return false;
    }
    if msg.message == WM_XBUTTONUP {
        // 滑鼠側鍵：上一頁／下一頁（與檔案總管、瀏覽器相同）。滑鼠在 Stillfeather 任何地方都有效。
        let Some(a) = app_opt() else { return false };
        // 就地重新命名中：先讓使用者完成或取消編輯
        if (msg.hwnd != a.hwnd && unsafe { IsChild(a.hwnd, msg.hwnd) } == 0) || a.renaming.is_some() {
            return false;
        }
        match hiword(msg.wParam) {
            XBUTTON1 => a.go_back(),
            XBUTTON2 => a.go_forward(),
            _ => return false,
        }
        a.update_chrome();
        return true;
    }
    if matches!(msg.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        // 先處理焦點，點擊本身照常交給目標視窗
        if let Some(a) = app_opt() {
            a.blur_inputs_on_click(msg.hwnd);
        }
        return false;
    }
    if msg.message == WM_CHAR {
        // 清單與樹狀圖的打字搜尋由這裡處理；字元不交給控制項，因此找不到項目時不會有提示音。
        let Some(a) = app_opt() else { return false };
        if a.side_of_list(msg.hwnd).is_some() || msg.hwnd == a.tree {
            if let Some(c) = char::from_u32(msg.wParam as u32).filter(|c| !c.is_control()) {
                a.type_to_find(msg.hwnd, c);
            }
            return true;
        }
        return false;
    }
    if msg.message == WM_KEYUP {
        // 放開空白鍵：關閉按住開啟的快速預覽（輸入法開著時收到的是 VK_PROCESSKEY）
        let Some(a) = app_opt() else { return false };
        if a.space_gate_open() {
            let mut vk = msg.wParam as u32;
            if vk == VK_PROCESSKEY {
                vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
            }
            if vk == VK_SPACE {
                a.close_space_gate();
                return true;
            }
        }
        if preview::is_holding(&a.preview) {
            let mut vk = msg.wParam as u32;
            if vk == VK_PROCESSKEY {
                vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
            }
            if vk == VK_SPACE {
                super::crashlog::crumb("按鍵：空白鍵放開");
                a.hide_preview();
                return true;
            }
        }
        return false;
    }
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    let Some(a) = app_opt() else { return false };
    if msg.hwnd != a.hwnd && unsafe { IsChild(a.hwnd, msg.hwnd) } == 0 {
        return false;
    }
    let handled = a.handle_key(msg);
    if handled {
        a.update_chrome();
    }
    handled
}

/// 輸入法是否正在組字（注音輸入到一半）。
fn ime_composing(hwnd: HWND) -> bool {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc == 0 {
            return false;
        }
        let len = ImmGetCompositionStringW(himc, GCS_COMPSTR, std::ptr::null_mut(), 0);
        ImmReleaseContext(hwnd, himc);
        len > 0
    }
}

fn post_select_all(edit: HWND) {
    unsafe {
        PostMessageW(edit, EM_SETSEL, 0, -1);
    }
}

pub(super) fn create_edit(parent: HWND, cid: u16, name: &str) -> HWND {
    let class = wide("Edit");
    let n = wide(name);
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            0,
            0,
            10,
            10,
            parent,
            cid as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        // 可及性名稱：Edit 沒有標籤時，以視窗文字以外的方式不易提供；改用提示文字。
        let _ = n;
        SendMessageW(h, EM_LIMITTEXT, 32_767, 0);
        h
    }
}

fn create_static(parent: HWND) -> HWND {
    let class = wide("Static");
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX | SS_ENDELLIPSIS | 0x0200, /* SS_CENTERIMAGE */
            0,
            0,
            10,
            10,
            parent,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    }
}

/// 顯示右鍵選單，回傳選取的命令。id 為 0 的項目是分隔線。
pub(super) fn popup_menu(owner: HWND, items: &[(u16, &str, bool)], pt: POINT) -> Option<u16> {
    unsafe {
        let menu = CreatePopupMenu();
        for (cid, text, enabled) in items {
            if *cid == 0 {
                AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            } else {
                let w = wide(text);
                AppendMenuW(menu, MF_STRING | if *enabled { 0 } else { MF_GRAYED }, *cid as usize, w.as_ptr());
            }
        }
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, owner, std::ptr::null());
        DestroyMenu(menu);
        if cmd > 0 {
            Some(cmd as u16)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- 視窗程序

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(a) = app_opt() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    if a.hwnd != hwnd {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    // 右鍵選單中 Windows 項目的子選單（新增 ▸、傳送到 ▸）與自繪圖示
    if let Some(r) = super::shellmenu::handle_menu_msg(msg, wparam, lparam) {
        return r;
    }
    // 檔案總管重新啟動了：重新登記到殼層視窗清單（見 shellwin.rs）
    if msg >= 0xC000 && msg == super::shellwin::taskbar_created_msg() {
        super::shellwin::on_taskbar_created();
    }
    match msg {
        WM_SIZE => {
            a.relayout();
            a.reposition_quicklook();
            return 0;
        }
        WM_MOVE => {
            a.reposition_quicklook();
            return 0;
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize = POINT { x: a.s(960), y: a.s(600) };
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            a.paint();
            return 0;
        }
        WM_SETCURSOR => {
            if swallow_modal_click(hwnd, lparam) {
                return 1;
            }
            if wparam as HWND == hwnd {
                let mut p = cursor_pos();
                ScreenToClient(hwnd, &mut p);
                if a.drag.is_some() || a.splitter_at(p.x, p.y).is_some() {
                    SetCursor(load_cursor(IDC_SIZEWE));
                    return 1;
                }
                if a.layout.promo_toggle.is_some_and(|r| r.contains(p.x, p.y)) {
                    SetCursor(load_cursor(IDC_HAND));
                    return 1;
                }
            }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (get_x_lparam(lparam), get_y_lparam(lparam));
            if let Some(d) = a.splitter_at(x, y) {
                a.drag = Some(d);
                SetCapture(hwnd);
                return 0;
            }
            if a.layout.promo_toggle.is_some_and(|r| r.contains(x, y)) {
                a.toggle_personalize();
                return 0;
            }
            if a.picker_click(x, y) {
                return 0;
            }
            for i in 0..2 {
                if a.layout.panes[i].contains(x, y) {
                    a.activate(Side::from_index(i));
                }
            }
            return 0;
        }
        WM_MOUSEMOVE => {
            if a.drag.is_some() {
                a.drag_to(get_x_lparam(lparam));
            } else {
                a.promo_mouse(get_x_lparam(lparam), get_y_lparam(lparam));
            }
            return 0;
        }
        WM_MOUSELEAVE => {
            a.mouse_tracked = false;
            a.promo_mouse(-1, -1);
            return 0;
        }
        WM_LBUTTONUP => {
            if a.drag.is_some() {
                ReleaseCapture();
                a.end_drag();
            }
            return 0;
        }
        WM_CAPTURECHANGED => {
            a.end_drag();
            return 0;
        }
        WM_COMMAND => {
            a.command(loword(wparam), hiword(wparam), lparam as HWND);
            return 0;
        }
        WM_NOTIFY => {
            let hdr = &*(lparam as *const NMHDR);
            if let Some(side) = a.side_of_list(hdr.hwndFrom) {
                return a.list_notify(side, hdr.code, lparam);
            }
            if hdr.hwndFrom == a.tree {
                return a.tree_notify(hdr.code, lparam);
            }
        }
        WM_CONTEXTMENU => {
            let src = wparam as HWND;
            let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
            if let Some(side) = a.side_of_list(src) {
                a.list_context_menu(side, pt, lparam == -1);
                return 0;
            }
            if src == a.tree {
                a.tree_context_menu(pt, lparam == -1);
                return 0;
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let p = a.theme.pal;
            let is_status = lparam as HWND == a.pane_status[0] || lparam as HWND == a.pane_status[1];
            SetTextColor(hdc, if is_status { p.muted } else { p.ink });
            SetBkColor(hdc, p.surface);
            return a.theme.brushes.surface;
        }
        WM_TIMER => {
            if wparam == preview::TIMER_ID {
                preview::on_timer(&mut a.preview, hwnd);
            } else if wparam == TIMER_DRIVES {
                KillTimer(hwnd, TIMER_DRIVES);
                a.refresh_drives();
            } else if wparam == TIMER_PROMO {
                a.on_promo_timer();
            } else if wparam == preview::HOLD_TIMER {
                // 空白鍵已放開（放開的訊息可能被輸入法或其他視窗拿走）
                if preview::is_holding(&a.preview) && !preview::space_held() {
                    super::crashlog::crumb("計時器：空白鍵已放開");
                    a.hide_preview();
                }
            } else if wparam == super::pane::HOVER_TIMER {
                a.on_hover_timer();
            } else if wparam == super::pane::GATE_TIMER {
                a.on_gate_timer();
            } else if wparam == preview::SHOW_TIMER {
                preview::on_show_timer(&mut a.preview, hwnd);
            } else if wparam == TIMER_SEARCH {
                a.on_search_timer();
            } else if wparam == TIMER_SHELL_REFRESH {
                KillTimer(hwnd, TIMER_SHELL_REFRESH);
                a.refresh_after_shell();
            } else if wparam == super::update::TIMER_CHECK {
                a.on_update_timer();
            } else if wparam == super::update::TIMER_PROMPT {
                a.try_update_prompt();
            } else if wparam == super::watch::TIMER_WATCH {
                a.on_watch_timer();
            } else if wparam == super::picker::TIMER_HOST {
                a.picker_host_timer();
            } else if wparam == super::thumbs::TIMER_REDRAW {
                a.thumbs.redraw_due(hwnd);
                a.redraw_thumbnails();
            }
            return 0;
        }
        super::shellwin::WM_SHELL_SELECT => {
            let sel = Box::from_raw(lparam as *mut super::shellwin::Select);
            a.shell_select(sel.path, sel.replace);
            return 0;
        }
        super::shellwin::WM_SHELL_FRONT => {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) == 0 {
                super::shellwin::log("無法把視窗拉到前面（Windows 不允許）");
            }
            return 0;
        }
        super::thumbs::WM_APP_THUMB => {
            let done = Box::from_raw(lparam as *mut super::thumbs::Done);
            a.thumbs.on_result(hwnd, *done);
            return 0;
        }
        super::watch::WM_APP_WATCH => {
            a.on_watch_message(wparam, lparam);
            return 0;
        }
        WM_APP_WORKER => {
            let msg = Box::from_raw(lparam as *mut WorkerMsg);
            match *msg {
                WorkerMsg::Dir { tab, generation, path, kind, result } => a.on_dir(tab, generation, path, kind, result),
                WorkerMsg::NotFolder { tab, generation, path, kind } => a.on_not_folder(tab, generation, path, kind),
                WorkerMsg::AutoDir { tab, generation, path, result, gone_to } => a.on_auto_dir(tab, generation, path, result, gone_to),
                WorkerMsg::SearchBatch { tab, generation, entries, scanned, skipped } => a.on_search_batch(tab, generation, entries, scanned, skipped),
                WorkerMsg::SearchDone { tab, generation, result } => a.on_search_done(tab, generation, result),
                WorkerMsg::Tree { item, token, result } => a.on_tree_children(item, token, result),
                WorkerMsg::Preview { generation, path, result } => {
                    super::crashlog::crumb("主視窗：收到預覽結果");
                    preview::on_result(&mut a.preview, generation, &path, result)
                }
                WorkerMsg::FileOp(report) => a.on_file_op(report),
                WorkerMsg::Update(event) => a.on_update_event(event),
            }
            return 0;
        }
        WM_APP_OPEN => {
            let paths = Box::from_raw(lparam as *mut Vec<String>);
            a.open_paths(*paths, false, false);
            return 0;
        }
        WM_APP_TREE_RESET => {
            a.reset_tree_node(lparam as HTREEITEM);
            return 0;
        }
        WM_APP_SEARCH => {
            a.sync_search();
            show(a.clear_btn, a.clear_shown());
            return 0;
        }
        WM_APP_RENAME => {
            let data = Box::from_raw(lparam as *mut (TabId, String, String));
            a.finish_rename(data.0, &data.1, &data.2);
            return 0;
        }
        tabs::WM_APP_TAB => {
            a.tab_action(wparam, lparam);
            return 0;
        }
        chips::WM_APP_CHIP => {
            a.chip_action(wparam);
            return 0;
        }
        crumbs::WM_APP_CRUMB => {
            a.crumb_action(wparam);
            return 0;
        }
        WM_COPYDATA => {
            return match instance::decode_copydata(lparam) {
                Some(paths) => {
                    let p = Box::into_raw(Box::new(paths));
                    if PostMessageW(hwnd, WM_APP_OPEN, 0, p as LPARAM) == 0 {
                        drop(Box::from_raw(p));
                        0
                    } else {
                        1
                    }
                }
                None => 0,
            };
        }
        WM_DEVICECHANGE => {
            if matches!(wparam, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | 7) {
                SetTimer(hwnd, TIMER_DRIVES, 500, std::ptr::null());
            }
            return 1;
        }
        WM_SYSCOMMAND => {
            // Alt＋字母（或單按 Alt、F10）在沒有選單列的視窗會發出提示音；只保留 Alt+空白鍵的系統選單。
            if wparam & 0xFFF0 == SC_KEYMENU && lparam != b' ' as LPARAM {
                return 0;
            }
        }
        WM_SETTINGCHANGE => {
            anim::refresh_settings();
            if lparam != 0 && from_wide_ptr(lparam as *const u16) == "ImmersiveColorSet" {
                a.system_theme_changed();
            }
        }
        WM_THEMECHANGED => {
            a.system_theme_changed();
        }
        WM_DPICHANGED => {
            let r = *(lparam as *const RECT);
            a.on_dpi_changed(loword(wparam) as u32, &r);
            return 0;
        }
        WM_ACTIVATE => {
            if loword(wparam) == 0 {
                a.hide_preview();
                let f = GetFocus();
                if f != 0 && IsChild(hwnd, f) != 0 {
                    a.last_focus = f;
                }
            } else if hiword(wparam) == 0 {
                a.restore_focus();
                return 0;
            }
        }
        WM_SETFOCUS => {
            a.restore_focus();
            return 0;
        }
        WM_CLOSE => {
            super::crashlog::crumb("主視窗：WM_CLOSE");
            if a.op_busy {
                dialog::info(hwnd, tr!("檔案作業還在進行", "A file operation is still running"), tr!("請等它完成，或在 Windows 的進度視窗按取消後再關閉 Stillfeather。", "Wait for it to finish, or click Cancel in the Windows progress window, then close Stillfeather."));
                return 0;
            }
            if a.picker_mode() {
                a.picker_finish(feather_core::picker::EXIT_CANCEL);
                return 0;
            }
            a.hide_preview();
            // 「還原上次的分頁」：記下現在的分頁
            a.save_session();
            DestroyWindow(hwnd);
            return 0;
        }
        WM_ENDSESSION => {
            // Windows 登出、關機或重新啟動：不會收到 WM_CLOSE，在這裡記下分頁
            if wparam != 0 {
                a.save_session();
            }
            return 0;
        }
        WM_DESTROY => {
            super::crashlog::crumb("主視窗：WM_DESTROY");
            super::shellwin::stop();
            a.stop_watching();
            for h in [a.lists[0], a.lists[1], a.tree, a.crumbs, a.strips[0], a.strips[1]] {
                dragdrop::revoke(h);
            }
            for t in a.tabs.values() {
                t.cancel.store(true, Ordering::Relaxed);
            }
            if a.brand_icon != 0 {
                DestroyIcon(a.brand_icon);
                a.brand_icon = 0;
            }
            if a.wordmark.0 != 0 {
                DestroyIcon(a.wordmark.0);
                a.wordmark.0 = 0;
            }
            // 選擇器：結束代碼告訴取代元件是確定、取消還是改用 Windows 的視窗
            PostQuitMessage(a.picker.as_ref().map_or(0, |p| p.exit_code));
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

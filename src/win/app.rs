//! 主視窗：左側全高的側欄卡片（程式名稱、資料夾樹），
//! 右側依序是麵包屑與搜尋膠囊、底線分頁、篩選籤與動作按鈕、檔案清單卡片、底部狀態列。
//!
//! 這個檔案是主視窗本體：全域存取、`App` 的欄位、設定與工作階段檔、建立控制項、布局、繪製、狀態同步、
//! 命令（WM_COMMAND）、麵包屑選單與說明。其他部分依功能放在子模組（都是 `impl App`）：
//! `tab`（分頁的狀態）、`nav`（導覽）、`search`（搜尋與篩選條件）、`tabmgr`（分頁、雙欄與還原分頁）、
//! `fileactions`（檔案操作）、`config`（設定與系統變更）、`keys`（鍵盤）、`winproc`（視窗程序）。
//! 子模組看得到這裡的私有欄位與方法；子模組裡給這裡或其他子模組用的方法標 `pub(super)`，給 win 的其他模組用的標
//! `pub(in crate::win)`。

use super::anim;
use super::chips;
use super::controls::{self, Kind, Surface};
use super::crumbmenu;
use super::crumbs;
use feather_core::keymap;
use super::dialog::{self, Icon};
use super::dragdrop;
use super::fileops::{self, OpReport};
use super::fs::{self, FolderListing, FsError, Listing, MenuFolders, SearchSummary};
use super::icons::{self, Icons};
use super::instance;
use super::menu;
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
use feather_core::recent::{self, RecentSearch};
use feather_core::runline;
use feather_core::searchquery::{format_bytes, parse_size, DateSpec, Filter, Query, Scorer, SizeRange, TimeRange, DATE_PRESETS, SIZE_PRESETS};
use feather_core::sorting;
use feather_core::session::{Session, SideTabs};
use feather_core::typeahead::TypeAhead;
use feather_core::settings::{Settings, SIDEBAR_MAX, SIDEBAR_MIN, SPLIT_MAX, SPLIT_MIN};
use feather_core::view::ViewMode;
use feather_core::workspace::{CloseResult, DualChange, Side, TabId, Workspace};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, Ordering};
use std::sync::Arc;

// 依功能拆開的部分（都是 `impl App` 或分頁的狀態；這裡是主視窗本體：建立、布局、繪製、狀態同步與命令）
mod config;
mod fileactions;
mod keys;
mod nav;
mod search;
mod tab;
mod tabmgr;
mod detach;
pub(super) use detach::run_detached;
mod winproc;

pub(super) use tab::{Baked, Tab, ViewState};
use keys::pretranslate;
use winproc::main_proc;

pub const WM_APP_WORKER: u32 = WM_APP + 1;
pub const WM_APP_OPEN: u32 = WM_APP + 2;
pub const WM_APP_TREE_RESET: u32 = WM_APP + 3;
pub const WM_APP_RENAME: u32 = WM_APP + 4;
/// 搜尋框得到或失去焦點後，焦點轉移完成時再決定要不要收合。
pub const WM_APP_SEARCH: u32 = WM_APP + 5;
/// 選取剛建立的項目並進入改名（「新增資料夾」、Windows 的「新增 ▸」）。`lparam` 是 `Box<NewItem>`。
/// 用貼出去的訊息：在按鈕或選單的點擊處理裡直接開始編輯，滑鼠放開、焦點轉移時編輯框會馬上被收掉。
pub const WM_APP_BEGIN_RENAME: u32 = WM_APP + 18;
/// 清單的選取改了（`wparam` 是哪一欄）：同一輪訊息裡的多次通知合併成一次再更新狀態列等（見 `App::selection_changed`）
pub const WM_APP_SELECTION: u32 = WM_APP + 19;

/// [`WM_APP_BEGIN_RENAME`] 的內容
pub struct NewItem {
    /// 項目所在的分頁；None：畫面上正顯示它所在資料夾的分頁（作用中的那一欄優先）
    pub tab: Option<TabId>,
    pub path: String,
    /// 進入改名（否則只選取）
    pub edit: bool,
}
pub const TIMER_DRIVES: usize = 2;
/// 執行 Windows 右鍵選單的命令後，稍後再重新整理一次（有些命令在背景完成）。
pub const TIMER_SHELL_REFRESH: usize = 3;
/// 搜尋按鈕展開／收合動畫
pub const TIMER_SEARCH: usize = 5;
/// 狀態列的「Ctrl+Z 可復原」提示用重點色顯示一段時間，時間到換回一般顏色
pub const TIMER_FOOTER_HOT: usize = 18;
/// 按著滑鼠拖曳選取（框選）時，狀態列等隔一下才跟上選取
pub const TIMER_SELECTION: usize = 19;
/// 「Ctrl+Z 可復原」用重點色顯示多久（毫秒）：刪除、移動不再確認，提示要夠顯眼
const FOOTER_HOT_MS: u32 = 8000;
/// 最多記住幾個剛關掉的分頁
const CLOSED_TABS_MAX: usize = 20;

/// 剛關掉的分頁：Ctrl+Shift+T 重新開啟時放回原處
struct ClosedTab {
    side: Side,
    index: usize,
    path: String,
    back: Vec<String>,
    forward: Vec<String>,
    select: Vec<String>,
}

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
    pub const SORT_RELEVANCE: u16 = 258;
    /// 篩選籤「日期」選單：不限、DATE_PRESETS（281…）、自訂範圍
    pub const DATE_ANY: u16 = 280;
    pub const DATE_FIRST: u16 = 281;
    pub const DATE_CUSTOM: u16 = 287;
    /// 篩選籤「大小」選單：不限、SIZE_PRESETS（291…）、自訂
    pub const SIZE_ANY: u16 = 290;
    pub const SIZE_FIRST: u16 = 291;
    pub const SIZE_CUSTOM: u16 = 296;
    pub const SHOW_COMPACT: u16 = 260;
    pub const SHOW_EXTENSIONS: u16 = 261;
    /// 把目前的檢視設為所有資料夾的檢視（清掉個別資料夾的記錄）
    pub const VIEW_APPLY_ALL: u16 = 262;
    /// 「檢視 › 顯示 › 相片資料夾用大圖示」
    pub const SHOW_MEDIA_ICONS: u16 = 263;
    /// 「檢視 › 文字大小」：TEXT_SIZE_FIRST + 0～3（順序同 feather_core::settings::TEXT_SIZES）
    pub const TEXT_SIZE_FIRST: u16 = 264;
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

pub(super) fn shortcut_command(id: u16) -> Option<keymap::Cmd> {
    use keymap::Cmd::*;
    Some(match id {
        id::TAB_NEW => NewTab, id::TAB_CLOSE => CloseTab, id::TAB_MOVE_LEFT => MoveTabLeft, id::TAB_MOVE_RIGHT => MoveTabRight,
        id::BACK => Back, id::FORWARD => Forward, id::UP => Up, id::REFRESH => Refresh, id::SEARCH => Search, id::DUAL => ToggleDual,
        id::SETTINGS => Settings, id::UNDO => Undo, id::OPEN => Open, id::COPY => Copy, id::CUT => Cut, id::PASTE => Paste,
        id::DELETE => Delete, id::RENAME => Rename, id::NEW_FOLDER => NewFolder, id::COPY_PATH => CopyPath,
        id::PROPERTIES => Properties, id::SELECT_ALL => SelectAll, id::HELP => Help,
        _ => return None,
    })
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

/// Windows 的資源回收筒設定「顯示刪除確認對話方塊」有沒有勾（SHELLSTATE 的 fNoConfirmRecycle 是 0）。
/// Windows 10／11 預設不勾：丟資源回收筒不再先問。
fn recycle_needs_confirm() -> bool {
    // SHELLSTATE 前面是一串 BOOL 位元欄位：fShowAllObjects、fShowExtensions、fNoConfirmRecycle…
    // 結構實際大小約 36 位元組，給足夠大的緩衝區
    let mut state = [0u32; 16];
    unsafe {
        SHGetSetSettings(state.as_mut_ptr() as *mut std::ffi::c_void, SSF_NOCONFIRMRECYCLE, 0);
    }
    state[0] & (1 << 2) == 0
}

/// 資料夾清單的狀態文字（項目數、無法讀取的項目數）。
pub(super) fn folder_status(count: usize, skipped: usize) -> String {
    trn!(count; "{} 個項目", "{} item", "{} items", group_digits(count as u64)) + &if skipped > 0 { trf!("  ·  {} 個無法讀取", "  ·  {} unreadable", skipped) } else { String::new() }
}

/// 背景工作把結果交回 UI 執行緒。
pub enum WorkerMsg {
    Detached(TabId, u64, Result<(), String>),
    /// `gone_to`：讀不到而且資料夾已經不在了，最近一個還在的上層（見 on_dir）
    Dir { tab: TabId, generation: u64, path: String, kind: NavKind, result: Result<Listing, FsError>, gone_to: Option<String> },
    /// [`App::navigate_or_reveal`] 要開的位置其實是檔案：改開它所在的資料夾並選取它
    NotFolder { tab: TabId, generation: u64, path: String, kind: NavKind },
    /// 自動重新整理的重新讀取（見 watch.rs）；`gone_to`：資料夾已不在時，還在的上一層
    AutoDir { tab: TabId, generation: u64, path: String, result: Result<Listing, FsError>, gone_to: Option<String> },
    SearchBatch { tab: TabId, generation: u64, entries: Vec<Entry>, scanned: usize, skipped: usize },
    SearchDone { tab: TabId, generation: u64, result: Result<SearchSummary, FsError> },
    Tree { item: HTREEITEM, token: u64, result: Result<FolderListing, FsError> },
    /// 麵包屑的子資料夾選單讀好了（見 crumbmenu.rs）
    CrumbMenu { token: u64, result: Result<MenuFolders, FsError> },
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
        icons: g.tabs.iter().map(|id| if a.tabs.get(id).is_some_and(|t| t.search_mode) { glyph::SEARCH } else { glyph::FOLDER }).collect(),
        selected: g.selected,
        outline: a.card_border(side.index()),
    }
}

/// 分頁列（拖曳分頁到另一欄時找目標）。單欄時右欄的分頁列不算（回傳 0）。
pub fn tab_strip(side: Side) -> HWND {
    match app_opt() {
        Some(a) if side == Side::Left || a.ws.dual => a.strips[side.index()],
        _ => 0,
    }
}

pub fn chip_model() -> chips::Model {
    app().chips_model()
}

pub fn crumb_model() -> Vec<(String, String)> {
    app().active_tab().map(|t| paths::segments(&t.display_path())).unwrap_or_default()
}

/// 麵包屑哪一段的子資料夾選單開著（那一段的箭頭朝下）
pub fn crumb_menu_segment() -> Option<usize> {
    app_opt().and_then(|a| crumbmenu::segment(&a.crumb_menu))
}

// ---------------------------------------------------------------- 分頁狀態

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavKind {
    Normal,
    NoHistory,
    Back,
    Forward,
}

// ---------------------------------------------------------------- 樹狀節點

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
    pub sidebar_splitter: RECT,
    pub content_left: i32,
    pub content_right: i32,
    /// 第一列中段：路徑列與搜尋共用的範圍
    pub top_row: RECT,
    pub crumbs: RECT,
    /// 搜尋膠囊目前的外框（收合時是圓形按鈕的位置）
    pub search: RECT,
    pub strips: [RECT; 2],
    pub chips: RECT,
    /// 動作列哪些按鈕顯示文字；篩選籤放不下時依序拿掉
    pub action_labels: ActionLabels,
    /// 檔案清單卡片
    pub panes: [RECT; 2],
    pub pane_splitter: Option<RECT>,
    /// 底部的單行狀態列（選擇器是檔名欄與按鈕）
    pub footer: RECT,
}

/// 動作列的按鈕要不要帶文字。空間不夠時先拿掉「排序」「檢視」的文字（圖示和箭頭就認得出來），
/// 再拿掉「新增資料夾」的；文字仍是提示與螢幕閱讀器名稱。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ActionLabels {
    #[default]
    All,
    NewFolderOnly,
    None,
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

/// 底部狀態列左側：主要文字（項目數或「已選取 N 個項目」）與細節。
#[derive(Default)]
struct FooterInfo {
    title: String,
    detail: String,
    /// 細節後面接一個「清除篩選」連結（有篩選條件時）
    link: bool,
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
    /// 搜尋框目前的淡色提示（跟著搜尋範圍變，見 `sync_search_cue`）
    search_cue: String,
    /// 收合時代表搜尋的圓形按鈕
    search_btn: HWND,
    /// 右上角的圓形按鈕（雙欄、隱藏項目、深淺色、設定）
    top_buttons: Vec<HWND>,
    /// 標題列右側的動作（新增資料夾、剪下、複製…）
    action_buttons: Vec<HWND>,
    dual_btn: HWND,
    /// 「排序」「檢視」下拉選單按鈕
    pub(super) sort_btn: HWND,
    pub(super) view_btn: HWND,
    help_btn: HWND,
    /// 側欄標頭的羽毛圖示與其邊長（DPI 改變時重建）
    brand_icon: HICON,
    brand_icon_px: i32,
    /// 側欄標頭的草書字樣（圖示）、它的寬高，以及產生時的（可用寬, 可用高, 墨色）
    wordmark: (HICON, i32, i32),
    wordmark_key: (i32, i32, u32),
    tooltip: HWND,
    /// 所在位置被截斷時，在那一格下方顯示完整位置的提示（見 pane.rs 的 `update_location_tip`）
    pub(super) loc_tip: HWND,
    /// 提示目前顯示的是哪一欄的哪一列
    pub(super) loc_tip_at: Option<(Side, usize)>,
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
    /// 狀態列提示用重點色顯示到這個時候（anim::now()，毫秒）
    footer_hot_until: f64,
    footer_info: FooterInfo,
    /// 狀態列上「清除篩選」的位置（畫的時候記下，按下時判斷）
    footer_link: Option<RECT>,
    pub(super) layout: Layout,
    drag: Option<Drag>,
    pub(super) preview: preview::State,
    /// 麵包屑的子資料夾選單
    pub(super) crumb_menu: crumbmenu::State,
    /// 更新提醒（上次檢查、略過的版本、等待提醒的新版本）
    pub(super) updates: super::update::Ui,
    /// 滑鼠停在清單項目上的預覽
    pub(super) hover: super::pane::Hover,
    pub(super) default_path: String,
    suppress_query_change: bool,
    pub(super) updating_list: bool,
    /// 這一欄的選取改了，狀態列等還沒跟上（見 `selection_changed`）
    pub(super) selection_dirty: [bool; 2],
    /// 上次讓狀態列等跟上選取的時間（`anim::now`）
    pub(super) selection_flushed: f64,
    /// 清單內建的框選開始了（圖示等檢視；見 `drag_selecting`）
    pub(super) builtin_marquee: [bool; 2],
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
    /// 拖放中標示的放置位置：清單的資料夾列、整欄、側欄「常用位置」
    pub(super) drop_row: Option<(Side, usize)>,
    /// 詳細資料檢視的框選（按下後到放開；見 marquee.rs）
    pub(super) marquee: Option<super::marquee::Marquee>,
    pub(super) drop_pane: Option<Side>,
    pub(super) drop_pin: bool,
    pub(super) pending_pins: Vec<(Vec<String>, Option<String>)>,
    /// 從側欄刪除的資料夾節點：刪除完成後若資料夾已不在就移除節點
    pub(super) pending_tree_delete: Option<(HTREEITEM, String)>,
    /// 側欄大標籤的拖曳排序（見 tree.rs）
    pub(super) section_drag: Option<super::tree::SectionDrag>,
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
    /// 進行中的移動的目的資料夾（完成後讓顯示被搬走資料夾的分頁跟過去）
    op_dest: Option<String>,
    /// 剛關掉的分頁（最多 CLOSED_TABS_MAX 個，Ctrl+Shift+T 由後往前重新開啟）
    closed_tabs: Vec<ClosedTab>,
    detached: bool,
    detaching: Vec<TabId>,
    incoming_tab: Option<(TabId, detach::Transfer)>,
    /// 搜尋框取得焦點時外框淡入重點色
    search_fx: Tween,
    /// 搜尋的展開程度：0 = 圓形按鈕、1 = 膠囊輸入框
    search_open: Tween,
    /// 自動重新整理：顯示中資料夾的變更通知與重新讀取排程
    pub(super) watch: super::watch::Watches,
    /// 清單縮圖（見 thumbs.rs）
    pub(super) thumbs: super::thumbs::Thumbs,
    /// 捷徑、程式等每個檔案各自的圖示（見 fileicons.rs）
    pub(super) file_icons: super::fileicons::FileIcons,
    /// 選擇器模式（其他程式要開啟檔案時）；一般視窗是 None
    pub(super) picker: Option<super::picker::Picker>,
    /// 快速鍵表（預設按法加上設定檔的自訂綁定）
    pub(super) keymap: keymap::Keymap,
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

// Compare the snapshot originally read by this process while holding a cross-process lock.
static LAST_SETTINGS: std::sync::Mutex<Option<Settings>> = std::sync::Mutex::new(None);
static LAST_VIEWS: std::sync::Mutex<Option<FolderViews>> = std::sync::Mutex::new(None);

fn save_shared<T: Clone + Default + PartialEq>(path: &str, last: &std::sync::Mutex<Option<T>>, next: &T,
    parse: fn(&str) -> Result<T, String>, encoded: &str) -> Result<(), String> {
    let dir = std::path::Path::new(path).parent().ok_or("Invalid settings path")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(format!("{path}.lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let current = match std::fs::read_to_string(path) {
        Ok(text) => parse(&text).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => return Err(e.to_string()),
    };
    let mut previous = last.lock().unwrap();
    if previous.as_ref().is_some_and(|old| old != &current) && current != *next {
        return Err(tr!("另一個視窗已更新設定。切回此視窗後請重試。", "Another window updated the settings. Switch back to this window and retry.").into());
    }
    write_atomic(path, encoded)?;
    *previous = Some(next.clone());
    Ok(())
}

pub fn write_settings(settings: &Settings) -> Result<(), String> {
    // 選擇器是另一個程序：改了檢視、隱藏項目等只在這次有效，不寫回設定檔（主程式可能同時開著）
    if super::picker::active() {
        return Ok(());
    }
    let mut copy = settings.clone();
    copy.validate()?;
    save_shared(&settings_path()?, &LAST_SETTINGS, &copy, Settings::from_json, &copy.to_json())
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
    save_shared(&folder_views_path()?, &LAST_VIEWS, views, FolderViews::from_json, &views.to_json())
}

/// 建立主視窗並執行訊息迴圈。`pick` 有值時是選擇器模式（見 picker.rs）：精簡的介面、蓋在要求的程式上，
/// 回傳值是交給取代元件的結束代碼。
pub(super) fn run(paths_in: Vec<String>, pick: Option<super::picker::Picker>) -> i32 {
    run_window(paths_in, pick, None)
}

fn run_window(paths_in: Vec<String>, pick: Option<super::picker::Picker>, incoming: Option<detach::Transfer>) -> i32 {
    let hinst = unsafe { GetModuleHandleW(std::ptr::null()) };
    // 選擇器用不同的視窗類別：單一實例與安裝程式找主視窗時不會找到它
    let class_name = if pick.is_some() { format!("{}.Picker", instance::main_class()) } else if incoming.is_some() { format!("{}.Detached", instance::main_class()) } else { instance::main_class() };
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
    super::help::register(hinst);
    tabs::register(hinst);
    chips::register(hinst);
    crumbs::register(hinst);
    crumbmenu::register(hinst);
    preview::register(hinst);
    settings_dialog::register(hinst);

    let (settings, settings_error) = load_settings();
    let dpi = unsafe { GetDpiForSystem() };
    let keymap = keymap::Keymap::with_overrides(&settings.shortcuts).0;
    let theme = Theme::new(&settings, dpi);
    theme::apply_app_mode(theme.pal.dark);

    // 選擇器：蓋在要求的程式的視窗上（它的視窗是擁有者），置中在那個視窗上
    let owner = pick.as_ref().map(|p| p.req.owner).filter(|o| *o != 0 && unsafe { IsWindow(*o) } != 0).unwrap_or(0);
    // 視窗大小：預設 1280×800（選擇器 1100×720，依 DPI 縮放），不超過游標（或擁有者）所在螢幕的工作區。
    let (x, y, w, h) = unsafe {
        let mut or = RECT::default();
        let anchor = if let Some(t) = &incoming { POINT { x: t.x, y: t.y } } else if owner != 0 && GetWindowRect(owner, &mut or) != 0 { POINT { x: or.left + or.width() / 2, y: or.top + or.height() / 2 } } else { cursor_pos() };
        let mon = MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let work = mi.rcWork;
        let (base_w, base_h) = if pick.is_some() { (1100, 720) } else { (1280, 800) };
        let w = scale(base_w, dpi).min(work.width() - 40).max(400);
        let h = scale(base_h, dpi).min(work.height() - 40).max(300);
        let x = (if incoming.is_some() { anchor.x } else { anchor.x - w / 2 }).clamp(work.left, (work.right - w).max(work.left));
        let y = (if incoming.is_some() { anchor.y } else { anchor.y - h / 2 }).clamp(work.top, (work.bottom - h).max(work.top));
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
    let icons = Icons::new(theme.dpi, settings.compact_rows, theme.fonts.body_height);
    // 程式要求一律顯示隱藏的項目（FOS_FORCESHOWHIDDEN）
    let show_hidden = settings.show_hidden || pick.as_ref().is_some_and(|p| p.req.show_hidden);
    let default_view = settings.view_mode;
    let (views, views_error) = load_folder_views();
    *LAST_SETTINGS.lock().unwrap() = Some(settings.clone());
    *LAST_VIEWS.lock().unwrap() = Some(views.clone());
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
        search_cue: String::new(),
        search_btn: 0,
        top_buttons: Vec::new(),
        action_buttons: Vec::new(),
        dual_btn: 0,
        sort_btn: 0,
        view_btn: 0,
        help_btn: 0,
        brand_icon: 0,
        brand_icon_px: 0,
        wordmark: (0, 0, 0),
        wordmark_key: (0, 0, 0),
        tooltip: 0,
        loc_tip: 0,
        loc_tip_at: None,
        icons,
        nodes: HashMap::new(),
        node_token: 0,
        show_hidden,
        views,
        view: [default_view; 2],
        op_busy: false,
        footer: tr!("就緒", "Ready").into(),
        footer_sticky: false,
        footer_hot_until: 0.0,
        footer_info: FooterInfo::default(),
        footer_link: None,
        layout: Layout::default(),
        drag: None,
        preview: preview::State::new(),
        crumb_menu: crumbmenu::State::new(0),
        updates: super::update::Ui::load(),
        hover: Default::default(),
        default_path: fs::profile_dir(),
        suppress_query_change: false,
        updating_list: false,
        selection_dirty: [false, false],
        selection_flushed: 0.0,
        builtin_marquee: [false, false],
        last_focus: 0,
        address_editing: false,
        typeahead: TypeAhead::default(),
        list_fx: [RowFx::default(), RowFx::default()],
        list_top: [-1, -1],
        tree_fx: RowFx::default(),
        tree_top: 0,
        footer_fx: Tween::new(1.0),
        footer_changed: 0.0,
        drop_row: None,
        marquee: None,
        drop_pane: None,
        drop_pin: false,
        pending_pins: Vec::new(),
        pending_tree_delete: None,
        section_drag: None,
        history: feather_core::undo::History::default(),
        op_pending: None,
        undo_select: Vec::new(),
        renaming: None,
        paste_cut: None,
        after_delete: None,
        op_sources: Vec::new(),
        op_dest: None,
        closed_tabs: Vec::new(),
        detached: incoming.is_some(),
        detaching: Vec::new(),
        incoming_tab: None,
        search_fx: Tween::new(0.0),
        search_open: Tween::new(0.0),
        watch: Default::default(),
        thumbs: Default::default(),
        file_icons: Default::default(),
        picker: pick,
        keymap,
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
    let restored = !a.detached && !a.picker_mode() && a.restore_session();
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
    } else if let Some(transfer) = incoming {
        a.receive_tab(transfer);
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
        self.back_btn = icon(id::BACK, glyph::BACK, tr!("上一頁", "Back"), Surface::Page);
        self.forward_btn = icon(id::FORWARD, glyph::FORWARD, tr!("下一頁", "Forward"), Surface::Page);
        self.up_btn = icon(id::UP, glyph::UP, tr!("上一層", "Up"), Surface::Page);
        self.refresh_btn = icon(id::REFRESH, glyph::REFRESH, tr!("重新整理", "Refresh"), Surface::Page);
        self.address = create_edit(hwnd, id::ADDRESS, tr!("資料夾路徑", "Folder path"));
        self.query = create_edit(hwnd, id::QUERY, tr!("檔名搜尋", "File name search"));
        let cue = wide(tr!("搜尋檔名…", "Search file names…"));
        send_ptr(self.query, EM_SETCUEBANNER, 1, cue.as_ptr());
        self.clear_btn = icon(id::CLEAR, glyph::CLOSE, tr!("清除或取消搜尋", "Clear or cancel search"), Surface::Card);
        // 不搶焦點：勾選後游標仍在搜尋框裡
        self.global_btn = controls::create(hwnd, id::SEARCH_GLOBAL, Kind::Toggle, tr!("全域搜尋", "Whole PC"), Surface::Card, false);
        self.search_btn = round(id::SEARCH, glyph::SEARCH, tr!("搜尋檔名", "Search file names"));

        // 右上角的圓形按鈕：雙欄、設定（深淺色與主題在設定裡）
        self.dual_btn = round(id::DUAL, glyph::DUAL, tr!("切換雙欄", "Toggle dual pane"));
        let settings = round(id::SETTINGS, glyph::SETTINGS, tr!("設定", "Settings"));
        self.top_buttons = vec![self.dual_btn, settings];

        // 標題列右側：主要動作是膠囊按鈕，其餘是圓形圖示按鈕
        let new_folder = controls::create(hwnd, id::NEW_FOLDER, Kind::Pill(glyph::NEW_FOLDER), tr!("新增資料夾", "New folder"), Surface::Page, false);
        self.action_buttons = vec![new_folder];
        for (cid, g, text) in [
            (id::CUT, glyph::CUT, tr!("剪下", "Cut")),
            (id::COPY, glyph::COPY, tr!("複製", "Copy")),
            (id::PASTE, glyph::PASTE, tr!("貼上", "Paste")),
            (id::RENAME, glyph::RENAME, tr!("重新命名", "Rename")),
            (id::DELETE, glyph::DELETE, tr!("刪除到資源回收筒；按住 Shift 永久刪除", "Delete to Recycle Bin; hold Shift to delete permanently")),
            (id::COPY_ACROSS, glyph::COPY_ACROSS, tr!("複製到另一欄", "Copy to the other pane")),
            (id::PROPERTIES, glyph::PROPERTIES, tr!("內容", "Properties")),
        ] {
            self.action_buttons.push(round(cid, g, text));
        }
        // 最右邊：排序與檢視（顯示隱藏項目、副檔名在「檢視 › 顯示」裡）
        self.sort_btn = controls::create(hwnd, id::SORT_MENU, Kind::Menu(glyph::SORT), tr!("排序", "Sort"), Surface::Page, false);
        self.view_btn = controls::create(hwnd, id::VIEW_MENU, Kind::Menu(glyph::VIEW_DETAILS), tr!("檢視", "View"), Surface::Page, false);

        // 側欄
        self.help_btn = icon(id::HELP, glyph::HELP_CIRCLE, tr!("使用說明", "Help"), Surface::Sidebar);
        controls::set_subtle(self.help_btn);
        self.tree = self.create_tree();
        self.lists = [self.create_list(id::LIST_LEFT, Side::Left), self.create_list(id::LIST_RIGHT, Side::Right)];
        self.pane_status = [create_static(hwnd), create_static(hwnd)];
        self.preview.popup = preview::create_popup(hwnd);
        self.crumb_menu.popup = crumbmenu::create_popup(hwnd);

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
            // 所在位置的提示：位置由程式決定（貼在那一格下方），不是滑鼠停著就自己出現
            self.loc_tip = CreateWindowExW(
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
            let mut empty = [0u16; 1];
            let info = TOOLINFOW {
                cbSize: std::mem::size_of::<TOOLINFOW>() as u32,
                // 滑鼠移到提示上時穿過去：清單不會以為滑鼠離開了（懸停預覽照常換項目）
                uFlags: TTF_TRACK | TTF_ABSOLUTE | TTF_TRANSPARENT,
                hwnd,
                uId: 1,
                rect: RECT::default(),
                hinst: 0,
                lpszText: empty.as_mut_ptr(),
                lParam: 0,
                lpReserved: std::ptr::null_mut(),
            };
            SendMessageW(self.loc_tip, TTM_ADDTOOLW, 0, &info as *const TOOLINFOW as LPARAM);
            SendMessageW(self.loc_tip, TTM_SETMAXTIPWIDTH, 0, self.s(640) as LPARAM);
        }
        self.sync_shortcut_tips(TTM_ADDTOOLW);
        self.apply_fonts();
    }

    fn sync_shortcut_tips(&self, message: u32) {
        let mut tipped: Vec<HWND> = vec![self.back_btn, self.forward_btn, self.up_btn, self.refresh_btn, self.clear_btn, self.search_btn, self.help_btn];
        tipped.extend(self.top_buttons.iter().copied());
        tipped.extend(self.action_buttons.iter().skip(1).copied());
        tipped.extend([self.sort_btn, self.view_btn]);
        for b in tipped {
            self.set_tooltip(b, message);
        }
    }

    fn set_tooltip(&self, target: HWND, message: u32) {
        let mut name = window_text(target);
        let id = unsafe { GetWindowLongPtrW(target, -12 /* GWLP_ID */) } as u16;
        if let Some(cmd) = shortcut_command(id) {
            let title = name.split('\t').next().unwrap_or(&name);
            name = self.keymap.with_hint(title, cmd, "\t");
            set_window_text(target, &name);
        }
        let mut text = wide(&name.replace('\t', " · "));
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
        send_ptr(self.tooltip, message, 0, &info);
    }

    fn apply_fonts(&self) {
        let ui = self.theme.fonts.ui as WPARAM;
        if self.loc_tip != 0 {
            // 所在位置的提示是清單內容的一部分：用內容文字（跟著文字大小）
            send(self.loc_tip, WM_SETFONT, self.theme.fonts.body as WPARAM, 1);
        }
        if self.tooltip != 0 {
            send(self.tooltip, WM_SETFONT, ui, 1);
        }
        // 路徑與搜尋輸入框：同麵包屑、篩選籤，跟著文字大小放大
        for h in [self.address, self.query] {
            if h != 0 {
                send(h, WM_SETFONT, self.theme.fonts.body as WPARAM, 1);
            }
        }
        // 檔名、欄位與側欄用內容文字（跟著「檢視 › 文字大小」放大）；欄位標題的高度跟著清單的字型
        let body = self.theme.fonts.body as WPARAM;
        for h in [self.tree, self.lists[0], self.lists[1]] {
            if h != 0 {
                send(h, WM_SETFONT, body, 1);
            }
        }
        for h in self.pane_status {
            if h != 0 {
                send(h, WM_SETFONT, self.theme.fonts.body_small as WPARAM, 1);
            }
        }
        if let Some(p) = self.picker.as_ref() {
            send(p.name_edit, WM_SETFONT, ui, 1);
        }
    }

    pub(super) fn apply_theme_to_controls(&mut self) {
        let p = self.theme.pal;
        theme::style_caption(self.hwnd, &p, p.background);
        // 開著的對話方塊（例如跟著 Windows 換深淺色時開著的設定）標題列也跟著換
        theme::restyle_thread_captions(self.hwnd, &p);
        theme::apply_app_mode(p.dark);
        for side in [Side::Left, Side::Right] {
            self.style_list(side);
        }
        self.style_tree();
        unsafe {
            for tip in [self.tooltip, self.loc_tip] {
                if p.dark {
                    let name = wide("DarkMode_Explorer");
                    SetWindowTheme(tip, name.as_ptr(), std::ptr::null());
                } else {
                    SetWindowTheme(tip, std::ptr::null(), std::ptr::null());
                }
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

    /// 篩選籤列的內容（作用中分頁的各類型數量與篩選）。「搜尋含子資料夾」只在搜尋時顯示：
    /// 搜尋框展開，或這個分頁正在搜尋、顯示搜尋結果。
    pub(super) fn chips_model(&self) -> chips::Model {
        let searching = self.search_open.target() > 0.5 || self.active_tab().is_some_and(|t| t.search_mode || t.searching);
        match self.active_tab() {
            Some(t) => chips::Model {
                counts: t.counts,
                filter: t.filter,
                recursive: t.recursive,
                show_recursive: searching,
                conditions: [t.date.map(|d| d.0.label()), t.size.map(|s| s.label())],
            },
            None => chips::Model { counts: [0; category::COUNT], filter: Category::All, recursive: true, show_recursive: searching, conditions: [None, None] },
        }
    }

    /// 第一列（導覽、路徑列、搜尋）的高度：容得下路徑列最後一段的大字（跟著文字大小放大）
    fn top_row_height(&self) -> i32 {
        self.theme.s(40).max(self.theme.fonts.body_heading_height + self.theme.s(14))
    }

    /// 動作列（篩選籤與動作按鈕）的高度：容得下篩選籤的字
    fn action_row_height(&self) -> i32 {
        self.theme.s(34).max(self.theme.fonts.body_height + self.theme.s(12))
    }

    /// 動作列右邊（動作按鈕、排序、檢視）的寬度
    fn actions_width(&self, labels: ActionLabels) -> i32 {
        let s = |v: i32| self.theme.s(v);
        let act = self.action_row_height();
        let round_actions = if self.picker_mode() { 0 } else { self.action_buttons.len() as i32 - 1 };
        let pill_w = if labels == ActionLabels::None { act } else { s(126) };
        let menu_w = if labels == ActionLabels::All { s(92) } else { s(54) };
        pill_w + s(10) + round_actions * (act + s(6)) + s(8) + 2 * menu_w + s(6)
    }

    /// 動作列的按鈕能帶多少文字：篩選籤（搜尋時含「搜尋含子資料夾」）全部放得下為準，
    /// 篩選籤不被省略
    fn action_labels_fit(&self, cl: i32, cr: i32) -> ActionLabels {
        let need = chips::natural_width(self.chips, &self.chips_model());
        let room = cr - cl - self.s(16);
        [ActionLabels::All, ActionLabels::NewFolderOnly].into_iter().find(|l| room - self.actions_width(*l) >= need).unwrap_or(ActionLabels::None)
    }

    /// 篩選籤的數量、搜尋狀態變了：動作按鈕要不要帶文字可能跟著變，變了才重排
    fn sync_action_labels(&mut self) {
        if self.picker_mode() || self.layout.content_right <= self.layout.content_left {
            return;
        }
        if self.action_labels_fit(self.layout.content_left, self.layout.content_right) != self.layout.action_labels {
            self.relayout();
        }
    }

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
        // 展開後最寬 560（跟著文字大小放大，輸入的字也會放大），最多佔這一列的 55%；
        // 路徑列跟著讓出寬度，放不下時前段收成「…」
        let full = self.theme.st(560).min((area.width() - s(12)) * 11 / 20).max(round);
        let w = round + ((full - round) as f32 * v).round() as i32;
        let h = round + ((row_h - round) as f32 * v).round() as i32;
        let top = area.top + (row_h - h) / 2;
        let search = RECT { left: area.right - w, top, right: area.right, bottom: top + h };
        let crumbs = RECT { left: area.left, top: area.top, right: search.left - s(12), bottom: area.bottom };
        let edit_h = self.theme.fonts.body_height + s(2);
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

    pub(super) fn relayout(&mut self) {
        let rc = client_rect(self.hwnd);
        if rc.width() <= 0 || rc.height() <= 0 {
            return;
        }
        let s = |v: i32| self.theme.s(v);
        let w = rc.width();
        let h = rc.height();
        let m = s(10);
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
        pos.push((self.tree, self.tree_rect(&l, l.sidebar.bottom - s(12)), true));
        let gap = s(18);
        l.sidebar_splitter = RECT { left: l.sidebar.right, top: l.sidebar.top, right: l.sidebar.right + gap, bottom: l.sidebar.bottom };
        let cl = l.sidebar.right + gap;
        let cr = w - m;
        l.content_left = cl;
        l.content_right = cr;

        // ---- 第一列：導覽、路徑列、搜尋膠囊、圓形按鈕。路徑列最後一段（目前資料夾）用較大的字，取代原本的大標題
        let row_top = m;
        let row_h = self.top_row_height();
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

        // ---- 第二列（動作列）：左邊是類型篩選籤，右邊是動作按鈕、排序、檢視。
        // 篩選籤放不下時，「新增資料夾」「排序」「檢視」只顯示圖示（文字仍是提示）
        let bar_top = row_top + row_h + s(6);
        let act = self.action_row_height();
        let labels = if pick { ActionLabels::All } else { self.action_labels_fit(cl, cr) };
        l.action_labels = labels;
        let pill_w = if labels == ActionLabels::None { act } else { s(126) };
        let menu_w = if labels == ActionLabels::All { s(92) } else { s(54) };
        let actions_w = self.actions_width(labels);
        let mut ax = cr - actions_w;
        for (i, b) in self.action_buttons.iter().enumerate() {
            if pick && i > 0 {
                pos.push((*b, RECT::default(), false));
                continue;
            }
            let bw = if i == 0 { pill_w } else { act };
            pos.push((*b, RECT::new(ax, bar_top, bw, act), true));
            ax += bw + if i == 0 { s(10) } else { s(6) };
        }
        ax += s(8);
        for b in [self.sort_btn, self.view_btn] {
            pos.push((b, RECT::new(ax, bar_top, menu_w, act), true));
            ax += menu_w + s(6);
        }
        l.chips = RECT { left: cl, top: bar_top, right: (cr - actions_w - s(16)).max(cl), bottom: bar_top + act };
        pos.push((self.chips, l.chips, !pick));

        // ---- 分頁（選擇器沒有：清單卡片直接接在動作列下）
        let strip_top = bar_top + act + s(4);
        // 分頁列：標題跟著文字大小放大，列高也跟著加高
        let strip_h = s(32).max(self.theme.fonts.body_height + s(14));

        // ---- 底部的單行狀態列（選擇器是檔名欄、類型與確定／取消）與清單卡片
        // 狀態列的字跟著文字大小放大，高度至少容得下一行
        let status_h = if pick { s(62) } else { s(26).max(self.theme.fonts.body_height + s(8)) };
        l.footer = RECT { left: cl, top: h - m - status_h, right: cr, bottom: h - m };
        if pick {
            pos.extend(self.picker_bar_layout(l.footer));
        }
        // 卡片緊接在分頁列下面：選取的分頁（Chrome 樣式）和卡片連成一片
        let cards_top = if pick { bar_top + act + s(10) } else { strip_top + strip_h };
        let cards_bottom = l.footer.top - s(if pick { 14 } else { 6 });
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
        // 單欄時清單直接填滿卡片（項目數、選取資訊都在底部狀態列）；雙欄時每張卡片底部各有一行自己的狀態
        let pane_status = self.ws.dual && !pick;
        for i in 0..2 {
            let visible = i == 0 || self.ws.dual;
            let pane = l.panes[i];
            let status_h = if pane_status { s(22).max(self.theme.fonts.body_height + s(6)) } else { 0 };
            let list = RECT { left: pane.left + s(10), top: pane.top + s(6), right: pane.right - s(10), bottom: pane.bottom - s(6) - status_h };
            let status = RECT { left: pane.left + s(20), top: list.bottom + s(2), right: pane.right - s(20), bottom: pane.bottom - s(4) };
            pos.push((self.lists[i], list, visible));
            pos.push((self.pane_status[i], status, visible && pane_status));
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

    /// 狀態列提示，並用重點色顯示幾秒（刪除、移動完成後的「Ctrl+Z 可復原」）
    fn set_footer_hot(&mut self, text: String) {
        self.footer_hot_until = anim::now() + FOOTER_HOT_MS as f64;
        unsafe {
            SetTimer(self.hwnd, TIMER_FOOTER_HOT, FOOTER_HOT_MS, std::ptr::null());
        }
        self.set_footer(text);
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

    /// 沒有選取時狀態列的文字（主要文字、細節）：資料夾的項目總數與摘要（資料夾、檔案數與大小）；
    /// 搜尋時是搜尋的字與進度；讀取中、讀不到、取消時是那個狀態。原本放在大標題底下。
    /// 有篩選條件時第三個值是 true：細節後面接「清除篩選」。
    fn folder_status_line(&self) -> (String, String, bool) {
        let Some(t) = self.active_tab() else { return (String::new(), String::new(), false) };
        let filtered = t.has_chip_conditions();
        if t.search_mode {
            let q = t.query.trim();
            let title = match (t.search_global, q.is_empty()) {
                (true, false) => trf!("在整台電腦搜尋「{}」", "Searching the whole PC for “{}”", q),
                (false, false) => trf!("搜尋「{}」", "Search “{}”", q),
                (true, true) => tr!("在整台電腦依篩選條件搜尋", "Searching the whole PC by filters").into(),
                (false, true) => tr!("依篩選條件搜尋", "Search by filters").into(),
            };
            // 搜尋完成、有篩選條件：「找到 86 個 · 篩選：文件、本週、> 10 MB · 清除篩選」
            if filtered && !t.searching && !t.status.starts_with(tr!("無法", "Couldn't")) && !t.status.starts_with(tr!("已取消", "Cancelled")) {
                let n = t.row_count();
                let mut parts =
                    vec![if n == 0 { tr!("找不到符合的項目", "Nothing matches").to_string() } else { trn!(n; "找到 {} 個", "Found {}", "Found {}", group_digits(n as u64)) }];
                if t.truncated {
                    parts.push(tr!("結果已達上限（未完整搜尋）", "Result limit reached (search incomplete)").into());
                }
                parts.push(trf!("篩選：{}", "Filters: {}", t.condition_text()));
                return (title, parts.join(" · "), true);
            }
            return (title, t.status.clone(), false);
        }
        if t.loading || t.restored || t.path.is_empty() || t.status.starts_with(tr!("無法", "Couldn't")) || t.status.starts_with(tr!("已取消", "Cancelled")) {
            return (t.status.clone(), String::new(), false);
        }
        let s = &t.summary;
        let total = s.folders + s.files;
        let title = trn!(total; "{} 個項目", "{} item", "{} items", group_digits(total as u64));
        let mut parts = vec![trn!(s.folders; "{} 個資料夾", "{} folder", "{} folders", group_digits(s.folders as u64)), trn!(s.files; "{} 個檔案", "{} file", "{} files", group_digits(s.files as u64))];
        if s.files > 0 {
            parts.push(format_size(Some(s.bytes)));
        }
        if t.skipped > 0 {
            parts.push(trf!("{} 個無法讀取", "{} unreadable", group_digits(t.skipped as u64)));
        }
        if filtered {
            let n = t.row_count();
            parts.push(trf!("篩選：{}", "Filters: {}", t.condition_text()));
            parts.push(if n == 0 { tr!("沒有符合的項目", "Nothing matches").into() } else { trf!("顯示 {} 項", "Showing {}", group_digits(n as u64)) });
            return (title, parts.join(" · "), true);
        }
        if t.pick.as_ref().is_some_and(|p| !p.is_all()) {
            // 選擇器的檔案類型藏起了一些檔案
            parts.push(trf!("顯示 {} 項", "Showing {}", group_digits(t.row_count() as u64)));
        }
        (title, parts.join(" · "), false)
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
        let mut footer_link: Option<RECT> = None;
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            let rect = client_rect(self.hwnd);
            // 只重畫需要的範圍（例如只有狀態列換了內容、在淡入）：緩衝區只開那麼大
            let dirty = RECT { left: ps.rcPaint.left.max(rect.left), top: ps.rcPaint.top.max(rect.top), right: ps.rcPaint.right.min(rect.right), bottom: ps.rcPaint.bottom.min(rect.bottom) };
            let dirty = if dirty.width() > 0 && dirty.height() > 0 { dirty } else { rect };
            {
                let buf = theme::Buffered::part(hdc, dirty);
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

                // ---- 清單卡片（雙欄時作用中的一欄以重點色描邊）
                for i in 0..2 {
                    if i == 1 && !self.ws.dual {
                        continue;
                    }
                    theme::card(dc, &l.panes[i], s(20), p.surface, Some(self.card_border(i)), &p, dpi);
                    // 選取的分頁（Chrome 樣式，見 tabs.rs）和卡片連成一片：擦掉它底下那一段上框
                    if !self.picker_mode() {
                        if let Some((a, b)) = tabs::active_span(self.strips[i]) {
                            let sr = l.strips[i];
                            let edge = RECT { left: sr.left + a + 1, top: l.panes[i].top, right: (sr.left + b - 1).min(l.panes[i].right - s(10)), bottom: l.panes[i].top + 1 };
                            if edge.right > edge.left {
                                theme::fill(dc, &edge, p.surface);
                            }
                        }
                    }
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
                // ---- 底部狀態列：單行。左邊是作用中欄的項目數或選取資訊（粗體的主要文字＋淡色的細節），
                // 右邊是操作結果、分頁數。高度固定，選取改變時清單不會移動。
                let ft = l.footer;
                let info = &self.footer_info;
                let fv = anim::value(&self.footer_fx).clamp(0.0, 1.0);
                if anim::running(&self.footer_fx) {
                    anim::request_rects(self.hwnd, vec![ft]);
                }
                let right_w = (ft.width() / 3).min(s(360));
                let rr = RECT { left: ft.right - s(12) - right_w, right: ft.right - s(12), ..ft };
                let text_right = rr.left - s(16);
                let x = ft.left + s(12);
                let main_w = theme::text_width(dc, &info.title, f.body_bold).min((text_right - x).max(0));
                let t1 = RECT { left: x, right: x + main_w, ..ft };
                theme::draw_text(dc, &info.title, &t1, f.body_bold, anim::mix(p.background, p.ink, fv), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                // 有篩選條件時細節後面接「清除篩選」（重點色，可以按），細節太長時先省略細節
                let link_text = tr!("清除篩選", "Clear filters");
                let link_w = if info.link { theme::text_width(dc, link_text, f.body) } else { 0 };
                let link_room = if info.link { link_w + s(24) } else { 0 };
                let mut end = t1.right;
                if !info.detail.is_empty() && t1.right + s(24) < text_right - link_room {
                    let detail = format!("  ·  {}", info.detail);
                    let w = theme::text_width(dc, &detail, f.body).min(text_right - link_room - t1.right);
                    let t2 = RECT { left: t1.right, right: t1.right + w, ..ft };
                    theme::draw_text(dc, &detail, &t2, f.body, anim::mix(p.background, p.muted, fv), DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    end = t2.right;
                }
                if info.link && end + link_room <= text_right {
                    let sep = RECT { left: end, right: end + s(24), ..ft };
                    theme::draw_text(dc, "·", &sep, f.body, anim::mix(p.background, p.muted, fv), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    let lr = RECT { left: sep.right, right: sep.right + link_w, ..ft };
                    theme::draw_text(dc, link_text, &lr, f.body, anim::mix(p.background, p.accent, fv), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    footer_link = Some(lr.inflate(s(4), 0));
                }
                let hot = self.footer_sticky && anim::now() < self.footer_hot_until;
                let color = if self.op_busy || hot { p.accent } else { p.muted };
                theme::draw_text(dc, &self.footer, &rr, if hot { f.body_bold } else { f.body }, color, DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                }
            }
            EndPaint(self.hwnd, &ps);
        }
        self.footer_link = footer_link;
    }

    // ------------------------------------------------------------ 狀態同步

    /// 讓共用的導覽、搜尋與動作列反映作用中欄位的分頁。
    pub(super) fn update_chrome(&mut self) {
        let side = self.active_side();
        let tab_id = self.active_tab_id();
        let (path, can_back, can_fwd, can_up, query) = match self.tabs.get(&tab_id) {
            Some(t) => (t.path.clone(), !t.back.is_empty(), !t.forward.is_empty(), paths::up_target(&t.path).is_some(), t.query.clone()),
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
        // 共用資料夾清單（`\\nas`）不是真的資料夾：不能新增資料夾、貼上
        let share_list = paths::unc_server(&path).is_some();
        for i in [0usize, 3] {
            if let Some(&b) = self.action_buttons.get(i) {
                controls::set_enabled(b, !share_list);
            }
        }
        // 殼層視窗清單登記的位置：作用中分頁顯示的資料夾（讀取新資料夾時 path 還是舊的，讀好才換）；搜尋結果、首頁不登記
        if self.picker.is_none() {
            let location = self.tabs.get(&tab_id).filter(|t| !t.search_mode && !t.path.is_empty()).map(|t| t.path.as_str());
            super::shellwin::set_location(location);
        }
        self.sync_search();
        self.sync_search_cue();
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
        self.set_footer_info(info);
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
        // 篩選籤可能變多或變少（換了資料夾）：動作按鈕要不要帶文字跟著重新判斷
        self.sync_action_labels();
        // 顯示的資料夾換了（導覽、切換分頁、雙欄、搜尋）：自動重新整理改監看新的資料夾
        self.sync_watches();
        self.sync_crumb_menu();
    }

    /// 清單卡片的外框顏色（分頁列上選取的分頁也用同一個顏色描邊）：拖放到這一欄、或雙欄時作用中的一欄是重點色
    pub(super) fn card_border(&self, i: usize) -> COLORREF {
        let p = &self.theme.pal;
        let dropping = self.drop_pane.is_some_and(|d| d.index() == i);
        if dropping || (self.ws.dual && self.active_side().index() == i) {
            p.accent
        } else {
            theme::blend(p.ink, p.surface, 0.05)
        }
    }

    /// 浮動狀態列左側：選取一個項目時顯示其圖示、名稱與細節；多選時顯示數量與合計大小。
    /// 底部狀態列的文字。沒有選取時是資料夾的項目總數與摘要（見 folder_status_line）；
    /// 選取時是「已選取 N 個項目」與大小等資訊（一個項目時是它的名稱、類型、大小與修改日期）。
    /// 換上狀態列左邊的內容；內容換了時回傳 true（文字淡入並稍微上浮。連續快速變化，
    /// 例如按住方向鍵、框選時不重播，避免文字一直閃）
    fn set_footer_info(&mut self, info: FooterInfo) -> bool {
        let changed = info.title != self.footer_info.title || info.detail != self.footer_info.detail || info.link != self.footer_info.link;
        if changed {
            let now = anim::now();
            if now - self.footer_changed > 350.0 {
                self.footer_fx.restart(0.25, 1.0, now, anim::dur(anim::TOGGLE_MS));
            }
            self.footer_changed = now;
        }
        self.footer_info = info;
        changed
    }

    /// 選取改了：只重算狀態列左邊的選取資訊、只重畫狀態列。`update_chrome` 會重畫整個主視窗與分頁列等，
    /// 框選、按住方向鍵時每次都那樣做會卡。狀態列右邊原本顯示著操作結果時，換回分頁數要整個重新整理。
    pub(super) fn update_selection_footer(&mut self) {
        if self.footer_sticky {
            self.clear_footer_message();
            self.update_chrome();
            return;
        }
        let info = self.compute_footer_info(self.active_side());
        if self.set_footer_info(info) {
            self.invalidate_footer();
        }
    }

    fn compute_footer_info(&mut self, side: Side) -> FooterInfo {
        let count = self.selected_count(side);
        if count == 0 {
            let (title, detail, link) = self.folder_status_line();
            return FooterInfo { title, detail, link };
        }
        if count == 1 {
            if let Some(e) = self.selected_entries(side).into_iter().next() {
                let mut parts = vec![e.name.clone(), e.type_label()];
                if !e.is_dir {
                    parts.push(format_size(e.size()));
                }
                let date = format_filetime(e.modified);
                if !date.is_empty() {
                    parts.push(date);
                }
                return FooterInfo { title: tr!("已選取 1 個項目", "1 item selected").to_string(), detail: parts.join(" · "), link: false };
            }
        }
        let detail = if count <= 5_000 {
            // 直接看清單上的項目，不複製（框選時每 0.1 秒就算一次）
            let (mut bytes, mut folders, mut files) = (0u64, 0usize, 0usize);
            if let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) {
                for i in self.list_selected_indices(self.lists[side.index()]) {
                    match tab.row(i) {
                        Some(e) if e.is_dir => folders += 1,
                        Some(e) => {
                            files += 1;
                            bytes += e.size;
                        }
                        None => {}
                    }
                }
            }
            trf!("{} 個資料夾 · {} 個檔案 · {}", "{} folders · {} files · {}", folders, files, format_size(Some(bytes)))
        } else {
            tr!("大小未計算（選取項目過多）", "Size not calculated (too many items selected)").to_string()
        };
        FooterInfo { title: trn!(count; "已選取 {} 個項目", "{} item selected", "{} items selected", group_digits(count as u64)), detail, link: false }
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

    fn tree_rect(&self, l: &Layout, bottom: i32) -> RECT {
        RECT { left: l.sidebar.left + self.s(8), top: l.brand.bottom + self.s(16), right: l.sidebar.right - self.s(8), bottom }
    }

    // ------------------------------------------------------------ 交叉淡化

    /// 某一欄內容切換時要交叉淡化的範圍：清單卡片；作用中的欄另外加上大標題與底部狀態列。
    pub(super) fn content_fade_areas(&self, side: Side) -> Vec<RECT> {
        let l = &self.layout;
        let mut v = vec![l.panes[side.index()].inflate(self.s(4), self.s(6))];
        if side == self.active_side() {
            v.push(l.crumbs.inflate(self.s(4), self.s(4)));
            v.push(l.footer.inflate(self.s(4), self.s(4)));
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
            id::NEW_FOLDER => self.new_folder(),
            id::CUT => self.clipboard_files(true),
            id::COPY => self.clipboard_files(false),
            id::PASTE => self.paste(),
            id::RENAME => self.begin_rename(),
            // 按住 Shift 點「刪除」（按鈕或右鍵選單）是永久刪除，和檔案總管相同
            id::DELETE => self.delete_selection(key_down(VK_SHIFT)),
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
                for (dirs, group) in std::mem::take(&mut self.pending_pins) {
                    self.pin_to_group(dirs, group.as_deref());
                }
            }
            _ => {}
        }
        self.update_chrome();
    }

    /// 麵包屑箭頭：開啟或關閉那一段的子資料夾選單（見 crumbmenu.rs）。
    /// `hover`：只是滑鼠移到這個箭頭，另一段的選單開著時才換成這一段的。
    fn crumb_menu_action(&mut self, index: usize, hover: bool) {
        let open = crumbmenu::segment(&self.crumb_menu);
        if hover && (open.is_none() || open == Some(index)) {
            return;
        }
        if !hover && open == Some(index) {
            self.close_crumb_menu();
            return;
        }
        let segments = crumb_model();
        let Some((_, path)) = segments.get(index) else { return };
        let Some(anchor) = crumbs::arrow_rect(self.crumbs, index) else { return };
        let tab = self.active_tab_id();
        // 順序與清單相同：分頁目前的排序
        let (sort_col, desc) = self.tabs.get(&tab).map(|t| (t.sort_col, t.desc)).unwrap_or((0, false));
        self.cancel_hover();
        crumbmenu::open(
            &mut self.crumb_menu,
            crumbmenu::Request {
                segment: index,
                tab,
                path: path.clone(),
                current: segments.get(index + 1).map(|s| s.0.clone()),
                anchor,
                show_hidden: self.show_hidden,
                sort_col,
                desc,
            },
        );
        invalidate(self.crumbs);
    }

    /// 關閉麵包屑的子資料夾選單（沒開著時什麼都不做）。
    pub(super) fn close_crumb_menu(&mut self) {
        if crumbmenu::close(&mut self.crumb_menu) {
            invalidate(self.crumbs);
        }
    }

    /// 子資料夾選單選了一個資料夾：在選單所屬的分頁開啟（一般的導覽，可以按上一頁回來）。
    /// 選的是目前位置經過的資料夾時，到了那一層選取原本所在的子資料夾（同點麵包屑的名稱）。
    pub(super) fn pick_crumb_menu(&mut self, path: String) {
        let Some((tab, _)) = crumbmenu::target(&self.crumb_menu) else { return };
        self.close_crumb_menu();
        if tab != self.active_tab_id() {
            return;
        }
        let segments = crumb_model();
        let select = segments
            .iter()
            .position(|s| paths::same_path(&s.1, &path))
            .and_then(|k| segments.get(k + 1))
            .map(|s| vec![s.1.clone()])
            .unwrap_or_default();
        self.navigate(tab, &path, NavKind::Normal, select);
        self.update_chrome();
    }

    /// 換了分頁或資料夾：子資料夾選單列的已經不是麵包屑上的那一層，關掉。
    fn sync_crumb_menu(&mut self) {
        let Some((tab, path)) = crumbmenu::target(&self.crumb_menu) else { return };
        let still = tab == self.active_tab_id()
            && crumbmenu::segment(&self.crumb_menu).and_then(|i| crumb_model().get(i).map(|s| paths::same_path(&s.1, path))).unwrap_or(false);
        if !still {
            self.close_crumb_menu();
        }
    }

    /// 點了滑鼠（含標題列、邊框）：點在選單外面就關閉選單。點的是麵包屑的箭頭時交給麵包屑（換選單或關閉）。
    fn crumb_menu_click(&mut self, msg: &MSG) {
        if !crumbmenu::is_open(&self.crumb_menu) || msg.hwnd == self.crumb_menu.popup {
            return;
        }
        if msg.hwnd == self.crumbs && crumbs::arrow_at(self.crumbs, msg.pt).is_some() {
            return;
        }
        self.close_crumb_menu();
    }

    /// 麵包屑：前往上層資料夾（並選取原本所在的子資料夾），或切換成輸入模式。
    /// `new_tab`：中鍵點的，在新分頁開啟（見 `open_in_background_tab`）
    fn crumb_action(&mut self, index: usize, new_tab: bool) {
        if index == crumbs::EDIT {
            self.begin_address_edit();
            return;
        }
        let segments = crumb_model();
        let Some((_, target)) = segments.get(index) else { return };
        if new_tab {
            let target = target.clone();
            self.open_in_background_tab(self.active_side(), &target);
            return;
        }
        let select: Vec<String> = segments.get(index + 1).map(|s| vec![s.1.clone()]).unwrap_or_default();
        let id = self.active_tab_id();
        self.navigate(id, &target.clone(), NavKind::Normal, select);
    }

    /// 說明頁（? 按鈕、F1）：獨立的視窗，快速鍵一覽從快速鍵表產生（見 help.rs）。
    /// 正在搜尋（搜尋框開著或顯示搜尋結果）時直接開在「搜尋」那一頁。
    pub(super) fn show_help(&mut self) {
        self.hide_preview();
        let searching = self.search_open.target() > 0.5 || self.active_tab().is_some_and(|t| t.search_mode);
        let topic = if searching { super::help::Topic::Search } else { super::help::Topic::Shortcuts };
        super::help::open(self.hwnd, super::help::pages(&self.settings, &self.keymap), topic);
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
/// 簡單的右鍵選單（分頁的選單）：(命令, 文字, 可用)，命令 0 是分隔線。字型跟著文字大小（見 menu.rs）。
pub(super) fn popup_menu(owner: HWND, items: &[(u16, &str, bool)], pt: POINT) -> Option<u16> {
    let items: Vec<menu::Item> = items
        .iter()
        .map(|(cid, text, enabled)| {
            if *cid == 0 {
                menu::Item::Separator
            } else {
                menu::Item::Command { id: *cid, text, glyph: None, mark: menu::Mark::None, on: false, enabled: *enabled }
            }
        })
        .collect();
    menu::popup_at(owner, pt, &items)
}

// ---------------------------------------------------------------- 視窗程序

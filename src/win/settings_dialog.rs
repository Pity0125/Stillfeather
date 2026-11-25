//! 設定視窗：左邊是分類導覽，右邊是那一類的設定，開啟時顯示「外觀與文字」。
//!
//! - 外觀與文字：文字大小（附範例檔名即時預覽）、介面語言、主題、重點色、背景色
//! - 檔案瀏覽：緊湊清單、顯示副檔名、側欄寬度、雙欄模式的左欄比例、還原上次的分頁
//! - 預覽：圖片與影片、PDF 的快速預覽、按空白鍵預覽
//! - Windows 整合：設為預設檔案總管、攔截 Win+E（放在它下面，停用時就近說明原因）、
//!   其他程式開啟或儲存檔案時也用 Stillfeather。這三項寫在登錄，不在設定檔（見 [`Integration`]）。
//! - 關於與更新：目前版本、檢查更新、啟動時提醒更新
//!
//! 所有分類的控制項一次建立，切換分類只是顯示或隱藏，還沒儲存的修改都留著；底部固定「儲存」「取消」，
//! 一次儲存全部分類。「還原『某分類』的預設值」只填入目前分類的預設值，按「儲存」才套用（常用位置不受影響）。
//! 文字大小和「檢視 › 文字大小」是同一個設定（`Settings::text_size`）；這個視窗的文字也跟著它放大，
//! 儲存後清單、側欄、分頁立即換成新的大小，下次開啟設定也是。

use super::app;
use super::controls::{self, Kind, Surface};
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::settings::{color_to_hex, parse_color, Language, Settings, ThemeMode, SIDEBAR_MAX, SIDEBAR_MIN, SPLIT_MAX, SPLIT_MIN, TEXT_SIZES};

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

const CLASS: &str = "FeatherFiles.Settings";

const SEG_SYSTEM: u16 = 10;
const SEG_LIGHT: u16 = 11;
const SEG_DARK: u16 = 12;
const LANG_SYSTEM: u16 = 13;
const LANG_CHINESE: u16 = 14;
const LANG_ENGLISH: u16 = 15;
const ACCENT_EDIT: u16 = 20;
const ACCENT_PICK: u16 = 21;
const BG_EDIT: u16 = 22;
const BG_PICK: u16 = 23;
const SIDEBAR_EDIT: u16 = 24;
const SPLIT_EDIT: u16 = 25;
const T_COMPACT: u16 = 30;
const T_EXT: u16 = 31;
const T_IMAGE: u16 = 32;
const T_PDF: u16 = 33;
const T_SPACE: u16 = 34;
const T_DEFAULT_FM: u16 = 35;
const T_REMINDERS: u16 = 36;
const T_PICKER: u16 = 37;
const T_WIN_E: u16 = 38;
const T_RESTORE: u16 = 39;
const RESET: u16 = 40;
const CHECK_UPDATES: u16 = 41;
/// 文字大小的四個級距（依 TEXT_SIZES 的順序）
const SIZE_FIRST: u16 = 50;
/// 分類導覽（依 PAGES 的順序）
const NAV_FIRST: u16 = 60;
const NOTE_ID: u16 = 90;

/// 分類
const P_LOOK: usize = 0;
const P_BROWSE: usize = 1;
const P_PREVIEW: usize = 2;
const P_WINDOWS: usize = 3;
const P_ABOUT: usize = 4;
const PAGES: usize = 5;

fn page_title(page: usize) -> &'static str {
    match page {
        P_LOOK => tr!("外觀與文字", "Appearance & text"),
        P_BROWSE => tr!("檔案瀏覽", "Browsing"),
        P_PREVIEW => tr!("預覽", "Preview"),
        P_WINDOWS => tr!("Windows 整合", "Windows integration"),
        _ => tr!("關於與更新", "About & updates"),
    }
}

fn page_glyph(page: usize) -> char {
    match page {
        P_LOOK => glyph::FONT_SIZE,
        P_BROWSE => glyph::FOLDER,
        P_PREVIEW => glyph::PREVIEW,
        P_WINDOWS => glyph::LINK,
        _ => glyph::PROPERTIES,
    }
}

fn size_label(i: usize) -> &'static str {
    [tr!("標準", "Standard"), tr!("大", "Large"), tr!("更大", "Larger"), tr!("最大", "Largest")][i.min(3)]
}

/// 版面的基準大小（邏輯像素，文字大小 100% 時）
const BASE_W: i32 = 780;
const BASE_H: i32 = 560;
const NAV_W: i32 = 200;
/// 底部「還原預設值、儲存、取消」那一列的高度
const BAR_H: i32 = 68;
/// 右側內容的左右留白、標籤欄寬
const MARGIN: i32 = 32;
const LABEL_W: i32 = 150;

/// 開著的設定視窗與它的更新狀態文字（背景檢查完成時由主視窗更新）
static DIALOG: AtomicIsize = AtomicIsize::new(0);
static UPDATE_NOTE: AtomicIsize = AtomicIsize::new(0);
/// 「檢查更新」按鈕；找到新版本後改成「立即更新」
static UPDATE_BUTTON: AtomicIsize = AtomicIsize::new(0);
/// 這次開著的設定已經找到新版本（按鈕是「立即更新」）
static UPDATE_READY: AtomicBool = AtomicBool::new(false);
/// 按了「立即更新」：設定關閉後直接下載更新，不再詢問
static UPDATE_NOW: AtomicBool = AtomicBool::new(false);

/// 和 Windows 整合的選項（不存在 settings.json，由主視窗套用到登錄）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integration {
    /// 設為預設檔案總管（Win+E 與開啟資料夾）
    pub default_fm: bool,
    /// 攔截 Win+E（新版 Windows 需要；只在設為預設檔案總管時有作用）
    pub win_e: bool,
    /// 其他程式開啟或儲存檔案時也用 Stillfeather
    pub picker: bool,
}

impl Integration {
    /// 「還原預設值」填入的值：和第一次執行時相同（設為預設檔案總管、攔截 Win+E 開啟，開檔視窗關閉）
    const DEFAULT: Integration = Integration { default_fm: true, win_e: true, picker: false };
}

/// 這個視窗用的字型：跟著文字大小放大（視窗太大放不進螢幕時縮小一點），關閉時刪除
struct Fonts {
    normal: HFONT,
    bold: HFONT,
    small: HFONT,
    heading: HFONT,
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.normal, self.bold, self.small, self.heading] {
                if f != 0 {
                    DeleteObject(f);
                }
            }
        }
    }
}

struct State {
    base: Settings,
    result: Option<Settings>,
    /// 按下儲存時的狀態
    integration: Integration,
    done: bool,
    page: usize,
    nav: [HWND; PAGES],
    /// 各分類的控制項（切換分類時顯示或隱藏）
    pages: [Vec<HWND>; PAGES],
    sizes: [HWND; 4],
    size_note: HWND,
    /// 範例檔名的預覽（只在「外觀與文字」顯示）
    preview: RECT,
    segments: [HWND; 3],
    languages: [HWND; 3],
    accent: HWND,
    background: HWND,
    sidebar: HWND,
    split: HWND,
    compact: HWND,
    extensions: HWND,
    restore_tabs: HWND,
    image: HWND,
    pdf: HWND,
    space: HWND,
    default_fm: HWND,
    win_e: HWND,
    win_e_note: HWND,
    picker: HWND,
    reminders: HWND,
    reset: HWND,
    /// 文字框的外框（畫在對話方塊上）與所在的分類
    frames: Vec<(RECT, HWND, usize)>,
    /// 文字大小 100% 時的 `k(v)`：版面的縮放比例（百分比）
    percent: u32,
    fonts: Fonts,
    custom_colors: [COLORREF; 16],
}

impl State {
    /// 依這個視窗的縮放比例換算（邏輯像素 → 實際像素）
    fn k(&self, v: i32) -> i32 {
        scale(v * self.percent as i32 / 100, app::theme().dpi)
    }

    /// 選了哪個文字大小（都沒選時用原本的設定值，例如設定檔裡寫了 120）
    fn text_size(&self) -> u32 {
        self.sizes.iter().position(|h| controls::is_checked(*h)).map_or(self.base.text_size, |i| TEXT_SIZES[i])
    }
}

/// 設定開著時，更新狀態文字（例如「已是最新版本」）。設定沒有開著時回傳 false。
pub fn update_status(text: &str) -> bool {
    let h = UPDATE_NOTE.load(Ordering::Relaxed);
    if h == 0 {
        return false;
    }
    set_window_text(h, text);
    true
}

/// 設定開著時，告訴使用者有新版本：「檢查更新」改成「立即更新」，按下就儲存設定、關閉並開始更新。
/// 設定沒有開著時回傳 false。
pub fn update_found(version: &str) -> bool {
    if !update_status(&trf!("有新版本 {version}", "{version} available")) {
        return false;
    }
    let button = UPDATE_BUTTON.load(Ordering::Relaxed);
    if button != 0 {
        set_window_text(button, tr!("立即更新", "Update now"));
        controls::set_primary(button);
        unsafe {
            InvalidateRect(button, std::ptr::null(), 0);
        }
    }
    UPDATE_READY.store(true, Ordering::Relaxed);
    true
}

/// 設定關閉後：使用者是不是按了「立即更新」（只回答一次）
pub fn take_update_now() -> bool {
    UPDATE_NOW.swap(false, Ordering::Relaxed)
}

/// 設定開著時，顯示檢查失敗的原因（對話方塊開在設定上方）。設定沒有開著時回傳 false。
pub fn update_failed(error: &str) -> bool {
    let dlg = DIALOG.load(Ordering::Relaxed);
    if dlg == 0 || !update_status(tr!("檢查失敗", "Check failed")) {
        return false;
    }
    super::dialog::Spec::new(super::dialog::Icon::Warning, tr!("無法檢查更新", "Couldn't check for updates")).body(error).show(dlg);
    true
}

/// 「儲存」：檢查輸入，沒問題就記下結果並關閉設定；有誤時切到那個分類、說明原因，留在設定裡
fn save(hwnd: HWND, st: &mut State) -> bool {
    match collect(st) {
        Ok(s) => {
            st.result = Some(s);
            st.integration = Integration {
                default_fm: controls::is_checked(st.default_fm),
                win_e: controls::is_checked(st.win_e),
                picker: controls::is_checked(st.picker),
            };
            st.done = true;
            true
        }
        Err((page, e)) => {
            show_page(hwnd, st, page);
            super::dialog::Spec::new(super::dialog::Icon::Warning, tr!("設定有誤", "Invalid setting")).body(&e).show(hwnd);
            false
        }
    }
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: dialog_proc,
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

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut()
}

fn static_text(parent: HWND, text: &str, r: RECT, font: HFONT, id: u16, center: bool) -> HWND {
    let class = wide("Static");
    let t = wide(text);
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX | if center { 0x0200 /* SS_CENTERIMAGE */ } else { 0 },
            r.left,
            r.top,
            r.width(),
            r.height(),
            parent,
            id as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SendMessageW(h, WM_SETFONT, font as WPARAM, 1);
        h
    }
}

/// 設定項目的名稱（垂直置中在一列裡）
fn label(parent: HWND, text: &str, r: RECT, font: HFONT) -> HWND {
    static_text(parent, text, r, font, 0, true)
}

/// 說明文字（次要文字色，可以換行）
fn note(parent: HWND, text: &str, r: RECT, font: HFONT) -> HWND {
    static_text(parent, text, r, font, NOTE_ID, false)
}

fn edit(parent: HWND, cid: u16, frame: RECT, st: &mut State, page: usize) -> HWND {
    let t = app::theme();
    let h_edit = theme::line_height(st.fonts.normal) + t.s(2);
    let r = RECT {
        left: frame.left + st.k(8),
        top: frame.top + (frame.height() - h_edit) / 2,
        right: frame.right - st.k(8),
        bottom: frame.top + (frame.height() - h_edit) / 2 + h_edit,
    };
    let class = wide("Edit");
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
            r.left,
            r.top,
            r.width(),
            r.height(),
            parent,
            cid as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SendMessageW(h, WM_SETFONT, st.fonts.normal as WPARAM, 1);
        st.frames.push((frame, h, page));
        h
    }
}

fn button(parent: HWND, cid: u16, kind: Kind, text: &str, r: RECT, fonts: &Fonts) -> HWND {
    let surface = if matches!(kind, Kind::Nav(_)) { Surface::Sidebar } else { Surface::Card };
    let h = controls::create(parent, cid, kind, text, surface, true);
    controls::set_fonts(h, fonts.normal, fonts.bold);
    move_window(h, r);
    h
}

const LANGUAGES: [Language; 3] = [Language::System, Language::Chinese, Language::English];
const THEMES: [ThemeMode; 3] = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark];

fn check_one(group: &[HWND], index: Option<usize>) {
    for (i, h) in group.iter().enumerate() {
        controls::set_checked(*h, Some(i) == index);
    }
}

/// 把設定值填進某個分類的控制項。
fn fill_page(st: &State, page: usize, s: &Settings, integration: Integration) {
    match page {
        P_LOOK => {
            check_one(&st.sizes, TEXT_SIZES.iter().position(|p| *p == s.text_size));
            check_one(&st.languages, LANGUAGES.iter().position(|l| *l == s.language));
            check_one(&st.segments, THEMES.iter().position(|m| *m == s.theme_mode));
            set_window_text(st.accent, s.accent_color.as_deref().unwrap_or(""));
            set_window_text(st.background, s.background_color.as_deref().unwrap_or(""));
            size_changed(st);
        }
        P_BROWSE => {
            controls::set_checked(st.compact, s.compact_rows);
            controls::set_checked(st.extensions, s.show_extensions);
            set_window_text(st.sidebar, &s.sidebar_width.to_string());
            set_window_text(st.split, &s.split_percent.to_string());
            controls::set_checked(st.restore_tabs, s.restore_tabs);
        }
        P_PREVIEW => {
            controls::set_checked(st.image, s.image_preview);
            controls::set_checked(st.pdf, s.pdf_preview);
            controls::set_checked(st.space, s.space_preview);
        }
        P_WINDOWS => {
            controls::set_checked(st.default_fm, integration.default_fm);
            controls::set_checked(st.win_e, integration.win_e);
            win_e_changed(st);
            controls::set_checked(st.picker, integration.picker);
        }
        _ => controls::set_checked(st.reminders, s.update_reminders),
    }
}

/// 攔截 Win+E 只在設為預設檔案總管時有作用：停用時在它下面說明原因
fn win_e_changed(st: &State) {
    let on = controls::is_checked(st.default_fm);
    controls::set_enabled(st.win_e, on);
    set_window_text(
        st.win_e_note,
        if on {
            tr!("新版 Windows 11 需要；開著時會在背景執行一個小程式。", "Needed on newer Windows 11; runs a small helper in the background.")
        } else {
            tr!("要先勾選上面的「設為預設檔案總管」才能使用：Win+E 只會開預設的檔案總管。", "Turn on “Set as default file manager” above first: Win+E only opens the default file manager.")
        },
    );
}

/// 選了別的文字大小：說明文字與範例檔名跟著換
fn size_changed(st: &State) {
    let pct = st.text_size();
    set_window_text(
        st.size_note,
        &trf!(
            "{pct}%：檔名與欄位、側欄、分頁、狀態列和這個視窗的文字都會跟著調整（也可以從「檢視 › 文字大小」切換）。",
            "{pct}%: file names, columns, the sidebar, tabs, the status bar and this window (also View › Text size)."
        ),
    );
    let dlg = DIALOG.load(Ordering::Relaxed);
    if dlg != 0 {
        unsafe {
            InvalidateRect(dlg, &st.preview, 0);
        }
    }
}

/// 各分類的「還原預設值」按鈕文字：寫明只還原這個分類
fn reset_text(page: usize) -> String {
    trf!("還原「{}」的預設值", "Restore “{}” defaults", page_title(page))
}

/// 切換到某個分類：只顯示它的控制項，左邊的導覽跟著標示，「還原預設值」換成這個分類的
fn show_page(dlg: HWND, st: &mut State, page: usize) {
    st.page = page.min(PAGES - 1);
    for (i, hs) in st.pages.iter().enumerate() {
        for h in hs {
            unsafe {
                ShowWindow(*h, if i == st.page { SW_SHOW } else { SW_HIDE });
            }
        }
    }
    check_one(&st.nav, Some(st.page));
    let text = reset_text(st.page);
    set_window_text(st.reset, &text);
    let width = unsafe {
        let hdc = GetDC(st.reset);
        let w = theme::text_width(hdc, &text, st.fonts.normal);
        ReleaseDC(st.reset, hdc);
        w
    } + st.k(36);
    let mut r = RECT::default();
    unsafe {
        GetWindowRect(st.reset, &mut r);
        let mut tl = POINT { x: r.left, y: r.top };
        ScreenToClient(dlg, &mut tl);
        SetWindowPos(st.reset, 0, tl.x, tl.y, width, r.height(), SWP_NOZORDER | SWP_NOACTIVATE);
        InvalidateRect(dlg, std::ptr::null(), 0);
    }
}

/// 讀出所有分類的值。錯誤時回傳是哪個分類與原因。
fn collect(st: &State) -> Result<Settings, (usize, String)> {
    let mut s = st.base.clone();
    s.text_size = st.text_size();
    s.theme_mode = st.segments.iter().position(|h| controls::is_checked(*h)).map_or(ThemeMode::System, |i| THEMES[i]);
    s.language = st.languages.iter().position(|h| controls::is_checked(*h)).map_or(Language::System, |i| LANGUAGES[i]);
    let color = |h: HWND| -> Result<Option<String>, (usize, String)> {
        let t = window_text(h).trim().to_string();
        if t.is_empty() {
            return Ok(None);
        }
        let t = if t.starts_with('#') { t } else { format!("#{t}") };
        match parse_color(&t) {
            Some((r, g, b)) => Ok(Some(color_to_hex(r, g, b))),
            None => Err((P_LOOK, tr!("自訂顏色必須是 #RRGGBB，或留白使用預設。", "Custom colors must be #RRGGBB, or leave them blank to use the default.").into())),
        }
    };
    s.accent_color = color(st.accent)?;
    s.background_color = color(st.background)?;
    s.sidebar_width = window_text(st.sidebar)
        .trim()
        .parse()
        .map_err(|_| (P_BROWSE, trf!("側欄寬度需介於 {SIDEBAR_MIN} 與 {SIDEBAR_MAX}。", "The sidebar width must be between {SIDEBAR_MIN} and {SIDEBAR_MAX}.")))?;
    s.split_percent = window_text(st.split)
        .trim()
        .trim_end_matches('%')
        .parse()
        .map_err(|_| (P_BROWSE, trf!("左欄比例需介於 {SPLIT_MIN}% 與 {SPLIT_MAX}%。", "The left pane ratio must be between {SPLIT_MIN}% and {SPLIT_MAX}%.")))?;
    s.compact_rows = controls::is_checked(st.compact);
    s.show_extensions = controls::is_checked(st.extensions);
    s.restore_tabs = controls::is_checked(st.restore_tabs);
    s.image_preview = controls::is_checked(st.image);
    s.pdf_preview = controls::is_checked(st.pdf);
    s.space_preview = controls::is_checked(st.space);
    s.update_reminders = controls::is_checked(st.reminders);
    // 超出範圍的是側欄寬度或左欄比例（在「檔案瀏覽」），其他（文字大小、顏色）在「外觀與文字」
    let page = if !(SIDEBAR_MIN..=SIDEBAR_MAX).contains(&s.sidebar_width) || !(SPLIT_MIN..=SPLIT_MAX).contains(&s.split_percent) { P_BROWSE } else { P_LOOK };
    s.validate().map_err(|e| (page, e))?;
    Ok(s)
}

fn pick_color(dlg: HWND, st: &mut State, target: HWND, fallback: COLORREF) {
    let current = parse_color(window_text(target).trim()).map(|(r, g, b)| rgb(r, g, b)).unwrap_or(fallback);
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: dlg,
        hInstance: 0,
        rgbResult: current,
        lpCustColors: st.custom_colors.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        lCustData: 0,
        lpfnHook: std::ptr::null(),
        lpTemplateName: std::ptr::null(),
    };
    if unsafe { ChooseColorW(&mut cc) } != 0 {
        let c = cc.rgbResult;
        set_window_text(target, &color_to_hex((c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8));
    }
}

/// 版面縮放比例：跟著文字大小，但整個視窗要放得進螢幕的工作區（放不下就縮小，最小 100%）
fn layout_percent(text_size: u32, work: &RECT, dpi: u32) -> u32 {
    // 不呼叫 Win32（單元測試在其他平台也能跑）：邏輯像素 × 百分比 × DPI ÷ 96
    let px = |v: i32, pct: u32| (v as i64 * pct as i64 * dpi as i64 / (100 * 96)) as i32;
    let mut pct = text_size.max(100);
    while pct > 100 && (px(BASE_W, pct) > work.width() * 96 / 100 || px(BASE_H, pct) > work.height() * 92 / 100) {
        pct -= 5;
    }
    pct.max(100)
}

/// 顯示對話方塊（強制回應）；按「儲存」且驗證通過時回傳新設定，以及和 Windows 整合的選項要不要開著。
/// `integration` 是目前的狀態。
pub fn show(owner: HWND, current: &Settings, integration: Integration) -> Option<(Settings, Integration)> {
    let t = app::theme();
    let work = unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, rcMonitor: RECT::default(), rcWork: RECT::default(), dwFlags: 0 };
        GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut mi);
        mi.rcWork
    };
    let percent = layout_percent(t.text_size, &work, t.dpi);
    let fonts = Fonts {
        normal: theme::ui_font(9, percent, false, t.dpi),
        bold: theme::ui_font(9, percent, true, t.dpi),
        small: theme::ui_font(8, percent, false, t.dpi),
        heading: theme::ui_font(13, percent, true, t.dpi),
    };
    let k = |v: i32| scale(v * percent as i32 / 100, t.dpi);
    let (cw, ch) = (k(BASE_W), k(BASE_H));
    let class = wide(CLASS);
    let title = wide(tr!("設定", "Settings"));
    let dlg = unsafe {
        let mut or = RECT::default();
        GetWindowRect(owner, &mut or);
        // 置中在主視窗上，但不超出螢幕的工作區
        let x = (or.left + (or.width() - cw) / 2).clamp(work.left, (work.right - cw).max(work.left));
        let y = (or.top + (or.height() - ch) / 2).clamp(work.top, (work.bottom - ch).max(work.top));
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            x,
            y,
            cw,
            ch,
            owner,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if dlg == 0 {
        return None;
    }
    // 依框線補足視窗大小，使內容區正好是 cw×ch。
    unsafe {
        let mut wr = RECT::default();
        GetWindowRect(dlg, &mut wr);
        let cr = client_rect(dlg);
        let (fw, fh) = (wr.width() - cr.width(), wr.height() - cr.height());
        let top = wr.top.min((work.bottom - ch - fh).max(work.top));
        SetWindowPos(dlg, 0, wr.left, top, cw + fw, ch + fh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    theme::style_caption(dlg, &t.pal, t.pal.surface);

    let mut st = Box::new(State {
        base: current.clone(),
        result: None,
        integration,
        done: false,
        page: P_LOOK,
        nav: [0; PAGES],
        pages: Default::default(),
        sizes: [0; 4],
        size_note: 0,
        preview: RECT::default(),
        segments: [0; 3],
        languages: [0; 3],
        accent: 0,
        background: 0,
        sidebar: 0,
        split: 0,
        compact: 0,
        extensions: 0,
        restore_tabs: 0,
        image: 0,
        pdf: 0,
        space: 0,
        default_fm: 0,
        win_e: 0,
        win_e_note: 0,
        picker: 0,
        reminders: 0,
        reset: 0,
        frames: Vec::new(),
        percent,
        fonts,
        custom_colors: [rgb(255, 255, 255); 16],
    });
    let (f_normal, f_small, f_heading) = (st.fonts.normal, st.fonts.small, st.fonts.heading);
    let bar_top = ch - k(BAR_H);

    // 左邊：分類導覽
    let nav_h = k(40);
    for page in 0..PAGES {
        let r = RECT::new(k(12), k(16) + page as i32 * (nav_h + k(4)), k(NAV_W) - k(24), nav_h);
        st.nav[page] = button(dlg, NAV_FIRST + page as u16, Kind::Nav(page_glyph(page)), page_title(page), r, &st.fonts);
    }

    // 右邊：各分類的內容
    let x0 = k(NAV_W + MARGIN);
    let right = cw - k(MARGIN);
    let full_w = right - x0;
    let cx = x0 + k(LABEL_W);
    let ctrl_w = right - cx;
    let row_h = k(32);
    let note_h = theme::line_height(f_small) + t.s(2);
    // 勾選方塊的文字從這裡開始（說明文字對齊它）
    let toggle_text = x0 + t.s(34);
    let heading_h = theme::line_height(f_heading) + t.s(4);
    for page in 0..PAGES {
        let h = label(dlg, page_title(page), RECT::new(x0, k(22), full_w, heading_h), f_heading);
        st.pages[page].push(h);
    }
    let top = k(22) + heading_h + k(18);

    // 外觀與文字
    {
        let p = P_LOOK;
        let mut y = top;
        let h = label(dlg, tr!("文字大小", "Text size"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        let seg_w = (ctrl_w - k(6) * 3) / 4;
        for i in 0..TEXT_SIZES.len() {
            let h = button(dlg, SIZE_FIRST + i as u16, Kind::Segment, size_label(i), RECT::new(cx + i as i32 * (seg_w + k(6)), y, seg_w, row_h), &st.fonts);
            st.sizes[i] = h;
            st.pages[p].push(h);
        }
        y += row_h + k(8);
        // 範例檔名：放得下最大的文字（150%），切換級距時框的大小不變
        let biggest = theme::ui_font(9, TEXT_SIZES[TEXT_SIZES.len() - 1], false, t.dpi);
        let preview_h = (theme::line_height(biggest) + t.s(18)).max(k(44));
        unsafe {
            DeleteObject(biggest);
        }
        st.preview = RECT::new(cx, y, ctrl_w, preview_h);
        y += preview_h + k(6);
        st.size_note = note(dlg, "", RECT::new(cx, y, ctrl_w, note_h * 2), f_small);
        st.pages[p].push(st.size_note);
        y += note_h * 2 + k(14);
        let h = label(dlg, tr!("介面語言", "Language"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        // 語言名稱用它自己的文字寫（看不懂目前的語言也找得到）
        let seg3 = (ctrl_w - k(6) * 2) / 3;
        for (i, (cid, text)) in [(LANG_SYSTEM, tr!("跟隨系統", "System")), (LANG_CHINESE, "繁體中文"), (LANG_ENGLISH, "English")].into_iter().enumerate() {
            let h = button(dlg, cid, Kind::Segment, text, RECT::new(cx + i as i32 * (seg3 + k(6)), y, seg3, row_h), &st.fonts);
            st.languages[i] = h;
            st.pages[p].push(h);
        }
        y += row_h + k(4);
        let h = note(dlg, tr!("換了語言要重新開啟 Stillfeather 才會套用。", "A new language applies after Stillfeather restarts."), RECT::new(cx, y, ctrl_w, note_h), f_small);
        st.pages[p].push(h);
        y += note_h + k(14);
        let h = label(dlg, tr!("主題", "Theme"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        for (i, (cid, text)) in [(SEG_SYSTEM, tr!("跟隨系統", "System")), (SEG_LIGHT, tr!("淺色", "Light")), (SEG_DARK, tr!("深色", "Dark"))].into_iter().enumerate() {
            let h = button(dlg, cid, Kind::Segment, text, RECT::new(cx + i as i32 * (seg3 + k(6)), y, seg3, row_h), &st.fonts);
            st.segments[i] = h;
            st.pages[p].push(h);
        }
        y += row_h + k(14);
        let pick_w = k(96);
        let edit_w = (ctrl_w - pick_w - k(10)).min(k(180));
        let h = label(dlg, tr!("重點色", "Accent color"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        st.accent = edit(dlg, ACCENT_EDIT, RECT::new(cx, y, edit_w, row_h), &mut st, p);
        st.pages[p].push(st.accent);
        let h = button(dlg, ACCENT_PICK, Kind::Text, tr!("選色…", "Pick…"), RECT::new(cx + edit_w + k(10), y, pick_w, row_h), &st.fonts);
        st.pages[p].push(h);
        y += row_h + k(14);
        let h = label(dlg, tr!("背景色", "Background"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        st.background = edit(dlg, BG_EDIT, RECT::new(cx, y, edit_w, row_h), &mut st, p);
        st.pages[p].push(st.background);
        let h = button(dlg, BG_PICK, Kind::Text, tr!("選色…", "Pick…"), RECT::new(cx + edit_w + k(10), y, pick_w, row_h), &st.fonts);
        st.pages[p].push(h);
        y += row_h + k(4);
        let h = note(dlg, tr!("顏色留白使用主題預設；格式為 #RRGGBB。", "Blank uses the theme default. Format: #RRGGBB"), RECT::new(cx, y, ctrl_w, note_h), f_small);
        st.pages[p].push(h);
    }

    // 勾選方塊與它下面的說明（回傳下一列的 y）
    let toggle = |st: &mut State, page: usize, cid: u16, text: &str, explain: Option<&str>, x: i32, y: i32| -> (HWND, i32) {
        let h = button(dlg, cid, Kind::Toggle, text, RECT::new(x, y, right - x, row_h), &st.fonts);
        st.pages[page].push(h);
        let mut y = y + row_h;
        if let Some(e) = explain {
            // 說明可能要兩行（英文比較長）：依實際寬度量高度
            let nx = x - x0 + toggle_text;
            let lines_h = unsafe {
                let hdc = GetDC(dlg);
                let h = theme::text_height(hdc, e, f_small, right - nx, DT_WORDBREAK);
                ReleaseDC(dlg, hdc);
                h
            }
            .max(note_h);
            let n = note(dlg, e, RECT::new(nx, y + t.s(2), right - nx, lines_h + t.s(2)), f_small);
            st.pages[page].push(n);
            y += lines_h + t.s(4);
        }
        (h, y + k(14))
    };
    // 名稱、文字框與範圍說明（回傳文字框與下一列的 y）
    let number = |st: &mut State, page: usize, cid: u16, name: &str, range: &str, y: i32| -> (HWND, i32) {
        let h = label(dlg, name, RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[page].push(h);
        let e = edit(dlg, cid, RECT::new(cx, y, k(96), row_h), st, page);
        st.pages[page].push(e);
        let n = note(dlg, range, RECT::new(cx + k(108), y + (row_h - note_h) / 2, right - cx - k(108), note_h), f_small);
        st.pages[page].push(n);
        (e, y + row_h + k(14))
    };

    // 檔案瀏覽
    {
        let p = P_BROWSE;
        let y = top;
        let (h, y) = toggle(&mut st, p, T_COMPACT, tr!("緊湊清單", "Compact list"), Some(tr!("列高較矮，一次看得到更多項目。", "Shorter rows, so more items fit on screen.")), x0, y);
        st.compact = h;
        let (h, y) = toggle(&mut st, p, T_EXT, tr!("顯示副檔名", "Show file extensions"), None, x0, y);
        st.extensions = h;
        let (h, y) = number(&mut st, p, SIDEBAR_EDIT, tr!("側欄寬度", "Sidebar width"), &trf!("{SIDEBAR_MIN}–{SIDEBAR_MAX} 像素", "{SIDEBAR_MIN}–{SIDEBAR_MAX} pixels"), y);
        st.sidebar = h;
        let (h, y) = number(&mut st, p, SPLIT_EDIT, tr!("雙欄模式的左欄比例", "Left pane in dual view"), &format!("{SPLIT_MIN}–{SPLIT_MAX} %"), y);
        st.split = h;
        let (h, _) = toggle(&mut st, p, T_RESTORE, tr!("還原上次的分頁", "Restore last tabs"), Some(tr!("開啟 Stillfeather 時，把上次開著的分頁開回來。", "Reopen the tabs from last time when Stillfeather starts.")), x0, y);
        st.restore_tabs = h;
    }

    // 預覽
    {
        let p = P_PREVIEW;
        let y = top;
        let (h, y) = toggle(&mut st, p, T_IMAGE, tr!("快速預覽圖片與影片", "Preview images & videos"), None, x0, y);
        st.image = h;
        let (h, y) = toggle(&mut st, p, T_PDF, tr!("快速預覽 PDF", "Preview PDFs"), None, x0, y);
        st.pdf = h;
        let (h, _) = toggle(
            &mut st,
            p,
            T_SPACE,
            tr!("按空白鍵預覽", "Preview with Space"),
            Some(tr!("不自動懸停預覽，按住空白鍵時才預覽滑鼠下的項目。", "Hold Space to preview the item under the mouse instead of previewing on hover.")),
            x0,
            y,
        );
        st.space = h;
    }

    // Windows 整合
    {
        let p = P_WINDOWS;
        let y = top;
        let (h, y) = toggle(&mut st, p, T_DEFAULT_FM, tr!("設為預設檔案總管（Win+E 與開啟資料夾）", "Set as default file manager (Win+E and opening folders)"), None, x0, y);
        st.default_fm = h;
        // 攔截 Win+E：屬於「設為預設檔案總管」，放在它下面、縮排；沒有勾選時停用並說明原因
        let ix = x0 + k(28);
        let h = button(dlg, T_WIN_E, Kind::Toggle, tr!("攔截 Win+E 按鍵，改開 Stillfeather", "Intercept Win+E and open Stillfeather"), RECT::new(ix, y - k(8), right - ix, row_h), &st.fonts);
        st.win_e = h;
        st.pages[p].push(h);
        let nx = ix + t.s(34);
        st.win_e_note = note(dlg, "", RECT::new(nx, y - k(8) + row_h + t.s(2), right - nx, note_h * 2), f_small);
        st.pages[p].push(st.win_e_note);
        let y = y - k(8) + row_h + note_h * 2 + k(18);
        let (h, _) = toggle(
            &mut st,
            p,
            T_PICKER,
            tr!("其他程式開啟或儲存檔案時也用 Stillfeather", "Use Stillfeather for other programs' Open and Save dialogs"),
            Some(tr!("例如網頁上傳檔案、另存新檔。", "For example uploading files in a browser, or Save As.")),
            x0,
            y,
        );
        st.picker = h;
    }

    // 關於與更新
    let check;
    let update_note;
    {
        let p = P_ABOUT;
        let mut y = top;
        let h = label(dlg, tr!("目前版本", "Version"), RECT::new(x0, y, k(LABEL_W) - k(8), row_h), f_normal);
        st.pages[p].push(h);
        let h = label(dlg, super::update::VERSION, RECT::new(cx, y, ctrl_w, row_h), f_normal);
        st.pages[p].push(h);
        y += row_h + k(10);
        // 英文的「Check for updates」比較長
        let check_w = k(tr!(110, 150));
        check = button(dlg, CHECK_UPDATES, Kind::Text, tr!("檢查更新", "Check for updates"), RECT::new(cx, y, check_w, row_h), &st.fonts);
        st.pages[p].push(check);
        update_note = note(dlg, "", RECT::new(cx + check_w + k(12), y + (row_h - note_h) / 2, right - cx - check_w - k(12), note_h), f_small);
        st.pages[p].push(update_note);
        y += row_h + k(18);
        let (h, _) = toggle(
            &mut st,
            p,
            T_REMINDERS,
            tr!("啟動時提醒更新", "Remind me about updates"),
            Some(tr!("Stillfeather 開著時在背景檢查，有新版本就提醒；關掉後仍可按「檢查更新」。", "Stillfeather checks in the background while it's open and tells you about new versions. You can still use Check for updates when this is off.")),
            x0,
            y,
        );
        st.reminders = h;
    }
    DIALOG.store(dlg, Ordering::Relaxed);
    UPDATE_NOTE.store(update_note, Ordering::Relaxed);
    UPDATE_BUTTON.store(check, Ordering::Relaxed);
    UPDATE_READY.store(false, Ordering::Relaxed);
    UPDATE_NOW.store(false, Ordering::Relaxed);

    // 底部固定：還原這個分類的預設值、儲存、取消
    let by = bar_top + (k(BAR_H) - k(36)) / 2;
    st.reset = button(dlg, RESET, Kind::Text, &reset_text(P_LOOK), RECT::new(k(20), by, k(200), k(36)), &st.fonts);
    let cancel = button(dlg, IDCANCEL as u16, Kind::Text, tr!("取消", "Cancel"), RECT::new(cw - k(MARGIN) - k(96), by, k(96), k(36)), &st.fonts);
    let save_btn = button(dlg, IDOK as u16, Kind::Text, tr!("儲存", "Save"), RECT::new(cw - k(MARGIN) - k(96) * 2 - k(10), by, k(96), k(36)), &st.fonts);
    controls::set_primary(save_btn);
    let _ = cancel;
    for page in 0..PAGES {
        fill_page(&st, page, current, integration);
    }

    let raw = Box::into_raw(st);
    unsafe {
        SetWindowLongPtrW(dlg, GWLP_USERDATA, raw as isize);
        show_page(dlg, &mut *raw, P_LOOK);
        size_changed(&*raw);
        EnableWindow(owner, 0);
        ShowWindow(dlg, SW_SHOW);
        SetFocus((*raw).nav[P_LOOK]);
        let mut msg: MSG = std::mem::zeroed();
        let mut quit = None;
        // 對話方塊程序透過同一個指標設定 done；以 volatile 讀取，明確表示它會在迴圈外被改變。
        while !std::ptr::read_volatile(std::ptr::addr_of!((*raw).done)) {
            let r = GetMessageW(&mut msg, 0, 0, 0);
            if r <= 0 {
                quit = Some(msg.wParam as i32);
                break;
            }
            // 對話方塊管理員在按到沒有對應的字母或 Alt＋字母時會發出提示音；
            // 只有文字框需要字元，其他控制項的字元訊息直接略過。
            if (msg.message == WM_CHAR || msg.message == WM_SYSCHAR) && (msg.hwnd == dlg || IsChild(dlg, msg.hwnd) != 0) {
                let is_edit = class_name(msg.hwnd).eq_ignore_ascii_case("Edit");
                if msg.message == WM_SYSCHAR || !is_edit {
                    continue;
                }
            }
            if msg.message == WM_KEYDOWN && (msg.hwnd == dlg || IsChild(dlg, msg.hwnd) != 0) && nav_key(dlg, &mut *raw, msg.hwnd, msg.wParam as u32) {
                continue;
            }
            if IsDialogMessageW(dlg, &mut msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DIALOG.store(0, Ordering::Relaxed);
        UPDATE_NOTE.store(0, Ordering::Relaxed);
        UPDATE_BUTTON.store(0, Ordering::Relaxed);
        UPDATE_READY.store(false, Ordering::Relaxed);
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
        SetWindowLongPtrW(dlg, GWLP_USERDATA, 0);
        DestroyWindow(dlg);
        // 控制項都銷毀了才刪字型（State 的 Fonts 在這裡 Drop）
        let st = Box::from_raw(raw);
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
        st.result.map(|r| (r, st.integration))
    }
}

/// 分類的鍵盤操作：Ctrl+Tab／Ctrl+Shift+Tab、Ctrl+PgDn／PgUp 換分類；焦點在左邊的導覽時 ↑／↓ 也換。
unsafe fn nav_key(dlg: HWND, st: &mut State, target: HWND, vk: u32) -> bool {
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(VK_SHIFT);
    let on_nav = st.nav.contains(&target);
    let delta: isize = match vk {
        VK_TAB if ctrl => {
            if shift {
                -1
            } else {
                1
            }
        }
        VK_NEXT if ctrl => 1,
        VK_PRIOR if ctrl => -1,
        VK_DOWN if on_nav => 1,
        VK_UP if on_nav => -1,
        _ => return false,
    };
    let page = (st.page as isize + delta).rem_euclid(PAGES as isize) as usize;
    show_page(dlg, st, page);
    if on_nav || ctrl {
        SetFocus(st.nav[page]);
    }
    true
}

/// 範例檔名：用選到的文字大小畫一列清單（圖示、檔名、大小），儲存前就看得到效果
unsafe fn paint_preview(hdc: HDC, st: &State) {
    let t = app::theme();
    let p = &t.pal;
    let r = st.preview;
    theme::round_rect(hdc, &r, t.s(10), Some(p.background), Some(p.line));
    let pct = st.text_size();
    let font = theme::ui_font(9, pct, false, t.dpi);
    let icon_x = r.left + st.k(14);
    if let Some(a) = app::app_opt() {
        let index = a.icons.file_index("docx");
        if index >= 0 {
            (comctl().image_list_draw)(a.icons.list, index, hdc, icon_x, r.top + (r.height() - a.icons.row_px) / 2, ILD_TRANSPARENT);
        }
    }
    let icon_w = GetSystemMetrics(SM_CXSMICON).max(16);
    let size_text = "24.5 KB";
    let size_w = theme::text_width(hdc, size_text, font);
    let name_r = RECT { left: icon_x + icon_w + st.k(10), top: r.top, right: r.right - st.k(14) - size_w - st.k(14), bottom: r.bottom };
    // 中英混排、在最大的文字大小下也放得下（不被「…」切掉）
    theme::draw_text(hdc, tr!("會議紀錄 Notes.docx", "Meeting notes.docx"), &name_r, font, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    let size_r = RECT { left: r.right - st.k(14) - size_w, right: r.right - st.k(14), ..r };
    theme::draw_text(hdc, size_text, &size_r, font, p.muted, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
    DeleteObject(font);
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(st) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    let t = app::theme();
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let rect = client_rect(hwnd);
            {
                let buf = theme::Buffered::new(hdc, rect);
                theme::fill(buf.hdc, &rect, t.pal.surface);
                let bar_top = rect.bottom - st.k(BAR_H);
                // 左邊的分類導覽：側欄的底色
                let nav = RECT { left: 0, top: 0, right: st.k(NAV_W), bottom: bar_top };
                theme::fill(buf.hdc, &nav, t.pal.sidebar);
                let focus = GetFocus();
                for (frame, edit, page) in &st.frames {
                    if *page == st.page {
                        let border = if focus == *edit { t.pal.accent } else { t.pal.line };
                        theme::round_rect(buf.hdc, frame, st.k(8), Some(t.pal.surface), Some(border));
                    }
                }
                if st.page == P_LOOK {
                    paint_preview(buf.hdc, st);
                }
                let line = RECT { left: 0, top: bar_top, right: rect.right, bottom: bar_top + 1 };
                theme::fill(buf.hdc, &line, t.pal.line);
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let is_note = GetWindowLongPtrW(lparam as HWND, -12 /* GWLP_ID */) == NOTE_ID as isize;
            SetTextColor(hdc, if is_note { t.pal.muted } else { t.pal.ink });
            SetBkColor(hdc, t.pal.surface);
            return t.brushes.surface;
        }
        WM_COMMAND => {
            let cid = loword(wparam);
            let code = hiword(wparam) as u32;
            if code == EN_SETFOCUS || code == EN_KILLFOCUS {
                InvalidateRect(hwnd, std::ptr::null(), 0);
                return 0;
            }
            match cid {
                c if (NAV_FIRST..NAV_FIRST + PAGES as u16).contains(&c) => show_page(hwnd, st, (c - NAV_FIRST) as usize),
                c if (SIZE_FIRST..SIZE_FIRST + TEXT_SIZES.len() as u16).contains(&c) => {
                    check_one(&st.sizes, Some((c - SIZE_FIRST) as usize));
                    size_changed(st);
                }
                SEG_SYSTEM | SEG_LIGHT | SEG_DARK => check_one(&st.segments, Some((cid - SEG_SYSTEM) as usize)),
                LANG_SYSTEM | LANG_CHINESE | LANG_ENGLISH => check_one(&st.languages, Some((cid - LANG_SYSTEM) as usize)),
                ACCENT_PICK => {
                    let target = st.accent;
                    pick_color(hwnd, st, target, t.pal.accent);
                }
                BG_PICK => {
                    let target = st.background;
                    pick_color(hwnd, st, target, t.pal.surface);
                }
                CHECK_UPDATES => {
                    if UPDATE_READY.load(Ordering::Relaxed) {
                        // 立即更新：和「儲存」一樣先儲存並關閉設定，主視窗接著下載更新
                        if save(hwnd, st) {
                            UPDATE_NOW.store(true, Ordering::Relaxed);
                        }
                    } else {
                        let text = if super::update::start_check(true) { tr!("正在檢查…", "Checking…") } else { tr!("這一版沒有設定更新來源", "This build has no update source") };
                        update_status(text);
                    }
                }
                RESET => {
                    // 只還原目前這個分類；常用位置等其他設定不動，按「儲存」才套用
                    let mut d = Settings::default();
                    d.favorites = st.base.favorites.clone();
                    fill_page(st, st.page, &d, Integration::DEFAULT);
                }
                // 攔截 Win+E 只在設為預設檔案總管時有作用
                T_DEFAULT_FM => win_e_changed(st),
                c if c == IDOK as u16 => {
                    save(hwnd, st);
                }
                c if c == IDCANCEL as u16 => st.done = true,
                _ => {}
            }
            return 0;
        }
        WM_CLOSE => {
            st.done = true;
            return 0;
        }
        WM_SETCURSOR => {
            if swallow_modal_click(hwnd, lparam) {
                return 1;
            }
        }
        WM_SYSCOMMAND => {
            if wparam & 0xFFF0 == SC_KEYMENU && lparam != b' ' as LPARAM {
                return 0;
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_shrinks_to_fit_small_screens() {
        let big = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        assert_eq!(layout_percent(100, &big, 96), 100);
        assert_eq!(layout_percent(150, &big, 96), 150);
        // 1366×728 的筆電：150% 放不下，縮到放得下的最大比例
        let small = RECT { left: 0, top: 0, right: 1366, bottom: 728 };
        let p = layout_percent(150, &small, 96);
        assert!(p < 150 && p >= 100, "{p}");
        assert!(BASE_H * p as i32 / 100 <= 728 * 92 / 100);
        // 再小也不低於 100%
        let tiny = RECT { left: 0, top: 0, right: 800, bottom: 500 };
        assert_eq!(layout_percent(150, &tiny, 96), 100);
    }

    #[test]
    fn reset_button_names_its_scope() {
        for page in 0..PAGES {
            assert!(reset_text(page).contains(page_title(page)));
        }
    }
}

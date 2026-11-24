//! 選擇器模式（`Stillfeather.exe --pick <請求檔>`）：其他程式要開啟檔案、另存新檔或選擇資料夾時開的精簡視窗。
//!
//! 由 picker/ 的取代元件啟動（見 src/picker.rs 的流程說明）。和一般的 Stillfeather 是不同的程序：
//! 不佔用單一實例、不檢查更新、不寫入設定檔與資料夾檢視記錄，記錄檔另外寫成 picker-trace.log 等。
//!
//! 介面只保留選檔需要的部分：上一頁／下一頁／上一層、重新整理、路徑列、搜尋、新增資料夾、排序與檢視、
//! 左側資料夾樹、檔案清單，以及底部的檔名欄、檔案類型、確定與取消。沒有分頁、雙欄與個人化卡片。
//! 選好後把路徑寫進結果檔，以結束代碼告訴取代元件是確定、取消，還是改用 Windows 原本的視窗。
//!
//! 另存新檔（`Mode::Save`）：檔名欄一開始就有焦點並選取主檔名，存檔時照 Windows 的做法補上副檔名、
//! 換類型時跟著換副檔名、檔案已經存在時先問要不要取代。檔案可以還不存在（結果是要存的位置）。

use super::app::{self, id, App, NavKind};
use super::controls::{self, Kind, Surface};
use super::dialog::{self, Icon};
use super::fs;
use super::shellreg;
use super::menu::Item;
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::install;
use feather_core::paths;
use feather_core::picker::{self, Filter, Mode, Request, Response};
use feather_core::workspace::Side;
use std::sync::atomic::{AtomicBool, Ordering};

/// 這個程序是選擇器（不寫設定檔、資料夾檢視記錄）
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// 每秒確認要求的程式還在；它結束了（例如當機）選擇器也跟著關閉
pub const TIMER_HOST: usize = 15;
/// 類型選單的命令：PICK_TYPE_FIRST + 第幾個類型
const MAX_TYPES: usize = 99;

pub(super) struct Picker {
    pub req: Request,
    /// 目前的檔案類型（從 0 起算）
    pub type_index: usize,
    pub name_edit: HWND,
    pub type_btn: HWND,
    pub ok_btn: HWND,
    pub cancel_btn: HWND,
    /// 右上角的「使用 Windows 視窗」
    pub system_btn: HWND,
    /// 要求的程式（結束時選擇器跟著關閉）
    host: HANDLE,
    pub exit_code: i32,
    finishing: bool,
}

impl Picker {
    pub fn new(req: Request) -> Picker {
        let type_index = req.initial_type();
        let host = if req.host_pid != 0 { unsafe { OpenProcess(SYNCHRONIZE, 0, req.host_pid) } } else { 0 };
        Picker {
            req,
            type_index,
            name_edit: 0,
            type_btn: 0,
            ok_btn: 0,
            cancel_btn: 0,
            system_btn: 0,
            host,
            exit_code: picker::EXIT_CANCEL,
            finishing: false,
        }
    }

    /// 清單目前的篩選
    pub fn filter(&self) -> Filter {
        if self.req.mode == Mode::Folders {
            return Filter::folders();
        }
        self.req.types.get(self.type_index).map(|t| Filter::from_spec(&t.spec)).unwrap_or_default()
    }

    pub fn shows_types(&self) -> bool {
        self.req.mode != Mode::Folders && !self.req.types.is_empty()
    }

    fn saving(&self) -> bool {
        self.req.mode == Mode::Save
    }

    /// 另存新檔時目前的類型要加的副檔名
    fn save_extension(&self) -> Option<String> {
        self.req.save_extension(self.type_index)
    }

    fn type_label(&self) -> String {
        self.req.types.get(self.type_index).map(|t| t.label()).unwrap_or_default()
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        if self.host != 0 {
            unsafe {
                CloseHandle(self.host);
            }
        }
    }
}

/// `--pick <請求檔>` 的進入點。讀不到請求時回傳 2：取代元件會改開 Windows 原本的視窗。
pub fn run(request_path: &str) -> i32 {
    let req = match std::fs::read_to_string(request_path).map_err(|e| e.to_string()).and_then(|t| Request::parse(&t)) {
        Ok(r) => r,
        Err(_) => return 2,
    };
    ACTIVE.store(true, Ordering::Relaxed);
    super::crashlog::install_named("picker-");
    super::crashlog::crumb("選擇器：啟動");
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        OleInitialize(std::ptr::null_mut());
    }
    let code = app::run(Vec::new(), Some(Picker::new(req)));
    super::crashlog::crumb("選擇器：結束");
    code
}

/// 每個程式上次用的資料夾記在設定檔旁的 picker-folders.txt
fn recent_path() -> Option<String> {
    let settings = app::settings_path().ok()?;
    Some(paths::join(&paths::parent(&settings)?, "picker-folders.txt"))
}

fn text_px(hwnd: HWND, text: &str, font: HFONT) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        let w = theme::text_width(dc, text, font);
        ReleaseDC(hwnd, dc);
        w
    }
}

impl App {
    pub(super) fn picker_mode(&self) -> bool {
        self.picker.is_some()
    }

    /// 建立底部列的檔名欄、類型選單、確定與取消，以及右上角的「使用 Windows 視窗」。
    pub(super) fn create_picker_controls(&mut self) {
        let hwnd = self.hwnd;
        let Some(p) = self.picker.as_mut() else { return };
        p.name_edit = app::create_edit(hwnd, id::PICK_NAME, &p.req.name_text());
        p.type_btn = controls::create(hwnd, id::PICK_TYPE, Kind::Menu(glyph::FILTER), &p.type_label(), Surface::Card, false);
        p.ok_btn = controls::create(hwnd, id::PICK_OK, Kind::Text, &p.req.ok_text(), Surface::Card, false);
        controls::set_primary(p.ok_btn);
        p.cancel_btn = controls::create(hwnd, id::PICK_CANCEL, Kind::Text, tr!("取消", "Cancel"), Surface::Card, false);
        p.system_btn = controls::create(hwnd, id::PICK_SYSTEM, Kind::Text, tr!("使用 Windows 視窗", "Use Windows dialog"), Surface::Page, false);
        controls::set_subtle(p.system_btn);
        let font = self.theme.fonts.ui;
        let name_edit = p.name_edit;
        let multi = p.req.multi;
        let file_name = p.req.file_name.clone();
        send(name_edit, WM_SETFONT, font as WPARAM, 1);
        let cue = wide(match p.req.mode {
            Mode::Folders => tr!("選取資料夾，或輸入路徑", "Select a folder, or type a path"),
            Mode::Save => tr!("輸入檔案名稱", "Type a file name"),
            Mode::Files => tr!("選取檔案，或輸入名稱、路徑", "Select files, or type a name or path"),
        });
        send_ptr(name_edit, EM_SETCUEBANNER, 1, cue.as_ptr());
        if !file_name.is_empty() {
            set_window_text(name_edit, &file_name);
        }
        // 只能選一個時清單也只能單選
        if !multi {
            for list in self.lists {
                unsafe {
                    let style = GetWindowLongPtrW(list, GWL_STYLE);
                    SetWindowLongPtrW(list, GWL_STYLE, style | LVS_SINGLESEL as isize);
                }
            }
        }
    }

    /// 第一個資料夾：程式指定一定要用的 → 這個程式上次用的 → 程式建議的 → 使用者資料夾。
    /// 檔名欄預先填了檔名時順便選取它。
    pub(super) fn picker_start(&mut self) {
        let Some(p) = self.picker.as_ref() else { return };
        let recent = recent_path().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| picker::recent_folder(&t, &p.req.host));
        let candidates = [Some(p.req.folder.clone()), recent, Some(p.req.default_folder.clone())];
        let folder = candidates.into_iter().flatten().find(|f| !f.trim().is_empty() && fs::is_dir(f)).unwrap_or_else(|| self.default_path.clone());
        let name = p.req.file_name.trim().to_string();
        // 另存新檔不選取同名的檔案（選取會把檔名欄換成那個名稱，蓋掉一開始選好的主檔名）
        let saving = p.saving();
        let select = if saving || name.is_empty() || picker::is_pattern(&name) { vec![] } else { vec![picker::resolve(&folder, &name)] };
        let filter = p.filter();
        let id = self.active_tab_id();
        if let Some(t) = self.tabs.get_mut(&id) {
            t.pick = Some(filter);
        }
        self.navigate(id, &folder, NavKind::NoHistory, select);
        unsafe {
            SetForegroundWindow(self.hwnd);
        }
        if saving {
            self.picker_focus_name();
        }
        if self.picker.as_ref().is_some_and(|p| p.host != 0) {
            unsafe {
                SetTimer(self.hwnd, TIMER_HOST, 1000, std::ptr::null());
            }
        }
    }

    /// 要求的程式已經結束：選擇器也關閉。
    pub(super) fn picker_host_timer(&mut self) {
        let host = self.picker.as_ref().map_or(0, |p| p.host);
        if host != 0 && unsafe { WaitForSingleObject(host, 0) } == WAIT_OBJECT_0 {
            super::crashlog::crumb("選擇器：要求的程式已結束");
            self.picker_finish(picker::EXIT_CANCEL);
        }
    }

    /// 底部列的版面（清單卡片下方、整個內容區寬度）。回傳要搬動的子視窗。
    pub(super) fn picker_bar_layout(&self, bar: RECT) -> Vec<(HWND, RECT, bool)> {
        let Some(p) = self.picker.as_ref() else { return vec![] };
        let s = |v: i32| self.s(v);
        let bh = s(36);
        let by = bar.top + (bar.height() - bh) / 2;
        let font = self.theme.fonts.ui;
        let ok_w = (text_px(self.hwnd, &p.req.ok_text(), font) + s(40)).clamp(s(96), s(200));
        let cancel_w = s(88);
        let mut x = bar.right - s(14);
        let ok = RECT::new(x - ok_w, by, ok_w, bh);
        x = ok.left - s(8);
        let cancel = RECT::new(x - cancel_w, by, cancel_w, bh);
        x = cancel.left - s(14);
        let types = p.shows_types();
        let type_w = if types { self.picker_type_width(bar) } else { 0 };
        let type_r = RECT::new(x - type_w, by, type_w, bh);
        let frame = self.picker_name_frame(bar);
        let edit_h = self.theme.fonts.ui_height + s(2);
        let top = frame.top + (frame.height() - edit_h) / 2;
        let edit = RECT { left: frame.left + s(14), top, right: (frame.right - s(14)).max(frame.left + s(40)), bottom: top + edit_h };
        vec![(p.name_edit, edit, true), (p.type_btn, type_r, types), (p.cancel_btn, cancel, true), (p.ok_btn, ok, true)]
    }

    /// 檔名欄的外框（主視窗自己畫）
    pub(super) fn picker_name_frame(&self, bar: RECT) -> RECT {
        let Some(p) = self.picker.as_ref() else { return RECT::default() };
        let s = |v: i32| self.s(v);
        let left = bar.left + s(20) + self.picker_label_width() + s(12);
        let font = self.theme.fonts.ui;
        let ok_w = (text_px(self.hwnd, &p.req.ok_text(), font) + s(40)).clamp(s(96), s(200));
        let mut right = bar.right - s(14) - ok_w - s(8) - s(88) - s(14);
        if p.shows_types() {
            right -= self.picker_type_width(bar) + s(12);
        }
        let h = s(38);
        RECT { left, top: bar.top + (bar.height() - h) / 2, right: right.max(left + s(60)), bottom: bar.top + (bar.height() - h) / 2 + h }
    }

    /// 檔名欄前面文字的寬度（以畫它的粗體量，DPI 改變後也正確）
    fn picker_label_width(&self) -> i32 {
        self.picker.as_ref().map_or(0, |p| text_px(self.hwnd, &p.req.name_text(), self.theme.fonts.ui_bold))
    }

    /// 類型選單按鈕的寬度：依目前類型的名稱，最多底部列的三分之一
    fn picker_type_width(&self, bar: RECT) -> i32 {
        let Some(p) = self.picker.as_ref() else { return 0 };
        let s = |v: i32| self.s(v);
        (text_px(self.hwnd, &p.type_label(), self.theme.fonts.ui) + s(72)).clamp(s(150), s(300)).min((bar.width() / 3).max(s(150)))
    }

    /// 點在檔名欄的外框上（輸入框以外的留白）：把焦點放進輸入框
    pub(super) fn picker_click(&mut self, x: i32, y: i32) -> bool {
        let Some(p) = self.picker.as_ref() else { return false };
        if !self.picker_name_frame(self.layout.footer).contains(x, y) {
            return false;
        }
        unsafe {
            SetFocus(p.name_edit);
        }
        true
    }

    /// 畫底部列：卡片、檔名欄前面的文字與外框（有焦點時重點色）
    pub(super) unsafe fn paint_picker_bar(&self, dc: HDC, bar: RECT) {
        let Some(p) = self.picker.as_ref() else { return };
        let pal = self.theme.pal;
        let s = |v: i32| self.s(v);
        theme::card(dc, &bar, s(22), pal.surface, Some(theme::blend(pal.ink, pal.surface, 0.05)), &pal, self.theme.dpi);
        let label = RECT { left: bar.left + s(20), top: bar.top, right: bar.left + s(20) + self.picker_label_width() + s(4), bottom: bar.bottom };
        theme::draw_text(dc, &p.req.name_text(), &label, self.theme.fonts.ui_bold, pal.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        let frame = self.picker_name_frame(bar);
        let border = if GetFocus() == p.name_edit { pal.accent } else { pal.line };
        theme::round_rect(dc, &frame, frame.height(), Some(pal.surface), Some(border));
    }

    /// 焦點放進檔名欄，選取主檔名（不含副檔名），直接打字就換掉名稱
    fn picker_focus_name(&self) {
        let Some(p) = self.picker.as_ref() else { return };
        let text = window_text(p.name_edit);
        let stem = match picker::extension_of(&text) {
            Some(ext) => text.len() - ext.len() - 1,
            None => text.len(),
        };
        let stem_units = text[..stem].encode_utf16().count();
        unsafe {
            SetFocus(p.name_edit);
        }
        send(p.name_edit, EM_SETSEL, 0, stem_units as LPARAM);
    }

    /// 選取改變：選了檔案（選資料夾時是資料夾）就把名稱填進檔名欄，其他情況保留使用者打的字。
    pub(super) fn picker_selection_changed(&mut self, side: Side) {
        let Some(p) = self.picker.as_ref() else { return };
        let folders = p.req.mode == Mode::Folders;
        let edit = p.name_edit;
        let entries = self.selected_entries(side);
        let names: Vec<&str> = entries.iter().filter(|e| e.is_dir == folders).map(|e| e.name.as_str()).collect();
        if names.is_empty() {
            return;
        }
        set_window_text(edit, &picker::names_text(&names));
    }

    /// 清單上按 Enter、雙擊或右鍵選單的「開啟」：資料夾就進去，檔案就選定。
    pub(super) fn picker_activate(&mut self, side: Side) {
        let Some(mode) = self.picker.as_ref().map(|p| p.req.mode) else { return };
        if self.active_tab().is_some_and(|t| t.loading) {
            return;
        }
        let entries = self.selected_entries(side);
        let files: Vec<String> = entries.iter().filter(|e| !e.is_dir).map(|e| e.full_path()).collect();
        let dirs: Vec<String> = entries.iter().filter(|e| e.is_dir).map(|e| e.full_path()).collect();
        if mode == Mode::Files && !files.is_empty() {
            self.picker_accept(files);
        } else if mode == Mode::Save && !files.is_empty() {
            // 另存新檔時雙擊已有的檔案：存成它（先問要不要取代）
            self.picker_save(files[0].clone());
        } else if let Some(dir) = dirs.first().filter(|_| dirs.len() == 1 || files.is_empty()) {
            let id = self.active_tab_id();
            let dir = dir.clone();
            self.navigate(id, &dir, NavKind::Normal, vec![]);
            self.picker_clear_name();
        }
    }

    /// 進到另一個資料夾後清掉檔名欄。另存新檔時保留打好的檔名（換資料夾不換檔名）。
    fn picker_clear_name(&mut self) {
        if let Some(p) = self.picker.as_ref().filter(|p| !p.saving()) {
            set_window_text(p.name_edit, "");
        }
    }

    /// 確定按鈕或在檔名欄按 Enter。
    pub(super) fn picker_confirm(&mut self) {
        let Some(p) = self.picker.as_ref() else { return };
        let (mode, multi, must_exist, edit) = (p.req.mode, p.req.multi, p.req.must_exist, p.name_edit);
        let text = window_text(edit).trim().to_string();
        let Some(folder) = self.active_tab().filter(|t| !t.loading).map(|t| t.path.clone()) else { return };
        let side = self.active_side();
        if mode == Mode::Save {
            self.picker_save_confirm(&text, &folder);
            return;
        }
        if text.is_empty() {
            match mode {
                Mode::Files | Mode::Save => self.picker_activate(side),
                Mode::Folders => {
                    // 沒有選也沒有打字：選目前的資料夾
                    let dirs: Vec<String> = self.selected_entries(side).iter().filter(|e| e.is_dir).map(|e| e.full_path()).collect();
                    if dirs.is_empty() {
                        if !folder.is_empty() {
                            self.picker_accept(vec![folder]);
                        }
                    } else {
                        self.picker_accept(if multi { dirs } else { dirs[..1].to_vec() });
                    }
                }
            }
            return;
        }
        // 打的是萬用字元：當作臨時的類型篩選（與 Windows 的視窗相同）
        if mode == Mode::Files && picker::is_pattern(&text) {
            let filter = Filter::from_spec(&text);
            let id = self.active_tab_id();
            if let Some(t) = self.tabs.get_mut(&id) {
                t.pick = Some(filter);
                t.refilter();
            }
            self.refresh_list(side, true);
            self.update_chrome();
            return;
        }
        // 搜尋結果的名稱以結果所在的資料夾為準
        let selected = self.selected_entries(side);
        let names = picker::parse_names(&text);
        let full: Vec<String> = names
            .iter()
            .map(|n| match selected.iter().find(|e| e.name == *n) {
                Some(e) => e.full_path(),
                None => picker::resolve(&folder, n),
            })
            .collect();
        // 在首頁（沒有目前的資料夾）打的名稱不是完整路徑：不能交給程式（會變成相對於程式目前目錄的位置），
        // 也不能拿去檢查是不是資料夾（同樣會相對於目前目錄）
        if full.iter().any(|f| !paths::is_fully_qualified(f)) {
            self.picker_needs_folder(false);
            return;
        }
        // 一個名稱而且是資料夾：進去（選資料夾時則是選定它）
        if full.len() == 1 && fs::is_dir(&full[0]) {
            if mode == Mode::Folders {
                self.picker_accept(full);
            } else {
                let id = self.active_tab_id();
                self.navigate(id, &full[0], NavKind::Normal, vec![]);
                self.picker_clear_name();
            }
            return;
        }
        if !multi && full.len() > 1 {
            dialog::Spec::new(Icon::Warning, tr!("只能選一個", "Only one item can be selected")).body(tr!("這個程式一次只接受一個檔案。", "This program accepts only one file at a time.")).show(self.hwnd);
            return;
        }
        let missing: Vec<&String> = full.iter().filter(|f| if mode == Mode::Folders { !fs::is_dir(f) } else { !fs::is_file(f) }).collect();
        if !missing.is_empty() && (must_exist || mode == Mode::Folders) {
            let what = if mode == Mode::Folders { tr!("資料夾", "folder") } else { tr!("檔案", "file") };
            let list: Vec<String> = missing.iter().take(5).map(|f| paths::file_name(f).to_string()).collect();
            dialog::Spec::new(Icon::Warning, &trf!("找不到{what}", "Can't find the {what}")).body(&trf!("{}\n\n請確認名稱是否正確。", "{}\n\nCheck that the name is correct.", list.join("\n"))).show(self.hwnd);
            return;
        }
        self.picker_accept(full);
    }

    /// 另存新檔的確定：檔名欄打的名稱補上副檔名，變成要存的位置。
    fn picker_save_confirm(&mut self, text: &str, folder: &str) {
        let Some(p) = self.picker.as_ref() else { return };
        let side = self.active_side();
        if text.is_empty() {
            // 沒打檔名：選了資料夾就進去，否則提醒要輸入檔名
            if self.selected_entries(side).iter().any(|e| e.is_dir) {
                self.picker_activate(side);
            } else {
                self.picker_focus_name();
            }
            return;
        }
        // 打的是萬用字元：當作臨時的類型篩選（與 Windows 的視窗相同）
        if picker::is_pattern(text) {
            let filter = Filter::from_spec(text);
            let id = self.active_tab_id();
            if let Some(t) = self.tabs.get_mut(&id) {
                t.pick = Some(filter);
                t.refilter();
            }
            self.refresh_list(side, true);
            self.update_chrome();
            return;
        }
        // 選了搜尋結果裡的檔案：以它所在的資料夾為準
        if let Some(e) = self.selected_entries(side).into_iter().find(|e| !e.is_dir && e.name == text) {
            self.picker_save(e.full_path());
            return;
        }
        let ext = p.save_extension();
        let strict = p.req.strict_types.then(|| p.filter());
        let original = p.req.file_name.clone();
        let edit = p.name_edit;
        // 打的是資料夾（例如 `..`、`D:\工作`）：進去，檔名欄換回原本的檔名
        let raw = picker::resolve(folder, text.trim_matches('"'));
        if paths::is_fully_qualified(&raw) && fs::is_dir(&raw) {
            let id = self.active_tab_id();
            self.navigate(id, &raw, NavKind::Normal, vec![]);
            set_window_text(edit, &original);
            self.picker_focus_name();
            return;
        }
        let name = picker::save_name(text, ext.as_deref(), strict.as_ref(), known_extension);
        let file = paths::file_name(&name).trim().to_string();
        if file.is_empty() {
            self.picker_focus_name();
            return;
        }
        if let Some(c) = picker::invalid_name_char(&name) {
            dialog::Spec::new(Icon::Warning, tr!("檔名不能用這個字元", "This character can't be used in file names")).body(&trf!("「{c}」不能出現在檔名裡。\n\n檔名不能包含下列字元：\\ / : * ? \" < > |", "“{c}” can't be used in a file name.\n\nFile names can't contain any of these characters: \\ / : * ? \" < > |")).show(self.hwnd);
            self.picker_focus_name();
            return;
        }
        if picker::reserved_name(&name) {
            dialog::Spec::new(Icon::Warning, tr!("這個名稱不能當檔名", "This name can't be used as a file name")).body(&trf!("「{file}」是 Windows 保留給裝置的名稱，請換一個名稱。", "“{file}” is reserved by Windows for a device. Choose a different name.")).show(self.hwnd);
            self.picker_focus_name();
            return;
        }
        let full = picker::resolve(folder, &name);
        // 在首頁（沒有目前的資料夾）：不是完整路徑，存不了（Windows 的視窗在「本機」也不能存檔）
        if !paths::is_fully_qualified(&full) {
            self.picker_needs_folder(true);
            return;
        }
        if let Some(parent) = paths::parent(&full).filter(|d| !fs::is_dir(d)) {
            dialog::Spec::new(Icon::Warning, tr!("找不到資料夾", "Folder not found")).body(&trf!("{parent}\n\n請確認路徑是否正確。", "{parent}\n\nCheck that the path is correct.")).show(self.hwnd);
            self.picker_focus_name();
            return;
        }
        if fs::is_dir(&full) {
            let id = self.active_tab_id();
            self.navigate(id, &full, NavKind::Normal, vec![]);
            set_window_text(edit, &original);
            self.picker_focus_name();
            return;
        }
        self.picker_save(full);
    }

    /// 在首頁打了名稱：要先進到一個資料夾，或打完整路徑
    fn picker_needs_folder(&mut self, saving: bool) {
        let heading = if saving { tr!("這裡不能存檔，請選一個資料夾", "You can't save here. Choose a folder") } else { tr!("請先選一個資料夾", "Choose a folder first") };
        dialog::Spec::new(Icon::Warning, heading)
            .body(tr!("首頁不是資料夾。請先進到一個資料夾再輸入名稱，或輸入完整路徑（例如 D:\\文件\\報告.txt）。", "Home isn't a folder. Open a folder before typing a name, or type a full path (for example D:\\Documents\\Report.txt)."))
            .show(self.hwnd);
        self.picker_focus_name();
    }

    /// 存成 `path`：已經存在時先問要不要取代（程式要求時），唯讀的檔案不能選（程式要求時）。
    fn picker_save(&mut self, path: String) {
        let Some(p) = self.picker.as_ref() else { return };
        let (overwrite_prompt, no_readonly) = (p.req.overwrite_prompt, p.req.no_readonly);
        if let Some(attrs) = fs::attributes(&path).filter(|a| a & FILE_ATTRIBUTE_DIRECTORY == 0) {
            let name = paths::file_name(&path).to_string();
            if no_readonly && attrs & FILE_ATTRIBUTE_READONLY != 0 {
                dialog::Spec::new(Icon::Warning, tr!("這個檔案是唯讀的", "This file is read-only")).body(&trf!("無法取代「{name}」。請換一個名稱，或先取消檔案的唯讀屬性。", "Can't replace “{name}”. Choose a different name, or clear the file's read-only attribute first.")).show(self.hwnd);
                self.picker_focus_name();
                return;
            }
            if overwrite_prompt && !dialog::confirm(self.hwnd, Icon::Warning, &trf!("「{name}」已經存在", "“{name}” already exists"), tr!("要取代它嗎？原本的內容會被新存的檔案蓋掉。", "Do you want to replace it? The new file will overwrite what's in it now."), tr!("取代", "Replace")) {
                self.picker_focus_name();
                return;
            }
        }
        self.picker_accept(vec![path]);
    }

    /// 類型選單
    pub(super) fn picker_type_menu(&mut self) {
        self.hide_preview();
        let Some(p) = self.picker.as_ref() else { return };
        let labels: Vec<String> = p.req.types.iter().take(MAX_TYPES).map(|t| t.label()).collect();
        let current = p.type_index;
        let button = p.type_btn;
        let items: Vec<Item> = labels.iter().enumerate().map(|(i, l)| Item::radio(id::PICK_TYPE_FIRST + i as u16, l, i == current)).collect();
        let Some(cmd) = super::menu::drop_down(self.hwnd, button, &items, &self.menu_look()) else { return };
        let index = (cmd - id::PICK_TYPE_FIRST) as usize;
        self.picker_set_type(index);
    }

    fn picker_set_type(&mut self, index: usize) {
        let Some(p) = self.picker.as_mut() else { return };
        if index >= p.req.types.len() {
            return;
        }
        let old_ext = p.save_extension();
        p.type_index = index;
        let filter = p.filter();
        let label = p.type_label();
        let button = p.type_btn;
        set_window_text(button, &label);
        invalidate(button);
        // 另存新檔：檔名欄的副檔名跟著換（報告.txt → 報告.html）
        if p.saving() {
            let edit = p.name_edit;
            if let Some(swapped) = picker::swap_extension(&window_text(edit), old_ext.as_deref(), p.save_extension().as_deref()) {
                set_window_text(edit, &swapped);
            }
        }
        let id = self.active_tab_id();
        if let Some(t) = self.tabs.get_mut(&id) {
            t.pick = Some(filter);
            t.refilter();
        }
        let side = self.active_side();
        self.refresh_list(side, true);
        self.relayout();
        self.update_chrome();
    }

    /// 選定：寫結果檔、記住這個程式用的資料夾，然後關閉。
    fn picker_accept(&mut self, chosen: Vec<String>) {
        let Some(p) = self.picker.as_ref() else { return };
        if chosen.is_empty() {
            return;
        }
        let type_index = if p.req.types.is_empty() { 0 } else { p.type_index as u32 + 1 };
        let out = p.req.out.clone();
        let host = p.req.host.clone();
        let response = Response { paths: chosen, type_index };
        if let Err(e) = app::write_atomic(&out, &response.to_text()) {
            self.show_error(&trf!("無法把選取的項目交給程式：{e}", "Couldn't pass the selected items to the program: {e}"));
            return;
        }
        let folder = self.active_tab().filter(|t| !t.search_mode).map(|t| t.path.clone()).unwrap_or_default();
        if let (Some(file), false) = (recent_path(), folder.is_empty() || host.is_empty()) {
            let old = std::fs::read_to_string(&file).unwrap_or_default();
            let _ = app::write_atomic(&file, &picker::remember_folder(&old, &host, &folder));
        }
        self.picker_finish(picker::EXIT_OK);
    }

    /// 關閉選擇器。先讓要求的程式恢復可以操作，關閉後焦點才會回到它的視窗。
    pub(super) fn picker_finish(&mut self, code: i32) {
        let Some(p) = self.picker.as_mut() else { return };
        if p.finishing {
            return;
        }
        p.finishing = true;
        p.exit_code = code;
        let owner = if p.req.enable_owner { p.req.owner } else { 0 };
        let pid = p.req.host_pid;
        self.hide_preview();
        unsafe {
            KillTimer(self.hwnd, TIMER_HOST);
            if owner != 0 && IsWindow(owner) != 0 {
                EnableWindow(owner, 1);
            }
            if pid != 0 {
                AllowSetForegroundWindow(pid);
            }
            DestroyWindow(self.hwnd);
        }
    }

    /// 選擇器的按鍵：檔名欄的 Enter／Esc，以及分頁、雙欄、設定這些在選擇器裡沒有的快捷鍵。
    /// 回傳 Some 表示已經處理（或要交給輸入框自己處理）。
    pub(super) fn picker_key(&mut self, vk: u32, ctrl: bool, shift: bool, alt: bool, focus: HWND) -> Option<bool> {
        let p = self.picker.as_ref()?;
        if focus == p.name_edit {
            return match vk {
                VK_RETURN => {
                    self.picker_confirm();
                    Some(true)
                }
                VK_ESCAPE => {
                    self.picker_finish(picker::EXIT_CANCEL);
                    Some(true)
                }
                VK_TAB if !ctrl && !alt => None,
                _ => Some(false),
            };
        }
        let letter = |c: u8| vk == c as u32;
        let blocked = (ctrl && !alt && (letter(b'T') || letter(b'N') || letter(b'W')))
            || (ctrl && vk == VK_TAB)
            || (ctrl && shift && (vk == VK_PRIOR || vk == VK_NEXT))
            || (ctrl && shift && !alt && letter(b'D'))
            || vk == VK_F12
            || (ctrl && vk == VK_OEM_COMMA);
        blocked.then_some(true)
    }

    /// 選擇器的命令（底部列與右上角的按鈕、類型選單）。回傳 true 表示已處理。
    pub(super) fn picker_command(&mut self, cmd: u16, code: u16, ctl: HWND) -> bool {
        let Some(p) = self.picker.as_ref() else { return false };
        if ctl != 0 && ctl == p.name_edit {
            let code = code as u32;
            if code == EN_SETFOCUS || code == EN_KILLFOCUS {
                let bar = self.layout.footer.inflate(self.s(4), self.s(4));
                unsafe {
                    InvalidateRect(self.hwnd, &bar, 0);
                }
            }
            return true;
        }
        match cmd {
            id::PICK_OK => self.picker_confirm(),
            id::PICK_CANCEL => self.picker_finish(picker::EXIT_CANCEL),
            id::PICK_SYSTEM => self.picker_finish(picker::EXIT_SYSTEM),
            id::PICK_TYPE => self.picker_type_menu(),
            _ => return false,
        }
        true
    }
}

/// 系統認得這個副檔名嗎（登錄的 HKEY_CLASSES_ROOT\.副檔名）：不認得的當成檔名的一部分，存檔時再補上副檔名
fn known_extension(ext: &str) -> bool {
    if ext.is_empty() || ext.contains([' ', '\\', '/']) {
        return false;
    }
    let key = wide(&format!(".{ext}"));
    let mut h: HKEY = 0;
    unsafe {
        if RegOpenKeyExW(HKEY_CLASSES_ROOT, key.as_ptr(), 0, KEY_QUERY_VALUE, &mut h) != 0 {
            return false;
        }
        RegCloseKey(h);
    }
    true
}

// ---------------------------------------------------------------- 開啟／關閉（設定）

/// 程式資料夾裡的取代元件（建置或安裝時放在 exe 旁邊）
fn bundled_dll() -> Option<String> {
    let exe = current_exe_path()?;
    let dll = paths::join(&paths::parent(&exe)?, picker::DLL_NAME);
    std::path::Path::new(&dll).is_file().then_some(dll)
}

/// 把程式資料夾裡的取代元件複製到登記的位置，回傳它的路徑
fn deploy_dll() -> Result<String, String> {
    let src = bundled_dll().ok_or_else(|| trf!("找不到 {}。開發版請重新執行 build.ps1；安裝版請重新安裝 Stillfeather。", "Can't find {}. For a development build, run build.ps1 again; for an installed copy, reinstall Stillfeather.", picker::DLL_NAME))?;
    shellreg::deploy_picker(&src)
}

/// 主程式啟動時依使用者的選擇處理（沒選過時預設關閉）：
/// - 開啟：還沒登記就登記（程式旁邊沒有取代元件時略過）；已經登記的更新成這一版（重新建置、更新之後），
///   記下的 Stillfeather 不見了就改用這一份。另一份 Stillfeather（例如安裝版）負責時不動。
/// - 關閉：還登記著就取消。
///
/// 程式資料夾和登記的那一份都不在時取消登記，免得其他程式開不了檔案。
pub(super) fn maintain() {
    if !shellreg::picker_choice() {
        if shellreg::picker_enabled() || shellreg::picker_save_enabled() {
            let _ = shellreg::disable_picker();
        }
        return;
    }
    let Some(exe) = current_exe_path() else { return };
    if shellreg::picker_enabled() {
        let recorded = shellreg::read_string(install::APP_KEY, install::PICKER_EXE_VALUE);
        let mine = recorded.as_deref().is_none_or(|p| install::same_path(p, &exe) || !std::path::Path::new(p).is_file());
        if !mine {
            return;
        }
    } else if bundled_dll().is_none() {
        return;
    }
    match deploy_dll() {
        Ok(dll) => {
            // 另一個程式已經取代了這個視窗時會失敗：不覆寫它，下次啟動再看
            let _ = shellreg::enable_picker(&dll, &exe);
        }
        Err(_) => {
            if shellreg::picker_registered_dll().is_some_and(|p| install::is_picker_dll(&p) && !std::path::Path::new(&p).is_file()) {
                let _ = shellreg::disable_picker();
            }
        }
    }
}

impl App {
    /// 設定裡的「其他程式開啟或儲存檔案時也用 Stillfeather」
    pub(super) fn set_picker_enabled(&mut self, on: bool) {
        // 先記下選擇：之後每次啟動都照這個選擇處理（沒選過時預設關閉）
        let _ = shellreg::set_picker_choice(on);
        let result = if on {
            current_exe_path().ok_or_else(|| tr!("找不到 Stillfeather 程式的位置。", "Can't find where the Stillfeather program is.").to_string()).and_then(|exe| {
                let dll = deploy_dll()?;
                shellreg::enable_picker(&dll, &exe)
            })
        } else {
            shellreg::disable_picker()
        };
        match result {
            Ok(()) if on => self.set_footer(tr!("已開啟：其他程式開啟、儲存檔案與網頁上傳時會跳出 Stillfeather（已經開著的程式可能要重新開啟）", "Turned on: other programs show Stillfeather when they open or save files, and for uploads on web pages (programs that are already open may need to be restarted)").into()),
            Ok(()) => self.set_footer(tr!("已改回 Windows 的開啟檔案與另存新檔視窗（已經開著的程式可能要重新開啟）", "Switched back to the Windows Open and Save As dialogs (programs that are already open may need to be restarted)").into()),
            Err(e) => self.show_error(&trf!("無法變更開啟檔案視窗：{e}", "Couldn't change the file dialog: {e}")),
        }
    }
}

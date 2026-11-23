//! 更新提醒：在背景檢查 GitHub 上的最新版，提醒使用者並在同意後下載、核對、啟動新的安裝程式。
//! 版本比較、update.json 的格式與檢查都在 `feather_core::update`（有單元測試）。
//!
//! - 開啟後約 10 秒自動檢查一次（最多每 20 小時一次），設定裡的「不要啟用更新提醒」會關掉自動檢查；
//!   設定的「檢查更新」不受影響，也不管之前略過的版本。
//! - 提醒用 Stillfeather 自己的對話方塊：「更新」「略過這個版本」「稍後」。Stillfeather 不在前景、
//!   或有其他對話方塊開著時先等，回到可以操作時才跳出來。
//! - 下載用 WinHTTP（遵循系統的 Proxy 設定），只接受 https；安裝程式的大小與 SHA-256 必須和 update.json 相符，
//!   目前的 Stillfeather 有數位簽章時，新的安裝程式也必須有有效的簽章，否則不執行。
//! - 安裝版以 `--update --dir <安裝位置>` 啟動新的安裝程式後關閉自己：安裝程式顯示進度、保留原本的捷徑與
//!   預設檔案總管設定，完成後重新開啟 Stillfeather。開發版（沒有安裝）只開啟安裝程式，讓使用者自己選。

use super::app::{self, App, WorkerMsg};
use super::crashlog::crumb;
use super::dialog::{self, Button, Icon};
use super::settings_dialog;
use super::sys::*;
use super::util::*;
use feather_core::format::format_size;
use feather_core::install;
use feather_core::paths;
use feather_core::update::{self, Manifest, Offer};
use std::ffi::c_void;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Cargo.toml 的 `repository`（GitHub 專案網址）；沒有填就不檢查更新
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// 啟動後多久第一次檢查，以及之後多久再看一次是不是該檢查了
pub const TIMER_CHECK: usize = 12;
const FIRST_CHECK_MS: u32 = 10_000;
const RECHECK_MS: u32 = 6 * 3600 * 1000;
/// 有新版本但現在不方便提醒（Stillfeather 不在前景、對話方塊開著）：過一會兒再試
pub const TIMER_PROMPT: usize = 13;
const PROMPT_RETRY_MS: u32 = 2_000;
const STATE_FILE: &str = "update-state.json";
const TEMP_FOLDER: &str = "Stillfeather-update";

/// 背景執行緒送回主視窗的結果
pub enum Event {
    Checked { manual: bool, result: Result<Manifest, String> },
    Progress(u32),
    Downloaded(Result<String, String>),
}

/// 主視窗保存的更新狀態
#[derive(Default)]
pub struct Ui {
    state: update::State,
    /// 檢查到、還沒提醒的新版本（第二個值：是不是使用者按「檢查更新」找到的）
    pending: Option<(Manifest, bool)>,
    /// 這次執行已經自動提醒過（按了「稍後」就不再打擾）
    prompted: bool,
    checking: bool,
    downloading: bool,
}

impl Ui {
    pub fn load() -> Ui {
        let state = state_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| update::State::from_json(&t)).unwrap_or_default();
        Ui { state, ..Default::default() }
    }

    fn save(&self) {
        if let Ok(p) = state_path() {
            if app::write_atomic(&p, &self.state.to_json()).is_err() {
                crumb("更新：無法儲存狀態");
            }
        }
    }
}

fn state_path() -> Result<String, String> {
    let settings = app::settings_path()?;
    let dir = paths::parent(&settings).ok_or("設定檔位置無效。")?;
    Ok(paths::join(&dir, STATE_FILE))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// update.json 的網址；可以用環境變數 STILLFEATHER_UPDATE_URL 換成別的 https 網址（測試用）
pub fn source_url() -> Option<String> {
    if let Ok(u) = std::env::var("STILLFEATHER_UPDATE_URL") {
        if update::split_https_url(u.trim()).is_some() {
            return Some(u.trim().to_string());
        }
    }
    update::manifest_url(REPOSITORY)
}

/// 在背景檢查（結果以 [`Event::Checked`] 送回主視窗）。沒有設定更新來源時回傳 false。
pub fn start_check(manual: bool) -> bool {
    let Some(url) = source_url() else { return false };
    std::thread::spawn(move || {
        let result = match fetch(&url, update::MAX_MANIFEST_BYTES, &mut |_, _| {}) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|_| "更新資訊不是 UTF-8 文字。".to_string()).and_then(|t| update::parse_manifest(&t)),
            Err((404, _)) => Err("GitHub 上還沒有含 update.json 的正式版。".into()),
            Err((_, e)) => Err(e),
        };
        app::post(WorkerMsg::Update(Event::Checked { manual, result }));
    });
    true
}

#[repr(C)]
struct GuiThreadInfo {
    cb_size: u32,
    flags: u32,
    active: HWND,
    focus: HWND,
    capture: HWND,
    menu_owner: HWND,
    move_size: HWND,
    caret: HWND,
    caret_rect: RECT,
}

#[repr(C)]
struct LastInputInfo {
    cb_size: u32,
    time: u32,
}

/// 使用者停手這麼久之後才跳出提醒：正在打字時跳出來，一個 Enter 就會按到「更新」
const IDLE_MS: u32 = 1_500;

const GUI_INMOVESIZE: u32 = 0x2;
const GUI_INMENUMODE: u32 = 0x4;
const GUI_POPUPMENUMODE: u32 = 0x10;

#[cfg_attr(windows, link(name = "user32", kind = "raw-dylib"))]
extern "system" {
    fn GetGUIThreadInfo(thread: u32, info: *mut GuiThreadInfo) -> BOOL;
    fn GetLastInputInfo(info: *mut LastInputInfo) -> BOOL;
}

/// 選單開著、正在拖曳（滑鼠被抓住）、移動視窗大小，或使用者剛剛還在打字、點按時，不要跳出提醒
pub fn ui_busy() -> bool {
    let mut last = LastInputInfo { cb_size: std::mem::size_of::<LastInputInfo>() as u32, time: 0 };
    if unsafe { GetLastInputInfo(&mut last) } != 0 && (unsafe { GetTickCount64() } as u32).wrapping_sub(last.time) < IDLE_MS {
        return true;
    }
    gui_busy()
}

/// 選單開著、滑鼠被抓住（拖曳、框選、拖分隔線）或正在移動、調整視窗大小（自動重新整理也等這些結束）
pub fn gui_busy() -> bool {
    let mut info = GuiThreadInfo { cb_size: std::mem::size_of::<GuiThreadInfo>() as u32, flags: 0, active: 0, focus: 0, capture: 0, menu_owner: 0, move_size: 0, caret: 0, caret_rect: RECT::default() };
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 {
        return false;
    }
    info.flags & (GUI_INMOVESIZE | GUI_INMENUMODE | GUI_POPUPMENUMODE) != 0 || info.capture != 0 || info.menu_owner != 0
}

/// 這個 exe 是不是用安裝程式裝的（旁邊有 Uninstall.exe）
fn installed_dir() -> Option<String> {
    let exe = current_exe_path()?;
    let dir = paths::parent(&exe)?;
    (paths::file_name(&exe).eq_ignore_ascii_case(install::EXE_NAME) && std::path::Path::new(&paths::join(&dir, install::UNINSTALLER_NAME)).is_file()).then_some(dir)
}

impl App {
    /// 啟動時排定第一次自動檢查
    pub(super) fn schedule_update_check(&self) {
        unsafe {
            SetTimer(self.hwnd, TIMER_CHECK, FIRST_CHECK_MS, std::ptr::null());
        }
    }

    pub(super) fn on_update_timer(&mut self) {
        unsafe {
            KillTimer(self.hwnd, TIMER_CHECK);
            SetTimer(self.hwnd, TIMER_CHECK, RECHECK_MS, std::ptr::null());
        }
        // 上次更新留下的安裝程式（安裝程式早就結束了）
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(TEMP_FOLDER));
        let u = &self.updates;
        if self.settings.update_reminders && !u.checking && !u.downloading && u.pending.is_none() && !u.prompted && u.state.due(now()) {
            self.updates.checking = start_check(false);
        }
    }

    pub(super) fn on_update_event(&mut self, event: Event) {
        match event {
            Event::Checked { manual, result } => {
                self.updates.checking = false;
                match result {
                    Ok(m) => {
                        self.updates.state.last_check = Some(now());
                        self.updates.save();
                        let offer = update::offer(VERSION, &m, self.updates.state.skipped.as_deref(), manual);
                        crumb(if offer == Offer::Newer { "更新：有新版本" } else { "更新：已是最新版本" });
                        if manual {
                            if offer == Offer::Newer {
                                // 設定還開著：「檢查更新」改成「立即更新」；已經關了就直接跳出提醒（見下面）
                                settings_dialog::update_found(&m.version);
                            } else if !settings_dialog::update_status("已是最新版本") {
                                // 設定已經關了：在狀態列告訴使用者
                                self.set_footer(format!("Stillfeather {VERSION} 已是最新版本"));
                            }
                        }
                        // 自動檢查：關掉提醒的話不跳出來（例如檢查途中剛好在設定裡關掉）
                        if offer == Offer::Newer && (manual || (self.settings.update_reminders && !self.updates.prompted)) {
                            self.updates.pending = Some((m, manual));
                            self.try_update_prompt();
                        }
                    }
                    Err(e) => {
                        crumb("更新：檢查失敗");
                        if manual && !settings_dialog::update_failed(&e) {
                            dialog::error(self.hwnd, "無法檢查更新", &e);
                        }
                    }
                }
            }
            Event::Progress(p) => self.set_footer(format!("正在下載 Stillfeather 更新… {p}%")),
            Event::Downloaded(result) => {
                self.updates.downloading = false;
                match result {
                    Ok(path) => self.run_update_setup(&path),
                    Err(e) => {
                        self.set_footer("更新沒有完成".into());
                        dialog::error(self.hwnd, "更新沒有完成", &e);
                    }
                }
            }
        }
    }

    /// 現在方便的話提醒有新版本，否則稍後再試
    pub(super) fn try_update_prompt(&mut self) {
        if self.updates.pending.is_none() {
            unsafe {
                KillTimer(self.hwnd, TIMER_PROMPT);
            }
            return;
        }
        if !self.ready_for_prompt() {
            unsafe {
                SetTimer(self.hwnd, TIMER_PROMPT, PROMPT_RETRY_MS, std::ptr::null());
            }
            return;
        }
        unsafe {
            KillTimer(self.hwnd, TIMER_PROMPT);
        }
        let Some((m, manual)) = self.updates.pending.take() else { return };
        self.updates.prompted = true;
        self.hide_preview();
        let installed = installed_dir().is_some();
        let mut foot = if installed { "更新時會先關閉 Stillfeather，安裝完成後自動重新開啟。".to_string() } else { "會開啟新版的安裝程式。".to_string() };
        if !manual {
            foot.push_str("不想收到提醒，可以在設定開啟「不要啟用更新提醒」。");
        }
        let mut spec = dialog::Spec::new(Icon::Info, "Stillfeather 有新版本")
            .rows(vec![("目前版本".into(), VERSION.into()), ("新版本".into(), m.version.clone()), ("下載大小".into(), format_size(Some(m.size)))])
            .footnote(&foot)
            .buttons(vec![Button::primary(1, "更新").key('U'), Button::normal(2, "略過這個版本"), Button::normal(0, "稍後")], 0)
            .width(480);
        let notes = notes_preview(&m.notes);
        if !notes.is_empty() {
            spec = spec.body(&notes);
        }
        match spec.show(self.hwnd) {
            1 => self.start_update_download(m),
            2 => {
                self.set_footer(format!("已略過 {} 版；有更新的版本時會再提醒", m.version));
                self.updates.state.skipped = Some(m.version);
                self.updates.save();
            }
            _ => {}
        }
    }

    fn start_update_download(&mut self, m: Manifest) {
        if self.updates.downloading {
            return;
        }
        self.updates.downloading = true;
        self.set_footer(format!("正在下載 Stillfeather {}…", m.version));
        std::thread::spawn(move || {
            let result = download(&m);
            app::post(WorkerMsg::Update(Event::Downloaded(result)));
        });
    }

    /// 啟動下載好的安裝程式。安裝版交給它更新後關閉自己（安裝程式完成後會重新開啟 Stillfeather）。
    fn run_update_setup(&mut self, setup: &str) {
        let dir = installed_dir();
        if dir.is_some() && self.op_busy {
            dialog::info(self.hwnd, "檔案作業還在進行", "更新會關閉 Stillfeather。請等檔案作業完成後，再到設定按「檢查更新」更新。");
            self.set_footer("檔案作業完成後再更新".into());
            return;
        }
        let mut cmd = std::process::Command::new(setup);
        if let Some(d) = &dir {
            cmd.args(update::setup_args(d));
        }
        if let Some(parent) = paths::parent(setup) {
            cmd.current_dir(parent);
        }
        match cmd.spawn() {
            Ok(_) if dir.is_some() => {
                crumb("更新：已啟動安裝程式，關閉主視窗");
                self.set_footer("正在更新 Stillfeather…".into());
                unsafe {
                    PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
                }
            }
            Ok(_) => self.set_footer("已開啟新版的安裝程式".into()),
            Err(e) => dialog::error(self.hwnd, "無法開啟安裝程式", &e.to_string()),
        }
    }

    /// 關閉設定之後：按了「立即更新」就直接下載；否則提醒在設定裡按「檢查更新」找到的新版本。
    /// 剛關掉更新提醒的話，自動檢查找到的就不提醒了。
    pub(super) fn after_settings_closed(&mut self) {
        if settings_dialog::take_update_now() {
            if let Some((m, _)) = self.updates.pending.take() {
                self.updates.prompted = true;
                unsafe {
                    KillTimer(self.hwnd, TIMER_PROMPT);
                }
                self.start_update_download(m);
                return;
            }
        }
        if !self.settings.update_reminders && matches!(self.updates.pending, Some((_, false))) {
            self.updates.pending = None;
        }
        self.try_update_prompt();
    }
}

/// 對話方塊裡的更新說明：最多 12 行、600 個字
fn notes_preview(notes: &str) -> String {
    const MAX_LINES: usize = 12;
    const MAX_CHARS: usize = 600;
    let mut out = String::new();
    let mut chars = 0;
    let mut cut = false;
    for (i, line) in notes.lines().enumerate() {
        if i >= MAX_LINES || chars >= MAX_CHARS {
            cut = true;
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        let room = MAX_CHARS - chars;
        let n = line.chars().count();
        if n > room {
            out.extend(line.chars().take(room));
            cut = true;
            break;
        }
        out.push_str(line);
        chars += n;
    }
    if cut {
        out.push('…');
    }
    out
}

/// 下載安裝程式、核對大小與 SHA-256（以及數位簽章），存到暫存資料夾。回傳安裝程式的位置。
fn download(m: &Manifest) -> Result<String, String> {
    let mut last = u32::MAX;
    let data = fetch(&m.url, m.size, &mut |got, total| {
        let total = total.unwrap_or(m.size).max(1);
        let pct = (got.saturating_mul(100) / total).min(100) as u32;
        if pct != last {
            last = pct;
            app::post(WorkerMsg::Update(Event::Progress(pct)));
        }
    })
    .map_err(|(status, e)| if status == 404 { "找不到這個版本的安裝程式（HTTP 404）。".to_string() } else { e })?;
    if data.len() as u64 != m.size {
        return Err(format!("下載的檔案大小不符（{} / {} 位元組），可能沒有下載完整。", data.len(), m.size));
    }
    if update::sha256(&data) != m.sha256 {
        return Err("下載的檔案和更新資訊不符（SHA-256 不同），可能下載不完整或被竄改，已停止更新。".into());
    }
    if !data.starts_with(b"MZ") {
        return Err("下載的檔案不是 Windows 程式，已停止更新。".into());
    }
    let dir = std::env::temp_dir().join(TEMP_FOLDER);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("無法建立暫存資料夾：{e}"))?;
    let version: String = m.version.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).collect();
    let path = dir.join(format!("StillfeatherSetup-{version}.exe"));
    std::fs::write(&path, &data).map_err(|e| format!("無法寫入安裝程式：{e}"))?;
    let path = path.to_string_lossy().to_string();
    // 目前這一版有簽章：新版也必須有有效的簽章
    if current_exe_path().is_some_and(|me| is_signed(&me)) && !is_signed(&path) {
        let _ = std::fs::remove_file(&path);
        return Err("新版的安裝程式沒有有效的數位簽章，已停止更新。".into());
    }
    Ok(path)
}

// ---------------------------------------------------------------- WinHTTP

#[allow(clippy::upper_case_acronyms)]
type HINTERNET = isize;

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const WINHTTP_QUERY_CONTENT_LENGTH: u32 = 5;
const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

#[cfg_attr(windows, link(name = "winhttp", kind = "raw-dylib"))]
extern "system" {
    fn WinHttpOpen(agent: *const u16, access: u32, proxy: *const u16, bypass: *const u16, flags: u32) -> HINTERNET;
    fn WinHttpSetTimeouts(h: HINTERNET, resolve: i32, connect: i32, send: i32, receive: i32) -> BOOL;
    fn WinHttpConnect(session: HINTERNET, server: *const u16, port: u16, reserved: u32) -> HINTERNET;
    fn WinHttpOpenRequest(conn: HINTERNET, verb: *const u16, object: *const u16, version: *const u16, referrer: *const u16, accept: *const *const u16, flags: u32) -> HINTERNET;
    fn WinHttpSendRequest(req: HINTERNET, headers: *const u16, headers_len: u32, optional: *const c_void, optional_len: u32, total_len: u32, context: usize) -> BOOL;
    fn WinHttpReceiveResponse(req: HINTERNET, reserved: *mut c_void) -> BOOL;
    fn WinHttpQueryHeaders(req: HINTERNET, level: u32, name: *const u16, buf: *mut c_void, len: *mut u32, index: *mut u32) -> BOOL;
    fn WinHttpQueryDataAvailable(req: HINTERNET, available: *mut u32) -> BOOL;
    fn WinHttpReadData(req: HINTERNET, buf: *mut c_void, to_read: u32, read: *mut u32) -> BOOL;
    fn WinHttpCloseHandle(h: HINTERNET) -> BOOL;
}

/// WinHTTP 控制代碼：離開時關閉
struct Net(HINTERNET);

impl Net {
    fn new(h: HINTERNET) -> Result<Net, (u32, String)> {
        if h == 0 {
            Err((0, net_error()))
        } else {
            Ok(Net(h))
        }
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn net_error() -> String {
    match unsafe { GetLastError() } {
        12002 => "連線逾時，請稍後再試。".into(),
        12007 => "找不到伺服器（可能沒有連上網路）。".into(),
        12029 | 12030 => "無法連線到伺服器（可能沒有連上網路，或被防火牆擋住）。".into(),
        12175 | 12045 | 12037 | 12038 => "無法建立安全連線（憑證有問題，或網路被攔截）。".into(),
        12152 => "伺服器的回應無效。".into(),
        c => format!("網路錯誤（代碼 {c}）。"),
    }
}

unsafe fn query_number(req: HINTERNET, level: u32) -> Option<u32> {
    let mut v = 0u32;
    let mut len = 4u32;
    (WinHttpQueryHeaders(req, level | WINHTTP_QUERY_FLAG_NUMBER, std::ptr::null(), (&mut v as *mut u32).cast(), &mut len, std::ptr::null_mut()) != 0).then_some(v)
}

/// 以 HTTPS GET 下載（跟隨轉址，例如 GitHub 的 releases/latest），內容超過 `limit` 位元組就停止。
/// 錯誤附上 HTTP 狀態碼（網路錯誤時是 0）。
fn fetch(url: &str, limit: u64, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Vec<u8>, (u32, String)> {
    let (host, port, path) = update::split_https_url(url).ok_or((0, "網址必須是 https。".to_string()))?;
    let too_big = || (0, "檔案比預期的大，已停止下載。".to_string());
    unsafe {
        let agent = wide(&format!("Stillfeather/{VERSION}"));
        let mut h = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0);
        if h == 0 {
            h = WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, std::ptr::null(), std::ptr::null(), 0);
        }
        let session = Net::new(h)?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
        let host_w = wide(&host);
        let conn = Net::new(WinHttpConnect(session.0, host_w.as_ptr(), port, 0))?;
        let (verb, object) = (wide("GET"), wide(&path));
        let req = Net::new(WinHttpOpenRequest(conn.0, verb.as_ptr(), object.as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), WINHTTP_FLAG_SECURE))?;
        let headers = wide("Cache-Control: no-cache\r\n");
        if WinHttpSendRequest(req.0, headers.as_ptr(), u32::MAX, std::ptr::null(), 0, 0, 0) == 0 || WinHttpReceiveResponse(req.0, std::ptr::null_mut()) == 0 {
            return Err((0, net_error()));
        }
        let status = query_number(req.0, WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err((status, format!("伺服器回應 HTTP {status}。")));
        }
        let total = query_number(req.0, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
        if total.is_some_and(|t| t > limit) {
            return Err(too_big());
        }
        let mut out: Vec<u8> = Vec::with_capacity(total.unwrap_or(0).min(limit) as usize);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut avail = 0u32;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err((0, net_error()));
            }
            if avail == 0 {
                break;
            }
            let mut read = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr().cast(), (avail as usize).min(buf.len()) as u32, &mut read) == 0 {
                return Err((0, net_error()));
            }
            if read == 0 {
                break;
            }
            if out.len() as u64 + read as u64 > limit {
                return Err(too_big());
            }
            out.extend_from_slice(&buf[..read as usize]);
            progress(out.len() as u64, total);
        }
        if total.is_some_and(|t| out.len() as u64 != t) {
            return Err((0, "下載沒有完成（連線中斷）。".into()));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- 數位簽章（Authenticode）

#[repr(C)]
struct WintrustFileInfo {
    cb_struct: u32,
    path: *const u16,
    file: HANDLE,
    known_subject: *const GUID,
}

#[repr(C)]
struct WintrustData {
    cb_struct: u32,
    policy_callback: *mut c_void,
    sip_client: *mut c_void,
    ui_choice: u32,
    revocation_checks: u32,
    union_choice: u32,
    file: *mut WintrustFileInfo,
    state_action: u32,
    state: HANDLE,
    url_reference: *mut u16,
    prov_flags: u32,
    ui_context: u32,
    signature_settings: *mut c_void,
}

const WINTRUST_ACTION_GENERIC_VERIFY_V2: GUID = GUID::from_u128(0x00AAC56B_CD44_11D0_8CC2_00C04FC295EE);
const WTD_UI_NONE: u32 = 2;
const WTD_REVOKE_NONE: u32 = 0;
const WTD_CHOICE_FILE: u32 = 1;
const WTD_STATEACTION_VERIFY: u32 = 1;
const WTD_STATEACTION_CLOSE: u32 = 2;
/// 不為了檢查憑證去下載撤銷清單（離線也能很快完成）
const WTD_CACHE_ONLY_URL_RETRIEVAL: u32 = 0x1000;

#[cfg_attr(windows, link(name = "wintrust", kind = "raw-dylib"))]
extern "system" {
    fn WinVerifyTrust(hwnd: HWND, action: *const GUID, data: *mut c_void) -> i32;
}

/// 檔案有沒有有效的 Authenticode 數位簽章（不顯示任何畫面）
pub fn is_signed(path: &str) -> bool {
    let p = wide(path);
    let mut file = WintrustFileInfo { cb_struct: std::mem::size_of::<WintrustFileInfo>() as u32, path: p.as_ptr(), file: 0, known_subject: std::ptr::null() };
    let mut data = WintrustData {
        cb_struct: std::mem::size_of::<WintrustData>() as u32,
        policy_callback: std::ptr::null_mut(),
        sip_client: std::ptr::null_mut(),
        ui_choice: WTD_UI_NONE,
        revocation_checks: WTD_REVOKE_NONE,
        union_choice: WTD_CHOICE_FILE,
        file: &mut file,
        state_action: WTD_STATEACTION_VERIFY,
        state: 0,
        url_reference: std::ptr::null_mut(),
        prov_flags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ui_context: 0,
        signature_settings: std::ptr::null_mut(),
    };
    unsafe {
        let rc = WinVerifyTrust(INVALID_HANDLE_VALUE, &WINTRUST_ACTION_GENERIC_VERIFY_V2, (&mut data as *mut WintrustData).cast());
        data.state_action = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(INVALID_HANDLE_VALUE, &WINTRUST_ACTION_GENERIC_VERIFY_V2, (&mut data as *mut WintrustData).cast());
        rc == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x64_layouts() {
        if std::mem::size_of::<usize>() != 8 {
            return;
        }
        // 與 x64 Windows SDK 的 WINTRUST_FILE_INFO、WINTRUST_DATA 相同
        assert_eq!(std::mem::size_of::<WintrustFileInfo>(), 32);
        assert_eq!(std::mem::size_of::<WintrustData>(), 88);
        assert_eq!(std::mem::offset_of!(WintrustData, file), 40);
        assert_eq!(std::mem::offset_of!(WintrustData, state), 56);
        assert_eq!(std::mem::offset_of!(WintrustData, prov_flags), 72);
        assert_eq!(std::mem::size_of::<GuiThreadInfo>(), 72);
        assert_eq!(std::mem::size_of::<LastInputInfo>(), 8);
    }

    #[test]
    fn notes_are_shortened() {
        assert_eq!(notes_preview("a\nb"), "a\nb");
        let many: Vec<String> = (0..20).map(|i| format!("第 {i} 項")).collect();
        let p = notes_preview(&many.join("\n"));
        assert_eq!(p.lines().count(), 12);
        assert!(p.ends_with("第 11 項…"));
        let long = notes_preview(&"字".repeat(800));
        assert_eq!(long.chars().count(), 601);
        assert!(long.ends_with('…'));
    }
}

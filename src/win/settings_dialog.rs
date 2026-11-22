//! 設定對話方塊：主題、重點色與背景色、側欄寬度、雙欄比例、緊湊清單、副檔名與快速預覽開關（含「按空白鍵預覽」），
//! 以及「設為預設檔案總管」與「其他程式開啟檔案時也用 Stillfeather」（寫在登錄，不在設定檔；「還原預設」不會改它們）。
//! 「還原預設」只填入預設值，需再按「儲存」才會套用，並保留常用位置。

use super::app;
use super::controls::{self, Kind, Surface};
use super::sys::*;
use super::theme;
use super::util::*;
use feather_core::settings::{color_to_hex, parse_color, Settings, ThemeMode, SIDEBAR_MAX, SIDEBAR_MIN, SPLIT_MAX, SPLIT_MIN};

use std::sync::atomic::{AtomicIsize, Ordering};

const CLASS: &str = "FeatherFiles.Settings";

const SEG_SYSTEM: u16 = 10;
const SEG_LIGHT: u16 = 11;
const SEG_DARK: u16 = 12;
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
const T_NO_UPDATES: u16 = 36;
const T_PICKER: u16 = 37;
const CHECK_UPDATES: u16 = 41;
const RESET: u16 = 40;
const NOTE_ID: u16 = 90;

/// 開著的設定視窗與它的更新狀態文字（背景檢查完成時由主視窗更新）
static DIALOG: AtomicIsize = AtomicIsize::new(0);
static UPDATE_NOTE: AtomicIsize = AtomicIsize::new(0);

struct State {
    base: Settings,
    result: Option<Settings>,
    /// 「設為預設檔案總管」按下儲存時的狀態
    default_fm: bool,
    /// 「其他程式開啟檔案時也用 Stillfeather」按下儲存時的狀態
    picker: bool,
    done: bool,
    segments: [HWND; 3],
    accent: HWND,
    background: HWND,
    sidebar: HWND,
    split: HWND,
    toggles: [HWND; 8],
    frames: Vec<(RECT, HWND)>,
    custom_colors: [COLORREF; 16],
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

/// 設定開著時，顯示檢查失敗的原因（對話方塊開在設定上方）。設定沒有開著時回傳 false。
pub fn update_failed(error: &str) -> bool {
    let dlg = DIALOG.load(Ordering::Relaxed);
    if dlg == 0 || !update_status("檢查失敗") {
        return false;
    }
    super::dialog::Spec::new(super::dialog::Icon::Warning, "無法檢查更新").body(error).show(dlg);
    true
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

fn label(parent: HWND, text: &str, r: RECT) -> HWND {
    let class = wide("Static");
    let t = wide(text);
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX | 0x0200, /* SS_CENTERIMAGE */
            r.left,
            r.top,
            r.width(),
            r.height(),
            parent,
            0,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SendMessageW(h, WM_SETFONT, app::theme().fonts.ui as WPARAM, 1);
        h
    }
}

fn note(parent: HWND, text: &str, r: RECT) -> HWND {
    let class = wide("Static");
    let t = wide(text);
    unsafe {
        let h = CreateWindowExW(
            0,
            class.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | SS_NOPREFIX,
            r.left,
            r.top,
            r.width(),
            r.height(),
            parent,
            NOTE_ID as HMENU,
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        );
        SendMessageW(h, WM_SETFONT, app::theme().fonts.ui as WPARAM, 1);
        h
    }
}

fn edit(parent: HWND, cid: u16, frame: RECT, st: &mut State) -> HWND {
    let t = app::theme();
    let h_edit = t.fonts.ui_height + t.s(2);
    let r = RECT {
        left: frame.left + t.s(8),
        top: frame.top + (frame.height() - h_edit) / 2,
        right: frame.right - t.s(8),
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
        SendMessageW(h, WM_SETFONT, t.fonts.ui as WPARAM, 1);
        st.frames.push((frame, h));
        h
    }
}

fn button(parent: HWND, cid: u16, kind: Kind, text: &str, r: RECT) -> HWND {
    let h = controls::create(parent, cid, kind, text, Surface::Card, true);
    move_window(h, r);
    h
}

fn fill(st: &State, s: &Settings) {
    for (i, h) in st.segments.iter().enumerate() {
        let mode = [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark][i];
        controls::set_checked(*h, s.theme_mode == mode);
    }
    set_window_text(st.accent, s.accent_color.as_deref().unwrap_or(""));
    set_window_text(st.background, s.background_color.as_deref().unwrap_or(""));
    set_window_text(st.sidebar, &s.sidebar_width.to_string());
    set_window_text(st.split, &s.split_percent.to_string());
    let values = [s.compact_rows, s.show_extensions, s.image_preview, s.pdf_preview, s.space_preview];
    for (h, v) in st.toggles.iter().zip(values) {
        controls::set_checked(*h, v);
    }
    controls::set_checked(st.toggles[6], !s.update_reminders);
}

fn collect(st: &State) -> Result<Settings, String> {
    let mut s = st.base.clone();
    s.theme_mode = if controls::is_checked(st.segments[1]) {
        ThemeMode::Light
    } else if controls::is_checked(st.segments[2]) {
        ThemeMode::Dark
    } else {
        ThemeMode::System
    };
    let color = |h: HWND| -> Result<Option<String>, String> {
        let t = window_text(h).trim().to_string();
        if t.is_empty() {
            return Ok(None);
        }
        let t = if t.starts_with('#') { t } else { format!("#{t}") };
        match parse_color(&t) {
            Some((r, g, b)) => Ok(Some(color_to_hex(r, g, b))),
            None => Err("自訂顏色必須是 #RRGGBB，或留白使用預設。".into()),
        }
    };
    s.accent_color = color(st.accent)?;
    s.background_color = color(st.background)?;
    s.sidebar_width = window_text(st.sidebar)
        .trim()
        .parse()
        .map_err(|_| format!("側欄寬度需介於 {SIDEBAR_MIN} 與 {SIDEBAR_MAX}。"))?;
    s.split_percent = window_text(st.split)
        .trim()
        .trim_end_matches('%')
        .parse()
        .map_err(|_| format!("左欄比例需介於 {SPLIT_MIN}% 與 {SPLIT_MAX}%。"))?;
    s.compact_rows = controls::is_checked(st.toggles[0]);
    s.show_extensions = controls::is_checked(st.toggles[1]);
    s.image_preview = controls::is_checked(st.toggles[2]);
    s.pdf_preview = controls::is_checked(st.toggles[3]);
    s.space_preview = controls::is_checked(st.toggles[4]);
    s.update_reminders = !controls::is_checked(st.toggles[6]);
    s.validate()?;
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

/// 顯示對話方塊（強制回應）；按「儲存」且驗證通過時回傳新設定，以及「設為預設檔案總管」
/// 與「其他程式開啟檔案時也用 Stillfeather」要不要開著。`default_fm`、`picker` 是目前的狀態。
pub fn show(owner: HWND, current: &Settings, default_fm: bool, picker: bool) -> Option<(Settings, bool, bool)> {
    let t = app::theme();
    let s = |v: i32| t.s(v);
    let (cw, ch) = (s(500), s(628));
    let class = wide(CLASS);
    let title = wide("設定");
    let dlg = unsafe {
        let mut or = RECT::default();
        GetWindowRect(owner, &mut or);
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
            or.left + (or.width() - cw) / 2,
            or.top + (or.height() - ch) / 2,
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
        SetWindowPos(dlg, 0, wr.left, wr.top, cw + fw, ch + fh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    theme::style_caption(dlg, &t.pal, t.pal.surface);

    let mut st = Box::new(State {
        base: current.clone(),
        result: None,
        default_fm,
        picker,
        done: false,
        segments: [0; 3],
        accent: 0,
        background: 0,
        sidebar: 0,
        split: 0,
        toggles: [0; 8],
        frames: Vec::new(),
        custom_colors: [rgb(255, 255, 255); 16],
    });
    let lx = s(24);
    let fx = s(140);
    let row_h = s(32);
    let mut y = s(20);
    label(dlg, "主題", RECT::new(lx, y, s(110), row_h));
    for (i, (cid, text)) in [(SEG_SYSTEM, "跟隨系統"), (SEG_LIGHT, "淺色"), (SEG_DARK, "深色")].iter().enumerate() {
        st.segments[i] = button(dlg, *cid, Kind::Segment, text, RECT::new(fx + i as i32 * s(104), y, s(98), row_h));
    }
    y += s(48);
    label(dlg, "重點色", RECT::new(lx, y, s(110), row_h));
    st.accent = edit(dlg, ACCENT_EDIT, RECT::new(fx, y, s(160), row_h), &mut st);
    button(dlg, ACCENT_PICK, Kind::Text, "選色…", RECT::new(fx + s(170), y, s(84), row_h));
    y += s(42);
    label(dlg, "背景色", RECT::new(lx, y, s(110), row_h));
    st.background = edit(dlg, BG_EDIT, RECT::new(fx, y, s(160), row_h), &mut st);
    button(dlg, BG_PICK, Kind::Text, "選色…", RECT::new(fx + s(170), y, s(84), row_h));
    y += s(38);
    note(dlg, "顏色留白使用主題預設；格式為 #RRGGBB。", RECT::new(fx, y, cw - fx - s(20), s(22)));
    y += s(34);
    label(dlg, "側欄寬度", RECT::new(lx, y, s(110), row_h));
    st.sidebar = edit(dlg, SIDEBAR_EDIT, RECT::new(fx, y, s(90), row_h), &mut st);
    note(dlg, &format!("{SIDEBAR_MIN}–{SIDEBAR_MAX} 像素"), RECT::new(fx + s(100), y + s(7), s(160), s(22)));
    y += s(42);
    label(dlg, "雙欄左欄比例", RECT::new(lx, y, s(116), row_h));
    st.split = edit(dlg, SPLIT_EDIT, RECT::new(fx, y, s(90), row_h), &mut st);
    note(dlg, &format!("{SPLIT_MIN}–{SPLIT_MAX} %"), RECT::new(fx + s(100), y + s(7), s(160), s(22)));
    y += s(52);
    let toggles = [(T_COMPACT, "緊湊清單"), (T_EXT, "顯示副檔名"), (T_IMAGE, "快速預覽圖片與影片"), (T_PDF, "快速預覽 PDF")];
    for (i, (cid, text)) in toggles.iter().enumerate() {
        let x = lx + (i as i32 % 2) * s(220);
        let yy = y + (i as i32 / 2) * s(38);
        st.toggles[i] = button(dlg, *cid, Kind::Toggle, text, RECT::new(x, yy, s(210), row_h));
    }
    let yy = y + 2 * s(38);
    st.toggles[4] = button(dlg, T_SPACE, Kind::Toggle, "按空白鍵預覽", RECT::new(lx, yy, s(210), row_h));
    note(dlg, "不自動懸停預覽；按住空白鍵時才預覽滑鼠下的項目，放開就關閉。", RECT::new(lx + s(4), yy + s(36), cw - lx * 2, s(22)));
    let yy = yy + s(70);
    st.toggles[5] = button(dlg, T_DEFAULT_FM, Kind::Toggle, "設為預設檔案總管（Win+E 與開啟資料夾）", RECT::new(lx, yy, cw - lx * 2, row_h));
    controls::set_checked(st.toggles[5], default_fm);
    let yy = yy + s(42);
    st.toggles[7] = button(dlg, T_PICKER, Kind::Toggle, "其他程式開啟檔案時也用 Stillfeather（上傳、選擇資料夾）", RECT::new(lx, yy, cw - lx * 2, row_h));
    controls::set_checked(st.toggles[7], picker);
    // 更新：關掉自動提醒的開關、目前版本與檢查結果、手動檢查
    let yy = yy + s(42);
    st.toggles[6] = button(dlg, T_NO_UPDATES, Kind::Toggle, "不要啟用更新提醒", RECT::new(lx, yy, s(200), row_h));
    let check_w = s(100);
    let check_x = cw - lx - check_w;
    button(dlg, CHECK_UPDATES, Kind::Text, "檢查更新", RECT::new(check_x, yy, check_w, row_h));
    let note_x = lx + s(212);
    let update_note = note(dlg, &format!("目前版本 {}", super::update::VERSION), RECT::new(note_x, yy + s(7), check_x - s(10) - note_x, s(22)));
    DIALOG.store(dlg, Ordering::Relaxed);
    UPDATE_NOTE.store(update_note, Ordering::Relaxed);
    let by = ch - s(56);
    button(dlg, RESET, Kind::Text, "還原預設", RECT::new(lx, by, s(100), s(34)));
    let cancel = button(dlg, IDCANCEL as u16, Kind::Text, "取消", RECT::new(cw - lx - s(92), by, s(92), s(34)));
    let save = button(dlg, IDOK as u16, Kind::Text, "儲存", RECT::new(cw - lx - s(92) * 2 - s(8), by, s(92), s(34)));
    controls::set_primary(save);
    let _ = cancel;
    fill(&st, current);

    let raw = Box::into_raw(st);
    unsafe {
        SetWindowLongPtrW(dlg, GWLP_USERDATA, raw as isize);
        EnableWindow(owner, 0);
        ShowWindow(dlg, SW_SHOW);
        let first = (*raw).segments[0];
        SetFocus(first);
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
            if IsDialogMessageW(dlg, &mut msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        DIALOG.store(0, Ordering::Relaxed);
        UPDATE_NOTE.store(0, Ordering::Relaxed);
        EnableWindow(owner, 1);
        SetForegroundWindow(owner);
        SetWindowLongPtrW(dlg, GWLP_USERDATA, 0);
        DestroyWindow(dlg);
        let st = Box::from_raw(raw);
        if let Some(code) = quit {
            PostQuitMessage(code);
        }
        st.result.map(|r| (r, st.default_fm, st.picker))
    }
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
                let focus = GetFocus();
                for (frame, edit) in &st.frames {
                    let border = if focus == *edit { t.pal.accent } else { t.pal.line };
                    theme::round_rect(buf.hdc, frame, t.s(8), Some(t.pal.surface), Some(border));
                }
                let line = RECT { left: 0, top: rect.bottom - t.s(72), right: rect.right, bottom: rect.bottom - t.s(72) + 1 };
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
                SEG_SYSTEM | SEG_LIGHT | SEG_DARK => {
                    for (i, h) in st.segments.iter().enumerate() {
                        controls::set_checked(*h, i as u16 == cid - SEG_SYSTEM);
                    }
                }
                ACCENT_PICK => {
                    let target = st.accent;
                    pick_color(hwnd, st, target, t.pal.accent);
                }
                BG_PICK => {
                    let target = st.background;
                    pick_color(hwnd, st, target, t.pal.surface);
                }
                CHECK_UPDATES => {
                    let text = if super::update::start_check(true) { "正在檢查…" } else { "這一版沒有設定更新來源" };
                    update_status(text);
                }
                RESET => {
                    let mut d = Settings::default();
                    d.favorites = st.base.favorites.clone();
                    fill(st, &d);
                }
                c if c == IDOK as u16 => match collect(st) {
                    Ok(s) => {
                        st.result = Some(s);
                        st.default_fm = controls::is_checked(st.toggles[5]);
                        st.picker = controls::is_checked(st.toggles[7]);
                        st.done = true;
                    }
                    Err(e) => {
                        super::dialog::Spec::new(super::dialog::Icon::Warning, "設定有誤").body(&e).show(hwnd);
                    }
                },
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

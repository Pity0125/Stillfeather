//! 安裝程式與解除安裝程式的畫面：Stillfeather 的米白／深灰配色、圓角按鈕與勾選方塊、草書字樣，
//! 依每個螢幕的 DPI 縮放，跟隨 Windows 的淺色／深色模式。只有路徑輸入框是系統的 Edit 控制項，其他都自己畫。
//!
//! 鍵盤：Tab／Shift+Tab 移動、空白鍵切換勾選或按下按鈕、Enter 按下主要按鈕、Esc 取消。
//! 安裝與解除安裝在背景執行緒進行，進度以訊息送回畫面。

use super::ffi::*;
use super::ops::{self, InstallOptions};
use super::sys::*;
use feather_core::install::{self, SetupArgs};
use feather_core::{paths, winres, wordmark};
use std::ffi::c_void;

const CLASS: &str = "Stillfeather.Setup";
const WM_APP_PROGRESS: u32 = WM_APP + 1;
const WM_APP_DONE: u32 = WM_APP + 2;

const PRIMARY: u16 = 1;
const CANCEL: u16 = 2;
const BROWSE: u16 = 3;
const PATH: u16 = 4;
const CHK_DEFAULT: u16 = 10;
const CHK_DESKTOP: u16 = 11;
const CHK_START: u16 = 12;
const CHK_LAUNCH: u16 = 13;
const CHK_PICKER: u16 = 14;
const EDIT_ID: usize = 100;

static ICO: &[u8] = include_bytes!("../../assets/FeatherFiles.ico");
const UI_FACE: &str = "Microsoft JhengHei UI";
const BASE_W: i32 = 560;
/// 更新完成後自動重新開啟 Stillfeather 的計時器
const DONE_TIMER: usize = 1;
const BASE_H: i32 = 466;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Install,
    Uninstall,
    /// Stillfeather 的更新提醒啟動：直接安裝到原本的位置，完成後重新開啟
    Update,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Start,
    Working,
    Done,
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

fn blend(fg: COLORREF, bg: COLORREF, a: f32) -> COLORREF {
    let ch = |c: COLORREF, s: u32| ((c >> s) & 0xFF) as f32;
    let mix = |s: u32| (ch(fg, s) * a + ch(bg, s) * (1.0 - a)).round() as u8;
    rgb(mix(0), mix(8), mix(16))
}

struct Palette {
    dark: bool,
    bg: COLORREF,
    surface: COLORREF,
    ink: COLORREF,
    muted: COLORREF,
    line: COLORREF,
    accent: COLORREF,
    accent_ink: COLORREF,
    hover: COLORREF,
}

/// 與 Stillfeather 相同的基本配色：淺色是米白，深色是接近黑的深灰
fn palette() -> Palette {
    let mut v = 1u32;
    let mut size = 4u32;
    let key = ops::w(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = ops::w("AppsUseLightTheme");
    let ok = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut v as *mut u32).cast(), &mut size) } == 0;
    let dark = ok && v == 0;
    let (bg, surface, ink, accent) = if dark {
        (rgb(18, 18, 18), rgb(30, 30, 30), rgb(236, 234, 231), rgb(232, 150, 118))
    } else {
        (rgb(248, 246, 242), rgb(255, 255, 255), rgb(46, 43, 41), rgb(210, 118, 86))
    };
    Palette {
        dark,
        bg,
        surface,
        ink,
        muted: blend(ink, surface, 0.58),
        line: blend(ink, bg, 0.14),
        accent,
        accent_ink: if dark { rgb(28, 22, 19) } else { rgb(255, 250, 244) },
        hover: blend(ink, surface, 0.06),
    }
}

struct Fonts {
    body: HFONT,
    small: HFONT,
    title: HFONT,
    button: HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Fonts {
        let f = |pt: i32, weight: i32| {
            let face = ops::w(UI_FACE);
            unsafe { CreateFontW(-(pt * dpi as i32 / 72), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, face.as_ptr()) }
        };
        Fonts { body: f(10, FW_NORMAL), small: f(9, FW_NORMAL), title: f(16, 700), button: f(10, FW_SEMIBOLD) }
    }

    fn destroy(&self) {
        for h in [self.body, self.small, self.title, self.button] {
            if h != 0 {
                unsafe {
                    DeleteObject(h);
                }
            }
        }
    }
}

enum Kind {
    Button { primary: bool },
    Check(bool),
    Path,
}

struct Widget {
    id: u16,
    kind: Kind,
    rect: RECT,
    text: String,
}

struct Wizard {
    hwnd: HWND,
    edit: HWND,
    flow: Flow,
    page: Page,
    dpi: u32,
    pal: Palette,
    fonts: Fonts,
    edit_brush: HBRUSH,
    args: SetupArgs,
    dir: String,
    default_fm: bool,
    /// 其他程式開啟或儲存檔案時也用 Stillfeather（預設不勾；已經選過的照原本的選擇）
    picker: bool,
    desktop: bool,
    start_menu: bool,
    launch: bool,
    was_running: bool,
    status: String,
    percent: u32,
    error: Option<String>,
    outcome: Option<Result<Vec<String>, String>>,
    widgets: Vec<Widget>,
    hot: u16,
    pressed: u16,
    focus: u16,
    show_focus: bool,
    edit_focused: bool,
    tracking: bool,
    icon: HICON,
    wordmark: (HICON, i32, i32),
    exit_code: i32,
}

// ---------------------------------------------------------------- 進入點

pub fn run_install(args: SetupArgs) -> i32 {
    let dir = args.dir.clone().unwrap_or_else(ops::default_install_dir);
    let default_fm = args.default_fm.unwrap_or(true);
    let desktop = args.desktop.unwrap_or(true);
    let start_menu = args.start_menu.unwrap_or(true);
    run(Flow::Install, args, dir, default_fm, desktop, start_menu)
}

/// 更新：沒有選項畫面，一開啟就裝到原本的位置
pub fn run_update(args: SetupArgs) -> i32 {
    let dir = args.dir.clone().unwrap_or_else(ops::default_install_dir);
    run(Flow::Update, args, dir, false, false, false)
}

pub fn run_uninstall(args: SetupArgs, dir: String) -> i32 {
    let code = run(Flow::Uninstall, args, dir, false, false, false);
    // 暫存資料夾裡的這一份：結束後刪掉自己
    ops::delete_self_if_temp();
    code
}

fn s_of(dpi: u32, v: i32) -> i32 {
    v * dpi as i32 / 96
}

fn run(flow: Flow, args: SetupArgs, dir: String, default_fm: bool, desktop: bool, start_menu: bool) -> i32 {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class = ops::w(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: wnd_proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: app_icon(32),
            hCursor: LoadCursorW(0, IDC_ARROW as *const u16),
            hbrBackground: 0,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: app_icon(16),
        };
        RegisterClassExW(&wc);
        let title = ops::w(match flow {
            Flow::Install => tr!("Stillfeather 安裝程式", "Stillfeather Setup"),
            Flow::Update => tr!("更新 Stillfeather", "Update Stillfeather"),
            Flow::Uninstall => tr!("解除安裝 Stillfeather", "Uninstall Stillfeather"),
        });
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT, 200, 200, 0, 0, hinst, std::ptr::null());
        if hwnd == 0 {
            return 1;
        }
        let dpi = GetDpiForWindow(hwnd).max(96);
        let pal = palette();
        if pal.dark {
            let on = 1u32;
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, (&on as *const u32).cast(), 4);
        }
        let fonts = Fonts::new(dpi);
        let edit_class = ops::w("Edit");
        let text = ops::w(&dir);
        let edit = CreateWindowExW(0, edit_class.as_ptr(), text.as_ptr(), WS_CHILD | WS_TABSTOP | ES_AUTOHSCROLL, 0, 0, 10, 10, hwnd, EDIT_ID as HMENU, hinst, std::ptr::null());
        SendMessageW(edit, WM_SETFONT, fonts.body as WPARAM, 1);
        let was_running = flow == Flow::Uninstall && ops::app_running();
        let picker = args.file_picker.unwrap_or_else(ops::picker_choice);
        let wiz = Box::new(Wizard {
            hwnd,
            edit,
            flow,
            page: Page::Start,
            dpi,
            edit_brush: CreateSolidBrush(pal.surface),
            pal,
            fonts,
            args,
            dir,
            default_fm,
            picker,
            desktop,
            start_menu,
            launch: true,
            was_running,
            status: String::new(),
            percent: 0,
            error: None,
            outcome: None,
            widgets: Vec::new(),
            hot: 0,
            pressed: 0,
            focus: PRIMARY,
            show_focus: false,
            edit_focused: false,
            tracking: false,
            icon: 0,
            wordmark: (0, 0, 0),
            exit_code: 2,
        });
        let raw = Box::into_raw(wiz);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, raw as isize);
        let wiz = &mut *raw;
        wiz.make_art();
        wiz.resize_window(None);
        wiz.layout();
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        if flow == Flow::Update {
            wiz.start();
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            if msg.message == WM_KEYDOWN && (msg.hwnd == hwnd || msg.hwnd == edit) {
                let shift = GetKeyState(VK_SHIFT) < 0;
                if (*raw).key(msg.wParam as u32, shift, msg.hwnd == edit) {
                    continue;
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let wiz = Box::from_raw(raw);
        let code = wiz.exit_code;
        if wiz.flow == Flow::Uninstall {
            if let Some(Ok(later)) = &wiz.outcome {
                let dirs = if later.is_empty() { Vec::new() } else { vec![wiz.dir.clone()] };
                ops::delete_after_exit(later, &dirs);
            }
        }
        wiz.fonts.destroy();
        DeleteObject(wiz.edit_brush);
        for icon in [wiz.icon, wiz.wordmark.0] {
            if icon != 0 {
                DestroyIcon(icon);
            }
        }
        code
    }
}

/// 從內嵌的 .ico 取一個尺寸的程式圖示
fn app_icon(size: i32) -> HICON {
    let Some(images) = winres::parse_ico(ICO) else { return 0 };
    let Some(i) = winres::pick_size(&images, size as u32) else { return 0 };
    let data = images[i].data;
    unsafe { CreateIconFromResourceEx(data.as_ptr(), data.len() as u32, 1, 0x0003_0000, size, size, LR_DEFAULTCOLOR) }
}

/// 由 0xAARRGGBB 像素（未預乘）建立圖示，畫出來時保留半透明邊緣
unsafe fn icon_from_pixels(px: &[u32], w: i32, h: i32) -> HICON {
    let hdc = GetDC(0);
    let bi = BITMAPINFO::top_down_32(w, h);
    let mut bits: *mut c_void = std::ptr::null_mut();
    let hbm = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &mut bits, 0, 0);
    ReleaseDC(0, hdc);
    if hbm == 0 || bits.is_null() {
        return 0;
    }
    std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len().min((w * h) as usize));
    let stride = (((w + 15) / 16) * 2) as usize;
    let mut mask = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if px.get(y * w as usize + x).is_none_or(|p| p >> 24 < 128) {
                mask[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    let hmask = CreateBitmap(w, h, 1, 1, mask.as_ptr().cast());
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: hmask, hbmColor: hbm };
    let icon = CreateIconIndirect(&info);
    DeleteObject(hbm);
    if hmask != 0 {
        DeleteObject(hmask);
    }
    icon
}

fn wide_text(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Wizard {
    fn s(&self, v: i32) -> i32 {
        s_of(self.dpi, v)
    }

    fn client(&self) -> RECT {
        let mut r = RECT::default();
        unsafe {
            GetClientRect(self.hwnd, &mut r);
        }
        r
    }

    /// 程式圖示與草書字樣（依 DPI）
    fn make_art(&mut self) {
        for icon in [self.icon, self.wordmark.0] {
            if icon != 0 {
                unsafe {
                    DestroyIcon(icon);
                }
            }
        }
        self.icon = app_icon(self.s(44));
        let min_line = 0.9 * (self.dpi as f32 / 96.0).min(2.0);
        let m = wordmark::render(self.s(230) as f32, self.s(34) as f32, min_line);
        let ink = self.pal.ink;
        let (r, g, b) = (ink & 0xFF, (ink >> 8) & 0xFF, (ink >> 16) & 0xFF);
        let color = (r << 16) | (g << 8) | b;
        let px: Vec<u32> = m.alpha.iter().map(|&a| ((a as u32) << 24) | color).collect();
        let icon = unsafe { icon_from_pixels(&px, m.width as i32, m.height as i32) };
        self.wordmark = (icon, m.width as i32, m.height as i32);
    }

    /// 視窗大小依 DPI；`suggested` 是 WM_DPICHANGED 建議的位置
    fn resize_window(&self, suggested: Option<&RECT>) {
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
        let mut r = RECT { left: 0, top: 0, right: self.s(BASE_W), bottom: self.s(BASE_H) };
        unsafe {
            AdjustWindowRectExForDpi(&mut r, style, 0, 0, self.dpi);
        }
        let (w, h) = (r.width(), r.height());
        let (x, y) = match suggested {
            Some(s) => (s.left, s.top),
            None => unsafe {
                let mut pt = POINT::default();
                GetCursorPos(&mut pt);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST), &mut mi);
                let wa = mi.rcWork;
                (wa.left + (wa.width() - w) / 2, wa.top + (wa.height() - h) / 2)
            },
        };
        unsafe {
            SetWindowPos(self.hwnd, 0, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    fn edit_text(&self) -> String {
        unsafe {
            let n = GetWindowTextLengthW(self.edit).max(0) as usize;
            let mut buf = vec![0u16; n + 1];
            let got = GetWindowTextW(self.edit, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            String::from_utf16_lossy(&buf[..got])
        }
    }

    fn set_edit_text(&self, text: &str) {
        let t = ops::w(text);
        unsafe {
            SetWindowTextW(self.edit, t.as_ptr());
        }
    }

    // ------------------------------------------------------------ 版面

    fn layout(&mut self) {
        let c = self.client();
        let (cw, ch) = (c.width(), c.height());
        let pad = self.s(32);
        let bw = self.s(108);
        let bh = self.s(36);
        let by = ch - self.s(60) + (self.s(60) - bh) / 2;
        let primary_rect = RECT::new(cw - pad - bw, by, bw, bh);
        let cancel_rect = RECT::new(primary_rect.left - self.s(10) - bw, by, bw, bh);
        let mut w = Vec::new();
        let button = |id: u16, rect: RECT, text: &str, primary: bool| Widget { id, kind: Kind::Button { primary }, rect, text: text.into() };
        let check = |id: u16, y: i32, text: &str, on: bool| Widget { id, kind: Kind::Check(on), rect: RECT::new(pad, y, cw - pad * 2, self.s(30)), text: text.into() };
        let mut edit_visible = false;
        match (self.flow, self.page) {
            (Flow::Install, Page::Start) => {
                let box_rect = RECT::new(pad, self.s(156), cw - pad * 2 - self.s(104) - self.s(10), self.s(38));
                w.push(Widget { id: PATH, kind: Kind::Path, rect: box_rect, text: String::new() });
                w.push(button(BROWSE, RECT::new(box_rect.right + self.s(10), box_rect.top, self.s(104), box_rect.height()), tr!("瀏覽…", "Browse…"), false));
                w.push(check(CHK_DEFAULT, self.s(232), tr!("設為預設檔案總管（Win+E 與開啟資料夾都改用 Stillfeather）", "Set as default file manager (Win+E and opening folders)"), self.default_fm));
                w.push(check(CHK_PICKER, self.s(268), tr!("其他程式開啟或儲存檔案時也用 Stillfeather（上傳、另存新檔）", "Use Stillfeather for other programs' Open and Save dialogs"), self.picker));
                w.push(check(CHK_DESKTOP, self.s(304), tr!("建立桌面捷徑", "Create a desktop shortcut"), self.desktop));
                w.push(check(CHK_START, self.s(340), tr!("加入開始功能表", "Add to the Start menu"), self.start_menu));
                w.push(button(CANCEL, cancel_rect, tr!("取消", "Cancel"), false));
                w.push(button(PRIMARY, primary_rect, tr!("安裝", "Install"), true));
                // 輸入框放在圓角框裡、垂直置中
                let eh = self.s(22);
                unsafe {
                    SetWindowPos(self.edit, 0, box_rect.left + self.s(12), box_rect.top + (box_rect.height() - eh) / 2, box_rect.width() - self.s(24), eh, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                edit_visible = true;
            }
            (Flow::Uninstall, Page::Start) => {
                w.push(button(CANCEL, cancel_rect, tr!("取消", "Cancel"), false));
                w.push(button(PRIMARY, primary_rect, tr!("解除安裝", "Uninstall"), true));
            }
            (Flow::Update, Page::Start) => {}
            (_, Page::Working) => {}
            (flow, Page::Done) => {
                let ok = matches!(self.outcome, Some(Ok(_)));
                if flow == Flow::Install && ok {
                    w.push(check(CHK_LAUNCH, self.s(354), tr!("立即開啟 Stillfeather", "Open Stillfeather now"), self.launch));
                }
                w.push(button(PRIMARY, primary_rect, if ok { tr!("完成", "Done") } else { tr!("關閉", "Close") }, true));
            }
        }
        unsafe {
            ShowWindow(self.edit, if edit_visible { SW_SHOW } else { SW_HIDE });
        }
        self.widgets = w;
        if !self.widgets.iter().any(|x| x.id == self.focus) {
            self.focus = PRIMARY;
        }
        self.invalidate();
    }

    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    fn hit(&self, x: i32, y: i32) -> u16 {
        self.widgets.iter().find(|w| w.rect.contains(x, y)).map(|w| w.id).unwrap_or(0)
    }

    // ------------------------------------------------------------ 繪製

    fn paint(&self, dc: HDC) {
        let c = self.client();
        let p = &self.pal;
        let pad = self.s(32);
        fill(dc, &c, p.bg);
        // 標頭：白色羽毛圖示＋草書字樣
        let (icon_x, icon_y, icon_s) = (pad, self.s(28), self.s(44));
        unsafe {
            if self.icon != 0 {
                DrawIconEx(dc, icon_x, icon_y, self.icon, icon_s, icon_s, 0, 0, DI_NORMAL);
            }
            let (wm, ww, wh) = self.wordmark;
            if wm != 0 {
                DrawIconEx(dc, icon_x + icon_s + self.s(14), icon_y + (icon_s - wh) / 2, wm, ww, wh, 0, 0, DI_NORMAL);
            }
        }
        let title_rect = RECT::new(pad, self.s(96), c.width() - pad * 2, self.s(34));
        let body_rect = |top: i32, bottom: i32| RECT { left: pad, top, right: c.width() - pad, bottom };
        let footer_top = c.height() - self.s(60);
        let ok = matches!(self.outcome, Some(Ok(_)));
        let (title, body): (String, String) = match (self.flow, self.page) {
            (Flow::Install, Page::Start) => (tr!("選擇安裝位置", "Choose where to install").into(), String::new()),
            (Flow::Install, Page::Working) => (tr!("正在安裝…", "Installing…").into(), String::new()),
            (Flow::Install, Page::Done) if ok => {
                let mut b = trf!("Stillfeather 已安裝到\n{}", "Stillfeather was installed to\n{}", self.dir);
                if self.default_fm {
                    b.push_str(tr!("\n\nWin+E 與開啟資料夾都會使用 Stillfeather。要改回 Windows 檔案總管，可以在 Stillfeather 的設定取消「設為預設檔案總管」，或解除安裝。", "\n\nWin+E and opening folders use Stillfeather. To switch back to File Explorer, turn off “Set as default file manager” in Stillfeather's Settings, or uninstall it."));
                }
                if self.picker {
                    b.push_str(tr!("\n\n其他程式開啟檔案、網頁上傳時也會跳出 Stillfeather（已經開著的程式要重新開啟）；可以在設定取消。", "\n\nOther programs also show Stillfeather when they open or save files, and for uploads on web pages (programs that are already open need to be restarted); you can turn this off in Settings."));
                }
                (tr!("安裝完成", "Installation complete").into(), b)
            }
            (Flow::Install, Page::Done) => (tr!("安裝沒有完成", "Installation didn't finish").into(), self.failure_text()),
            (Flow::Update, Page::Start | Page::Working) => (tr!("正在更新 Stillfeather…", "Updating Stillfeather…").into(), String::new()),
            (Flow::Update, Page::Done) if ok => (trf!("已更新到 {}", "Updated to {}", crate::VERSION), tr!("Stillfeather 馬上會重新開啟。", "Stillfeather will reopen in a moment.").into()),
            (Flow::Update, Page::Done) => (tr!("更新沒有完成", "The update didn't finish").into(), trf!("{}\n\n按「關閉」會重新開啟 Stillfeather；可以稍後在設定按「檢查更新」再試一次。", "{}\n\nClick “Close” to reopen Stillfeather; you can try again later with “Check for updates” in Settings.", self.failure_text())),
            (Flow::Uninstall, Page::Start) => {
                let mut b = trf!(
                    "解除安裝會：\n•  把預設的檔案總管改回 Windows 檔案總管，開啟檔案視窗改回 Windows 原本的\n•  刪除程式檔案與捷徑（{}）\n•  刪除設定、資料夾檢視記錄與記錄檔（%LOCALAPPDATA%\\{}）", "Uninstalling will:\n•  Switch the default file manager back to File Explorer, and the file dialogs back to the Windows ones\n•  Delete the program files and shortcuts ({})\n•  Delete settings, folder view records and logs (%LOCALAPPDATA%\\{})",
                    self.dir,
                    install::DATA_DIR_NAME
                );
                if self.was_running {
                    b.push_str(tr!("\n\nStillfeather 正在執行，會先將它關閉。", "\n\nStillfeather is running and will be closed first."));
                }
                (tr!("解除安裝 Stillfeather", "Uninstall Stillfeather").into(), b)
            }
            (Flow::Uninstall, Page::Working) => (tr!("正在解除安裝…", "Uninstalling…").into(), String::new()),
            (Flow::Uninstall, Page::Done) if ok => (tr!("已解除安裝 Stillfeather", "Stillfeather was uninstalled").into(), tr!("預設的檔案總管已改回 Windows 檔案總管，程式檔案、捷徑、設定與記錄都已刪除。", "The default file manager is back to File Explorer, and the program files, shortcuts, settings and logs were deleted.").into()),
            (Flow::Uninstall, Page::Done) => (tr!("解除安裝沒有完成", "Uninstall didn't finish").into(), self.failure_text()),
        };
        text(dc, &title, &title_rect, self.fonts.title, p.ink, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        if !body.is_empty() {
            // 下方有「立即開啟」勾選時留出位置
            let bottom = if self.widgets.iter().any(|w| w.id == CHK_LAUNCH) { self.s(346) } else { footer_top - self.s(12) };
            text(dc, &body, &body_rect(self.s(140), bottom), self.fonts.body, p.ink, DT_LEFT | DT_WORDBREAK);
        }
        match self.page {
            Page::Start if self.flow == Flow::Install => {
                text(dc, tr!("安裝位置", "Install location"), &body_rect(self.s(132), self.s(152)), self.fonts.small, p.muted, DT_LEFT | DT_SINGLELINE | DT_VCENTER);
                let note = tr!("預設裝在你的使用者資料夾，不需要系統管理員權限；選 Program Files 等位置時會請你允許。", "Installs to your user folder by default, so no administrator permission is needed; you'll be asked to allow it if you pick a location such as Program Files.");
                text(dc, note, &body_rect(self.s(198), self.s(222)), self.fonts.small, p.muted, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
            }
            Page::Working => {
                text(dc, &self.status, &body_rect(self.s(146), self.s(172)), self.fonts.body, p.muted, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
                let track = RECT::new(pad, self.s(186), c.width() - pad * 2, self.s(6));
                round_rect(dc, &track, track.height(), Some(p.line), None);
                let w = (track.width() as f32 * self.percent.min(100) as f32 / 100.0).round() as i32;
                if w > 0 {
                    let bar = RECT::new(track.left, track.top, w.max(track.height()), track.height());
                    round_rect(dc, &bar, bar.height(), Some(p.accent), None);
                }
            }
            _ => {}
        }
        if let Some(e) = &self.error {
            text(dc, e, &body_rect(self.s(336), footer_top - self.s(2)), self.fonts.small, p.accent, DT_LEFT | DT_WORDBREAK);
        }
        // 底部分隔線
        fill(dc, &RECT::new(0, footer_top, c.width(), 1), p.line);
        for wdg in &self.widgets {
            self.paint_widget(dc, wdg);
        }
    }

    fn failure_text(&self) -> String {
        match &self.outcome {
            Some(Err(e)) => e.clone(),
            _ => String::new(),
        }
    }

    fn paint_widget(&self, dc: HDC, wdg: &Widget) {
        let p = &self.pal;
        let hot = self.hot == wdg.id;
        let focused = self.show_focus && self.focus == wdg.id;
        let r = wdg.rect;
        match wdg.kind {
            Kind::Button { primary } => {
                let pressed = self.pressed == wdg.id && hot;
                let (fill_c, border, ink) = if primary {
                    let base = if pressed { blend(rgb(0, 0, 0), p.accent, 0.16) } else if hot { blend(rgb(0, 0, 0), p.accent, 0.08) } else { p.accent };
                    (base, None, p.accent_ink)
                } else {
                    (if pressed { blend(p.ink, p.surface, 0.12) } else if hot { p.hover } else { p.surface }, Some(p.line), p.ink)
                };
                round_rect(dc, &r, r.height(), Some(fill_c), border);
                text(dc, &wdg.text, &r, self.fonts.button, ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                if focused {
                    round_rect(dc, &r.inflate(self.s(3), self.s(3)), r.height() + self.s(6), None, Some(p.accent));
                }
            }
            Kind::Check(on) => {
                let bs = self.s(20);
                let b = RECT::new(r.left, r.top + (r.height() - bs) / 2, bs, bs);
                if on {
                    round_rect(dc, &b, self.s(6), Some(p.accent), None);
                    check_mark(dc, &b, p.accent_ink, self.s(2).max(2));
                } else {
                    round_rect(dc, &b, self.s(6), Some(p.surface), Some(if hot { p.muted } else { p.line }));
                }
                let tr = RECT { left: b.right + self.s(10), top: r.top, right: r.right, bottom: r.bottom };
                text(dc, &wdg.text, &tr, self.fonts.body, p.ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                if focused {
                    let width = text_width(dc, &wdg.text, self.fonts.body);
                    let ring = RECT { left: r.left - self.s(4), top: r.top, right: (tr.left + width + self.s(6)).min(r.right + self.s(4)), bottom: r.bottom };
                    round_rect(dc, &ring, self.s(8), None, Some(p.accent));
                }
            }
            Kind::Path => {
                let border = if self.edit_focused { p.accent } else if hot { p.muted } else { p.line };
                round_rect(dc, &r, self.s(10), Some(p.surface), Some(border));
            }
        }
    }

    // ------------------------------------------------------------ 操作

    fn focusables(&self) -> Vec<u16> {
        self.widgets.iter().map(|w| w.id).collect()
    }

    fn set_focus(&mut self, id: u16) {
        self.focus = id;
        unsafe {
            SetFocus(if id == PATH { self.edit } else { self.hwnd });
        }
        self.invalidate();
    }

    /// 鍵盤（訊息迴圈先交給這裡）。回傳 true 表示已處理。
    fn key(&mut self, vk: u32, shift: bool, in_edit: bool) -> bool {
        match vk {
            VK_TAB => {
                let list = self.focusables();
                if list.is_empty() {
                    return true;
                }
                let current = if in_edit { PATH } else { self.focus };
                let i = list.iter().position(|id| *id == current).unwrap_or(0);
                let next = if shift { (i + list.len() - 1) % list.len() } else { (i + 1) % list.len() };
                self.show_focus = true;
                self.set_focus(list[next]);
                true
            }
            VK_RETURN => {
                let target = if !in_edit && matches!(self.widgets.iter().find(|w| w.id == self.focus).map(|w| &w.kind), Some(Kind::Button { .. })) { self.focus } else { PRIMARY };
                self.activate(target);
                true
            }
            VK_ESCAPE => {
                if self.page != Page::Working {
                    if self.widgets.iter().any(|w| w.id == CANCEL) {
                        self.activate(CANCEL);
                    } else {
                        self.activate(PRIMARY);
                    }
                }
                true
            }
            VK_SPACE if !in_edit => {
                self.show_focus = true;
                self.activate(self.focus);
                true
            }
            _ => false,
        }
    }

    fn activate(&mut self, id: u16) {
        match id {
            CHK_DEFAULT => self.default_fm = !self.default_fm,
            CHK_PICKER => self.picker = !self.picker,
            CHK_DESKTOP => self.desktop = !self.desktop,
            CHK_START => self.start_menu = !self.start_menu,
            CHK_LAUNCH => self.launch = !self.launch,
            BROWSE => self.browse(),
            PATH => self.set_focus(PATH),
            CANCEL => unsafe {
                DestroyWindow(self.hwnd);
                return;
            },
            PRIMARY => match self.page {
                Page::Start => self.start(),
                Page::Done => {
                    // 更新：不論成功與否都重新開啟 Stillfeather（失敗時開的是原本那一版）
                    let ok = matches!(self.outcome, Some(Ok(_)));
                    if (self.flow == Flow::Install && ok && self.launch) || self.flow == Flow::Update {
                        ops::launch_app(&self.dir);
                    }
                    unsafe {
                        DestroyWindow(self.hwnd);
                    }
                    return;
                }
                Page::Working => {}
            },
            _ => return,
        }
        // 勾選狀態寫回元件
        for w in &mut self.widgets {
            w.kind = match (&w.kind, w.id) {
                (Kind::Check(_), CHK_DEFAULT) => Kind::Check(self.default_fm),
                (Kind::Check(_), CHK_PICKER) => Kind::Check(self.picker),
                (Kind::Check(_), CHK_DESKTOP) => Kind::Check(self.desktop),
                (Kind::Check(_), CHK_START) => Kind::Check(self.start_menu),
                (Kind::Check(_), CHK_LAUNCH) => Kind::Check(self.launch),
                (Kind::Button { primary }, _) => Kind::Button { primary: *primary },
                (Kind::Check(on), _) => Kind::Check(*on),
                (Kind::Path, _) => Kind::Path,
            };
        }
        self.invalidate();
    }

    /// 選擇安裝位置（系統的選擇資料夾視窗）
    fn browse(&mut self) {
        unsafe {
            let mut dlg: *mut c_void = std::ptr::null_mut();
            if CoCreateInstance(&CLSID_FILE_OPEN_DIALOG, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IFILE_OPEN_DIALOG, &mut dlg) < 0 || dlg.is_null() {
                return;
            }
            let v = vtbl::<IFileOpenDialogVtbl>(dlg);
            let mut opts = 0u32;
            (v.GetOptions)(dlg, &mut opts);
            (v.SetOptions)(dlg, opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
            let (title, ok) = (ops::w(tr!("選擇安裝位置", "Choose an install location")), ops::w(tr!("選擇資料夾", "Select folder")));
            (v.SetTitle)(dlg, title.as_ptr());
            (v.SetOkButtonLabel)(dlg, ok.as_ptr());
            // 從目前輸入的位置（或它存在的上層）開始
            let mut start = Some(self.edit_text());
            while let Some(p) = start.take() {
                if std::path::Path::new(&p).is_dir() {
                    let wp = ops::w(&p);
                    let mut item: *mut c_void = std::ptr::null_mut();
                    if SHCreateItemFromParsingName(wp.as_ptr(), std::ptr::null_mut(), &IID_ISHELL_ITEM, &mut item) >= 0 && !item.is_null() {
                        (v.SetFolder)(dlg, item);
                        release(item);
                    }
                    break;
                }
                start = paths::parent(&p);
            }
            if (v.Show)(dlg, self.hwnd) >= 0 {
                let mut item: *mut c_void = std::ptr::null_mut();
                if (v.GetResult)(dlg, &mut item) >= 0 && !item.is_null() {
                    let mut p: *mut u16 = std::ptr::null_mut();
                    if (vtbl::<IShellItemVtbl>(item).GetDisplayName)(item, SIGDN_FILESYSPATH, &mut p) >= 0 && !p.is_null() {
                        let mut n = 0;
                        while *p.add(n) != 0 {
                            n += 1;
                        }
                        let picked = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                        CoTaskMemFree(p.cast());
                        self.set_edit_text(&install::with_app_folder(&picked));
                        self.error = None;
                    }
                    release(item);
                }
            }
            release(dlg);
        }
        self.invalidate();
    }

    /// 按下「安裝」或「解除安裝」：檢查後在背景執行緒開始
    fn start(&mut self) {
        let hwnd = self.hwnd;
        let elevated = self.args.elevated;
        let post_progress = move |percent: u32, status: &str| unsafe {
            let b = Box::into_raw(Box::new(status.to_string()));
            if PostMessageW(hwnd, WM_APP_PROGRESS, percent as WPARAM, b as LPARAM) == 0 {
                drop(Box::from_raw(b));
            }
        };
        let post_done = move |r: Result<Vec<String>, String>| unsafe {
            let b = Box::into_raw(Box::new(r));
            if PostMessageW(hwnd, WM_APP_DONE, 0, b as LPARAM) == 0 {
                drop(Box::from_raw(b));
            }
        };
        match self.flow {
            Flow::Install => {
                let prot = ops::protected_dirs();
                let prot: Vec<&str> = prot.iter().map(String::as_str).collect();
                let dir = match install::normalize_install_dir(&self.edit_text(), &prot) {
                    Ok(d) => d,
                    Err(e) => {
                        self.error = Some(e);
                        self.set_focus(PATH);
                        return;
                    }
                };
                self.dir = dir.clone();
                let opts = InstallOptions { dir, default_fm: Some(self.default_fm), picker: Some(self.picker), desktop: Some(self.desktop), start_menu: Some(self.start_menu) };
                std::thread::spawn(move || {
                    unsafe {
                        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
                    }
                    let mut progress = post_progress;
                    let r = ops::install_with_elevation(&opts, elevated, &mut progress).map(|_| Vec::new());
                    post_done(r);
                });
            }
            Flow::Update => {
                // 維持原本的預設檔案總管設定與捷徑
                let opts = InstallOptions { dir: self.dir.clone(), default_fm: None, picker: None, desktop: None, start_menu: None };
                std::thread::spawn(move || {
                    unsafe {
                        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
                    }
                    let mut progress = post_progress;
                    let r = ops::install_with_elevation(&opts, elevated, &mut progress).map(|_| Vec::new());
                    post_done(r);
                });
            }
            Flow::Uninstall => {
                let dir = self.dir.clone();
                std::thread::spawn(move || {
                    unsafe {
                        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED);
                    }
                    let mut progress = post_progress;
                    post_done(ops::uninstall_with_elevation(&dir, elevated, &mut progress));
                });
            }
        }
        self.error = None;
        self.page = Page::Working;
        self.status = tr!("準備中…", "Getting ready…").into();
        self.percent = 0;
        self.show_focus = false;
        unsafe {
            SetFocus(self.hwnd);
        }
        self.layout();
    }
}

// ---------------------------------------------------------------- 繪圖工具

fn fill(dc: HDC, r: &RECT, color: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(color);
        FillRect(dc, r, b);
        DeleteObject(b);
    }
}

fn round_rect(dc: HDC, r: &RECT, radius: i32, fill: Option<COLORREF>, border: Option<COLORREF>) {
    unsafe {
        let brush = fill.map(|c| CreateSolidBrush(c)).unwrap_or(0);
        let pen = border.map(|c| CreatePen(PS_SOLID, 1, c)).unwrap_or(0);
        let old_b = SelectObject(dc, if brush != 0 { brush } else { GetStockObject(NULL_BRUSH) });
        let old_p = SelectObject(dc, if pen != 0 { pen } else { GetStockObject(NULL_PEN) });
        // 沒有外框時多畫一個像素，讓填色與有外框時一樣大
        let extra = if pen == 0 { 1 } else { 0 };
        RoundRect(dc, r.left, r.top, r.right + extra, r.bottom + extra, radius, radius);
        SelectObject(dc, old_b);
        SelectObject(dc, old_p);
        if brush != 0 {
            DeleteObject(brush);
        }
        if pen != 0 {
            DeleteObject(pen);
        }
    }
}

fn check_mark(dc: HDC, b: &RECT, color: COLORREF, width: i32) {
    let (w, h) = (b.width() as f32, b.height() as f32);
    let pt = |fx: f32, fy: f32| (b.left + (w * fx).round() as i32, b.top + (h * fy).round() as i32);
    unsafe {
        let pen = CreatePen(PS_SOLID, width, color);
        let old = SelectObject(dc, pen);
        let (x0, y0) = pt(0.26, 0.52);
        let (x1, y1) = pt(0.43, 0.69);
        let (x2, y2) = pt(0.75, 0.34);
        MoveToEx(dc, x0, y0, std::ptr::null_mut());
        LineTo(dc, x1, y1);
        LineTo(dc, x2, y2);
        SelectObject(dc, old);
        DeleteObject(pen);
    }
}

fn text(dc: HDC, s: &str, r: &RECT, font: HFONT, color: COLORREF, flags: u32) {
    if s.is_empty() {
        return;
    }
    let w = wide_text(s);
    let mut rr = *r;
    unsafe {
        let old = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, color);
        DrawTextW(dc, w.as_ptr(), w.len() as i32, &mut rr, flags | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

fn text_width(dc: HDC, s: &str, font: HFONT) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let w = wide_text(s);
    let mut r = RECT::default();
    unsafe {
        let old = SelectObject(dc, font);
        DrawTextW(dc, w.as_ptr(), w.len() as i32, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
    r.width()
}

// ---------------------------------------------------------------- 視窗程序

unsafe fn wizard<'a>(hwnd: HWND) -> Option<&'a mut Wizard> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Wizard).as_mut()
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(wz) = wizard(hwnd) else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let c = wz.client();
            let mem = CreateCompatibleDC(hdc);
            let bmp = CreateCompatibleBitmap(hdc, c.width().max(1), c.height().max(1));
            let old = SelectObject(mem, bmp);
            wz.paint(mem);
            BitBlt(hdc, 0, 0, c.width(), c.height(), mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32);
            let id = wz.hit(x, y);
            if id != wz.hot {
                wz.hot = id;
                wz.invalidate();
            }
            if !wz.tracking {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                wz.tracking = true;
            }
            0
        }
        WM_MOUSELEAVE => {
            wz.tracking = false;
            if wz.hot != 0 {
                wz.hot = 0;
                wz.invalidate();
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32);
            let id = wz.hit(x, y);
            wz.show_focus = false;
            if id == PATH {
                wz.set_focus(PATH);
            } else if id != 0 {
                wz.pressed = id;
                wz.focus = id;
                SetCapture(hwnd);
                SetFocus(hwnd);
                wz.invalidate();
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = ((lparam & 0xFFFF) as i16 as i32, ((lparam >> 16) & 0xFFFF) as i16 as i32);
            let pressed = std::mem::replace(&mut wz.pressed, 0);
            ReleaseCapture();
            if pressed != 0 && wz.hit(x, y) == pressed {
                wz.activate(pressed);
            } else {
                wz.invalidate();
            }
            0
        }
        WM_COMMAND => {
            if (wparam & 0xFFFF) == EDIT_ID {
                match ((wparam >> 16) & 0xFFFF) as u32 {
                    EN_SETFOCUS => {
                        wz.edit_focused = true;
                        wz.focus = PATH;
                        wz.invalidate();
                    }
                    EN_KILLFOCUS => {
                        wz.edit_focused = false;
                        wz.invalidate();
                    }
                    EN_CHANGE if wz.error.is_some() => {
                        wz.error = None;
                        wz.invalidate();
                    }
                    _ => {}
                }
            }
            0
        }
        WM_CTLCOLOREDIT => {
            SetTextColor(wparam as HDC, wz.pal.ink);
            SetBkColor(wparam as HDC, wz.pal.surface);
            wz.edit_brush
        }
        WM_APP_PROGRESS => {
            let status = Box::from_raw(lparam as *mut String);
            wz.percent = wparam as u32;
            wz.status = *status;
            wz.invalidate();
            0
        }
        WM_APP_DONE => {
            let result = Box::from_raw(lparam as *mut Result<Vec<String>, String>);
            wz.exit_code = if result.is_ok() { 0 } else { 1 };
            let ok = result.is_ok();
            wz.outcome = Some(*result);
            wz.page = Page::Done;
            wz.layout();
            SetForegroundWindow(hwnd);
            // 更新成功：顯示一下「已更新」就自動重新開啟 Stillfeather
            if ok && wz.flow == Flow::Update {
                SetTimer(hwnd, DONE_TIMER, 1200, std::ptr::null());
            }
            0
        }
        WM_TIMER if wparam == DONE_TIMER => {
            KillTimer(hwnd, DONE_TIMER);
            wz.activate(PRIMARY);
            0
        }
        WM_DPICHANGED => {
            wz.dpi = ((wparam >> 16) & 0xFFFF) as u32;
            wz.fonts.destroy();
            wz.fonts = Fonts::new(wz.dpi);
            SendMessageW(wz.edit, WM_SETFONT, wz.fonts.body as WPARAM, 1);
            wz.make_art();
            let suggested = *(lparam as *const RECT);
            wz.resize_window(Some(&suggested));
            wz.layout();
            0
        }
        WM_SIZE => {
            wz.layout();
            0
        }
        WM_CLOSE => {
            // 安裝或解除安裝進行中不能關閉
            if wz.page != Page::Working {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

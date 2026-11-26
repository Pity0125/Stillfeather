//! 動畫的 Windows 部分：時鐘、系統「在 Windows 中顯示動畫」設定、逐格重畫、
//! 視窗淡入淡出，以及「擷取舊畫面 → 更新內容 → 舊畫面淡出」的交叉淡化。
//!
//! 逐格重畫用一次性的計時器：繪圖時發現還有動畫在進行，就呼叫 `request_frame`
//! 預約約 15 毫秒後再重畫一次；動畫結束時不再預約，計時器自然停止，閒置時不耗 CPU。

use super::sys::*;
use super::util::*;
use feather_core::anim::Tween;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use feather_core::anim::Ease;

/// 每格間隔（毫秒）。Windows 計時器的實際解析度約 15.6 毫秒，約每秒 60 格。
const FRAME_MS: u32 = 15;
const FRAME_TIMER: usize = 0xF7A0;
const FADE_TIMER: usize = 0xF7A1;

/// 懸停底色淡入淡出
pub const HOVER_MS: f64 = 120.0;
/// 按下
pub const PRESS_MS: f64 = 70.0;
/// 勾選、選取狀態切換
pub const TOGGLE_MS: f64 = 170.0;
/// 底線、分段選取膠囊的滑動
pub const SLIDE_MS: f64 = 240.0;
/// 清單與樹狀圖的列選取
pub const ROW_MS: f64 = 150.0;
/// 內容切換的交叉淡化
pub const CROSSFADE_MS: f64 = 180.0;
/// 深淺色切換
pub const THEME_MS: f64 = 300.0;

pub const OVERLAY_CLASS: &str = "FeatherFiles.Crossfade";

thread_local! {
    static FREQ: Cell<i64> = const { Cell::new(0) };
    static ENABLED: Cell<Option<bool>> = const { Cell::new(None) };
    static DIRTY: RefCell<HashMap<HWND, Option<Vec<RECT>>>> = RefCell::new(HashMap::new());
    static FADES: RefCell<HashMap<HWND, WindowFade>> = RefCell::new(HashMap::new());
    /// 仍在畫面上的交叉淡化覆蓋層與它們蓋住的範圍（owner 用戶區座標）
    static OVERLAYS: RefCell<Vec<(HWND, RECT)>> = const { RefCell::new(Vec::new()) };
}

/// 單調遞增的時間（毫秒）。
pub fn now() -> f64 {
    let freq = FREQ.with(|f| {
        if f.get() == 0 {
            let mut q = 0i64;
            unsafe {
                QueryPerformanceFrequency(&mut q);
            }
            f.set(q.max(1));
        }
        f.get()
    });
    let mut c = 0i64;
    unsafe {
        QueryPerformanceCounter(&mut c);
    }
    c as f64 * 1000.0 / freq as f64
}

/// 系統是否開啟「在 Windows 中顯示動畫」。關閉時所有動畫直接跳到結果。
pub fn enabled() -> bool {
    ENABLED.with(|e| {
        if let Some(v) = e.get() {
            return v;
        }
        let mut v: BOOL = 1;
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut v as *mut BOOL as *mut c_void, 0) };
        let on = ok == 0 || v != 0;
        e.set(Some(on));
        on
    })
}

/// 系統設定變更（WM_SETTINGCHANGE）後重新讀取。
pub fn refresh_settings() {
    ENABLED.with(|e| e.set(None));
}

/// 動畫時長；系統關閉動畫時為 0。
pub fn dur(ms: f64) -> f64 {
    if enabled() {
        ms
    } else {
        0.0
    }
}

/// 把補間移向新目標（系統關閉動畫時直接跳過去）。
pub fn to(t: &mut Tween, value: f32, ms: f64) {
    t.retarget(value, now(), dur(ms));
}

pub fn value(t: &Tween) -> f32 {
    t.value_at(now())
}

pub fn running(t: &Tween) -> bool {
    t.running_at(now())
}

/// 約 15 毫秒後重畫整個視窗一次。繪圖時若仍有動畫，再呼叫一次即可繼續播放。
pub fn request_frame(hwnd: HWND) {
    request(hwnd, None);
}

/// 約 15 毫秒後只重畫指定範圍（例如清單中正在淡入淡出的那幾列）。
pub fn request_rects(hwnd: HWND, rects: Vec<RECT>) {
    if rects.is_empty() {
        return;
    }
    request(hwnd, Some(rects));
}

fn request(hwnd: HWND, rects: Option<Vec<RECT>>) {
    DIRTY.with(|d| {
        let mut d = d.borrow_mut();
        if let Some(slot) = d.get_mut(&hwnd) {
            match (slot.as_mut(), rects) {
                (Some(existing), Some(new)) => existing.extend(new),
                (_, None) => *slot = None,
                (None, Some(_)) => {}
            }
        } else {
            d.insert(hwnd, rects);
        }
    });
    unsafe {
        SetTimer(hwnd, FRAME_TIMER, FRAME_MS, frame_proc as *const c_void);
    }
}

unsafe extern "system" fn frame_proc(hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    KillTimer(hwnd, id);
    let dirty = DIRTY.with(|d| d.borrow_mut().remove(&hwnd));
    if IsWindow(hwnd) == 0 {
        return;
    }
    match dirty {
        Some(Some(rects)) => {
            for r in rects {
                InvalidateRect(hwnd, &r, 0);
            }
        }
        _ => {
            InvalidateRect(hwnd, std::ptr::null(), 0);
        }
    }
}

// ---------------------------------------------------------------- 視窗淡入淡出

struct WindowFade {
    alpha: Tween,
    destroy: bool,
}

unsafe fn ensure_layered(hwnd: HWND, alpha: u8) {
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_LAYERED == 0 {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_LAYERED) as isize);
    }
    SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
}

/// 分層視窗（例如懸停預覽）從透明淡入。請在顯示視窗之前呼叫。
pub fn fade_in(hwnd: HWND, ms: f64) {
    unsafe {
        if !enabled() || ms <= 0.0 {
            ensure_layered(hwnd, 255);
            return;
        }
        ensure_layered(hwnd, 0);
        let mut alpha = Tween::new(0.0);
        alpha.retarget(1.0, now(), ms);
        FADES.with(|f| f.borrow_mut().insert(hwnd, WindowFade { alpha, destroy: false }));
        SetTimer(hwnd, FADE_TIMER, FRAME_MS, fade_proc as *const c_void);
    }
}

/// 淡出後銷毀視窗；淡出期間滑鼠會穿透它。
pub fn fade_out_and_destroy(hwnd: HWND, ms: f64) {
    unsafe {
        if !enabled() || ms <= 0.0 {
            FADES.with(|f| f.borrow_mut().remove(&hwnd));
            DestroyWindow(hwnd);
            return;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if ex & WS_EX_LAYERED == 0 {
            ensure_layered(hwnd, 255);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 | WS_EX_TRANSPARENT) as isize);
        let start = FADES.with(|f| f.borrow().get(&hwnd).map(|w| w.alpha.value_at(now()))).unwrap_or(1.0);
        let mut alpha = Tween::new(start).with_ease(Ease::InOut);
        alpha.retarget(0.0, now(), ms);
        FADES.with(|f| f.borrow_mut().insert(hwnd, WindowFade { alpha, destroy: true }));
        SetTimer(hwnd, FADE_TIMER, FRAME_MS, fade_proc as *const c_void);
    }
}

unsafe extern "system" fn fade_proc(hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    let t = now();
    let step = FADES.with(|f| f.borrow().get(&hwnd).map(|w| (w.alpha.value_at(t), w.alpha.running_at(t), w.destroy)));
    let Some((a, running, destroy)) = step else {
        KillTimer(hwnd, id);
        return;
    };
    if IsWindow(hwnd) == 0 {
        KillTimer(hwnd, id);
        FADES.with(|f| f.borrow_mut().remove(&hwnd));
        return;
    }
    SetLayeredWindowAttributes(hwnd, 0, (a.clamp(0.0, 1.0) * 255.0).round() as u8, LWA_ALPHA);
    if !running {
        KillTimer(hwnd, id);
        FADES.with(|f| f.borrow_mut().remove(&hwnd));
        if destroy {
            DestroyWindow(hwnd);
        }
    }
}

// ---------------------------------------------------------------- 交叉淡化

struct Overlay {
    bitmap: HBITMAP,
    w: i32,
    h: i32,
}

pub fn register(hinst: HINSTANCE) {
    let class = wide(OVERLAY_CLASS);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: overlay_proc,
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

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let data = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Overlay;
    match msg {
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(o) = data.as_ref() {
                let mem = CreateCompatibleDC(hdc);
                let old = SelectObject(mem, o.bitmap);
                BitBlt(hdc, 0, 0, o.w, o.h, mem, 0, 0, SRCCOPY);
                SelectObject(mem, old);
                DeleteDC(mem);
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_NCHITTEST => return HTTRANSPARENT,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE,
        WM_NCDESTROY => {
            OVERLAYS.with(|o| o.borrow_mut().retain(|(h, _)| *h != hwnd));
            let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut Overlay;
            if !p.is_null() {
                let o = Box::from_raw(p);
                DeleteObject(o.bitmap);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 交叉淡化：`begin` 擷取指定範圍目前的畫面並蓋在原處；接著照常更新底下的內容；
/// 這個值被丟棄（離開作用域）時，覆蓋層開始淡出，看起來就是舊畫面平順地變成新畫面。
/// 覆蓋層不接收滑鼠與鍵盤，淡出途中也能直接操作。
pub struct Crossfade {
    overlays: Vec<HWND>,
    ms: f64,
}

impl Crossfade {
    pub fn none() -> Crossfade {
        Crossfade { overlays: Vec::new(), ms: 0.0 }
    }

    /// `areas` 為 owner 用戶區座標。
    pub fn begin(owner: HWND, areas: &[RECT], ms: f64) -> Crossfade {
        if !enabled() || ms <= 0.0 || areas.is_empty() {
            return Crossfade::none();
        }
        unsafe {
            if IsWindowVisible(owner) == 0 || IsIconic(owner) != 0 {
                return Crossfade::none();
            }
            // 上一次的淡化還在進行（例如新分頁先變空白、內容隨後才讀到）：
            // 讓舊覆蓋層停在目前的透明度繼續蓋著最初的畫面，之後再一起淡出，中間狀態就不會閃出來。
            let live: Vec<(HWND, RECT)> = OVERLAYS.with(|o| o.borrow().clone()).into_iter().filter(|(h, _)| IsWindow(*h) != 0).collect();
            let t = now();
            for (h, _) in &live {
                KillTimer(*h, FADE_TIMER);
                FADES.with(|f| {
                    let mut f = f.borrow_mut();
                    let current = f.get(h).map(|w| w.alpha.value_at(t)).unwrap_or(1.0);
                    f.insert(*h, WindowFade { alpha: Tween::new(current), destroy: true });
                });
            }
            let held: Vec<HWND> = live.iter().map(|(h, _)| *h).collect();
            let covered = |a: &RECT| live.iter().any(|(_, r)| r.left <= a.left && r.top <= a.top && r.right >= a.right && r.bottom >= a.bottom);
            let areas: Vec<RECT> = areas.iter().filter(|a| !covered(a)).copied().collect();
            if areas.is_empty() {
                return Crossfade { overlays: held, ms };
            }
            let client = client_rect(owner);
            if client.width() <= 0 || client.height() <= 0 {
                return Crossfade { overlays: held, ms };
            }
            let hdc = GetDC(owner);
            if hdc == 0 {
                return Crossfade { overlays: held, ms };
            }
            let full_dc = CreateCompatibleDC(hdc);
            let full = CreateCompatibleBitmap(hdc, client.width(), client.height());
            let old_full = SelectObject(full_dc, full);
            // 以 DWM 取得包含所有子視窗的目前畫面；失敗或拿到全黑畫面時改從視窗 DC 複製，
            // 仍然不行就放棄這次動畫（寧可直接切換，也不要閃一下黑框）。
            let (w, h) = (client.width(), client.height());
            let looks_blank = |dc: HDC| [(4, 4), (w / 2, h / 2), (w - 5, h - 5)].iter().all(|(x, y)| GetPixel(dc, *x, *y) & 0x00FF_FFFF == 0);
            let mut ok = PrintWindow(owner, full_dc, PW_CLIENTONLY | PW_RENDERFULLCONTENT) != 0 && !looks_blank(full_dc);
            if !ok {
                BitBlt(full_dc, 0, 0, w, h, hdc, 0, 0, SRCCOPY);
                ok = !looks_blank(full_dc);
            }
            if !ok {
                SelectObject(full_dc, old_full);
                DeleteObject(full);
                DeleteDC(full_dc);
                ReleaseDC(owner, hdc);
                return Crossfade { overlays: held, ms };
            }
            let mut overlays: Vec<(HWND, RECT)> = Vec::new();
            let class = wide(OVERLAY_CLASS);
            for a in &areas {
                let r = RECT {
                    left: a.left.max(0),
                    top: a.top.max(0),
                    right: a.right.min(client.right),
                    bottom: a.bottom.min(client.bottom),
                };
                if r.width() <= 0 || r.height() <= 0 {
                    continue;
                }
                let part_dc = CreateCompatibleDC(hdc);
                let part = CreateCompatibleBitmap(hdc, r.width(), r.height());
                let old_part = SelectObject(part_dc, part);
                BitBlt(part_dc, 0, 0, r.width(), r.height(), full_dc, r.left, r.top, SRCCOPY);
                SelectObject(part_dc, old_part);
                DeleteDC(part_dc);
                let mut pt = POINT { x: r.left, y: r.top };
                ClientToScreen(owner, &mut pt);
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_POPUP,
                    pt.x,
                    pt.y,
                    r.width(),
                    r.height(),
                    owner,
                    0,
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                );
                if hwnd == 0 {
                    DeleteObject(part);
                    continue;
                }
                let data = Box::new(Overlay { bitmap: part, w: r.width(), h: r.height() });
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
                let off: BOOL = 1;
                DwmSetWindowAttribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &off as *const BOOL as *const c_void, 4);
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                UpdateWindow(hwnd);
                overlays.push((hwnd, r));
            }
            SelectObject(full_dc, old_full);
            DeleteObject(full);
            DeleteDC(full_dc);
            ReleaseDC(owner, hdc);
            OVERLAYS.with(|o| o.borrow_mut().extend(overlays.iter().copied()));
            let mut all = held;
            all.extend(overlays.iter().map(|(h, _)| *h));
            Crossfade { overlays: all, ms }
        }
    }
}

impl Drop for Crossfade {
    fn drop(&mut self) {
        for h in self.overlays.drain(..) {
            if unsafe { IsWindow(h) } != 0 {
                fade_out_and_destroy(h, self.ms);
            }
        }
    }
}

// ---------------------------------------------------------------- 顏色與小工具

/// 顏色依進度混合：t = 0 為 `from`，t = 1 為 `to`。
pub fn mix(from: COLORREF, to: COLORREF, t: f32) -> COLORREF {
    super::theme::blend(to, from, t.clamp(0.0, 1.0))
}

//! 拖放。
//!
//! 拖出：在清單選取檔案或資料夾後拖曳，可以放到清單中的資料夾、另一欄、側欄的資料夾或磁碟，
//! 也可以拖到檔案總管、桌面或其他程式。拖曳圖示是 Stillfeather 自己畫的分身（見 drag_image）。
//!
//! 拖入：清單與側欄都是拖放目標。Stillfeather 決定「放在哪裡」——
//! - 清單中的資料夾列 → 放進那個資料夾
//! - 清單的其他地方 → 放進這一欄目前的資料夾
//! - 側欄的資料夾、常用位置或磁碟 → 放進那裡（停留一下會自動展開）
//! - 側欄的「常用位置」標題或空白處 → 釘選成常用位置（只限資料夾）
//! - 路徑列的某一層 → 放進那一層資料夾
//! - 分頁 → 放進那個分頁目前的資料夾；停留一下會切換到那個分頁，再放進它的清單
//!
//! 實際的移動、複製與建立捷徑交給 Windows Shell 的拖放目標，規則與檔案總管相同：
//! 同一個磁碟預設移動、不同磁碟預設複製；Ctrl 複製、Shift 移動、Alt 建立捷徑；右鍵拖曳會出現選單。

use super::app::{self, id, App, NodeKind};
use super::crumbs;
use super::tabs;
use super::tree::FAVORITES_KEY;
use super::shellmenu::{self, Pidl, ShellFolderVtbl, IID_ISHELLFOLDER};
use super::sys::*;
use super::util::*;
use feather_core::paths;
use feather_core::workspace::Side;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);
const IID_IDROPTARGET: GUID = GUID::from_u128(0x00000122_0000_0000_C000_000000000046);
const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010E_0000_0000_C000_000000000046);
const CLSID_DRAGDROPHELPER: GUID = GUID::from_u128(0x4657278A_411B_11D2_839A_00C04FD918D0);
const IID_IDROPTARGETHELPER: GUID = GUID::from_u128(0x4657278B_411B_11D2_839A_00C04FD918D0);

const ALL_EFFECTS: u32 = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
/// 拖到清單或側欄上下邊緣時自動捲動
const SCROLL_EDGE: i32 = 28;
const SCROLL_EVERY_MS: f64 = 70.0;
/// 在側欄的資料夾上停留多久後自動展開
const SPRING_MS: f64 = 800.0;
/// 拖到別的分頁上停留多久切換過去
const TAB_SPRING_MS: f64 = 550.0;

// ---------------------------------------------------------------- COM 介面

#[repr(C)]
struct UnknownVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct DataObjectVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here: usize,
    query_get_data: usize,
    get_canonical_format_etc: usize,
    set_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM, BOOL) -> HRESULT,
}

/// IDropTarget（Windows Shell 的資料夾拖放目標）
#[repr(C)]
struct ShellDropVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    drag_enter: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, u32, POINT, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
}

/// IDropTargetHelper：讓拖曳圖示在 Stillfeather 的視窗上也看得到
#[repr(C)]
struct HelperVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    drag_enter: unsafe extern "system" fn(*mut c_void, HWND, *mut c_void, *const POINT, u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut c_void, *const POINT, u32) -> HRESULT,
    drop: unsafe extern "system" fn(*mut c_void, *mut c_void, *const POINT, u32) -> HRESULT,
}

unsafe fn vt<T>(obj: *mut c_void) -> &'static T {
    shellmenu::vtbl::<T>(obj)
}

unsafe fn add_ref(obj: *mut c_void) {
    if !obj.is_null() {
        (vt::<UnknownVtbl>(obj).add_ref)(obj);
    }
}

// ---------------------------------------------------------------- 拖出

/// 選取項目的拖曳資料。同一個資料夾的項目用該資料夾提供的物件（與檔案總管相同）；
/// 搜尋結果跨資料夾時改用 SHCreateDataObject。
fn data_object(owner: HWND, items: &[String]) -> Option<*mut c_void> {
    unsafe {
        if let Some(obj) = shellmenu::ui_object(owner, items, &IID_IDATAOBJECT) {
            return Some(obj);
        }
        let pidls: Vec<Pidl> = items.iter().map(|p| Pidl::parse(p)).collect::<Option<_>>()?;
        let ptrs: Vec<*const c_void> = pidls.iter().map(|p| p.0 as *const c_void).collect();
        let mut obj: *mut c_void = std::ptr::null_mut();
        let hr = SHCreateDataObject(std::ptr::null(), ptrs.len() as u32, ptrs.as_ptr(), std::ptr::null_mut(), &IID_IDATAOBJECT, &mut obj);
        (hr >= 0 && !obj.is_null()).then_some(obj)
    }
}

const IID_IDRAGSOURCEHELPER: GUID = GUID::from_u128(0xDE5BF786_477A_11D2_839D_00C04FD918D0);
const IID_IDRAGSOURCEHELPER2: GUID = GUID::from_u128(0x83E07D0D_0C5F_4163_BF1A_60B274051E40);
const IID_IIMAGELIST: GUID = GUID::from_u128(0x46EB5926_582E_4017_9FDF_E8998DAA0950);
const IID_IDROPSOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_C000_000000000046);
/// 拖曳圖示整體的不透明度（像檔案總管一樣半透明，有「被拖出來」的感覺）
const GHOST_ALPHA: f32 = 0.82;

/// IDragSourceHelper2（只有第 2 版才有 SetFlags）
#[repr(C)]
struct SourceHelperVtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    initialize_from_bitmap: unsafe extern "system" fn(*mut c_void, *const SHDRAGIMAGE, *mut c_void) -> HRESULT,
    initialize_from_window: unsafe extern "system" fn(*mut c_void, HWND, *const POINT, *mut c_void) -> HRESULT,
    set_flags: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

/// 檔案或資料夾在系統圖示清單中的位置（用實際路徑，特殊資料夾會拿到自己的圖示）。
fn system_icon_index(path: &str) -> i32 {
    let w = wide(path);
    let mut info: SHFILEINFOW = unsafe { std::mem::zeroed() };
    let r = unsafe { SHGetFileInfoW(w.as_ptr(), 0, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX) };
    if r == 0 {
        -1
    } else {
        info.iIcon
    }
}

impl App {
    /// 拖曳圖示：大圖示（多選時最多疊三個，右上角有數量），下方是名稱標籤。
    /// GDI 畫不出透明度，所以在黑底與白底各畫一次，由兩者的差算出每個像素的透明度。
    unsafe fn drag_image(&self, items: &[String]) -> Option<SHDRAGIMAGE> {
        let t = &self.theme;
        let p = &t.pal;
        let s = |v: i32| t.s(v);
        let mut list: *mut c_void = std::ptr::null_mut();
        if SHGetImageList(SHIL_EXTRALARGE, &IID_IIMAGELIST, &mut list) < 0 || list.is_null() {
            return None;
        }
        // IImageList 與 HIMAGELIST 可以互換使用；這份清單由系統持有，不需釋放
        let himl = list as HIMAGELIST;
        let icons: Vec<i32> = items.iter().take(3).map(|p| system_icon_index(p)).filter(|i| *i >= 0).collect();
        let icon = 48;
        let step = s(7);
        let stack = icon + step * (icons.len().max(1) as i32 - 1);
        let count = items.len();
        let label = paths::file_name(&items[0]).to_string();
        let font = t.fonts.ui;
        let screen = GetDC(0);
        let text_w = theme_text_width(screen, &label, font).min(s(190));
        ReleaseDC(0, screen);
        let pill_h = t.fonts.ui_height + s(8);
        let pill_w = text_w + s(20);
        let badge = s(22);
        let w = (stack + badge).max(pill_w) + s(4);
        let h = stack + s(6) + pill_h + s(2);

        let render = |bg: COLORREF| -> Option<(HDC, HBITMAP, HBITMAP, *mut u32)> {
            let info = BITMAPINFO::top_down_32(w, h);
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(0, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
            if bmp == 0 || bits.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(0);
            let old = SelectObject(dc, bmp);
            let rect = RECT::new(0, 0, w, h);
            super::theme::fill(dc, &rect, bg);
            let x0 = (w - stack) / 2;
            // 後面的圖示先畫，形成往右下疊的一疊
            for (k, idx) in icons.iter().enumerate().rev() {
                let off = step * k as i32;
                (comctl().image_list_draw)(himl, *idx, dc, x0 + off, off, ILD_TRANSPARENT);
            }
            if count > 1 {
                // 數量徽章疊在圖示右上角
                let br = RECT::new(x0 + stack - badge + s(6), 0, badge, badge);
                super::theme::circle(dc, &br, Some(p.accent), None);
                let text = if count > 99 { "99+".to_string() } else { count.to_string() };
                super::theme::draw_text(dc, &text, &br, t.fonts.small_bold, super::theme::contrast_text(p.accent), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
            let pill = RECT::new((w - pill_w) / 2, stack + s(6), pill_w, pill_h);
            super::theme::round_rect(dc, &pill, pill_h, Some(p.surface), Some(super::theme::blend(p.ink, p.surface, 0.12)));
            let tr = RECT { left: pill.left + s(10), right: pill.right - s(10), ..pill };
            super::theme::draw_text(dc, &label, &tr, font, p.ink, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            Some((dc, bmp, old, bits as *mut u32))
        };
        let (black_dc, black_bmp, black_old, black) = render(rgb(0, 0, 0))?;
        let Some((white_dc, white_bmp, white_old, white)) = render(rgb(255, 255, 255)) else {
            SelectObject(black_dc, black_old);
            DeleteObject(black_bmp);
            DeleteDC(black_dc);
            return None;
        };
        // 最終的圖：由下往上排列的 32 位元 DIB，預乘透明度
        let mut info = BITMAPINFO::top_down_32(w, h);
        info.bmiHeader.biHeight = h;
        let mut bits: *mut c_void = std::ptr::null_mut();
        let out = CreateDIBSection(0, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
        if out != 0 && !bits.is_null() {
            let dst = bits as *mut u32;
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as usize;
                    let b = *black.add(i);
                    let wv = *white.add(i);
                    let ch = |v: u32, sh: u32| ((v >> sh) & 0xFF) as i32;
                    // 白底比黑底亮多少 = 有多透明
                    let diff = (0..3).map(|k| ch(wv, k * 8) - ch(b, k * 8)).max().unwrap_or(255).clamp(0, 255);
                    let alpha = (255 - diff) as f32 * GHOST_ALPHA;
                    let a = alpha.round() as u32;
                    let scale = GHOST_ALPHA;
                    let c = |sh: u32| ((ch(b, sh) as f32 * scale).round() as u32).min(a);
                    // 由下往上：第 0 列是最底下一列
                    let row = (h - 1 - y) as usize;
                    *dst.add(row * w as usize + x as usize) = (a << 24) | (c(16) << 16) | (c(8) << 8) | c(0);
                }
            }
        }
        for (dc, bmp, old) in [(black_dc, black_bmp, black_old), (white_dc, white_bmp, white_old)] {
            SelectObject(dc, old);
            DeleteObject(bmp);
            DeleteDC(dc);
        }
        if out == 0 || bits.is_null() {
            return None;
        }
        Some(SHDRAGIMAGE {
            sizeDragImage: SIZE { cx: w, cy: h },
            // 游標落在圖示中央，像把圖示抓在手上
            ptOffset: POINT { x: w / 2, y: stack / 2 },
            hbmpDragImage: out,
            crColorKey: 0xFFFF_FFFF,
        })
    }

    /// 從清單開始拖曳選取的項目（左鍵或右鍵）。
    pub(super) fn begin_drag(&mut self, side: Side, right_button: bool) {
        let items = self.selected_paths(side);
        if items.is_empty() {
            return;
        }
        self.hide_preview();
        let list = self.lists[side.index()];
        let Some(data) = data_object(self.hwnd, &items) else { return };
        unsafe {
            // 拖曳圖示：允許拖放目標顯示說明文字（例如「移動到 下載」），再放上自己畫的圖
            let mut helper: *mut c_void = std::ptr::null_mut();
            let mut v2 = true;
            if CoCreateInstance(&CLSID_DRAGDROPHELPER, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IDRAGSOURCEHELPER2, &mut helper) < 0 {
                v2 = false;
                if CoCreateInstance(&CLSID_DRAGDROPHELPER, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IDRAGSOURCEHELPER, &mut helper) < 0 {
                    helper = std::ptr::null_mut();
                }
            }
            if !helper.is_null() {
                let hv = vt::<SourceHelperVtbl>(helper);
                if v2 {
                    (hv.set_flags)(helper, DSH_ALLOWDROPDESCRIPTIONTEXT);
                }
                let mut ok = false;
                if let Some(img) = self.drag_image(&items) {
                    ok = (hv.initialize_from_bitmap)(helper, &img, data) >= 0;
                    if !ok {
                        DeleteObject(img.hbmpDragImage);
                    }
                }
                if !ok {
                    (hv.initialize_from_window)(helper, list, std::ptr::null(), data);
                }
            }
            let source = Box::into_raw(Box::new(DropSource { vtbl: &SOURCE_VTBL, refs: Cell::new(1), right: right_button }));
            let mut effect = DROPEFFECT_NONE;
            DoDragDrop(data, source as *mut c_void, ALL_EFFECTS, &mut effect);
            ds_release(source);
            shellmenu::release(helper);
            shellmenu::release(data);
        }
        // 項目可能被移走、或在別處多了副本
        self.after_shell_command();
    }
}

fn theme_text_width(hdc: HDC, text: &str, font: HFONT) -> i32 {
    super::theme::text_width(hdc, text, font)
}

// ---------------------------------------------------------------- 拖曳來源物件（IDropSource）

#[repr(C)]
struct DropSource {
    vtbl: *const DropSourceVtbl,
    refs: Cell<u32>,
    /// 以右鍵開始拖曳：放開右鍵時放下，期間按左鍵取消（反之亦然）
    right: bool,
}

#[repr(C)]
struct DropSourceVtbl {
    query_interface: unsafe extern "system" fn(*mut DropSource, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropSource) -> u32,
    release: unsafe extern "system" fn(*mut DropSource) -> u32,
    query_continue_drag: unsafe extern "system" fn(*mut DropSource, BOOL, u32) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut DropSource, u32) -> HRESULT,
}

static SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
    query_interface: ds_query_interface,
    add_ref: ds_add_ref,
    release: ds_release,
    query_continue_drag: ds_query_continue_drag,
    give_feedback: ds_give_feedback,
};

unsafe extern "system" fn ds_query_interface(this: *mut DropSource, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_NOINTERFACE;
    }
    if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &IID_IDROPSOURCE) {
        ds_add_ref(this);
        *out = this as *mut c_void;
        0
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn ds_add_ref(this: *mut DropSource) -> u32 {
    let r = (*this).refs.get() + 1;
    (*this).refs.set(r);
    r
}

unsafe extern "system" fn ds_release(this: *mut DropSource) -> u32 {
    let r = (*this).refs.get().saturating_sub(1);
    (*this).refs.set(r);
    if r == 0 {
        drop(Box::from_raw(this));
    }
    r
}

unsafe extern "system" fn ds_query_continue_drag(this: *mut DropSource, escape: BOOL, keys: u32) -> HRESULT {
    let (main, other) = if (*this).right { (MK_RBUTTON, MK_LBUTTON) } else { (MK_LBUTTON, MK_RBUTTON) };
    if escape != 0 || keys & other != 0 {
        DRAGDROP_S_CANCEL
    } else if keys & main == 0 {
        DRAGDROP_S_DROP
    } else {
        0
    }
}

unsafe extern "system" fn ds_give_feedback(_this: *mut DropSource, _effect: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

// ---------------------------------------------------------------- 拖入

/// 註冊為拖放目標的視窗
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum DropWindow {
    List(Side),
    Tree,
    /// 路徑列（麵包屑）
    Crumbs,
    /// 分頁列
    Tabs(Side),
}

/// 放置位置在畫面上的標示
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mark {
    /// 清單中的資料夾列
    Row(Side, usize),
    /// 整欄（這一欄目前的資料夾）
    Pane(Side),
    /// 側欄項目
    Tree(HTREEITEM),
    /// 路徑列的某一層
    Crumb(usize),
    /// 分頁（那個分頁目前的資料夾）
    Tab(Side, usize),
}

/// 放開滑鼠後會發生什麼
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Zone {
    None,
    /// 放進資料夾
    Folder { path: String, mark: Mark },
    /// 釘選成側欄的常用位置
    Pin,
}

struct DragState {
    data: *mut c_void,
    paths: Vec<String>,
    zone: Zone,
    /// 目前轉交的 Windows 資料夾拖放目標
    shell: *mut c_void,
    /// 停留中的側欄項目或分頁，與開始停留的時間（停久了就展開或切換）
    spring: Option<Mark>,
    spring_since: f64,
    last_scroll: f64,
}

thread_local! {
    static DRAG: RefCell<Option<DragState>> = const { RefCell::new(None) };
    static HELPER: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    static DESCRIPTION_FORMAT: Cell<u32> = const { Cell::new(0) };
}

/// 正有東西拖到 Stillfeather 的視窗上（清單的放置標示以列號記，這段期間不要換掉清單內容）
pub(super) fn drag_active() -> bool {
    DRAG.with(|d| d.try_borrow().map_or(true, |s| s.is_some()))
}

fn helper() -> *mut c_void {
    HELPER.with(|h| {
        if h.get().is_null() {
            let mut p: *mut c_void = std::ptr::null_mut();
            unsafe {
                if CoCreateInstance(&CLSID_DRAGDROPHELPER, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IDROPTARGETHELPER, &mut p) < 0 {
                    p = std::ptr::null_mut();
                }
            }
            h.set(p);
        }
        h.get()
    })
}

/// 拖曳資料中的檔案路徑（CF_HDROP）。虛擬項目（例如手機、壓縮檔內）沒有路徑，回傳空的。
unsafe fn hdrop_paths(data: *mut c_void) -> Vec<String> {
    let fmt = FORMATETC { cfFormat: CF_HDROP as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL };
    let mut med: STGMEDIUM = std::mem::zeroed();
    if (vt::<DataObjectVtbl>(data).get_data)(data, &fmt, &mut med) < 0 {
        return Vec::new();
    }
    let h = med.hGlobal;
    let count = DragQueryFileW(h, 0xFFFF_FFFF, std::ptr::null_mut(), 0);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count.min(10_000) {
        let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0);
        let mut buf = vec![0u16; len as usize + 1];
        DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
        out.push(from_wide(&buf));
    }
    ReleaseStgMedium(&mut med);
    out
}

/// 拖曳圖示下方的說明文字。`kind` 為 DROPIMAGE_INVALID 時交還給系統與 Windows 的拖放目標。
unsafe fn set_description(data: *mut c_void, kind: i32, message: &str, insert: &str) {
    let format = DESCRIPTION_FORMAT.with(|f| {
        if f.get() == 0 {
            let name = wide("DropDescription");
            f.set(RegisterClipboardFormatW(name.as_ptr()));
        }
        f.get()
    });
    if format == 0 {
        return;
    }
    let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, std::mem::size_of::<DROPDESCRIPTION>());
    if h == 0 {
        return;
    }
    let p = GlobalLock(h) as *mut DROPDESCRIPTION;
    if p.is_null() {
        GlobalFree(h);
        return;
    }
    (*p).kind = kind;
    for (dst, src) in [(&mut (*p).szMessage, message), (&mut (*p).szInsert, insert)] {
        for (d, c) in dst.iter_mut().take(259).zip(src.encode_utf16()) {
            *d = c;
        }
    }
    GlobalUnlock(h);
    let fmt = FORMATETC { cfFormat: format as u16, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT, lindex: -1, tymed: TYMED_HGLOBAL };
    let mut med = STGMEDIUM { tymed: TYMED_HGLOBAL, hGlobal: h, pUnkForRelease: std::ptr::null_mut() };
    if (vt::<DataObjectVtbl>(data).set_data)(data, &fmt, &mut med, 1) < 0 {
        GlobalFree(h);
    }
}

/// Windows 對某個資料夾的拖放目標（放進該資料夾）。
unsafe fn folder_drop_target(owner: HWND, path: &str) -> *mut c_void {
    let Some(pidl) = Pidl::parse(path) else { return std::ptr::null_mut() };
    let mut sf: *mut c_void = std::ptr::null_mut();
    let mut child: *const c_void = std::ptr::null();
    if SHBindToParent(pidl.0, &IID_ISHELLFOLDER, &mut sf, &mut child) < 0 || sf.is_null() {
        return std::ptr::null_mut();
    }
    let mut dt: *mut c_void = std::ptr::null_mut();
    let hr = (vt::<ShellFolderVtbl>(sf).get_ui_object_of)(sf, owner, 1, &child, &IID_IDROPTARGET, std::ptr::null_mut(), &mut dt);
    shellmenu::release(sf);
    if hr < 0 {
        std::ptr::null_mut()
    } else {
        dt
    }
}

impl App {
    /// 滑鼠（螢幕座標）下的放置位置。
    pub(super) fn drop_zone(&self, window: DropWindow, pt: POINT, sources: &[String]) -> Zone {
        let zone = match window {
            DropWindow::List(side) => {
                let list = self.lists[side.index()];
                let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return Zone::None };
                let mut p = pt;
                unsafe {
                    ScreenToClient(list, &mut p);
                }
                let mut ht = LVHITTESTINFO { pt: p, flags: 0, iItem: -1, iSubItem: 0, iGroup: 0 };
                let index = send(list, LVM_HITTEST, 0, &mut ht as *mut LVHITTESTINFO as LPARAM);
                let on_folder = (index >= 0 && ht.flags & LVHT_ONITEM != 0)
                    .then(|| tab.row(index as usize))
                    .flatten()
                    .filter(|e| e.is_dir)
                    .map(|e| e.full_path());
                match on_folder {
                    Some(path) => Zone::Folder { path, mark: Mark::Row(side, index as usize) },
                    None if !tab.search_mode && !tab.path.is_empty() => Zone::Folder { path: tab.path.clone(), mark: Mark::Pane(side) },
                    None => Zone::None,
                }
            }
            DropWindow::Tree => {
                let mut p = pt;
                unsafe {
                    ScreenToClient(self.tree, &mut p);
                }
                let mut ht = TVHITTESTINFO { pt: p, flags: 0, hItem: 0 };
                let item = send(self.tree, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM);
                match self.nodes.get(&item) {
                    // 空白處：釘選
                    None => Zone::Pin,
                    Some(n) => match n.kind {
                        NodeKind::Header if n.path == FAVORITES_KEY => Zone::Pin,
                        NodeKind::Header | NodeKind::Info => Zone::None,
                        _ => Zone::Folder { path: n.path.clone(), mark: Mark::Tree(item) },
                    },
                }
            }
            DropWindow::Crumbs => {
                let segments = app::crumb_model();
                match crumbs::drop_hit(self.crumbs, pt).and_then(|i| segments.get(i).map(|s| (i, s.1.clone()))) {
                    Some((i, path)) if paths::is_fully_qualified(&path) => Zone::Folder { path, mark: Mark::Crumb(i) },
                    _ => Zone::None,
                }
            }
            DropWindow::Tabs(side) => {
                let strip = self.strips[side.index()];
                let tab = tabs::drop_hit(strip, pt).and_then(|i| self.ws.group(side).tabs.get(i).map(|id| (i, *id)));
                match tab.and_then(|(i, id)| self.tabs.get(&id).map(|t| (i, t))) {
                    Some((i, t)) if !t.search_mode && !t.path.is_empty() => Zone::Folder { path: t.path.clone(), mark: Mark::Tab(side, i) },
                    _ => Zone::None,
                }
            }
        };
        match zone {
            Zone::Folder { ref path, .. } if !paths::drop_target_ok(path, sources) => Zone::None,
            Zone::Pin if sources.is_empty() || !sources.iter().all(|s| std::path::Path::new(s).is_dir()) => Zone::None,
            z => z,
        }
    }

    /// 標示放置位置（清單的資料夾列、整欄外框、側欄項目、常用位置標題）。
    fn show_drop_zone(&mut self, zone: &Zone, on: bool) {
        match zone {
            Zone::Folder { mark, .. } => self.show_mark(*mark, on),
            Zone::Pin => {
                self.drop_pin = on;
                invalidate(self.tree);
            }
            Zone::None => {}
        }
    }

    fn show_mark(&mut self, mark: Mark, on: bool) {
        match mark {
            Mark::Row(side, index) => {
                self.drop_row = if on { Some((side, index)) } else { None };
                let list = self.lists[side.index()];
                let mut r = RECT { left: LVIR_BOUNDS, ..Default::default() };
                if send(list, LVM_GETITEMRECT, index, &mut r as *mut RECT as LPARAM) != 0 {
                    unsafe {
                        InvalidateRect(list, &r, 0);
                    }
                }
            }
            Mark::Pane(side) => {
                self.drop_pane = if on { Some(side) } else { None };
                let r = self.layout.panes[side.index()].inflate(self.s(4), self.s(6));
                unsafe {
                    InvalidateRect(self.hwnd, &r, 0);
                }
            }
            Mark::Tree(item) => {
                send(self.tree, TVM_SELECTITEM, TVGN_DROPHILITE, if on { item } else { 0 });
            }
            Mark::Crumb(index) => crumbs::set_drop(self.crumbs, on.then_some(index)),
            Mark::Tab(side, index) => tabs::set_drop(self.strips[side.index()], on.then_some(index)),
        }
    }

    /// 拖放結束（放開或離開）：清掉所有標示。
    fn clear_drop_marks(&mut self) {
        if let Some((side, index)) = self.drop_row.take() {
            self.show_mark(Mark::Row(side, index), false);
        }
        crumbs::set_drop(self.crumbs, None);
        for strip in self.strips {
            tabs::set_drop(strip, None);
        }
        if let Some(side) = self.drop_pane.take() {
            let r = self.layout.panes[side.index()].inflate(self.s(4), self.s(6));
            unsafe {
                InvalidateRect(self.hwnd, &r, 0);
            }
        }
        if self.drop_pin {
            self.drop_pin = false;
            invalidate(self.tree);
        }
        send(self.tree, TVM_SELECTITEM, TVGN_DROPHILITE, 0);
    }

    /// 拖到清單或側欄的上下邊緣時自動捲動；在側欄的資料夾上停留一下會自動展開，
    /// 在別的分頁上停留一下會切換過去（即使那個分頁本身不能放，例如資料夾相同）。
    fn drag_assist(&mut self, window: DropWindow, pt: POINT, st: &mut DragState) {
        let now = super::anim::now();
        let scroll_target = match window {
            DropWindow::List(side) => Some(self.lists[side.index()]),
            DropWindow::Tree => Some(self.tree),
            DropWindow::Crumbs | DropWindow::Tabs(_) => None,
        };
        if let Some(target) = scroll_target {
            let mut p = pt;
            unsafe {
                ScreenToClient(target, &mut p);
            }
            let rc = client_rect(target);
            let edge = self.s(SCROLL_EDGE);
            if now - st.last_scroll > SCROLL_EVERY_MS {
                if p.y < rc.top + edge {
                    send(target, WM_VSCROLL, SB_LINEUP, 0);
                    st.last_scroll = now;
                } else if p.y > rc.bottom - edge {
                    send(target, WM_VSCROLL, SB_LINEDOWN, 0);
                    st.last_scroll = now;
                }
            }
        }
        let spring = match window {
            DropWindow::Tree => match st.zone {
                Zone::Folder { mark: Mark::Tree(item), .. } => Some(Mark::Tree(item)),
                _ => None,
            },
            DropWindow::Tabs(side) => tabs::drop_hit(self.strips[side.index()], pt).map(|i| Mark::Tab(side, i)),
            _ => None,
        };
        if spring != st.spring {
            st.spring = spring;
            st.spring_since = now;
            return;
        }
        match spring {
            Some(Mark::Tree(item)) => {
                let expanded = send(self.tree, TVM_GETITEMSTATE, item as usize, TVIS_EXPANDED as LPARAM) as u32 & TVIS_EXPANDED != 0;
                if !expanded && now - st.spring_since > SPRING_MS {
                    send(self.tree, TVM_EXPAND, TVE_EXPAND, item);
                }
            }
            Some(Mark::Tab(side, index)) => {
                let switch = index != self.ws.group(side).selected || side != self.active_side();
                if switch && now - st.spring_since > TAB_SPRING_MS {
                    // 交給主視窗的訊息迴圈切換，不在拖放的回呼裡重建清單
                    tabs::request_select(side, index);
                    st.spring_since = f64::INFINITY;
                }
            }
            _ => {}
        }
    }
}

/// 依滑鼠位置更新放置位置，回傳這次的效果（複製、移動、捷徑或不可放置）。
unsafe fn update(window: DropWindow, keys: u32, pt: POINT, allowed: u32) -> u32 {
    let Some(a) = app::app_opt() else { return DROPEFFECT_NONE };
    let Some(mut st) = DRAG.with(|d| d.borrow_mut().take()) else { return DROPEFFECT_NONE };
    let zone = a.drop_zone(window, pt, &st.paths);
    if zone != st.zone {
        // 換了位置：離開原本的 Windows 拖放目標，標示新位置
        if !st.shell.is_null() {
            (vt::<ShellDropVtbl>(st.shell).drag_leave)(st.shell);
            shellmenu::release(st.shell);
            st.shell = std::ptr::null_mut();
        }
        a.show_drop_zone(&st.zone, false);
        a.show_drop_zone(&zone, true);
        match &zone {
            Zone::Pin => set_description(st.data, DROPIMAGE_LINK, tr!("釘選到 %1", "Pin to %1"), tr!("常用位置", "Favorites")),
            _ => set_description(st.data, DROPIMAGE_INVALID, "", ""),
        }
        if let Zone::Folder { path, .. } = &zone {
            st.shell = folder_drop_target(a.hwnd, path);
            if !st.shell.is_null() {
                let mut e = allowed;
                if (vt::<ShellDropVtbl>(st.shell).drag_enter)(st.shell, st.data, keys, pt, &mut e) < 0 {
                    shellmenu::release(st.shell);
                    st.shell = std::ptr::null_mut();
                }
            }
        }
        st.zone = zone;
    }
    a.drag_assist(window, pt, &mut st);
    let effect = match &st.zone {
        Zone::Folder { .. } if !st.shell.is_null() => {
            let mut e = allowed;
            if (vt::<ShellDropVtbl>(st.shell).drag_over)(st.shell, keys, pt, &mut e) < 0 {
                e = DROPEFFECT_NONE;
            }
            e
        }
        Zone::Pin => [DROPEFFECT_LINK, DROPEFFECT_COPY, DROPEFFECT_MOVE].into_iter().find(|e| allowed & e != 0).unwrap_or(DROPEFFECT_NONE),
        _ => DROPEFFECT_NONE,
    };
    DRAG.with(|d| *d.borrow_mut() = Some(st));
    effect
}

/// 結束拖曳：放開 Windows 拖放目標與拖曳資料，清掉標示。
unsafe fn finish() -> Option<DragState> {
    let st = DRAG.with(|d| d.borrow_mut().take());
    if let Some(a) = app::app_opt() {
        a.clear_drop_marks();
    }
    st
}

unsafe fn release_state(st: DragState) {
    if !st.shell.is_null() {
        shellmenu::release(st.shell);
    }
    shellmenu::release(st.data);
}

// ---------------------------------------------------------------- 拖放目標物件（IDropTarget）

#[repr(C)]
struct DropTarget {
    vtbl: *const DropTargetVtbl,
    refs: Cell<u32>,
    window: DropWindow,
    hwnd: HWND,
}

#[repr(C)]
struct DropTargetVtbl {
    query_interface: unsafe extern "system" fn(*mut DropTarget, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut DropTarget) -> u32,
    release: unsafe extern "system" fn(*mut DropTarget) -> u32,
    drag_enter: unsafe extern "system" fn(*mut DropTarget, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
    drag_over: unsafe extern "system" fn(*mut DropTarget, u32, POINT, *mut u32) -> HRESULT,
    drag_leave: unsafe extern "system" fn(*mut DropTarget) -> HRESULT,
    drop: unsafe extern "system" fn(*mut DropTarget, *mut c_void, u32, POINT, *mut u32) -> HRESULT,
}

static VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface: dt_query_interface,
    add_ref: dt_add_ref,
    release: dt_release,
    drag_enter: dt_drag_enter,
    drag_over: dt_drag_over,
    drag_leave: dt_drag_leave,
    drop: dt_drop,
};

const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;

fn same_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn dt_query_interface(this: *mut DropTarget, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_NOINTERFACE;
    }
    if same_guid(&*riid, &IID_IUNKNOWN) || same_guid(&*riid, &IID_IDROPTARGET) {
        dt_add_ref(this);
        *out = this as *mut c_void;
        0
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn dt_add_ref(this: *mut DropTarget) -> u32 {
    let r = (*this).refs.get() + 1;
    (*this).refs.set(r);
    r
}

unsafe extern "system" fn dt_release(this: *mut DropTarget) -> u32 {
    let r = (*this).refs.get().saturating_sub(1);
    (*this).refs.set(r);
    if r == 0 {
        drop(Box::from_raw(this));
    }
    r
}

unsafe extern "system" fn dt_drag_enter(this: *mut DropTarget, data: *mut c_void, keys: u32, pt: POINT, effect: *mut u32) -> HRESULT {
    if let Some(old) = finish() {
        release_state(old);
    }
    add_ref(data);
    let paths = hdrop_paths(data);
    DRAG.with(|d| {
        *d.borrow_mut() = Some(DragState { data, paths, zone: Zone::None, shell: std::ptr::null_mut(), spring: None, spring_since: 0.0, last_scroll: 0.0 })
    });
    let allowed = if effect.is_null() { ALL_EFFECTS } else { *effect };
    let e = update((*this).window, keys, pt, allowed);
    let h = helper();
    if !h.is_null() {
        (vt::<HelperVtbl>(h).drag_enter)(h, (*this).hwnd, data, &pt, e);
    }
    if !effect.is_null() {
        *effect = e;
    }
    0
}

unsafe extern "system" fn dt_drag_over(this: *mut DropTarget, keys: u32, pt: POINT, effect: *mut u32) -> HRESULT {
    let allowed = if effect.is_null() { ALL_EFFECTS } else { *effect };
    let e = update((*this).window, keys, pt, allowed);
    let h = helper();
    if !h.is_null() {
        (vt::<HelperVtbl>(h).drag_over)(h, &pt, e);
    }
    if !effect.is_null() {
        *effect = e;
    }
    0
}

unsafe extern "system" fn dt_drag_leave(_this: *mut DropTarget) -> HRESULT {
    let h = helper();
    if !h.is_null() {
        (vt::<HelperVtbl>(h).drag_leave)(h);
    }
    if let Some(st) = finish() {
        if !st.shell.is_null() {
            (vt::<ShellDropVtbl>(st.shell).drag_leave)(st.shell);
        }
        set_description(st.data, DROPIMAGE_INVALID, "", "");
        release_state(st);
    }
    0
}

unsafe extern "system" fn dt_drop(this: *mut DropTarget, data: *mut c_void, keys: u32, pt: POINT, effect: *mut u32) -> HRESULT {
    let allowed = if effect.is_null() { ALL_EFFECTS } else { *effect };
    let e = update((*this).window, keys, pt, allowed);
    let h = helper();
    if !h.is_null() {
        (vt::<HelperVtbl>(h).drop)(h, data, &pt, e);
    }
    let Some(st) = finish() else { return 0 };
    let mut result = DROPEFFECT_NONE;
    match &st.zone {
        Zone::Folder { path, .. } if !st.shell.is_null() && e != DROPEFFECT_NONE => {
            // 由 Windows 執行移動、複製或建立捷徑（可能顯示進度、衝突或右鍵選單）
            let mut out = allowed;
            if (vt::<ShellDropVtbl>(st.shell).drop)(st.shell, data, keys, pt, &mut out) >= 0 {
                result = out;
            }
            if let Some(a) = app::app_opt() {
                // 可能是移動就記下來（Windows 可能還在背景搬，也可能其實是複製）：
                // 按 Ctrl+Z 時才依檔案現況判斷，沒被移走的項目不會動
                let moved = out & DROPEFFECT_MOVE != 0 || (out == DROPEFFECT_NONE && e & DROPEFFECT_MOVE != 0);
                if moved && !st.paths.is_empty() {
                    a.record_move(&st.paths, path);
                }
                a.after_shell_command();
            }
        }
        Zone::Folder { .. } if !st.shell.is_null() => {
            (vt::<ShellDropVtbl>(st.shell).drag_leave)(st.shell);
        }
        Zone::Pin if e != DROPEFFECT_NONE => {
            let dirs: Vec<String> = st.paths.iter().filter(|p| std::path::Path::new(p).is_dir()).cloned().collect();
            if let Some(a) = app::app_opt() {
                // 拖放的處理還在進行中（來源程式等著回應），稍後再加入，避免在這裡跳出對話方塊
                a.pending_pins = dirs;
                PostMessageW(a.hwnd, WM_COMMAND, id::PIN_DROPPED as usize, 0);
            }
            result = e;
        }
        _ => {}
    }
    set_description(st.data, DROPIMAGE_INVALID, "", "");
    release_state(st);
    if !effect.is_null() {
        *effect = result;
    }
    0
}

/// 讓視窗成為拖放目標。
pub(super) fn register(hwnd: HWND, window: DropWindow) {
    let target = Box::into_raw(Box::new(DropTarget { vtbl: &VTBL, refs: Cell::new(1), window, hwnd }));
    unsafe {
        RegisterDragDrop(hwnd, target as *mut c_void);
        // RegisterDragDrop 自己保有一個參考；交還我們建立時的那一個
        dt_release(target);
    }
}

pub(super) fn revoke(hwnd: HWND) {
    unsafe {
        RevokeDragDrop(hwnd);
    }
}

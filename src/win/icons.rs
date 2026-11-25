//! 系統圖示快取。清單只依副檔名取圖示（SHGFI_USEFILEATTRIBUTES），不讀取檔案內容；
//! 每種副檔名只取一次。清單列高由影像清單高度決定，所以把圖示置中放進較高的影像。
//!
//! 圖示類的檢視（超大／大／中圖示、並排、內容）另有一份大圖示影像清單，索引與小圖示清單一致
//! （同一個項目在兩份清單用同一個索引），切換大小時依原本的順序重新載入。

use super::sys::*;
use super::util::*;
use feather_core::entry::Entry;
use feather_core::view;
use feather_core::winres;
use std::collections::HashMap;

const IID_IIMAGELIST: GUID = GUID::from_u128(0x46EB5926_582E_4017_9FDF_E8998DAA0950);
/// 透明的那一格（有縮圖的項目用它，縮圖由清單自繪畫在同一個位置）
const BLANK_KEY: &str = "\\blank";

pub struct Icons {
    pub list: HIMAGELIST,
    pub tree: HIMAGELIST,
    map: HashMap<String, i32>,
    pub icon_px: i32,
    pub row_px: i32,
    /// 側欄的圖示由 Stillfeather 自己畫（見 tree.rs）；影像清單只放一張透明圖，替圖示保留位置。
    pub tree_blank: i32,
    pub tree_slot: i32,
    /// 大圖示影像清單（尚未用到圖示類檢視時為 0）與其邊長
    /// 兩欄各自的大圖示清單（兩欄的檢視可以不同）
    pub big: [HIMAGELIST; 2],
    pub big_px: [i32; 2],
    /// 依加入影像清單的順序記下每個鍵（副檔名，或資料夾），重建大圖示清單時照同樣順序載入
    order: Vec<(String, bool)>,
    /// 每個檔案各自的圖示（捷徑、程式…，見 fileicons.rs）：檔案的鍵 → 索引
    files: HashMap<String, i32>,
}

impl Icons {
    /// `text_px`：清單文字（`Fonts::body`）的單行高度；文字調大時列高跟著加高，字才不會被切掉
    pub fn new(dpi: u32, compact: bool, text_px: i32) -> Icons {
        let icon_px = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        let row_px = if compact { icon_px + scale(4, dpi) } else { (icon_px + scale(8, dpi)).max(scale(28, dpi)) };
        let row_px = row_px.max(text_px + scale(if compact { 6 } else { 12 }, dpi));
        let cc = comctl();
        let (list, tree) = unsafe {
            (
                (cc.image_list_create)(icon_px, row_px, ILC_COLOR32 | ILC_MASK, 32, 32),
                (cc.image_list_create)(scale(22, dpi), scale(22, dpi), ILC_COLOR32 | ILC_MASK, 1, 1),
            )
        };
        let mut icons = Icons {
            list,
            tree,
            map: HashMap::new(),
            icon_px,
            row_px,
            tree_blank: I_IMAGENONE,
            tree_slot: scale(22, dpi),
            big: [0; 2],
            big_px: [0; 2],
            order: Vec::new(),
            files: HashMap::new(),
        };
        icons.tree_blank = icons.add_blank();
        icons
    }

    /// 完全透明的圖示（遮罩全為 1、顏色全為 0）。失敗時改用「不顯示圖示」。
    fn add_blank(&mut self) -> i32 {
        let size = self.tree_slot;
        unsafe {
            let row = ((size + 15) / 16 * 2) as usize; // 單色點陣圖每列以 WORD 對齊
            let mask_bits = vec![0xFFu8; row * size as usize];
            let color_bits = vec![0u8; (size * size * 4) as usize];
            let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const std::ffi::c_void);
            let color = CreateBitmap(size, size, 1, 32, color_bits.as_ptr() as *const std::ffi::c_void);
            let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
            let icon = CreateIconIndirect(&info);
            DeleteObject(mask);
            DeleteObject(color);
            if icon == 0 {
                return I_IMAGENONE;
            }
            let index = (comctl().image_list_replace_icon)(self.tree, -1, icon);
            DestroyIcon(icon);
            if index < 0 {
                I_IMAGENONE
            } else {
                index
            }
        }
    }

    /// 完全透明的一格（小圖示與大圖示清單用同一個索引）：有縮圖的項目用它，縮圖另外畫上去
    pub fn blank_index(&mut self) -> i32 {
        if let Some(i) = self.map.get(BLANK_KEY) {
            return *i;
        }
        let index = unsafe {
            let icon = make_icon(&vec![0u32; (self.icon_px * self.row_px) as usize], self.icon_px, self.row_px);
            if icon == 0 {
                return I_IMAGENONE;
            }
            let i = (comctl().image_list_replace_icon)(self.list, -1, icon);
            DestroyIcon(icon);
            if i >= 0 {
                self.order.push((BLANK_KEY.to_string(), false));
                for slot in 0..2 {
                    if self.big[slot] != 0 {
                        add_big(self.big[slot], self.big_px[slot], BLANK_KEY, false);
                    }
                }
            }
            i
        };
        let index = if index >= 0 { index } else { I_IMAGENONE };
        self.map.insert(BLANK_KEY.to_string(), index);
        index
    }

    /// 清單列的圖示索引（依副檔名快取）。
    pub fn index_for(&mut self, entry: &Entry) -> i32 {
        let key = if entry.is_dir { "\\dir".to_string() } else { entry.extension() };
        if let Some(i) = self.map.get(&key) {
            return *i;
        }
        let index = unsafe { self.load(&key, entry.is_dir) };
        self.map.insert(key, index);
        index
    }

    /// 這個檔案自己的圖示已經取到的話，回傳它的索引（鍵見 `feather_core::fileicons::key`）
    pub fn file_icon(&self, key: &str) -> Option<i32> {
        self.files.get(key).copied()
    }

    /// 已經記住幾個檔案自己的圖示
    pub fn file_icon_count(&self) -> usize {
        self.files.len()
    }

    /// 加入一個檔案自己的小圖示（背景取到的，呼叫端負責 DestroyIcon `icon`），回傳索引。
    /// 大圖示清單的同一格放這種副檔名的圖示：大圖示檢視仍顯示副檔名的圖示，索引和小圖示清單保持一致。
    pub fn add_file_icon(&mut self, key: &str, ext: &str, icon: HICON) -> Option<i32> {
        if let Some(i) = self.files.get(key) {
            return Some(*i);
        }
        let index = unsafe {
            let padded = padded_icon(icon, self.icon_px, self.row_px);
            if padded != 0 {
                let i = (comctl().image_list_replace_icon)(self.list, -1, padded);
                DestroyIcon(padded);
                i
            } else {
                (comctl().image_list_replace_icon)(self.list, -1, icon)
            }
        };
        if index < 0 {
            return None;
        }
        let ext = ext.to_lowercase();
        for slot in 0..2 {
            if self.big[slot] != 0 {
                unsafe {
                    add_big(self.big[slot], self.big_px[slot], &ext, false);
                }
            }
        }
        self.order.push((ext, false));
        self.files.insert(key.to_string(), index);
        Some(index)
    }

    /// 某種副檔名（不含點，例如 "docx"）的檔案圖示索引（設定視窗預覽文字大小的範例檔名用）
    pub fn file_index(&mut self, ext: &str) -> i32 {
        let key = ext.to_lowercase();
        if let Some(i) = self.map.get(&key) {
            return *i;
        }
        let index = unsafe { self.load(&key, false) };
        self.map.insert(key, index);
        index
    }

    /// 資料夾的圖示索引（麵包屑的子資料夾選單用，與清單裡的資料夾相同）
    pub fn folder_index(&mut self) -> i32 {
        const KEY: &str = "\\dir";
        if let Some(i) = self.map.get(KEY) {
            return *i;
        }
        let index = unsafe { self.load(KEY, true) };
        self.map.insert(KEY.to_string(), index);
        index
    }

    unsafe fn load(&mut self, ext: &str, is_dir: bool) -> i32 {
        let name = if is_dir {
            "folder".to_string()
        } else if ext.is_empty() {
            "file".to_string()
        } else {
            format!("file.{ext}")
        };
        let w = wide(&name);
        let mut info: SHFILEINFOW = std::mem::zeroed();
        let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        let ok = SHGetFileInfoW(
            w.as_ptr(),
            attrs,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        );
        if ok == 0 || info.hIcon == 0 {
            return -1;
        }
        let padded = padded_icon(info.hIcon, self.icon_px, self.row_px);
        let index = if padded != 0 {
            let i = (comctl().image_list_replace_icon)(self.list, -1, padded);
            DestroyIcon(padded);
            i
        } else {
            (comctl().image_list_replace_icon)(self.list, -1, info.hIcon)
        };
        DestroyIcon(info.hIcon);
        if index >= 0 {
            self.order.push((ext.to_string(), is_dir));
            for slot in 0..2 {
                if self.big[slot] != 0 {
                    add_big(self.big[slot], self.big_px[slot], ext, is_dir);
                }
            }
        }
        index
    }

    /// 為某一欄（`slot` 0 左、1 右）準備指定邊長的大圖示清單。換了大小時回傳舊的清單：等清單控制項改用新的之後再銷毀。
    pub fn ensure_big(&mut self, slot: usize, px: i32) -> Option<HIMAGELIST> {
        if self.big[slot] != 0 && self.big_px[slot] == px {
            return None;
        }
        let list = unsafe { (comctl().image_list_create)(px, px, ILC_COLOR32 | ILC_MASK, self.order.len() as i32 + 8, 16) };
        if list == 0 {
            return None;
        }
        for (ext, is_dir) in &self.order {
            unsafe {
                add_big(list, px, ext, *is_dir);
            }
        }
        let old = std::mem::replace(&mut self.big[slot], list);
        self.big_px[slot] = px;
        (old != 0).then_some(old)
    }

    pub fn destroy(&mut self) {
        unsafe {
            if self.list != 0 {
                (comctl().image_list_destroy)(self.list);
            }
            if self.tree != 0 {
                (comctl().image_list_destroy)(self.tree);
            }
            for big in self.big {
                if big != 0 {
                    (comctl().image_list_destroy)(big);
                }
            }
        }
        self.list = 0;
        self.tree = 0;
        self.big = [0; 2];
        self.order.clear();
        self.map.clear();
        self.files.clear();
    }
}

/// 把一個大圖示加進大圖示清單；取不到時（以及透明的那一格）放透明圖，讓索引仍與小圖示清單一致。
unsafe fn add_big(list: HIMAGELIST, px: i32, ext: &str, is_dir: bool) {
    let mut icon = if ext == BLANK_KEY { 0 } else { big_icon(ext, is_dir, px) };
    if icon == 0 {
        icon = make_icon(&vec![0u32; (px * px) as usize], px, px);
    }
    if icon != 0 {
        (comctl().image_list_replace_icon)(list, -1, icon);
        DestroyIcon(icon);
    }
}

/// 系統影像清單中的圖示（依邊長選 16／32／48／256）。
unsafe fn system_icon(index: i32, source: i32) -> Option<(Vec<u32>, i32, i32)> {
    let kind = match source {
        16 => SHIL_SMALL,
        32 => SHIL_LARGE,
        48 => SHIL_EXTRALARGE,
        _ => SHIL_JUMBO,
    };
    let mut list: *mut std::ffi::c_void = std::ptr::null_mut();
    if SHGetImageList(kind, &IID_IIMAGELIST, &mut list) < 0 || list.is_null() {
        return None;
    }
    // IImageList 與 HIMAGELIST 可以互換使用；這份清單由系統持有，不需釋放
    let icon = (comctl().image_list_get_icon)(list as HIMAGELIST, index, ILD_TRANSPARENT);
    if icon == 0 {
        return None;
    }
    let pixels = icon_pixels(icon);
    DestroyIcon(icon);
    pixels
}

/// 依副檔名取得邊長 `px` 的圖示。256 像素版本若只是角落的小圖（舊式圖示），改把 48 像素版本置中，
/// 和檔案總管一樣，不把小圖放大到模糊。
unsafe fn big_icon(ext: &str, is_dir: bool, px: i32) -> HICON {
    let name = if is_dir {
        "folder".to_string()
    } else if ext.is_empty() {
        "file".to_string()
    } else {
        format!("file.{ext}")
    };
    let w = wide(&name);
    let mut info: SHFILEINFOW = std::mem::zeroed();
    let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    if SHGetFileInfoW(w.as_ptr(), attrs, &mut info, std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES) == 0 {
        return 0;
    }
    let source = view::source_px(px);
    let Some((pixels, sw, sh)) = system_icon(info.iIcon, source) else { return 0 };
    let size = sw.min(sh) as usize;
    let square: Vec<u32> = if sw == sh { pixels } else { (0..size * size).map(|i| pixels[(i / size) * sw as usize + i % size]).collect() };
    let pm = premultiply(&square);
    let out = if size >= 128 && view::is_small_in_big(&pm, size) {
        match system_icon(info.iIcon, 48) {
            Some((small, w48, h48)) if w48 == h48 => view::center_into(&premultiply(&small), w48 as usize, px as usize),
            _ => view::center_into(&pm, size, px as usize),
        }
    } else {
        view::resample(&pm, size, px as usize)
    };
    make_icon(&unpremultiply(&out), px, px)
}

fn premultiply(px: &[u32]) -> Vec<u32> {
    px.iter()
        .map(|p| {
            let a = p >> 24;
            let ch = |shift: u32| (((p >> shift) & 0xFF) * a + 127) / 255;
            (a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
        })
        .collect()
}

fn unpremultiply(px: &[u32]) -> Vec<u32> {
    px.iter()
        .map(|p| {
            let a = p >> 24;
            if a == 0 {
                return 0;
            }
            let ch = |shift: u32| ((((p >> shift) & 0xFF) * 255 + a / 2) / a).min(255);
            (a << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
        })
        .collect()
}

/// 讀出圖示的像素（0xAARRGGBB，未預乘），回傳（像素, 寬, 高）。
unsafe fn icon_pixels(src: HICON) -> Option<(Vec<u32>, i32, i32)> {
    let mut ii: ICONINFO = std::mem::zeroed();
    if GetIconInfo(src, &mut ii) == 0 {
        return None;
    }
    let cleanup = |ii: &ICONINFO| {
        if ii.hbmColor != 0 {
            DeleteObject(ii.hbmColor);
        }
        if ii.hbmMask != 0 {
            DeleteObject(ii.hbmMask);
        }
    };
    if ii.hbmColor == 0 {
        cleanup(&ii);
        return None;
    }
    let mut bm: BITMAP = std::mem::zeroed();
    if GetObjectW(ii.hbmColor, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut std::ffi::c_void) == 0
        || bm.bmWidth <= 0
        || bm.bmHeight <= 0
        || bm.bmWidth > 256
        || bm.bmHeight > 256
    {
        cleanup(&ii);
        return None;
    }
    let (sw, sh) = (bm.bmWidth, bm.bmHeight);
    let hdc = GetDC(0);
    let mut color = vec![0u32; (sw * sh) as usize];
    let mut bi = BITMAPINFO::top_down_32(sw, sh);
    let lines = GetDIBits(hdc, ii.hbmColor, 0, sh as u32, color.as_mut_ptr() as *mut std::ffi::c_void, &mut bi, DIB_RGB_COLORS);
    if lines == 0 {
        ReleaseDC(0, hdc);
        cleanup(&ii);
        return None;
    }
    if !color.iter().any(|p| p >> 24 != 0) {
        // 舊式圖示沒有 alpha：用遮罩決定透明度（遮罩白色＝透明）。
        let mut mask = vec![0u32; (sw * sh) as usize];
        let mut bim = BITMAPINFO::top_down_32(sw, sh);
        GetDIBits(hdc, ii.hbmMask, 0, sh as u32, mask.as_mut_ptr() as *mut std::ffi::c_void, &mut bim, DIB_RGB_COLORS);
        for (c, m) in color.iter_mut().zip(mask.iter()) {
            let alpha = if m & 0x00FF_FFFF == 0 { 0xFF00_0000 } else { 0 };
            *c = (*c & 0x00FF_FFFF) | alpha;
        }
    }
    ReleaseDC(0, hdc);
    cleanup(&ii);
    Some((color, sw, sh))
}

/// 由 w×h 的 0xAARRGGBB 像素（未預乘）建立圖示。
unsafe fn make_icon(out: &[u32], w: i32, h: i32) -> HICON {
    let hdc = GetDC(0);
    let bi_out = BITMAPINFO::top_down_32(w, h);
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbm = CreateDIBSection(hdc, &bi_out, DIB_RGB_COLORS, &mut bits, 0, 0);
    ReleaseDC(0, hdc);
    if hbm == 0 || bits.is_null() {
        if hbm != 0 {
            DeleteObject(hbm);
        }
        return 0;
    }
    std::ptr::copy_nonoverlapping(out.as_ptr(), bits as *mut u32, out.len().min((w * h) as usize));
    // 單色遮罩：每列以 16 位元對齊，透明像素設為 1。
    let stride = (((w + 15) / 16) * 2) as usize;
    let mut mask_bits = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            if out.get(y * w as usize + x).is_none_or(|p| p >> 24 < 128) {
                mask_bits[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    let hmask = CreateBitmap(w, h, 1, 1, mask_bits.as_ptr() as *const std::ffi::c_void);
    let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: hmask, hbmColor: hbm };
    let icon = CreateIconIndirect(&info);
    DeleteObject(hbm);
    if hmask != 0 {
        DeleteObject(hmask);
    }
    icon
}

/// 讀出圖示像素（含 alpha），放進 w×h 透明畫布的置中位置，建立新的圖示。
unsafe fn padded_icon(src: HICON, w: i32, h: i32) -> HICON {
    let Some((color, sw, sh)) = icon_pixels(src) else { return 0 };
    let size = w.min(h);
    let ox = (w - size) / 2;
    let oy = (h - size) / 2;
    let mut out = vec![0u32; (w * h) as usize];
    for y in 0..size {
        for x in 0..size {
            let sx = (x * sw / size).min(sw - 1);
            let sy = (y * sh / size).min(sh - 1);
            out[((oy + y) * w + ox + x) as usize] = color[(sy * sw + sx) as usize];
        }
    }
    make_icon(&out, w, h)
}

static APP_ICO: &[u8] = include_bytes!("../../assets/FeatherFiles.ico");

/// Stillfeather 的程式圖示（標題列、工作列、Alt+Tab）。優先用 exe 內嵌的資源；
/// 沒有資源時（GNU 工具鏈找不到 windres）改用編進程式裡的同一份 .ico 資料。
pub fn app_icon(size: i32) -> HICON {
    let size = size.max(16);
    unsafe {
        let from_resource = LoadImageW(GetModuleHandleW(std::ptr::null()), winres::ICON_ID as usize as *const u16, IMAGE_ICON, size, size, LR_DEFAULTCOLOR);
        if from_resource != 0 {
            return from_resource;
        }
        let Some(images) = winres::parse_ico(APP_ICO) else { return 0 };
        let Some(i) = winres::pick_size(&images, size as u32) else { return 0 };
        let data = images[i].data;
        CreateIconFromResourceEx(data.as_ptr(), data.len() as u32, 1, 0x0003_0000, size, size, LR_DEFAULTCOLOR)
    }
}

/// 側欄標頭的 Stillfeather 草書字樣：等比縮放到不超過 max_w × max_h，以 `ink` 上色。
/// 回傳（圖示, 寬, 高），呼叫端負責 DestroyIcon；失敗時圖示為 0。
pub fn wordmark_icon(max_w: i32, max_h: i32, ink: COLORREF, dpi: u32) -> (HICON, i32, i32) {
    if max_w < 8 || max_h < 8 {
        return (0, 0, 0);
    }
    // 髮絲線至少約 0.9 個像素（高 DPI 時跟著放大），小尺寸才看得到筆畫
    let min_line = 0.9 * (dpi.max(96) as f32 / 96.0).min(2.0);
    let m = feather_core::wordmark::render(max_w as f32, max_h as f32, min_line);
    let (r, g, b) = (ink & 0xFF, (ink >> 8) & 0xFF, (ink >> 16) & 0xFF);
    let rgb = (r << 16) | (g << 8) | b;
    let px: Vec<u32> = m.alpha.iter().map(|&a| ((a as u32) << 24) | rgb).collect();
    let icon = unsafe { make_icon(&px, m.width as i32, m.height as i32) };
    (icon, m.width as i32, m.height as i32)
}

/// 側欄標頭的品牌圖示（白色羽毛），呼叫端負責 DestroyIcon。
/// .ico 裡剛好有這個尺寸就直接用；沒有時（例如 125%、150% 縮放）從大一號的那張以面積平均縮小，
/// 邊緣比系統縮放圖示來得平滑，也保留同尺寸級距的細節（中尺寸有淡灰外框，在淺色側欄上才看得清楚）。
pub fn brand_icon(size: i32) -> HICON {
    let size = size.clamp(16, 256);
    let Some(images) = winres::parse_ico(APP_ICO) else { return app_icon(size) };
    if winres::has_size(&images, size as u32) {
        return app_icon(size);
    }
    let source = winres::pick_size(&images, size as u32).map(|i| if images[i].width == 0 { 256 } else { images[i].width as i32 }).unwrap_or(256);
    unsafe {
        let big = app_icon(source);
        if big == 0 {
            return app_icon(size);
        }
        let pixels = icon_pixels(big);
        DestroyIcon(big);
        let Some((px, w, h)) = pixels else { return app_icon(size) };
        let out = winres::downscale_argb(&px, w as usize, h as usize, size as usize, size as usize);
        let icon = make_icon(&out, size, size);
        if icon != 0 {
            icon
        } else {
            app_icon(size)
        }
    }
}

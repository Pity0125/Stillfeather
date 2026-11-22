//! 剪貼簿：檔案清單（CF_HDROP + Preferred DropEffect，與檔案總管互通）與純文字。

use super::sys::*;
use super::util::*;
use std::ffi::c_void;

struct Clipboard;

impl Clipboard {
    /// 其他程式可能暫時占用剪貼簿，短暫重試。
    fn open(owner: HWND) -> Result<Clipboard, String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Clipboard);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err("剪貼簿正被其他程式使用，請稍後再試。".into())
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

unsafe fn global_from_bytes(bytes: &[u8]) -> Result<HGLOBAL, String> {
    let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len().max(1));
    if h == 0 {
        return Err("記憶體不足。".into());
    }
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        GlobalFree(h);
        return Err("記憶體不足。".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
    GlobalUnlock(h);
    Ok(h)
}

unsafe fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    let h = global_from_bytes(bytes)?;
    if SetClipboardData(format, h) == 0 {
        GlobalFree(h);
        return Err(last_error_text());
    }
    Ok(())
}

fn drop_effect_format() -> u32 {
    let name = wide("Preferred DropEffect");
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

pub fn set_text(owner: HWND, text: &str) -> Result<(), String> {
    let _cb = Clipboard::open(owner)?;
    let bytes: Vec<u8> = wide(text).iter().flat_map(|u| u.to_le_bytes()).collect();
    unsafe {
        EmptyClipboard();
        put(CF_UNICODETEXT, &bytes)
    }
}

pub fn set_files(owner: HWND, paths: &[String], cut: bool) -> Result<(), String> {
    let _cb = Clipboard::open(owner)?;
    let header = DROPFILES { pFiles: std::mem::size_of::<DROPFILES>() as u32, pt: POINT::default(), fNC: 0, fWide: 1 };
    let mut bytes: Vec<u8> = Vec::new();
    let hp = &header as *const DROPFILES as *const u8;
    bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(hp, std::mem::size_of::<DROPFILES>()) });
    for p in paths {
        for u in p.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes.extend_from_slice(&[0, 0]);
    let effect: u32 = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
    unsafe {
        EmptyClipboard();
        put(CF_HDROP, &bytes)?;
        put(drop_effect_format(), &effect.to_le_bytes())?;
    }
    Ok(())
}

/// 「剪下」的檔案貼上（移動）之後：剪貼簿若還是同一批剪下的檔案就清空，與檔案總管相同。
/// 否則再按一次貼上會對已經搬走的來源再移動一次，每一項都失敗。
pub fn clear_cut_files(owner: HWND, moved: &[String]) {
    let Ok(Some((files, true))) = get_files(owner) else { return };
    let same = files.len() == moved.len()
        && files.iter().all(|f| moved.iter().any(|m| feather_core::paths::same_path(f, m)));
    if !same {
        return;
    }
    if let Ok(_cb) = Clipboard::open(owner) {
        unsafe {
            EmptyClipboard();
        }
    }
}

pub fn has_files() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_HDROP) != 0 }
}

/// 讀取剪貼簿中的檔案清單；第二個值表示是否為「剪下」（移動）。
pub fn get_files(owner: HWND) -> Result<Option<(Vec<String>, bool)>, String> {
    if !has_files() {
        return Ok(None);
    }
    let _cb = Clipboard::open(owner)?;
    unsafe {
        let h = GetClipboardData(CF_HDROP);
        if h == 0 {
            return Ok(None);
        }
        // 注意：剪貼簿擁有這個 HDROP，不可呼叫 DragFinish。
        let count = DragQueryFileW(h, 0xFFFF_FFFF, std::ptr::null_mut(), 0);
        let mut files = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0);
            let mut buf = vec![0u16; len as usize + 1];
            DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
            files.push(from_wide(&buf));
        }
        let mut moving = false;
        let he = GetClipboardData(drop_effect_format());
        if he != 0 && GlobalSize(he) >= 4 {
            let p = GlobalLock(he) as *const c_void as *const u32;
            if !p.is_null() {
                moving = std::ptr::read_unaligned(p) & DROPEFFECT_MOVE != 0;
                GlobalUnlock(he);
            }
        }
        Ok(Some((files, moving)))
    }
}

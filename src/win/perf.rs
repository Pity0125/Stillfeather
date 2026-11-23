//! `FeatherFiles.exe --perf [報告路徑] [範例檔案…]`：不開視窗，在這台電腦上量測 Stillfeather 實際使用的
//! Windows 程式路徑——資料夾列舉（含排序）、側欄子資料夾、檔名搜尋、清單圖示、快速預覽——
//! 並把結果寫成 UTF-8 文字檔（預設是 exe 旁的 perf-core.txt）。由 perf.ps1 呼叫。
//!
//! 只讀取，不修改任何使用者的檔案；預覽用的範例圖片與文字檔產生在暫存資料夾，量完就刪除。

use super::fs::{self, SearchScope};
use super::icons::Icons;
use super::preview;
use super::sys::*;
use feather_core::paths;
use std::fmt::Write as _;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// 預覽影像的邊長（Stillfeather 在一般大小的視窗中置中預覽時的上限）
const PREVIEW_EDGE: i32 = 1100;

struct Report {
    text: String,
}

impl Report {
    fn section(&mut self, title: &str) {
        let _ = writeln!(self.text, "\n## {title}");
    }

    fn line(&mut self, label: &str, value: impl std::fmt::Display) {
        let _ = writeln!(self.text, "{label:<34} {value}");
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

pub fn run(args: &[String]) -> i32 {
    let out_path = args.first().cloned().unwrap_or_else(|| {
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("perf-core.txt")));
        exe.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "perf-core.txt".into())
    });
    let samples: Vec<String> = args.iter().skip(1).filter(|a| !a.trim().is_empty()).cloned().collect();
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let mut r = Report { text: String::new() };
    let total = Instant::now();
    let _ = writeln!(
        r.text,
        "Stillfeather {} · Windows 程式路徑量測（--perf）· 處理器執行緒 {}",
        env!("CARGO_PKG_VERSION"),
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    );

    let listing = folders(&mut r);
    tree(&mut r);
    search(&mut r);
    icons(&mut r, &listing);
    previews(&mut r, &samples);

    let _ = writeln!(r.text, "\n（--perf 全部量測共 {:.1} 秒）", total.elapsed().as_secs_f64());
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(r.text.replace('\n', "\r\n").as_bytes());
    match std::fs::write(&out_path, bytes) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// 常見資料夾讀三次：第一次（可能需要讀磁碟）、之後取中位數（系統已快取）。回傳最大的那個清單給圖示量測用。
fn folders(r: &mut Report) -> Vec<feather_core::entry::Entry> {
    r.section("資料夾讀取（列舉＋依名稱排序；第一次／再讀一次）");
    let mut list: Vec<(String, String)> = vec![("使用者資料夾".into(), fs::profile_dir())];
    for (label, id) in [
        ("桌面", fs::FOLDERID_DESKTOP),
        ("文件", fs::FOLDERID_DOCUMENTS),
        ("下載", fs::FOLDERID_DOWNLOADS),
        ("圖片", fs::FOLDERID_PICTURES),
    ] {
        if let Some(p) = fs::known_folder(&id) {
            list.push((label.into(), p));
        }
    }
    if let Ok(p) = std::env::var("OneDrive") {
        list.push(("OneDrive".into(), p));
    }
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    list.push(("Windows".into(), windows.clone()));
    list.push(("System32".into(), paths::join(&windows, "System32")));
    list.push(("WinSxS（很大的資料夾）".into(), paths::join(&windows, "WinSxS")));
    list.push(("DriverStore\\FileRepository".into(), paths::join(&windows, "System32\\DriverStore\\FileRepository")));
    if let Ok(p) = std::env::var("ProgramFiles") {
        list.push(("Program Files".into(), p));
    }
    let cancel = AtomicBool::new(false);
    let mut biggest = Vec::new();
    for (label, path) in list {
        if !fs::is_dir(&path) {
            continue;
        }
        let mut times = Vec::new();
        let mut count = 0usize;
        let mut error = None;
        for _ in 0..3 {
            let t = Instant::now();
            match fs::read_dir(&path, false, 0, false, false, &cancel) {
                Ok(l) => {
                    times.push(ms(t));
                    count = l.entries.len();
                    if l.entries.len() > biggest.len() {
                        biggest = l.entries;
                    }
                }
                Err(fs::FsError::Message(e)) => {
                    error = Some(e);
                    break;
                }
                Err(fs::FsError::Cancelled) => break,
            }
        }
        match error {
            Some(e) => r.line(&label, format!("無法讀取：{e}")),
            None if !times.is_empty() => {
                let first = times[0];
                let warm = median(times[1..].to_vec());
                let per = if count > 0 { warm * 1000.0 / count as f64 } else { 0.0 };
                r.line(&label, format!("{count:>7} 項   {first:>8.1} ms ／ {warm:>8.1} ms   （每項 {per:.1} µs）"));
            }
            None => {}
        }
    }
    biggest
}

fn tree(r: &mut Report) {
    r.section("側欄展開（只列子資料夾）");
    let cancel = AtomicBool::new(false);
    for path in ["C:\\".to_string(), fs::profile_dir()] {
        let t = Instant::now();
        match fs::child_folders(&path, false, &cancel) {
            Ok(l) => r.line(&path, format!("{:>5} 個資料夾   {:>8.1} ms", l.folders.len(), ms(t))),
            Err(fs::FsError::Message(e)) => r.line(&path, format!("無法讀取：{e}")),
            Err(fs::FsError::Cancelled) => {}
        }
    }
}

fn search(r: &mut Report) {
    r.section("檔名搜尋（含子資料夾，多執行緒）");
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let cases = [("使用者資料夾找「minecraft」", fs::profile_dir(), "minecraft"), ("Windows 資料夾找「notepad」", windows, "notepad")];
    // 每個搜尋跑兩次：第一次可能要從磁碟讀資料夾內容，第二次是系統已快取時的速度
    for (round, (label, root, query)) in cases.iter().flat_map(|c| [(1, c.clone()), (2, c.clone())]) {
        let label = format!("{label}（{}）", if round == 1 { "第一次" } else { "再一次" });
        let label = label.as_str();
        let root = root.clone();
        let cancel = AtomicBool::new(false);
        let t = Instant::now();
        let mut first_batch: Option<f64> = None;
        let mut found = 0usize;
        let result = fs::search(&SearchScope::Folder(root), query, true, false, &cancel, |entries, _, _| {
            if !entries.is_empty() && first_batch.is_none() {
                first_batch = Some(ms(t));
            }
            found += entries.len();
        });
        let total = ms(t);
        match result {
            Ok(s) => {
                let rate = s.scanned as f64 / (total / 1000.0).max(0.001);
                let first = first_batch.map(|f| format!("，第一批結果 {f:.0} ms")).unwrap_or_default();
                r.line(
                    label,
                    format!("{total:>8.0} ms   檢查 {} 項（每秒 {:.0} 項），找到 {found}，略過 {} 個資料夾{first}", s.scanned, rate, s.skipped),
                );
            }
            Err(fs::FsError::Message(e)) => r.line(label, format!("無法搜尋：{e}")),
            Err(fs::FsError::Cancelled) => {}
        }
    }
}

fn icons(r: &mut Report, entries: &[feather_core::entry::Entry]) {
    r.section("清單圖示（依副檔名向系統取一次，之後查快取）");
    if entries.is_empty() {
        r.line("（沒有可用的資料夾清單）", "");
        return;
    }
    let mut icons = Icons::new(96, false);
    let t = Instant::now();
    let mut distinct = std::collections::HashSet::new();
    for e in entries {
        icons.index_for(e);
        distinct.insert(if e.is_dir { "\\dir".to_string() } else { e.extension() });
    }
    let cold = ms(t);
    let t = Instant::now();
    for e in entries {
        icons.index_for(e);
    }
    let warm = ms(t);
    r.line("第一次（含向系統取圖示）", format!("{:>8.1} ms   {} 項、{} 種圖示（每種 {:.2} ms）", cold, entries.len(), distinct.len(), cold / distinct.len().max(1) as f64));
    r.line("之後（快取）", format!("{:>8.1} ms   每列 {:.2} µs", warm, warm * 1000.0 / entries.len() as f64));
    icons.destroy();
}

fn previews(r: &mut Report, samples: &[String]) {
    r.section("快速預覽（第一次／Windows 已有縮圖／Stillfeather 快取）");
    let dir = std::env::temp_dir().join(format!("FeatherFiles-perf-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let mut files: Vec<(String, String)> = Vec::new();
    // 產生的範例：1 MiB 程式碼、1920×1080 圖片
    let code = dir.join("範例程式碼.rs");
    let unit = "/// 說明\npub fn compute(items: &[u32], factor: f64) -> Option<String> {\n    let total: u64 = items.iter().map(|x| *x as u64).sum(); // 加總\n    if total > 0x1F_FFFF { return None; }\n    Some(format!(\"{} {:.2}\", total, factor * 1.5e3))\n}\n";
    if std::fs::write(&code, unit.repeat(1024 * 1024 / unit.len() + 1)).is_ok() {
        files.push(("1 MiB Rust 程式碼".into(), code.to_string_lossy().into_owned()));
    }
    let bmp = dir.join("範例圖片.bmp");
    if std::fs::write(&bmp, sample_bmp(1920, 1080)).is_ok() {
        files.push(("1920×1080 BMP 圖片".into(), bmp.to_string_lossy().into_owned()));
    }
    for s in samples {
        if fs::is_file(s) {
            files.push((paths::file_name(s).to_string(), s.clone()));
        }
    }
    for (label, path) in files {
        let (first, what) = preview::perf_load(&path, PREVIEW_EDGE, false);
        let (again, _) = preview::perf_load(&path, PREVIEW_EDGE, false);
        let (cached, _) = preview::perf_load(&path, PREVIEW_EDGE, true);
        r.line(&label, format!("{first:>7.0} ms ／ {again:>6.0} ms ／ {cached:>5.1} ms   {what}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 24 位元、由下而上的 BMP：漸層加上格線，讓縮圖程式有實際內容可以處理。
fn sample_bmp(w: u32, h: u32) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut b = Vec::with_capacity(size as usize);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(row * h).to_le_bytes());
    b.extend_from_slice(&2835u32.to_le_bytes());
    b.extend_from_slice(&2835u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    for y in 0..h {
        for x in 0..w {
            let grid = if x % 120 == 0 || y % 120 == 0 { 60 } else { 0 };
            let r = (x * 255 / w) as u8;
            let g = (y * 255 / h) as u8;
            let bl = (200u32.saturating_sub(grid)) as u8;
            b.extend_from_slice(&[bl, g, r]);
        }
        b.resize(b.len() + (row - w * 3) as usize, 0);
    }
    b
}

#[cfg(test)]
mod tests {
    #[test]
    fn bmp_layout() {
        let b = super::sample_bmp(3, 2);
        // 每列 9 位元組補到 12，兩列；檔頭 54
        assert_eq!(b.len(), 54 + 12 * 2);
        assert_eq!(&b[0..2], b"BM");
        assert_eq!(u32::from_le_bytes([b[2], b[3], b[4], b[5]]) as usize, b.len());
    }
}

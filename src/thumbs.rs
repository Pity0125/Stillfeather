//! 檔案清單的縮圖：哪些檔案取縮圖、相片資料夾的判斷、縮圖工作程序的傳輸格式、像素整理、快取與置中。
//! Windows 端（工作程序、佇列、在清單上畫出來）在 `src/win/thumbs.rs`。
//!
//! 縮圖只在圖示類的檢視（超大／大／中圖示、並排、內容）顯示，和檔案總管相同。

use crate::entry::Entry;
use crate::paths;
use std::collections::HashMap;
use std::hash::Hash;

/// 圖片（包括相機的 RAW 檔；系統沒有對應的解碼器時取不到縮圖，就照常顯示圖示）
const IMAGE: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "ico", "webp", "heic", "heif", "avif", "jxl", "jxr", "wdp", "hdp", "dds", "tga",
    "arw", "cr2", "cr3", "crw", "dng", "nef", "nrw", "orf", "pef", "raf", "rw2", "sr2", "srw", "x3f", "3fr", "erf", "kdc", "mef", "mos", "mrw", "rwl", "iiq",
];
const VIDEO: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "avi", "wmv", "webm", "mpg", "mpeg", "mpe", "m2ts", "mts", "ts", "3gp", "3g2", "flv", "asf", "vob", "ogv", "divx",
];
/// 其他常有縮圖處理程式的格式（裝了 Acrobat、Photoshop、Illustrator 或 PowerToys 等才有）
const OTHER: &[&str] = &["pdf", "svg", "psd", "ai", "eps"];

/// 同一個副檔名連續這麼多次取不到、一次都沒成功，就認定這台電腦沒有它的縮圖處理程式，這次執行不再試
pub const EXT_GIVE_UP: u32 = 3;

pub fn is_media_ext(ext: &str) -> bool {
    IMAGE.contains(&ext) || VIDEO.contains(&ext)
}

/// 這種檔案要不要試著取縮圖
pub fn wants_ext(ext: &str) -> bool {
    is_media_ext(ext) || OTHER.contains(&ext)
}

pub fn wants(entry: &Entry) -> bool {
    !entry.is_dir && wants_ext(&entry.extension())
}

/// 相片資料夾：圖片與影片至少佔全部項目（含子資料夾）的一半。檔案總管會把這種資料夾當成「圖片」類型，預設用大圖示。
pub fn media_folder(entries: &[Entry]) -> bool {
    let media = entries.iter().filter(|e| !e.is_dir && is_media_ext(&e.extension())).count();
    media > 0 && media * 2 >= entries.len()
}

/// 快取的鍵：檔案改過（修改時間或大小不同）就是另一張縮圖
pub fn key(entry: &Entry, edge: i32) -> (String, i32) {
    (format!("{}|{}|{}", entry.full_path().to_lowercase(), entry.modified, entry.size), edge)
}

// ------------------------------------------------------------ 工作程序的傳輸格式

/// 工作程序的參數
pub const WORKER_ARG: &str = "--thumb-worker";
/// 回應的開頭
pub const MAGIC: &[u8; 4] = b"FFT1";
/// 回應的標頭長度：開頭 4、編號 8、寬 4、高 4
pub const HEADER_LEN: usize = 20;
/// 縮圖邊長上限（超大圖示在 300% 縮放時是 768）
pub const MAX_EDGE: i32 = 1024;

/// 一個要求一行："<編號>\t<邊長>\t<路徑>"。Windows 的路徑不會有 Tab 或換行。
pub fn encode_request(id: u64, edge: i32, path: &str) -> Option<String> {
    if path.contains(['\t', '\n', '\r']) {
        return None;
    }
    Some(format!("{id}\t{edge}\t{path}\n"))
}

pub fn parse_request(line: &str) -> Option<(u64, i32, &str)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut parts = line.splitn(3, '\t');
    let id = parts.next()?.parse().ok()?;
    let edge: i32 = parts.next()?.parse().ok()?;
    let path = parts.next()?;
    ((16..=MAX_EDGE).contains(&edge) && !path.is_empty()).then_some((id, edge, path))
}

/// 回應標頭；取不到縮圖時寬高都是 0，後面沒有像素
pub fn encode_header(id: u64, width: u32, height: u32) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..4].copy_from_slice(MAGIC);
    h[4..12].copy_from_slice(&id.to_le_bytes());
    h[12..16].copy_from_slice(&width.to_le_bytes());
    h[16..20].copy_from_slice(&height.to_le_bytes());
    h
}

/// (編號, 寬, 高)；格式不對或尺寸超過上限時回傳 None
pub fn parse_header(h: &[u8]) -> Option<(u64, u32, u32)> {
    if h.len() < HEADER_LEN || &h[..4] != MAGIC {
        return None;
    }
    let id = u64::from_le_bytes(h[4..12].try_into().ok()?);
    let w = u32::from_le_bytes(h[12..16].try_into().ok()?);
    let hgt = u32::from_le_bytes(h[16..20].try_into().ok()?);
    let max = 2 * MAX_EDGE as u32;
    ((w == 0 && hgt == 0) || (w >= 1 && hgt >= 1 && w <= max && hgt <= max)).then_some((id, w, hgt))
}

// ------------------------------------------------------------ 像素

/// 整理成可以直接畫的 BGRA：沒有透明資訊（Alpha 全為 0，例如 JPEG）的當成不透明；
/// 有透明的轉成預乘（AlphaBlend 需要）。回傳是否有透明的部分（沒有的話用一般的複製就好）。
pub fn prepare_pixels(px: &mut [u32]) -> bool {
    let any_alpha = px.iter().any(|p| p >> 24 != 0);
    if !any_alpha || px.iter().all(|p| p >> 24 == 0xFF) {
        for p in px.iter_mut() {
            *p |= 0xFF00_0000;
        }
        return false;
    }
    let premultiplied = px.iter().all(|p| {
        let a = p >> 24;
        (p & 0xFF) <= a && ((p >> 8) & 0xFF) <= a && ((p >> 16) & 0xFF) <= a
    });
    if !premultiplied {
        for p in px.iter_mut() {
            let a = *p >> 24;
            let m = |c: u32| c * a / 255;
            *p = (a << 24) | m(*p & 0xFF) | (m((*p >> 8) & 0xFF) << 8) | (m((*p >> 16) & 0xFF) << 16);
        }
    }
    true
}

/// 把 `w`×`h` 的縮圖放進邊長 `edge` 的方格（不放大）：水平置中；`bottom` 時貼齊方格底部（圖示檢視，靠近名稱），
/// 否則垂直置中。回傳 (x, y, 寬, 高)，相對於方格左上角。
pub fn place(w: i32, h: i32, edge: i32, bottom: bool) -> (i32, i32, i32, i32) {
    if w <= 0 || h <= 0 || edge <= 0 {
        return (0, 0, 0, 0);
    }
    let scale = (edge as f64 / w.max(h) as f64).min(1.0);
    let dw = ((w as f64 * scale).round() as i32).max(1);
    let dh = ((h as f64 * scale).round() as i32).max(1);
    let x = (edge - dw) / 2;
    let y = if bottom { edge - dh } else { (edge - dh) / 2 };
    (x, y, dw, dh)
}

// ------------------------------------------------------------ 快取

/// 依位元組與數量淘汰最久沒用的快取（縮圖有幾百張，所以用雜湊表；淘汰時才掃一遍找最舊的）。
pub struct Cache<K, V> {
    map: HashMap<K, (V, usize, u64)>,
    bytes: usize,
    tick: u64,
    max_bytes: usize,
    max_items: usize,
}

impl<K: Eq + Hash + Clone, V> Cache<K, V> {
    pub fn new(max_bytes: usize, max_items: usize) -> Self {
        Cache { map: HashMap::new(), bytes: 0, tick: 0, max_bytes, max_items }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// 取用並標示為最近用過
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        let item = self.map.get_mut(key)?;
        item.2 = tick;
        Some(&item.0)
    }

    /// 放入（同一個鍵會取代舊的）；空間不夠時先丟掉最久沒用的。單一項目超過整個預算就不保存。
    pub fn put(&mut self, key: K, value: V, bytes: usize) {
        if let Some((_, old, _)) = self.map.remove(&key) {
            self.bytes -= old;
        }
        if bytes > self.max_bytes || self.max_items == 0 {
            return;
        }
        while !self.map.is_empty() && (self.bytes + bytes > self.max_bytes || self.map.len() >= self.max_items) {
            let oldest = self.map.iter().min_by_key(|(_, (_, _, t))| *t).map(|(k, _)| k.clone());
            match oldest.and_then(|k| self.map.remove(&k)) {
                Some((_, b, _)) => self.bytes -= b,
                None => break,
            }
        }
        self.tick += 1;
        self.bytes += bytes;
        self.map.insert(key, (value, bytes, self.tick));
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

/// 每個副檔名取縮圖的成敗：同一個副檔名連續失敗 [`EXT_GIVE_UP`] 次、一次都沒成功就不再試
#[derive(Default)]
pub struct ExtStats {
    map: HashMap<String, (u32, u32)>,
}

impl ExtStats {
    pub fn record(&mut self, ext: &str, ok: bool) {
        let e = self.map.entry(ext.to_string()).or_default();
        if ok {
            e.0 += 1;
        } else {
            e.1 += 1;
        }
    }

    pub fn given_up(&self, ext: &str) -> bool {
        self.map.get(ext).is_some_and(|(ok, fail)| *ok == 0 && *fail >= EXT_GIVE_UP)
    }
}

/// 路徑的小寫副檔名（快取鍵與統計用）
pub fn ext_of(path: &str) -> String {
    paths::extension_lower(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry { name: name.into(), dir: Arc::from("C:\\Photos"), is_dir, size: 100, modified: 5, is_link: false }
    }

    #[test]
    fn which_files() {
        for name in ["a.JPG", "b.heic", "c.cr3", "d.mp4", "e.MOV", "f.pdf", "g.psd"] {
            assert!(wants(&entry(name, false)), "{name}");
        }
        for name in ["a.txt", "b.docx", "c.mp3", "noext"] {
            assert!(!wants(&entry(name, false)), "{name}");
        }
        assert!(!wants(&entry("photos.jpg", true)), "資料夾不取縮圖");
        assert!(is_media_ext("png") && !is_media_ext("pdf"));
    }

    #[test]
    fn photo_folders() {
        let v = |names: &[(&str, bool)]| names.iter().map(|(n, d)| entry(n, *d)).collect::<Vec<_>>();
        // 使用者的「圖片」：4 個資料夾、11 個檔案大多是 PNG
        let mut pics = v(&[("Camera Roll", true), ("Screenshots", true), ("Overwolf", true), ("Lightroom", true)]);
        for i in 0..9 {
            pics.push(entry(&format!("{i}.png"), false));
        }
        pics.push(entry("a.ico", false));
        pics.push(entry("b.ico", false));
        assert!(media_folder(&pics));
        assert!(media_folder(&v(&[("one.jpg", false)])));
        assert!(media_folder(&v(&[("a.jpg", false), ("notes.txt", false)])), "剛好一半");
        assert!(!media_folder(&v(&[("a.jpg", false), ("b.txt", false), ("c.txt", false)])));
        assert!(!media_folder(&v(&[("a", true), ("b", true), ("c.jpg", false)])), "大多是子資料夾");
        assert!(!media_folder(&v(&[("report.pdf", false)])), "PDF 不算相片");
        assert!(!media_folder(&[]));
    }

    #[test]
    fn cache_keys_change_with_the_file() {
        let a = entry("A.jpg", false);
        let mut b = a.clone();
        assert_eq!(key(&a, 96), key(&b, 96));
        assert_eq!(key(&a, 96).0, key(&entry("a.JPG", false), 96).0, "不分大小寫");
        assert_ne!(key(&a, 96), key(&a, 48));
        b.modified = 6;
        assert_ne!(key(&a, 96), key(&b, 96));
        b.modified = 5;
        b.size = 101;
        assert_ne!(key(&a, 96), key(&b, 96));
    }

    #[test]
    fn requests_roundtrip() {
        let line = encode_request(42, 144, r"C:\Users\王小明\Pictures\相片 1.jpg").unwrap();
        assert_eq!(parse_request(&line), Some((42, 144, r"C:\Users\王小明\Pictures\相片 1.jpg")));
        assert_eq!(parse_request("7\t96\t\\\\nas\\共用\\a.png\r\n"), Some((7, 96, "\\\\nas\\共用\\a.png")));
        assert!(encode_request(1, 96, "bad\tname").is_none());
        assert!(parse_request("x\t96\tC:\\a.jpg").is_none());
        assert!(parse_request("1\t5000\tC:\\a.jpg").is_none(), "邊長超過上限");
        assert!(parse_request("1\t96\t").is_none());
        assert!(parse_request("1\t96").is_none());
    }

    #[test]
    fn headers_roundtrip() {
        assert_eq!(parse_header(&encode_header(9, 256, 171)), Some((9, 256, 171)));
        assert_eq!(parse_header(&encode_header(9, 0, 0)), Some((9, 0, 0)), "取不到縮圖");
        assert!(parse_header(&encode_header(9, 0, 10)).is_none());
        assert!(parse_header(&encode_header(9, 5000, 10)).is_none());
        let mut bad = encode_header(1, 2, 2);
        bad[0] = b'X';
        assert!(parse_header(&bad).is_none());
        assert!(parse_header(&bad[..10]).is_none());
    }

    #[test]
    fn pixels() {
        // JPEG：Alpha 全為 0 → 不透明
        let mut jpeg = vec![0x0012_3456, 0x00AB_CDEF];
        assert!(!prepare_pixels(&mut jpeg));
        assert_eq!(jpeg, vec![0xFF12_3456, 0xFFAB_CDEF]);
        // 全部不透明
        let mut opaque = vec![0xFF00_00FF];
        assert!(!prepare_pixels(&mut opaque));
        // 沒有預乘的半透明：轉成預乘
        let mut png = vec![0x80FF_FFFF, 0xFF00_0000];
        assert!(prepare_pixels(&mut png));
        assert_eq!(png[0], 0x8080_8080);
        // 已經預乘的維持原樣
        let mut pre = vec![0x8040_4040, 0x0000_0000];
        assert!(prepare_pixels(&mut pre));
        assert_eq!(pre, vec![0x8040_4040, 0]);
    }

    #[test]
    fn placing() {
        // 橫的相片放進 96 的方格：寬滿、貼底
        assert_eq!(place(96, 64, 96, true), (0, 32, 96, 64));
        // 垂直置中
        assert_eq!(place(96, 64, 96, false), (0, 16, 96, 64));
        // 直的相片：高滿、水平置中
        assert_eq!(place(54, 96, 96, true), (21, 0, 54, 96));
        // 小圖不放大
        assert_eq!(place(16, 16, 96, true), (40, 80, 16, 16));
        // 比方格大（系統回傳較大的縮圖）：縮小
        assert_eq!(place(200, 100, 100, false), (0, 25, 100, 50));
        assert_eq!(place(0, 10, 96, true), (0, 0, 0, 0));
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut c: Cache<u32, &str> = Cache::new(300, 10);
        c.put(1, "a", 100);
        c.put(2, "b", 100);
        c.put(3, "c", 100);
        assert_eq!(c.get(&1), Some(&"a"));
        c.put(4, "d", 100);
        assert!(c.contains(&1), "剛用過的留著");
        assert!(!c.contains(&2), "最久沒用的先丟");
        assert_eq!(c.bytes(), 300);
        // 同一個鍵取代
        c.put(4, "dd", 50);
        assert_eq!(c.bytes(), 250);
        assert_eq!(c.get(&4), Some(&"dd"));
        // 太大的不保存
        c.put(9, "huge", 1000);
        assert!(!c.contains(&9));
        // 數量上限
        let mut c: Cache<u32, ()> = Cache::new(1000, 2);
        c.put(1, (), 1);
        c.put(2, (), 1);
        c.put(3, (), 1);
        assert_eq!(c.len(), 2);
        assert!(!c.contains(&1));
        c.clear();
        assert!(c.is_empty() && c.bytes() == 0);
    }

    #[test]
    fn extensions_without_handlers_are_given_up() {
        let mut s = ExtStats::default();
        for _ in 0..EXT_GIVE_UP - 1 {
            s.record("pdf", false);
        }
        assert!(!s.given_up("pdf"));
        s.record("pdf", false);
        assert!(s.given_up("pdf"));
        // 成功過一次就不放棄（個別檔案損壞）
        s.record("jpg", true);
        for _ in 0..10 {
            s.record("jpg", false);
        }
        assert!(!s.given_up("jpg"));
        assert_eq!(ext_of(r"C:\a\B.PDF"), "pdf");
    }
}

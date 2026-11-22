//! 唯讀的 ZIP 讀取（Office Open XML 與 OpenDocument 文件都是 ZIP）。只用標準函式庫。
//!
//! 讀中央目錄找檔案，支援不壓縮與 DEFLATE；加密或 ZIP64 的檔案回傳錯誤。
//! 每個檔案解壓後的大小有上限（防止壓縮炸彈），任何不合法的輸入都回傳錯誤，不會 panic。

use crate::inflate;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    compressed: usize,
    pub size: usize,
    offset: usize,
}

pub struct Zip<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
/// 舊版 Office（.doc／.xls／.ppt）與加密的 Office 文件都是 OLE 複合文件
pub const OLE_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

impl<'a> Zip<'a> {
    pub fn open(data: &'a [u8]) -> Result<Zip<'a>, String> {
        if data.starts_with(&OLE_SIGNATURE) {
            return Err("ole".into());
        }
        // 中央目錄結尾記錄在檔案最後（後面可能有最多 65535 位元組的註解）
        let min = data.len().saturating_sub(22 + 65535);
        let mut eocd = None;
        let mut i = data.len().saturating_sub(22);
        loop {
            if u32_at(data, i) == Some(EOCD) {
                eocd = Some(i);
                break;
            }
            if i == min || i == 0 {
                break;
            }
            i -= 1;
        }
        let e = eocd.ok_or("不是 ZIP 格式（找不到中央目錄）。")?;
        let count = u16_at(data, e + 10).ok_or("ZIP 中央目錄損壞。")? as usize;
        let cd_offset = u32_at(data, e + 16).ok_or("ZIP 中央目錄損壞。")? as usize;
        if count == 0xFFFF || cd_offset == 0xFFFF_FFFF {
            return Err("不支援 ZIP64 格式的大型檔案。".into());
        }
        let mut entries = Vec::with_capacity(count.min(10_000));
        let mut p = cd_offset;
        for _ in 0..count.min(100_000) {
            if u32_at(data, p) != Some(CENTRAL) {
                return Err("ZIP 中央目錄損壞。".into());
            }
            let flags = u16_at(data, p + 8).ok_or("ZIP 中央目錄損壞。")?;
            let method = u16_at(data, p + 10).ok_or("ZIP 中央目錄損壞。")?;
            let compressed = u32_at(data, p + 20).ok_or("ZIP 中央目錄損壞。")? as usize;
            let size = u32_at(data, p + 24).ok_or("ZIP 中央目錄損壞。")? as usize;
            let name_len = u16_at(data, p + 28).ok_or("ZIP 中央目錄損壞。")? as usize;
            let extra_len = u16_at(data, p + 30).ok_or("ZIP 中央目錄損壞。")? as usize;
            let comment_len = u16_at(data, p + 32).ok_or("ZIP 中央目錄損壞。")? as usize;
            let offset = u32_at(data, p + 42).ok_or("ZIP 中央目錄損壞。")? as usize;
            let name = data.get(p + 46..p + 46 + name_len).ok_or("ZIP 中央目錄損壞。")?;
            entries.push(Entry { name: String::from_utf8_lossy(name).replace('\\', "/"), method, flags, compressed, size, offset });
            p += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { data, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 依名稱找檔案（先完全相同，再不分大小寫；開頭的 / 不算）
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries.iter().find(|e| e.name == name).or_else(|| self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.find(name).is_some()
    }

    /// 讀出檔案內容；解壓後超過 `limit` 位元組回傳錯誤。
    pub fn read(&self, name: &str, limit: usize) -> Result<Vec<u8>, String> {
        let (e, raw) = self.raw(name)?;
        if e.size > limit {
            return Err("文件內容過大。".into());
        }
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => inflate::inflate_with_hint(raw, limit, e.size),
            m => Err(format!("不支援的壓縮方式（{m}）。")),
        }
    }

    /// 讀成文字（UTF-8，去掉開頭的 BOM）
    pub fn read_string(&self, name: &str, limit: usize) -> Result<String, String> {
        let bytes = self.read(name, limit)?;
        Ok(to_text(bytes))
    }

    /// 只讀出前面 `max` 位元組（大型的 XML 只需要開頭）。回傳文字與是否完整；
    /// 從中間切斷時，最後一個不完整的字元會變成替代字元。
    pub fn read_string_prefix(&self, name: &str, max: usize) -> Result<(String, bool), String> {
        let (e, raw) = self.raw(name)?;
        let (bytes, complete) = match e.method {
            0 => (raw[..raw.len().min(max)].to_vec(), raw.len() <= max),
            8 => inflate::inflate_prefix(raw, max, e.size)?,
            m => return Err(format!("不支援的壓縮方式（{m}）。")),
        };
        Ok((to_text(bytes), complete))
    }

    /// 項目與它的原始（壓縮過的）資料
    fn raw(&self, name: &str) -> Result<(&Entry, &'a [u8]), String> {
        let e = self.find(name).ok_or_else(|| format!("文件裡缺少 {name}。"))?;
        if e.flags & 1 != 0 {
            return Err("文件已加密。".into());
        }
        let d = self.data;
        if u32_at(d, e.offset) != Some(LOCAL) {
            return Err("ZIP 檔案資料損壞。".into());
        }
        let name_len = u16_at(d, e.offset + 26).ok_or("ZIP 檔案資料損壞。")? as usize;
        let extra_len = u16_at(d, e.offset + 28).ok_or("ZIP 檔案資料損壞。")? as usize;
        let start = e.offset + 30 + name_len + extra_len;
        let raw = d.get(start..start.checked_add(e.compressed).ok_or("ZIP 檔案資料損壞。")?).ok_or("ZIP 檔案資料損壞。")?;
        Ok((e, raw))
    }
}

/// UTF-8 轉文字（不合法的位元組換成替代字元），去掉開頭的 BOM；已經是合法 UTF-8 時不複製
fn to_text(bytes: Vec<u8>) -> String {
    let s = String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
    match s.strip_prefix('\u{FEFF}') {
        Some(rest) => rest.to_string(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_and_deflated_entries() {
        let data = include_bytes!("../testdata/sample.zip");
        let z = Zip::open(data).unwrap();
        assert_eq!(z.read_string("hello.txt", 1 << 20).unwrap(), "hello stored");
        let (head, complete) = z.read_string_prefix("dir/big.txt", 100).unwrap();
        assert!(!complete && head.starts_with("line 0 預覽\n") && head.len() <= 100);
        let (head, complete) = z.read_string_prefix("hello.txt", 5).unwrap();
        assert_eq!((head.as_str(), complete), ("hello", false));
        assert!(z.read_string_prefix("hello.txt", 100).unwrap().1);
        let big = z.read_string("dir/big.txt", 1 << 20).unwrap();
        assert!(big.starts_with("line 0 預覽\n") && big.lines().count() == 2000);
        assert_eq!(z.read_string("/DIR/BIG.TXT", 1 << 20).unwrap(), big);
        assert_eq!(z.read_string("中文名稱.xml", 1 << 20).unwrap(), "<a>甲乙丙</a>");
        assert!(z.read("missing.txt", 100).is_err());
        assert!(z.read("dir/big.txt", 100).is_err(), "超過上限");
        assert!(z.contains("dir/") || z.entries().iter().any(|e| e.name.starts_with("dir")));
    }

    #[test]
    fn rejects_non_zip_and_ole() {
        assert!(Zip::open(b"").is_err());
        assert!(Zip::open(b"hello world, not a zip").is_err());
        let mut ole = OLE_SIGNATURE.to_vec();
        ole.extend_from_slice(&[0; 600]);
        assert_eq!(Zip::open(&ole).err().as_deref(), Some("ole"));
    }

    #[test]
    fn corrupted_zip_never_panics() {
        let data = include_bytes!("../testdata/sample.zip").to_vec();
        let mut x: u32 = 99;
        for round in 0..400 {
            let mut d = data.clone();
            for _ in 0..(1 + round % 8) {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                let i = (x as usize) % d.len();
                d[i] = (x >> 24) as u8;
            }
            if let Ok(z) = Zip::open(&d) {
                for e in z.entries().to_vec() {
                    let _ = z.read(&e.name, 1 << 20);
                }
            }
            let cut = (x as usize) % data.len();
            let _ = Zip::open(&data[..cut]);
        }
    }
}

//! 檔名比對（搜尋用）：直接比對 Windows 傳回的 UTF-16 檔名，不分大小寫，不配置記憶體。
//!
//! 大小寫以查表處理：每個 BMP 字元對應到它的小寫（`char::to_lowercase` 只產生一個 BMP 字元時），
//! 其餘（代理字組、會變成多個字元的少數字母）保持原樣。關鍵字與檔名用同一張表，所以結果一致；
//! 一般的英文、全形英文、希臘與西里爾字母都不分大小寫，中日韓文字不受影響。

use std::sync::OnceLock;

static FOLD: OnceLock<Box<[u16]>> = OnceLock::new();

fn table() -> &'static [u16] {
    FOLD.get_or_init(|| {
        (0u32..=0xFFFF)
            .map(|c| {
                let Some(ch) = char::from_u32(c) else { return c as u16 };
                let mut lower = ch.to_lowercase();
                match (lower.next(), lower.next()) {
                    (Some(l), None) if (l as u32) <= 0xFFFF => l as u32 as u16,
                    _ => c as u16,
                }
            })
            .collect()
    })
}

/// 轉成比對用的小寫（單一 UTF-16 單位）。
#[inline]
pub fn fold(c: u16) -> u16 {
    if c < 0x80 {
        // ASCII 最常見，不查表
        if (b'A' as u16..=b'Z' as u16).contains(&c) {
            c + 32
        } else {
            c
        }
    } else {
        table()[c as usize]
    }
}

/// 要找的關鍵字（已轉成比對用的形式）
pub struct Needle {
    units: Vec<u16>,
}

impl Needle {
    /// 前後空白不算；空字串回傳 None。
    pub fn new(query: &str) -> Option<Needle> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        // 先建表，免得多個搜尋執行緒同時第一次查表
        let _ = table();
        Some(Needle { units: q.encode_utf16().map(fold).collect() })
    }

    /// `name` 是否包含關鍵字（不分大小寫）。`name` 不含結尾的 0。
    pub fn matches(&self, name: &[u16]) -> bool {
        let n = &self.units;
        if n.len() > name.len() {
            return false;
        }
        let first = n[0];
        let last_start = name.len() - n.len();
        let mut i = 0;
        while i <= last_start {
            if fold(name[i]) == first && name[i + 1..i + n.len()].iter().zip(&n[1..]).all(|(a, b)| fold(*a) == *b) {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// Windows 檔名緩衝區（以 0 結尾）中實際的檔名部分。
pub fn until_nul(buf: &[u16]) -> &[u16] {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    &buf[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(query: &str, name: &str) -> bool {
        let w: Vec<u16> = name.encode_utf16().collect();
        Needle::new(query).unwrap().matches(&w)
    }

    #[test]
    fn ascii_case_insensitive() {
        assert!(m("report", "Q3 REPORT final.xlsx"));
        assert!(m("RePoRt", "report.docx"));
        assert!(m(".PDF", "a.pdf"));
        assert!(!m("reports", "report.pdf"));
        assert!(m("a", "a"));
        assert!(!m("ab", "a"));
    }

    #[test]
    fn unicode() {
        assert!(m("報告", "2024 年度報告.pdf"));
        assert!(!m("報表", "2024 年度報告.pdf"));
        // 全形英文、希臘、西里爾、帶重音的拉丁字母
        assert!(m("ａｂｃ", "ＡＢＣ.txt"));
        assert!(m("σοφία", "ΣΟΦΊΑ.txt"));
        assert!(m("привет", "ПРИВЕТ.doc"));
        assert!(m("über", "Über uns.md"));
        // 表情符號（代理字組）照原樣比對
        assert!(m("😀", "smile 😀.png"));
        assert!(!m("😁", "smile 😀.png"));
    }

    #[test]
    fn trims_and_rejects_empty() {
        assert!(Needle::new("   ").is_none());
        assert!(m("  notes  ", "my notes.txt"));
    }

    #[test]
    fn agrees_with_to_lowercase_on_common_text() {
        let names = ["README.md", "Cargo.TOML", "ÉCOLE", "Straße", "İstanbul", "ǅ", "ΑΒΓ", "日本語テキスト", "MixedCase_File-01.TXT"];
        for name in names {
            for q in ["e", "readme", "TOML", "école", "straße", "abc", "ΒΓ", "テキスト", "file-01"] {
                let expected = name.to_lowercase().contains(&q.to_lowercase());
                // 只檢查小寫後長度不變的情況（查表只做一對一的對應）
                if name.to_lowercase().encode_utf16().count() == name.encode_utf16().count() && q.to_lowercase().encode_utf16().count() == q.encode_utf16().count() {
                    assert_eq!(m(q, name), expected, "{q} in {name}");
                }
            }
        }
    }

    #[test]
    fn nul_terminated() {
        let mut buf = [0u16; 8];
        for (i, c) in "abc".encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(until_nul(&buf), &[97, 98, 99]);
        assert_eq!(until_nul(&[1, 2]), &[1, 2]);
    }
}

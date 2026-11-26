//! 檔名比對（搜尋用）：直接比對 Windows 傳回的 UTF-16 檔名，不分大小寫，不配置記憶體。
//!
//! - [`Needle`]：檔名「包含」這個詞。
//! - [`Pattern`]：一個搜尋詞。有 `*`、`?` 時是萬用字元，要比對整個檔名（同檔案總管、Everything：
//!   `*.pdf` 是結尾為 .pdf 的檔名，`報告*` 是開頭為「報告」的檔名）；沒有時就是「包含」。
//! - [`all_match`]：多個詞都要有，順序不限（`報告 2025`）。
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

const STAR: u16 = b'*' as u16;
const QUESTION: u16 = b'?' as u16;

/// 一個搜尋詞：包含，或（有 `*`、`?` 時）比對整個檔名的萬用字元
pub enum Pattern {
    Contains(Needle),
    /// 已轉成比對用的小寫；`*` 是任意長度（含 0），`?` 是任一個字元
    Wildcard(Vec<u16>),
}

impl Pattern {
    /// 前後空白不算；空字串回傳 None。
    pub fn new(text: &str) -> Option<Pattern> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        if t.contains(['*', '?']) {
            let _ = table();
            // 連續的 * 等於一個
            let mut units: Vec<u16> = Vec::new();
            for c in t.encode_utf16().map(fold) {
                if !(c == STAR && units.last() == Some(&STAR)) {
                    units.push(c);
                }
            }
            Some(Pattern::Wildcard(units))
        } else {
            Needle::new(t).map(Pattern::Contains)
        }
    }

    pub fn is_wildcard(&self) -> bool {
        matches!(self, Pattern::Wildcard(_))
    }

    /// `name` 不含結尾的 0
    pub fn matches(&self, name: &[u16]) -> bool {
        match self {
            Pattern::Contains(n) => n.matches(name),
            Pattern::Wildcard(p) => wildcard(p, name),
        }
    }
}

/// 所有的詞都要有（順序不限）；沒有詞時一律符合
pub fn all_match(patterns: &[Pattern], name: &[u16]) -> bool {
    patterns.iter().all(|p| p.matches(name))
}

/// 萬用字元比對整個檔名（不分大小寫）。回溯只記最後一個 `*` 的位置，不配置記憶體，最壞 O(m·n)。
fn wildcard(p: &[u16], name: &[u16]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ni < name.len() {
        if pi < p.len() && p[pi] == STAR {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && (p[pi] == QUESTION || p[pi] == fold(name[ni])) {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            // 讓上一個 * 多吃一個字元再試
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == STAR)
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

    fn w(pattern: &str, name: &str) -> bool {
        let n: Vec<u16> = name.encode_utf16().collect();
        Pattern::new(pattern).unwrap().matches(&n)
    }

    #[test]
    fn wildcards_match_the_whole_name() {
        assert!(w("*.pdf", "年度報告.PDF"));
        assert!(!w("*.pdf", "a.pdf.txt"));
        assert!(w("報告*", "報告 2025.docx"));
        assert!(!w("報告*", "年度報告.docx"));
        assert!(w("*報告*", "年度報告.docx"));
        assert!(w("a?c.txt", "ABC.txt"));
        assert!(!w("a?c.txt", "ac.txt"));
        assert!(w("*", ""));
        assert!(w("**a**", "banana"));
        assert!(w("*a*a*a*", "banana"));
        assert!(!w("*a*a*a*a*", "banana"));
        assert!(w("img_???.jpg", "IMG_007.JPG"));
        // 沒有萬用字元：照舊是「包含」
        assert!(w("port", "Report.pdf"));
        assert!(!Pattern::new("port").unwrap().is_wildcard());
        assert!(Pattern::new(" ").is_none());
    }

    #[test]
    fn every_term_must_match_in_any_order() {
        let terms: Vec<Pattern> = ["報告", "2025"].iter().filter_map(|t| Pattern::new(t)).collect();
        let name = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert!(all_match(&terms, &name("2025 年度報告.pdf")));
        assert!(all_match(&terms, &name("報告-final-2025.docx")));
        assert!(!all_match(&terms, &name("2024 年度報告.pdf")));
        assert!(all_match(&[], &name("anything")));
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

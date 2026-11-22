//! 清單排序：資料夾永遠在前，名稱採與檔案總管相近的自然排序（file2 < file10）。

use crate::entry::Entry;
use std::cmp::Ordering;

pub const COL_NAME: usize = 0;
pub const COL_SIZE: usize = 1;
pub const COL_TYPE: usize = 2;
pub const COL_MODIFIED: usize = 3;
pub const COL_LOCATION: usize = 4;

fn lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

/// 不分大小寫的自然排序；完全相同時再以逐字比較決定，確保結果穩定。
///
/// 不配置記憶體：排序一萬個項目要比較十幾萬次，舊版每次比較都把兩個名稱轉成字元陣列、
/// 數字段再各建一個字串，十萬個項目要半秒以上。這裡直接在 UTF-8 位元組上比較，
/// ASCII 走快速路徑，其他字元才解碼；結果與舊版完全相同（見測試）。
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    while i < ab.len() && j < bb.len() {
        let (x, y) = (ab[i], bb[j]);
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let si = i;
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let ta = trim_leading_zeros(&ab[si..i]);
            let tb = trim_leading_zeros(&bb[sj..j]);
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if ord != Ordering::Equal {
                return ord;
            }
            continue;
        }
        if x < 0x80 && y < 0x80 {
            let ord = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
            continue;
        }
        // 非 ASCII：解出完整字元再比較（i、j 一定落在字元邊界）
        let (Some(cx), Some(cy)) = (a[i..].chars().next(), b[j..].chars().next()) else { break };
        let ord = lower(cx).cmp(&lower(cy));
        if ord != Ordering::Equal {
            return ord;
        }
        i += cx.len_utf8();
        j += cy.len_utf8();
    }
    a[i..].chars().count().cmp(&b[j..].chars().count()).then_with(|| a.cmp(b))
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let n = digits.iter().take_while(|d| **d == b'0').count();
    &digits[n..]
}

/// 不分大小寫比較（與比較兩者的小寫字串相同），ASCII 不必解碼、不配置記憶體。
fn cmp_ignore_case(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.bytes().map(|c| c.to_ascii_lowercase()).cmp(b.bytes().map(|c| c.to_ascii_lowercase()));
    }
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

/// 類型欄比較用的副檔名（不含句點、不轉小寫，不配置記憶體）；資料夾與沒有副檔名時為空字串。
/// 與 Entry::extension 取同一段文字。
fn raw_extension(e: &Entry) -> &str {
    if e.is_dir {
        return "";
    }
    match e.name.rfind('.') {
        Some(i) if i + 1 < e.name.len() => &e.name[i + 1..],
        _ => "",
    }
}

pub fn compare(a: &Entry, b: &Entry, column: usize, descending: bool) -> Ordering {
    if a.is_dir != b.is_dir {
        return if a.is_dir { Ordering::Less } else { Ordering::Greater };
    }
    let primary = match column {
        COL_SIZE => a.size().cmp(&b.size()),
        // 不分大小寫比較副檔名，不為每次比較建立小寫字串
        COL_TYPE => cmp_ignore_case(raw_extension(a), raw_extension(b)),
        COL_MODIFIED => a.modified.cmp(&b.modified),
        COL_LOCATION => natural_cmp(&a.dir, &b.dir),
        _ => Ordering::Equal,
    };
    let ord = primary.then_with(|| natural_cmp(&a.name, &b.name));
    if descending {
        ord.reverse()
    } else {
        ord
    }
}

pub fn sort_entries(entries: &mut [Entry], column: usize, descending: bool) {
    entries.sort_by(|a, b| compare(a, b, column, descending));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::sample;

    #[test]
    fn natural() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("File2", "file2"), Ordering::Less); // tie-break ordinal
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_ne!(natural_cmp("img007", "img7"), Ordering::Equal); // 數值相同時仍有穩定順序
        assert_eq!(natural_cmp("abc", "abcd"), Ordering::Less);
        assert_eq!(natural_cmp("報告2", "報告10"), Ordering::Less);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
    }

    /// 舊版（會配置記憶體）的實作，用來確認新版的順序完全相同。
    fn reference(a: &str, b: &str) -> Ordering {
        let ac: Vec<char> = a.chars().collect();
        let bc: Vec<char> = b.chars().collect();
        let (mut i, mut j) = (0, 0);
        while i < ac.len() && j < bc.len() {
            let (x, y) = (ac[i], bc[j]);
            if x.is_ascii_digit() && y.is_ascii_digit() {
                let si = i;
                while i < ac.len() && ac[i].is_ascii_digit() {
                    i += 1;
                }
                let sj = j;
                while j < bc.len() && bc[j].is_ascii_digit() {
                    j += 1;
                }
                let na: String = ac[si..i].iter().collect();
                let nb: String = bc[sj..j].iter().collect();
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
                continue;
            }
            let ord = lower(x).cmp(&lower(y));
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
        (ac.len() - i).cmp(&(bc.len() - j)).then_with(|| a.cmp(b))
    }

    #[test]
    fn same_order_as_reference() {
        let alphabet = ['a', 'A', 'z', 'Z', '0', '1', '9', '.', ' ', '_', '-', 'ß', 'İ', 'ı', 'K', 'k', 'É', 'é', '中', '文', '😀', 'Σ', 'ς'];
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let words: Vec<String> = (0..600)
            .map(|_| {
                let n = (next() % 9) as usize;
                (0..n).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect()
            })
            .collect();
        for a in &words {
            for b in &words {
                assert_eq!(natural_cmp(a, b), reference(a, b), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn type_column_ignores_case_without_allocating() {
        let mut v = vec![sample("b.TXT", false, 1, 1), sample("a.png", false, 1, 1), sample("c.txt", false, 1, 1), sample("noext", false, 1, 1)];
        sort_entries(&mut v, COL_TYPE, false);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["noext", "a.png", "b.TXT", "c.txt"]);
    }

    #[test]
    fn folders_first_even_descending() {
        let mut v = vec![
            sample("b.txt", false, 5, 1),
            sample("zdir", true, 0, 9),
            sample("a.txt", false, 50, 2),
            sample("adir", true, 0, 3),
        ];
        sort_entries(&mut v, COL_NAME, false);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["adir", "zdir", "a.txt", "b.txt"]);
        sort_entries(&mut v, COL_SIZE, true);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zdir", "adir", "a.txt", "b.txt"]);
        sort_entries(&mut v, COL_MODIFIED, false);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["adir", "zdir", "b.txt", "a.txt"]);
    }
}

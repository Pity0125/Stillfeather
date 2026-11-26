//! 清單排序：資料夾和檔案分開排，名稱採與檔案總管相近的自然排序（file2 < file10）。
//!
//! 和檔案總管相同：遞增時資料夾在前面，遞減時資料夾在後面（例如依修改日期遞減，最新的檔案排在最上面）。
//! 「資料夾一律排在最前面」（`folders_on_top`，在排序選單）開著時，不論方向資料夾都在前面。

use crate::entry::Entry;
use std::cmp::Ordering;

pub const COL_NAME: usize = 0;
pub const COL_SIZE: usize = 1;
pub const COL_TYPE: usize = 2;
pub const COL_MODIFIED: usize = 3;
pub const COL_LOCATION: usize = 4;
/// 搜尋結果的相關性（不是清單的欄位；分數見 `searchquery::Scorer`，和項目分開存在分頁裡）
pub const COL_RELEVANCE: usize = 5;

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

pub fn compare(a: &Entry, b: &Entry, column: usize, descending: bool, folders_on_top: bool) -> Ordering {
    if a.is_dir != b.is_dir {
        let folders_first = if a.is_dir { Ordering::Less } else { Ordering::Greater };
        return if descending && !folders_on_top { folders_first.reverse() } else { folders_first };
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

/// 從選單選一個排序欄位時先用的方向：修改日期、大小從新的、大的開始（遞減），名稱、類型、所在位置遞增
pub fn default_desc(column: usize) -> bool {
    matches!(column, COL_SIZE | COL_MODIFIED)
}

pub fn sort_entries(entries: &mut [Entry], column: usize, descending: bool, folders_on_top: bool) {
    entries.sort_by(|a, b| compare(a, b, column, descending, folders_on_top));
}

/// 帶相關性分數比較（搜尋結果）。相關性：分數小的在前（等級、路徑深度），同分時新的在前，再比名稱；
/// 不分資料夾與檔案（同檔案總管），「資料夾一律排在最前面」開著時才把資料夾放前面。其他欄位同 [`compare`]。
pub fn compare_scored(a: (&Entry, u32), b: (&Entry, u32), column: usize, descending: bool, folders_on_top: bool) -> Ordering {
    if column != COL_RELEVANCE {
        return compare(a.0, b.0, column, descending, folders_on_top);
    }
    if folders_on_top && a.0.is_dir != b.0.is_dir {
        return if a.0.is_dir { Ordering::Less } else { Ordering::Greater };
    }
    a.1.cmp(&b.1).then_with(|| b.0.modified.cmp(&a.0.modified)).then_with(|| natural_cmp(&a.0.name, &b.0.name)).then_with(|| natural_cmp(&a.0.dir, &b.0.dir))
}

/// 排序項目與和它對齊的分數（搜尋結果）。`scores` 和 `entries` 不一樣長時（沒有分數）只排項目。
pub fn sort_scored(entries: &mut Vec<Entry>, scores: &mut Vec<u32>, column: usize, descending: bool, folders_on_top: bool) {
    if scores.len() != entries.len() {
        scores.clear();
        sort_entries(entries, if column == COL_RELEVANCE { COL_NAME } else { column }, descending, folders_on_top);
        return;
    }
    let mut pairs: Vec<(Entry, u32)> = entries.drain(..).zip(scores.drain(..)).collect();
    pairs.sort_by(|a, b| compare_scored((&a.0, a.1), (&b.0, b.1), column, descending, folders_on_top));
    for (e, s) in pairs {
        entries.push(e);
        scores.push(s);
    }
}

/// 把一批新的搜尋結果併進已經排好的結果（同一個順序），不必整個重排：先排好這一批，再一次合併。
pub fn merge_scored(entries: &mut Vec<Entry>, scores: &mut Vec<u32>, batch: Vec<Entry>, batch_scores: Vec<u32>, column: usize, descending: bool, folders_on_top: bool) {
    if batch.is_empty() {
        return;
    }
    if scores.len() != entries.len() || batch_scores.len() != batch.len() {
        entries.extend(batch);
        scores.clear();
        sort_scored(entries, scores, column, descending, folders_on_top);
        return;
    }
    let mut new: Vec<(Entry, u32)> = batch.into_iter().zip(batch_scores).collect();
    new.sort_by(|a, b| compare_scored((&a.0, a.1), (&b.0, b.1), column, descending, folders_on_top));
    let old: Vec<(Entry, u32)> = entries.drain(..).zip(scores.drain(..)).collect();
    entries.reserve(old.len() + new.len());
    scores.reserve(old.len() + new.len());
    let mut a = old.into_iter().peekable();
    let mut b = new.into_iter().peekable();
    loop {
        // 同分時原本的在前：已經看到的項目不會被新的擠下去
        let take_old = match (a.peek(), b.peek()) {
            (Some(x), Some(y)) => compare_scored((&x.0, x.1), (&y.0, y.1), column, descending, folders_on_top) != Ordering::Greater,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        let (e, s) = if take_old { a.next() } else { b.next() }.expect("peeked");
        entries.push(e);
        scores.push(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevance_then_newer_then_name() {
        let mut entries = vec![sample("c.txt", false, 1, 300), sample("b.txt", false, 1, 100), sample("dir", true, 0, 500), sample("a.txt", false, 1, 100)];
        let mut scores = vec![5, 1, 9, 1];
        sort_scored(&mut entries, &mut scores, COL_RELEVANCE, false, false);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt", "c.txt", "dir"]);
        assert_eq!(scores, [1, 1, 5, 9]);
        // 資料夾一律排在最前面
        sort_scored(&mut entries, &mut scores, COL_RELEVANCE, false, true);
        assert_eq!(entries[0].name, "dir");
        assert_eq!(scores[0], 9);
        // 換成名稱：分數跟著項目走
        sort_scored(&mut entries, &mut scores, COL_NAME, false, false);
        let pairs: Vec<(&str, u32)> = entries.iter().map(|e| e.name.as_str()).zip(scores.iter().copied()).collect();
        assert_eq!(pairs, [("dir", 9), ("a.txt", 1), ("b.txt", 1), ("c.txt", 5)]);
    }

    #[test]
    fn merging_batches_keeps_the_order() {
        let mut entries = Vec::new();
        let mut scores = Vec::new();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for batch in 0..20 {
            let n = (next() % 40) as usize;
            let items: Vec<Entry> = (0..n).map(|i| sample(&format!("f{batch}_{i}.txt"), next() % 5 == 0, next() % 1000, next() % 50)).collect();
            let s: Vec<u32> = (0..n).map(|_| (next() % 4) as u32).collect();
            merge_scored(&mut entries, &mut scores, items, s, COL_RELEVANCE, false, false);
            assert_eq!(entries.len(), scores.len());
            for i in 1..entries.len() {
                assert_ne!(compare_scored((&entries[i - 1], scores[i - 1]), (&entries[i], scores[i]), COL_RELEVANCE, false, false), Ordering::Greater);
            }
        }
        // 依大小合併也一樣
        let mut by_size = entries.clone();
        let mut size_scores = scores.clone();
        sort_scored(&mut by_size, &mut size_scores, COL_SIZE, true, false);
        merge_scored(&mut by_size, &mut size_scores, vec![sample("new.bin", false, 999_999, 0)], vec![0], COL_SIZE, true, false);
        // 遞減時資料夾在後面，最大的檔案在最前面
        assert_eq!(by_size[0].name, "new.bin");
        assert_eq!(size_scores[0], 0);
    }

    #[test]
    fn default_directions() {
        assert!(!default_desc(COL_NAME));
        assert!(default_desc(COL_SIZE));
        assert!(!default_desc(COL_TYPE));
        assert!(default_desc(COL_MODIFIED));
        assert!(!default_desc(COL_LOCATION));
    }
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
        sort_entries(&mut v, COL_TYPE, false, false);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["noext", "a.png", "b.TXT", "c.txt"]);
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn folders_follow_the_sort_direction() {
        let mut v = vec![
            sample("b.txt", false, 5, 1),
            sample("zdir", true, 0, 9),
            sample("a.txt", false, 50, 2),
            sample("adir", true, 0, 3),
        ];
        // 遞增：資料夾在前面（和檔案總管相同）
        sort_entries(&mut v, COL_NAME, false, false);
        assert_eq!(names(&v), ["adir", "zdir", "a.txt", "b.txt"]);
        sort_entries(&mut v, COL_MODIFIED, false, false);
        assert_eq!(names(&v), ["adir", "zdir", "b.txt", "a.txt"]);
        // 遞減：整個反過來，資料夾在後面（最新、最大的檔案在最上面）
        sort_entries(&mut v, COL_NAME, true, false);
        assert_eq!(names(&v), ["b.txt", "a.txt", "zdir", "adir"]);
        sort_entries(&mut v, COL_MODIFIED, true, false);
        assert_eq!(names(&v), ["a.txt", "b.txt", "zdir", "adir"]);
        sort_entries(&mut v, COL_SIZE, true, false);
        assert_eq!(names(&v), ["a.txt", "b.txt", "zdir", "adir"]);
    }

    #[test]
    fn folders_on_top_in_both_directions() {
        let mut v = vec![
            sample("b.txt", false, 5, 1),
            sample("zdir", true, 0, 9),
            sample("a.txt", false, 50, 2),
            sample("adir", true, 0, 3),
        ];
        sort_entries(&mut v, COL_NAME, false, true);
        assert_eq!(names(&v), ["adir", "zdir", "a.txt", "b.txt"]);
        sort_entries(&mut v, COL_NAME, true, true);
        assert_eq!(names(&v), ["zdir", "adir", "b.txt", "a.txt"]);
        sort_entries(&mut v, COL_SIZE, true, true);
        assert_eq!(names(&v), ["zdir", "adir", "a.txt", "b.txt"]);
        sort_entries(&mut v, COL_MODIFIED, true, true);
        assert_eq!(names(&v), ["zdir", "adir", "a.txt", "b.txt"]);
    }
}

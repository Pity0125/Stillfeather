//! 清單與樹狀圖的「打字跳到項目」。
//! 取代 Windows 控制項內建的漸進式搜尋：內建版本找不到項目時會發出提示音，這裡找不到就安靜地不動。

/// 連續輸入的間隔超過這個時間（毫秒）就重新開始。
pub const TIMEOUT_MS: f64 = 1000.0;

#[derive(Default)]
pub struct TypeAhead {
    buf: String,
    last: f64,
}

impl TypeAhead {
    /// 加入一個字元，回傳目前累積的搜尋字串。
    pub fn push(&mut self, c: char, now_ms: f64) -> String {
        if now_ms - self.last > TIMEOUT_MS {
            self.buf.clear();
        }
        self.last = now_ms;
        self.buf.extend(c.to_lowercase());
        self.buf.clone()
    }

    /// 空白鍵只在已經開始輸入時才算搜尋字元（否則交給控制項，例如 Ctrl+空白切換選取）。
    pub fn active(&self, now_ms: f64) -> bool {
        !self.buf.is_empty() && now_ms - self.last <= TIMEOUT_MS
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

/// 找下一個名稱以 `query` 開頭的項目（不分大小寫），找不到回傳 None。
/// - 只輸入一個字元，或同一個字元連打（"aaa"）：從目前項目的下一個開始，在以該字元開頭的項目間循環。
/// - 多個不同字元：從目前項目（含）開始找，讓繼續打字時停在同一個項目上。
pub fn find(count: usize, current: Option<usize>, query: &str, name_of: impl Fn(usize) -> Option<String>) -> Option<usize> {
    if count == 0 || query.is_empty() {
        return None;
    }
    let query = query.to_lowercase();
    let mut chars = query.chars();
    let first = chars.next()?;
    let repeated = chars.all(|c| c == first);
    let (needle, start) = if repeated {
        (first.to_string(), current.map(|c| c + 1).unwrap_or(0))
    } else {
        (query.clone(), current.unwrap_or(0))
    };
    (0..count).map(|k| (start + k) % count).find(|&i| name_of(i).is_some_and(|n| n.to_lowercase().starts_with(&needle)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<&'static str> {
        vec!["Apple", "apricot", "Banana", "berry", "Cherry", "數學", "數位"]
    }

    fn find_in(current: Option<usize>, q: &str) -> Option<usize> {
        let n = names();
        find(n.len(), current, q, |i| n.get(i).map(|s| s.to_string()))
    }

    #[test]
    fn single_and_repeated_letters_cycle() {
        assert_eq!(find_in(None, "a"), Some(0));
        assert_eq!(find_in(Some(0), "a"), Some(1));
        assert_eq!(find_in(Some(1), "a"), Some(0));
        assert_eq!(find_in(Some(0), "bb"), Some(2));
        assert_eq!(find_in(Some(2), "bbb"), Some(3));
        assert_eq!(find_in(Some(5), "數"), Some(6));
    }

    #[test]
    fn prefix_stays_on_current_and_misses_quietly() {
        assert_eq!(find_in(Some(0), "ap"), Some(0));
        assert_eq!(find_in(Some(0), "apr"), Some(1));
        assert_eq!(find_in(Some(4), "che"), Some(4));
        assert_eq!(find_in(Some(0), "zz"), None);
        assert_eq!(find_in(Some(0), "q"), None);
        assert_eq!(find(0, None, "a", |_| None), None);
    }

    #[test]
    fn buffer_resets_after_pause() {
        let mut t = TypeAhead::default();
        assert_eq!(t.push('A', 10_000.0), "a");
        assert_eq!(t.push('p', 10_300.0), "ap");
        assert!(t.active(10_400.0));
        assert_eq!(t.push('b', 12_000.0), "b");
        assert!(!t.active(14_000.0));
        t.reset();
        assert!(!t.active(12_000.0));
    }
}

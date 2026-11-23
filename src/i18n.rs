//! 介面語言：繁體中文或英文。
//!
//! 文字直接寫在用到的地方，兩種語言並列：`tr!("開啟", "Open")`、`trf!("找不到 {name}", "Can't find {name}")`
//! （`trf!` 和 `format!` 一樣，可以用 `{name}` 或後面接參數）。啟動時依設定（或 Windows 的顯示語言）呼叫一次
//! [`set_english`]，之後整個程序都用那個語言；改了設定要重新開啟才套用。
//!
//! picker/ 的取代元件以 `#[path]` 引入這個檔案（src/picker.rs 用到 `tr!`），它不設定語言，一律是中文，
//! 反正元件本身沒有畫面。

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// 整個程序改用英文（啟動時呼叫一次）
pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

/// 目前是不是英文介面
pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// 英文的單複數：`count(1, "item", "items")` → `1 item`，`count(3, …)` → `3 items`。中文介面不用它。
pub fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Windows 的介面語言（GetUserDefaultUILanguage 的值）是不是中文：主要語言 0x04（繁體、簡體都算）
pub fn is_chinese_langid(langid: u16) -> bool {
    langid & 0x3FF == 0x04
}

/// 兩種語言的文字選一個：`tr!("開啟", "Open")`
#[macro_export]
macro_rules! tr {
    ($zh:expr, $en:expr $(,)?) => {
        if $crate::i18n::english() {
            $en
        } else {
            $zh
        }
    };
}

/// 兩種語言的 `format!`：`trf!("找不到 {name}", "Can't find {name}")`、`trf!("共 {} 項", "{} items", n)`
#[macro_export]
macro_rules! trf {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::english() {
            format!($en $(, $arg)*)
        } else {
            format!($zh $(, $arg)*)
        }
    };
}

/// 英文分單複數的 `trf!`：`trn!(n; "{} 個項目", "{} item", "{} items", n)`。三個格式要用同樣的參數。
#[macro_export]
macro_rules! trn {
    ($n:expr; $zh:literal, $one:literal, $many:literal $(, $arg:expr)* $(,)?) => {
        if !$crate::i18n::english() {
            format!($zh $(, $arg)*)
        } else if $n == 1 {
            format!($one $(, $arg)*)
        } else {
            format!($many $(, $arg)*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert!(is_chinese_langid(0x0404)); // zh-TW
        assert!(is_chinese_langid(0x0804)); // zh-CN
        assert!(is_chinese_langid(0x0C04)); // zh-HK
        assert!(!is_chinese_langid(0x0409)); // en-US
        assert!(!is_chinese_langid(0x0411)); // ja-JP
        assert_eq!(count(1, "item", "items"), "1 item");
        assert_eq!(count(0, "item", "items"), "0 items");
        let n = 2;
        // 測試時不切換全域的語言（其他測試同時在跑），只確認兩種寫法都能展開
        assert_eq!(trf!("{n} 個", "{n} items"), "2 個");
        assert_eq!(tr!("開啟", "Open"), "開啟");
        assert_eq!(trn!(n; "{} 個項目", "{} item", "{} items", n), "2 個項目");
    }
}

//! 清單裡「每個檔案各自有圖示」的檔案類型（捷徑、程式、圖示檔…）。
//!
//! 一般檔案的圖示只看副檔名，同一種副檔名共用一個（見 `src/win/icons.rs`）。這些類型不行：
//! 每個捷徑顯示它自己指定的圖示或目標程式的圖示，每個 .exe 有自己的圖示。圖示要用檔案的完整路徑
//! 向 Windows 取，快取也要以檔案（路徑、修改時間、大小）區分，不能把不同的 .lnk 當成同一個。
//! Windows 端（背景載入、加進清單的影像清單）在 `src/win/fileicons.rs`。

use crate::entry::Entry;

/// 圖示由檔案本身決定的副檔名（登錄的 DefaultIcon 是 "%1"，或有自己的圖示處理程式）
const PER_FILE: &[&str] = &["lnk", "url", "pif", "appref-ms", "website", "exe", "scr", "cpl", "ico", "cur", "ani"];
/// 捷徑類：圖示加上捷徑箭頭（和檔案總管相同）
const SHORTCUTS: &[&str] = &["lnk", "url", "pif", "appref-ms", "website"];

/// 一次執行最多記住幾個檔案的圖示（每個佔清單影像清單的一格）；超過時其餘的顯示副檔名的圖示
pub const MAX_FILE_ICONS: usize = 4096;

/// 這種副檔名的圖示是不是每個檔案各自的
pub fn per_file(ext: &str) -> bool {
    PER_FILE.contains(&ext)
}

/// 捷徑類的副檔名（圖示要加捷徑箭頭）
pub fn is_shortcut(ext: &str) -> bool {
    SHORTCUTS.contains(&ext)
}

/// 這個項目要不要用它自己的圖示
pub fn wants(entry: &Entry) -> bool {
    !entry.is_dir && per_file(&entry.extension())
}

/// 快取的鍵：完整路徑（不分大小寫）加上修改時間與大小。不同的捷徑是不同的鍵；
/// 捷徑改過（換了圖示或目標）時修改時間不同，也會重新取。
pub fn key(entry: &Entry) -> String {
    format!("{}|{}|{}", entry.full_path().to_lowercase(), entry.modified, entry.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::sample;

    #[test]
    fn which_files_have_their_own_icon() {
        for ext in ["lnk", "url", "exe", "ico", "appref-ms"] {
            assert!(per_file(ext), "{ext}");
        }
        for ext in ["txt", "docx", "dll", "png", ""] {
            assert!(!per_file(ext), "{ext}");
        }
        assert!(wants(&sample("藍色星原：旅謠.lnk", false, 834, 1)));
        assert!(wants(&sample("Setup.EXE", false, 1, 1)));
        // 資料夾名稱剛好像捷徑也不算
        assert!(!wants(&sample("links.lnk", true, 0, 1)));
        assert!(is_shortcut("lnk") && is_shortcut("url"));
        assert!(!is_shortcut("exe") && !is_shortcut("ico"));
    }

    #[test]
    fn each_shortcut_is_its_own_cache_entry() {
        let a = sample("藍色星原：旅謠.lnk", false, 834, 100);
        let b = sample("記事本.lnk", false, 834, 100);
        assert_ne!(key(&a), key(&b));
        // 同一個捷徑：路徑大小寫不同仍是同一個
        let a2 = sample("藍色星原：旅謠.LNK", false, 834, 100);
        assert_eq!(key(&a), key(&a2));
        // 改過（修改時間或大小不同）就重新取
        assert_ne!(key(&a), key(&sample("藍色星原：旅謠.lnk", false, 834, 101)));
        assert_ne!(key(&a), key(&sample("藍色星原：旅謠.lnk", false, 900, 100)));
        assert!(key(&a).starts_with("c:\\t\\藍色星原：旅謠.lnk|"));
    }
}

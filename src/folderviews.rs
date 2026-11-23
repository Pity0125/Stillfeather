//! 每個資料夾各自的檢視模式（Rust 版專用，存在設定檔旁的 `folder-views.json`）。
//!
//! - 只記錄和預設檢視不同的資料夾；改回預設就移除。
//! - 路徑不分大小寫，結尾的反斜線不算（`C:\` 這種根目錄除外）。
//! - 最多記住 [`MAX_FOLDERS`] 個，超過時移除最久沒有改過的。
//! - 搜尋結果用 [`SEARCH_KEY`] 另外記一份。
//! - 副檔名、隱藏項目等顯示選項是整體設定，不在這裡。

use crate::json::{self, Value};
use crate::view::ViewMode;
use std::collections::HashMap;

pub const MAX_FOLDERS: usize = 5_000;
/// 搜尋結果的檢視
pub const SEARCH_KEY: &str = "::search";
/// 檔案大小上限（讀取時）
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderViews {
    /// 鍵（小寫）→（原本的路徑、檢視、最後修改的順序）
    map: HashMap<String, (String, ViewMode, u64)>,
    counter: u64,
}

/// 比對用的鍵：不分大小寫，去掉結尾的反斜線（根目錄保留）。
pub fn key(path: &str) -> String {
    let p = path.trim();
    let trimmed = p.trim_end_matches(['\\', '/']);
    let p = if trimmed.len() == 2 && trimmed.ends_with(':') { &p[..3.min(p.len())] } else { trimmed };
    p.to_lowercase()
}

impl FolderViews {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 這個資料夾記住的檢視；沒有記錄時回傳 None（用預設）。
    pub fn get(&self, path: &str) -> Option<ViewMode> {
        if path.trim().is_empty() {
            return None;
        }
        self.map.get(&key(path)).map(|(_, m, _)| *m)
    }

    /// 記下資料夾的檢視；和預設相同就移除記錄。回傳內容是否有變。
    pub fn set(&mut self, path: &str, mode: ViewMode, default: ViewMode) -> bool {
        if path.trim().is_empty() {
            return false;
        }
        let k = key(path);
        if mode == default {
            return self.map.remove(&k).is_some();
        }
        if self.map.get(&k).is_some_and(|(_, m, _)| *m == mode) {
            return false;
        }
        self.counter += 1;
        self.map.insert(k, (path.trim().to_string(), mode, self.counter));
        while self.map.len() > MAX_FOLDERS {
            let oldest = self.map.iter().min_by_key(|(_, (_, _, n))| *n).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => {
                    self.map.remove(&k);
                }
                None => break,
            }
        }
        true
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// 依最後修改的順序（舊到新）
    fn ordered(&self) -> Vec<&(String, ViewMode, u64)> {
        let mut v: Vec<_> = self.map.values().collect();
        v.sort_by_key(|(_, _, n)| *n);
        v
    }

    pub fn to_json(&self) -> String {
        let items: Vec<String> = self
            .ordered()
            .iter()
            .map(|(path, mode, _)| format!("    {{ \"Path\": {}, \"View\": {} }}", json::quote(path), json::quote(mode.as_str())))
            .collect();
        if items.is_empty() {
            "{\n  \"Version\": 1,\n  \"Folders\": []\n}".to_string()
        } else {
            format!("{{\n  \"Version\": 1,\n  \"Folders\": [\n{}\n  ]\n}}", items.join(",\n"))
        }
    }

    /// 讀取記錄。個別無效的項目略過（不讓一筆壞資料清掉全部）；整個檔案格式不對才回傳錯誤。
    pub fn from_json(text: &str) -> Result<FolderViews, String> {
        let root = json::parse(text)?;
        let Some(Value::Arr(items)) = root.get("Folders") else {
            return Err(tr!("資料夾檢視記錄缺少 Folders 清單。", "The folder view record is missing its Folders list.").into());
        };
        let mut views = FolderViews::default();
        for item in items {
            let (Some(Value::Str(path)), Some(Value::Str(view))) = (item.get("Path"), item.get("View")) else { continue };
            let Some(mode) = ViewMode::parse(view) else { continue };
            if path.trim().is_empty() {
                continue;
            }
            views.counter += 1;
            views.map.insert(key(path), (path.trim().to_string(), mode, views.counter));
        }
        while views.map.len() > MAX_FOLDERS {
            let oldest = views.map.iter().min_by_key(|(_, (_, _, n))| *n).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                views.map.remove(&k);
            }
        }
        Ok(views)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ignore_case_and_trailing_slash() {
        assert_eq!(key("C:\\Users\\Pity0\\Desktop\\"), key("c:\\users\\PITY0\\desktop"));
        assert_eq!(key("C:\\"), "c:\\");
        assert_eq!(key("C:"), "c:");
        assert_eq!(key("D:\\資料\\照片"), "d:\\資料\\照片");
        assert_eq!(key("\\\\NAS\\Share\\"), "\\\\nas\\share");
    }

    #[test]
    fn set_get_and_default_removes() {
        let mut v = FolderViews::default();
        assert_eq!(v.get("C:\\Photos"), None);
        assert!(v.set("C:\\Photos", ViewMode::Large, ViewMode::Details));
        assert_eq!(v.get("c:\\photos\\"), Some(ViewMode::Large));
        assert!(!v.set("C:\\PHOTOS", ViewMode::Large, ViewMode::Details));
        assert!(v.set("C:\\Photos", ViewMode::Details, ViewMode::Details));
        assert_eq!(v.get("C:\\Photos"), None);
        assert!(v.is_empty());
        assert!(!v.set("", ViewMode::Large, ViewMode::Details));
        assert!(v.set(SEARCH_KEY, ViewMode::List, ViewMode::Details));
        assert_eq!(v.get(SEARCH_KEY), Some(ViewMode::List));
    }

    #[test]
    fn roundtrip_keeps_order_and_unicode() {
        let mut v = FolderViews::default();
        v.set("C:\\Users\\pity0\\圖片", ViewMode::ExtraLarge, ViewMode::Details);
        v.set("D:\\Music \"Live\"", ViewMode::Tiles, ViewMode::Details);
        v.set("E:\\", ViewMode::List, ViewMode::Details);
        let text = v.to_json();
        let back = FolderViews::from_json(&text).unwrap();
        assert_eq!(back.get("c:\\users\\PITY0\\圖片"), Some(ViewMode::ExtraLarge));
        assert_eq!(back.get("D:\\Music \"Live\""), Some(ViewMode::Tiles));
        assert_eq!(back.get("e:\\"), Some(ViewMode::List));
        assert_eq!(back.to_json(), text);
        assert_eq!(FolderViews::from_json(&FolderViews::default().to_json()).unwrap().len(), 0);
    }

    #[test]
    fn bad_items_are_skipped() {
        let text = r#"{"Folders":[{"Path":"C:\\A","View":"Huge"},{"Path":"C:\\B","View":"List"},{"View":"List"},3]}"#;
        let v = FolderViews::from_json(text).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v.get("C:\\B"), Some(ViewMode::List));
        assert!(FolderViews::from_json("[]").is_err());
        assert!(FolderViews::from_json("not json").is_err());
    }

    #[test]
    fn oldest_is_evicted() {
        let mut v = FolderViews::default();
        for i in 0..MAX_FOLDERS + 3 {
            v.set(&format!("C:\\f{i}"), ViewMode::List, ViewMode::Details);
        }
        assert_eq!(v.len(), MAX_FOLDERS);
        assert_eq!(v.get("C:\\f0"), None);
        assert_eq!(v.get("C:\\f2"), None);
        assert_eq!(v.get("C:\\f3"), Some(ViewMode::List));
        // 重新設定會變成最新的
        v.set("C:\\f3", ViewMode::Tiles, ViewMode::Details);
        v.set("C:\\new", ViewMode::List, ViewMode::Details);
        assert_eq!(v.get("C:\\f3"), Some(ViewMode::Tiles));
        assert_eq!(v.get("C:\\f4"), None);
    }
}

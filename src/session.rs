//! 上次關閉時的分頁（`session.json`，和設定檔放在同一個資料夾）。設定「還原上次的分頁」開著時，
//! 主視窗正常關閉或 Windows 登出、關機時寫入，下次啟動照原樣開回來：左右欄的分頁、各欄選取的分頁、雙欄與作用中的欄。
//!
//! 只記資料夾路徑（不記上一頁、下一頁與搜尋）。啟動時讀完就刪掉、正常關閉時再寫：
//! 某個分頁的資料夾讓 Stillfeather 當掉時，下次不會再開回那個分頁而一直當掉。

use crate::json::{self, Value};
use crate::paths;

/// 每一欄最多還原幾個分頁
pub const MAX_TABS: usize = 50;
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
pub const FILE_NAME: &str = "session.json";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideTabs {
    pub tabs: Vec<String>,
    /// 選取的分頁（`tabs` 的索引）
    pub selected: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub dual: bool,
    /// 作用中的欄：0 左、1 右
    pub active: usize,
    pub left: SideTabs,
    pub right: SideTabs,
}

impl Session {
    /// 沒有可以還原的分頁
    pub fn is_empty(&self) -> bool {
        self.left.tabs.is_empty() && self.right.tabs.is_empty()
    }

    /// 整理成可以直接還原的樣子：去掉不是完整路徑的、超過上限的；左欄空了就把右欄移過去；選取與作用中的欄落在範圍內。
    pub fn sanitize(mut self) -> Session {
        for side in [&mut self.left, &mut self.right] {
            side.tabs.retain(|p| !p.trim().is_empty() && paths::is_fully_qualified(p));
            side.tabs.truncate(MAX_TABS);
            side.selected = side.selected.min(side.tabs.len().saturating_sub(1));
        }
        if self.left.tabs.is_empty() {
            self.left = std::mem::take(&mut self.right);
        }
        if self.right.tabs.is_empty() {
            self.dual = false;
        }
        if !self.dual || self.active > 1 {
            self.active = 0;
        }
        self
    }

    pub fn to_json(&self) -> String {
        let side = |s: &SideTabs| {
            let tabs: Vec<String> = s.tabs.iter().map(|t| format!("      {}", json::quote(t))).collect();
            let list = if tabs.is_empty() { "[]".to_string() } else { format!("[\n{}\n    ]", tabs.join(",\n")) };
            format!("{{\n    \"Tabs\": {list},\n    \"Selected\": {}\n  }}", s.selected)
        };
        format!(
            "{{\n  \"Version\": 1,\n  \"Dual\": {},\n  \"Active\": {},\n  \"Left\": {},\n  \"Right\": {}\n}}",
            self.dual,
            self.active,
            side(&self.left),
            side(&self.right)
        )
    }

    pub fn from_json(text: &str) -> Result<Session, String> {
        if text.len() as u64 > MAX_FILE_BYTES {
            return Err(tr!("分頁記錄超過大小限制。", "The tab record is larger than the size limit.").into());
        }
        let root = json::parse(text)?;
        if !matches!(root, Value::Obj(_)) {
            return Err(tr!("分頁記錄必須是 JSON 物件。", "The tab record must be a JSON object.").into());
        }
        let mut s = Session::default();
        match root.get("Dual") {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => s.dual = *b,
            Some(_) => return Err(tr!("Dual 必須是 true 或 false。", "Dual must be true or false.").into()),
        }
        s.active = index(&root, "Active")?;
        for (key, slot) in [("Left", &mut s.left), ("Right", &mut s.right)] {
            match root.get(key) {
                None | Some(Value::Null) => {}
                Some(v @ Value::Obj(_)) => {
                    match v.get("Tabs") {
                        None | Some(Value::Null) => {}
                        Some(Value::Arr(items)) => {
                            for item in items {
                                match item {
                                    Value::Str(p) => slot.tabs.push(p.clone()),
                                    _ => return Err(tr!("分頁必須是字串陣列。", "Tabs must be an array of strings.").into()),
                                }
                            }
                        }
                        Some(_) => return Err(tr!("分頁必須是字串陣列。", "Tabs must be an array of strings.").into()),
                    }
                    slot.selected = index(v, "Selected")?;
                }
                Some(_) => return Err(trf!("{key} 必須是物件。", "{key} must be an object.")),
            }
        }
        Ok(s.sanitize())
    }
}

fn index(v: &Value, key: &str) -> Result<usize, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(Value::Num(n)) if n.fract() == 0.0 && *n >= 0.0 && *n < 1e6 => Ok(*n as usize),
        Some(_) => Err(trf!("{key} 必須是非負整數。", "{key} must be a non-negative integer.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(tabs: &[&str], selected: usize) -> SideTabs {
        SideTabs { tabs: tabs.iter().map(|s| s.to_string()).collect(), selected }
    }

    #[test]
    fn roundtrip_with_unicode_and_unc() {
        let s = Session {
            dual: true,
            active: 1,
            left: side(&[r"C:\Users\王小明\Pictures", r"D:\"], 1),
            right: side(&[r"\\nas\共用\相片"], 0),
        };
        let text = s.to_json();
        assert!(text.contains("王小明"));
        assert_eq!(Session::from_json(&text).unwrap(), s);
        let empty = Session::default();
        assert_eq!(Session::from_json(&empty.to_json()).unwrap(), empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn sanitize_rules() {
        // 不是完整路徑的去掉，選取跟著落在範圍內
        let s = Session { dual: false, active: 0, left: side(&["relative", r"C:\a", "  "], 5), right: SideTabs::default() }.sanitize();
        assert_eq!(s.left, side(&[r"C:\a"], 0));
        // 左欄空了：右欄移到左欄、回到單欄
        let s = Session { dual: true, active: 1, left: side(&[], 0), right: side(&[r"C:\r1", r"C:\r2"], 1) }.sanitize();
        assert_eq!(s.left, side(&[r"C:\r1", r"C:\r2"], 1));
        assert!(s.right.tabs.is_empty() && !s.dual && s.active == 0);
        // 右欄沒有分頁就不是雙欄
        let s = Session { dual: true, active: 1, left: side(&[r"C:\a"], 0), right: SideTabs::default() }.sanitize();
        assert!(!s.dual && s.active == 0);
        // 單欄時保留右欄分頁（再開雙欄時繼續用），作用中的一定是左欄
        let s = Session { dual: false, active: 1, left: side(&[r"C:\a"], 0), right: side(&[r"C:\b"], 0) }.sanitize();
        assert_eq!(s.right.tabs.len(), 1);
        assert_eq!(s.active, 0);
        // 分頁數上限
        let many: Vec<String> = (0..80).map(|i| format!(r"C:\f{i}")).collect();
        let s = Session { left: SideTabs { tabs: many, selected: 70 }, ..Default::default() }.sanitize();
        assert_eq!(s.left.tabs.len(), MAX_TABS);
        assert_eq!(s.left.selected, MAX_TABS - 1);
    }

    #[test]
    fn rejects_bad_files() {
        for bad in ["[1]", "{\"Dual\":1}", "{\"Active\":-1}", "{\"Left\":[]}", "{\"Left\":{\"Tabs\":[1]}}", "{\"Left\":{\"Selected\":\"x\"}}", "not json"] {
            assert!(Session::from_json(bad).is_err(), "{bad}");
        }
        // 缺少的欄位用預設值
        assert!(Session::from_json("{}").unwrap().is_empty());
        let big = format!("{{\"Left\":{{\"Tabs\":[\"{}\"]}}}}", "x".repeat(MAX_FILE_BYTES as usize));
        assert!(Session::from_json(&big).is_err());
    }
}

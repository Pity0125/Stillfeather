//! 復原（Ctrl+Z）：記錄重新命名、移動與刪除到資源回收筒，並提供從資源回收筒找回項目所需的解析。
//!
//! 刪除到資源回收筒時，Windows 會在該磁碟的 `$Recycle.Bin\<使用者 SID>` 放兩個檔案：
//! `$Rxxxxxx.ext` 是項目本身，`$Ixxxxxx.ext` 記錄原本的完整路徑與刪除時間。
//! 復原刪除時依「原路徑相同、刪除時間落在這次操作期間」找出對應的項目，搬回原位並移除紀錄。

use crate::paths;

/// 最多記住幾個可以復原的操作
pub const MAX_UNDO: usize = 30;

/// 一個已完成、可以復原的操作。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// 重新命名：`from` 改成了 `to`（完整路徑）
    Rename { from: String, to: String },
    /// 移動：每一項是（原本的位置, 移動後的位置）
    Move { items: Vec<(String, String)> },
    /// 刪除到資源回收筒：原本的位置，以及操作期間（FILETIME，100 奈秒單位）
    Recycle { items: Vec<String>, since: u64, until: u64 },
}

impl Action {
    /// 選單與狀態列上的說明，例如「重新命名」「移動 3 個項目」。
    pub fn label(&self) -> String {
        match self {
            Action::Rename { .. } => "重新命名".to_string(),
            Action::Move { items } if items.len() == 1 => "移動".to_string(),
            Action::Move { items } => format!("移動 {} 個項目", items.len()),
            Action::Recycle { items, .. } if items.len() == 1 => "刪除".to_string(),
            Action::Recycle { items, .. } => format!("刪除 {} 個項目", items.len()),
        }
    }
}

/// 復原紀錄：新的在最後，超過上限時丟掉最舊的。
#[derive(Default, Debug)]
pub struct History {
    items: Vec<Action>,
}

impl History {
    pub fn push(&mut self, action: Action) {
        let empty = match &action {
            Action::Rename { .. } => false,
            Action::Move { items } => items.is_empty(),
            Action::Recycle { items, .. } => items.is_empty(),
        };
        if empty {
            return;
        }
        self.items.push(action);
        if self.items.len() > MAX_UNDO {
            let extra = self.items.len() - MAX_UNDO;
            self.items.drain(..extra);
        }
    }

    pub fn pop(&mut self) -> Option<Action> {
        self.items.pop()
    }

    pub fn last(&self) -> Option<&Action> {
        self.items.last()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 移動前後的路徑：每個來源移到 `dest` 底下的同名項目。來源已在 `dest` 裡的略過。
pub fn move_pairs(sources: &[String], dest: &str) -> Vec<(String, String)> {
    sources
        .iter()
        .filter(|s| paths::parent(s).is_none_or(|p| !paths::eq_ignore_case(paths::trim_end_sep(&p), paths::trim_end_sep(dest))))
        .filter(|s| !paths::file_name(s).is_empty())
        .map(|s| (s.clone(), paths::join(dest, paths::file_name(s))))
        .collect()
}

/// 現在時間（FILETIME：自 1601-01-01 起的 100 奈秒數，UTC）。
pub fn filetime_now() -> u64 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (unix.as_secs() + 11_644_473_600) * 10_000_000 + unix.subsec_nanos() as u64 / 100
}

/// 一秒的 FILETIME 單位數
pub const FILETIME_SECOND: u64 = 10_000_000;

/// 資源回收筒中 `$I` 紀錄檔的內容。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleInfo {
    /// 原本的完整路徑
    pub original: String,
    /// 刪除時間（FILETIME）
    pub deleted: u64,
    /// 刪除時的大小（位元組）
    pub size: u64,
}

/// 解析 `$I` 紀錄檔。版本 1（Windows Vista～8.1）路徑固定 260 字元；版本 2（Windows 10 起）先記錄路徑長度。
pub fn parse_recycle_info(bytes: &[u8]) -> Option<RecycleInfo> {
    let u64_at = |i: usize| bytes.get(i..i + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()));
    let version = u64_at(0)?;
    let size = u64_at(8)?;
    let deleted = u64_at(16)?;
    let (start, chars) = match version {
        1 => (24, 260),
        2 => {
            let n = u32::from_le_bytes(bytes.get(24..28)?.try_into().ok()?) as usize;
            (28, n)
        }
        _ => return None,
    };
    let raw = bytes.get(start..start + chars * 2).or_else(|| bytes.get(start..))?;
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    let original = String::from_utf16(&units).ok()?;
    if original.is_empty() {
        return None;
    }
    Some(RecycleInfo { original, deleted, size })
}

/// `$I` 紀錄檔對應的項目名稱（`$IAB12CD.txt` → `$RAB12CD.txt`）。
pub fn data_name_for(info_name: &str) -> Option<String> {
    info_name.strip_prefix("$I").map(|rest| format!("$R{rest}"))
}

/// 在候選紀錄中挑出要還原的那一個：原路徑相同、刪除時間在 [since, until] 之內，取最晚刪除的。
pub fn pick_restore(candidates: &[RecycleInfo], original: &str, since: u64, until: u64) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| paths::eq_ignore_case(paths::trim_end_sep(&c.original), paths::trim_end_sep(original)))
        .filter(|(_, c)| c.deleted >= since && c.deleted <= until)
        .max_by_key(|(_, c)| c.deleted)
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_v2(path: &str, deleted: u64, size: u64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(2u64.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend(deleted.to_le_bytes());
        let units: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        b.extend((units.len() as u32).to_le_bytes());
        for u in units {
            b.extend(u.to_le_bytes());
        }
        b
    }

    fn info_v1(path: &str, deleted: u64) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(1u64.to_le_bytes());
        b.extend(42u64.to_le_bytes());
        b.extend(deleted.to_le_bytes());
        let mut units: Vec<u16> = path.encode_utf16().collect();
        units.resize(260, 0);
        for u in units {
            b.extend(u.to_le_bytes());
        }
        b
    }

    #[test]
    fn parses_both_versions() {
        let v2 = parse_recycle_info(&info_v2(r"C:\Users\pity0\Desktop\報告.docx", 133_000_000_000_000_000, 1234)).unwrap();
        assert_eq!(v2.original, r"C:\Users\pity0\Desktop\報告.docx");
        assert_eq!(v2.deleted, 133_000_000_000_000_000);
        assert_eq!(v2.size, 1234);
        let v1 = parse_recycle_info(&info_v1(r"D:\舊資料", 5)).unwrap();
        assert_eq!(v1.original, r"D:\舊資料");
        assert_eq!(v1.size, 42);
        assert_eq!(parse_recycle_info(&[0u8; 10]), None);
        let mut bad = info_v2("x", 1, 1);
        bad[0] = 9;
        assert_eq!(parse_recycle_info(&bad), None);
        assert_eq!(parse_recycle_info(&info_v2("", 1, 1)), None);
    }

    #[test]
    fn data_names() {
        assert_eq!(data_name_for("$IAB12CD.txt").as_deref(), Some("$RAB12CD.txt"));
        assert_eq!(data_name_for("$IXYZ").as_deref(), Some("$RXYZ"));
        assert_eq!(data_name_for("$RAB12CD.txt"), None);
        assert_eq!(data_name_for("desktop.ini"), None);
    }

    #[test]
    fn picks_newest_within_window() {
        let c = vec![
            RecycleInfo { original: r"C:\a\x.txt".into(), deleted: 100, size: 0 },
            RecycleInfo { original: r"C:\A\X.TXT".into(), deleted: 300, size: 0 },
            RecycleInfo { original: r"C:\a\x.txt".into(), deleted: 900, size: 0 },
            RecycleInfo { original: r"C:\a\y.txt".into(), deleted: 350, size: 0 },
        ];
        assert_eq!(pick_restore(&c, r"C:\a\x.txt", 200, 500), Some(1));
        assert_eq!(pick_restore(&c, r"C:\a\x.txt", 0, 1000), Some(2));
        assert_eq!(pick_restore(&c, r"C:\a\x.txt", 400, 800), None);
        assert_eq!(pick_restore(&c, r"C:\a\z.txt", 0, 1000), None);
    }

    #[test]
    fn move_pairs_skip_same_folder() {
        let src = vec![r"C:\a\x.txt".to_string(), r"C:\b\y".to_string(), r"C:\dest\z.txt".to_string()];
        let pairs = move_pairs(&src, r"C:\dest");
        assert_eq!(pairs, vec![(r"C:\a\x.txt".to_string(), r"C:\dest\x.txt".to_string()), (r"C:\b\y".to_string(), r"C:\dest\y".to_string())]);
        assert_eq!(move_pairs(&src, r"C:\dest\").len(), 2);
    }

    #[test]
    fn history_is_bounded_and_skips_empty() {
        let mut h = History::default();
        h.push(Action::Move { items: vec![] });
        h.push(Action::Recycle { items: vec![], since: 0, until: 0 });
        assert!(h.is_empty());
        for i in 0..(MAX_UNDO + 5) {
            h.push(Action::Rename { from: format!("a{i}"), to: format!("b{i}") });
        }
        assert_eq!(h.len(), MAX_UNDO);
        assert_eq!(h.pop(), Some(Action::Rename { from: format!("a{}", MAX_UNDO + 4), to: format!("b{}", MAX_UNDO + 4) }));
        assert_eq!(h.last().map(|a| a.label()), Some("重新命名".to_string()));
    }

    #[test]
    fn labels() {
        let one = Action::Move { items: vec![("a".into(), "b".into())] };
        assert_eq!(one.label(), "移動");
        let many = Action::Recycle { items: vec!["a".into(), "b".into(), "c".into()], since: 0, until: 0 };
        assert_eq!(many.label(), "刪除 3 個項目");
    }

    #[test]
    fn filetime_is_plausible() {
        // 2020 年之後、2200 年之前
        let now = filetime_now();
        assert!(now > 132_223_104_000_000_000);
        assert!(now < 195_000_000_000_000_000);
    }
}

//! 自動重新整理的純邏輯：顯示中的資料夾有變更時什麼時候重新讀取、
//! 一個變更通知影響哪個資料夾（或資料夾本身改名、搬走），以及讀回來的內容和畫面上的是否相同。
//!
//! 向 Windows 註冊變更通知、在背景重新讀取與更新清單在 `src/win/watch.rs`。

use crate::entry::Entry;
use crate::paths;
use std::collections::{HashMap, HashSet};

/// 收到變更通知後等這麼久再讀：同一批變更（例如解壓縮、一次貼上多個檔案）通常接連發生，一起處理。
pub const SETTLE_MS: u64 = 200;
/// 同一個分頁兩次自動重新讀取至少間隔這麼久：大量複製進行中時，清單約每秒更新一次，不會一直重讀。
pub const MIN_INTERVAL_MS: u64 = 1_000;
/// 正在重新命名、拖放、選單開著或按著滑鼠時，過這麼久再試。
pub const RETRY_MS: u64 = 400;
/// 切回分頁（或雙欄重新顯示右欄）時補讀一次的延遲：等切換的淡入結束，也讓連續切換只讀最後停下的那一頁。
pub const CATCH_UP_MS: u64 = 150;

/// 一個分頁的自動重新讀取排程。時間都是同一個時鐘的毫秒數（由呼叫端提供，方便測試）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    /// 預定重新讀取的時間
    pub due: Option<u64>,
    /// 上一次自動重新讀取開始的時間
    pub last: Option<u64>,
    /// 已經在背景讀取（結果還沒回來）
    pub reading: bool,
}

impl Schedule {
    /// 收到變更通知。已經排定的時間不會因為一直有新的變更而一直延後。
    pub fn changed(&mut self, now: u64) {
        let wait = match self.last {
            Some(last) if now.saturating_sub(last) < MIN_INTERVAL_MS => (last + MIN_INTERVAL_MS).saturating_sub(now).max(SETTLE_MS),
            _ => SETTLE_MS,
        };
        self.request(now + wait);
    }

    /// 盡快補讀一次（分頁重新顯示時，期間的變更沒有收到通知）。
    pub fn catch_up(&mut self, now: u64) {
        self.request(now + CATCH_UP_MS);
    }

    fn request(&mut self, at: u64) {
        self.due = Some(self.due.map_or(at, |d| d.min(at)));
    }

    pub fn is_due(&self, now: u64) -> bool {
        self.due.is_some_and(|d| d <= now)
    }

    /// 現在不方便更新清單：稍後再試。
    pub fn postpone(&mut self, now: u64) {
        self.due = Some(now + RETRY_MS);
    }

    /// 開始在背景讀取。讀取期間再有變更會重新排定，讀完後再讀一次，不會漏掉。
    pub fn start(&mut self, now: u64) {
        self.due = None;
        self.last = Some(now);
        self.reading = true;
    }

    /// 讀取結束（或被使用者的操作取代）。
    pub fn finish(&mut self) {
        self.reading = false;
    }

    /// 不必再讀（分頁已經不顯示、改成搜尋結果等）。
    pub fn cancel(&mut self) {
        self.due = None;
    }
}

/// 所有分頁裡最早要重新讀取的時間（計時器下一次要醒來的時間）。
pub fn next_due<'a>(items: impl IntoIterator<Item = &'a Schedule>) -> Option<u64> {
    items.into_iter().filter_map(|s| s.due).min()
}

/// 一個變更通知對某個顯示中的資料夾的影響。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// 資料夾裡的項目（或資料夾本身的屬性）變了：重新讀取
    Refresh,
    /// 資料夾本身（或它的上層）改名、搬到別處：改顯示新的位置
    Moved(String),
}

/// 變更通知帶來的路徑（項目一、項目二；改名時項目二是新的路徑）對顯示中的 `folder` 有什麼影響。
/// `folder_renamed` 表示這是資料夾改名的通知。路徑以 Windows 的規則比較（不分大小寫）。
pub fn effect(folder: &str, folder_renamed: bool, first: Option<&str>, second: Option<&str>) -> Option<Effect> {
    if folder_renamed {
        if let (Some(old), Some(new)) = (first, second) {
            // 丟進資源回收筒在檔案系統上也是搬移，但對使用者來說是刪除：不跟過去（重新讀取時會發現它不在了）
            if !paths::same_path(old, new) && !in_recycle_bin(new) {
                if let Some(moved) = rebase(folder, old, new) {
                    return Some(Effect::Moved(moved));
                }
            }
        }
    }
    let touches = |p: Option<&str>| p.is_some_and(|p| paths::same_path(p, folder) || paths::parent(p).is_some_and(|d| paths::same_path(&d, folder)));
    (touches(first) || touches(second)).then_some(Effect::Refresh)
}

/// `path` 等於 `old` 或位於其下時，換成 `new` 底下對應的位置；否則回傳 None。
pub fn rebase(path: &str, old: &str, new: &str) -> Option<String> {
    if !paths::is_same_or_descendant(old, path) {
        return None;
    }
    let old = paths::normalize_separators(old);
    let old = paths::trim_end_sep(&old);
    let path = paths::normalize_separators(path);
    // 比較時不分大小寫，大小寫對照不改變字元數，以字元數切開比較保險
    let rest: String = path.chars().skip(old.chars().count()).collect();
    let rest = rest.trim_start_matches('\\');
    let new = paths::normalize_separators(new);
    let base = paths::trim_end_sep(&new);
    Some(if rest.is_empty() { base.to_string() } else { paths::join(base, rest) })
}

/// 路徑位於資源回收筒裡（`$Recycle.Bin`）。
pub fn in_recycle_bin(path: &str) -> bool {
    paths::normalize_separators(path).split('\\').any(|c| paths::eq_ignore_case(c, "$Recycle.Bin"))
}

/// 往上找第一個還存在的資料夾（不含 `path` 本身）：顯示中的資料夾被刪除時改顯示它。
pub fn nearest_existing(path: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let mut p = paths::parent(path)?;
    loop {
        if exists(&p) {
            return Some(p);
        }
        p = paths::parent(&p)?;
    }
}

/// 重新讀取的結果和畫面上的一樣嗎：名稱、資料夾或檔案、大小、修改時間與連結都相同，順序也相同。
/// 相同時不必動清單，畫面完全不閃。
pub fn same_entries(a: &[Entry], b: &[Entry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| x.name == y.name && x.is_dir == y.is_dir && x.size == y.size && x.modified == y.modified && x.is_link == y.is_link)
}

/// 側欄子資料夾更新的一步（依新的順序）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// 保留原本第 i 個節點（連同它已展開的子樹）
    Keep(usize),
    /// 插入新清單的第 j 個資料夾
    Insert(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildPlan {
    /// 要移除的原本節點（索引）
    pub remove: Vec<usize>,
    /// 依新的順序逐一保留或插入
    pub steps: Vec<Step>,
}

impl ChildPlan {
    /// 沒有要移除也沒有要插入的。
    pub fn is_noop(&self) -> bool {
        self.remove.is_empty() && self.steps.iter().all(|s| matches!(s, Step::Keep(_)))
    }
}

/// 側欄某個已展開的資料夾：原本的子資料夾 `old`（畫面順序）要變成 `new`（已排序）。
/// 不在 `new` 裡的移除，其餘保留原本的節點，只插入新出現的，已展開的子資料夾不會被收合。
/// 名稱逐字比較：只改大小寫的改名會換成新的節點，顯示新的名稱。
pub fn plan_children(old: &[&str], new: &[&str]) -> ChildPlan {
    let index: HashMap<&str, usize> = old.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let wanted: HashSet<&str> = new.iter().copied().collect();
    let remove = old.iter().enumerate().filter(|(_, n)| !wanted.contains(*n)).map(|(i, _)| i).collect();
    let steps = new
        .iter()
        .enumerate()
        .map(|(j, n)| match index.get(n) {
            Some(&i) => Step::Keep(i),
            None => Step::Insert(j),
        })
        .collect();
    ChildPlan { remove, steps }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::sample;

    #[test]
    fn first_change_waits_briefly() {
        let mut s = Schedule::default();
        s.changed(1_000);
        assert_eq!(s.due, Some(1_000 + SETTLE_MS));
        assert!(!s.is_due(1_000 + SETTLE_MS - 1));
        assert!(s.is_due(1_000 + SETTLE_MS));
    }

    #[test]
    fn later_changes_do_not_push_the_read_back() {
        let mut s = Schedule::default();
        s.changed(1_000);
        s.changed(1_150);
        s.changed(1_190);
        assert_eq!(s.due, Some(1_000 + SETTLE_MS));
    }

    #[test]
    fn continuous_changes_read_about_once_a_second() {
        let mut s = Schedule::default();
        s.changed(0);
        s.start(SETTLE_MS);
        s.finish();
        // 剛讀完又有變更：等到距離上次讀取滿一秒
        s.changed(SETTLE_MS + 50);
        assert_eq!(s.due, Some(SETTLE_MS + MIN_INTERVAL_MS));
        // 很久沒讀過：照常短暫等待
        let mut t = Schedule { last: Some(0), ..Default::default() };
        t.changed(5_000);
        assert_eq!(t.due, Some(5_000 + SETTLE_MS));
        // 快滿一秒時至少還是等一下，讓同一批變更一起處理
        let mut u = Schedule { last: Some(0), ..Default::default() };
        u.changed(MIN_INTERVAL_MS - 10);
        assert_eq!(u.due, Some(MIN_INTERVAL_MS - 10 + SETTLE_MS));
    }

    #[test]
    fn change_during_read_is_not_lost() {
        let mut s = Schedule::default();
        s.changed(0);
        s.start(200);
        assert!(s.reading);
        assert_eq!(s.due, None);
        s.changed(300);
        assert!(s.due.is_some());
        s.finish();
        assert!(!s.reading);
        assert!(s.due.is_some());
    }

    #[test]
    fn postpone_and_catch_up() {
        let mut s = Schedule::default();
        s.changed(0);
        s.postpone(SETTLE_MS);
        assert_eq!(s.due, Some(SETTLE_MS + RETRY_MS));
        s.catch_up(SETTLE_MS);
        assert_eq!(s.due, Some(SETTLE_MS + CATCH_UP_MS));
        s.cancel();
        assert!(!s.is_due(u64::MAX));
    }

    #[test]
    fn next_due_is_the_earliest() {
        let a = Schedule { due: Some(500), ..Default::default() };
        let b = Schedule { due: Some(300), ..Default::default() };
        let c = Schedule::default();
        assert_eq!(next_due([&a, &b, &c]), Some(300));
        assert_eq!(next_due([&c]), None);
    }

    #[test]
    fn effect_on_items_in_the_folder() {
        let f = "C:\\Users\\me\\Downloads";
        assert_eq!(effect(f, false, Some("C:\\Users\\me\\Downloads\\a.zip"), None), Some(Effect::Refresh));
        assert_eq!(effect(f, false, Some("c:\\users\\ME\\downloads\\a.zip"), None), Some(Effect::Refresh));
        // 資料夾本身（更新資料夾、屬性）
        assert_eq!(effect(f, false, Some("C:\\Users\\me\\Downloads\\"), None), Some(Effect::Refresh));
        // 從別處搬進來（項目二在這裡）
        assert_eq!(effect(f, false, Some("D:\\x\\a.zip"), Some("C:\\Users\\me\\Downloads\\a.zip")), Some(Effect::Refresh));
        // 別的資料夾、子資料夾裡面
        assert_eq!(effect(f, false, Some("C:\\Users\\me\\Desktop\\a.zip"), None), None);
        assert_eq!(effect(f, false, Some("C:\\Users\\me\\Downloads\\sub\\a.zip"), None), None);
        assert_eq!(effect(f, false, None, None), None);
        // 磁碟根目錄
        assert_eq!(effect("D:\\", false, Some("D:\\new.txt"), None), Some(Effect::Refresh));
    }

    #[test]
    fn effect_of_folder_rename() {
        let f = "C:\\work\\photos";
        assert_eq!(effect(f, true, Some("C:\\work\\photos"), Some("C:\\work\\相片")), Some(Effect::Moved("C:\\work\\相片".into())));
        // 上層改名
        assert_eq!(effect(f, true, Some("C:\\WORK"), Some("C:\\jobs")), Some(Effect::Moved("C:\\jobs\\photos".into())));
        // 丟進資源回收筒：不跟過去，重新讀取
        assert_eq!(effect(f, true, Some("C:\\work\\photos"), Some("C:\\$Recycle.Bin\\S-1-5-21\\$R1A2B3C")), Some(Effect::Refresh));
        // 子資料夾改名：只要重新讀取
        assert_eq!(effect(f, true, Some("C:\\work\\photos\\2024"), Some("C:\\work\\photos\\2025")), Some(Effect::Refresh));
        // 只改大小寫：路徑相同，重新讀取即可
        assert_eq!(effect(f, true, Some("C:\\work\\photos"), Some("C:\\work\\Photos")), Some(Effect::Refresh));
        // 無關的資料夾
        assert_eq!(effect(f, true, Some("C:\\work\\photos2"), Some("C:\\work\\photos3")), None);
    }

    #[test]
    fn rebase_paths() {
        assert_eq!(rebase("C:\\a\\b\\c", "C:\\a", "D:\\z").as_deref(), Some("D:\\z\\b\\c"));
        assert_eq!(rebase("C:\\a", "c:\\A\\", "C:\\b").as_deref(), Some("C:\\b"));
        assert_eq!(rebase("C:\\ab", "C:\\a", "C:\\z"), None);
        assert_eq!(rebase("C:\\a\\b", "C:\\a", "D:\\").as_deref(), Some("D:\\b"));
        assert_eq!(rebase("\\\\srv\\share\\x\\y", "\\\\srv\\share\\x", "\\\\srv\\share\\w").as_deref(), Some("\\\\srv\\share\\w\\y"));
        assert_eq!(rebase("C:/a/文件/b", "C:\\a\\文件", "C:\\a\\資料").as_deref(), Some("C:\\a\\資料\\b"));
    }

    #[test]
    fn recycle_bin() {
        assert!(in_recycle_bin("C:\\$Recycle.Bin\\S-1-5-21-1\\$RABC.txt"));
        assert!(in_recycle_bin("d:\\$RECYCLE.BIN\\x"));
        assert!(!in_recycle_bin("C:\\Users\\me\\Recycle.Bin"));
    }

    #[test]
    fn nearest_existing_folder() {
        let exists = |p: &str| p == "C:\\a" || p == "C:\\";
        assert_eq!(nearest_existing("C:\\a\\b\\c", exists).as_deref(), Some("C:\\a"));
        assert_eq!(nearest_existing("C:\\x", exists).as_deref(), Some("C:\\"));
        assert_eq!(nearest_existing("C:\\", exists), None);
        assert_eq!(nearest_existing("E:\\gone\\deeper", |_| false), None);
    }

    #[test]
    fn same_listing() {
        let a = vec![sample("x", true, 0, 1), sample("y.txt", false, 10, 2)];
        let mut b = a.clone();
        assert!(same_entries(&a, &b));
        b[1].size = 11;
        assert!(!same_entries(&a, &b));
        let mut c = a.clone();
        c[1].modified = 3;
        assert!(!same_entries(&a, &c));
        let mut d = a.clone();
        d.swap(0, 1);
        assert!(!same_entries(&a, &d));
        assert!(!same_entries(&a, &a[..1]));
        let mut e = a.clone();
        e[0].is_link = true;
        assert!(!same_entries(&a, &e));
        assert!(same_entries(&[], &[]));
    }

    #[test]
    fn child_plan() {
        assert!(plan_children(&["a", "b"], &["a", "b"]).is_noop());
        let p = plan_children(&["a", "c"], &["a", "b", "c"]);
        assert_eq!(p.remove, Vec::<usize>::new());
        assert_eq!(p.steps, vec![Step::Keep(0), Step::Insert(1), Step::Keep(1)]);
        assert!(!p.is_noop());
        let p = plan_children(&["a", "b", "c"], &["c"]);
        assert_eq!(p.remove, vec![0, 1]);
        assert_eq!(p.steps, vec![Step::Keep(2)]);
        // 改名：舊的移除、新的插入
        let p = plan_children(&["new folder", "z"], &["Photos", "z"]);
        assert_eq!(p.remove, vec![0]);
        assert_eq!(p.steps, vec![Step::Insert(0), Step::Keep(1)]);
        // 只改大小寫也換成新的節點
        let p = plan_children(&["photos"], &["Photos"]);
        assert_eq!(p.remove, vec![0]);
        assert_eq!(p.steps, vec![Step::Insert(0)]);
        let p = plan_children(&[], &[]);
        assert!(p.is_noop());
        let p = plan_children(&["a"], &[]);
        assert_eq!(p.remove, vec![0]);
        assert!(p.steps.is_empty());
    }
}

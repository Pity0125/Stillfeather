//! 分頁的狀態：顯示中的資料夾或搜尋結果、篩選條件、排序、導覽歷史與背景工作的世代。
//! 和視窗無關的部分都在這裡，主視窗（`App`）以 `TabId` 管理多個分頁。

use super::*;

/// 這次搜尋在搜尋執行緒裡就套用的篩選籤條件（其他的條件在介面執行緒上篩選現有結果）
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::win) struct Baked {
    pub category: Option<Category>,
    pub size: Option<SizeRange>,
    pub time: Option<TimeRange>,
}

impl Baked {
    /// 用現在的條件篩選這次的結果，等於重新搜尋的結果（新的條件不比搜尋時寬）
    pub(super) fn covers(&self, category: Category, size: Option<SizeRange>, time: Option<TimeRange>) -> bool {
        let cat = self.category.is_none_or(|c| c == category);
        let size = self.size.is_none_or(|b| size.is_some_and(|n| b.covers(&n)));
        let time = self.time.is_none_or(|b| time.is_some_and(|n| b.covers(&n)));
        cat && size && time
    }
}

#[derive(Default)]
pub(in crate::win) struct ViewState {
    pub selected: Vec<usize>,
    pub focused: i32,
    pub top: i32,
}

pub(in crate::win) struct Tab {
    pub path: String,
    pub requested: String,
    pub back: Vec<String>,
    pub forward: Vec<String>,
    /// 目前資料夾（或搜尋結果）的全部項目
    pub entries: Vec<Entry>,
    /// 套用類型篩選後顯示的列；None 表示全部顯示
    pub rows: Option<Vec<u32>>,
    pub filter: Category,
    /// 篩選籤的修改日期條件（與換算好的時間範圍）
    pub date: Option<(DateSpec, TimeRange)>,
    /// 篩選籤的大小條件（有大小條件時不顯示資料夾）
    pub size: Option<SizeRange>,
    pub summary: Summary,
    /// 篩選籤上的數字：符合日期、大小條件的各類型數量
    pub counts: [usize; category::COUNT],
    /// 搜尋結果的相關性分數，和 entries 一一對齊（不是搜尋結果時是空的）
    pub scores: Vec<u32>,
    pub scorer: Option<Scorer>,
    /// 開始搜尋前的排序：搜尋結果預設依相關性，離開搜尋時還原
    pub saved_sort: Option<(usize, bool)>,
    pub baked: Baked,
    /// 這次搜尋達到結果上限（沒有搜尋完整）
    pub truncated: bool,
    pub sort_col: usize,
    pub desc: bool,
    pub search_mode: bool,
    pub searching: bool,
    pub loading: bool,
    pub query: String,
    pub recursive: bool,
    /// 這次搜尋是整台電腦（全域搜尋）
    pub search_global: bool,
    pub status: String,
    pub generation: u64,
    pub cancel: Arc<AtomicBool>,
    pub view: ViewState,
    pub pending_select: Vec<String>,
    pub pending_rename: bool,
    /// 隱藏中的分頁設定變更後需要重新讀取（切回時才讀，背景分頁不掃描）。
    pub stale: bool,
    /// 這次讀取資料夾或搜尋開始的時間（寫進 trace.log 供效能量測）
    pub work_started: Option<std::time::Instant>,
    /// 選擇器的檔案類型篩選（一般視窗是 None）
    pub pick: Option<feather_core::picker::Filter>,
    /// 相片資料夾（圖片與影片佔一半以上）：沒記住檢視時用大圖示。換資料夾時判斷一次，自動重新整理不會讓檢視跳來跳去
    pub media: bool,
    /// 還原的分頁還沒讀取：第一次顯示時才讀（`requested` 是要開的資料夾）
    pub restored: bool,
    /// 讀資料夾時無法讀取而略過的項目數（狀態列顯示）
    pub skipped: usize,
    /// 隱藏期間顯示的資料夾被刪除或搬走、已改成別的位置：切回這個分頁時在狀態列說明
    pub notice: Option<String>,
}

impl Tab {
    pub(super) fn new() -> Tab {
        Tab {
            path: String::new(),
            requested: String::new(),
            back: Vec::new(),
            forward: Vec::new(),
            entries: Vec::new(),
            rows: None,
            filter: Category::All,
            date: None,
            size: None,
            summary: Summary::default(),
            counts: [0; category::COUNT],
            scores: Vec::new(),
            scorer: None,
            saved_sort: None,
            baked: Baked::default(),
            truncated: false,
            sort_col: 0,
            desc: false,
            search_mode: false,
            searching: false,
            loading: false,
            query: String::new(),
            recursive: true,
            search_global: false,
            status: tr!("選擇資料夾開始瀏覽", "Select a folder to start browsing").into(),
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            view: ViewState::default(),
            pending_select: Vec::new(),
            pending_rename: false,
            stale: false,
            work_started: None,
            pick: None,
            media: false,
            restored: false,
            skipped: 0,
            notice: None,
        }
    }

    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            self.requested.clone()
        } else {
            self.path.clone()
        }
    }

    pub fn working(&self) -> bool {
        self.loading || self.searching
    }

    /// 清單上顯示的列數（已套用篩選）。
    pub fn row_count(&self) -> usize {
        self.rows.as_ref().map(Vec::len).unwrap_or(self.entries.len())
    }

    /// 第 i 個顯示列對應的項目。
    pub fn row(&self, i: usize) -> Option<&Entry> {
        match &self.rows {
            Some(r) => r.get(i).and_then(|j| self.entries.get(*j as usize)),
            None => self.entries.get(i),
        }
    }

    pub fn refilter(&mut self) {
        let plain = self.date.is_none() && self.size.is_none() && self.pick.as_ref().is_none_or(|p| p.is_all());
        self.rows = if plain {
            filter_rows(&self.entries, self.filter)
        } else {
            Some(self.entries.iter().enumerate().filter(|(_, e)| self.shows(e)).map(|(i, _)| i as u32).collect())
        };
    }

    /// 符合篩選籤的日期與大小條件（不看類型）
    pub(super) fn passes_conditions(&self, e: &Entry) -> bool {
        self.size.is_none_or(|r| !e.is_dir && r.contains(e.size)) && self.date.as_ref().is_none_or(|(_, t)| t.contains(e.modified))
    }

    /// 這個項目在清單上顯示嗎（類型、日期、大小篩選籤，以及選擇器的檔案類型）
    pub fn shows(&self, e: &Entry) -> bool {
        self.filter.matches(e) && self.passes_conditions(e) && self.pick.as_ref().is_none_or(|p| p.matches(&e.name, e.is_dir))
    }

    pub fn resummarize(&mut self) {
        self.summary = Summary::of(&self.entries);
        self.counts = [0; category::COUNT];
        for i in 0..self.entries.len() {
            self.count(i);
        }
    }

    /// 篩選籤上的數字加上第 i 個項目
    fn count(&mut self, i: usize) {
        let Some(e) = self.entries.get(i) else { return };
        if self.passes_conditions(e) {
            let c = Category::of(e).index();
            self.counts[Category::All.index()] += 1;
            self.counts[c] += 1;
        }
    }

    /// 有篩選籤的條件（類型、日期或大小）
    pub fn has_chip_conditions(&self) -> bool {
        self.filter != Category::All || self.date.is_some() || self.size.is_some()
    }

    /// 狀態列上的篩選條件：「文件、本週、> 10 MB」
    pub fn condition_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.filter != Category::All {
            parts.push(self.filter.label().into());
        }
        if let Some((d, _)) = &self.date {
            parts.push(d.label());
        }
        if let Some(s) = &self.size {
            parts.push(s.label());
        }
        parts.join(tr!("、", ", "))
    }

    /// 移除項目（相關性分數跟著移除）
    pub fn retain_entries(&mut self, mut keep: impl FnMut(&Entry) -> bool) {
        let flags: Vec<bool> = self.entries.iter().map(&mut keep).collect();
        if self.scores.len() == self.entries.len() {
            let mut it = flags.iter();
            self.scores.retain(|_| *it.next().unwrap_or(&true));
        } else {
            self.scores.clear();
        }
        let mut it = flags.iter();
        self.entries.retain(|_| *it.next().unwrap_or(&true));
    }

    /// 依目前的排序重排（搜尋結果連同相關性分數）
    pub fn resort(&mut self, folders_on_top: bool) {
        if let Some(scorer) = self.scorer.as_ref().filter(|_| self.scores.len() != self.entries.len()) {
            self.scores = self.entries.iter().map(|e| scorer.score(&e.name, e.is_dir, &e.dir)).collect();
        }
        sorting::sort_scored(&mut self.entries, &mut self.scores, self.sort_col, self.desc, folders_on_top);
    }

    /// 離開搜尋結果（改顯示資料夾）：相關性分數不再需要，排序換回搜尋前的
    pub(super) fn leave_search(&mut self) {
        self.scores = Vec::new();
        self.scorer = None;
        self.baked = Baked::default();
        self.truncated = false;
        if let Some((col, desc)) = self.saved_sort.take() {
            self.sort_col = col;
            self.desc = desc;
        }
        if self.sort_col == sorting::COL_RELEVANCE {
            self.sort_col = sorting::COL_NAME;
            self.desc = false;
        }
    }

    /// 讀資料夾時用的排序（搜尋結果的相關性不是資料夾的排序）
    pub(super) fn folder_sort(&self) -> (usize, bool) {
        match self.saved_sort {
            Some(s) => s,
            None if self.sort_col == sorting::COL_RELEVANCE => (sorting::COL_NAME, false),
            None => (self.sort_col, self.desc),
        }
    }

    /// 取消目前工作並開始新的一輪；舊結果會因世代不符而被忽略。
    pub(in crate::win) fn begin_work(&mut self) -> (u64, Arc<AtomicBool>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation += 1;
        (self.generation, self.cancel.clone())
    }
}

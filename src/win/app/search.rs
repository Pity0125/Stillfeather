//! 搜尋：搜尋框的展開與收合、開始搜尋（搜尋框的條件見 `feather_core::searchquery`）、分批收到的結果與相關性排序、
//! 篩選籤（類型、日期、大小）的條件與選單、自訂範圍的對話方塊，以及說明裡的搜尋語法。

use super::*;

impl App {
    /// 搜尋該展開的時候：輸入框有焦點（視窗暫時不在前景時看離開前的焦點），或這個分頁正在搜尋、顯示搜尋結果。
    fn search_wanted(&self) -> bool {
        let focus = unsafe { GetFocus() };
        let focused = if focus == 0 { self.last_focus == self.query } else { focus == self.query };
        focused || self.active_tab().is_some_and(|t| t.searching || t.search_mode)
    }

    fn set_search_open(&mut self, open: bool) {
        if (self.search_open.target() > 0.5) == open {
            return;
        }
        anim::to(&mut self.search_open, open as u8 as f32, anim::SLIDE_MS);
        if anim::running(&self.search_open) {
            unsafe {
                SetTimer(self.hwnd, TIMER_SEARCH, 15, std::ptr::null());
            }
        }
        self.place_top_row();
        // 「搜尋含子資料夾」跟著搜尋框出現、收起
        invalidate(self.chips);
        self.sync_action_labels();
    }

    /// 依目前狀態展開或收合搜尋。
    pub(in crate::win) fn sync_search(&mut self) {
        let want = self.search_wanted();
        self.set_search_open(want);
    }

    /// 按下搜尋按鈕或 Ctrl+F：展開並把焦點放進輸入框。
    pub(super) fn open_search(&mut self) {
        self.set_search_open(true);
        unsafe {
            SetFocus(self.query);
        }
    }

    pub(super) fn on_search_timer(&mut self) {
        self.place_top_row();
        if !anim::running(&self.search_open) {
            unsafe {
                KillTimer(self.hwnd, TIMER_SEARCH);
            }
        }
    }

    pub(super) fn start_search(&mut self) {
        let id = self.active_tab_id();
        let text = window_text(self.query);
        let global = self.search_global;
        if !global {
            let here = self.active_tab().map(|t| t.path.clone()).unwrap_or_default();
            if self.refuse_share_list(&here) {
                return;
            }
        }
        self.run_search(id, text, global, false, None);
    }

    /// 開始搜尋。搜尋框的文字解析成條件（見 searchquery.rs），日期在這裡換算成時間範圍。
    /// `rerun`：篩選籤的條件改了，用同樣的文字重新搜尋（保留目前的排序，篩選籤的條件一律在搜尋執行緒裡套用）；
    /// `notice`：顯示在狀態列右邊的說明。
    pub(super) fn run_search(&mut self, id: TabId, text: String, global: bool, rerun: bool, notice: Option<String>) {
        let show_hidden = self.show_hidden;
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if (tab.path.is_empty() && !global) || tab.loading {
            return;
        }
        let query = match Query::parse(&text) {
            Ok(q) => q,
            Err(message) => {
                tab.status = message;
                unsafe {
                    SetFocus(self.query);
                }
                self.update_chrome();
                return;
            }
        };
        // 搜尋框空著也可以搜尋，只要有篩選條件（在「下載」選「文件」「本週」後按 Enter：列出本週的文件）
        if query.is_empty() && !tab.has_chip_conditions() {
            tab.status = tr!("請輸入要搜尋的檔案名稱，或先選篩選條件", "Enter a file name to search for, or pick a filter first").into();
            unsafe {
                SetFocus(self.query);
            }
            self.update_chrome();
            return;
        }
        // 沒有檔名可以比對時，篩選籤的條件在搜尋執行緒裡就套用（不然會列出整個資料夾）；有檔名時結果通常不多，
        // 條件改變時直接篩選現有的結果。重新搜尋（結果太多被截斷）時一律在搜尋執行緒裡套用。
        let baked = if rerun || query.terms.is_empty() {
            Baked { category: (tab.filter != Category::All).then_some(tab.filter), size: tab.size, time: tab.date.map(|d| d.1) }
        } else {
            Baked::default()
        };
        let filter = Filter::new(&query, query_time(&query)).and(baked.size, baked.time);
        let recursive = tab.recursive;
        tab.query = text;
        tab.search_global = global;
        let (generation, cancel) = tab.begin_work();
        tab.work_started = Some(std::time::Instant::now());
        tab.searching = true;
        if !tab.search_mode {
            tab.saved_sort = Some((tab.sort_col, tab.desc));
        }
        // 搜尋結果預設依相關性排序；重新搜尋時保留使用者點選的欄位
        if !rerun {
            tab.sort_col = sorting::COL_RELEVANCE;
            tab.desc = false;
        }
        tab.search_mode = true;
        tab.entries.clear();
        tab.scores.clear();
        tab.scorer = Some(Scorer::new(&query));
        tab.baked = baked;
        tab.truncated = false;
        tab.refilter();
        tab.resummarize();
        tab.view = ViewState::default();
        let category = baked.category;
        let scope = if global { fs::SearchScope::Computer } else { fs::SearchScope::Folder(tab.path.clone()) };
        tab.status = if global {
            trf!("搜尋整台電腦（{}）· 只比對檔名，不讀取檔案內容", "Searching the whole PC ({}) · file names only, file contents aren't read", fs::computer_roots().iter().map(|r| r.trim_end_matches('\\')).collect::<Vec<_>>().join(tr!("、", ", ")))
        } else {
            tr!("搜尋中 · 只比對檔名，不讀取檔案內容", "Searching · file names only, file contents aren't read").into()
        };
        std::thread::spawn(move || {
            let result = fs::search(&scope, &filter, category, recursive, show_hidden, &cancel, |entries, scanned, skipped| {
                post(WorkerMsg::SearchBatch { tab: id, generation, entries, scanned, skipped });
            });
            post(WorkerMsg::SearchDone { tab: id, generation, result });
        });
        if let Some(n) = notice {
            self.set_footer(n);
        }
        self.hide_preview();
        if let Some(side) = self.ws.side_of(id) {
            if self.bound[side.index()] == Some(id) {
                self.refresh_list(side, true);
            }
        }
        self.update_chrome();
    }

    /// 一批搜尋結果：依相關性（或點選的欄位）併進已經排好的結果，選取與捲動位置不動。
    pub(super) fn on_search_batch(&mut self, id: TabId, generation: u64, entries: Vec<Entry>, scanned: usize, skipped: usize) {
        if self.tabs.get(&id).is_none_or(|t| t.generation != generation) {
            return;
        }
        let side = self.ws.side_of(id).filter(|s| self.bound[s.index()] == Some(id));
        let kept = side.map(|s| self.keep_view(s));
        let folders_on_top = self.settings.folders_on_top;
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        let mut counts = [0usize; category::COUNT];
        for e in &entries {
            tab.summary.add(e);
            if tab.passes_conditions(e) {
                counts[Category::All.index()] += 1;
                counts[Category::of(e).index()] += 1;
            }
        }
        for (total, n) in tab.counts.iter_mut().zip(counts) {
            *total += n;
        }
        let scores: Vec<u32> = match &tab.scorer {
            Some(s) => entries.iter().map(|e| s.score(&e.name, e.is_dir, &e.dir)).collect(),
            None => Vec::new(),
        };
        sorting::merge_scored(&mut tab.entries, &mut tab.scores, entries, scores, tab.sort_col, tab.desc, folders_on_top);
        tab.refilter();
        tab.status = trn!(
            tab.entries.len();
            "搜尋中 · {} 個結果 · 已檢查 {} 項 · 略過 {} 項", "Searching · {} result · {} checked · {} skipped", "Searching · {} results · {} checked · {} skipped",
            group_digits(tab.entries.len() as u64),
            group_digits(scanned as u64),
            group_digits(skipped as u64)
        );
        if let (Some(side), Some(kept)) = (side, kept) {
            self.restore_view(side, kept);
        }
        invalidate(self.chips);
    }

    pub(super) fn on_search_done(&mut self, id: TabId, generation: u64, result: Result<SearchSummary, FsError>) {
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.generation != generation {
            return;
        }
        tab.searching = false;
        if let Some(started) = tab.work_started.take() {
            crate::win::crashlog::crumb_value("效能：搜尋完成（毫秒）", started.elapsed().as_millis() as u64);
            crate::win::crashlog::crumb_value("效能：搜尋結果數", tab.entries.len() as u64);
        }
        if let Ok(s) = &result {
            tab.truncated = s.truncated;
        }
        tab.status = match result {
            Ok(s) => trn!(
                s.matched;
                "{} · {} 個結果 · 已檢查 {} 項 · 略過 {} 項", "{} · {} result · {} checked · {} skipped", "{} · {} results · {} checked · {} skipped",
                if s.truncated {
                    tr!("結果已達上限（未完整搜尋）", "Result limit reached (search incomplete)")
                } else if s.skipped > 0 {
                    tr!("部分完成", "Partly done")
                } else {
                    tr!("搜尋完成", "Search complete")
                },
                group_digits(s.matched as u64),
                group_digits(s.scanned as u64),
                group_digits(s.skipped as u64)
            ),
            Err(FsError::Cancelled) => trf!("已取消 · 保留 {} 個結果（未完整搜尋）", "Cancelled · kept {} results (search incomplete)", group_digits(tab.entries.len() as u64)),
            Err(FsError::Message(e)) => trf!("無法完成：{e}", "Couldn't complete: {e}"),
        };
        if let Some(side) = self.ws.side_of(id) {
            if self.bound[side.index()] == Some(id) {
                let restore_selection = !self.tabs[&id].pending_select.is_empty();
                self.refresh_list(side, restore_selection);
                self.fit_columns(side);
            }
        }
        self.update_chrome();
    }

    /// 篩選籤的條件（類型、日期、大小）改了。搜尋結果被截斷、或這次搜尋在搜尋執行緒裡套用的條件比新的窄時，
    /// 帶著新的條件重新搜尋；否則直接篩選現有的結果（交叉淡化，保留選取），不重新搜尋。
    pub(in crate::win) fn filters_changed(&mut self, id: TabId) {
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.search_mode && !tab.loading && (tab.truncated || !tab.baked.covers(tab.filter, tab.size, tab.date.map(|d| d.1))) {
            let only_filters = Query::parse(&tab.query).is_ok_and(|q| q.is_empty());
            if only_filters && !tab.has_chip_conditions() {
                // 只靠篩選條件搜尋，條件又全部清掉了：回到資料夾
                self.refresh_tab(id);
                return;
            }
            let notice = if tab.truncated {
                tr!("結果太多，已用篩選條件重新搜尋", "Too many results; searched again with the filters")
            } else {
                tr!("已用新的篩選條件重新搜尋", "Searched again with the new filters")
            };
            let (text, global) = (tab.query.clone(), tab.search_global);
            self.run_search(id, text, global, true, Some(notice.into()));
            return;
        }
        let side = self.ws.side_of(id).filter(|s| self.bound[s.index()] == Some(id));
        let _fade = match side {
            Some(s) => self.crossfade(self.content_fade_areas(s), anim::CROSSFADE_MS),
            None => anim::Crossfade::none(),
        };
        let selected = side.map(|s| self.selected_paths(s)).unwrap_or_default();
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        tab.refilter();
        tab.resummarize();
        tab.view = ViewState::default();
        tab.pending_select = selected;
        self.hide_preview();
        if let Some(s) = side {
            self.refresh_list(s, true);
        }
        self.update_chrome();
    }

    /// 狀態列的「清除篩選」：類型、日期、大小都回到不限
    pub(super) fn clear_filters(&mut self) {
        let id = self.active_tab_id();
        let Some(t) = self.tabs.get_mut(&id) else { return };
        if !t.has_chip_conditions() {
            return;
        }
        t.filter = Category::All;
        t.date = None;
        t.size = None;
        self.filters_changed(id);
    }

    fn set_date_condition(&mut self, id: TabId, spec: Option<DateSpec>) {
        let Some(t) = self.tabs.get_mut(&id) else { return };
        let new = spec.map(|d| (d, date_range(&d)));
        if t.date == new {
            return;
        }
        t.date = new;
        self.filters_changed(id);
    }

    fn set_size_condition(&mut self, id: TabId, size: Option<SizeRange>) {
        let Some(t) = self.tabs.get_mut(&id) else { return };
        if t.size == size {
            return;
        }
        t.size = size;
        self.filters_changed(id);
    }

    /// 篩選籤「日期」「大小」的選單（開在按鈕下方）
    fn condition_menu(&mut self, which: chips::Which) {
        self.hide_preview();
        let id = self.active_tab_id();
        let Some(anchor) = chips::anchor(self.chips, which) else { return };
        match which {
            chips::Which::Date => {
                let current = self.tabs.get(&id).and_then(|t| t.date.map(|d| d.0));
                let labels: Vec<String> = DATE_PRESETS.iter().map(|d| d.label()).collect();
                let custom = current.is_some_and(|c| !DATE_PRESETS.contains(&c));
                let mut items = vec![menu::Item::radio(id::DATE_ANY, tr!("不限", "Any time"), current.is_none())];
                for (i, (d, label)) in DATE_PRESETS.iter().zip(&labels).enumerate() {
                    items.push(menu::Item::radio(id::DATE_FIRST + i as u16, label, current == Some(*d)));
                }
                items.push(menu::Item::Separator);
                items.push(menu::Item::radio(id::DATE_CUSTOM, tr!("自訂範圍…", "Custom range…"), custom));
                chips::set_menu_open(self.chips, Some(which));
                let cmd = menu::drop_down_at(self.hwnd, anchor, &items);
                chips::set_menu_open(self.chips, None);
                let spec = match cmd {
                    Some(id::DATE_ANY) => None,
                    Some(c) if (id::DATE_FIRST..id::DATE_FIRST + DATE_PRESETS.len() as u16).contains(&c) => Some(DATE_PRESETS[(c - id::DATE_FIRST) as usize]),
                    Some(id::DATE_CUSTOM) => match self.custom_date_dialog(current) {
                        Some(spec) => spec,
                        None => return,
                    },
                    _ => return,
                };
                self.set_date_condition(id, spec);
            }
            chips::Which::Size => {
                let current = self.tabs.get(&id).and_then(|t| t.size);
                let labels: Vec<String> = SIZE_PRESETS
                    .iter()
                    .map(|r| match (r.min, r.max) {
                        (0, max) => trf!("小於 {}", "Smaller than {}", format_bytes(max)),
                        (min, u64::MAX) => trf!("大於 {}", "Larger than {}", format_bytes(min)),
                        _ => r.label(),
                    })
                    .collect();
                let custom = current.is_some_and(|c| !SIZE_PRESETS.contains(&c));
                let mut items = vec![menu::Item::radio(id::SIZE_ANY, tr!("不限", "Any size"), current.is_none())];
                for (i, (r, label)) in SIZE_PRESETS.iter().zip(&labels).enumerate() {
                    items.push(menu::Item::radio(id::SIZE_FIRST + i as u16, label, current == Some(*r)));
                }
                items.push(menu::Item::Separator);
                items.push(menu::Item::radio(id::SIZE_CUSTOM, tr!("自訂…", "Custom…"), custom));
                chips::set_menu_open(self.chips, Some(which));
                let cmd = menu::drop_down_at(self.hwnd, anchor, &items);
                chips::set_menu_open(self.chips, None);
                let size = match cmd {
                    Some(id::SIZE_ANY) => None,
                    Some(c) if (id::SIZE_FIRST..id::SIZE_FIRST + SIZE_PRESETS.len() as u16).contains(&c) => Some(SIZE_PRESETS[(c - id::SIZE_FIRST) as usize]),
                    Some(id::SIZE_CUSTOM) => match self.custom_size_dialog(current) {
                        Some(size) => size,
                        None => return,
                    },
                    _ => return,
                };
                self.set_size_condition(id, size);
            }
        }
    }

    /// 「日期 › 自訂範圍…」：兩個日期挑選器（不勾是不限）。取消時回傳 None；兩邊都不限時是 Some(None)。
    fn custom_date_dialog(&self, current: Option<DateSpec>) -> Option<Option<DateSpec>> {
        let today = local_today();
        let (from, to) = match current {
            Some(DateSpec::Range { from, to }) => (from, to),
            // 從預設的「本週」「最近 7 天」改成自訂：從那段日期開始改
            Some(d) => {
                let (a, b) = d.local_days(today, first_day_of_week());
                (a, b.map(|x| x.add_days(-1)))
            }
            None => (Some(today.add_days(-6)), Some(today)),
        };
        let (button, values) = dialog::Spec::new(Icon::Info, tr!("自訂日期範圍", "Custom date range"))
            .body(tr!("只顯示這段期間修改過的項目（頭尾兩天都包含在內）。", "Show only items modified in this period (both days included)."))
            .fields(vec![
                dialog::Field::Date { label: tr!("從", "From").into(), value: from, optional: true },
                dialog::Field::Date { label: tr!("到", "To").into(), value: to, optional: true },
            ])
            .footnote(tr!("取消勾選表示不限。也可以在搜尋框輸入 dm:2026-11-01..2026-11-20。", "Clear a check box for no limit. You can also type dm:2026-11-01..2026-11-20 in the search box."))
            .buttons(vec![dialog::Button::primary(1, tr!("套用", "Apply")), dialog::Button::normal(0, tr!("取消", "Cancel"))], 0)
            .show_form(self.hwnd);
        if button != 1 {
            return None;
        }
        let date = |i: usize| match values.get(i) {
            Some(dialog::FieldValue::Date(d)) => *d,
            _ => None,
        };
        let (from, to) = (date(0), date(1));
        Some((from.is_some() || to.is_some()).then_some(DateSpec::Range { from, to }))
    }

    /// 「大小 › 自訂…」：最小、最大（空著是不限）。看不懂時說明寫法再問一次。取消時回傳 None；兩邊都空著時是 Some(None)。
    fn custom_size_dialog(&self, current: Option<SizeRange>) -> Option<Option<SizeRange>> {
        // 範圍的上限不含；顯示成「最大」時減回去（1 MB 以下顯示「1 MB」）
        let shown_max = |m: u64| if m % 1024 == 1 { m - 1 } else { m };
        let mut min_text = current.filter(|r| r.min > 0).map(|r| format_bytes(r.min)).unwrap_or_default();
        let mut max_text = current.filter(|r| r.max != u64::MAX).map(|r| format_bytes(shown_max(r.max))).unwrap_or_default();
        loop {
            let (button, values) = dialog::Spec::new(Icon::Info, tr!("自訂大小", "Custom size"))
                .body(tr!("只顯示大小在這個範圍內的檔案（資料夾沒有大小，不會顯示）。", "Show only files in this size range (folders have no size and are hidden)."))
                .fields(vec![
                    dialog::Field::Text { label: tr!("最小", "At least").into(), value: min_text.clone(), cue: tr!("例如 10 MB", "e.g. 10 MB").into() },
                    dialog::Field::Text { label: tr!("最大", "At most").into(), value: max_text.clone(), cue: tr!("例如 1.5 GB", "e.g. 1.5 GB").into() },
                ])
                .footnote(tr!("可以寫 KB、MB、GB、TB（1 KB = 1024 位元組）；空著表示不限。也可以在搜尋框輸入 size:10mb..1gb。", "Use KB, MB, GB or TB (1 KB = 1024 bytes); leave blank for no limit. You can also type size:10mb..1gb in the search box."))
                .buttons(vec![dialog::Button::primary(1, tr!("套用", "Apply")), dialog::Button::normal(0, tr!("取消", "Cancel"))], 0)
                .show_form(self.hwnd);
            if button != 1 {
                return None;
            }
            let text = |i: usize| match values.get(i) {
                Some(dialog::FieldValue::Text(t)) => t.trim().to_string(),
                _ => String::new(),
            };
            (min_text, max_text) = (text(0), text(1));
            let parse = |s: &str| if s.is_empty() { Some(None) } else { parse_size(s).map(Some) };
            match (parse(&min_text), parse(&max_text)) {
                (Some(None), Some(None)) => return Some(None),
                (Some(a), Some(b)) => {
                    let min = a.unwrap_or(0);
                    let max = b.map_or(u64::MAX, |v| v.saturating_add(1));
                    // 寫反了（最小比最大大）：對調
                    let (min, max) = if max <= min { (max.saturating_sub(1), min.saturating_add(1)) } else { (min, max) };
                    return Some(Some(SizeRange { min, max }));
                }
                _ => dialog::error(
                    self.hwnd,
                    tr!("看不懂這個大小", "Can't read that size"),
                    tr!("請寫成數字加單位，例如 500 KB、10 MB、1.5 GB；空著表示不限。", "Write a number and a unit, such as 500 KB, 10 MB or 1.5 GB; leave blank for no limit."),
                ),
            }
        }
    }

    pub(super) fn clear_or_cancel(&mut self) {
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.working() {
            tab.cancel.store(true, Ordering::Relaxed);
            tab.status = tr!("正在取消…", "Cancelling…").into();
            self.update_chrome();
        } else {
            tab.query.clear();
            self.suppress_query_change = true;
            set_window_text(self.query, "");
            self.suppress_query_change = false;
            self.refresh_tab(id);
        }
    }

    /// 篩選籤列：類型篩選、日期與大小、搜尋含子資料夾。
    pub(super) fn chip_action(&mut self, wparam: WPARAM) {
        let value = wparam >> 8;
        match wparam & 0xFF {
            chips::ACTION_FILTER => {
                let Some(c) = chips::category_from_index(value) else { return };
                let id = self.active_tab_id();
                let Some(tab) = self.tabs.get_mut(&id) else { return };
                if tab.filter != c {
                    tab.filter = c;
                    self.filters_changed(id);
                }
            }
            chips::ACTION_MENU => {
                if let Some(w) = chips::Which::from_index(value) {
                    self.condition_menu(w);
                }
            }
            chips::ACTION_CLEAR => {
                let id = self.active_tab_id();
                match chips::Which::from_index(value) {
                    Some(chips::Which::Date) => self.set_date_condition(id, None),
                    Some(chips::Which::Size) => self.set_size_condition(id, None),
                    None => {}
                }
            }
            chips::ACTION_RECURSIVE => {
                if let Some(t) = self.active_tab_mut() {
                    t.recursive = !t.recursive;
                }
            }
            _ => {}
        }
        self.update_chrome();
    }
}

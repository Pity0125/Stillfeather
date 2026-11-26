//! 導覽：在背景讀取資料夾、讀好後換上清單（保留選取、上一頁／下一頁紀錄），上一頁、下一頁、上一層與重新整理。

use super::*;

impl App {
    pub(in crate::win) fn navigate(&mut self, tab_id: TabId, input: &str, kind: NavKind, select: Vec<String>) {
        self.navigate_with(tab_id, input, kind, select, false);
    }

    /// 開啟還不知道是檔案還是資料夾的位置（網路上的位置）：先當成資料夾在背景讀取，畫面顯示「正在讀取資料夾…」；
    /// 其實是檔案時改開它所在的資料夾並選取它（見 on_not_folder）。不在介面執行緒上先檢查，
    /// 因為網路位置離線時連檢查都要等到 SMB 逾時（約 20 秒），這段時間整個視窗會凍住。
    pub(in crate::win) fn navigate_or_reveal(&mut self, tab_id: TabId, input: &str, kind: NavKind) {
        self.navigate_with(tab_id, input, kind, vec![], true);
    }

    pub(super) fn navigate_with(&mut self, tab_id: TabId, input: &str, kind: NavKind, select: Vec<String>, reveal: bool) {
        let show_hidden = self.show_hidden;
        let path = match full_path(input) {
            // 只有電腦名稱（`\\nas`、`//nas\`）：統一寫成 `\\nas`，麵包屑、上一頁才對得上
            Ok(p) if paths::unc_server(&p).is_some() => format!("\\\\{}", paths::unc_server(&p).unwrap_or_default()),
            Ok(p) => p,
            Err(e) => {
                if let Some(t) = self.tabs.get_mut(&tab_id) {
                    t.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
                }
                self.update_chrome();
                return;
            }
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        let (generation, cancel) = tab.begin_work();
        tab.work_started = Some(std::time::Instant::now());
        tab.loading = true;
        tab.searching = false;
        tab.stale = false;
        tab.pending_select = select;
        if tab.path.is_empty() {
            tab.requested = path.clone();
        }
        tab.status = tr!("正在讀取資料夾…", "Reading folder…").into();
        let (sort_col, desc) = tab.folder_sort();
        let folders_on_top = self.settings.folders_on_top;
        let p = path.clone();
        let owner = self.hwnd;
        std::thread::spawn(move || {
            let read = || -> Result<Option<Listing>, FsError> {
                if let Some(server) = paths::unc_server(&p) {
                    return fs::list_shares(server, show_hidden, sort_col, desc, folders_on_top).map(Some);
                }
                if reveal {
                    fs::read_dir_or_file(&p, show_hidden, sort_col, desc, folders_on_top, &cancel)
                } else {
                    fs::read_dir(&p, show_hidden, sort_col, desc, folders_on_top, &cancel).map(Some)
                }
            };
            let mut result = read();
            // 網路位置要帳號密碼：叫出 Windows 自己的登入視窗，登入成功後再讀一次
            if let Err(e) = &result {
                if paths::is_unc(&p) && fs::needs_credentials(e.code()) && !cancel.load(Ordering::Relaxed) {
                    let remote = paths::unc_share_root(&p).unwrap_or_else(|| format!("{}\\IPC$", paths::trim_end_sep(&p)));
                    crate::win::crashlog::crumb("網路位置：要求登入");
                    result = match fs::connect_interactive(owner, &remote) {
                        Ok(()) => read(),
                        Err(1223) => Err(FsError::msg(trf!("需要登入才能開啟 {remote}（已取消）。", "Signing in is required to open {remote} (cancelled)."))),
                        Err(code) => Err(FsError::win32(code)),
                    };
                }
            }
            let result = match result {
                Ok(Some(listing)) => Ok(listing),
                Ok(None) => return post(WorkerMsg::NotFolder { tab: tab_id, generation, path: p, kind }),
                Err(e) => Err(e),
            };
            // 讀不到時順便確認資料夾還在不在（同自動重新整理）：網路位置斷線時這要等很久，不能在介面執行緒上做
            let gone_to = match &result {
                Err(FsError::Message(_)) if paths::unc_server(&p).is_none() && !fs::is_dir(&p) => feather_core::watch::nearest_existing(&p, fs::is_dir),
                _ => None,
            };
            post(WorkerMsg::Dir { tab: tab_id, generation, path: p, kind, result, gone_to });
        });
        if self.ws.side_of(tab_id).is_some_and(|s| self.bound[s.index()] == Some(tab_id)) {
            self.hide_preview();
        }
        self.update_chrome();
        invalidate(self.strips[0]);
        invalidate(self.strips[1]);
    }

    pub(super) fn on_dir(&mut self, tab_id: TabId, generation: u64, path: String, kind: NavKind, result: Result<Listing, FsError>, gone_to: Option<String>) {
        // 換到另一個資料夾、而且這個分頁正顯示在畫面上：舊清單淡成新清單。
        let _fade = match (self.tabs.get(&tab_id), self.ws.side_of(tab_id)) {
            (Some(tab), Some(side))
                if tab.generation == generation
                    && result.is_ok()
                    && !paths::same_path(&tab.path, &path)
                    && self.bound[side.index()] == Some(tab_id)
                    && (side == Side::Left || self.ws.dual) =>
            {
                self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
            }
            _ => anim::Crossfade::none(),
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        if tab.generation != generation {
            return;
        }
        tab.loading = false;
        let loaded_ok = result.is_ok();
        if !loaded_ok {
            tab.work_started = None;
        }
        // 重新讀取同一個資料夾（F5、Stillfeather 自己的檔案作業之後）：側欄裡展開的這個資料夾也跟著更新
        let mut tree_sync: Option<Vec<(String, bool)>> = None;
        // 重新讀取目前的資料夾，它卻已經不在了（例如隱藏期間被別的程式刪除）：改顯示還在的上一層
        let mut gone: Option<(String, String)> = None;
        match result {
            Ok(listing) => {
                let old = std::mem::take(&mut tab.path);
                // 只是重新整理同一個資料夾時保留類型篩選；換資料夾時回到「全部」。
                let folder_changed = !paths::same_path(&old, &path);
                match kind {
                    NavKind::Normal => {
                        if !old.is_empty() && old != path {
                            tab.back.push(old);
                            tab.forward.clear();
                        }
                    }
                    NavKind::Back => {
                        tab.back.pop();
                        if !old.is_empty() {
                            tab.forward.push(old);
                        }
                    }
                    NavKind::Forward => {
                        tab.forward.pop();
                        if !old.is_empty() {
                            tab.back.push(old);
                        }
                    }
                    NavKind::NoHistory => {}
                }
                if folder_changed {
                    tab.filter = Category::All;
                    tab.date = None;
                    tab.size = None;
                }
                if tab.search_mode {
                    tab.leave_search();
                }
                tab.path = path.clone();
                tab.requested = path;
                tab.entries = listing.entries;
                if folder_changed {
                    tab.media = feather_core::thumbs::media_folder(&tab.entries);
                }
                tab.refilter();
                tab.resummarize();
                tab.search_mode = false;
                tab.view = ViewState::default();
                tab.status = folder_status(tab.entries.len(), listing.skipped);
                tab.skipped = listing.skipped;
                if !folder_changed {
                    tree_sync = Some(tab.entries.iter().filter(|e| e.is_dir).map(|e| (e.name.clone(), e.is_link)).collect());
                }
            }
            Err(FsError::Cancelled) => {
                tab.status = tr!("已取消讀取", "Cancelled reading").into();
                tab.pending_select.clear();
                tab.pending_rename = false;
            }
            Err(FsError::Message(e)) => {
                tab.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
                tab.pending_select.clear();
                tab.pending_rename = false;
                // 讀不到的是畫面上這個資料夾本身（重新整理、切回隱藏的分頁）：舊內容已經不對了，清空，只顯示原因。
                // 換到別的資料夾失敗時（例如輸入了不存在的路徑），畫面上的仍是原本資料夾的內容，保留。
                if paths::same_path(&tab.path, &path) {
                    tab.entries.clear();
                    tab.scores.clear();
                    tab.refilter();
                    tab.resummarize();
                    if kind == NavKind::NoHistory {
                        gone = gone_to.map(|target| (path.clone(), target));
                    }
                }
            }
        }
        let shown = self.ws.side_of(tab_id).is_some_and(|side| self.bound[side.index()] == Some(tab_id));
        if let Some(side) = self.ws.side_of(tab_id) {
            if self.bound[side.index()] == Some(tab_id) {
                self.refresh_list(side, true);
            }
        }
        if tab_id == self.active_tab_id() {
            set_window_text(self.address, &self.tabs[&tab_id].path.clone());
        }
        if let Some(folders) = tree_sync {
            let path = self.tabs[&tab_id].path.clone();
            self.sync_tree_folder(&path, &folders);
        }
        self.update_chrome();
        // 效能記錄：從開始讀取到清單換好（含背景列舉、排序與畫面更新）
        if let Some(t) = self.tabs.get_mut(&tab_id) {
            if let Some(started) = t.work_started.take() {
                let count = t.entries.len() as u64;
                crate::win::crashlog::crumb_value("效能：資料夾讀取完成（毫秒）", started.elapsed().as_millis() as u64);
                crate::win::crashlog::crumb_value("效能：項目數", count);
                if shown && !FIRST_FOLDER_SHOWN.swap(true, Ordering::Relaxed) {
                    crate::win::crashlog::crumb_value("效能：第一個資料夾顯示（啟動後毫秒）", crate::win::since_start_ms());
                }
            }
        }
        self.finish_incoming_tab(tab_id, loaded_ok);
        if let Some((path, target)) = gone {
            self.folder_gone(tab_id, &path, &target);
        }
    }

    /// [`navigate_or_reveal`](Self::navigate_or_reveal) 的位置其實是檔案：同一個分頁改開它所在的資料夾並選取它
    pub(super) fn on_not_folder(&mut self, tab_id: TabId, generation: u64, path: String, kind: NavKind) {
        if self.tabs.get(&tab_id).is_none_or(|t| t.generation != generation) {
            return;
        }
        let folder = paths::parent(&path).unwrap_or_else(|| path.clone());
        self.navigate(tab_id, &folder, kind, vec![path]);
    }

    pub(in crate::win) fn go_back(&mut self) {
        let id = self.active_tab_id();
        let target = self.tabs.get(&id).and_then(|t| if t.loading { None } else { t.back.last().cloned() });
        if let Some(t) = target {
            self.navigate(id, &t, NavKind::Back, vec![]);
        }
    }

    pub(in crate::win) fn go_forward(&mut self) {
        let id = self.active_tab_id();
        let target = self.tabs.get(&id).and_then(|t| if t.loading { None } else { t.forward.last().cloned() });
        if let Some(t) = target {
            self.navigate(id, &t, NavKind::Forward, vec![]);
        }
    }

    pub(in crate::win) fn go_up(&mut self) {
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get(&id) else { return };
        if let Some(parent) = paths::up_target(&tab.path) {
            let select = vec![tab.path.clone()];
            self.navigate(id, &parent, NavKind::Normal, select);
        }
    }

    /// 重新整理：回到目前資料夾清單，保留選取。
    pub(in crate::win) fn refresh_tab(&mut self, id: TabId) {
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.path.is_empty() {
            return;
        }
        let path = tab.path.clone();
        let select = self.bound_selection_paths(id);
        self.navigate(id, &path, NavKind::NoHistory, select);
    }
}

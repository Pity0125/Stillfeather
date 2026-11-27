//! 檔案操作：新增資料夾、剪下／複製／貼上、複製到另一欄、刪除，背景執行與完成後的更新
//! （復原紀錄、搜尋結果拿掉不在的項目、分頁跟著被搬走的資料夾），以及開啟、內容、複製路徑、PowerShell 與常用位置。
//! 真正的檔案操作（IFileOperation）在 `fileops.rs`。

use super::*;

impl App {
    pub(super) fn new_folder(&mut self) {
        if self.op_busy {
            return;
        }
        let id = self.active_tab_id();
        let Some(tab) = self.tabs.get(&id) else { return };
        if tab.path.is_empty() || tab.working() {
            return;
        }
        let dir = tab.path.clone();
        if self.refuse_share_list(&dir) {
            return;
        }
        crate::win::crashlog::crumb("新增資料夾：建立");
        match fileops::create_new_folder(&dir) {
            Ok(path) if self.tabs.get(&id).is_some_and(|t| t.search_mode) => {
                // 正在看搜尋結果：回到資料夾清單，讀好時選取並改名（同以前）
                crate::win::crashlog::crumb("新增資料夾：已建立，回到資料夾清單");
                if let Some(t) = self.tabs.get_mut(&id) {
                    t.pending_rename = true;
                }
                self.navigate(id, &dir, NavKind::NoHistory, vec![path]);
            }
            Ok(path) => {
                crate::win::crashlog::crumb("新增資料夾：已建立，加進清單");
                // 不重新讀取整個資料夾：直接把新資料夾加進清單、選取它，再（貼出訊息）進入改名
                self.add_new_item(id, &path);
                let msg = Box::into_raw(Box::new(NewItem { tab: Some(id), path, edit: true }));
                if unsafe { PostMessageW(self.hwnd, WM_APP_BEGIN_RENAME, 0, msg as LPARAM) } == 0 {
                    drop(unsafe { Box::from_raw(msg) });
                }
            }
            Err(e) => self.show_error(&trf!("無法新增資料夾：{e}", "Couldn't create the folder: {e}")),
        }
    }

    /// 把剛建立的項目加進分頁的清單（已經有了就不重複）。類型篩選會把它藏起來時換回「全部」。
    /// 回傳它在清單裡的列；分頁正在讀取、項目不在這個資料夾、讀不到項目時回傳 None。
    fn add_new_item(&mut self, id: TabId, path: &str) -> Option<usize> {
        let folders_on_top = self.settings.folders_on_top;
        let tab = self.tabs.get_mut(&id)?;
        if tab.loading || tab.search_mode || !paths::parent(path).is_some_and(|p| paths::same_path(&p, &tab.path)) {
            return None;
        }
        let find = |t: &Tab| (0..t.row_count()).find(|&i| t.row(i).is_some_and(|e| paths::same_path(&e.full_path(), path)));
        if let Some(i) = find(tab) {
            return Some(i);
        }
        if !tab.entries.iter().any(|e| paths::same_path(&e.full_path(), path)) {
            let entry = fs::entry_of(path)?;
            tab.entries.push(entry);
            feather_core::sorting::sort_entries(&mut tab.entries, tab.sort_col, tab.desc, folders_on_top);
        }
        let entry = tab.entries.iter().find(|e| paths::same_path(&e.full_path(), path))?;
        // 篩選「圖片」時新增資料夾：換回「全部」，否則看不到正在改名的資料夾
        if !tab.filter.matches(entry) {
            tab.filter = Category::All;
        }
        tab.refilter();
        tab.resummarize();
        tab.pending_select = vec![path.to_string()];
        if let Some(side) = self.ws.side_of(id).filter(|s| self.bound[s.index()] == Some(id)) {
            self.refresh_list(side, true);
        }
        self.update_chrome();
        let tab = self.tabs.get(&id)?;
        find(tab)
    }

    /// [`WM_APP_BEGIN_RENAME`]：選取剛建立的項目，`edit` 時進入改名。清單還沒有它（Windows「新增 ▸」剛建立）
    /// 就補上；分頁還在讀取時交給讀取完成（`pending_rename`）。
    pub(super) fn begin_rename_new(&mut self, item: NewItem) {
        let NewItem { tab, path, edit } = item;
        crate::win::crashlog::crumb("改名新項目：收到要求");
        let parent = paths::parent(&path).unwrap_or_default();
        let shows = |a: &App, id: TabId| a.tabs.get(&id).is_some_and(|t| !t.search_mode && paths::same_path(&t.path, &parent));
        let id = match tab {
            Some(id) => Some(id),
            None => {
                let active = self.active_side();
                [active, active.other()]
                    .into_iter()
                    .filter(|&s| s == Side::Left || self.ws.dual)
                    .filter_map(|s| self.bound[s.index()])
                    .find(|&id| shows(self, id))
            }
        };
        let Some(id) = id.filter(|&id| shows(self, id)) else {
            crate::win::crashlog::crumb("改名新項目：沒有顯示它所在資料夾的分頁");
            return;
        };
        let Some(side) = self.ws.side_of(id).filter(|s| self.bound[s.index()] == Some(id)) else {
            crate::win::crashlog::crumb("改名新項目：分頁不在畫面上");
            return;
        };
        if self.tabs.get(&id).is_some_and(|t| t.loading) {
            // 正在讀取（例如右鍵選單命令後的重新整理）：讀好時選取並改名
            crate::win::crashlog::crumb("改名新項目：等資料夾讀取完成");
            if let Some(t) = self.tabs.get_mut(&id) {
                t.pending_select = vec![path];
                t.pending_rename = edit;
            }
            return;
        }
        let Some(index) = self.add_new_item(id, &path) else {
            crate::win::crashlog::crumb("改名新項目：清單裡找不到");
            return;
        };
        let list = self.lists[side.index()];
        if side != self.active_side() {
            self.activate(side);
        }
        unsafe {
            SetFocus(list);
        }
        crate::win::pane::select_only(list, index);
        if edit {
            send(list, LVM_EDITLABELW, index, 0);
            let editing = send(list, LVM_GETEDITCONTROL, 0, 0) != 0;
            crate::win::crashlog::crumb(if editing { "改名新項目：已進入編輯" } else { "改名新項目：清單沒有進入編輯" });
        }
    }

    /// 目前的位置是共用資料夾清單（`\\nas`）：這一層不能新增、貼上、搜尋。顯示原因並回傳 true。
    pub(in crate::win) fn refuse_share_list(&mut self, dir: &str) -> bool {
        let Some(server) = paths::unc_server(dir) else { return false };
        self.set_footer(trf!(
            "這裡列出 {server} 分享的資料夾，這一層不能新增、貼上或搜尋。請先開啟其中一個共用資料夾。",
            "This lists the folders {server} shares; you can't create, paste or search at this level. Open one of the shared folders first."
        ));
        true
    }

    pub(super) fn clipboard_files(&mut self, cut: bool) {
        let side = self.active_side();
        let files = self.selected_paths(side);
        self.clipboard_paths(files, cut);
    }

    /// 把指定的檔案或資料夾放上剪貼簿（清單的選取或側欄的資料夾）。
    pub(in crate::win) fn clipboard_paths(&mut self, files: Vec<String>, cut: bool) {
        if files.is_empty() {
            return;
        }
        match crate::win::clipboard::set_files(self.hwnd, &files, cut) {
            Ok(()) => {
                self.update_chrome();
                self.set_footer(trn!(files.len(); "已{} {} 個項目，請至目的地貼上", "{} {} item. Go to the destination and paste it.", "{} {} items. Go to the destination and paste them.", if cut { tr!("剪下", "Cut") } else { tr!("複製", "Copied") }, files.len()));
            }
            Err(e) => self.show_error(&e),
        }
    }

    pub(super) fn paste(&mut self) {
        if self.op_busy {
            return;
        }
        let Some(dest) = self.active_tab().filter(|t| !t.working() && !t.path.is_empty()).map(|t| t.path.clone()) else { return };
        if self.refuse_share_list(&dest) {
            return;
        }
        match crate::win::clipboard::get_files(self.hwnd) {
            Ok(Some((files, moving))) if !files.is_empty() => {
                let cut = moving.then(|| files.clone());
                if self.run_operation(if moving { OpKind::Move } else { OpKind::Copy }, files, Some(dest)) {
                    self.paste_cut = cut;
                }
            }
            Ok(_) => {
                self.set_footer(tr!("剪貼簿中沒有檔案", "There are no files on the clipboard").into());
            }
            Err(e) => self.show_error(&e),
        }
    }

    pub(super) fn copy_across(&mut self) {
        if self.op_busy {
            return;
        }
        let side = self.active_side();
        let files = self.selected_paths(side);
        if files.is_empty() {
            self.set_footer(tr!("請先選取要複製的檔案或資料夾", "First select the files or folders to copy").into());
            return;
        }
        if !self.ws.dual {
            self.set_dual(true);
            self.set_footer(tr!("已開啟雙欄；右欄讀取完成後請再按一次「複製到另一欄」", "Dual pane is on. Once the right pane finishes loading, click “Copy to the other pane” again").into());
            return;
        }
        let other = side.other();
        let Some(dest_tab) = self.ws.current(other).and_then(|id| self.tabs.get(&id)) else { return };
        if dest_tab.loading || dest_tab.path.is_empty() {
            return;
        }
        let dest = dest_tab.path.clone();
        self.run_operation(OpKind::Copy, files, Some(dest));
    }

    /// 刪除選取的項目：移到資源回收筒，`permanent` 時永久刪除。
    pub(super) fn delete_selection(&mut self, permanent: bool) {
        if self.op_busy {
            return;
        }
        let side = self.active_side();
        if self.active_tab().is_none_or(|t| t.working()) {
            return;
        }
        let files = self.selected_paths(side);
        // 刪除後選取原本的下一個項目（沒有下一個就選上一個），鍵盤可以接著操作，與檔案總管相同。
        let next = self.row_after_selection(side);
        let tab = self.active_tab_id();
        if self.delete_paths(files, permanent) {
            self.after_delete = next.map(|p| (tab, p));
        }
    }

    /// 刪除指定的項目，回傳是否開始了刪除。
    /// - 移到資源回收筒：要不要先確認照 Windows 自己的設定（資源回收筒「內容」的「顯示刪除確認對話方塊」，
    ///   Windows 10／11 預設不確認）；完成後狀態列用重點色提示可以 Ctrl+Z 復原。
    /// - `permanent`（Shift+Delete、按住 Shift 點刪除）：一定確認，預設焦點在「取消」；不記錄復原。
    pub(in crate::win) fn delete_paths(&mut self, files: Vec<String>, permanent: bool) -> bool {
        if self.op_busy || files.is_empty() {
            return false;
        }
        let names: Vec<String> = files.iter().take(5).map(|f| paths::file_name(f).to_string()).collect();
        let more = if files.len() > 5 { trf!("\n…等 {} 個項目", "\n…and {} more", files.len()) } else { String::new() };
        if permanent {
            let (heading, body) = if files.len() == 1 {
                (trf!("要永久刪除「{}」嗎？", "Permanently delete “{}”?", names[0]), tr!("不會放進資源回收筒，無法復原。", "It won't go to the Recycle Bin and can't be undone.").to_string())
            } else {
                (
                    trf!("要永久刪除 {} 個項目嗎？", "Permanently delete {} items?", files.len()),
                    trf!("{}{}\n\n不會放進資源回收筒，無法復原。", "{}{}\n\nThey won't go to the Recycle Bin and can't be undone.", names.join("\n"), more),
                )
            };
            if !dialog::confirm_danger(self.hwnd, &heading, &body, tr!("永久刪除", "Delete permanently")) {
                return false;
            }
            return self.run_operation(OpKind::DeletePermanently, files, None);
        }
        if recycle_needs_confirm() {
            let (heading, body) = if files.len() == 1 {
                (trf!("要把「{}」移到資源回收筒嗎？", "Move “{}” to the Recycle Bin?", names[0]), tr!("之後仍可從資源回收筒還原。", "You can still restore it from the Recycle Bin later.").to_string())
            } else {
                (trf!("要把 {} 個項目移到資源回收筒嗎？", "Move {} items to the Recycle Bin?", files.len()), trf!("{}{}\n\n之後仍可從資源回收筒還原。", "{}{}\n\nYou can still restore them from the Recycle Bin later.", names.join("\n"), more))
            };
            if !dialog::confirm(self.hwnd, Icon::Delete, &heading, &body, tr!("移到資源回收筒", "Move to Recycle Bin")) {
                return false;
            }
        }
        self.run_operation(OpKind::Delete, files, None)
    }

    /// 開始背景檔案作業；回傳是否真的開始了。移動不再先確認（剪下貼上、拖放都是明確的動作，
    /// 完成後狀態列提示可以 Ctrl+Z 復原）；目的地已有同名項目時 Windows 仍會詢問。
    fn run_operation(&mut self, kind: OpKind, sources: Vec<String>, dest: Option<String>) -> bool {
        if self.op_busy || sources.is_empty() {
            return false;
        }
        if dest.as_deref().is_some_and(|d| self.refuse_share_list(d)) {
            return false;
        }
        // 共用資料夾本身（共用資料夾清單裡的一列）：要在那台電腦上管理
        if kind != OpKind::Copy && sources.iter().any(|s| paths::is_share_root(s)) {
            self.set_footer(tr!(
                "共用資料夾要在分享它的那台電腦上刪除、移動或改名；可以開啟它，管理裡面的檔案。",
                "Shared folders are deleted, moved or renamed on the computer that shares them; you can open one and manage the files inside."
            )
            .into());
            return false;
        }
        self.op_busy = true;
        self.set_footer(trn!(sources.len(); "正在{} {} 個項目…", "Starting to {} {} item…", "Starting to {} {} items…", kind.verb(), sources.len()));
        self.prepare_undo(kind, &sources, dest.as_deref());
        if kind != OpKind::Copy {
            self.op_sources = sources.clone();
        }
        self.op_dest = if kind == OpKind::Move { dest.clone() } else { None };
        fileops::start(kind, sources, dest, self.hwnd);
        true
    }

    /// 搜尋結果中已被移走或刪除的項目：從結果拿掉（搜尋結果不會重新讀取資料夾）。
    fn drop_gone_search_results(&mut self, sources: &[String]) {
        if sources.is_empty() {
            return;
        }
        let gone: std::collections::HashSet<String> =
            sources.iter().filter(|p| fs::attributes(p).is_none()).map(|p| paths::trim_end_sep(p).to_lowercase()).collect();
        if gone.is_empty() {
            return;
        }
        for side in [Side::Left, Side::Right] {
            let Some(id) = self.bound[side.index()] else { continue };
            let Some(tab) = self.tabs.get_mut(&id) else { continue };
            if !tab.search_mode {
                continue;
            }
            let before = tab.entries.len();
            tab.retain_entries(|e| !gone.contains(&e.full_path().to_lowercase()));
            if tab.entries.len() != before {
                tab.resummarize();
                tab.refilter();
                self.refresh_list(side, true);
            }
        }
        // 沒顯示在畫面上的搜尋分頁也一併更新
        for tab in self.tabs.values_mut().filter(|t| t.search_mode) {
            let before = tab.entries.len();
            tab.retain_entries(|e| !gone.contains(&e.full_path().to_lowercase()));
            if tab.entries.len() != before {
                tab.resummarize();
                tab.refilter();
            }
        }
    }

    pub(super) fn on_file_op(&mut self, report: OpReport) {
        self.op_busy = false;
        let sources = std::mem::take(&mut self.op_sources);
        let dest = self.op_dest.take();
        let paste_cut = self.paste_cut.take();
        let after_delete = self.after_delete.take();
        if !report.undo {
            // 剪下貼上完成：清空剪貼簿，避免再貼一次
            if let Some(files) = paste_cut.filter(|_| report.kind == OpKind::Move && report.completed > 0) {
                crate::win::clipboard::clear_cut_files(self.hwnd, &files);
            }
            self.drop_gone_search_results(&sources);
        }
        // 從側欄刪除的資料夾：不在了就把節點拿掉
        if let Some((item, path)) = self.pending_tree_delete.take() {
            if !fs::is_dir(&path) && self.nodes.get(&item).is_some_and(|n| paths::eq_ignore_case(&n.path, &path)) {
                send(self.tree, TVM_DELETEITEM, 0, item);
                self.nodes.remove(&item);
            }
        }
        let select = if report.undo {
            std::mem::take(&mut self.undo_select)
        } else {
            self.finish_undo_record();
            match after_delete {
                // 只在刪除時所在的分頁還顯示著同一個資料夾時選取（refresh_after_undo 依資料夾比對）
                Some((tab, path)) if report.kind.is_delete() && self.tabs.contains_key(&tab) => vec![path],
                _ => Vec::new(),
            }
        };
        self.refresh_after_undo(&select);
        // 顯示的資料夾被刪掉或搬走的分頁（含隱藏的）：改顯示還在的位置。復原時不跳回原處。
        if !report.undo && report.completed > 0 {
            self.follow_moved_or_deleted(report.kind, &sources, dest.as_deref());
        }
        self.update_chrome();
        if report.undo {
            self.set_footer(trf!(
                "已復原{} · 還原 {} 項{}", "Undid {} · restored {}{}",
                report.kind.verb(),
                report.completed,
                if report.skipped > 0 { trf!(" · {} 項無法還原", " · {} couldn't be restored", report.skipped) } else { String::new() }
            ));
        } else if report.completed > 0 && report.skipped == 0 && report.errors.is_empty() && !report.cancelled {
            // 全部完成：一句話說清楚做了什麼；可以復原的用重點色顯示幾秒（刪除、移動不再先確認）
            let n = report.completed;
            match report.kind {
                OpKind::Delete => self.set_footer_hot(trn!(n; "已把 {} 個項目移到資源回收筒 · Ctrl+Z 可復原", "Moved {} item to the Recycle Bin · Ctrl+Z to undo", "Moved {} items to the Recycle Bin · Ctrl+Z to undo", n)),
                OpKind::Move => self.set_footer_hot(trn!(n; "已移動 {} 個項目 · Ctrl+Z 可復原", "Moved {} item · Ctrl+Z to undo", "Moved {} items · Ctrl+Z to undo", n)),
                OpKind::DeletePermanently => self.set_footer(trn!(n; "已永久刪除 {} 個項目", "Permanently deleted {} item", "Permanently deleted {} items", n)),
                OpKind::Copy => self.set_footer(trn!(n; "已複製 {} 個項目", "Copied {} item", "Copied {} items", n)),
            }
        } else {
            let can_undo = report.completed > 0 && matches!(report.kind, OpKind::Delete | OpKind::Move);
            let text = trf!(
                "{}{} · 完成 {} 項 · 略過 {} 項 · 錯誤 {} 項{}", "{1} {0} · {2} done · {3} skipped · {4} errors{5}",
                report.kind.verb(),
                if report.cancelled { tr!("已取消", "Cancelled") } else { tr!("結束", "Finished") },
                report.completed,
                report.skipped,
                report.errors.len(),
                if can_undo { tr!(" · Ctrl+Z 可復原", " · Ctrl+Z to undo") } else { "" }
            );
            if can_undo {
                self.set_footer_hot(text);
            } else {
                self.set_footer(text);
            }
        }
        if !report.errors.is_empty() {
            let mut text: Vec<String> = report.errors.iter().take(8).cloned().collect();
            if report.errors.len() > 8 {
                text.push(trf!("…等 {} 項", "…and {} more", report.errors.len()));
            }
            let heading = if report.undo {
                trn!(report.errors.len(); "復原{}時有 {} 個項目沒有還原", "Undo {0}: {1} item wasn't restored", "Undo {0}: {1} items weren't restored", report.kind.verb(), report.errors.len())
            } else {
                trn!(report.errors.len(); "{}時有 {} 個項目沒有完成", "{1} item didn't finish ({0})", "{1} items didn't finish ({0})", report.kind.verb(), report.errors.len())
            };
            dialog::Spec::new(Icon::Warning, &heading).body(&text.join("\n")).width(500).show(self.hwnd);
        }
    }

    /// 刪除或移動完成後：顯示的資料夾（或它的上層）被刪掉或搬走的分頁改顯示還在的位置——刪除時是最近一個
    /// 還在的上層，移動時跟到新位置（`watch::redirect_after_op`）。包含隱藏的分頁；搜尋結果另外處理。
    /// 顯示中的分頁直接前往；隱藏的分頁只改路徑、清空內容、保持過期，切回時讀取並在狀態列說明。
    /// 要在 refresh_after_undo 之後呼叫：它會重新讀取顯示中分頁原本的路徑，晚一步才不會蓋掉這裡的導覽。
    fn follow_moved_or_deleted(&mut self, kind: OpKind, sources: &[String], dest: Option<&str>) {
        if kind == OpKind::Copy || sources.is_empty() {
            return;
        }
        let visible: Vec<TabId> = self.ws.visible_sides().iter().filter_map(|s| self.ws.current(*s)).collect();
        let ids: Vec<TabId> = self.tabs.iter().filter(|(_, t)| !t.search_mode && !t.path.is_empty()).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(old) = self.tabs.get(&id).map(|t| t.path.clone()) else { continue };
            let Some(target) = feather_core::watch::redirect_after_op(&old, kind, sources, dest, fs::is_dir) else { continue };
            let source = sources.iter().find(|s| paths::is_same_or_descendant(s, &old)).map_or(String::new(), |s| paths::tab_title(s));
            let went_up = paths::is_same_or_descendant(&target, &old);
            let note = match (went_up, kind) {
                (false, _) => trf!("「{}」已移到「{}」，分頁跟著過去", "“{}” was moved to “{}”; the tab followed it", source, paths::tab_title(dest.unwrap_or(&target))),
                (true, OpKind::Move) => trf!("「{}」已被移走，改為顯示「{}」", "“{}” was moved away, so “{}” is shown instead", source, paths::tab_title(&target)),
                (true, _) => trf!("「{}」已被刪除，改為顯示「{}」", "“{}” was deleted, so “{}” is shown instead", source, paths::tab_title(&target)),
            };
            crate::win::crashlog::crumb("檔案操作後：分頁顯示的資料夾已不在，改顯示還在的位置");
            if visible.contains(&id) {
                self.set_footer(note);
                self.navigate(id, &target, NavKind::NoHistory, vec![]);
            } else if let Some(t) = self.tabs.get_mut(&id) {
                t.path = target.clone();
                t.requested = target;
                t.entries.clear();
                t.scores.clear();
                t.refilter();
                t.resummarize();
                t.view = ViewState::default();
                t.stale = true;
                t.notice = Some(note);
            }
        }
    }

    /// 檔案操作或復原之後重新讀取畫面上的分頁；`select` 所在資料夾的分頁會選取這些項目。
    pub(in crate::win) fn refresh_after_undo(&mut self, select: &[String]) {
        for side in self.ws.visible_sides().to_vec() {
            let Some(id) = self.ws.current(side) else { continue };
            let Some(tab) = self.tabs.get(&id) else { continue };
            if tab.search_mode {
                continue;
            }
            let here: Vec<String> = select
                .iter()
                .filter(|p| paths::parent(p).is_some_and(|d| paths::eq_ignore_case(paths::trim_end_sep(&d), paths::trim_end_sep(&tab.path))))
                .cloned()
                .collect();
            if here.is_empty() {
                self.refresh_tab(id);
            } else {
                let dir = tab.path.clone();
                self.navigate(id, &dir, NavKind::NoHistory, here);
            }
        }
        // 隱藏中的分頁切回時再讀取。
        let visible: Vec<TabId> = self.ws.visible_sides().iter().filter_map(|s| self.ws.current(*s)).collect();
        for (id, t) in self.tabs.iter_mut() {
            if !visible.contains(id) && !t.path.is_empty() {
                t.stale = true;
            }
        }
    }

    pub(super) fn properties(&mut self) {
        let side = self.active_side();
        // 焦點在側欄（Alt+Enter）：顯示側欄選取的資料夾
        let tree = (unsafe { GetFocus() } == self.tree).then(|| self.tree_caret_path()).flatten();
        let target = tree
            .or_else(|| self.selected_paths(side).into_iter().next())
            .or_else(|| self.active_tab().map(|t| t.path.clone()).filter(|p| !p.is_empty()));
        if let Some(p) = target {
            fileops::show_properties(self.hwnd, &p);
        }
    }

    pub(super) fn copy_path(&mut self) {
        let side = self.active_side();
        let mut files = self.selected_paths(side);
        if files.is_empty() {
            if let Some(p) = self.active_tab().map(|t| t.path.clone()).filter(|p| !p.is_empty()) {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        match crate::win::clipboard::set_text(self.hwnd, &files.join("\r\n")) {
            Ok(()) => {
                self.update_chrome();
                self.set_footer(tr!("已複製路徑", "Path copied").into());
            }
            Err(e) => self.show_error(&e),
        }
    }

    pub(super) fn open_in_new_tab(&mut self) {
        let side = self.active_side();
        let Some(entry) = self.selected_entries(side).into_iter().next() else { return };
        let path = if entry.is_dir { entry.full_path() } else { entry.dir.to_string() };
        self.new_tab(side, &path);
    }

    /// 清單右鍵選單的「在這裡開啟 PowerShell 視窗」：選了一個資料夾時開在它裡面，否則開在目前的資料夾
    pub(super) fn open_powershell_here(&mut self) {
        let side = self.active_side();
        let entries = self.selected_entries(side);
        let folder = match entries.as_slice() {
            [e] if e.is_dir => Some(e.full_path()),
            _ => self.active_tab().filter(|t| !t.search_mode && !t.path.is_empty()).map(|t| t.path.clone()),
        };
        if let Some(folder) = folder {
            self.open_powershell(&folder);
        }
    }

    pub(in crate::win) fn open_powershell(&mut self, folder: &str) {
        if let Err(e) = crate::win::fileops::open_powershell(self.hwnd, folder) {
            self.show_error(&e);
        }
    }

    pub(super) fn open_location(&mut self) {
        let side = self.active_side();
        let Some(entry) = self.selected_entries(side).into_iter().next() else { return };
        let id = self.active_tab_id();
        self.navigate(id, &entry.dir, NavKind::Normal, vec![entry.full_path()]);
    }

    pub(in crate::win) fn open_selection(&mut self, side: Side) {
        // 選擇器：資料夾就進去，檔案就選定（不用預設程式開啟）
        if self.picker_mode() {
            self.picker_activate(side);
            return;
        }
        let Some(tab) = self.ws.current(side).and_then(|id| self.tabs.get(&id)) else { return };
        if tab.loading {
            return;
        }
        let entries = self.selected_entries(side);
        let Some(entry) = entries.first().cloned() else { return };
        if entries.len() > 1 {
            self.open_many(side, entries);
            return;
        }
        if entry.is_dir {
            let id = self.ws.current(side).unwrap_or(0);
            self.navigate(id, &entry.full_path(), NavKind::Normal, vec![]);
        } else if let Err(e) = fileops::shell_open(self.hwnd, &entry.full_path()) {
            if let Some(t) = self.ws.current(side).and_then(|id| self.tabs.get_mut(&id)) {
                t.status = trf!("無法完成：{e}", "Couldn't complete: {e}");
            }
            self.update_chrome();
        }
    }

    /// 選了多個項目按 Enter（或雙擊）：檔案全部用預設程式開啟，資料夾各開一個新分頁；
    /// 與檔案總管相同，超過 15 個先確認。
    fn open_many(&mut self, side: Side, entries: Vec<Entry>) {
        const ASK_ABOVE: usize = 15;
        if entries.len() > ASK_ABOVE {
            let heading = trf!("要開啟 {} 個項目嗎？", "Open {} items?", entries.len());
            let body = tr!("每個檔案都會用預設程式開啟，資料夾會各開一個新分頁。", "Each file opens in its default app, and each folder opens in a new tab.");
            if !dialog::confirm(self.hwnd, Icon::Question, &heading, body, tr!("全部開啟", "Open all")) {
                return;
            }
        }
        let mut failed: Vec<String> = Vec::new();
        for e in entries.iter().filter(|e| !e.is_dir) {
            if let Err(err) = fileops::shell_open(self.hwnd, &e.full_path()) {
                failed.push(trf!("{}：{err}", "{}: {err}", e.name));
            }
        }
        for e in entries.iter().filter(|e| e.is_dir) {
            self.new_tab(side, &e.full_path());
        }
        if !failed.is_empty() {
            let mut text: Vec<String> = failed.iter().take(8).cloned().collect();
            if failed.len() > 8 {
                text.push(trf!("…等 {} 項", "…and {} more", failed.len()));
            }
            dialog::Spec::new(Icon::Warning, &trn!(failed.len(); "有 {} 個項目無法開啟", "{} item couldn't be opened", "{} items couldn't be opened", failed.len())).body(&text.join("\n")).width(500).show(self.hwnd);
        }
        self.update_chrome();
    }

    pub(in crate::win) fn add_favorites(&mut self, candidates: Vec<String>) {
        self.pin_to_group(candidates, None);
    }

    pub(in crate::win) fn pin_to_group(&mut self, candidates: Vec<String>, group: Option<&str>) {
        let mut settings = self.settings.clone();
        let mut added = 0;
        let mut ignored = 0;
        for p in candidates {
            let p = paths::trim_end_sep(&p).to_string();
            if !fs::is_dir(&p) || !paths::is_fully_qualified(&p) {
                ignored += 1;
                continue;
            }
            if !settings.favorites.iter().any(|f| paths::eq_ignore_case(f, &p)) {
                if settings.favorites.len() >= feather_core::settings::MAX_FAVORITES {
                    ignored += 1;
                    continue;
                }
                settings.favorites.push(p.clone());
            } else if group.is_none() {
                // 已經釘選、放在某個標籤裡：拖到「常用位置」就移回常用位置
                if settings.group_of(&p).is_some() {
                    settings.move_bookmark(&p, None);
                    added += 1;
                }
                continue;
            }
            if let Some(group) = group {
                for (name, members) in &mut settings.bookmark_groups {
                    members.retain(|f| !paths::eq_ignore_case(f, &p));
                    if name == group { members.push(p.clone()); }
                }
            }
            added += 1;
        }
        if added > 0 {
            if let Err(e) = write_settings(&settings) {
                self.show_error(&e);
                return;
            }
            self.settings = settings;
            self.build_tree();
        }
        self.update_chrome();
        let skipped = if ignored > 0 { trf!("，{ignored} 個不是資料夾或已達上限", "; {ignored} weren't folders or the limit was reached") } else { String::new() };
        self.set_footer(match group {
            Some(name) => trn!(added; "已把 {added} 個資料夾放進「{name}」{skipped}", "Added {added} folder to “{name}”{skipped}", "Added {added} folders to “{name}”{skipped}"),
            None => trn!(added; "已釘選 {added} 個資料夾到常用位置{skipped}", "Pinned {added} folder to Favorites{skipped}", "Pinned {added} folders to Favorites{skipped}"),
        });
    }

    pub(in crate::win) fn remove_favorite(&mut self, path: &str) {
        self.remove_favorites(&[path.to_string()]);
    }

    /// 從常用位置取消釘選（資料夾本身不動）。
    pub(in crate::win) fn remove_favorites(&mut self, targets: &[String]) {
        let mut settings = self.settings.clone();
        let before = settings.favorites.len();
        settings.favorites.retain(|f| !targets.iter().any(|t| paths::eq_ignore_case(f, paths::trim_end_sep(t))));
        for (_, members) in &mut settings.bookmark_groups {
            members.retain(|p| !targets.iter().any(|t| paths::eq_ignore_case(p, paths::trim_end_sep(t))));
        }
        let removed = before - settings.favorites.len();
        if removed > 0 {
            if let Err(e) = write_settings(&settings) {
                self.show_error(&e);
                return;
            }
            self.settings = settings;
            self.build_tree();
            self.update_chrome();
            self.set_footer(trn!(removed; "已從常用位置取消釘選 {removed} 個資料夾（資料夾本身沒有刪除）", "Unpinned {removed} folder from Favorites (the folder itself wasn't deleted)", "Unpinned {removed} folders from Favorites (the folders themselves weren't deleted)"));
        }
    }

    /// 這個資料夾是否已釘選在常用位置。
    pub(in crate::win) fn is_pinned(&self, path: &str) -> bool {
        let p = paths::trim_end_sep(path);
        self.settings.favorites.iter().any(|f| paths::eq_ignore_case(f, p))
    }
}

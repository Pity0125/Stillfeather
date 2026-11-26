//! 分頁與雙欄：新增、關閉、重新開啟剛關掉的分頁、切換、調整順序與拖放，單欄／雙欄，
//! 其他程式要求開啟的位置，以及還原上次的分頁（格式見 `feather_core::session`）。

use super::*;

impl App {
    pub(in crate::win) fn new_tab(&mut self, side: Side, path: &str) -> TabId {
        self.new_tab_with(side, path, false)
    }

    /// `reveal`：`path` 還不知道是檔案還是資料夾（見 navigate_or_reveal）
    fn new_tab_with(&mut self, side: Side, path: &str, reveal: bool) -> TabId {
        // 選擇器沒有分頁：「在新分頁開啟」就是在原地開啟
        if self.picker_mode() {
            let id = self.active_tab_id();
            self.navigate_with(id, path, NavKind::Normal, vec![], reveal);
            return id;
        }
        let side = if self.ws.dual { side } else { Side::Left };
        let _fade = self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS);
        let id = self.ws.add_tab(side);
        self.tabs.insert(id, Tab::new());
        self.navigate_with(id, path, NavKind::NoHistory, vec![], reveal);
        self.bind(side);
        self.update_chrome();
        id
    }

    pub(super) fn new_tab_active(&mut self) {
        let path = self.active_tab().map(|t| t.display_path()).filter(|p| !p.is_empty()).unwrap_or_else(|| self.default_path.clone());
        self.new_tab(self.active_side(), &path);
    }

    pub(in crate::win) fn close_tab(&mut self, side: Side, index: usize) {
        // 記下來，Ctrl+Shift+T 可以重新開啟（位置、路徑、上一頁／下一頁紀錄、選取的項目）
        let closing = self.ws.group(side).tabs.get(index).copied().and_then(|id| {
            let t = self.tabs.get(&id)?;
            let path = t.display_path();
            (!path.is_empty()).then(|| ClosedTab { side, index, path, back: t.back.clone(), forward: t.forward.clone(), select: self.bound_selection_paths(id) })
        });
        self.hide_preview();
        let _fade = self.crossfade(vec![self.whole_content_area()], anim::CROSSFADE_MS);
        let was_dual = self.ws.dual;
        match self.ws.close(side, index) {
            CloseResult::Nothing => {}
            CloseResult::CloseWindow => unsafe {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },
            CloseResult::Closed(removed) => {
                if let Some(c) = closing {
                    if self.closed_tabs.len() >= CLOSED_TABS_MAX {
                        self.closed_tabs.remove(0);
                    }
                    self.closed_tabs.push(c);
                }
                for id in removed {
                    if let Some(t) = self.tabs.remove(&id) {
                        t.cancel.store(true, Ordering::Relaxed);
                    }
                    for b in self.bound.iter_mut() {
                        if *b == Some(id) {
                            *b = None;
                        }
                    }
                }
                for side in [Side::Left, Side::Right] {
                    if let Some(b) = self.bound[side.index()] {
                        if self.ws.side_of(b) != Some(side) {
                            self.bound[side.index()] = None;
                        }
                    }
                }
                if was_dual != self.ws.dual {
                    self.relayout();
                }
                self.bind(Side::Left);
                if self.ws.dual {
                    self.bind(Side::Right);
                }
                self.update_chrome();
            }
        }
    }

    /// Ctrl+Shift+T：重新開啟剛關掉的分頁，放回原來的位置，上一頁／下一頁紀錄與選取的項目也還原。
    /// 原本在右欄、現在是單欄時放到左欄最後。
    pub(super) fn reopen_closed_tab(&mut self) {
        if self.picker_mode() {
            return;
        }
        let Some(c) = self.closed_tabs.pop() else {
            self.set_footer(tr!("沒有剛關掉的分頁", "No recently closed tabs").into());
            return;
        };
        let (side, index) = if c.side == Side::Right && !self.ws.dual { (Side::Left, usize::MAX) } else { (c.side, c.index) };
        self.hide_preview();
        let _fade = self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS);
        let id = self.ws.insert_tab(side, index);
        let mut tab = Tab::new();
        tab.back = c.back;
        tab.forward = c.forward;
        self.tabs.insert(id, tab);
        self.navigate(id, &c.path, NavKind::NoHistory, c.select);
        self.footer_sticky = false;
        self.bind(side);
        self.update_chrome();
    }

    /// Ctrl+1～8：跳到作用中那一欄的第 N 個分頁；Ctrl+9：最後一個（同瀏覽器）
    pub(super) fn go_to_tab(&mut self, n: u8) {
        let side = self.active_side();
        let len = self.ws.group(side).tabs.len();
        if len == 0 || n == 0 {
            return;
        }
        let index = if n >= 9 { len - 1 } else { n as usize - 1 };
        if index < len {
            self.select_tab(side, index);
        }
    }

    pub(super) fn close_current_tab(&mut self) {
        let side = self.active_side();
        let index = self.ws.group(side).selected;
        self.close_tab(side, index);
    }

    pub(in crate::win) fn select_tab(&mut self, side: Side, index: usize) {
        if side == Side::Right && !self.ws.dual {
            return;
        }
        if index >= self.ws.group(side).tabs.len() {
            self.activate(side);
            return;
        }
        self.hide_preview();
        let _fade = if self.ws.group(side).selected != index {
            self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
        } else {
            anim::Crossfade::none()
        };
        self.footer_sticky = false;
        self.ws.select(side, index);
        self.bind(side);
        self.update_chrome();
    }

    pub(super) fn switch_tab(&mut self, delta: isize) {
        let side = self.active_side();
        let _fade = if self.ws.group(side).tabs.len() > 1 {
            self.crossfade(self.content_fade_areas(side), anim::CROSSFADE_MS)
        } else {
            anim::Crossfade::none()
        };
        self.ws.active = side;
        self.ws.switch(delta);
        self.footer_sticky = false;
        self.hide_preview();
        self.bind(side);
        self.update_chrome();
    }

    pub(super) fn move_tab(&mut self, side: Side, index: usize, delta: isize) {
        if self.ws.move_tab(side, index, delta) {
            self.update_chrome();
        }
    }

    pub(in crate::win) fn set_dual(&mut self, enabled: bool) {
        if self.picker_mode() {
            return;
        }
        self.hide_preview();
        let _fade = if enabled != self.ws.dual { self.crossfade(vec![self.whole_content_area()], anim::CROSSFADE_MS + 40.0) } else { anim::Crossfade::none() };
        match self.ws.set_dual(enabled) {
            DualChange::Unchanged => return,
            DualChange::Shown { new_right_tab } => {
                if let Some(id) = new_right_tab {
                    self.tabs.insert(id, Tab::new());
                    let path = self.tabs.get(&self.ws.current(Side::Left).unwrap_or(0)).map(|t| t.display_path()).unwrap_or_default();
                    let path = if path.is_empty() { self.default_path.clone() } else { path };
                    self.navigate(id, &path, NavKind::NoHistory, vec![]);
                }
                self.relayout();
                self.bind(Side::Right);
                // 右欄收起期間沒有監看：補讀一次
                if let Some(id) = self.ws.current(Side::Right) {
                    self.watch_catch_up(id);
                }
            }
            DualChange::Hidden => {
                self.relayout();
            }
        }
        self.update_chrome();
    }

    pub(super) fn tab_action(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let side = Side::from_index(wparam & 0xF);
        let Some(action) = tabs::Action::from_bits((wparam >> 4) & 0xF) else { return };
        let index = wparam >> 8;
        match action {
            tabs::Action::Select => self.select_tab(side, index),
            tabs::Action::Close => self.close_tab(side, index),
            tabs::Action::New => {
                self.activate(side);
                let path = self
                    .ws
                    .current(side)
                    .and_then(|id| self.tabs.get(&id))
                    .map(|t| t.display_path())
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| self.default_path.clone());
                self.new_tab(side, &path);
            }
            tabs::Action::Menu => {
                let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
                self.tab_menu(side, index, pt);
            }
            tabs::Action::Detach => self.detach_tab(side, index, POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) }),
            tabs::Action::Move => {
                let to = Side::from_index(((lparam as usize) >> 16) & 0xF);
                let gap = (lparam as usize) & 0xFFFF;
                self.drop_tab(side, index, to, gap);
            }
        }
    }

    /// 拖曳分頁放開：同一欄是調整順序，另一欄（雙欄時）是移過去並選取它
    fn drop_tab(&mut self, from: Side, index: usize, to: Side, gap: usize) {
        if (from == Side::Right || to == Side::Right) && !self.ws.dual {
            return;
        }
        let across = from != to;
        let _fade = if across { self.crossfade(vec![self.whole_content_area()], anim::CROSSFADE_MS) } else { anim::Crossfade::none() };
        if !self.ws.move_tab_across(from, index, to, gap) {
            return;
        }
        if across {
            self.hide_preview();
            // 兩欄顯示的分頁可能都換了
            self.bind(from);
            self.bind(to);
        }
        self.update_chrome();
        invalidate(self.strips[0]);
        invalidate(self.strips[1]);
    }

    fn tab_menu(&mut self, side: Side, index: usize, pt: POINT) {
        let count = self.ws.group(side).tabs.len();
        let valid = index < count;
        if valid {
            self.select_tab(side, index);
        }
        let mut items: Vec<(u16, &str, bool)> = vec![(id::TAB_NEW, tr!("新增分頁\tCtrl+T", "New tab\tCtrl+T"), true)];
        if valid {
            items.push((id::TAB_DUPLICATE, tr!("複製分頁", "Duplicate tab"), true));
            items.push((id::TAB_MOVE_LEFT, tr!("向左移\tCtrl+Shift+PgUp", "Move left\tCtrl+Shift+PgUp"), index > 0));
            items.push((id::TAB_MOVE_RIGHT, tr!("向右移\tCtrl+Shift+PgDn", "Move right\tCtrl+Shift+PgDn"), index + 1 < count));
            items.push((0, "", true));
            items.push((id::TAB_CLOSE, tr!("關閉分頁\tCtrl+W", "Close tab\tCtrl+W"), true));
        }
        let labels: Vec<_> = items.iter().map(|(id, text, _)| match shortcut_command(*id) {
            Some(cmd) => self.keymap.with_hint(text.split('\t').next().unwrap_or(text), cmd, "\t"),
            None => text.to_string(),
        }).collect();
        for (item, label) in items.iter_mut().zip(&labels) { item.1 = label; }
        let Some(cmd) = popup_menu(self.hwnd, &items, pt) else { return };
        match cmd {
            id::TAB_NEW => self.tab_action(side.index() | ((tabs::Action::New as usize) << 4), 0),
            id::TAB_DUPLICATE => {
                if let Some(path) = self.ws.group(side).tabs.get(index).and_then(|id| self.tabs.get(id)).map(|t| t.display_path()) {
                    self.new_tab(side, &path);
                }
            }
            id::TAB_MOVE_LEFT => self.move_tab(side, index, -1),
            id::TAB_MOVE_RIGHT => self.move_tab(side, index, 1),
            id::TAB_CLOSE => self.close_tab(side, index),
            _ => {}
        }
    }

    /// 與 C# 版相同：相同路徑字串的分頁直接聚焦；檔案路徑開啟父資料夾並選取。
    /// 切到這個分頁（在收起的右欄時先開雙欄）
    fn show_tab(&mut self, id: TabId) {
        if self.ws.side_of(id) == Some(Side::Right) && !self.ws.dual {
            self.set_dual(true);
        }
        self.ws.activate_tab(id);
        if let Some(side) = self.ws.side_of(id) {
            self.bind(side);
        }
    }

    /// 開啟要求的路徑（命令列、另一個 Stillfeather 轉過來的、Win+E）。`restored`：啟動時已經還原了上次的分頁，
    /// 要求的路徑開在新分頁（已經開著同一個位置就切過去），沒有要求時就維持還原的分頁。
    pub(in crate::win) fn open_paths(&mut self, requested: Vec<String>, startup: bool, restored: bool) {
        if !startup {
            unsafe {
                if IsIconic(self.hwnd) != 0 {
                    ShowWindow(self.hwnd, SW_RESTORE);
                }
                SetForegroundWindow(self.hwnd);
            }
        }
        // Win+E：啟動時開首頁；已經開著就在新分頁開首頁（像檔案總管每次開新視窗）
        let home = requested.iter().any(|p| p == feather_core::install::HOME_MARKER);
        let requested: Vec<String> = requested.into_iter().filter(|p| p != feather_core::install::HOME_MARKER).collect();
        if home && requested.is_empty() && (!startup || restored) {
            let path = self.default_path.clone();
            // 剛還原了上次的分頁、其中已經有首頁：切過去就好，不再多開一個
            if restored {
                let home_tab = self.ws.all_tabs().find(|id| self.tabs.get(id).is_some_and(|t| paths::same_request_path(&t.display_path(), &path)));
                if let Some(id) = home_tab {
                    self.show_tab(id);
                    self.update_chrome();
                    return;
                }
            }
            let side = self.active_side();
            self.new_tab(side, &path);
            self.update_chrome();
            return;
        }
        if requested.is_empty() {
            if startup && !restored {
                let id = self.active_tab_id();
                let path = self.default_path.clone();
                self.navigate(id, &path, NavKind::NoHistory, vec![]);
            }
            return;
        }
        let mut first = startup && !restored;
        for raw in requested {
            let full = full_path(&raw).unwrap_or(raw);
            // 網路上的位置：在這裡（介面執行緒）判斷是不是檔案，離線時要等到 SMB 逾時（約 20 秒），整個視窗凍住；
            // 改成先當資料夾在背景讀取，其實是檔案時再改開它所在的資料夾並選取（見 navigate_or_reveal）
            let reveal = !fs::on_local_drive(&full);
            let (path, select) = if !reveal && fs::is_file(&full) {
                (paths::parent(&full).unwrap_or_else(|| full.clone()), vec![full.clone()])
            } else {
                (full.clone(), vec![])
            };
            let existing = self.ws.all_tabs().find(|id| self.tabs.get(id).is_some_and(|t| paths::same_request_path(&t.display_path(), &path)));
            if let Some(id) = existing {
                self.show_tab(id);
                if !select.is_empty() {
                    self.navigate(id, &path, NavKind::NoHistory, select);
                }
            } else if first {
                let id = self.ws.current(Side::Left).unwrap_or(0);
                self.navigate_with(id, &path, NavKind::NoHistory, select, reveal);
            } else {
                let side = self.active_side();
                let id = self.new_tab_with(side, &path, reveal);
                if !select.is_empty() {
                    self.navigate(id, &path, NavKind::NoHistory, select);
                }
            }
            first = false;
        }
        self.update_chrome();
    }

    /// 還原的分頁第一次顯示：現在才讀取
    pub(in crate::win) fn load_restored(&mut self, id: TabId) {
        let Some(t) = self.tabs.get_mut(&id) else { return };
        if !t.restored {
            return;
        }
        t.restored = false;
        let path = t.requested.clone();
        self.navigate(id, &path, NavKind::NoHistory, vec![]);
    }

    /// 啟動時（設定「還原上次的分頁」開著）照上次關閉時的樣子開回分頁。只有顯示中的分頁馬上讀取，其他的切過去時才讀。
    /// 回傳是否還原了。
    pub(super) fn restore_session(&mut self) -> bool {
        if !self.settings.restore_tabs {
            return false;
        }
        let Some(session) = take_session() else { return false };
        // 左欄的第一個分頁沿用啟動時建立的那一個
        let first = self.ws.current(Side::Left).unwrap_or(0);
        for (side, saved) in [(Side::Left, &session.left), (Side::Right, &session.right)] {
            for (i, path) in saved.tabs.iter().enumerate() {
                let id = if side == Side::Left && i == 0 {
                    first
                } else {
                    let id = self.ws.add_tab(side);
                    self.tabs.insert(id, Tab::new());
                    id
                };
                if let Some(t) = self.tabs.get_mut(&id) {
                    t.requested = path.clone();
                    t.status = tr!("切換到這個分頁時讀取", "Loads when you switch to this tab").into();
                    t.restored = true;
                }
            }
        }
        if !session.right.tabs.is_empty() {
            self.ws.select(Side::Right, session.right.selected);
        }
        self.ws.select(Side::Left, session.left.selected);
        if session.dual {
            // 還沒顯示視窗：直接開雙欄，不播放切換動畫
            self.ws.set_dual(true);
            if session.active == 1 {
                self.ws.active = Side::Right;
            }
            self.relayout();
        }
        for side in [Side::Left, Side::Right] {
            if side == Side::Left || self.ws.dual {
                self.bind(side);
                if let Some(id) = self.ws.current(side) {
                    self.load_restored(id);
                }
            }
        }
        true
    }

    /// 主視窗關閉、Windows 登出或關機時：設定開著就記下分頁；關著就刪掉舊的記錄。選擇器不記。
    pub(in crate::win) fn save_session(&self) {
        if self.detached || self.picker_mode() {
            return;
        }
        let Ok(path) = session_path() else { return };
        if !self.settings.restore_tabs {
            let _ = std::fs::remove_file(&path);
            return;
        }
        let side_tabs = |side: Side| {
            let g = self.ws.group(side);
            let mut saved = SideTabs::default();
            for (i, id) in g.tabs.iter().enumerate() {
                let Some(p) = self.tabs.get(id).map(Tab::display_path).filter(|p| !p.is_empty()) else { continue };
                if i == g.selected {
                    saved.selected = saved.tabs.len();
                }
                saved.tabs.push(p);
            }
            saved
        };
        let session = Session { dual: self.ws.dual, active: self.ws.active.index(), left: side_tabs(Side::Left), right: side_tabs(Side::Right) }.sanitize();
        if session.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if write_atomic(&path, &session.to_json()).is_err() {
            crate::win::crashlog::crumb("分頁記錄寫入失敗");
        }
    }
}

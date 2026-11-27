//! 設定與系統的變更：設定視窗、套用設定、顯示隱藏的項目、語言改了要不要重新開啟、預設檔案總管、
//! DPI 與系統深淺色改變，以及殼層命令（右鍵選單）完成後的重新整理。

use super::*;

impl App {
    pub(in crate::win) fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        // 整體設定：記到設定檔（不分資料夾）
        let mut new = self.settings.clone();
        new.show_hidden = self.show_hidden;
        match write_settings(&new) {
            Ok(()) => self.settings = new,
            Err(e) => {
                self.settings.show_hidden = self.show_hidden;
                self.set_footer(trf!("隱藏項目已切換，但無法儲存設定：{e}", "Hidden items were toggled, but the settings couldn't be saved: {e}"));
            }
        }
        let visible: Vec<TabId> = self.ws.visible_sides().iter().filter_map(|s| self.ws.current(*s)).collect();
        for (id, t) in self.tabs.iter_mut() {
            if !visible.contains(id) && !t.path.is_empty() {
                t.stale = true;
            }
        }
        for id in visible {
            if self.tabs.get(&id).is_some_and(|t| !t.search_mode) {
                self.refresh_tab(id);
            }
        }
        self.build_tree();
        self.update_chrome();
    }

    pub(super) fn reload_shared_preferences(&mut self) {
        if self.picker_mode() { return }
        let (settings, error) = load_settings();
        if error.is_none() && LAST_SETTINGS.lock().unwrap().as_ref() != Some(&settings) {
            *LAST_SETTINGS.lock().unwrap() = Some(settings.clone());
            self.show_hidden = settings.show_hidden;
            self.apply_settings(settings);
        }
        let (views, error) = load_folder_views();
        if error.is_none() && LAST_VIEWS.lock().unwrap().as_ref() != Some(&views) {
            *LAST_VIEWS.lock().unwrap() = Some(views.clone());
            self.views = views;
            self.apply_view();
        }
    }

    pub(super) fn open_settings(&mut self) {
        self.hide_preview();
        let current = self.settings.clone();
        let exe = current_exe_path();
        // 「攔截 Win+E」與「其他程式開啟或儲存檔案時也用 Stillfeather」顯示使用者的選擇（沒選過時：攔截 Win+E 開啟、開檔視窗關閉）
        let was = settings_dialog::Integration {
            default_fm: exe.as_deref().is_some_and(shellreg::is_default_file_manager),
            win_e: shellreg::win_e_choice(),
            picker: shellreg::picker_choice(),
        };
        if let Some((new, want)) = settings_dialog::show(self.hwnd, &current, was) {
            let english = new.language.english(system_ui_chinese());
            if current != self.settings {
                self.show_error(tr!("設定已在其他視窗變更，請重新開啟設定。", "Settings changed in another window. Reopen Settings to edit the latest values."));
                return;
            }
            let saved = match write_settings(&new) {
                Ok(()) => {
                    self.apply_settings(new);
                    true
                }
                Err(e) => {
                    self.show_error(&trf!("設定未儲存，保留先前設定：{e}", "Settings weren't saved; the previous settings are kept: {e}"));
                    false
                }
            };
            if want.win_e != was.win_e {
                let _ = shellreg::set_win_e_choice(want.win_e);
            }
            if want.default_fm != was.default_fm {
                self.set_default_file_manager(want.default_fm);
            } else if want.win_e != was.win_e && want.default_fm {
                self.set_footer(if want.win_e { tr!("Win+E 會開 Stillfeather（在背景攔截按鍵）", "Win+E opens Stillfeather (the key is intercepted in the background)").into() } else { tr!("已停止攔截 Win+E", "Stopped intercepting Win+E").into() });
            }
            if want.default_fm != was.default_fm || want.win_e != was.win_e {
                // 依「設為預設檔案總管」與「攔截 Win+E」啟動或停止背景攔截
                crate::win::wine::maintain();
            }
            if want.picker != was.picker {
                self.set_picker_enabled(want.picker);
            }
            // 換了語言：重新開啟才套用（畫面上的文字都是建立時就決定的）
            if saved && english != feather_core::i18n::english() {
                self.offer_language_restart(english);
            }
        }
        // 在設定裡按「檢查更新」找到新版本：現在提醒
        self.after_settings_closed();
    }

    /// 其他程式的「在資料夾中顯示」要選取 `path`（見 shellwin.rs）：作用中的分頁就是它所在的資料夾時在這個分頁選取
    /// （清單上已經有它就直接選取；剛下載好、清單還沒更新到時重新讀取資料夾再選取），否則照一般的開啟要求
    /// （開它所在的資料夾並選取）。`replace`：取消其他的選取。視窗拉到前面。
    pub(in crate::win) fn shell_select(&mut self, path: String, replace: bool) {
        let side = self.active_side();
        let id = self.active_tab_id();
        let Some(folder) = paths::parent(&path) else { return };
        let here = self.tabs.get(&id).is_some_and(|t| !t.search_mode && paths::same_path(&folder, &t.path));
        if !here {
            crate::win::shellwin::log(&format!("選取 {path}：不在作用中的資料夾，照一般的開啟要求"));
            self.open_paths(vec![path], false, false);
            return;
        }
        let front = unsafe {
            if IsIconic(self.hwnd) != 0 {
                ShowWindow(self.hwnd, SW_RESTORE);
            }
            SetForegroundWindow(self.hwnd) != 0
        };
        if !front {
            crate::win::shellwin::log("無法把視窗拉到前面（Windows 不允許）");
        }
        let Some(t) = self.tabs.get(&id) else { return };
        // 同一次要求選取好幾個時會連續呼叫：讀取中就接在還沒套用的選取後面
        let mut selected = if replace {
            Vec::new()
        } else if t.loading {
            t.pending_select.clone()
        } else {
            self.selected_paths(side)
        };
        if !selected.iter().any(|p| paths::same_path(p, &path)) {
            selected.push(path.clone());
        }
        let shown = !t.loading && (0..t.row_count()).any(|i| t.row(i).is_some_and(|e| paths::same_path(&e.full_path(), &path)));
        crate::win::shellwin::log(&format!("選取 {path}（{}）", if shown { "清單上已經有" } else { "重新讀取資料夾再選取" }));
        if shown {
            if let Some(t) = self.tabs.get_mut(&id) {
                t.pending_select = selected;
            }
            self.refresh_list(side, true);
        } else {
            let folder = self.tabs[&id].path.clone();
            self.navigate(id, &folder, NavKind::NoHistory, selected);
        }
        self.focus_list(side);
    }

    /// 問要不要現在重新開啟，套用新的語言。問題用新的語言寫（使用者剛選了它）。
    fn offer_language_restart(&mut self, english: bool) {
        let old = feather_core::i18n::english();
        feather_core::i18n::set_english(english);
        let body = if self.settings.restore_tabs {
            tr!("新的語言會在重新開啟 Stillfeather 後套用。分頁會照現在的樣子開回來。", "The new language applies after Stillfeather restarts. Your tabs will be reopened as they are now.")
        } else {
            tr!("新的語言會在重新開啟 Stillfeather 後套用。", "The new language applies after Stillfeather restarts.")
        };
        let choice = dialog::Spec::new(Icon::Question, tr!("要現在重新開啟嗎？", "Restart now?"))
            .body(body)
            .buttons(vec![dialog::Button::primary(1, tr!("立即重新開啟", "Restart now")), dialog::Button::normal(0, tr!("稍後", "Later"))], 0)
            .show(self.hwnd);
        feather_core::i18n::set_english(old);
        if choice == 1 {
            self.restart();
        } else {
            self.set_footer(tr!("新的語言會在下次開啟 Stillfeather 時套用", "The new language applies the next time Stillfeather starts").into());
        }
    }

    /// 重新開啟 Stillfeather：先啟動新的一份（等這一份結束才接手，見 win::entry 的 --wait-pid），再關閉這個視窗。
    /// 關閉時照常記下分頁，新的一份照「還原上次的分頁」開回來。
    pub(super) fn restart(&mut self) {
        let Some(exe) = current_exe_path() else { return };
        let arg = format!("{}{}", crate::win::WAIT_PID_ARG, unsafe { GetCurrentProcessId() });
        match std::process::Command::new(&exe).arg(arg).spawn() {
            Ok(_) => unsafe {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
            },
            Err(e) => self.show_error(&trf!("無法重新開啟 Stillfeather：{e}", "Couldn't restart Stillfeather: {e}")),
        }
    }

    /// 現在跳出更新提醒會不會打斷使用者：Stillfeather 要在前景，而且沒有對話方塊或選單開著、
    /// 沒有在拖曳、重新命名或輸入路徑
    pub(in crate::win) fn ready_for_prompt(&self) -> bool {
        let front = unsafe { GetForegroundWindow() == self.hwnd && IsWindowEnabled(self.hwnd) != 0 };
        front
            && self.renaming.is_none()
            && self.drag.is_none()
            && !self.address_editing
            && !crate::win::update::ui_busy()
    }

    /// 設為／取消預設檔案總管（Win+E 與開啟資料夾）。只改目前使用者的登錄，不需要系統管理員權限。
    pub(super) fn set_default_file_manager(&mut self, on: bool) {
        // 先記下選擇：之後啟動時照這個選擇處理（沒選過時預設開啟）
        let _ = shellreg::set_default_fm_choice(on);
        let result = if on {
            match current_exe_path() {
                Some(exe) => shellreg::set_default_file_manager(&exe),
                None => Err(tr!("找不到 Stillfeather 程式的位置。", "Can't find where the Stillfeather program is.").into()),
            }
        } else {
            shellreg::restore_default_file_manager()
        };
        crate::win::shellwin::refresh();
        match result {
            Ok(()) if on => self.set_footer(tr!("已設為預設檔案總管：Win+E 與開啟資料夾都會使用 Stillfeather", "Set as the default file manager: Win+E and opening folders both use Stillfeather").into()),
            Ok(()) => self.set_footer(tr!("已改回 Windows 檔案總管", "Switched back to File Explorer").into()),
            Err(e) => self.show_error(&trf!("無法變更預設檔案總管：{e}", "Couldn't change the default file manager: {e}")),
        }
    }

    pub(in crate::win) fn apply_settings(&mut self, new: Settings) {
        let _fade = self.crossfade(vec![client_rect(self.hwnd)], anim::THEME_MS);
        let compact_changed = new.compact_rows != self.settings.compact_rows;
        let favorites_changed = new.favorites != self.settings.favorites || new.bookmark_groups != self.settings.bookmark_groups || new.sidebar_order != self.settings.sidebar_order
            || new.collapsed_sections != self.settings.collapsed_sections;
        self.keymap = keymap::Keymap::with_overrides(&new.shortcuts).0;
        let text_changed = new.text_size != self.settings.text_size;
        self.settings = new;
        self.sync_shortcut_tips(TTM_UPDATETIPTEXTW);
        self.theme.set_palette(&self.settings);
        if text_changed {
            // 字型換了：清單、樹狀圖換新字型，列高跟著文字重算（重建影像清單）
            self.theme.set_text_size(self.settings.text_size);
            self.apply_fonts();
        }
        if compact_changed || text_changed {
            self.rebuild_icons();
        }
        self.apply_theme_to_controls();
        if favorites_changed {
            self.build_tree();
        }
        self.relayout();
        for side in [Side::Left, Side::Right] {
            self.sync_view(side);
            invalidate(self.lists[side.index()]);
        }
        // 預覽的總開關或預覽方式可能換了：關掉時收起懸停預覽（右鍵開的置中預覽留著），名稱提示跟著開關
        if !self.settings.preview && preview::mode(&self.preview) == Some(preview::Mode::Hover) {
            self.hide_preview();
        }
        self.cancel_hover();
        self.sync_label_tips();
        self.update_chrome();
    }

    fn rebuild_icons(&mut self) {
        let mut old = std::mem::replace(&mut self.icons, Icons::new(self.theme.dpi, self.settings.compact_rows, self.theme.fonts.body_height));
        for list in self.lists {
            send(list, LVM_SETIMAGELIST, LVSIL_SMALL, self.icons.list);
        }
        send(self.tree, TVM_SETIMAGELIST, TVSIL_NORMAL, self.icons.tree);
        // 大圖示清單也跟著新的 DPI 重建；清單改用新的之後才銷毀舊的
        self.apply_view();
        old.destroy();
        self.build_tree();
    }

    pub(super) fn on_dpi_changed(&mut self, dpi: u32, suggested: &RECT) {
        self.theme.set_dpi(dpi);
        self.apply_fonts();
        self.rebuild_icons();
        unsafe {
            SetWindowPos(self.hwnd, 0, suggested.left, suggested.top, suggested.width(), suggested.height(), SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.relayout();
        self.apply_theme_to_controls();
    }

    pub(super) fn system_theme_changed(&mut self) {
        if self.settings.theme_mode == feather_core::settings::ThemeMode::System {
            let _fade = self.crossfade(vec![client_rect(self.hwnd)], anim::THEME_MS);
            self.sync_shortcut_tips(TTM_UPDATETIPTEXTW);
        self.theme.set_palette(&self.settings);
            self.apply_theme_to_controls();
            self.update_chrome();
        }
    }

    /// 執行了 Windows 右鍵選單的命令（新增、建立捷徑、解壓縮…）：檔案可能有變動，馬上重新整理一次，
    /// 稍後再整理一次給在背景完成的命令。
    pub(in crate::win) fn after_shell_command(&mut self) {
        self.refresh_after_shell();
        unsafe {
            SetTimer(self.hwnd, TIMER_SHELL_REFRESH, 1200, std::ptr::null());
        }
    }

    pub(super) fn refresh_after_shell(&mut self) {
        for side in [Side::Left, Side::Right] {
            if side == Side::Right && !self.ws.dual {
                continue;
            }
            let Some(id) = self.bound[side.index()] else { continue };
            // 正在改名（例如「新增 ▸」剛建立的項目）：重新讀取會把編輯框收掉，自動重新整理會在改完後補上
            if send(self.lists[side.index()], LVM_GETEDITCONTROL, 0, 0) != 0 {
                continue;
            }
            if self.tabs.get(&id).is_some_and(|t| !t.search_mode && !t.loading && !t.path.is_empty()) {
                self.refresh_tab(id);
            }
        }
    }
}

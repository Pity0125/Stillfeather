//! 鍵盤：主視窗的按鍵處理。快速鍵查 `feather_core::keymap` 的表（說明的快速鍵一覽也從那張表產生）；
//! 不在表裡的是要看按下／放開或輸入法狀態的按鍵：按住空白鍵預覽、輸入框裡的 Enter／Esc、Tab 換焦點、
//! 清單的打字跳到項目，以及麵包屑選單、選擇器自己的按鍵。訊息迴圈在分派前先經過 `pretranslate`。

use super::*;

impl App {
    /// 在清單或樹狀圖打字：跳到名稱開頭相符的項目；找不到就安靜地不動。
    fn type_to_find(&mut self, target: HWND, c: char) {
        let now = anim::now();
        if c == ' ' && !self.typeahead.active(now) {
            return;
        }
        let query = self.typeahead.push(c, now);
        if let Some(side) = self.side_of_list(target) {
            self.list_type_ahead(side, &query);
        } else if target == self.tree {
            self.tree_type_ahead(&query);
        }
    }

    fn handle_key(&mut self, msg: &MSG) -> bool {
        let mut vk = msg.wParam as u32;
        if vk == VK_PROCESSKEY {
            // 中文輸入法（例如注音）開著時，按鍵先交給輸入法，這裡收到的是 VK_PROCESSKEY。
            // 正在組字就交給輸入法；沒有組字時取回原本的按鍵，讓空白鍵等快捷鍵照常作用。
            if ime_composing(msg.hwnd) {
                return false;
            }
            vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
        }
        let ctrl = key_down(VK_CONTROL);
        let shift = key_down(VK_SHIFT);
        let alt = msg.message == WM_SYSKEYDOWN || key_down(VK_MENU);
        let focus = unsafe { GetFocus() };
        let in_edit = focus == self.address || focus == self.query;
        // 拖曳分頁中按 Esc：取消拖曳，分頁留在原位
        if vk == VK_ESCAPE && tabs::cancel_drag() {
            return true;
        }
        // 麵包屑的子資料夾選單開著：方向鍵、Enter、Esc 與打字交給選單，其他按鍵不作用在後面的清單上
        if crumbmenu::is_open(&self.crumb_menu) {
            match crumbmenu::key(&mut self.crumb_menu, vk, ctrl, alt) {
                crumbmenu::Key::Handled => return true,
                // 交給 TranslateMessage 變成 WM_CHAR（見 pretranslate）
                crumbmenu::Key::Char => return false,
                crumbmenu::Key::Close => {
                    self.close_crumb_menu();
                    return true;
                }
                crumbmenu::Key::Pick(path) => {
                    self.pick_crumb_menu(path);
                    return true;
                }
                crumbmenu::Key::Neighbor(delta) => {
                    // 左右鍵：換成左右相鄰那一段的選單（被收進「…」的段落沒有箭頭，就停在原處）
                    if let Some(j) = crumbmenu::segment(&self.crumb_menu).and_then(|i| i.checked_add_signed(delta)) {
                        if crumbs::arrow_rect(self.crumbs, j).is_some() {
                            self.crumb_menu_action(j, false);
                        }
                    }
                    return true;
                }
                crumbmenu::Key::Pass => self.close_crumb_menu(),
            }
        }
        // 選擇器：檔名欄的 Enter／Esc，以及分頁、雙欄、設定這些不存在的快捷鍵
        if let Some(handled) = self.picker_key(vk, ctrl, shift, alt, focus) {
            return handled;
        }
        if !in_edit && focus != 0 && class_name(focus).eq_ignore_ascii_case("Edit") {
            return false; // 清單的就地重新命名
        }
        // 按鍵（空白鍵與 Shift／Ctrl／Alt／Windows 鍵除外）關掉懸停的預覽，滑鼠移到別的項目前不再出現
        let modifier = matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5);
        if vk != VK_SPACE && !modifier {
            let hovering = preview::mode(&self.preview) == Some(preview::Mode::Hover);
            self.cancel_hover();
            if hovering && vk == VK_ESCAPE {
                return true;
            }
        }
        // 「空白鍵預覽」：按住空白鍵時滑鼠停在哪個項目就預覽哪個，放開就關閉（不看選取）。
        // 置中的預覽（右鍵選單開啟）開著時照舊由下面處理；清單正在打字找檔案時空白是名稱的一部分。
        if self.settings.preview
            && self.settings.space_preview
            && vk == VK_SPACE
            && !ctrl
            && !alt
            && !shift
            && !in_edit
            && !preview::mode(&self.preview).is_some_and(|m| m != preview::Mode::Hover)
            && !(self.side_of_list(focus).is_some() && self.typeahead.active(anim::now()))
            && self.space_gate_applies(focus)
        {
            let repeat = msg.lParam & (1 << 30) != 0;
            if !repeat && !self.space_gate_open() {
                self.open_space_gate();
            }
            return true;
        }
        // 快速預覽開著時 Esc 先關閉預覽
        if vk == VK_ESCAPE && preview::is_open(&self.preview) {
            self.hide_preview();
            return true;
        }
        // 選擇器：Esc 取消（在路徑或搜尋輸入框裡時，照常先離開輸入框、取消搜尋）
        if vk == VK_ESCAPE && !in_edit && self.picker_mode() {
            self.picker_finish(feather_core::picker::EXIT_CANCEL);
            return true;
        }
        // 快速鍵表（feather_core::keymap）：分頁、雙欄、檢視、導覽、剪貼簿、刪除等。說明的快速鍵一覽也從同一張表產生。
        if let Some(cmd) = self.keymap.lookup(vk, ctrl, shift, alt, in_edit, self.side_of_list(focus).is_some()) {
            self.run_key_command(cmd, focus);
            return true;
        }
        if vk == VK_TAB && !ctrl && !alt {
            let next = unsafe { GetNextDlgTabItem(self.hwnd, focus, shift as BOOL) };
            if next != 0 {
                unsafe {
                    SetFocus(next);
                }
            }
            return true;
        }
        if in_edit {
            match vk {
                VK_RETURN => {
                    if focus == self.address {
                        let text = window_text(self.address);
                        let id = self.active_tab_id();
                        // 相對路徑（..、子資料夾名稱、\Windows）以目前的資料夾為準
                        let here = self.active_tab().map(|t| t.path.clone()).unwrap_or_default();
                        // 輸入的是檔案路徑：開啟所在資料夾並選取該檔案（不直接執行）。
                        // 網路上的位置不在這裡檢查（離線時要等 SMB 逾時、視窗凍住），交給背景讀取時再判斷（見 navigate_or_reveal）
                        match full_path_in(&text, Some(&here)) {
                            Ok(full) if !fs::on_local_drive(&full) => self.navigate_or_reveal(id, &full, NavKind::Normal),
                            Ok(full) if fs::is_file(&full) => {
                                let parent = paths::parent(&full).unwrap_or_else(|| full.clone());
                                self.navigate(id, &parent, NavKind::Normal, vec![full]);
                            }
                            Ok(full) => self.navigate(id, &full, NavKind::Normal, vec![]),
                            _ => self.navigate(id, &text, NavKind::Normal, vec![]),
                        }
                        self.end_address_edit();
                        self.focus_list(self.active_side());
                    } else {
                        self.start_search();
                    }
                    return true;
                }
                VK_ESCAPE => {
                    if focus == self.address {
                        let path = self.active_tab().map(|t| t.path.clone()).unwrap_or_default();
                        set_window_text(self.address, &path);
                        self.end_address_edit();
                    } else if self.active_tab().is_some_and(|t| t.working()) {
                        self.clear_or_cancel();
                        return true;
                    }
                    self.focus_list(self.active_side());
                    return true;
                }
                _ => return false,
            }
        }
        if let Some(side) = self.side_of_list(focus) {
            // 空白鍵：按住預覽、放開關閉（正在打字找檔案時，空白是名稱的一部分）。
            // 按住時的自動重複一律略過，否則預覽會一直開關閃爍。
            // 關閉快速預覽時空白鍵不開預覽（右鍵開的預覽開著時照樣用空白鍵關閉）。
            if vk == VK_SPACE && !ctrl && !alt && !shift && !self.typeahead.active(anim::now()) && (self.settings.preview || preview::is_open(&self.preview)) {
                let repeat = msg.lParam & (1 << 30) != 0;
                if !repeat {
                    crate::win::crashlog::crumb("按鍵：空白鍵按下");
                    match preview::mode(&self.preview) {
                        // 懸停中：放大懸停的項目，放開就關閉
                        Some(preview::Mode::Hover) => self.enlarge_hover_preview(),
                        // 從選單開啟、維持開著的預覽：空白鍵關閉
                        Some(_) => self.hide_preview(),
                        None => {
                            self.cancel_hover();
                            self.open_quicklook(side, true);
                        }
                    }
                }
                return true;
            }
        }
        if focus == self.tree && vk == VK_RETURN {
            self.tree_navigate_selected();
            return true;
        }
        false
    }

    /// 執行快速鍵表裡的命令（見 handle_key）。`focus` 是按鍵時有焦點的視窗。
    fn run_key_command(&mut self, cmd: keymap::Cmd, focus: HWND) {
        use keymap::Cmd;
        let side = self.side_of_list(focus).unwrap_or(self.active_side());
        // 焦點在側欄：複製、剪下、複製路徑針對側欄選取的資料夾（與右鍵選單相同），而不是清單裡剛好選著的項目
        if focus == self.tree && matches!(cmd, Cmd::Copy | Cmd::Cut | Cmd::CopyPath) {
            if let Some(path) = self.tree_caret_path() {
                match cmd {
                    Cmd::CopyPath => match crate::win::clipboard::set_text(self.hwnd, &path) {
                        Err(e) => self.show_error(&e),
                        Ok(()) => self.set_footer(tr!("已複製路徑", "Path copied").into()),
                    },
                    Cmd::Cut if paths::parent(&path).is_none() => self.set_footer(tr!("磁碟根目錄不能剪下", "A drive's root folder can't be cut").into()),
                    _ => self.clipboard_paths(vec![path], cmd == Cmd::Cut),
                }
            }
            return;
        }
        match cmd {
            Cmd::NewTab => self.new_tab_active(),
            Cmd::CloseTab => self.close_current_tab(),
            Cmd::NextTab => self.switch_tab(1),
            Cmd::PrevTab => self.switch_tab(-1),
            Cmd::GoToTab(n) => self.go_to_tab(n),
            Cmd::MoveTabLeft | Cmd::MoveTabRight => {
                let side = self.active_side();
                let index = self.ws.group(side).selected;
                self.move_tab(side, index, if cmd == Cmd::MoveTabLeft { -1 } else { 1 });
            }
            Cmd::ReopenTab => self.reopen_closed_tab(),
            Cmd::ToggleDual => {
                let v = !self.ws.dual;
                self.set_dual(v);
            }
            // 舊的 Ctrl+Shift+D：保留一兩個版本，只提示新的按鍵（不會落到清單的 Ctrl+D 刪除）
            Cmd::OldDualShortcut => self.set_footer(tr!("切換雙欄改成 Ctrl+\\", "Single/dual pane is now Ctrl+\\").into()),
            Cmd::OldPathShortcut => self.set_footer(tr!("輸入路徑改成 Ctrl+E（Alt+D 也可以）", "Editing the path is now Ctrl+E (or Alt+D)").into()),
            Cmd::View(d) => {
                if let Some(mode) = feather_core::view::ViewMode::from_digit(d) {
                    self.set_view_mode(mode);
                }
            }
            Cmd::Settings => self.open_settings(),
            Cmd::EditPath => self.begin_address_edit(),
            Cmd::Search => self.open_search(),
            Cmd::Refresh => {
                let id = self.active_tab_id();
                self.refresh_tab(id);
            }
            Cmd::Back => self.go_back(),
            Cmd::Forward => self.go_forward(),
            Cmd::Up => self.go_up(),
            Cmd::Properties => self.properties(),
            Cmd::Undo => self.undo(),
            Cmd::Copy => self.clipboard_files(false),
            Cmd::Cut => self.clipboard_files(true),
            Cmd::Paste => self.paste(),
            Cmd::CopyPath => self.copy_path(),
            Cmd::NewFolder => self.new_folder(),
            Cmd::SelectAll => self.select_all(side),
            Cmd::Open => {
                self.hide_preview();
                self.open_selection(side);
            }
            Cmd::Rename => self.begin_rename(),
            // Shift+Delete：永久刪除（不經過資源回收筒）
            Cmd::Delete => self.delete_selection(false),
            Cmd::DeletePermanently => self.delete_selection(true),
            Cmd::Help => self.show_help(),
        }
    }
}

pub(super) fn pretranslate(msg: &MSG) -> bool {
    // 麵包屑的子資料夾選單：點在選單外面（含標題列、邊框）就關閉，點擊本身照常處理
    if matches!(msg.message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN) {
        if let Some(a) = app_opt() {
            a.crumb_menu_click(msg);
        }
    }
    if msg.message == WM_MOUSEWHEEL || msg.message == WM_MOUSEHWHEEL {
        let Some(a) = app_opt() else { return false };
        let horizontal = msg.message == WM_MOUSEHWHEEL;
        // 子資料夾選單開著：滑鼠在選單上就捲動選單，在別的地方就關掉選單、照常捲動
        if crumbmenu::is_open(&a.crumb_menu) {
            let mut r = RECT::default();
            unsafe {
                GetWindowRect(a.crumb_menu.popup, &mut r);
            }
            if r.contains(msg.pt.x, msg.pt.y) {
                if !horizontal {
                    crumbmenu::wheel(&mut a.crumb_menu, ((msg.wParam >> 16) & 0xFFFF) as u16 as i16 as i32);
                }
                return true;
            }
            a.close_crumb_menu();
        }
        // 懸停的預覽：清單照常捲動，預覽先關掉，滑鼠再移動時依新的項目重新計時
        if preview::mode(&a.preview) == Some(preview::Mode::Hover) {
            a.reset_hover();
            return false;
        }
        // 滑鼠在置中的快速預覽上：滾輪捲動預覽的內容（試算表也可左右捲動），而不是清單
        if preview::is_open(&a.preview) {
            let mut pt = POINT::default();
            let mut r = RECT::default();
            unsafe {
                GetCursorPos(&mut pt);
                GetWindowRect(a.preview.popup, &mut r);
            }
            if pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom {
                let delta = ((msg.wParam >> 16) & 0xFFFF) as u16 as i16 as i32;
                if horizontal {
                    preview::hwheel(&mut a.preview, delta);
                } else {
                    preview::wheel(&mut a.preview, delta);
                }
                return true;
            }
        }
        if horizontal {
            a.reset_hover();
        }
        return false;
    }
    if msg.message == WM_XBUTTONUP {
        // 滑鼠側鍵：上一頁／下一頁（與檔案總管、瀏覽器相同）。滑鼠在 Stillfeather 任何地方都有效。
        let Some(a) = app_opt() else { return false };
        // 就地重新命名中：先讓使用者完成或取消編輯
        if (msg.hwnd != a.hwnd && unsafe { IsChild(a.hwnd, msg.hwnd) } == 0) || a.renaming.is_some() {
            return false;
        }
        match hiword(msg.wParam) {
            XBUTTON1 => a.go_back(),
            XBUTTON2 => a.go_forward(),
            _ => return false,
        }
        a.update_chrome();
        return true;
    }
    if matches!(msg.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN) {
        // 先處理焦點，點擊本身照常交給目標視窗
        if let Some(a) = app_opt() {
            a.blur_inputs_on_click(msg.hwnd);
        }
        return false;
    }
    if msg.message == WM_CHAR {
        // 清單與樹狀圖的打字搜尋由這裡處理；字元不交給控制項，因此找不到項目時不會有提示音。
        let Some(a) = app_opt() else { return false };
        // 子資料夾選單開著：打的字跳到選單裡的資料夾，不進到後面的清單或輸入框
        if crumbmenu::is_open(&a.crumb_menu) && (msg.hwnd == a.hwnd || unsafe { IsChild(a.hwnd, msg.hwnd) } != 0) {
            if let Some(c) = char::from_u32(msg.wParam as u32).filter(|c| !c.is_control()) {
                crumbmenu::type_char(&mut a.crumb_menu, c);
            }
            return true;
        }
        if a.side_of_list(msg.hwnd).is_some() || msg.hwnd == a.tree {
            if let Some(c) = char::from_u32(msg.wParam as u32).filter(|c| !c.is_control()) {
                a.type_to_find(msg.hwnd, c);
            }
            return true;
        }
        return false;
    }
    if msg.message == WM_KEYUP {
        // 放開空白鍵：關閉按住開啟的快速預覽（輸入法開著時收到的是 VK_PROCESSKEY）
        let Some(a) = app_opt() else { return false };
        if a.space_gate_open() {
            let mut vk = msg.wParam as u32;
            if vk == VK_PROCESSKEY {
                vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
            }
            if vk == VK_SPACE {
                a.close_space_gate();
                return true;
            }
        }
        if preview::is_holding(&a.preview) {
            let mut vk = msg.wParam as u32;
            if vk == VK_PROCESSKEY {
                vk = unsafe { ImmGetVirtualKey(msg.hwnd) };
            }
            if vk == VK_SPACE {
                crate::win::crashlog::crumb("按鍵：空白鍵放開");
                a.hide_preview();
                return true;
            }
        }
        return false;
    }
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    let Some(a) = app_opt() else { return false };
    if msg.hwnd != a.hwnd && unsafe { IsChild(a.hwnd, msg.hwnd) } == 0 {
        return false;
    }
    let handled = a.handle_key(msg);
    if handled {
        a.update_chrome();
    }
    handled
}

/// 輸入法是否正在組字（注音輸入到一半）。
fn ime_composing(hwnd: HWND) -> bool {
    unsafe {
        let himc = ImmGetContext(hwnd);
        if himc == 0 {
            return false;
        }
        let len = ImmGetCompositionStringW(himc, GCS_COMPSTR, std::ptr::null_mut(), 0);
        ImmReleaseContext(hwnd, himc);
        len > 0
    }
}

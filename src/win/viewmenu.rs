//! 動作列右側的「排序」「檢視」下拉選單（對應檔案總管命令列的同名選單）。
//! 檢視模式記在各個資料夾；「檢視 › 顯示」裡的精簡檢視、副檔名與隱藏的項目，以及「檢視 › 文字大小」是整體設定。
//! 排序原本另有 A–Z／最新／大小 三個快速籤，和這個選單重複，已併進來：選修改日期、大小時從新的、大的開始。

use super::anim;
use super::app::{id, write_folder_views, write_settings, App};
use super::dialog::{self, Icon};
use super::menu::{self, Item, Look, Mark};
use super::theme::glyph;
use feather_core::folderviews;
use feather_core::settings::{Settings, TEXT_SIZES};
use feather_core::sorting;
use feather_core::view::{self, ViewMode};
use feather_core::workspace::Side;

/// 檢視選單各項目前面的圖示
fn view_glyph(m: ViewMode) -> char {
    match m {
        ViewMode::ExtraLarge => glyph::PHOTO,
        ViewMode::Large => glyph::VIEW_LARGE,
        ViewMode::Medium => glyph::VIEW_MEDIUM,
        ViewMode::Small => glyph::VIEW_SMALL,
        ViewMode::List => glyph::VIEW_LIST,
        ViewMode::Details => glyph::VIEW_DETAILS,
        ViewMode::Tiles => glyph::VIEW_TILES,
        ViewMode::Content => glyph::VIEW_CONTENT,
    }
}

impl App {
    pub(super) fn menu_look(&self) -> Look {
        Look { font: self.theme.fonts.icon_small, color: self.theme.pal.ink, size: self.s(16) }
    }

    /// 目前分頁的排序欄位與方向
    fn current_sort(&self) -> (usize, bool) {
        self.active_tab().map(|t| (t.sort_col, t.desc)).unwrap_or((0, false))
    }

    pub(super) fn sort_menu(&mut self) {
        self.hide_preview();
        let (col, desc) = self.current_sort();
        let searching = self.active_tab().is_some_and(|t| t.search_mode);
        let relevance = col == sorting::COL_RELEVANCE;
        let mut items = Vec::new();
        // 搜尋結果：相關性（檔名完全相同的在前，再來是開頭相同、在詞的開頭出現、中間有出現）
        if searching {
            items.push(Item::radio(id::SORT_RELEVANCE, tr!("相關性", "Relevance"), relevance));
        }
        items.extend([
            Item::radio(id::SORT_NAME, tr!("名稱", "Name"), col == sorting::COL_NAME),
            Item::radio(id::SORT_DATE, tr!("修改日期（新的在前）", "Date modified (newest first)"), col == sorting::COL_MODIFIED),
            Item::radio(id::SORT_SIZE, tr!("大小（大的在前）", "Size (largest first)"), col == sorting::COL_SIZE),
            Item::radio(id::SORT_TYPE, tr!("類型", "Type"), col == sorting::COL_TYPE),
            Item::Submenu { text: tr!("更多選項", "More options"), glyph: None, items: vec![Item::radio(id::SORT_LOCATION, tr!("所在位置", "Location"), col == sorting::COL_LOCATION)] },
            Item::Separator,
            // 相關性沒有方向
            Item::Command { id: id::SORT_ASC, text: tr!("遞增", "Ascending"), glyph: None, mark: Mark::Radio, on: !desc && !relevance, enabled: !relevance },
            Item::Command { id: id::SORT_DESC, text: tr!("遞減", "Descending"), glyph: None, mark: Mark::Radio, on: desc && !relevance, enabled: !relevance },
            Item::Separator,
            Item::check(id::SORT_FOLDERS_ON_TOP, tr!("資料夾一律排在最前面", "Keep folders on top"), self.settings.folders_on_top),
        ]);
        if let Some(cmd) = menu::drop_down(self.hwnd, self.sort_btn, &items) {
            if cmd == id::SORT_FOLDERS_ON_TOP {
                self.toggle_folders_on_top();
            } else {
                self.sort_command(cmd);
            }
        }
    }

    /// 排序選單的「資料夾一律排在最前面」：關著時和檔案總管相同（遞增時資料夾在前、遞減時在後）。
    /// 每個分頁都照新的規則重新排好。
    fn toggle_folders_on_top(&mut self) {
        self.change_setting(|s| s.folders_on_top = !s.folders_on_top);
        let on = self.settings.folders_on_top;
        // 顯示中的分頁：保留選取、重畫清單
        for side in [Side::Left, Side::Right] {
            if let Some(t) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) {
                let (col, desc) = (t.sort_col, t.desc);
                self.sort_to(side, col, desc);
            }
        }
        // 其他分頁：直接重排，切換過去時就是新的順序
        let shown: Vec<_> = self.bound.iter().flatten().copied().collect();
        for (id, t) in self.tabs.iter_mut() {
            if !shown.contains(id) && !t.loading {
                t.resort(on);
                t.refilter();
            }
        }
        self.update_chrome();
        self.set_footer(if on {
            tr!("資料夾一律排在最前面", "Folders are always kept on top").into()
        } else {
            tr!("資料夾跟著排序方向：遞增在前、遞減在後（和檔案總管相同）", "Folders follow the sort order: first when ascending, last when descending (like File Explorer)").into()
        });
    }

    /// 換排序欄位時用那一欄習慣的方向（修改日期、大小從新的、大的開始，同原本的「最新」「大小」快速籤）；
    /// 選目前的欄位不改方向；遞增、遞減只換方向。
    pub(super) fn sort_command(&mut self, cmd: u16) {
        let side = self.active_side();
        let (col, desc) = self.current_sort();
        let pick = |target: usize| if target == col { (col, desc) } else { (target, sorting::default_desc(target)) };
        let (col, desc) = match cmd {
            id::SORT_NAME => pick(sorting::COL_NAME),
            id::SORT_SIZE => pick(sorting::COL_SIZE),
            id::SORT_TYPE => pick(sorting::COL_TYPE),
            id::SORT_DATE => pick(sorting::COL_MODIFIED),
            id::SORT_LOCATION => pick(sorting::COL_LOCATION),
            id::SORT_RELEVANCE => (sorting::COL_RELEVANCE, false),
            id::SORT_ASC => (col, false),
            id::SORT_DESC => (col, true),
            _ => return,
        };
        self.sort_to(side, col, desc);
        self.update_chrome();
    }

    pub(super) fn view_menu(&mut self) {
        self.hide_preview();
        let current = self.view[self.active_side().index()];
        let labels: Vec<String> = view::ALL.iter().map(|m| self.keymap.with_hint(m.label(), feather_core::keymap::Cmd::View(m.digit()), "\t")).collect();
        let mut items: Vec<Item> = view::ALL
            .iter()
            .zip(labels.iter())
            .enumerate()
            .map(|(i, (m, label))| Item::radio(id::VIEW_FIRST + i as u16, label, *m == current).with_glyph(view_glyph(*m)))
            .collect();
        items.push(Item::Separator);
        items.push(Item::Submenu {
            text: tr!("顯示", "Show"),
            glyph: Some(glyph::HIDDEN),
            items: vec![
                Item::check(id::SHOW_COMPACT, tr!("精簡檢視", "Compact view"), self.settings.compact_rows),
                Item::check(id::SHOW_EXTENSIONS, tr!("副檔名", "File name extensions"), self.settings.show_extensions),
                Item::check(id::HIDDEN, tr!("隱藏的項目", "Hidden items"), self.show_hidden),
                Item::Separator,
                Item::check(id::SHOW_MEDIA_ICONS, tr!("相片資料夾用大圖示", "Large icons for photo folders"), self.settings.media_folder_icons),
            ],
        });
        // 文字大小：檔名、欄位、側欄與狀態列的字（存在設定裡，重開後仍有效）
        let size_labels = [tr!("標準", "Standard"), tr!("大", "Large"), tr!("更大", "Larger"), tr!("最大", "Largest")];
        let sizes: Vec<String> = TEXT_SIZES.iter().zip(size_labels).map(|(pct, label)| format!("{label}\t{pct}%")).collect();
        items.push(Item::Submenu {
            text: tr!("文字大小", "Text size"),
            glyph: Some(glyph::FONT_SIZE),
            items: sizes.iter().enumerate().map(|(i, label)| Item::radio(id::TEXT_SIZE_FIRST + i as u16, label, self.settings.text_size == TEXT_SIZES[i])).collect(),
        });
        items.push(Item::Command { id: id::VIEW_APPLY_ALL, text: tr!("套用到所有資料夾", "Apply to all folders"), glyph: None, mark: Mark::None, on: false, enabled: true });
        let Some(cmd) = menu::drop_down(self.hwnd, self.view_btn, &items) else { return };
        match cmd {
            id::SHOW_COMPACT => self.toggle_compact(),
            id::SHOW_EXTENSIONS => self.toggle_extensions(),
            id::HIDDEN => self.toggle_hidden(),
            id::SHOW_MEDIA_ICONS => self.toggle_media_icons(),
            c if (id::TEXT_SIZE_FIRST..id::TEXT_SIZE_FIRST + TEXT_SIZES.len() as u16).contains(&c) => self.set_text_size(TEXT_SIZES[(c - id::TEXT_SIZE_FIRST) as usize]),
            id::VIEW_APPLY_ALL => self.apply_view_to_all(),
            c if (id::VIEW_FIRST..id::VIEW_FIRST + 8).contains(&c) => {
                if let Some(mode) = ViewMode::from_digit((c - id::VIEW_FIRST + 1) as u8) {
                    self.set_view_mode(mode);
                }
            }
            _ => {}
        }
    }

    /// 作用中那一欄的檢視模式；記在目前的資料夾（搜尋時記在「搜尋結果」）。
    /// 另一欄顯示同一個資料夾時一起換。
    pub(super) fn set_view_mode(&mut self, mode: ViewMode) {
        let side = self.active_side();
        if self.view[side.index()] == mode {
            return;
        }
        let key = self.active_tab().map(|t| if t.search_mode { folderviews::SEARCH_KEY.to_string() } else { t.path.clone() }).unwrap_or_default();
        let searching = key == folderviews::SEARCH_KEY;
        let other = side.other();
        let other_same = self.ws.dual && self.bound[other.index()].and_then(|id| self.tabs.get(&id)).is_some_and(|t| {
            let k = if t.search_mode { folderviews::SEARCH_KEY.to_string() } else { t.path.clone() };
            !key.is_empty() && folderviews::key(&k) == folderviews::key(&key)
        });
        let mut areas = vec![self.layout.panes[side.index()].inflate(self.s(4), self.s(6))];
        if other_same {
            areas.push(self.layout.panes[other.index()].inflate(self.s(4), self.s(6)));
        }
        let _fade = self.crossfade(areas, anim::CROSSFADE_MS);
        // 和這個資料夾的預設（相片資料夾是大圖示）一樣就不必記
        let folder_default = self.active_tab().map(|t| self.folder_default_view(t)).unwrap_or(self.settings.view_mode);
        let changed = self.views.set(&key, mode, folder_default);
        self.view[side.index()] = mode;
        self.apply_view_side(side);
        if other_same {
            self.sync_view(other);
        }
        let place = if searching { tr!("搜尋結果", "search results") } else { tr!("這個資料夾", "this folder") };
        if !changed {
            self.set_footer(trf!("檢視：{}", "View: {}", mode.label()));
        } else {
            match write_folder_views(&self.views) {
                Ok(()) => self.set_footer(trf!("檢視：{}（記在{place}）", "View: {} (remembered for {place})", mode.label())),
                Err(e) => self.set_footer(trf!("檢視已切換，但無法儲存資料夾檢視記錄：{e}", "The view changed, but the folder view record couldn't be saved: {e}")),
            }
        }
        for s in [Side::Left, Side::Right] {
            self.refresh_list(s, false);
        }
    }

    /// 「套用到所有資料夾」：目前的檢視變成預設，清掉個別資料夾的記錄；相片資料夾也不再另外用大圖示。
    pub(super) fn apply_view_to_all(&mut self) {
        let mode = self.view[self.active_side().index()];
        if !self.views.is_empty() {
            let body = trf!("所有資料夾都會改用「{}」，各資料夾記住的 {} 個檢視會清除。", "Every folder will use “{}”, and the {} views remembered by individual folders will be cleared.", mode.label(), self.views.len());
            if !dialog::confirm(self.hwnd, Icon::Info, tr!("套用到所有資料夾？", "Apply to all folders?"), &body, tr!("套用", "Apply")) {
                return;
            }
        }
        self.views.clear();
        let saved_views = write_folder_views(&self.views);
        let mut new = self.settings.clone();
        new.view_mode = mode;
        new.media_folder_icons = false;
        match (write_settings(&new), saved_views) {
            (Ok(()), Ok(())) => {
                self.settings = new;
                self.set_footer(trf!("所有資料夾都改用「{}」", "Every folder now uses “{}”", mode.label()));
            }
            (Err(e), _) | (_, Err(e)) => {
                self.settings = new;
                self.set_footer(trf!("已套用，但無法儲存：{e}", "Applied, but couldn't save: {e}"));
            }
        }
        let areas: Vec<_> = (0..if self.ws.dual { 2 } else { 1 }).map(|i| self.layout.panes[i].inflate(self.s(4), self.s(6))).collect();
        let _fade = self.crossfade(areas, anim::CROSSFADE_MS);
        for s in [Side::Left, Side::Right] {
            self.sync_view(s);
            self.refresh_list(s, false);
        }
    }

    fn change_setting(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut new = self.settings.clone();
        change(&mut new);
        match write_settings(&new) {
            Ok(()) => self.apply_settings(new),
            Err(e) => self.show_error(&trf!("設定未儲存，保留先前設定：{e}", "Settings weren't saved; the previous settings are kept: {e}")),
        }
    }

    /// 「檢視 › 文字大小」
    pub(super) fn set_text_size(&mut self, pct: u32) {
        if self.settings.text_size != pct {
            self.change_setting(|s| s.text_size = pct);
        }
    }

    pub(super) fn toggle_compact(&mut self) {
        self.change_setting(|s| s.compact_rows = !s.compact_rows);
    }

    pub(super) fn toggle_extensions(&mut self) {
        self.change_setting(|s| s.show_extensions = !s.show_extensions);
    }

    /// 「檢視 › 顯示 › 相片資料夾用大圖示」：沒記住檢視的相片資料夾用大圖示顯示縮圖
    pub(super) fn toggle_media_icons(&mut self) {
        self.change_setting(|s| s.media_folder_icons = !s.media_folder_icons);
        let on = self.settings.media_folder_icons;
        self.set_footer(if on { tr!("相片資料夾會用大圖示顯示縮圖", "Photo folders show thumbnails in large icons").into() } else { tr!("相片資料夾改用預設的檢視", "Photo folders use the default view").into() });
    }
}

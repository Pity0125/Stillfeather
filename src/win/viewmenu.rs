//! 標題列右側的「排序」「檢視」下拉選單（對應檔案總管命令列的同名選單）。
//! 檢視模式記在各個資料夾；「檢視 › 顯示」裡的精簡檢視、副檔名與隱藏的項目是整體設定。

use super::anim;
use super::app::{id, write_folder_views, write_settings, App};
use super::dialog::{self, Icon};
use super::menu::{self, Item, Look, Mark};
use super::theme::glyph;
use feather_core::folderviews;
use feather_core::settings::Settings;
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
        let items = vec![
            Item::radio(id::SORT_NAME, "名稱", col == 0),
            Item::radio(id::SORT_DATE, "修改日期", col == 3),
            Item::radio(id::SORT_TYPE, "類型", col == 2),
            Item::Submenu {
                text: "更多選項",
                glyph: None,
                items: vec![Item::radio(id::SORT_SIZE, "大小", col == 1), Item::radio(id::SORT_LOCATION, "所在位置", col == 4)],
            },
            Item::Separator,
            Item::radio(id::SORT_ASC, "遞增", !desc),
            Item::radio(id::SORT_DESC, "遞減", desc),
        ];
        if let Some(cmd) = menu::drop_down(self.hwnd, self.sort_btn, &items, &self.menu_look()) {
            self.sort_command(cmd);
        }
    }

    /// 換排序欄位時保留目前的遞增／遞減；遞增、遞減只換方向。
    pub(super) fn sort_command(&mut self, cmd: u16) {
        let side = self.active_side();
        let (col, desc) = self.current_sort();
        let (col, desc) = match cmd {
            id::SORT_NAME => (0, desc),
            id::SORT_SIZE => (1, desc),
            id::SORT_TYPE => (2, desc),
            id::SORT_DATE => (3, desc),
            id::SORT_LOCATION => (4, desc),
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
        let labels: Vec<String> = view::ALL.iter().map(|m| format!("{}\tCtrl+Shift+{}", m.label(), m.digit())).collect();
        let mut items: Vec<Item> = view::ALL
            .iter()
            .zip(labels.iter())
            .enumerate()
            .map(|(i, (m, label))| Item::radio(id::VIEW_FIRST + i as u16, label, *m == current).with_glyph(view_glyph(*m)))
            .collect();
        items.push(Item::Separator);
        items.push(Item::Submenu {
            text: "顯示",
            glyph: Some(glyph::HIDDEN),
            items: vec![
                Item::check(id::SHOW_COMPACT, "精簡檢視", self.settings.compact_rows),
                Item::check(id::SHOW_EXTENSIONS, "副檔名", self.settings.show_extensions),
                Item::check(id::HIDDEN, "隱藏的項目", self.show_hidden),
            ],
        });
        items.push(Item::Command { id: id::VIEW_APPLY_ALL, text: "套用到所有資料夾", glyph: None, mark: Mark::None, on: false, enabled: true });
        let Some(cmd) = menu::drop_down(self.hwnd, self.view_btn, &items, &self.menu_look()) else { return };
        match cmd {
            id::SHOW_COMPACT => self.toggle_compact(),
            id::SHOW_EXTENSIONS => self.toggle_extensions(),
            id::HIDDEN => self.toggle_hidden(),
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
        let changed = self.views.set(&key, mode, self.settings.view_mode);
        self.view[side.index()] = mode;
        self.apply_view_side(side);
        if other_same {
            self.sync_view(other);
        }
        let place = if searching { "搜尋結果" } else { "這個資料夾" };
        if !changed {
            self.set_footer(format!("檢視：{}", mode.label()));
        } else {
            match write_folder_views(&self.views) {
                Ok(()) => self.set_footer(format!("檢視：{}（記在{place}）", mode.label())),
                Err(e) => self.set_footer(format!("檢視已切換，但無法儲存資料夾檢視記錄：{e}")),
            }
        }
        for s in [Side::Left, Side::Right] {
            self.refresh_list(s, false);
        }
    }

    /// 「套用到所有資料夾」：目前的檢視變成預設，清掉個別資料夾的記錄。
    pub(super) fn apply_view_to_all(&mut self) {
        let mode = self.view[self.active_side().index()];
        if !self.views.is_empty() {
            let body = format!("所有資料夾都會改用「{}」，各資料夾記住的 {} 個檢視會清除。", mode.label(), self.views.len());
            if !dialog::confirm(self.hwnd, Icon::Info, "套用到所有資料夾？", &body, "套用") {
                return;
            }
        }
        self.views.clear();
        let saved_views = write_folder_views(&self.views);
        let mut new = self.settings.clone();
        new.view_mode = mode;
        match (write_settings(&new), saved_views) {
            (Ok(()), Ok(())) => {
                self.settings = new;
                self.set_footer(format!("所有資料夾都改用「{}」", mode.label()));
            }
            (Err(e), _) | (_, Err(e)) => {
                self.settings = new;
                self.set_footer(format!("已套用，但無法儲存：{e}"));
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
            Err(e) => self.show_error(&format!("設定未儲存，保留先前設定：{e}")),
        }
    }

    pub(super) fn toggle_compact(&mut self) {
        self.change_setting(|s| s.compact_rows = !s.compact_rows);
    }

    pub(super) fn toggle_extensions(&mut self) {
        self.change_setting(|s| s.show_extensions = !s.show_extensions);
    }
}

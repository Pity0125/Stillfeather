//! 檔案清單（每欄一個虛擬化 ListView，分頁切換時只換資料，不另建控制項）。

use super::anim;
use super::shellmenu::{self, Cmd, Part, Pick};

/// 選取項目的 Windows 右鍵選單中，Stillfeather 已經有同樣命令的項目（藏起來，避免重複）。
pub(super) const ITEM_HIDDEN_VERBS: &[&str] = &["open", "explore", "opennewwindow", "opennewprocess", "opennewtab", "cut", "copy", "paste", "delete", "rename", "properties", "copyaspath", "powershell"];
/// 資料夾空白處的 Windows 右鍵選單中，Stillfeather 已經有的項目。
pub(super) const BACKGROUND_HIDDEN_VERBS: &[&str] = &["paste", "properties", "refresh", "powershell"];
use super::app::{self, id, App, NavKind, WM_APP_RENAME};
use super::preview;
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::entry::Entry;
use feather_core::folderviews;
use feather_core::format::format_size;
use feather_core::paths;
use feather_core::sorting;
use feather_core::typeahead;
use feather_core::view::{self, ViewMode};
use feather_core::workspace::{Side, TabId};

/// 清單的欄位：（中文標題, 英文標題, 寬度, 對齊）
const COLUMNS: [(&str, &str, i32, i32); 5] = [
    ("名稱", "Name", 270, LVCFMT_LEFT),
    ("大小", "Size", 90, LVCFMT_RIGHT),
    ("類型", "Type", 90, LVCFMT_LEFT),
    ("修改日期", "Date modified", 145, LVCFMT_LEFT),
    ("所在位置", "Location", 260, LVCFMT_LEFT),
];

/// 欄位標題（依介面語言）
fn column_title(c: &(&'static str, &'static str, i32, i32)) -> &'static str {
    tr!(c.0, c.1)
}

/// 並排與內容檢視在名稱下方顯示的欄位（欄位索引同 COLUMNS）。
fn tile_columns(mode: ViewMode, is_dir: bool) -> &'static [u32] {
    match (mode, is_dir) {
        (ViewMode::Tiles, false) => &[2, 1],
        (ViewMode::Tiles, true) => &[2],
        (ViewMode::Content, false) => &[2, 3, 1],
        (ViewMode::Content, true) => &[2, 3],
        _ => &[],
    }
}

fn set_item_state(list: HWND, index: isize, state: u32, mask: u32) {
    let mut item: LVITEMW = unsafe { std::mem::zeroed() };
    item.stateMask = mask;
    item.state = state;
    send_ptr(list, LVM_SETITEMSTATE, index as usize, &item);
}

fn next_item(list: HWND, start: isize, flags: u32) -> isize {
    send(list, LVM_GETNEXTITEM, start as usize, flags as LPARAM)
}

fn header_of(list: HWND) -> HWND {
    send(list, LVM_GETHEADER, 0, 0)
}

impl App {
    pub(super) fn create_list(&mut self, cid: u16, side: Side) -> HWND {
        let class = wide("SysListView32");
        let name = wide(if side == Side::Left { tr!("左欄檔案清單", "Left pane file list") } else { tr!("右欄檔案清單", "Right pane file list") });
        let list = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_OWNERDATA | LVS_EDITLABELS | LVS_SHAREIMAGELISTS,
                0,
                0,
                10,
                10,
                self.hwnd,
                cid as HMENU,
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
        send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as LPARAM);
        for (i, c) in COLUMNS.iter().enumerate() {
            let (width, fmt) = (&c.2, &c.3);
            let mut text = wide(column_title(c));
            let col = LVCOLUMNW {
                mask: LVCF_FMT | LVCF_WIDTH | LVCF_TEXT | LVCF_SUBITEM,
                fmt: *fmt,
                cx: self.s(*width),
                pszText: text.as_mut_ptr(),
                cchTextMax: 0,
                iSubItem: i as i32,
                iImage: 0,
                iOrder: 0,
                cxMin: 0,
                cxDefault: 0,
                cxIdeal: 0,
            };
            send_ptr(list, LVM_INSERTCOLUMNW, i, &col);
        }
        send(list, LVM_SETIMAGELIST, LVSIL_SMALL, self.icons.list);
        unsafe {
            (comctl().set_window_subclass)(list, list_subclass, side.index(), 0);
        }
        list
    }

    /// 兩欄都重新套用目前的檢視模式（圖示類檢視另外準備大圖示）。
    pub(super) fn apply_view(&mut self) {
        for side in [Side::Left, Side::Right] {
            self.apply_view_side(side);
        }
    }

    /// 這一欄的資料夾該用的檢視：有記錄就用記錄，否則用這個資料夾的預設（搜尋結果另外記一份）。
    pub(super) fn desired_view(&self, side: Side) -> ViewMode {
        let default = self.settings.view_mode;
        let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return default };
        let key = if tab.search_mode { folderviews::SEARCH_KEY } else { tab.path.as_str() };
        self.views.get(key).unwrap_or_else(|| self.folder_default_view(tab))
    }

    /// 資料夾沒記住檢視時用的：相片資料夾用大圖示顯示縮圖（和檔案總管相同），除非預設的檢視本來就有縮圖，
    /// 或使用者在「檢視 › 顯示」關掉了；其餘用預設的檢視。
    pub(super) fn folder_default_view(&self, tab: &app::Tab) -> ViewMode {
        let default = self.settings.view_mode;
        if self.settings.media_folder_icons && tab.media && !tab.search_mode && !default.shows_thumbnails() {
            ViewMode::Large
        } else {
            default
        }
    }

    /// 這一欄顯示縮圖時的方格邊長（目前的檢視不顯示縮圖時是 None）
    pub(super) fn thumb_edge(&self, side: Side) -> Option<i32> {
        self.view[side.index()].big_icon_px().map(|base| view::scaled_icon_px(base, self.theme.dpi))
    }

    /// 收到縮圖後（已合併）：重畫顯示縮圖的清單
    pub(super) fn redraw_thumbnails(&self) {
        for side in [Side::Left, Side::Right] {
            if self.thumb_edge(side).is_some() && (side == Side::Left || self.ws.dual) {
                invalidate(self.lists[side.index()]);
            }
        }
    }

    /// 換了資料夾（或分頁、開始搜尋）時，切成這個資料夾記住的檢視。
    pub(super) fn sync_view(&mut self, side: Side) {
        let want = self.desired_view(side);
        if self.view[side.index()] != want {
            self.view[side.index()] = want;
            self.apply_view_side(side);
        }
    }

    pub(super) fn apply_view_side(&mut self, side: Side) {
        let list = self.lists[side.index()];
        if list == 0 {
            return;
        }
        let slot = side.index();
        let mode = self.view[slot];
        // 縮圖大小跟著檢視：還沒做的要求作廢，畫面上的項目重畫時再要求
        self.thumbs.forget_queue();
        let old_big = mode.big_icon_px().and_then(|base| self.icons.ensure_big(slot, view::scaled_icon_px(base, self.theme.dpi)));
        let lv = match mode {
            ViewMode::ExtraLarge | ViewMode::Large | ViewMode::Medium => LV_VIEW_ICON,
            ViewMode::Small => LV_VIEW_SMALLICON,
            ViewMode::List => LV_VIEW_LIST,
            ViewMode::Details => LV_VIEW_DETAILS,
            ViewMode::Tiles | ViewMode::Content => LV_VIEW_TILE,
        };
        send(list, LVM_SETIMAGELIST, LVSIL_NORMAL, self.icons.big[slot]);
        if self.icons.big[slot] != 0 {
            // 依新的圖示大小重算預設間距
            send(list, LVM_SETICONSPACING, 0, -1);
        }
        self.fit_tiles(side);
        if send(list, LVM_SETVIEW, lv, 0) == -1 && lv == LV_VIEW_TILE {
            send(list, LVM_SETVIEW, LV_VIEW_ICON, 0);
        }
        self.list_fx[slot].clear();
        self.list_top[slot] = -1;
        let focused = next_item(list, -1, LVNI_FOCUSED);
        if focused >= 0 {
            send(list, LVM_ENSUREVISIBLE, focused as usize, 0);
        }
        self.fit_columns(side);
        invalidate(list);
        if let Some(old) = old_big {
            unsafe {
                (comctl().image_list_destroy)(old);
            }
        }
    }

    /// 並排與內容檢視的格子寬度與額外文字列（內容檢視一列佔滿整個寬度）。
    fn fit_tiles(&self, side: Side) {
        let mode = self.view[side.index()];
        if !matches!(mode, ViewMode::Tiles | ViewMode::Content) {
            return;
        }
        let list = self.lists[side.index()];
        let r = client_rect(list);
        let has_scroll = unsafe { GetWindowLongPtrW(list, GWL_STYLE) } as u32 & WS_VSCROLL != 0;
        let scroll = if has_scroll { 0 } else { unsafe { GetSystemMetrics(SM_CXVSCROLL) } };
        let width = match mode {
            ViewMode::Tiles => self.s(270).min(r.width() - scroll - self.s(4)),
            _ => r.width() - scroll - self.s(4),
        }
        .max(self.s(140));
        let info = LVTILEVIEWINFO {
            cbSize: std::mem::size_of::<LVTILEVIEWINFO>() as u32,
            dwMask: LVTVIM_TILESIZE | LVTVIM_COLUMNS | LVTVIM_LABELMARGIN,
            dwFlags: LVTVIF_FIXEDWIDTH,
            sizeTile: SIZE { cx: width, cy: 0 },
            cLines: tile_columns(mode, false).len() as i32,
            rcLabelMargin: RECT { left: self.s(8), top: self.s(2), right: self.s(8), bottom: self.s(2) },
        };
        send_ptr(list, LVM_SETTILEVIEWINFO, 0, &info);
    }

    pub(super) fn style_list(&self, side: Side) {
        let list = self.lists[side.index()];
        let p = self.theme.pal;
        theme::apply_explorer_theme(list, p.dark);
        theme::apply_header_theme(header_of(list), p.dark);
        send(list, LVM_SETBKCOLOR, 0, p.surface as LPARAM);
        send(list, LVM_SETTEXTBKCOLOR, 0, 0xFFFF_FFFFu32 as LPARAM); // CLR_NONE
        send(list, LVM_SETTEXTCOLOR, 0, p.ink as LPARAM);
        invalidate(list);
        invalidate(header_of(list));
    }

    fn save_view(&mut self, side: Side, tab: TabId) {
        let list = self.lists[side.index()];
        let selected = self.list_selected_indices(list);
        let focused = next_item(list, -1, LVNI_FOCUSED) as i32;
        let top = send(list, LVM_GETTOPINDEX, 0, 0) as i32;
        if let Some(t) = self.tabs.get_mut(&tab) {
            t.view = app::ViewState { selected, focused, top };
        }
    }

    /// 讓清單顯示該欄目前選取的分頁。
    pub(super) fn bind(&mut self, side: Side) {
        let list = self.lists[side.index()];
        let new_id = self.ws.current(side);
        if self.bound[side.index()] == new_id {
            self.refresh_list(side, false);
            return;
        }
        if let Some(old) = self.bound[side.index()] {
            if self.tabs.contains_key(&old) {
                self.save_view(side, old);
            }
        }
        send(list, LVM_CANCELEDITLABEL, 0, 0);
        self.bound[side.index()] = new_id;
        self.refresh_list(side, true);
        // 背景分頁不掃描：設定變更或檔案作業後標記為過期，切回時才重新讀取。
        if let Some(id) = new_id {
            // 還原的分頁：第一次顯示時才讀取
            self.load_restored(id);
            if self.tabs.get(&id).is_some_and(|t| t.stale && !t.working() && !t.search_mode) {
                self.refresh_tab(id);
            }
            // 隱藏期間沒有監看：補讀一次，有變才更新
            self.watch_catch_up(id);
        }
    }

    /// 同步清單筆數；reset 時重新套用選取與捲動位置。
    pub(super) fn refresh_list(&mut self, side: Side, reset: bool) {
        let list = self.lists[side.index()];
        let Some(id) = self.bound[side.index()] else {
            send(list, LVM_SETITEMCOUNT, 0, 0);
            return;
        };
        // 每個資料夾記住自己的檢視
        self.sync_view(side);
        if reset {
            // 內容換了：還沒做的縮圖要求作廢，另一欄畫面上的項目重畫時再要求
            self.thumbs.forget_queue();
            if self.thumb_edge(side.other()).is_some() {
                invalidate(self.lists[side.other().index()]);
            }
        }
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        let n = tab.row_count();
        if !reset {
            send(list, LVM_SETITEMCOUNT, n, (LVSICF_NOINVALIDATEALL | LVSICF_NOSCROLL) as LPARAM);
            invalidate(list);
            return;
        }
        self.updating_list = true;
        // 內容整個換掉：列的動畫狀態從頭開始
        self.list_fx[side.index()].clear();
        self.list_top[side.index()] = -1;
        set_item_state(list, -1, 0, LVIS_SELECTED | LVIS_FOCUSED);
        send(list, LVM_SETITEMCOUNT, n, 0);
        let pending = std::mem::take(&mut tab.pending_select);
        let rename = std::mem::replace(&mut tab.pending_rename, false);
        let mut focus_index: Option<usize> = None;
        if !pending.is_empty() && !tab.loading {
            // 以小寫路徑集合比對，選取數量很多時也維持線性時間。
            let wanted: std::collections::HashSet<String> =
                pending.iter().map(|p| paths::trim_end_sep(&paths::normalize_separators(p)).to_lowercase()).collect();
            for (i, e) in (0..n).filter_map(|i| tab.row(i).map(|e| (i, e))) {
                if wanted.contains(&e.full_path().to_lowercase()) {
                    set_item_state(list, i as isize, LVIS_SELECTED, LVIS_SELECTED);
                    focus_index.get_or_insert(i);
                }
            }
            if let Some(f) = focus_index {
                set_item_state(list, f as isize, LVIS_FOCUSED, LVIS_FOCUSED);
                send(list, LVM_SETSELECTIONMARK, 0, f as LPARAM);
                send(list, LVM_ENSUREVISIBLE, f, 0);
            }
        } else if tab.loading && !pending.is_empty() {
            // 讀取尚未完成：保留到結果回來
            tab.pending_select = pending;
            tab.pending_rename = rename;
        } else {
            let view = std::mem::take(&mut tab.view);
            for i in &view.selected {
                if *i < n {
                    set_item_state(list, *i as isize, LVIS_SELECTED, LVIS_SELECTED);
                }
            }
            if view.focused >= 0 && (view.focused as usize) < n {
                set_item_state(list, view.focused as isize, LVIS_FOCUSED, LVIS_FOCUSED);
            }
            if n > 0 {
                // 先回到頂端，再捲到先前的位置（新資料夾則停在頂端）。
                send(list, LVM_ENSUREVISIBLE, 0, 0);
                if view.top > 0 {
                    let page = send(list, LVM_GETCOUNTPERPAGE, 0, 0).max(1) as usize;
                    let last = (view.top as usize + page - 1).min(n - 1);
                    send(list, LVM_ENSUREVISIBLE, last, 0);
                    send(list, LVM_ENSUREVISIBLE, view.top as usize, 0);
                }
            }
            tab.view = view;
        }
        self.updating_list = false;
        let status = self.tabs.get(&id).map(|t| t.status.clone()).unwrap_or_default();
        set_window_text(self.pane_status[side.index()], &status);
        self.update_sort_arrows(side);
        self.fit_columns(side);
        invalidate(list);
        if rename {
            if let Some(f) = focus_index {
                unsafe {
                    SetFocus(list);
                }
                send(list, LVM_EDITLABELW, f, 0);
            }
        }
    }

    /// 自動重新整理前，記下清單的選取、焦點、捲動位置與滑鼠下的項目（以路徑記，項目增減後仍對得回去）。
    pub(super) fn keep_view(&self, side: Side) -> KeptView {
        let list = self.lists[side.index()];
        let tab = self.bound[side.index()].and_then(|id| self.tabs.get(&id));
        let key = |i: usize| tab.and_then(|t| t.row(i)).map(|e| e.full_path().to_lowercase());
        let selected = self.list_selected_indices(list).into_iter().filter_map(&key).collect();
        let focused_index = next_item(list, -1, LVNI_FOCUSED);
        let focused = if focused_index >= 0 { key(focused_index as usize) } else { None };
        let hovered = match self.hover.item {
            Some((s, row)) if s == side && row >= 0 => key(row as usize),
            _ => None,
        };
        KeptView { selected, focused, focused_index, top: send(list, LVM_GETTOPINDEX, 0, 0), hovered }
    }

    /// 分頁的項目已換成重新讀取的結果（同一個資料夾）：更新清單筆數，還原選取與焦點，捲動位置不動。
    /// 和 refresh_list(reset) 不同，這裡不回到頂端，也不交叉淡化，畫面只換掉有變的列。
    pub(super) fn restore_view(&mut self, side: Side, kept: KeptView) {
        let list = self.lists[side.index()];
        let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return };
        let n = tab.row_count();
        self.updating_list = true;
        // 列號可能都位移了：選取直接畫成新的狀態，不播放淡入淡出；
        // 懸停跟著滑鼠的位置，保留（不然每次更新，滑鼠下那一列都會重新淡入）
        self.list_fx[side.index()].reset_seen();
        set_item_state(list, -1, 0, LVIS_SELECTED | LVIS_FOCUSED);
        send(list, LVM_SETITEMCOUNT, n, LVSICF_NOSCROLL as LPARAM);
        let mut focus: Option<usize> = None;
        if !kept.selected.is_empty() || kept.focused.is_some() {
            for i in 0..n {
                let Some(e) = tab.row(i) else { continue };
                let p = e.full_path().to_lowercase();
                if kept.selected.contains(&p) {
                    set_item_state(list, i as isize, LVIS_SELECTED, LVIS_SELECTED);
                }
                if kept.focused.as_deref() == Some(p.as_str()) {
                    focus = Some(i);
                }
            }
        }
        // 有焦點的項目被移走或刪除了：焦點留在原來的位置附近，鍵盤可以接著操作（不選取）
        if focus.is_none() && kept.focused_index >= 0 && n > 0 {
            focus = Some((kept.focused_index as usize).min(n - 1));
        }
        if let Some(f) = focus {
            set_item_state(list, f as isize, LVIS_FOCUSED, LVIS_FOCUSED);
            send(list, LVM_SETSELECTIONMARK, 0, f as LPARAM);
        }
        // 項目變少、原本捲到的位置已經超過結尾：捲回最後一列
        if n > 0 && kept.top >= n as isize {
            send(list, LVM_ENSUREVISIBLE, n - 1, 0);
        }
        // 滑鼠下的列換成別的項目了：懸停預覽重新計時，不要繼續顯示原本那個
        let hover_moved = kept.hovered.is_some()
            && match self.hover.item {
                Some((s, row)) if s == side && row >= 0 => tab.row(row as usize).map(|e| e.full_path().to_lowercase()) != kept.hovered,
                _ => true,
            };
        let status = tab.status.clone();
        self.updating_list = false;
        set_window_text(self.pane_status[side.index()], &status);
        invalidate(list);
        if hover_moved {
            self.reset_hover();
        }
    }

    pub(super) fn fit_columns(&self, side: Side) {
        let list = self.lists[side.index()];
        if list == 0 {
            return;
        }
        let r = client_rect(list);
        let avail = (r.width() - 2).max(self.s(160));
        let wide_mode = avail >= self.s(430);
        let search = self.bound[side.index()].and_then(|id| self.tabs.get(&id)).is_some_and(|t| t.search_mode);
        let size = if wide_mode { self.s(78) } else { self.s(62) };
        let date = if wide_mode { self.s(140) } else { self.s(112) };
        let kind = if wide_mode { self.s(118) } else { 0 };
        let location = if (search && avail >= self.s(600)) || avail >= self.s(800) { self.s(280).min(avail / 3) } else { 0 };
        let name = (avail - size - date - kind - location).max(self.s(60));
        for (i, w) in [name, size, kind, date, location].iter().enumerate() {
            send(list, LVM_SETCOLUMNWIDTH, i, *w as LPARAM);
        }
        self.fit_tiles(side);
    }

    fn update_sort_arrows(&self, side: Side) {
        let list = self.lists[side.index()];
        let header = header_of(list);
        let (col, desc, show) = self.bound[side.index()]
            .and_then(|id| self.tabs.get(&id))
            .map(|t| (t.sort_col, t.desc, !t.search_mode || !t.searching))
            .unwrap_or((0, false, false));
        for i in 0..COLUMNS.len() {
            let mut item: HDITEMW = unsafe { std::mem::zeroed() };
            item.mask = HDI_FORMAT;
            send(header, HDM_GETITEMW, i, &mut item as *mut HDITEMW as LPARAM);
            item.fmt &= !(HDF_SORTUP | HDF_SORTDOWN);
            if show && i == col {
                item.fmt |= if desc { HDF_SORTDOWN } else { HDF_SORTUP };
            }
            send_ptr(header, HDM_SETITEMW, i, &item);
        }
    }

    fn list_selected_indices(&self, list: HWND) -> Vec<usize> {
        let mut out = Vec::new();
        let mut i = -1isize;
        loop {
            i = next_item(list, i, LVNI_SELECTED);
            if i < 0 {
                break;
            }
            out.push(i as usize);
            if out.len() > 1_000_000 {
                break;
            }
        }
        out
    }

    pub(super) fn selected_count(&self, side: Side) -> usize {
        if side == Side::Right && !self.ws.dual {
            return 0;
        }
        send(self.lists[side.index()], LVM_GETSELECTEDCOUNT, 0, 0).max(0) as usize
    }

    pub(super) fn selected_entries(&self, side: Side) -> Vec<Entry> {
        let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return vec![] };
        self.list_selected_indices(self.lists[side.index()]).into_iter().filter_map(|i| tab.row(i).cloned()).collect()
    }

    pub(super) fn selected_paths(&self, side: Side) -> Vec<String> {
        self.selected_entries(side).iter().map(Entry::full_path).collect()
    }

    /// 選取範圍之後第一個沒被選取的項目；選取已到最後就往前找（刪除後要選取的項目）。
    pub(super) fn row_after_selection(&self, side: Side) -> Option<String> {
        let tab = self.bound[side.index()].and_then(|id| self.tabs.get(&id))?;
        let selected = self.list_selected_indices(self.lists[side.index()]);
        let index = feather_core::view::next_unselected(&selected, tab.row_count())?;
        tab.row(index).map(Entry::full_path)
    }

    /// 若分頁目前顯示在清單上，回傳選取項目的路徑（重新整理後用來還原選取）。
    pub(super) fn bound_selection_paths(&self, tab: TabId) -> Vec<String> {
        match self.ws.side_of(tab) {
            Some(side) if self.bound[side.index()] == Some(tab) => self.selected_paths(side),
            _ => vec![],
        }
    }

    /// 打字跳到名稱開頭相符的項目（見 App::type_to_find）。
    pub(super) fn list_type_ahead(&mut self, side: Side, query: &str) {
        let list = self.lists[side.index()];
        let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return };
        let show_ext = self.settings.show_extensions;
        let focused = next_item(list, -1, LVNI_FOCUSED);
        let current = (focused >= 0).then_some(focused as usize);
        let found = typeahead::find(tab.row_count(), current, query, |i| tab.row(i).map(|e| e.display_name(show_ext).to_string()));
        let Some(i) = found else { return };
        set_item_state(list, -1, 0, LVIS_SELECTED);
        set_item_state(list, i as isize, LVIS_SELECTED | LVIS_FOCUSED, LVIS_SELECTED | LVIS_FOCUSED);
        send(list, LVM_SETSELECTIONMARK, 0, i as LPARAM);
        send(list, LVM_ENSUREVISIBLE, i, 0);
    }

    pub(super) fn select_all(&self, side: Side) {
        set_item_state(self.lists[side.index()], -1, LVIS_SELECTED, LVIS_SELECTED);
    }

    pub(super) fn focus_list(&self, side: Side) {
        unsafe {
            SetFocus(self.lists[side.index()]);
        }
    }

    pub(super) fn begin_rename(&mut self) {
        let side = self.active_side();
        if self.active_tab().is_none_or(|t| t.working()) {
            return;
        }
        let list = self.lists[side.index()];
        let sel = self.list_selected_indices(list);
        if sel.len() != 1 {
            self.update_chrome();
            self.show_footer(tr!("請選取一個項目再重新命名", "Select one item to rename"));
            return;
        }
        unsafe {
            SetFocus(list);
        }
        send(list, LVM_EDITLABELW, sel[0], 0);
    }

    pub(super) fn show_footer(&mut self, text: &str) {
        self.set_footer(text.to_string());
    }

    pub(super) fn finish_rename(&mut self, tab: TabId, old: &str, typed: &str) {
        let name = typed.trim().to_string();
        let old_name = paths::file_name(old).to_string();
        let is_file = super::fs::is_file(old);
        let mut new_name = name.clone();
        if is_file && !self.settings.show_extensions {
            if let Some(i) = old_name.rfind('.').filter(|i| *i > 0) {
                new_name.push_str(&old_name[i..]);
            }
        }
        if new_name == old_name {
            return;
        }
        if !paths::valid_name(&new_name) || paths::is_reserved_device_name(&new_name) {
            self.show_error(tr!("請輸入有效名稱，且不要在結尾使用空白或句點，也不要使用 \\ / : * ? \" < > | 等字元或 CON、NUL 等保留名稱。", "Enter a valid name. Don't end it with a space or period, and don't use characters such as \\ / : * ? \" < > | or reserved names such as CON and NUL."));
            return;
        }
        let Some(dir) = paths::parent(old) else { return };
        let target = paths::join(&dir, &new_name);
        match super::fileops::rename(old, &target) {
            Ok(()) => {
                self.record_rename(old, &target);
                let in_search = self.tabs.get(&tab).is_some_and(|t| t.search_mode);
                if in_search {
                    // 搜尋結果中直接更新那一列
                    if let Some(t) = self.tabs.get_mut(&tab) {
                        if let Some(e) = t.entries.iter_mut().find(|e| paths::same_path(&e.full_path(), old)) {
                            e.name = new_name;
                        }
                        t.resummarize();
                        t.refilter();
                    }
                    for side in [Side::Left, Side::Right] {
                        if self.bound[side.index()] == Some(tab) {
                            invalidate(self.lists[side.index()]);
                        }
                    }
                } else {
                    self.navigate(tab, &dir, NavKind::NoHistory, vec![target]);
                }
            }
            Err(e) => self.show_error(&trf!("無法重新命名：{e}", "Couldn't rename: {e}")),
        }
    }

    /// 按欄位標題：同一欄再按一次就反向。
    fn sort_by(&mut self, side: Side, col: usize) {
        let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return };
        let desc = if tab.sort_col == col { !tab.desc } else { false };
        self.sort_to(side, col, desc);
    }

    pub(super) fn sort_to(&mut self, side: Side, col: usize, desc: bool) {
        let Some(id) = self.bound[side.index()] else { return };
        let _fade = match self.tabs.get(&id) {
            Some(t) if !t.working() && (t.sort_col != col || t.desc != desc) => {
                self.crossfade(vec![self.layout.panes[side.index()].inflate(self.s(4), self.s(6))], anim::CROSSFADE_MS)
            }
            _ => anim::Crossfade::none(),
        };
        let selected = self.selected_paths(side);
        let folders_on_top = self.settings.folders_on_top;
        let Some(tab) = self.tabs.get_mut(&id) else { return };
        if tab.working() {
            return;
        }
        tab.desc = desc;
        tab.sort_col = col;
        sorting::sort_entries(&mut tab.entries, col, tab.desc, folders_on_top);
        tab.refilter();
        tab.pending_select = selected;
        tab.view = Default::default();
        self.refresh_list(side, true);
    }

    pub(super) fn list_notify(&mut self, side: Side, code: u32, lparam: LPARAM) -> LRESULT {
        unsafe {
            match code {
                LVN_BEGINDRAG | LVN_BEGINRDRAG => {
                    self.begin_drag(side, code == LVN_BEGINRDRAG);
                    0
                }
                LVN_GETDISPINFOW => {
                    let info = &mut *(lparam as *mut NMLVDISPINFOW);
                    let show_ext = self.settings.show_extensions;
                    let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return 0 };
                    let idx = info.item.iItem;
                    let Some(e) = (idx >= 0).then(|| tab.row(idx as usize)).flatten() else { return 0 };
                    if info.item.mask & LVIF_TEXT != 0 {
                        let text = match info.item.iSubItem {
                            0 => e.display_name(show_ext).to_string(),
                            1 => format_size(e.size()),
                            2 => e.type_label(),
                            3 => format_filetime(e.modified),
                            4 => e.dir.to_string(),
                            _ => String::new(),
                        };
                        copy_to_buffer(&text, info.item.pszText, info.item.cchTextMax);
                    }
                    if info.item.mask & LVIF_IMAGE != 0 && info.item.iSubItem == 0 {
                        // 已經有縮圖：圖示用透明的那一格，縮圖在自繪的最後畫上去
                        let thumb = self.thumb_edge(side).is_some_and(|edge| self.thumbs.has(e, edge));
                        info.item.iImage = if thumb { self.icons.blank_index() } else { self.icons.index_for(e) };
                    }
                    if info.item.mask & LVIF_COLUMNS != 0 && !info.item.puColumns.is_null() {
                        // 並排／內容檢視：名稱下方顯示的欄位（資料夾不顯示大小）
                        let mode = self.view[side.index()];
                        let cols = tile_columns(mode, e.is_dir);
                        let room = tile_columns(mode, false).len();
                        let cap = if info.item.cColumns > 0 { (info.item.cColumns as usize).min(room) } else { room };
                        let n = cols.len().min(cap);
                        for (k, c) in cols.iter().take(n).enumerate() {
                            *info.item.puColumns.add(k) = *c;
                            if info.item.mask & LVIF_COLFMT != 0 && !info.item.piColFmt.is_null() {
                                *info.item.piColFmt.add(k) = LVCFMT_LEFT;
                            }
                        }
                        info.item.cColumns = n as u32;
                    }
                    0
                }
                LVN_ODFINDITEMW => {
                    let fi = &*(lparam as *const NMLVFINDITEMW);
                    if fi.lvfi.flags & (LVFI_STRING | LVFI_PARTIAL) == 0 || fi.lvfi.psz.is_null() {
                        return -1;
                    }
                    let needle = from_wide_ptr(fi.lvfi.psz).to_lowercase();
                    let show_ext = self.settings.show_extensions;
                    let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return -1 };
                    let n = tab.row_count();
                    if n == 0 || needle.is_empty() {
                        return -1;
                    }
                    let start = (fi.iStart.max(0) as usize).min(n - 1);
                    for k in 0..n {
                        let i = (start + k) % n;
                        if tab.row(i).is_some_and(|e| e.display_name(show_ext).to_lowercase().starts_with(&needle)) {
                            return i as LRESULT;
                        }
                    }
                    // 回傳 -1 會讓清單發出提示音；找不到時停在目前的項目。
                    let focused = next_item(self.lists[side.index()], -1, LVNI_FOCUSED);
                    focused.max(0) as LRESULT
                }
                NM_CUSTOMDRAW => self.list_custom_draw(side, lparam),
                LVN_ITEMCHANGED | LVN_ODSTATECHANGED => {
                    self.selection_changed(side);
                    0
                }
                LVN_ITEMACTIVATE => {
                    self.open_selection(side);
                    0
                }
                LVN_COLUMNCLICK => {
                    let nm = &*(lparam as *const NMLISTVIEW);
                    self.sort_by(side, nm.iSubItem.max(0) as usize);
                    0
                }
                NM_SETFOCUS => {
                    self.activate(side);
                    0
                }
                LVN_BEGINLABELEDITW => {
                    let info = &*(lparam as *const NMLVDISPINFOW);
                    let Some(tab) = self.bound[side.index()].and_then(|id| self.tabs.get(&id)) else { return 1 };
                    if tab.working() || info.item.iItem < 0 {
                        return 1;
                    }
                    let Some(e) = tab.row(info.item.iItem as usize) else { return 1 };
                    // 預設只選取主檔名（不含副檔名）
                    let shown = e.display_name(self.settings.show_extensions).to_string();
                    let is_dir = e.is_dir;
                    let Some(id) = self.bound[side.index()] else { return 1 };
                    self.renaming = Some((id, e.full_path()));
                    self.hide_preview();
                    let end = if !is_dir && self.settings.show_extensions {
                        shown.rfind('.').filter(|i| *i > 0).map(|i| shown[..i].encode_utf16().count()).unwrap_or(shown.encode_utf16().count())
                    } else {
                        shown.encode_utf16().count()
                    };
                    let edit = send(self.lists[side.index()], LVM_GETEDITCONTROL, 0, 0);
                    if edit != 0 {
                        PostMessageW(edit, EM_SETSEL, 0, end as LPARAM);
                    }
                    0
                }
                LVN_ENDLABELEDITW => {
                    let info = &*(lparam as *const NMLVDISPINFOW);
                    // 開始編輯時記下的項目；清單在編輯中重新整理過（背景作業完成、搜尋結果陸續加入），
                    // 這一列可能已經是別的檔案，所以不用列號找項目。
                    let started = self.renaming.take();
                    if info.item.pszText.is_null() || info.item.iItem < 0 {
                        return 0;
                    }
                    let typed = from_wide_ptr(info.item.pszText);
                    let Some((id, old)) = started else { return 0 };
                    if !self.tabs.contains_key(&id) {
                        return 0;
                    }
                    let data = Box::into_raw(Box::new((id, old, typed)));
                    if PostMessageW(self.hwnd, WM_APP_RENAME, 0, data as LPARAM) == 0 {
                        drop(Box::from_raw(data));
                    }
                    0
                }
                _ => 0,
            }
        }
    }

    /// 清單列：選取與懸停改畫成圓角膠囊（淡陶土橘），取代系統的藍色反白；
    /// 選取與懸停的變化以短動畫淡入淡出。
    unsafe fn list_custom_draw(&mut self, side: Side, lparam: LPARAM) -> LRESULT {
        let cd = &mut *(lparam as *mut NMLVCUSTOMDRAW);
        let list = self.lists[side.index()];
        let si = side.index();
        match cd.nmcd.dwDrawStage {
            CDDS_PREPAINT => {
                // 捲動後才進入畫面的列直接顯示目前狀態，不播放動畫。
                let top = send(list, LVM_GETTOPINDEX, 0, 0) as isize;
                if top != self.list_top[si] {
                    self.list_top[si] = top;
                    self.list_fx[si].reset_seen();
                }
                CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT
            }
            CDDS_ITEMPREPAINT => {
                let index = cd.nmcd.dwItemSpec;
                let p = self.theme.pal;
                let selected = send(list, LVM_GETITEMSTATE, index, LVIS_SELECTED as LPARAM) as u32 & LVIS_SELECTED != 0;
                let hot = cd.nmcd.uItemState & CDIS_HOT != 0;
                let (sv, hv) = self.list_fx[si].observe(index, selected, hot, anim::now(), anim::dur(anim::ROW_MS), anim::dur(anim::HOVER_MS));
                if self.drop_row == Some((side, index)) {
                    // 拖放到這個資料夾：重點色外框
                    let mut r = RECT { left: LVIR_BOUNDS, ..Default::default() };
                    if send(list, LVM_GETITEMRECT, index, &mut r as *mut RECT as LPARAM) != 0 {
                        let r = RECT { left: r.left.max(0) + self.s(2), top: r.top + 1, right: r.right - self.s(2), bottom: r.bottom - 1 };
                        theme::round_rect(cd.nmcd.hdc, &r, self.s(12), Some(p.selected_tint), Some(p.accent));
                    }
                } else if sv > 0.001 || hv > 0.001 {
                    let mut r = RECT { left: LVIR_BOUNDS, ..Default::default() };
                    if send(list, LVM_GETITEMRECT, index, &mut r as *mut RECT as LPARAM) != 0 {
                        let r = RECT { left: r.left.max(0) + self.s(2), top: r.top + 1, right: r.right - self.s(2), bottom: r.bottom - 1 };
                        let focused = GetFocus() == list;
                        let sel_color = if focused { p.selected_tint } else { theme::blend(p.selected_tint, p.surface, 0.6) };
                        let fill = anim::mix(anim::mix(p.surface, p.hover, hv), sel_color, sv);
                        theme::round_rect(cd.nmcd.hdc, &r, self.s(12), Some(fill), None);
                    }
                }
                cd.nmcd.uItemState &= !(CDIS_SELECTED | CDIS_HOT | CDIS_FOCUS);
                cd.clrText = p.ink;
                cd.clrTextBk = CLR_NONE;
                // 圖示類的檢視：有縮圖就在最後畫上去，還沒有就排進佇列
                let mut result = CDRF_DODEFAULT;
                if let Some(edge) = self.thumb_edge(side) {
                    let owner = self.hwnd;
                    let thumbs = &mut self.thumbs;
                    if let Some(e) = self.bound[si].and_then(|id| self.tabs.get(&id)).and_then(|t| t.row(index)) {
                        if thumbs.get_or_request(owner, e, edge).is_some() {
                            result = CDRF_NOTIFYPOSTPAINT;
                        }
                    }
                }
                result
            }
            CDDS_ITEMPOSTPAINT => {
                let index = cd.nmcd.dwItemSpec;
                if let Some(edge) = self.thumb_edge(side) {
                    let thumbs = &mut self.thumbs;
                    let bmp = self.bound[si].and_then(|id| self.tabs.get(&id)).and_then(|t| t.row(index)).and_then(|e| thumbs.cached(e, edge));
                    let mut r = RECT { left: LVIR_ICON, ..Default::default() };
                    if let Some(bmp) = bmp {
                        if send(list, LVM_GETITEMRECT, index, &mut r as *mut RECT as LPARAM) != 0 {
                            // 圖示檢視貼齊方格底部（靠近名稱）；並排、內容檢視垂直置中
                            let bottom = matches!(self.view[si], ViewMode::ExtraLarge | ViewMode::Large | ViewMode::Medium);
                            super::thumbs::draw(cd.nmcd.hdc, &bmp, r, edge, bottom);
                        }
                    }
                }
                CDRF_DODEFAULT
            }
            CDDS_POSTPAINT => {
                let busy = self.list_fx[si].animating(anim::now());
                let rects: Vec<RECT> = busy
                    .into_iter()
                    .filter_map(|i| {
                        let mut r = RECT { left: LVIR_BOUNDS, ..Default::default() };
                        (send(list, LVM_GETITEMRECT, i, &mut r as *mut RECT as LPARAM) != 0).then_some(r)
                    })
                    .collect();
                anim::request_rects(list, rects);
                CDRF_DODEFAULT
            }
            _ => CDRF_DODEFAULT,
        }
    }

    fn selection_changed(&mut self, side: Side) {
        if self.updating_list {
            return;
        }
        if self.picker_mode() {
            self.picker_selection_changed(side);
        }
        if self.active_side() == side {
            self.clear_footer_message();
            self.update_chrome();
            // 快速預覽開著時跟著換項目
            if preview::is_open(&self.preview) {
                let target = self.quicklook_target(side);
                let owner = self.hwnd;
                preview::follow(&mut self.preview, owner, target);
            }
        }
    }

    pub(super) fn list_context_menu(&mut self, side: Side, pt: POINT, keyboard: bool) {
        self.activate(side);
        self.hide_preview();
        let list = self.lists[side.index()];
        let mut pt = pt;
        if keyboard {
            let focused = next_item(list, -1, LVNI_FOCUSED);
            let mut r = RECT { left: LVIR_BOUNDS, ..Default::default() };
            let mut p = POINT::default();
            if focused >= 0 && send(list, LVM_GETITEMRECT, focused as usize, &mut r as *mut RECT as LPARAM) != 0 {
                p = POINT { x: r.left + self.s(24), y: r.bottom };
            }
            unsafe {
                ClientToScreen(list, &mut p);
            }
            pt = p;
        }
        let entries = self.selected_entries(side);
        let tab = self.bound[side.index()].and_then(|id| self.tabs.get(&id));
        let tab_search = tab.is_some_and(|t| t.search_mode);
        let folder = tab.map(|t| t.path.clone()).unwrap_or_default();
        let can_paste = super::clipboard::has_files();
        // 選取的資料夾中，哪些已釘選在常用位置、哪些還沒有
        let dirs: Vec<String> = entries.iter().filter(|e| e.is_dir).map(Entry::full_path).collect();
        let (pinned, unpinned): (Vec<String>, Vec<String>) = if entries.is_empty() {
            let here = (!tab_search && !folder.is_empty()).then(|| folder.clone()).into_iter();
            here.partition(|p| self.is_pinned(p))
        } else {
            dirs.iter().cloned().partition(|p| self.is_pinned(p))
        };
        // 資料夾空白處的選單最上面是「復原…」（與檔案總管相同）
        let undo_text = self.undo_label().map(|l| format!("{l}\tCtrl+Z"));
        let mut organize: Vec<Cmd> = Vec::new();
        let (pin_text, unpin_text) = if entries.is_empty() {
            (tr!("釘選這個資料夾到常用位置", "Pin this folder to Favorites"), tr!("從常用位置取消釘選這個資料夾", "Unpin this folder from Favorites"))
        } else {
            (tr!("釘選到常用位置", "Pin to Favorites"), tr!("從常用位置取消釘選", "Unpin from Favorites"))
        };
        if !unpinned.is_empty() {
            organize.push(Cmd::new(id::ADD_FAVORITE, pin_text, glyph::PIN));
        }
        if !pinned.is_empty() {
            organize.push(Cmd::new(id::REMOVE_FAVORITE, unpin_text, glyph::UNPIN));
        }
        // 版面：Stillfeather 的命令分成「開啟／編輯／整理」三段放在上面，
        // 接著是 Windows 與已安裝程式的項目，最後是「內容」。
        let (parts, shell, hide): ([Part; 5], _, &[&str]) = if !entries.is_empty() {
            let mut open = vec![Cmd::new(id::OPEN, tr!("開啟\tEnter", "Open\tEnter"), glyph::OPEN), Cmd::new(id::QUICKLOOK, tr!("快速預覽\t空白鍵", "Quick preview\tSpace"), glyph::PREVIEW)];
            if !dirs.is_empty() {
                open.push(Cmd::new(id::OPEN_NEW_TAB, tr!("在新分頁開啟", "Open in new tab"), glyph::OPEN_NEW_TAB));
            }
            // 只選了一個資料夾：在它裡面開 PowerShell（同檔案總管）
            if entries.len() == 1 && entries[0].is_dir {
                open.push(Cmd::new(id::OPEN_POWERSHELL, powershell_text(), glyph::TERMINAL));
            }
            if tab_search {
                open.push(Cmd::new(id::OPEN_LOCATION, tr!("開啟所在位置", "Open file location"), glyph::FOLDER));
            }
            let edit = vec![
                Cmd::new(id::CUT, tr!("剪下\tCtrl+X", "Cut\tCtrl+X"), glyph::CUT),
                Cmd::new(id::COPY, tr!("複製\tCtrl+C", "Copy\tCtrl+C"), glyph::COPY),
                Cmd::new(id::PASTE, tr!("貼上\tCtrl+V", "Paste\tCtrl+V"), glyph::PASTE).enabled(can_paste),
                Cmd::new(id::RENAME, tr!("重新命名\tF2", "Rename\tF2"), glyph::RENAME).enabled(entries.len() == 1),
                Cmd::new(id::DELETE, tr!("刪除\tDelete", "Delete\tDelete"), glyph::DELETE),
            ];
            organize.push(Cmd::new(id::COPY_PATH, tr!("複製路徑\tCtrl+Shift+C", "Copy path\tCtrl+Shift+C"), glyph::LINK));
            let paths: Vec<String> = entries.iter().map(Entry::full_path).collect();
            let shell = shellmenu::ShellMenu::for_items(self.hwnd, &paths);
            let last = vec![Cmd::new(id::PROPERTIES, tr!("內容\tAlt+Enter", "Properties\tAlt+Enter"), glyph::PROPERTIES)];
            ([Part::Ours(open), Part::Ours(edit), Part::Ours(organize), Part::Shell, Part::Ours(last)], shell, ITEM_HIDDEN_VERBS)
        } else {
            let mut view = Vec::new();
            if let Some(text) = undo_text.as_deref() {
                view.push(Cmd::new(id::UNDO, text, glyph::UNDO));
            }
            view.push(Cmd::new(id::REFRESH, tr!("重新整理\tF5", "Refresh\tF5"), glyph::REFRESH));
            view.push(Cmd::new(id::SELECT_ALL, tr!("全選\tCtrl+A", "Select all\tCtrl+A"), glyph::SELECT_ALL));
            let edit = vec![
                Cmd::new(id::PASTE, tr!("貼上\tCtrl+V", "Paste\tCtrl+V"), glyph::PASTE).enabled(can_paste),
                Cmd::new(id::NEW_FOLDER, tr!("新增資料夾", "New folder"), glyph::NEW_FOLDER).enabled(!tab_search),
            ];
            organize.push(Cmd::new(id::COPY_PATH, tr!("複製目前資料夾路徑", "Copy current folder path"), glyph::LINK));
            // 在目前的資料夾開 PowerShell（搜尋結果、首頁沒有單一的資料夾）
            if !tab_search && !folder.is_empty() {
                organize.push(Cmd::new(id::OPEN_POWERSHELL, powershell_text(), glyph::TERMINAL));
            }
            let shell = (!tab_search && !folder.is_empty()).then(|| shellmenu::ShellMenu::for_background(self.hwnd, &folder)).flatten();
            let last = vec![Cmd::new(id::PROPERTIES, tr!("內容", "Properties"), glyph::PROPERTIES)];
            ([Part::Ours(view), Part::Ours(edit), Part::Ours(organize), Part::Shell, Part::Ours(last)], shell, BACKGROUND_HIDDEN_VERBS)
        };
        let default = (!entries.is_empty()).then_some(id::OPEN);
        let look = self.menu_look();
        let picked = shellmenu::popup(self.hwnd, &parts, shell.as_ref(), !entries.is_empty(), hide, default, pt, &look);
        match picked {
            None => {}
            Some(Pick::Shell(cmd)) => {
                if let Some(sm) = &shell {
                    if let Err(e) = sm.invoke(self.hwnd, cmd, &folder, pt) {
                        self.show_error(&e);
                    }
                }
                self.after_shell_command();
            }
            Some(Pick::Ours(id::ADD_FAVORITE)) => self.add_favorites(unpinned),
            Some(Pick::Ours(id::REMOVE_FAVORITE)) => self.remove_favorites(&pinned),
            Some(Pick::Ours(cmd)) => unsafe {
                SendMessageW(self.hwnd, WM_COMMAND, cmd as usize, 0);
            },
        }
    }

    /// 快速預覽的對象：清單的焦點項目（若有選取），否則第一個選取的項目。
    fn quicklook_target(&self, side: Side) -> Option<preview::Target> {
        let list = self.lists[side.index()];
        let focused = next_item(list, -1, LVNI_FOCUSED);
        let focused_selected = focused >= 0 && send(list, LVM_GETITEMSTATE, focused as usize, LVIS_SELECTED as LPARAM) as u32 & LVIS_SELECTED != 0;
        let index = if focused_selected { focused } else { next_item(list, -1, LVNI_SELECTED) };
        self.target_at(side, index as i32)
    }

    /// 清單第 `index` 列要預覽的項目。
    fn target_at(&self, side: Side, index: i32) -> Option<preview::Target> {
        let tab = self.bound[side.index()].and_then(|id| self.tabs.get(&id))?;
        let e = (index >= 0).then(|| tab.row(index as usize)).flatten()?;
        let mut meta = vec![e.type_label()];
        if !e.is_dir {
            meta.push(format_size(e.size()));
        }
        if e.modified != 0 {
            meta.push(trf!("修改於 {}", "Modified {}", format_filetime(e.modified)));
        }
        if tab.search_mode {
            meta.push(e.dir.to_string());
        }
        Some(preview::Target { path: e.full_path(), name: e.name.clone(), is_dir: e.is_dir, meta: meta.join(" · ") })
    }

    fn quicklook_area(&self) -> RECT {
        let mut r = RECT::default();
        unsafe {
            GetWindowRect(self.hwnd, &mut r);
        }
        r
    }

    fn preview_opts(&self) -> preview::Opts {
        preview::Opts { images: self.settings.image_preview, pdf: self.settings.pdf_preview, space: self.settings.space_preview }
    }

    /// 開啟選取項目的快速預覽；`hold` 表示按住空白鍵開啟（放開就關閉），否則（右鍵選單）維持開著。
    pub(super) fn open_quicklook(&mut self, side: Side, hold: bool) {
        let Some(target) = self.quicklook_target(side) else {
            self.set_footer(tr!("先選取一個項目，或把滑鼠停在項目上預覽", "Select an item first, or hover over an item to preview it").into());
            return;
        };
        let opts = self.preview_opts();
        let area = self.quicklook_area();
        let owner = self.hwnd;
        let mode = if hold { preview::Mode::Hold } else { preview::Mode::Pinned };
        preview::open(&mut self.preview, owner, target, area, POINT::default(), opts, mode);
    }

    /// 懸停中按住空白鍵：把同一個項目放大成置中的預覽，放開就關閉。
    pub(super) fn enlarge_hover_preview(&mut self) {
        let Some(target) = preview::target(&self.preview).cloned() else { return };
        // 只停止計時；預覽不關，同一個項目已載入的內容直接沿用
        if self.hover.armed {
            self.hover.armed = false;
            unsafe {
                KillTimer(self.hwnd, HOVER_TIMER);
            }
        }
        self.hover.suppressed = true;
        let opts = self.preview_opts();
        let area = self.quicklook_area();
        let owner = self.hwnd;
        preview::open(&mut self.preview, owner, target, area, POINT::default(), opts, preview::Mode::Hold);
    }

    // ------------------------------------------------------------ 懸停預覽

    /// 滑鼠（清單用戶區座標）下的列；不在項目上時是 -1。詳細資料檢視中任一欄都算。
    fn hover_row(&self, side: Side, pt: POINT) -> i32 {
        let list = self.lists[side.index()];
        let mut ht = LVHITTESTINFO { pt, flags: 0, iItem: -1, iSubItem: 0, iGroup: 0 };
        let index = send(list, LVM_SUBITEMHITTEST, 0, &mut ht as *mut LVHITTESTINFO as LPARAM) as i32;
        if index >= 0 && ht.flags & LVHT_ONITEM != 0 {
            index
        } else {
            -1
        }
    }

    fn arm_hover(&mut self, delay: u32) {
        self.hover.armed = true;
        unsafe {
            SetTimer(self.hwnd, HOVER_TIMER, delay, std::ptr::null());
        }
    }

    /// 停止等待並關掉懸停的預覽（置中的預覽不受影響）。
    fn stop_hover(&mut self) {
        if self.hover.armed {
            self.hover.armed = false;
            unsafe {
                KillTimer(self.hwnd, HOVER_TIMER);
            }
        }
        if preview::mode(&self.preview) == Some(preview::Mode::Hover) {
            self.hide_preview();
        }
        self.sync_label_tips();
    }

    /// 清單的「完整名稱提示」（名稱被截斷時，滑鼠停上去顯示的小方框）只在沒有預覽時才開。
    ///
    /// 這個提示會蓋在滑鼠所在的那一列上：滑鼠底下變成提示視窗，懸停預覽就以為滑鼠離開了清單而不換項目，
    /// 卡在上一個檔案；提示也會和預覽疊在一起。預覽的標題本來就顯示完整名稱，所以：
    /// - 一般模式（滑鼠停著就預覽）：一律不顯示提示。
    /// - 「按空白鍵預覽」：按住空白鍵或預覽開著時關掉，放開後恢復。
    pub(super) fn sync_label_tips(&mut self) {
        let want = self.settings.space_preview && !self.hover.gate && !preview::is_open(&self.preview);
        if self.hover.label_tips == Some(want) {
            return;
        }
        self.hover.label_tips = Some(want);
        for list in self.lists {
            if list == 0 {
                continue;
            }
            send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, LVS_EX_LABELTIP as usize, if want { LVS_EX_LABELTIP as LPARAM } else { 0 });
            if !want {
                // 已經顯示出來的提示立刻收起
                let tip = send(list, LVM_GETTOOLTIPS, 0, 0) as HWND;
                if tip != 0 {
                    send(tip, TTM_POP, 0, 0);
                }
            }
        }
    }

    /// 滑鼠底下的視窗算不算「在清單上」：清單本身、清單自己的提示視窗，或滑鼠穿透的預覽視窗。
    fn under_list(&self, list: HWND, under: HWND) -> bool {
        under == list || under == self.preview.popup || (under != 0 && under == send(list, LVM_GETTOOLTIPS, 0, 0) as HWND)
    }

    /// 點擊、按鍵等：關掉懸停的預覽，滑鼠移到別的項目之前不再出現。
    pub(super) fn cancel_hover(&mut self) {
        self.stop_hover();
        self.hover.suppressed = true;
    }

    /// 捲動清單：關掉懸停的預覽，滑鼠再移動時依新的項目重新計時。
    pub(super) fn reset_hover(&mut self) {
        self.stop_hover();
        self.hover.item = None;
    }

    pub(super) fn on_list_mouse_move(&mut self, side: Side, lparam: LPARAM) {
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        // 清單捲動或視窗變動時系統會補發位置沒變的移動訊息，略過，免得用鍵盤時跳出預覽
        if self.hover.last == Some((side, pt.x, pt.y)) {
            return;
        }
        self.hover.last = Some((side, pt.x, pt.y));
        let list = self.lists[side.index()];
        if !self.hover.tracking[side.index()] {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: list, dwHoverTime: 0 };
            unsafe {
                TrackMouseEvent(&mut tme);
            }
            self.hover.tracking[side.index()] = true;
        }
        let row = self.hover_row(side, pt);
        if row < 0 {
            self.reset_hover();
            return;
        }
        let item = Some((side, row));
        // 預覽已經開著，或正按住空白鍵（「按空白鍵預覽」）：很快就換過去
        let quick = self.hover.gate || preview::mode(&self.preview) == Some(preview::Mode::Hover);
        if self.hover.item != item {
            self.hover.item = item;
            self.hover.suppressed = false;
            if self.hover_allowed() {
                self.arm_hover(if quick { HOVER_SWITCH_MS } else { HOVER_DELAY_MS });
            }
        } else if self.hover_allowed() && !self.hover.suppressed && !self.hover.armed && !preview::is_open(&self.preview) {
            self.arm_hover(if quick { HOVER_SWITCH_MS } else { HOVER_DELAY_MS });
        }
    }

    /// 現在可以懸停預覽嗎：一般是隨時；「按空白鍵預覽」時只在空白鍵按著的時候。
    fn hover_allowed(&self) -> bool {
        !self.settings.space_preview || self.hover.gate
    }

    /// 「按空白鍵預覽」的空白鍵是否正按著。
    pub(super) fn space_gate_open(&self) -> bool {
        self.hover.gate
    }

    /// 「按空白鍵預覽」時這個空白鍵要不要拿來預覽：焦點在清單、資料夾樹或主視窗，或滑鼠在清單上。
    pub(super) fn space_gate_applies(&self, focus: HWND) -> bool {
        if self.side_of_list(focus).is_some() || focus == self.tree || focus == self.hwnd || focus == 0 {
            return true;
        }
        let mut pt = POINT::default();
        unsafe {
            GetCursorPos(&mut pt);
        }
        self.list_under_cursor(pt).is_some()
    }

    /// 滑鼠（螢幕座標）停在哪一欄的清單上；蓋在清單上的名稱提示或預覽視窗也算在清單上。
    fn list_under_cursor(&self, pt: POINT) -> Option<Side> {
        let under = unsafe { WindowFromPoint(pt) };
        if let Some(side) = self.side_of_list(under) {
            return Some(side);
        }
        [Side::Left, Side::Right].into_iter().find(|s| {
            let list = self.lists[s.index()];
            let mut r = RECT::default();
            let visible = unsafe {
                GetWindowRect(list, &mut r);
                IsWindowVisible(list) != 0
            };
            visible && self.under_list(list, under) && pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
        })
    }

    /// 按下空白鍵（「按空白鍵預覽」）：按住期間滑鼠停在哪個項目就預覽哪個；滑鼠已經在項目上就馬上顯示。
    pub(super) fn open_space_gate(&mut self) {
        super::crashlog::crumb("按空白鍵預覽：按下");
        self.hover.gate = true;
        self.hover.suppressed = false;
        // 先收起名稱提示，否則它蓋在滑鼠底下，預覽會以為滑鼠不在清單上
        self.sync_label_tips();
        unsafe {
            SetTimer(self.hwnd, GATE_TIMER, GATE_POLL_MS, std::ptr::null());
        }
        let mut pt = POINT::default();
        unsafe {
            GetCursorPos(&mut pt);
        }
        let Some(side) = self.list_under_cursor(pt) else {
            super::crashlog::crumb("按空白鍵預覽：滑鼠不在清單上");
            return;
        };
        let list = self.lists[side.index()];
        let mut client = pt;
        unsafe {
            ScreenToClient(list, &mut client);
        }
        let row = self.hover_row(side, client);
        if row >= 0 {
            self.hover.item = Some((side, row));
            self.on_hover_timer();
        } else {
            super::crashlog::crumb("按空白鍵預覽：滑鼠不在項目上");
        }
    }

    /// 放開空白鍵（「按空白鍵預覽」）：關閉預覽。
    pub(super) fn close_space_gate(&mut self) {
        if !self.hover.gate {
            return;
        }
        super::crashlog::crumb("按空白鍵預覽：放開");
        self.hover.gate = false;
        unsafe {
            KillTimer(self.hwnd, GATE_TIMER);
        }
        self.stop_hover();
    }

    pub(super) fn on_gate_timer(&mut self) {
        if !self.hover.gate || !preview::space_held() {
            self.close_space_gate();
            unsafe {
                KillTimer(self.hwnd, GATE_TIMER);
            }
        }
    }

    pub(super) fn on_list_mouse_leave(&mut self, side: Side) {
        self.hover.tracking[side.index()] = false;
        self.hover.last = None;
        if self.hover.item.is_some_and(|(s, _)| s == side) || preview::mode(&self.preview) == Some(preview::Mode::Hover) {
            self.reset_hover();
        }
    }

    /// 滑鼠停夠久了：確認還停在同一個項目上、沒有被其他視窗（例如選單）擋住，就顯示預覽。
    pub(super) fn on_hover_timer(&mut self) {
        unsafe {
            KillTimer(self.hwnd, HOVER_TIMER);
        }
        self.hover.armed = false;
        let Some((side, row)) = self.hover.item else { return };
        if self.hover.suppressed || !self.hover_allowed() {
            return;
        }
        // 置中的預覽（空白鍵、右鍵選單）開著時不換成懸停
        if preview::mode(&self.preview).is_some_and(|m| m != preview::Mode::Hover) {
            return;
        }
        let list = self.lists[side.index()];
        unsafe {
            if GetForegroundWindow() != self.hwnd
                || GetAsyncKeyState(VK_LBUTTON) < 0
                || GetAsyncKeyState(VK_RBUTTON) < 0
                || GetAsyncKeyState(VK_MBUTTON) < 0
                || send(list, LVM_GETEDITCONTROL, 0, 0) != 0
            {
                super::crashlog::crumb("懸停：略過（Stillfeather 不在前景、按著滑鼠或正在重新命名）");
                return;
            }
        }
        let mut pt = POINT::default();
        unsafe {
            GetCursorPos(&mut pt);
            let under = WindowFromPoint(pt);
            if !self.under_list(list, under) {
                super::crashlog::crumb("懸停：略過（滑鼠底下是別的視窗）");
                return;
            }
        }
        let mut client = pt;
        unsafe {
            ScreenToClient(list, &mut client);
        }
        if self.hover_row(side, client) != row {
            super::crashlog::crumb("懸停：略過（滑鼠已不在原本那一列）");
            return;
        }
        let Some(target) = self.target_at(side, row) else { return };
        if preview::mode(&self.preview) == Some(preview::Mode::Hover) && preview::target(&self.preview) == Some(&target) {
            return;
        }
        super::crashlog::mark("懸停：預覽");
        let opts = self.preview_opts();
        let area = self.quicklook_area();
        let owner = self.hwnd;
        preview::open(&mut self.preview, owner, target, area, pt, opts, preview::Mode::Hover);
        self.sync_label_tips();
    }

    /// 右鍵選單的「快速預覽」：開著就關閉，否則開啟並維持開著。
    pub(super) fn toggle_quicklook(&mut self, side: Side) {
        if preview::is_open(&self.preview) {
            self.hide_preview();
        } else {
            self.open_quicklook(side, false);
        }
    }

    /// Stillfeather 視窗移動或改變大小時，預覽跟著置中（懸停的預覽直接關掉）。
    pub(super) fn reposition_quicklook(&mut self) {
        if preview::mode(&self.preview) == Some(preview::Mode::Hover) {
            self.reset_hover();
            return;
        }
        let area = self.quicklook_area();
        preview::reposition(&mut self.preview, area);
    }

    /// 預覽視窗的「開啟」：資料夾在目前分頁開啟，檔案用預設程式開啟。
    pub(super) fn open_path(&mut self, path: &str, is_dir: bool) {
        if is_dir {
            let id = self.active_tab_id();
            self.navigate(id, path, NavKind::Normal, vec![]);
        } else if let Err(e) = super::fileops::shell_open(self.hwnd, path) {
            self.set_footer(trf!("無法開啟：{e}", "Couldn't open: {e}"));
        }
    }
}

impl App {
    /// 欄位標題：與卡片同色、小字次要色，排序欄附上箭頭；不畫系統的灰底與分隔線。
    unsafe fn header_custom_draw(&self, side: Side, header: HWND, cd: &NMCUSTOMDRAW) -> LRESULT {
        let p = &self.theme.pal;
        let f = &self.theme.fonts;
        match cd.dwDrawStage {
            CDDS_PREPAINT => {
                theme::fill(cd.hdc, &client_rect(header), p.surface);
                CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT
            }
            CDDS_ITEMPREPAINT => {
                let col = cd.dwItemSpec;
                let r = cd.rc;
                theme::fill(cd.hdc, &r, p.surface);
                if r.width() > self.s(8) {
                    let (sorted, desc) = self.bound[side.index()]
                        .and_then(|id| self.tabs.get(&id))
                        .map(|t| (t.sort_col == col && !t.searching, t.desc))
                        .unwrap_or((false, false));
                    let title = COLUMNS.get(col).map(column_title).unwrap_or("");
                    let right_align = COLUMNS.get(col).is_some_and(|c| c.3 == LVCFMT_RIGHT);
                    let mut tr = RECT { left: r.left + self.s(12), right: r.right - self.s(10), ..r };
                    if sorted {
                        let g = if desc { glyph::SORT_DOWN } else { glyph::SORT_UP };
                        let gw = self.s(14);
                        let gr = if right_align {
                            let gr = RECT { left: tr.left, right: tr.left + gw, ..tr };
                            tr.left += gw;
                            gr
                        } else {
                            let tw = theme::text_width(cd.hdc, title, f.small_bold).min(tr.width() - gw);
                            RECT { left: tr.left + tw + self.s(4), right: tr.left + tw + self.s(4) + gw, ..tr }
                        };
                        theme::draw_text(cd.hdc, &g.to_string(), &gr, f.icon_small, p.accent, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    }
                    let align = if right_align { DT_RIGHT } else { DT_LEFT };
                    let color = if sorted { p.ink } else { p.muted };
                    theme::draw_text(cd.hdc, title, &tr, f.small_bold, color, align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                    // 很淡的欄位分隔，方便找到可拖曳調整寬度的位置
                    let sep = RECT { left: r.right - 1, top: r.top + r.height() / 3, right: r.right, bottom: r.bottom - r.height() / 3 };
                    theme::fill(cd.hdc, &sep, theme::blend(p.ink, p.surface, 0.08));
                }
                CDRF_SKIPDEFAULT
            }
            CDDS_POSTPAINT => {
                // 最後一欄右側的空白與底線
                let rc = client_rect(header);
                let count = send(header, HDM_GETITEMCOUNT, 0, 0);
                let mut last = RECT::default();
                if count > 0 && send(header, HDM_GETITEMRECT, (count - 1) as usize, &mut last as *mut RECT as LPARAM) != 0 && last.right < rc.right {
                    theme::fill(cd.hdc, &RECT { left: last.right, ..rc }, p.surface);
                }
                let line = RECT { left: rc.left + self.s(6), top: rc.bottom - 1, right: rc.right - self.s(6), bottom: rc.bottom };
                theme::fill(cd.hdc, &line, theme::blend(p.ink, p.surface, 0.08));
                CDRF_DODEFAULT
            }
            _ => CDRF_DODEFAULT,
        }
    }
}

/// 滑鼠停在項目上多久才預覽；預覽開著時移到別的項目，等這麼久就換過去
pub const HOVER_TIMER: usize = 8;
const HOVER_DELAY_MS: u32 = 450;
const HOVER_SWITCH_MS: u32 = 150;
/// 「按空白鍵預覽」按住期間，定期確認空白鍵還按著（放開的訊息被輸入法吃掉時也能關閉）
pub const GATE_TIMER: usize = 9;
const GATE_POLL_MS: u32 = 60;

/// 自動重新整理前的清單狀態（見 keep_view／restore_view）
pub(super) struct KeptView {
    /// 選取項目的完整路徑（小寫）
    selected: std::collections::HashSet<String>,
    /// 有焦點的項目（小寫路徑）與它原本的列號
    focused: Option<String>,
    focused_index: isize,
    /// 最上面一列的列號（詳細資料與清單檢視）
    top: isize,
    /// 滑鼠下的項目（小寫路徑）
    hovered: Option<String>,
}

/// 懸停預覽的狀態
#[derive(Default)]
pub(super) struct Hover {
    /// 滑鼠下的項目（哪一欄、第幾列）
    item: Option<(Side, i32)>,
    /// 正在計時
    armed: bool,
    /// 點擊或按鍵後：滑鼠移到別的項目之前不再預覽
    suppressed: bool,
    /// 上次的滑鼠位置（略過系統補發、位置沒變的移動訊息）
    last: Option<(Side, i32, i32)>,
    /// 已向系統要求滑鼠離開的通知
    tracking: [bool; 2],
    /// 「按空白鍵預覽」：空白鍵正按著（這段期間才懸停預覽）
    gate: bool,
    /// 清單目前是否開著「截斷名稱的完整名稱提示」（LVS_EX_LABELTIP）；None 表示還沒設定過
    label_tips: Option<bool>,
}

unsafe extern "system" fn list_subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, sub_id: usize, _data: usize) -> LRESULT {
    let side = Side::from_index(sub_id);
    if let Some(a) = app::app_opt() {
        match msg {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                a.cancel_hover();
                a.activate(side);
            }
            WM_XBUTTONDOWN => a.cancel_hover(),
            WM_MOUSEMOVE => a.on_list_mouse_move(side, lparam),
            WM_MOUSELEAVE => a.on_list_mouse_leave(side),
            WM_SETFOCUS => a.activate(side),
            WM_NOTIFY => {
                // 欄位標題文字顏色跟隨主題（深色時預設是黑字）。
                let hdr = &*(lparam as *const NMHDR);
                if hdr.code == NM_CUSTOMDRAW && hdr.hwndFrom == header_of(hwnd) {
                    return a.header_custom_draw(side, hdr.hwndFrom, &*(lparam as *const NMCUSTOMDRAW));
                }
            }
            _ => {}
        }
    }
    (comctl().def_subclass_proc)(hwnd, msg, wparam, lparam)
}

/// 右鍵選單的「在這裡開啟 PowerShell 視窗」（和檔案總管的同名項目相同；資料夾樹的選單也用）
pub(super) fn powershell_text() -> &'static str {
    tr!("在這裡開啟 PowerShell 視窗", "Open PowerShell window here")
}

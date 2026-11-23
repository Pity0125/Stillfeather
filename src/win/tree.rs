//! 左側樹狀導覽：常用位置與磁碟。展開時才在背景讀取子資料夾，收合後釋放；
//! 磁碟清單依 Windows 裝置通知更新，沒有閒置輪詢。

use super::app::{self, id, App, NavKind, Node, NodeKind, WorkerMsg, WM_APP_TREE_RESET};
use super::anim;
use super::shellmenu::{self, Cmd, Part, Pick};

/// 側欄的 Windows 右鍵選單中藏起來的項目：Stillfeather 已有的命令，以及在側欄無法使用的「重新命名」。
const TREE_HIDDEN_VERBS: &[&str] = &["open", "explore", "opennewwindow", "opennewprocess", "opennewtab", "rename", "properties", "copyaspath", "cut", "copy", "delete"];
use super::clipboard;
use super::fileops;
use super::fs::{self, FolderListing, FsError};
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::paths;
use feather_core::typeahead;
use feather_core::watch::{self, Step};
use std::sync::atomic::AtomicBool;

pub(super) const FAVORITES_KEY: &str = "::favorites";
const DRIVES_KEY: &str = "::drives";

impl App {
    pub(super) fn create_tree(&mut self) -> HWND {
        let class = wide("SysTreeView32");
        let name = wide(tr!("資料夾樹狀導覽", "Folder tree"));
        let tree = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                name.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_TRACKSELECT | TVS_FULLROWSELECT | TVS_NOHSCROLL,
                0,
                0,
                10,
                10,
                self.hwnd,
                id::TREE as HMENU,
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        let ex = TVS_EX_DOUBLEBUFFER;
        send(tree, TVM_SETEXTENDEDSTYLE, ex, ex as LPARAM);
        send(tree, TVM_SETIMAGELIST, TVSIL_NORMAL, self.icons.tree);
        unsafe {
            // 拖放改由 OLE 拖放目標處理（dragdrop.rs）：放進資料夾或釘選成常用位置
            (comctl().set_window_subclass)(tree, tree_subclass, 0, 0);
        }
        tree
    }

    pub(super) fn style_tree(&self) {
        let p = self.theme.pal;
        // 樹狀圖底色跟著側欄卡片。
        theme::apply_explorer_theme(self.tree, theme::is_dark_color(p.sidebar));
        send(self.tree, TVM_SETBKCOLOR, 0, p.sidebar as LPARAM);
        send(self.tree, TVM_SETTEXTCOLOR, 0, p.sidebar_ink as LPARAM);
        send(self.tree, TVM_SETLINECOLOR, 0, p.sidebar_muted as LPARAM);
        send(self.tree, TVM_SETITEMHEIGHT, self.s(36) as usize, 0);
        invalidate(self.tree);
    }

    fn insert_node(&mut self, parent: HTREEITEM, text: &str, image: i32, children: bool, bold: bool, node: Node) -> HTREEITEM {
        self.insert_node_after(parent, TVI_LAST, text, image, children, bold, node)
    }

    /// 插在 `after` 之後（TVI_FIRST 表示最前面）。
    #[allow(clippy::too_many_arguments)]
    fn insert_node_after(&mut self, parent: HTREEITEM, after: HTREEITEM, text: &str, image: i32, children: bool, bold: bool, node: Node) -> HTREEITEM {
        let mut t = wide(text);
        let ins = TVINSERTSTRUCTW {
            hParent: parent,
            hInsertAfter: after,
            item: TVITEMW {
                mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE | TVIF_CHILDREN | TVIF_STATE,
                hItem: 0,
                state: if bold { TVIS_BOLD } else { 0 },
                stateMask: TVIS_BOLD,
                pszText: t.as_mut_ptr(),
                cchTextMax: 0,
                iImage: image,
                iSelectedImage: image,
                cChildren: children as i32,
                lParam: 0,
            },
            _ex: [0; 24],
        };
        let item = send_ptr(self.tree, TVM_INSERTITEMW, 0, &ins);
        if item != 0 {
            self.nodes.insert(item, node);
        }
        item
    }

    fn info_node(&mut self, parent: HTREEITEM, text: &str) {
        let blank = self.icons.tree_blank;
        self.insert_node(parent, text, blank, false, false, Node { path: String::new(), kind: NodeKind::Info, loaded: true, token: 0 });
    }

    fn root_by_key(&self, key: &str) -> Option<HTREEITEM> {
        self.nodes.iter().find(|(_, n)| n.kind == NodeKind::Header && n.path == key).map(|(h, _)| *h)
    }

    fn children_of(&self, item: HTREEITEM) -> Vec<HTREEITEM> {
        let mut out = Vec::new();
        let mut child = send(self.tree, TVM_GETNEXTITEM, TVGN_CHILD, item);
        while child != 0 {
            out.push(child);
            child = send(self.tree, TVM_GETNEXTITEM, TVGN_NEXT, child);
        }
        out
    }

    fn delete_children(&mut self, item: HTREEITEM) {
        for c in self.children_of(item) {
            send(self.tree, TVM_DELETEITEM, 0, c);
            self.forget_subtree(c);
        }
    }

    /// TVN_DELETEITEM 也會移除，這裡保險再清一次（被刪除的節點不再使用）。
    fn forget_subtree(&mut self, item: HTREEITEM) {
        self.nodes.remove(&item);
    }

    fn set_has_children(&self, item: HTREEITEM, has: bool) {
        let tv = TVITEMW {
            mask: TVIF_CHILDREN | TVIF_HANDLE,
            hItem: item,
            state: 0,
            stateMask: 0,
            pszText: std::ptr::null_mut(),
            cchTextMax: 0,
            iImage: 0,
            iSelectedImage: 0,
            cChildren: has as i32,
            lParam: 0,
        };
        send_ptr(self.tree, TVM_SETITEMW, 0, &tv);
    }

    /// 重建整棵樹：常用位置（含自訂捷徑）與磁碟。
    pub(super) fn build_tree(&mut self) {
        send(self.tree, TVM_DELETEITEM, 0, TVI_ROOT);
        self.nodes.clear();
        self.tree_fx.clear();
        self.tree_top = 0;
        let folder = self.icons.tree_blank;
        let fav = self.insert_node(TVI_ROOT, tr!("常用位置", "Favorites"), folder, true, true, Node {
            path: FAVORITES_KEY.into(),
            kind: NodeKind::Header,
            loaded: true,
            token: 0,
        });
        let known = [
            (tr!("使用者", "User"), Some(fs::profile_dir())),
            (tr!("桌面", "Desktop"), fs::known_folder(&fs::FOLDERID_DESKTOP)),
            (tr!("文件", "Documents"), fs::known_folder(&fs::FOLDERID_DOCUMENTS)),
            (tr!("下載", "Downloads"), fs::known_folder(&fs::FOLDERID_DOWNLOADS)),
            (tr!("圖片", "Pictures"), fs::known_folder(&fs::FOLDERID_PICTURES)),
        ];
        for (label, path) in known {
            if let Some(p) = path {
                self.insert_node(fav, label, folder, true, false, Node { path: p, kind: NodeKind::Favorite { custom: false }, loaded: false, token: 0 });
            }
        }
        for p in self.settings.favorites.clone() {
            let label = paths::tab_title(&p);
            self.insert_node(fav, &label, folder, true, false, Node { path: p, kind: NodeKind::Favorite { custom: true }, loaded: false, token: 0 });
        }
        let drives = self.insert_node(TVI_ROOT, tr!("磁碟", "Drives"), folder, true, true, Node {
            path: DRIVES_KEY.into(),
            kind: NodeKind::Header,
            loaded: true,
            token: 0,
        });
        self.fill_drives(drives);
        send(self.tree, TVM_EXPAND, TVE_EXPAND, fav);
        send(self.tree, TVM_EXPAND, TVE_EXPAND, drives);
    }

    fn fill_drives(&mut self, root: HTREEITEM) {
        for (drive, _kind) in fs::drives() {
            let image = self.icons.tree_blank;
            self.insert_node(root, &drive, image, true, false, Node { path: drive.clone(), kind: NodeKind::Drive, loaded: false, token: 0 });
        }
    }

    /// 插拔 USB 等裝置後更新磁碟清單。
    pub(super) fn refresh_drives(&mut self) {
        let Some(root) = self.root_by_key(DRIVES_KEY) else { return };
        let current: Vec<String> = fs::drives().into_iter().map(|d| d.0).collect();
        let shown: Vec<String> = self.children_of(root).iter().filter_map(|c| self.nodes.get(c).map(|n| n.path.clone())).collect();
        if current == shown {
            return;
        }
        self.delete_children(root);
        self.fill_drives(root);
        send(self.tree, TVM_EXPAND, TVE_EXPAND, root);
    }

    pub(super) fn tree_notify(&mut self, code: u32, lparam: LPARAM) -> LRESULT {
        unsafe {
            match code {
                NM_CUSTOMDRAW => self.tree_custom_draw(lparam),
                TVN_ITEMEXPANDINGW => {
                    let nm = &*(lparam as *const NMTREEVIEWW);
                    if nm.action as usize & TVE_EXPAND != 0 {
                        self.expand_node(nm.itemNew.hItem);
                    }
                    0
                }
                TVN_ITEMEXPANDEDW => {
                    let nm = &*(lparam as *const NMTREEVIEWW);
                    let item = nm.itemNew.hItem;
                    if nm.action as usize & TVE_COLLAPSE != 0 && self.nodes.get(&item).is_some_and(|n| n.kind != NodeKind::Header && n.loaded) {
                        PostMessageW(self.hwnd, WM_APP_TREE_RESET, 0, item);
                    }
                    0
                }
                TVN_DELETEITEMW => {
                    let nm = &*(lparam as *const NMTREEVIEWW);
                    self.nodes.remove(&nm.itemOld.hItem);
                    self.tree_fx.forget(nm.itemOld.hItem);
                    0
                }
                NM_CLICK => {
                    let mut p = cursor_pos();
                    ScreenToClient(self.tree, &mut p);
                    let mut ht = TVHITTESTINFO { pt: p, flags: 0, hItem: 0 };
                    let item = send(self.tree, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM);
                    if item != 0 && ht.flags & (TVHT_ONITEMICON | TVHT_ONITEMLABEL | TVHT_ONITEMRIGHT) != 0 {
                        self.tree_navigate(item);
                    }
                    0
                }
                TVN_KEYDOWN => {
                    let nm = &*(lparam as *const NMTVKEYDOWN);
                    if nm.wVKey as u32 == VK_RETURN {
                        self.tree_navigate_selected();
                    }
                    0
                }
                _ => 0,
            }
        }
    }

    /// 參考圖的側欄：選取項目是浮起的白色膠囊，懸停是淡色膠囊（皆以短動畫淡入淡出）；
    /// 「常用位置」「磁碟」這類標題用小字次要色，像區段標籤。
    unsafe fn tree_custom_draw(&mut self, lparam: LPARAM) -> LRESULT {
        let cd = &mut *(lparam as *mut NMTVCUSTOMDRAW);
        match cd.nmcd.dwDrawStage {
            CDDS_PREPAINT => {
                let top = send(self.tree, TVM_GETNEXTITEM, TVGN_FIRSTVISIBLE, 0) as HTREEITEM;
                if top != self.tree_top {
                    self.tree_top = top;
                    self.tree_fx.reset_seen();
                }
                CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT
            }
            CDDS_ITEMPREPAINT => {
                // 每一列都由 Stillfeather 自己畫（底色、膠囊、展開箭頭、圖示、文字）。
                // 樹狀圖在「整列選取＋檔案總管主題」下會在預設繪製時蓋上自己的整列底色，
                // 先畫的膠囊與圖示會被蓋掉，所以這裡完全取代預設繪製（CDRF_SKIPDEFAULT）。
                let item = cd.nmcd.dwItemSpec as HTREEITEM;
                let hot = cd.nmcd.uItemState & CDIS_HOT != 0;
                self.draw_tree_row(cd.nmcd.hdc, item, cd.nmcd.rc, hot);
                CDRF_SKIPDEFAULT
            }
            CDDS_POSTPAINT => {
                let busy = self.tree_fx.animating(anim::now());
                let rects: Vec<RECT> = busy
                    .into_iter()
                    .filter_map(|item| {
                        // TVM_GETITEMRECT：RECT 開頭先放項目代碼
                        let mut r = RECT::default();
                        (&mut r as *mut RECT as *mut HTREEITEM).write_unaligned(item);
                        (send(self.tree, TVM_GETITEMRECT, 0, &mut r as *mut RECT as LPARAM) != 0).then_some(r)
                    })
                    .collect();
                anim::request_rects(self.tree, rects);
                CDRF_DODEFAULT
            }
            _ => CDRF_DODEFAULT,
        }
    }

    fn expand_node(&mut self, item: HTREEITEM) {
        let show_hidden = self.show_hidden;
        self.node_token += 1;
        let token = self.node_token;
        let Some(node) = self.nodes.get_mut(&item) else { return };
        if node.kind == NodeKind::Header || node.loaded {
            return;
        }
        node.loaded = true;
        node.token = token;
        let path = node.path.clone();
        self.info_node(item, tr!("讀取中…", "Loading…"));
        std::thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let result = fs::child_folders(&path, show_hidden, &cancel);
            app::post(WorkerMsg::Tree { item, token, result });
        });
    }

    pub(super) fn on_tree_children(&mut self, item: HTREEITEM, token: u64, result: Result<FolderListing, FsError>) {
        if !self.nodes.get(&item).is_some_and(|n| n.token == token && n.loaded) {
            return;
        }
        self.delete_children(item);
        let folder = self.icons.tree_blank;
        match result {
            Ok(listing) => {
                if listing.folders.is_empty() {
                    self.set_has_children(item, false);
                }
                for f in listing.folders {
                    self.insert_node(item, &f.name, folder, !f.is_link, false, Node { path: f.path, kind: NodeKind::Folder, loaded: false, token: 0 });
                }
                if listing.truncated {
                    self.info_node(item, &trf!("僅顯示前 {} 個資料夾", "Only the first {} folders are shown", feather_core::format::group_digits(fs::TREE_MAX_CHILDREN as u64)));
                }
            }
            Err(FsError::Cancelled) => {}
            Err(FsError::Message(e)) => self.info_node(item, &trf!("無法讀取：{e}", "Couldn't read: {e}")),
        }
    }

    /// 清單重新讀取了某個資料夾（自動重新整理或 Stillfeather 自己的檔案作業之後）：
    /// 側欄裡已展開的同一個資料夾跟著加入新的子資料夾、移除不見的。
    /// 其他子資料夾的節點原封不動，已經展開的不會被收合；還在讀取、讀取失敗或只顯示部分時不動。
    /// `folders`：資料夾裡的子資料夾名稱與是否為連結（與側欄相同，已依顯示隱藏項目的設定過濾）。
    pub(super) fn sync_tree_folder(&mut self, path: &str, folders: &[(String, bool)]) {
        let targets: Vec<(HTREEITEM, String)> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.loaded && !matches!(n.kind, NodeKind::Header | NodeKind::Info) && paths::same_path(&n.path, path))
            .map(|(h, n)| (*h, n.path.clone()))
            .collect();
        if targets.is_empty() || folders.len() > fs::TREE_MAX_CHILDREN {
            return;
        }
        let mut sorted: Vec<&(String, bool)> = folders.iter().collect();
        sorted.sort_by(|a, b| feather_core::sorting::natural_cmp(&a.0, &b.0));
        let new: Vec<&str> = sorted.iter().map(|f| f.0.as_str()).collect();
        let image = self.icons.tree_blank;
        for (item, node_path) in targets {
            let children = self.children_of(item);
            if children.iter().any(|c| self.nodes.get(c).is_none_or(|n| n.kind != NodeKind::Folder)) {
                continue;
            }
            let names: Vec<String> = children.iter().map(|c| paths::file_name(&self.nodes[c].path).to_string()).collect();
            let old: Vec<&str> = names.iter().map(String::as_str).collect();
            let plan = watch::plan_children(&old, &new);
            if plan.is_noop() {
                continue;
            }
            for &i in &plan.remove {
                send(self.tree, TVM_DELETEITEM, 0, children[i]);
                self.forget_subtree(children[i]);
            }
            let mut after = TVI_FIRST;
            for step in &plan.steps {
                match *step {
                    Step::Keep(i) => after = children[i],
                    Step::Insert(j) => {
                        let (name, is_link) = sorted[j];
                        let node = Node { path: paths::join(&node_path, name), kind: NodeKind::Folder, loaded: false, token: 0 };
                        let h = self.insert_node_after(item, after, name, image, !*is_link, false, node);
                        if h != 0 {
                            after = h;
                        }
                    }
                }
            }
            self.set_has_children(item, !new.is_empty());
        }
    }

    /// 收合後釋放子節點，下次展開再重新讀取。
    pub(super) fn reset_tree_node(&mut self, item: HTREEITEM) {
        if !self.nodes.get(&item).is_some_and(|n| n.kind != NodeKind::Header && n.loaded) {
            return;
        }
        if let Some(n) = self.nodes.get_mut(&item) {
            n.loaded = false;
            n.token = 0;
        }
        send(self.tree, TVM_EXPAND, TVE_COLLAPSE | TVE_COLLAPSERESET, item);
        self.delete_children(item);
        self.set_has_children(item, true);
    }

    fn node_path(&self, item: HTREEITEM) -> Option<String> {
        self.nodes.get(&item).filter(|n| !matches!(n.kind, NodeKind::Header | NodeKind::Info)).map(|n| n.path.clone())
    }

    fn tree_navigate(&mut self, item: HTREEITEM) {
        match self.nodes.get(&item).map(|n| n.kind) {
            Some(NodeKind::Header) => {
                send(self.tree, TVM_EXPAND, 3 /* TVE_TOGGLE */, item);
            }
            Some(NodeKind::Info) | None => {}
            Some(_) => {
                send(self.tree, TVM_SELECTITEM, TVGN_CARET, item);
                if let Some(p) = self.node_path(item) {
                    let id = self.active_tab_id();
                    self.navigate(id, &p, NavKind::Normal, vec![]);
                }
            }
        }
    }

    /// 畫側欄的一列。版面沿用樹狀圖自己的計算（縮排、圖示位置、文字起點），
    /// 所以點選、展開箭頭的感應範圍與畫出來的位置一致。
    unsafe fn draw_tree_row(&mut self, hdc: HDC, item: HTREEITEM, row: RECT, hot: bool) {
        let p = self.theme.pal;
        let f = &self.theme.fonts;
        theme::fill(hdc, &row, p.sidebar);
        let kind = self.nodes.get(&item).map(|n| n.kind);
        let state = send(self.tree, TVM_GETITEMSTATE, item as usize, (TVIS_SELECTED | TVIS_EXPANDED | TVIS_DROPHILITED) as LPARAM) as u32;
        let selected = state & TVIS_SELECTED != 0;
        let expanded = state & TVIS_EXPANDED != 0;
        let drop_target = state & TVIS_DROPHILITED != 0;
        let label = self.tree_item_text(item);

        if kind == Some(NodeKind::Header) {
            // 區段標題：小字標籤，後面一個小箭頭表示展開或收合（點標題可切換）
            let x = row.left + self.s(16);
            let tr = RECT { left: x, top: row.top + self.s(6), ..row };
            let pin_here = self.drop_pin && self.nodes.get(&item).is_some_and(|n| n.path == FAVORITES_KEY);
            if pin_here {
                // 拖曳資料夾到這裡：標題變成重點色並提示放開即可釘選
                let pill = RECT { left: row.left + self.s(4), top: row.top + self.s(4), right: row.right - self.s(4), bottom: row.bottom };
                theme::round_rect(hdc, &pill, self.s(12), Some(theme::blend(p.accent, p.sidebar, 0.14)), Some(p.accent));
                theme::draw_text(hdc, tr!("放開即可釘選到常用位置", "Release to pin to Favorites"), &tr, f.small_bold, p.accent, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                return;
            }
            theme::draw_text(hdc, &label, &tr, f.small_bold, p.sidebar_muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            let w = theme::text_width(hdc, &label, f.small_bold);
            let cr = RECT { left: x + w + self.s(4), right: x + w + self.s(20), ..tr };
            let g = if expanded { glyph::SORT_DOWN } else { glyph::CHEVRON_RIGHT };
            let c = if hot { p.sidebar_ink } else { theme::blend(p.sidebar_muted, p.sidebar, 0.75) };
            theme::draw_text(hdc, &g.to_string(), &cr, f.icon_small, c, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            return;
        }

        // 樹狀圖算好的文字位置（TVM_GETITEMRECT 只取文字範圍：RECT 開頭先放項目代碼）
        let mut tr = RECT::default();
        (&mut tr as *mut RECT as *mut HTREEITEM).write_unaligned(item);
        if send(self.tree, TVM_GETITEMRECT, 1, &mut tr as *mut RECT as LPARAM) == 0 {
            return;
        }
        let slot = self.icons.tree_slot;
        let icon_right = tr.left - self.s(3);
        let icon_top = row.top + (row.height() - slot) / 2;
        let icon = RECT { left: icon_right - slot, top: icon_top, right: icon_right, bottom: icon_top + slot };

        if kind == Some(NodeKind::Info) {
            let r = RECT { left: icon.left, right: row.right - self.s(8), ..row };
            theme::draw_text(hdc, &label, &r, f.small, p.sidebar_muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            return;
        }

        // 選取與懸停的膠囊（淡入淡出）
        let (sv, hv) = self.tree_fx.observe(item, selected, hot, anim::now(), anim::dur(anim::ROW_MS), anim::dur(anim::HOVER_MS));
        let fill = anim::mix(anim::mix(p.sidebar, p.sidebar_hover, hv), p.sidebar_selected, sv);
        let pill = RECT { left: row.left + self.s(4), top: row.top + self.s(2), right: row.right - self.s(4), bottom: row.bottom - self.s(2) };
        if drop_target {
            // 拖放到這個資料夾：重點色外框
            theme::round_rect(hdc, &pill, self.s(14), Some(theme::blend(p.accent, p.sidebar, 0.16)), Some(p.accent));
        } else if sv > 0.5 {
            theme::card(hdc, &pill, self.s(14), fill, None, &p, self.theme.dpi);
        } else if fill != p.sidebar {
            theme::round_rect(hdc, &pill, self.s(14), Some(fill), None);
        }

        // 展開箭頭：放在樹狀圖的按鈕區（圖示左邊一個縮排寬），點這裡就會展開或收合
        let mut it: TVITEMW = std::mem::zeroed();
        it.mask = TVIF_CHILDREN;
        it.hItem = item;
        send(self.tree, TVM_GETITEMW, 0, &mut it as *mut TVITEMW as LPARAM);
        if it.cChildren != 0 {
            let indent = (send(self.tree, TVM_GETINDENT, 0, 0) as i32).max(self.s(16));
            let br = RECT { left: icon.left - indent, top: row.top, right: icon.left, bottom: row.bottom };
            let g = if expanded { glyph::SORT_DOWN } else { glyph::CHEVRON_RIGHT };
            theme::draw_text(hdc, &g.to_string(), &br, f.icon_small, p.sidebar_muted, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }

        if let Some(place) = self.nodes.get(&item).and_then(place_of) {
            let badge = matches!(kind, Some(NodeKind::Favorite { .. }) | Some(NodeKind::Drive));
            self.draw_place_icon(hdc, &icon, place, badge, fill);
        }

        let text = RECT { left: tr.left + self.s(2), right: row.right - self.s(10), ..row };
        let font = if selected { f.ui_bold } else { f.ui };
        theme::draw_text(hdc, &label, &text, font, p.sidebar_ink, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    }

    /// 位置圖示：最上層（常用位置、磁碟）放在淡色圓角方塊裡，子資料夾只畫彩色圖形。
    unsafe fn draw_place_icon(&self, hdc: HDC, icon: &RECT, place: Place, badge: bool, background: COLORREF) {
        let p = &self.theme.pal;
        let icon = *icon;
        let (g, base) = place.style(p.accent);
        // 深色模式稍微提亮，在深灰側欄上才清楚
        let color = if p.dark { theme::blend(rgb(255, 255, 255), base, 0.2) } else { base };
        if badge {
            let tint = theme::blend(color, background, if p.dark { 0.26 } else { 0.18 });
            theme::round_rect(hdc, &icon, self.s(7), Some(tint), None);
            theme::draw_text(hdc, &g.to_string(), &icon, self.theme.fonts.icon_small, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            theme::draw_text(hdc, &g.to_string(), &icon, self.theme.fonts.icon, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }

    fn tree_item_text(&self, item: HTREEITEM) -> String {
        let mut buf = [0u16; 260];
        let mut it: TVITEMW = unsafe { std::mem::zeroed() };
        it.mask = TVIF_TEXT;
        it.hItem = item;
        it.pszText = buf.as_mut_ptr();
        it.cchTextMax = buf.len() as i32;
        send(self.tree, TVM_GETITEMW, 0, &mut it as *mut TVITEMW as LPARAM);
        from_wide(&buf)
    }

    /// 打字跳到側欄中名稱開頭相符的項目（只看目前展開可見的項目，略過區段標題）。
    pub(super) fn tree_type_ahead(&mut self, query: &str) {
        let mut items = Vec::new();
        let mut h = send(self.tree, TVM_GETNEXTITEM, TVGN_ROOT, 0) as HTREEITEM;
        while h != 0 && items.len() < 20_000 {
            if !matches!(self.nodes.get(&h).map(|n| n.kind), Some(NodeKind::Header) | Some(NodeKind::Info)) {
                items.push(h);
            }
            h = send(self.tree, TVM_GETNEXTITEM, TVGN_NEXTVISIBLE, h) as HTREEITEM;
        }
        let sel = send(self.tree, TVM_GETNEXTITEM, TVGN_CARET, 0) as HTREEITEM;
        let current = items.iter().position(|x| *x == sel);
        let found = typeahead::find(items.len(), current, query, |i| Some(self.tree_item_text(items[i])));
        if let Some(i) = found {
            send(self.tree, TVM_SELECTITEM, TVGN_CARET, items[i]);
            send(self.tree, TVM_ENSUREVISIBLE, 0, items[i]);
        }
    }

    /// 側欄目前選取（游標所在）的資料夾路徑；標題列與說明列沒有路徑。
    pub(super) fn tree_caret_path(&self) -> Option<String> {
        let item = send(self.tree, TVM_GETNEXTITEM, TVGN_CARET, 0);
        (item != 0).then(|| self.node_path(item)).flatten()
    }

    pub(super) fn tree_navigate_selected(&mut self) {
        let item = send(self.tree, TVM_GETNEXTITEM, TVGN_CARET, 0);
        if item != 0 {
            self.tree_navigate(item);
        }
    }

    pub(super) fn tree_context_menu(&mut self, pt: POINT, keyboard: bool) {
        self.hide_preview();
        let item = if keyboard {
            send(self.tree, TVM_GETNEXTITEM, TVGN_CARET, 0)
        } else {
            let mut p = pt;
            unsafe {
                ScreenToClient(self.tree, &mut p);
            }
            let mut ht = TVHITTESTINFO { pt: p, flags: 0, hItem: 0 };
            send(self.tree, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM)
        };
        let Some(path) = (item != 0).then(|| self.node_path(item)).flatten() else { return };
        let pt = if keyboard {
            let r = client_rect(self.tree);
            let mut p = POINT { x: r.left + self.s(40), y: r.top + self.s(20) };
            unsafe {
                ClientToScreen(self.tree, &mut p);
            }
            p
        } else {
            pt
        };
        let kind = self.nodes.get(&item).map(|n| n.kind);
        // 內建的常用位置（桌面、文件、下載…）一直都在，不需要釘選
        let builtin = matches!(kind, Some(NodeKind::Favorite { custom: false }));
        let pinned = self.is_pinned(&path);
        let is_root = paths::parent(&path).is_none();
        // 版面同清單的右鍵選單：開啟／編輯／整理，接著是 Windows 與已安裝程式的項目，最後是內容
        let open = vec![Cmd::new(id::OPEN, tr!("開啟", "Open"), glyph::OPEN), Cmd::new(id::OPEN_NEW_TAB, tr!("在新分頁開啟", "Open in new tab"), glyph::OPEN_NEW_TAB)];
        let edit = vec![
            Cmd::new(id::CUT, tr!("剪下", "Cut"), glyph::CUT).enabled(!is_root),
            Cmd::new(id::COPY, tr!("複製", "Copy"), glyph::COPY),
            Cmd::new(id::DELETE, tr!("刪除", "Delete"), glyph::DELETE).enabled(!is_root),
        ];
        let mut organize = Vec::new();
        if pinned {
            organize.push(Cmd::new(id::REMOVE_FAVORITE, tr!("從常用位置取消釘選", "Unpin from Favorites"), glyph::UNPIN));
        } else if !builtin {
            organize.push(Cmd::new(id::ADD_FAVORITE, tr!("釘選到常用位置", "Pin to Favorites"), glyph::PIN));
        }
        organize.push(Cmd::new(id::COPY_PATH, tr!("複製路徑", "Copy path"), glyph::LINK));
        let last = vec![Cmd::new(id::PROPERTIES, tr!("內容", "Properties"), glyph::PROPERTIES)];
        let shell = shellmenu::ShellMenu::for_items(self.hwnd, std::slice::from_ref(&path));
        let parts = [Part::Ours(open), Part::Ours(edit), Part::Ours(organize), Part::Shell, Part::Ours(last)];
        let look = self.menu_look();
        let picked = shellmenu::popup(self.hwnd, &parts, shell.as_ref(), true, TREE_HIDDEN_VERBS, Some(id::OPEN), pt, &look);
        let cmd = match picked {
            None => return,
            Some(Pick::Shell(cmd)) => {
                if let Some(sm) = &shell {
                    if let Err(e) = sm.invoke(self.hwnd, cmd, &path, pt) {
                        self.show_error(&e);
                    }
                }
                self.after_shell_command();
                return;
            }
            Some(Pick::Ours(cmd)) => cmd,
        };
        match cmd {
            id::OPEN => {
                let tab = self.active_tab_id();
                self.navigate(tab, &path, NavKind::Normal, vec![]);
            }
            id::OPEN_NEW_TAB => {
                let side = self.active_side();
                self.new_tab(side, &path);
            }
            id::ADD_FAVORITE => self.add_favorites(vec![path]),
            id::REMOVE_FAVORITE => self.remove_favorite(&path),
            id::CUT | id::COPY => self.clipboard_paths(vec![path], cmd == id::CUT),
            id::DELETE => {
                if self.delete_paths(vec![path.clone()]) {
                    self.pending_tree_delete = Some((item, path));
                }
            }
            id::COPY_PATH => {
                if let Err(e) = clipboard::set_text(self.hwnd, &path) {
                    self.show_error(&e);
                }
            }
            id::PROPERTIES => fileops::show_properties(self.hwnd, &path),
            _ => {}
        }
    }
}

unsafe extern "system" fn tree_subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if let Some(a) = app::app_opt() {
        match msg {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MOUSEWHEEL => a.hide_preview(),
            _ => {}
        }
    }
    (comctl().def_subclass_proc)(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------- 位置圖示

/// 側欄項目的圖示種類。常用資料夾（桌面、文件、下載…）不論出現在哪一層都用自己的圖示。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Place {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    Cloud,
    Folder,
    SystemDrive,
    Drive,
    Removable,
    NetworkDrive,
    Disc,
}

impl Place {
    /// 圖形與顏色（避開綠色，與篩選籤的色點同一系列）。
    fn style(self, accent: COLORREF) -> (char, COLORREF) {
        use theme::glyph;
        match self {
            Place::Home => (glyph::HOME, accent),
            Place::Desktop => (glyph::MONITOR, rgb(84, 136, 208)),
            Place::Documents => (glyph::DOCUMENT, rgb(104, 118, 196)),
            Place::Downloads => (glyph::DOWNLOAD, rgb(46, 146, 190)),
            Place::Pictures => (glyph::PHOTO, rgb(222, 104, 112)),
            Place::Music => (glyph::MUSIC, rgb(204, 96, 156)),
            Place::Videos => (glyph::VIDEO, rgb(140, 108, 206)),
            Place::Cloud => (glyph::CLOUD, rgb(56, 128, 220)),
            Place::Folder => (glyph::FOLDER_FILL, rgb(218, 160, 48)),
            Place::SystemDrive => (glyph::HARD_DRIVE, accent),
            Place::Drive => (glyph::HARD_DRIVE, rgb(112, 122, 140)),
            Place::Removable => (glyph::USB, rgb(212, 136, 56)),
            Place::NetworkDrive => (glyph::MAP_DRIVE, rgb(70, 128, 200)),
            Place::Disc => (glyph::HARD_DRIVE, rgb(150, 110, 190)),
        }
    }
}

fn known_places() -> &'static [(String, Place)] {
    static KNOWN: std::sync::OnceLock<Vec<(String, Place)>> = std::sync::OnceLock::new();
    KNOWN.get_or_init(|| {
        let mut v = vec![(fs::profile_dir(), Place::Home)];
        for (id, place) in [
            (&fs::FOLDERID_DESKTOP, Place::Desktop),
            (&fs::FOLDERID_DOCUMENTS, Place::Documents),
            (&fs::FOLDERID_DOWNLOADS, Place::Downloads),
            (&fs::FOLDERID_PICTURES, Place::Pictures),
            (&fs::FOLDERID_MUSIC, Place::Music),
            (&fs::FOLDERID_VIDEOS, Place::Videos),
            (&fs::FOLDERID_SKYDRIVE, Place::Cloud),
        ] {
            if let Some(path) = fs::known_folder(id) {
                v.push((path, place));
            }
        }
        v
    })
}

fn place_of(node: &Node) -> Option<Place> {
    match node.kind {
        NodeKind::Header | NodeKind::Info => None,
        NodeKind::Drive => {
            let w = wide(&node.path);
            let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
            let system = std::env::var("SystemDrive").is_ok_and(|d| !d.is_empty() && node.path.to_ascii_uppercase().starts_with(&d.to_ascii_uppercase()));
            Some(match kind {
                DRIVE_REMOVABLE => Place::Removable,
                DRIVE_REMOTE => Place::NetworkDrive,
                DRIVE_CDROM => Place::Disc,
                _ if system => Place::SystemDrive,
                _ => Place::Drive,
            })
        }
        NodeKind::Favorite { .. } | NodeKind::Folder => {
            Some(known_places().iter().find(|(p, _)| paths::same_path(p, &node.path)).map(|(_, place)| *place).unwrap_or(Place::Folder))
        }
    }
}

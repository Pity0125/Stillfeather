//! 左側樹狀導覽：大標籤（常用位置、使用者自己新增的標籤、磁碟）與底下的資料夾。展開時才在背景讀取子資料夾，
//! 收合後釋放；磁碟清單依 Windows 裝置通知更新，沒有閒置輪詢。
//!
//! 點大標籤的標題收起或展開（記在設定檔），平常不顯示展開箭頭，收起來時標題後面有「›」；
//! 按住標題拖曳可以調整順序（順序見 feather_core::sidebar）；
//! 在標題或側欄的空白處按右鍵可以新增標籤，自訂標籤可以改名、刪除。把資料夾拖到標籤上就放進那個標籤。

use super::app::{self, id, App, NavKind, Node, NodeKind, WorkerMsg, WM_APP_TREE_RESET};
use super::anim;
use super::shellmenu::{self, Cmd, Part, Pick};

/// 側欄的 Windows 右鍵選單中藏起來的項目：Stillfeather 已有的命令，以及在側欄無法使用的「重新命名」。
const TREE_HIDDEN_VERBS: &[&str] = &["open", "explore", "opennewwindow", "opennewprocess", "opennewtab", "rename", "properties", "copyaspath", "cut", "copy", "delete", "powershell"];
use super::clipboard;
use super::fileops;
use super::fs::{self, FolderListing, FsError};
use super::sys::*;
use super::theme::{self, glyph};
use super::util::*;
use feather_core::paths;
use feather_core::sidebar::{self, Section};
use feather_core::typeahead;
use feather_core::watch::{self, Step};
use std::sync::atomic::AtomicBool;

pub(super) const FAVORITES_KEY: &str = "::favorites";
const BOOKMARK_MOVE: u16 = 450;
const DRIVES_KEY: &str = "::drives";
/// 自訂標籤標題節點的路徑：後面接 `BookmarkGroups` 的索引
pub(super) const GROUP_PREFIX: &str = "::bookmark-group/";

/// 側欄大標籤的拖曳排序：按住標題、移動超過拖曳距離後開始，放開時移到最近的縫
pub(super) struct SectionDrag {
    item: HTREEITEM,
    /// 第幾個大標籤（由上到下）
    from: usize,
    start: POINT,
    active: bool,
    /// 要放進第幾個縫（0 是最上面）
    slot: Option<usize>,
}

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
        // 列高至少容得下內容文字（文字調大時跟著加高）
        let item_h = self.s(36).max(self.theme.fonts.body_height + self.s(16));
        send(self.tree, TVM_SETITEMHEIGHT, item_h as usize, 0);
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

    fn insert_header(&mut self, text: &str, key: String) -> HTREEITEM {
        let blank = self.icons.tree_blank;
        self.insert_node(TVI_ROOT, text, blank, true, true, Node { path: key, kind: NodeKind::Header, loaded: true, token: 0 })
    }

    /// 重建整棵樹：照設定的順序放大標籤——常用位置（內建的資料夾與沒放進標籤的釘選）、自訂標籤、磁碟。
    pub(super) fn build_tree(&mut self) {
        send(self.tree, TVM_DELETEITEM, 0, TVI_ROOT);
        self.nodes.clear();
        self.tree_fx.clear();
        self.tree_top = 0;
        // 拖曳標籤到一半側欄重建了（例如另一個視窗改了設定）：取消拖曳、放開滑鼠
        self.cancel_section_drag();
        let folder = self.icons.tree_blank;
        let favorites = self.settings.favorites.clone();
        let groups = self.settings.bookmark_groups.clone();
        let in_group = |p: &str| groups.iter().position(|(_, members)| members.iter().any(|m| paths::eq_ignore_case(m, p)));
        for section in self.settings.sidebar_sections() {
            let header = match section {
                Section::Favorites => {
                    let fav = self.insert_header(tr!("常用位置", "Favorites"), FAVORITES_KEY.into());
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
                    for p in favorites.iter().filter(|p| in_group(p).is_none()) {
                        let label = paths::tab_title(p);
                        self.insert_node(fav, &label, folder, true, false, Node { path: p.clone(), kind: NodeKind::Favorite { custom: true }, loaded: false, token: 0 });
                    }
                    fav
                }
                Section::Group(i) => {
                    let Some((name, _)) = groups.get(i) else { continue };
                    let node = self.insert_header(name, format!("{GROUP_PREFIX}{i}"));
                    let mut any = false;
                    for p in favorites.iter().filter(|p| in_group(p) == Some(i)) {
                        let label = paths::tab_title(p);
                        self.insert_node(node, &label, folder, true, false, Node { path: p.clone(), kind: NodeKind::Favorite { custom: true }, loaded: false, token: 0 });
                        any = true;
                    }
                    if !any {
                        // 空的標籤：提示怎麼放資料夾進來（拖放到這一行也算放進這個標籤）
                        self.info_node(node, tr!("把資料夾拖到這裡", "Drag folders here"));
                    }
                    node
                }
                Section::Drives => {
                    let drives = self.insert_header(tr!("磁碟", "Drives"), DRIVES_KEY.into());
                    self.fill_drives(drives);
                    drives
                }
            };
            if !self.settings.is_collapsed(section) {
                send(self.tree, TVM_EXPAND, TVE_EXPAND, header);
            }
        }
    }

    /// 點大標籤的標題（或在標題上按 Enter）：收起或展開
    fn toggle_section(&mut self, item: HTREEITEM) {
        let expanded = send(self.tree, TVM_GETITEMSTATE, item as usize, TVIS_EXPANDED as LPARAM) as u32 & TVIS_EXPANDED != 0;
        send(self.tree, TVM_EXPAND, if expanded { TVE_COLLAPSE } else { TVE_EXPAND }, item);
        self.remember_section_state(item);
    }

    /// 記下大標籤現在是收起來還是展開（存不進設定檔時就算了，下次開啟照舊展開）
    fn remember_section_state(&mut self, item: HTREEITEM) {
        let Some(section) = self.section_of(item) else { return };
        let expanded = send(self.tree, TVM_GETITEMSTATE, item as usize, TVIS_EXPANDED as LPARAM) as u32 & TVIS_EXPANDED != 0;
        let mut settings = self.settings.clone();
        if settings.set_collapsed(section, !expanded) && app::write_settings(&settings).is_ok() {
            self.settings = settings;
        }
        invalidate(self.tree);
    }

    /// 標題節點是哪一個大標籤
    pub(super) fn section_of(&self, item: HTREEITEM) -> Option<Section> {
        let n = self.nodes.get(&item).filter(|n| n.kind == NodeKind::Header)?;
        match n.path.as_str() {
            FAVORITES_KEY => Some(Section::Favorites),
            DRIVES_KEY => Some(Section::Drives),
            p => p.strip_prefix(GROUP_PREFIX)?.parse().ok().map(Section::Group),
        }
    }

    /// 拖放到這一列就放進哪個自訂標籤：標籤的標題，或空標籤裡的提示。回傳標籤名稱與標題節點
    pub(super) fn group_drop_target(&self, item: HTREEITEM) -> Option<(String, HTREEITEM)> {
        let header = match self.nodes.get(&item)?.kind {
            NodeKind::Header => item,
            NodeKind::Info => send(self.tree, TVM_GETNEXTITEM, TVGN_PARENT, item) as HTREEITEM,
            _ => return None,
        };
        match self.section_of(header)? {
            Section::Group(i) => self.settings.bookmark_groups.get(i).map(|(name, _)| (name.clone(), header)),
            _ => None,
        }
    }

    /// 由上到下的大標籤標題
    fn section_headers(&self) -> Vec<HTREEITEM> {
        let mut out = Vec::new();
        let mut h = send(self.tree, TVM_GETNEXTITEM, TVGN_ROOT, 0) as HTREEITEM;
        while h != 0 {
            out.push(h);
            h = send(self.tree, TVM_GETNEXTITEM, TVGN_NEXT, h) as HTREEITEM;
        }
        out
    }

    /// 項目的整列範圍（樹狀圖用戶區座標）
    fn tree_row_rect(&self, item: HTREEITEM) -> Option<RECT> {
        let mut r = RECT::default();
        unsafe {
            (&mut r as *mut RECT as *mut HTREEITEM).write_unaligned(item);
        }
        (send(self.tree, TVM_GETITEMRECT, 0, &mut r as *mut RECT as LPARAM) != 0).then_some(r)
    }

    /// 大標籤之間的縫的高度（第 k 個是第 k 個標題的上緣，最後一個是最後一列的下緣）
    fn section_slots(&self) -> Vec<i32> {
        let mut v: Vec<i32> = self.section_headers().into_iter().filter_map(|h| self.tree_row_rect(h).map(|r| r.top)).collect();
        let last = send(self.tree, TVM_GETNEXTITEM, TVGN_LASTVISIBLE, 0) as HTREEITEM;
        if let Some(r) = (last != 0).then(|| self.tree_row_rect(last)).flatten() {
            v.push(r.bottom);
        }
        v
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
                    // 用方向鍵收起、展開大標籤：記下來
                    if self.nodes.get(&item).is_some_and(|n| n.kind == NodeKind::Header) {
                        self.remember_section_state(item);
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
                // 拖曳大標籤：放開後會移到的縫畫一條重點色的線
                if let Some(y) = self.section_drop_y() {
                    let c = client_rect(self.tree);
                    let bar = RECT { left: c.left + self.s(10), top: y - self.s(1), right: c.right - self.s(10), bottom: y + self.s(2) };
                    theme::round_rect(cd.nmcd.hdc, &bar, self.s(3), Some(self.theme.pal.accent), None);
                }
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
            // 大標籤：收起或展開（按住可以拖曳調整順序）
            Some(NodeKind::Header) => self.toggle_section(item),
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
            // 大標籤的標題：小字標籤。點一下收起或展開，按住可以拖曳調整順序。平常不顯示展開箭頭：
            // 收起來時後面有「›」，展開時滑鼠移上來才淡淡顯示可以收起的箭頭
            let group = self.nodes.get(&item).is_some_and(|n| n.path.starts_with(GROUP_PREFIX));
            let tr = RECT { left: row.left + self.s(16), top: row.top + self.s(6), ..row };
            let pill = RECT { left: row.left + self.s(4), top: row.top + self.s(4), right: row.right - self.s(4), bottom: row.bottom };
            let pin_here = self.drop_pin && self.nodes.get(&item).is_some_and(|n| n.path == FAVORITES_KEY);
            if pin_here || (group && drop_target) {
                // 拖曳資料夾到這裡：標題變成重點色並提示放開即可放進來
                theme::round_rect(hdc, &pill, self.s(12), Some(theme::blend(p.accent, p.sidebar, 0.14)), Some(p.accent));
                let hint = if pin_here { tr!("放開即可釘選到常用位置", "Release to pin to Favorites").to_string() } else { trf!("放開即可放進「{label}」", "Release to add to “{label}”") };
                theme::draw_text(hdc, &hint, &tr, f.body_small_bold, p.accent, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
                return;
            }
            let dragging = self.section_drag.as_ref().is_some_and(|d| d.active && d.item == item);
            if dragging {
                theme::round_rect(hdc, &pill, self.s(12), Some(theme::blend(p.accent, p.sidebar, 0.10)), Some(p.accent));
            }
            let color = if dragging { p.accent } else if hot { p.sidebar_ink } else { p.sidebar_muted };
            theme::draw_text(hdc, &label, &tr, f.body_small_bold, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
            if !dragging && (!expanded || hot) {
                let w = theme::text_width(hdc, &label, f.body_small_bold).min(tr.width());
                let cr = RECT { left: tr.left + w + self.s(4), right: tr.left + w + self.s(20), ..tr };
                let g = if expanded { glyph::SORT_DOWN } else { glyph::CHEVRON_RIGHT };
                let c = if expanded { theme::blend(p.sidebar_muted, p.sidebar, 0.75) } else { color };
                theme::draw_text(hdc, &g.to_string(), &cr, f.icon_small, c, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            }
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
            theme::draw_text(hdc, &label, &r, f.body_small, p.sidebar_muted, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
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
        let font = if selected { f.body_bold } else { f.body };
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

    /// 存下側欄標籤的變更並重建側欄；`message` 顯示在狀態列
    fn save_sections(&mut self, settings: feather_core::settings::Settings, message: String) {
        match app::write_settings(&settings) {
            Ok(()) => {
                self.settings = settings;
                self.build_tree();
                self.set_footer(message);
            }
            Err(error) => self.show_error(&error),
        }
    }

    /// 大標籤的選單（在標題上按右鍵，或在側欄的空白處）：新增標籤（放在這個標籤下面）；
    /// 自訂標籤可以改名、刪除；往上移、往下移（同拖曳標題）。
    pub(super) fn section_menu(&mut self, section: Option<Section>, pt: POINT) {
        const ADD: u16 = 1;
        const RENAME: u16 = 2;
        const REMOVE: u16 = 3;
        const UP: u16 = 4;
        const DOWN: u16 = 5;
        self.hide_preview();
        let sections = self.settings.sidebar_sections();
        let pos = section.and_then(|s| sections.iter().position(|x| *x == s));
        let mut items = vec![(ADD, tr!("新增標籤…", "New section…"), true)];
        if let Some(Section::Group(_)) = section {
            items.push((RENAME, tr!("重新命名標籤…", "Rename section…"), true));
            items.push((REMOVE, tr!("刪除標籤（資料夾移回常用位置）", "Delete section (folders go back to Favorites)"), true));
        }
        if let Some(p) = pos {
            items.push((0, "", true));
            items.push((UP, tr!("往上移", "Move up"), p > 0));
            items.push((DOWN, tr!("往下移", "Move down"), p + 1 < sections.len()));
        }
        let Some(cmd) = app::popup_menu(self.hwnd, &items, pt) else { return };
        match (cmd, section, pos) {
            (ADD, _, _) => self.new_section(section),
            (RENAME, Some(Section::Group(i)), _) => self.rename_section(i),
            (REMOVE, Some(Section::Group(i)), _) => self.remove_section(i),
            (UP, _, Some(p)) if p > 0 => self.move_section(p, p - 1),
            (DOWN, _, Some(p)) => self.move_section(p, p + 2),
            _ => {}
        }
    }

    /// 問標籤名稱；名稱不能用時說明原因再問一次（保留打好的字）。取消時回傳 None。
    fn ask_section_name(&mut self, title: &str, current: &str, check: impl Fn(&feather_core::settings::Settings, &str) -> Option<String>) -> Option<String> {
        let mut text = current.to_string();
        loop {
            let name = super::dialog::text_input(self.hwnd, title, tr!("標籤名稱", "Section name"), &text, tr!("例如：工作、專案、照片", "For example: Work, Projects, Photos"))?;
            match check(&self.settings, &name) {
                None => return Some(name.trim().to_string()),
                Some(e) => {
                    self.show_error(&e);
                    text = name;
                }
            }
        }
    }

    fn group_names(&self) -> Vec<&str> {
        self.settings.bookmark_groups.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// 新增標籤：放在 `after` 那個標籤下面（從空白處新增時放在磁碟前面）
    fn new_section(&mut self, after: Option<Section>) {
        if self.settings.bookmark_groups.len() >= sidebar::MAX_GROUPS {
            self.show_error(&trf!("側欄最多 {} 個自訂標籤。", "Up to {} custom sections are allowed.", sidebar::MAX_GROUPS));
            return;
        }
        let Some(name) = self.ask_section_name(tr!("新增標籤", "New section"), "", |s, n| sidebar::name_error(n, &s.bookmark_groups.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), None)) else { return };
        let mut settings = self.settings.clone();
        match settings.add_group(&name, after) {
            Ok(_) => self.save_sections(settings, trf!("已新增標籤「{name}」：把資料夾拖到標籤上就能放進去", "Added the section “{name}”. Drag folders onto it to add them")),
            Err(e) => self.show_error(&e),
        }
    }

    fn rename_section(&mut self, index: usize) {
        let Some(current) = self.group_names().get(index).map(|s| s.to_string()) else { return };
        let Some(name) = self.ask_section_name(tr!("重新命名標籤", "Rename section"), &current, move |s, n| {
            sidebar::name_error(n, &s.bookmark_groups.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), Some(index))
        }) else { return };
        if name == current {
            return;
        }
        let mut settings = self.settings.clone();
        match settings.rename_group(index, &name) {
            Ok(()) => self.save_sections(settings, trf!("已把標籤「{current}」改名為「{name}」", "Renamed the section “{current}” to “{name}”")),
            Err(e) => self.show_error(&e),
        }
    }

    fn remove_section(&mut self, index: usize) {
        let Some(name) = self.group_names().get(index).map(|s| s.to_string()) else { return };
        let mut settings = self.settings.clone();
        settings.remove_group(index);
        self.save_sections(settings, trf!("已刪除標籤「{name}」，裡面的資料夾移回常用位置（資料夾本身沒有刪除）", "Deleted the section “{name}”; its folders went back to Favorites (the folders themselves weren't deleted)"));
    }

    /// 第 `from` 個大標籤移到第 `slot` 個縫（0 是最上面）
    fn move_section(&mut self, from: usize, slot: usize) {
        let sections = self.settings.sidebar_sections();
        let Some(moved) = sidebar::move_to_slot(&sections, from, slot) else { return };
        let header = self.section_headers().get(from).map(|h| self.tree_item_text(*h)).unwrap_or_default();
        let mut settings = self.settings.clone();
        settings.set_sidebar_sections(&moved);
        self.save_sections(settings, trf!("已移動「{header}」", "Moved “{header}”"));
    }

    // ---------------------------------------------------------------- 拖曳大標籤

    /// 這個位置（樹狀圖用戶區座標）在大標籤的標題上
    fn section_at(&self, lparam: LPARAM) -> bool {
        let mut ht = TVHITTESTINFO { pt: POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) }, flags: 0, hItem: 0 };
        let item = send(self.tree, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM);
        item != 0 && self.section_of(item).is_some()
    }

    /// 在大標籤的標題上按下左鍵：準備拖曳排序，沒有拖曳就是收起或展開（由這裡處理時回傳 true，樹狀圖不再處理這次按下）
    fn section_press(&mut self, lparam: LPARAM) -> bool {
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        let mut ht = TVHITTESTINFO { pt, flags: 0, hItem: 0 };
        let item = send(self.tree, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM);
        if item == 0 || self.section_of(item).is_none() {
            return false;
        }
        let Some(from) = self.section_headers().iter().position(|h| *h == item) else { return false };
        unsafe {
            SetFocus(self.tree);
            SetCapture(self.tree);
        }
        self.section_drag = Some(SectionDrag { item, from, start: pt, active: false, slot: None });
        true
    }

    fn section_move(&mut self, lparam: LPARAM) -> bool {
        let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
        let Some(d) = self.section_drag.as_ref() else { return false };
        if !d.active {
            let (dx, dy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if (pt.x - d.start.x).abs() <= dx && (pt.y - d.start.y).abs() <= dy {
                return true;
            }
        }
        // 拖到側欄上下緣：捲動
        let c = client_rect(self.tree);
        if pt.y < c.top + self.s(16) {
            send(self.tree, WM_VSCROLL, SB_LINEUP, 0);
        } else if pt.y > c.bottom - self.s(16) {
            send(self.tree, WM_VSCROLL, SB_LINEDOWN, 0);
        }
        let slots = self.section_slots();
        let slot = slots.iter().enumerate().min_by_key(|(_, y)| (**y - pt.y).abs()).map(|(i, _)| i);
        let Some(d) = self.section_drag.as_mut() else { return true };
        if !d.active || d.slot != slot {
            d.active = true;
            d.slot = slot;
            invalidate(self.tree);
        }
        true
    }

    fn section_release(&mut self) -> bool {
        let Some(d) = self.section_drag.take() else { return false };
        unsafe {
            if GetCapture() == self.tree {
                ReleaseCapture();
            }
        }
        invalidate(self.tree);
        match (d.active, d.slot) {
            (true, Some(slot)) => self.move_section(d.from, slot),
            // 沒有拖曳，只是點一下：收起或展開
            (false, _) => self.toggle_section(d.item),
            _ => {}
        }
        true
    }

    /// 滑鼠被搶走、按了 Esc：取消拖曳，標籤留在原位。有在拖曳時回傳 true
    pub(super) fn cancel_section_drag(&mut self) -> bool {
        let Some(d) = self.section_drag.take() else { return false };
        unsafe {
            if GetCapture() == self.tree {
                ReleaseCapture();
            }
        }
        invalidate(self.tree);
        d.active
    }

    /// 拖曳大標籤時，放開後會移到的縫的高度（放回原處時沒有）
    fn section_drop_y(&self) -> Option<i32> {
        let d = self.section_drag.as_ref().filter(|d| d.active)?;
        let slot = d.slot.filter(|s| *s != d.from && *s != d.from + 1)?;
        let c = client_rect(self.tree);
        self.section_slots().get(slot).map(|y| (*y).clamp(c.top + self.s(2), c.bottom - self.s(2)))
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
        // 側欄的空白處、大標籤的標題、空標籤裡的提示：標籤的選單（新增、改名、刪除、移動）
        let Some(node) = self.nodes.get(&item) else {
            self.section_menu(None, pt);
            return;
        };
        if node.kind == NodeKind::Info || node.kind == NodeKind::Header {
            let header = if node.kind == NodeKind::Header { item } else { send(self.tree, TVM_GETNEXTITEM, TVGN_PARENT, item) };
            let section = self.section_of(header);
            if node.kind == NodeKind::Header || section.is_some_and(|s| matches!(s, Section::Group(_))) {
                self.section_menu(section, pt);
            }
            return;
        }
        let Some(path) = self.node_path(item) else { return };
        let kind = self.nodes.get(&item).map(|n| n.kind);
        // 內建的常用位置（桌面、文件、下載…）一直都在，不需要釘選
        let builtin = matches!(kind, Some(NodeKind::Favorite { custom: false }));
        let pinned = self.is_pinned(&path);
        let is_root = paths::parent(&path).is_none();
        // 版面同清單的右鍵選單：開啟／編輯／整理，接著是 Windows 與已安裝程式的項目，最後是內容
        let open = vec![
            Cmd::new(id::OPEN, tr!("開啟", "Open"), glyph::OPEN),
            Cmd::new(id::OPEN_NEW_TAB, tr!("在新分頁開啟", "Open in new tab"), glyph::OPEN_NEW_TAB),
            Cmd::new(id::OPEN_POWERSHELL, super::pane::powershell_text(), glyph::TERMINAL),
        ];
        let edit = vec![
            Cmd::new(id::CUT, tr!("剪下", "Cut"), glyph::CUT).enabled(!is_root),
            Cmd::new(id::COPY, tr!("複製", "Copy"), glyph::COPY),
            Cmd::new(id::DELETE, tr!("刪除", "Delete"), glyph::DELETE).enabled(!is_root),
        ];
        let mut organize = Vec::new();
        if pinned {
            if !self.settings.bookmark_groups.is_empty() {
                organize.push(Cmd::new(BOOKMARK_MOVE, tr!("移到其他標籤…", "Move to another section…"), glyph::FOLDER));
            }
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
            BOOKMARK_MOVE => {
                let current = self.settings.group_of(&path);
                let names: Vec<String> = self.group_names().iter().map(|s| s.to_string()).collect();
                let mut choices = vec![super::menu::Item::radio(1, tr!("常用位置", "Favorites"), current.is_none())];
                for (i, name) in names.iter().enumerate() {
                    choices.push(super::menu::Item::radio((i + 2) as u16, name, current == Some(i)));
                }
                if let Some(choice) = super::menu::popup_at(self.hwnd, pt, &choices) {
                    let group = (choice >= 2).then(|| choice as usize - 2);
                    if group != current {
                        let mut settings = self.settings.clone();
                        settings.move_bookmark(&path, group);
                        let target = group.and_then(|g| names.get(g).cloned()).unwrap_or_else(|| tr!("常用位置", "Favorites").into());
                        let title = paths::tab_title(&path);
                        self.save_sections(settings, trf!("已把「{title}」移到「{target}」", "Moved “{title}” to “{target}”"));
                    }
                }
            }
            id::OPEN => {
                let tab = self.active_tab_id();
                self.navigate(tab, &path, NavKind::Normal, vec![]);
            }
            id::OPEN_NEW_TAB => {
                let side = self.active_side();
                self.new_tab(side, &path);
            }
            id::OPEN_POWERSHELL => self.open_powershell(&path),
            id::ADD_FAVORITE => self.add_favorites(vec![path]),
            id::REMOVE_FAVORITE => self.remove_favorite(&path),
            id::CUT | id::COPY => self.clipboard_paths(vec![path], cmd == id::CUT),
            // 按住 Shift 點「刪除」：永久刪除
            id::DELETE => {
                if self.delete_paths(vec![path.clone()], key_down(VK_SHIFT)) {
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
        // 大標籤的標題：按住拖曳調整順序（點一下不收合）
        match msg {
            WM_LBUTTONDOWN if a.section_press(lparam) => return 0,
            // 連點兩下的第二下：已經收起又展開過一次，不再交給樹狀圖切換
            WM_LBUTTONDBLCLK if a.section_at(lparam) => return 0,
            WM_MOUSEMOVE if a.section_move(lparam) => return 0,
            WM_LBUTTONUP if a.section_release() => return 0,
            WM_CAPTURECHANGED => {
                a.cancel_section_drag();
            }
            _ => {}
        }
        // 中鍵點資料夾：在新分頁開啟
        if msg == WM_MBUTTONUP {
            let mut ht = TVHITTESTINFO { pt: POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) }, flags: 0, hItem: 0 };
            let item = send(hwnd, TVM_HITTEST, 0, &mut ht as *mut TVHITTESTINFO as LPARAM);
            if item != 0 && ht.flags & (TVHT_ONITEMICON | TVHT_ONITEMLABEL | TVHT_ONITEMRIGHT) != 0 {
                if let Some(path) = a.node_path(item) {
                    a.open_in_background_tab(a.active_side(), &path);
                }
            }
            return 0;
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

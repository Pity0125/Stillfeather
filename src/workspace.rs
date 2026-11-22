//! 左右兩組分頁與雙欄狀態的純模型（不含任何視窗）。
//!
//! 規則與 C# 版相同：
//! - 左右欄各自有分頁；上方導覽、搜尋與工具列作用於「作用中欄」的選取分頁。
//! - 切回單欄會保留右欄分頁；再開雙欄時繼續使用。
//! - 關閉右欄最後一頁會收起並清空右欄。
//! - 雙欄時關閉左欄最後一頁，右欄分頁移到左側並回到單欄。
//! - 單欄時關閉最後一頁會關閉主視窗。

pub type TabId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left = 0,
    Right = 1,
}

impl Side {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

    pub fn from_index(i: usize) -> Side {
        if i == 1 {
            Side::Right
        } else {
            Side::Left
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub tabs: Vec<TabId>,
    pub selected: usize,
}

impl Group {
    pub fn current(&self) -> Option<TabId> {
        self.tabs.get(self.selected).copied()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CloseResult {
    /// 應關閉主視窗。
    CloseWindow,
    /// 已移除這些分頁（呼叫端應取消其背景工作並釋放資料）。
    Closed(Vec<TabId>),
    /// 索引無效，未變更。
    Nothing,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DualChange {
    Unchanged,
    /// 已開啟雙欄；若右欄原本沒有分頁，會建立這個新分頁，呼叫端應以左欄路徑初始化。
    Shown { new_right_tab: Option<TabId> },
    Hidden,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub groups: [Group; 2],
    pub dual: bool,
    pub active: Side,
    next_id: TabId,
}

impl Workspace {
    /// 建立工作區，左欄含一個分頁。
    pub fn new() -> (Workspace, TabId) {
        let mut ws = Workspace { groups: [Group::default(), Group::default()], dual: false, active: Side::Left, next_id: 1 };
        let id = ws.alloc();
        ws.groups[0].tabs.push(id);
        (ws, id)
    }

    fn alloc(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn group(&self, side: Side) -> &Group {
        &self.groups[side.index()]
    }

    /// 目前可見的欄位（單欄時右欄不算）。
    pub fn visible_sides(&self) -> &'static [Side] {
        if self.dual {
            &[Side::Left, Side::Right]
        } else {
            &[Side::Left]
        }
    }

    pub fn active_tab(&self) -> TabId {
        self.groups[self.active.index()].current().or_else(|| self.groups[0].current()).unwrap_or(0)
    }

    pub fn current(&self, side: Side) -> Option<TabId> {
        self.groups[side.index()].current()
    }

    /// 與 C# TabCount 相同：左欄數量，加上雙欄時的右欄數量。
    pub fn tab_count(&self) -> usize {
        self.groups[0].tabs.len() + if self.dual { self.groups[1].tabs.len() } else { 0 }
    }

    pub fn all_tabs(&self) -> impl Iterator<Item = TabId> + '_ {
        self.groups[0].tabs.iter().chain(self.groups[1].tabs.iter()).copied()
    }

    pub fn side_of(&self, id: TabId) -> Option<Side> {
        if self.groups[0].tabs.contains(&id) {
            Some(Side::Left)
        } else if self.groups[1].tabs.contains(&id) {
            Some(Side::Right)
        } else {
            None
        }
    }

    /// 在指定欄位尾端新增分頁並選取、設為作用中。
    pub fn add_tab(&mut self, side: Side) -> TabId {
        let id = self.alloc();
        let g = &mut self.groups[side.index()];
        g.tabs.push(id);
        g.selected = g.tabs.len() - 1;
        self.active = side;
        id
    }

    /// 在作用中欄位新增分頁（單欄時一律為左欄）。
    pub fn add_tab_active(&mut self) -> TabId {
        let side = if self.dual { self.active } else { Side::Left };
        self.add_tab(side)
    }

    pub fn select(&mut self, side: Side, index: usize) -> bool {
        let g = &mut self.groups[side.index()];
        if index >= g.tabs.len() {
            return false;
        }
        g.selected = index;
        self.active = side;
        true
    }

    /// 讓指定分頁成為作用中；若在隱藏的右欄則回傳 false（呼叫端需先開啟雙欄）。
    pub fn activate_tab(&mut self, id: TabId) -> bool {
        for side in [Side::Left, Side::Right] {
            if let Some(i) = self.groups[side.index()].tabs.iter().position(|t| *t == id) {
                if side == Side::Right && !self.dual {
                    return false;
                }
                self.groups[side.index()].selected = i;
                self.active = side;
                return true;
            }
        }
        false
    }

    pub fn switch(&mut self, delta: isize) {
        let g = &mut self.groups[self.active.index()];
        let n = g.tabs.len() as isize;
        if n > 0 {
            g.selected = ((g.selected as isize + delta).rem_euclid(n)) as usize;
        }
    }

    /// 在同一欄內調整順序；回傳是否有移動。
    pub fn move_tab(&mut self, side: Side, index: usize, delta: isize) -> bool {
        let g = &mut self.groups[side.index()];
        let target = index as isize + delta;
        if index >= g.tabs.len() || target < 0 || target as usize >= g.tabs.len() {
            return false;
        }
        let target = target as usize;
        let selected_id = g.current();
        g.tabs.swap(index, target);
        if let Some(sel) = selected_id {
            g.selected = g.tabs.iter().position(|t| *t == sel).unwrap_or(0);
        }
        true
    }

    pub fn set_dual(&mut self, enabled: bool) -> DualChange {
        if enabled == self.dual {
            return DualChange::Unchanged;
        }
        self.dual = enabled;
        if enabled {
            let mut new_right_tab = None;
            if self.groups[1].tabs.is_empty() {
                let id = self.alloc();
                self.groups[1].tabs.push(id);
                self.groups[1].selected = 0;
                new_right_tab = Some(id);
            }
            DualChange::Shown { new_right_tab }
        } else {
            self.active = Side::Left;
            DualChange::Hidden
        }
    }

    pub fn close(&mut self, side: Side, index: usize) -> CloseResult {
        let len = self.groups[side.index()].tabs.len();
        if index >= len {
            return CloseResult::Nothing;
        }
        if len > 1 {
            let g = &mut self.groups[side.index()];
            let id = g.tabs.remove(index);
            if index < g.selected || g.selected >= g.tabs.len() {
                g.selected = g.selected.saturating_sub(1);
            }
            return CloseResult::Closed(vec![id]);
        }
        match side {
            Side::Right => {
                let removed = std::mem::take(&mut self.groups[1].tabs);
                self.groups[1].selected = 0;
                self.dual = false;
                self.active = Side::Left;
                CloseResult::Closed(removed)
            }
            Side::Left => {
                if self.dual && !self.groups[1].tabs.is_empty() {
                    let removed = std::mem::take(&mut self.groups[0].tabs);
                    self.groups[0] = std::mem::take(&mut self.groups[1]);
                    self.dual = false;
                    self.active = Side::Left;
                    CloseResult::Closed(removed)
                } else {
                    CloseResult::CloseWindow
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_switch_close() {
        let (mut ws, first) = Workspace::new();
        let second = ws.add_tab_active();
        let third = ws.add_tab_active();
        assert_eq!(ws.active_tab(), third);
        assert_eq!(ws.tab_count(), 3);
        ws.switch(1);
        assert_eq!(ws.active_tab(), first);
        ws.switch(-1);
        assert_eq!(ws.active_tab(), third);
        // 關閉中間分頁，選取保持在原本的分頁
        assert_eq!(ws.close(Side::Left, 1), CloseResult::Closed(vec![second]));
        assert_eq!(ws.active_tab(), third);
        // 關閉選取的最後一頁，選取往前移
        assert_eq!(ws.close(Side::Left, 1), CloseResult::Closed(vec![third]));
        assert_eq!(ws.active_tab(), first);
        // 單欄最後一頁 → 關閉視窗
        assert_eq!(ws.close(Side::Left, 0), CloseResult::CloseWindow);
    }

    #[test]
    fn dual_rules() {
        let (mut ws, left) = Workspace::new();
        let right = match ws.set_dual(true) {
            DualChange::Shown { new_right_tab: Some(id) } => id,
            other => panic!("{other:?}"),
        };
        assert_eq!(ws.current(Side::Right), Some(right));
        assert_eq!(ws.tab_count(), 2);
        ws.select(Side::Right, 0);
        let right2 = ws.add_tab_active();
        assert_eq!(ws.side_of(right2), Some(Side::Right));
        // 收起右欄保留分頁
        assert_eq!(ws.set_dual(false), DualChange::Hidden);
        assert_eq!(ws.active, Side::Left);
        assert_eq!(ws.tab_count(), 1);
        assert!(!ws.activate_tab(right2));
        assert_eq!(ws.set_dual(true), DualChange::Shown { new_right_tab: None });
        assert!(ws.activate_tab(right2));
        // 雙欄時關閉左欄最後一頁：右欄移到左側
        assert_eq!(ws.close(Side::Left, 0), CloseResult::Closed(vec![left]));
        assert!(!ws.dual);
        assert_eq!(ws.groups[0].tabs, vec![right, right2]);
        assert!(ws.groups[1].tabs.is_empty());
    }

    #[test]
    fn closing_last_right_tab_collapses() {
        let (mut ws, _) = Workspace::new();
        ws.set_dual(true);
        let r = ws.current(Side::Right).unwrap();
        assert_eq!(ws.close(Side::Right, 0), CloseResult::Closed(vec![r]));
        assert!(!ws.dual);
        assert_eq!(ws.active, Side::Left);
        assert!(ws.groups[1].tabs.is_empty());
        assert_eq!(ws.close(Side::Right, 0), CloseResult::Nothing);
    }

    #[test]
    fn reorder_keeps_selection() {
        let (mut ws, a) = Workspace::new();
        let b = ws.add_tab_active();
        let c = ws.add_tab_active();
        ws.select(Side::Left, 0);
        assert!(ws.move_tab(Side::Left, 0, 1));
        assert_eq!(ws.groups[0].tabs, vec![b, a, c]);
        assert_eq!(ws.active_tab(), a);
        assert!(!ws.move_tab(Side::Left, 2, 1));
        assert!(!ws.move_tab(Side::Left, 0, -1));
    }

    #[test]
    fn single_mode_adds_to_left() {
        let (mut ws, _) = Workspace::new();
        ws.set_dual(true);
        ws.select(Side::Right, 0);
        ws.set_dual(false);
        let id = ws.add_tab_active();
        assert_eq!(ws.side_of(id), Some(Side::Left));
    }
}

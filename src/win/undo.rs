//! 復原（Ctrl+Z）：重新命名、移動（含拖放與剪下貼上）、刪除到資源回收筒。
//!
//! 操作完成時記下「做了什麼」（feather_core::undo::Action）；按 Ctrl+Z 時先檢查檔案系統的現況，
//! 已經被別的程式改動、無法復原的紀錄直接略過，往前找下一個可以復原的。
//! 重新命名在這裡直接改回；移動與刪除交給背景執行緒（fileops::start_undo），完成後由 on_file_op 收尾。

use super::app::App;
use super::fileops;
use super::fs;
use feather_core::paths;
use feather_core::undo::{self, Action, FILETIME_SECOND};

/// 背景檔案操作開始時先記下的內容；完成後依實際結果寫入復原紀錄。
pub(super) enum Pending {
    /// （原本的位置, 移動後的位置）
    Move(Vec<(String, String)>),
    /// 要刪除的項目與開始時間
    Recycle { items: Vec<String>, since: u64 },
}

fn exists(path: &str) -> bool {
    fs::attributes(path).is_some()
}

impl App {
    /// 背景的移動或刪除即將開始：記下來源與目的地。
    pub(super) fn prepare_undo(&mut self, kind: feather_core::plan::OpKind, sources: &[String], dest: Option<&str>) {
        use feather_core::plan::OpKind;
        self.op_pending = match (kind, dest) {
            (OpKind::Move, Some(d)) => Some(Pending::Move(undo::move_pairs(sources, d))),
            (OpKind::Delete, _) => Some(Pending::Recycle { items: sources.to_vec(), since: undo::filetime_now().saturating_sub(2 * FILETIME_SECOND) }),
            _ => None,
        };
    }

    /// 背景操作完成：只記下真的完成的項目（被略過、取消或失敗的不算）。
    pub(super) fn finish_undo_record(&mut self) {
        match self.op_pending.take() {
            Some(Pending::Move(pairs)) => {
                let items = pairs.into_iter().filter(|(from, to)| exists(to) && !exists(from)).collect();
                self.history.push(Action::Move { items });
            }
            Some(Pending::Recycle { items, since }) => {
                let items = items.into_iter().filter(|p| !exists(p)).collect();
                self.history.push(Action::Recycle { items, since, until: undo::filetime_now() + 2 * FILETIME_SECOND });
            }
            None => {}
        }
    }

    /// 由 Windows 完成的移動（拖放到資料夾）：可能還在背景進行，按 Ctrl+Z 時才依現況判斷。
    pub(super) fn record_move(&mut self, sources: &[String], dest: &str) {
        self.history.push(Action::Move { items: undo::move_pairs(sources, dest) });
    }

    pub(super) fn record_rename(&mut self, from: &str, to: &str) {
        self.history.push(Action::Rename { from: from.to_string(), to: to.to_string() });
    }

    /// 選單上顯示的「復原…」文字；沒有可復原的操作時為 None。
    pub(super) fn undo_label(&self) -> Option<String> {
        self.history.last().map(|a| trf!("復原{}", "Undo {}", a.label()))
    }

    pub(super) fn undo(&mut self) {
        if self.op_busy {
            self.set_footer(tr!("請等目前的檔案操作完成再復原", "Wait for the current file operation to finish before undoing").into());
            return;
        }
        loop {
            let Some(action) = self.history.pop() else {
                self.set_footer(tr!("沒有可以復原的操作", "There's nothing to undo").into());
                return;
            };
            match action {
                Action::Rename { from, to } => {
                    // 只改大小寫時兩個路徑指向同一個項目
                    let case_only = paths::eq_ignore_case(&from, &to);
                    if !exists(&to) || (!case_only && exists(&from)) {
                        continue;
                    }
                    match fileops::rename(&to, &from) {
                        Ok(()) => {
                            self.refresh_after_undo(std::slice::from_ref(&from));
                            self.set_footer(trf!("已復原重新命名：「{}」改回「{}」", "Undid rename: “{}” is back to “{}”", paths::file_name(&to), paths::file_name(&from)));
                        }
                        Err(e) => self.show_error(&trf!("無法復原重新命名：{e}", "Couldn't undo the rename: {e}")),
                    }
                    return;
                }
                Action::Move { items } => {
                    let items: Vec<(String, String)> = items.into_iter().filter(|(from, to)| exists(to) && !exists(from)).collect();
                    if items.is_empty() {
                        continue;
                    }
                    self.undo_select = items.iter().map(|(from, _)| from.clone()).collect();
                    self.run_undo(Action::Move { items });
                    return;
                }
                Action::Recycle { items, since, until } => {
                    let items: Vec<String> = items.into_iter().filter(|p| !exists(p)).collect();
                    if items.is_empty() {
                        continue;
                    }
                    self.undo_select = items.clone();
                    self.run_undo(Action::Recycle { items, since, until });
                    return;
                }
            }
        }
    }

    fn run_undo(&mut self, action: Action) {
        self.op_busy = true;
        self.op_pending = None;
        self.set_footer(trf!("正在復原{}…", "Undoing {}…", action.label()));
        fileops::start_undo(action, self.hwnd);
    }
}

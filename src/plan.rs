//! 檔案作業（複製、移動、刪除）開始前的純路徑檢查，移植自 C# FileOperationService。
//! 需要讀檔案系統的檢查（是否存在、重新解析點、同名不同類型）由 Windows 層補做。

use crate::paths;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "複製",
            OpKind::Move => "移動",
            OpKind::Delete => "刪除",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    pub source: String,
    /// 複製／移動的目的完整路徑；刪除為 None。
    pub target: Option<String>,
    /// 複製到來源所在的同一資料夾：交由 Windows 自動改名（例如「- 複製」）。
    pub rename_on_collision: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejected {
    pub label: String,
    pub reason: String,
}

pub type PlanItem = Result<Planned, Rejected>;

/// 正規化：必須是完整路徑、不可是延伸／裝置格式、元件名稱必須合法；去掉結尾分隔符號。
pub fn normalize(path: &str) -> Result<String, String> {
    let p = paths::normalize_separators(path.trim());
    if p.is_empty() {
        return Err("路徑為空白。".into());
    }
    if paths::is_extended(&p) {
        return Err("目前的 Windows Shell 操作不支援裝置或延伸路徑格式。".into());
    }
    if !paths::is_fully_qualified(&p) {
        return Err("請使用完整磁碟或 UNC 路徑。".into());
    }
    if !paths::components_valid(&p) {
        return Err("路徑含有 Windows Shell 不支援或會自動正規化的名稱。".into());
    }
    Ok(paths::trim_end_sep(&p).to_string())
}

/// 檢查每個來源；保留原本順序，被拒絕的項目附上原因。
/// `protected` 為不可直接操作的系統或使用者設定檔根目錄。
pub fn plan(kind: OpKind, sources: &[String], destination: Option<&str>, protected: &[String]) -> Result<Vec<PlanItem>, String> {
    let dest = match kind {
        OpKind::Delete => None,
        _ => {
            let d = destination.ok_or("缺少目的地資料夾。")?;
            Some(normalize(d).map_err(|e| format!("目的地無法使用：{e}"))?)
        }
    };
    let normalized: Vec<Option<String>> = sources.iter().map(|s| normalize(s).ok()).collect();
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(sources.len());
    for (index, raw) in sources.iter().enumerate() {
        let label = if raw.trim().is_empty() { "（空白路徑）".to_string() } else { raw.clone() };
        let reject = |reason: &str| Err(Rejected { label: label.clone(), reason: reason.to_string() });
        let source = match normalize(raw) {
            Ok(s) => s,
            Err(e) => {
                out.push(reject(&e));
                continue;
            }
        };
        if seen.iter().any(|s| paths::same_path(s, &source)) {
            out.push(reject("同一個來源重複出現，已略過重複項目。"));
            continue;
        }
        seen.push(source.clone());
        if paths::is_root(&source) {
            out.push(reject("不能把整個磁碟或網路共用根目錄當作操作來源。"));
            continue;
        }
        if protected.iter().any(|p| !p.is_empty() && paths::same_path(p, &source)) {
            out.push(reject("目前版本不允許直接操作整個系統或使用者設定檔根目錄。"));
            continue;
        }
        let nested = normalized.iter().enumerate().any(|(j, other)| {
            j != index
                && other
                    .as_deref()
                    .is_some_and(|o| !paths::same_path(o, &source) && paths::is_same_or_descendant(o, &source))
        });
        if nested {
            out.push(reject("來源也位於另一個已選來源內，為避免重複處理已略過。"));
            continue;
        }
        let Some(dest) = dest.as_deref() else {
            out.push(Ok(Planned { source, target: None, rename_on_collision: false }));
            continue;
        };
        let target = paths::join(dest, paths::file_name(&source));
        let same = paths::same_path(&source, &target);
        if same && kind == OpKind::Copy {
            // 在同一資料夾貼上：建立副本並由 Windows 自動命名。
            out.push(Ok(Planned { source, target: Some(target), rename_on_collision: true }));
            continue;
        }
        if same {
            out.push(reject("來源與目的地相同，不需要移動。"));
            continue;
        }
        if paths::is_same_or_descendant(&source, &target) {
            out.push(reject("不能將來源複製或移動至自身或其子目錄。"));
            continue;
        }
        let covers_other = normalized.iter().enumerate().any(|(j, other)| {
            j != index && other.as_deref().is_some_and(|o| paths::is_same_or_descendant(&target, o))
        });
        if covers_other {
            out.push(reject("目的項目會覆蓋或包含另一個已選來源，已拒絕此操作。"));
            continue;
        }
        out.push(Ok(Planned { source, target: Some(target), rename_on_collision: false }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn copy_basic_and_same_folder() {
        let r = plan(OpKind::Copy, &s(&["C:\\a\\x.txt", "C:\\b\\y"]), Some("C:\\a\\"), &[]).unwrap();
        assert_eq!(
            r[0],
            Ok(Planned { source: "C:\\a\\x.txt".into(), target: Some("C:\\a\\x.txt".into()), rename_on_collision: true })
        );
        assert_eq!(
            r[1],
            Ok(Planned { source: "C:\\b\\y".into(), target: Some("C:\\a\\y".into()), rename_on_collision: false })
        );
    }

    #[test]
    fn move_rejections() {
        let r = plan(OpKind::Move, &s(&["C:\\a\\x.txt"]), Some("C:\\a"), &[]).unwrap();
        assert!(r[0].is_err());
        let r = plan(OpKind::Move, &s(&["C:\\a"]), Some("C:\\a\\sub"), &[]).unwrap();
        assert_eq!(r[0].as_ref().unwrap_err().reason, "不能將來源複製或移動至自身或其子目錄。");
        let r = plan(OpKind::Copy, &s(&["C:\\"]), Some("D:\\x"), &[]).unwrap();
        assert!(r[0].as_ref().unwrap_err().reason.contains("根目錄"));
    }

    #[test]
    fn duplicates_nested_and_protected() {
        let r = plan(
            OpKind::Delete,
            &s(&["C:\\a", "c:\\A\\", "C:\\a\\b", "C:\\Windows", "relative", "C:\\ok."]),
            None,
            &s(&["C:\\Windows"]),
        )
        .unwrap();
        assert!(r[0].is_ok());
        assert!(r[1].as_ref().unwrap_err().reason.contains("重複"));
        assert!(r[2].as_ref().unwrap_err().reason.contains("另一個已選來源內"));
        assert!(r[3].as_ref().unwrap_err().reason.contains("系統"));
        assert!(r[4].as_ref().unwrap_err().reason.contains("完整"));
        assert!(r[5].as_ref().unwrap_err().reason.contains("不支援"));
    }

    #[test]
    fn target_covers_other_source() {
        // 把 C:\x\a 移到 C:\ 會產生 C:\a，而 C:\a\f 是另一個來源。
        let r = plan(OpKind::Move, &s(&["C:\\x\\a", "C:\\a\\f"]), Some("C:\\"), &[]).unwrap();
        assert!(r[0].as_ref().unwrap_err().reason.contains("覆蓋"));
    }

    #[test]
    fn bad_destination() {
        assert!(plan(OpKind::Copy, &s(&["C:\\a"]), Some("rel"), &[]).is_err());
        assert!(plan(OpKind::Copy, &s(&["C:\\a"]), None, &[]).is_err());
        assert!(plan(OpKind::Copy, &s(&["C:\\a"]), Some("\\\\?\\C:\\x"), &[]).is_err());
    }
}

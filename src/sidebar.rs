//! 側欄的大標籤：「常用位置」、使用者自己新增的標籤、「磁碟」，以及它們由上到下的順序（可以拖曳調整）。
//!
//! 自訂標籤就是設定檔的 `BookmarkGroups`（標籤名稱與裡面的資料夾，資料夾本身也列在 `Favorites`）；
//! 順序存在 `SidebarOrder`：標籤名稱，常用位置、磁碟用 [`FAVORITES`]、[`DRIVES`]。
//! 順序裡沒列到的照預設的位置補上，列了但已經不存在的略過，所以順序永遠是完整、不重複的。

/// 順序裡代表「常用位置」的名稱
pub const FAVORITES: &str = "::favorites";
/// 順序裡代表「磁碟」的名稱
pub const DRIVES: &str = "::drives";
/// 標籤名稱最多幾個字
pub const MAX_NAME: usize = 40;
/// 自訂標籤最多幾個
pub const MAX_GROUPS: usize = 50;

/// 側欄的一個大標籤
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Favorites,
    /// 第幾個自訂標籤（`BookmarkGroups` 的索引）
    Group(usize),
    Drives,
}

fn find_group(groups: &[&str], name: &str) -> Option<usize> {
    groups.iter().position(|g| *g == name).or_else(|| groups.iter().position(|g| g.to_lowercase() == name.to_lowercase()))
}

/// 側欄由上到下的大標籤：照 `order`，沒列到的補上——常用位置放最前面，自訂標籤放在磁碟前面，磁碟放最後。
pub fn sections(order: &[String], groups: &[&str]) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for name in order {
        let s = match name.as_str() {
            FAVORITES => Some(Section::Favorites),
            DRIVES => Some(Section::Drives),
            other => find_group(groups, other).map(Section::Group),
        };
        if let Some(s) = s.filter(|s| !out.contains(s)) {
            out.push(s);
        }
    }
    if !out.contains(&Section::Favorites) {
        out.insert(0, Section::Favorites);
    }
    for i in 0..groups.len() {
        if !out.contains(&Section::Group(i)) {
            match out.iter().position(|s| *s == Section::Drives) {
                Some(d) => out.insert(d, Section::Group(i)),
                None => out.push(Section::Group(i)),
            }
        }
    }
    if !out.contains(&Section::Drives) {
        out.push(Section::Drives);
    }
    out
}

/// 寫回設定檔的順序
pub fn order_names(sections: &[Section], groups: &[&str]) -> Vec<String> {
    sections
        .iter()
        .filter_map(|s| match s {
            Section::Favorites => Some(FAVORITES.to_string()),
            Section::Drives => Some(DRIVES.to_string()),
            Section::Group(i) => groups.get(*i).map(|g| g.to_string()),
        })
        .collect()
}

/// 拖曳排序：第 `from` 個移到第 `slot` 個縫（0 是最前面，`len` 是最後面）。位置沒變時回傳 None。
pub fn move_to_slot(sections: &[Section], from: usize, slot: usize) -> Option<Vec<Section>> {
    if from >= sections.len() || slot > sections.len() || slot == from || slot == from + 1 {
        return None;
    }
    let mut v = sections.to_vec();
    let s = v.remove(from);
    let at = if slot > from { slot - 1 } else { slot };
    v.insert(at, s);
    Some(v)
}

/// 標籤名稱有問題時回傳原因：空白、太長、和別的標籤（不分大小寫）或「常用位置」「磁碟」同名。
/// `except`：改名時自己原本的索引。
pub fn name_error(name: &str, groups: &[&str], except: Option<usize>) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some(tr!("請輸入標籤名稱。", "Enter a name for the section.").into());
    }
    if name.chars().count() > MAX_NAME {
        return Some(trf!("標籤名稱最多 {MAX_NAME} 個字。", "Section names can be up to {MAX_NAME} characters."));
    }
    if name.starts_with("::") {
        return Some(tr!("標籤名稱不能以 :: 開頭。", "Section names can't start with ::.").into());
    }
    let lower = name.to_lowercase();
    let reserved = [tr!("常用位置", "Favorites"), tr!("磁碟", "Drives"), "常用位置", "磁碟", "Favorites", "Drives"];
    if reserved.iter().any(|r| r.to_lowercase() == lower) || groups.iter().enumerate().any(|(i, g)| Some(i) != except && g.to_lowercase() == lower) {
        return Some(trf!("已經有叫「{name}」的標籤了，請換一個名稱。", "There's already a section called “{name}”. Choose another name."));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use Section::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_order_puts_groups_between_favorites_and_drives() {
        assert_eq!(sections(&[], &[]), vec![Favorites, Drives]);
        assert_eq!(sections(&[], &["工作", "照片"]), vec![Favorites, Group(0), Group(1), Drives]);
    }

    #[test]
    fn saved_order_is_followed_and_completed() {
        let groups = ["工作", "照片"];
        assert_eq!(sections(&names(&["照片", DRIVES, FAVORITES, "工作"]), &groups), vec![Group(1), Drives, Favorites, Group(0)]);
        // 新增的標籤（順序裡還沒有）放在磁碟前面；刪掉的、重複的略過；名稱不分大小寫
        assert_eq!(sections(&names(&[DRIVES, "照片", "舊的", "照片"]), &groups), vec![Favorites, Group(0), Drives, Group(1)]);
        // 常用位置沒列到：放最前面
        assert_eq!(sections(&names(&["work", DRIVES]), &["Work"]), vec![Favorites, Group(0), Drives]);
        // 磁碟沒列到：放最後
        assert_eq!(sections(&names(&["照片", FAVORITES]), &groups), vec![Group(1), Favorites, Group(0), Drives]);
    }

    #[test]
    fn order_names_round_trip() {
        let groups = ["工作", "照片"];
        let s = vec![Group(1), Favorites, Drives, Group(0)];
        let n = order_names(&s, &groups);
        assert_eq!(n, names(&["照片", FAVORITES, DRIVES, "工作"]));
        assert_eq!(sections(&n, &groups), s);
    }

    #[test]
    fn moving_by_slot() {
        let s = vec![Favorites, Group(0), Group(1), Drives];
        // 放回原處（自己前面或後面的縫）：沒有移動
        assert_eq!(move_to_slot(&s, 1, 1), None);
        assert_eq!(move_to_slot(&s, 1, 2), None);
        assert_eq!(move_to_slot(&s, 0, 4), Some(vec![Group(0), Group(1), Drives, Favorites]));
        assert_eq!(move_to_slot(&s, 3, 0), Some(vec![Drives, Favorites, Group(0), Group(1)]));
        assert_eq!(move_to_slot(&s, 2, 1), Some(vec![Favorites, Group(1), Group(0), Drives]));
        assert_eq!(move_to_slot(&s, 1, 3), Some(vec![Favorites, Group(1), Group(0), Drives]));
        assert_eq!(move_to_slot(&s, 4, 0), None);
        assert_eq!(move_to_slot(&s, 0, 5), None);
    }

    #[test]
    fn names_are_checked() {
        let groups = ["工作", "Photos"];
        assert!(name_error("  ", &groups, None).is_some());
        assert!(name_error("工作", &groups, None).is_some());
        assert!(name_error("photos", &groups, None).is_some());
        assert!(name_error("常用位置", &groups, None).is_some());
        assert!(name_error("Drives", &groups, None).is_some());
        assert!(name_error("::x", &groups, None).is_some());
        assert!(name_error(&"字".repeat(MAX_NAME + 1), &groups, None).is_some());
        assert!(name_error(&"字".repeat(MAX_NAME), &groups, None).is_none());
        // 改名時可以維持原名（只改大小寫）
        assert!(name_error("photos", &groups, Some(1)).is_none());
        assert!(name_error("專案", &groups, None).is_none());
    }
}

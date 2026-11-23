//! 連續啟動失敗時的救援（實際的檔案與登錄操作在 `src/win/rescue.rs`）。
//!
//! Stillfeather 設為預設檔案總管、攔截 Win+E 時，如果某台電腦上它一啟動就當掉（常見原因是第三方的右鍵選單、
//! 縮圖或圖示擴充在它的程式裡出錯），使用者會連資料夾都打不開。所以主程式每次啟動先在記號資料夾放一個
//! `<開始時間毫秒>-<程序代號>` 的空檔案，正常結束或撐過 [`STABLE_MS`] 就刪掉。下次啟動時看到
//! [`WINDOW_MS`] 內已經留下 [`LIMIT`] 個沒刪掉的記號，就表示連續當掉：改回 Windows 檔案總管並提示使用者。
//!
//! 每次啟動各自一個檔案（不是一個共用的計數檔），同時開好幾個資料夾時也不會互相覆寫。
//! 還在執行的程序（例如剛啟動、或正把要求轉給已經開著的視窗）的記號不算：只算已經結束、卻沒拿掉記號的那些。

/// 記號資料夾（在 Stillfeather 的資料夾底下）
pub const DIR_NAME: &str = "startup";
/// 多久之內的失敗才算「連續」
pub const WINDOW_MS: u64 = 60_000;
/// 幾次沒能正常啟動就救援
pub const LIMIT: usize = 3;
/// 撐過這麼久就算啟動成功（之後才當掉的不算啟動失敗）
pub const STABLE_MS: u64 = 20_000;
/// 時鐘稍微往回調時，略晚於現在的記號仍然算數
const CLOCK_SLACK_MS: u64 = 5_000;

/// 一次啟動的記號檔名
pub fn marker_name(start_ms: u64, pid: u32) -> String {
    format!("{start_ms}-{pid}")
}

/// 記號檔名裡的（開始時間, 程序代號）；不是記號就回傳 None
pub fn parse_marker(name: &str) -> Option<(u64, u32)> {
    let (time, pid) = name.split_once('-')?;
    if time.is_empty() || pid.is_empty() || !time.bytes().all(|b| b.is_ascii_digit()) || !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((time.parse().ok()?, pid.parse().ok()?))
}

/// 記號檔名裡的開始時間；不是記號就回傳 None
pub fn marker_time(name: &str) -> Option<u64> {
    parse_marker(name).map(|(t, _)| t)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// 連續當掉：這次改回 Windows 檔案總管，不開 Stillfeather
    pub rescue: bool,
    /// 要刪掉的記號（太舊的、看不懂的；救援時全部刪掉，讓下一次重新計算）
    pub remove: Vec<String>,
}

/// `existing`：記號資料夾裡現有的檔名；`now_ms`：現在的時間；`running(pid)`：那個程序是不是還在執行。
/// 還在執行的程序的記號不算失敗也不刪（它會自己拿掉）；其餘的就是沒能正常結束的啟動。
pub fn check(existing: &[String], now_ms: u64, running: &dyn Fn(u32) -> bool) -> Check {
    let ended: Vec<&String> = existing.iter().filter(|n| parse_marker(n).is_none_or(|(_, pid)| !running(pid))).collect();
    let recent = |name: &str| marker_time(name).is_some_and(|t| t <= now_ms.saturating_add(CLOCK_SLACK_MS) && now_ms.saturating_sub(t) < WINDOW_MS);
    let failures = ended.iter().filter(|n| recent(n)).count();
    if failures >= LIMIT {
        return Check { rescue: true, remove: ended.into_iter().cloned().collect() };
    }
    Check { rescue: false, remove: ended.into_iter().filter(|n| !recent(n)).cloned().collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 測試裡的程序都已經結束
    fn none_running(_: u32) -> bool {
        false
    }

    fn check(existing: &[String], now_ms: u64) -> Check {
        super::check(existing, now_ms, &none_running)
    }

    #[test]
    fn marker_names() {
        assert_eq!(marker_name(1_795_400_000_123, 4321), "1795400000123-4321");
        assert_eq!(marker_time("1795400000123-4321"), Some(1_795_400_000_123));
        assert_eq!(parse_marker("1795400000123-4321"), Some((1_795_400_000_123, 4321)));
        assert_eq!(parse_marker("1-99999999999"), None, "程序代號超過 u32");
        assert_eq!(marker_time("1795400000123"), None);
        assert_eq!(marker_time("-4321"), None);
        assert_eq!(marker_time("12-"), None);
        assert_eq!(marker_time("12-x"), None);
        assert_eq!(marker_time("desktop.ini"), None);
    }

    #[test]
    fn three_recent_failures_rescue() {
        let now = 1_000_000;
        // 兩次失敗：照常啟動
        let c = check(&names(&["990000-1", "995000-2"]), now);
        assert!(!c.rescue);
        assert!(c.remove.is_empty());
        // 一分鐘內三次：救援，並清掉所有記號（下一次重新計算）
        let all = names(&["950000-1", "990000-2", "995000-3"]);
        let c = check(&all, now);
        assert!(c.rescue);
        assert_eq!(c.remove, all);
    }

    #[test]
    fn old_and_foreign_markers_are_cleaned_not_counted() {
        let now = 1_000_000;
        let c = check(&names(&["100-1", "939999-2", "990000-3", "notes.txt", "995000-4"]), now);
        assert!(!c.rescue, "超過一分鐘的不算");
        assert_eq!(c.remove, names(&["100-1", "939999-2", "notes.txt"]));
    }

    #[test]
    fn markers_of_running_processes_do_not_count() {
        // 一次選四個資料夾按 Enter：四個程序幾乎同時啟動，前三個還在執行（記號還在）時第四個不能誤判成連續當掉
        let now = 1_000_000;
        let all = names(&["999000-11", "999100-12", "999200-13"]);
        let c = super::check(&all, now, &|pid| pid != 0);
        assert!(!c.rescue);
        assert!(c.remove.is_empty(), "執行中的記號由它自己拿掉");
        // 兩個已經結束、一個還在執行：還不到三次
        let c = super::check(&all, now, &|pid| pid == 13);
        assert!(!c.rescue);
        // 三個都結束了才算
        assert!(super::check(&all, now, &|_| false).rescue);
        // 救援時只刪已經結束的
        let mut more = all.clone();
        more.push("999300-14".into());
        let c = super::check(&more, now, &|pid| pid == 14);
        assert!(c.rescue);
        assert_eq!(c.remove, all);
    }

    #[test]
    fn clock_changes() {
        let now = 1_000_000;
        // 時鐘稍微往回調：略晚於現在的仍然算
        assert!(check(&names(&["1003000-1", "1004000-2", "999000-3"]), now).rescue);
        // 遠在未來的（時鐘大幅調整）不算，清掉
        let c = check(&names(&["9000000-1", "999000-2"]), now);
        assert!(!c.rescue);
        assert_eq!(c.remove, names(&["9000000-1"]));
        // 時間 0 附近也不會溢位
        assert!(!check(&names(&["0-1"]), 0).rescue);
    }
}

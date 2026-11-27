//! 最近搜尋：在搜尋框按 ↓ 列出最近 10 次搜尋，連篩選條件（類型、日期、大小）與範圍
//! （整台電腦、含不含子資料夾）一起記，存在設定檔的 `RecentSearches`。

use crate::category::{self, Category};
use crate::json::{self, Value};
use crate::searchquery::{DateSpec, SizeRange, Ymd};

/// 最多記幾次
pub const MAX: usize = 10;
/// 一筆最多幾個字（太長的不記，免得設定檔被撐大）
const MAX_TEXT: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentSearch {
    /// 搜尋框的文字（可能是空的：只靠篩選條件搜尋）
    pub text: String,
    /// 搜尋整台電腦
    pub global: bool,
    /// 含子資料夾
    pub recursive: bool,
    pub category: Category,
    pub date: Option<DateSpec>,
    pub size: Option<SizeRange>,
}

impl RecentSearch {
    /// 選單上的主要文字
    pub fn title(&self) -> String {
        let t = self.text.trim();
        if t.is_empty() {
            tr!("（只用篩選條件）", "(filters only)").into()
        } else {
            t.to_string()
        }
    }

    /// 選單右邊的條件：類型、日期、大小、範圍（沒有就是空的）
    pub fn conditions(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.category != Category::All {
            parts.push(self.category.label().into());
        }
        if let Some(d) = self.date {
            parts.push(d.label());
        }
        if let Some(s) = self.size {
            parts.push(s.label());
        }
        if self.global {
            parts.push(tr!("整台電腦", "Whole PC").into());
        } else if !self.recursive {
            parts.push(tr!("不含子資料夾", "No subfolders").into());
        }
        parts.join(" · ")
    }
}

/// 記下一次搜尋，放在最前面（之前一模一樣的那一筆拿掉）。`refine`：同一次搜尋改了篩選條件，
/// 最前面那一筆的文字一樣時直接換掉，不另外多一筆。回傳清單有沒有變。
pub fn remember(list: &mut Vec<RecentSearch>, item: RecentSearch, refine: bool) -> bool {
    if item.text.chars().count() > MAX_TEXT {
        return false;
    }
    if list.first() == Some(&item) {
        return false;
    }
    let before = list.clone();
    if refine && list.first().is_some_and(|f| f.text == item.text && f.global == item.global) {
        list.remove(0);
    }
    list.retain(|r| *r != item);
    list.insert(0, item);
    list.truncate(MAX);
    *list != before
}

fn category_name(c: Category) -> &'static str {
    match c {
        Category::All => "All",
        Category::Folder => "Folder",
        Category::Image => "Image",
        Category::Document => "Document",
        Category::Media => "Media",
        Category::Archive => "Archive",
        Category::Other => "Other",
    }
}

fn parse_category(s: &str) -> Option<Category> {
    category::ALL.iter().copied().find(|c| category_name(*c).eq_ignore_ascii_case(s))
}

fn encode_date(d: DateSpec) -> String {
    let endpoint = |d: Option<Ymd>| d.map_or("-".into(), |d| d.text());
    match d {
        DateSpec::Today => "today".into(),
        DateSpec::Yesterday => "yesterday".into(),
        DateSpec::ThisWeek => "week".into(),
        DateSpec::ThisMonth => "month".into(),
        DateSpec::ThisYear => "year".into(),
        DateSpec::LastDays(n) => format!("days {n}"),
        DateSpec::Range { from, to } => format!("range {} {}", endpoint(from), endpoint(to)),
    }
}

fn parse_date(s: &str) -> Option<DateSpec> {
    fn endpoint(s: &str) -> Option<Option<Ymd>> {
        if s == "-" {
            return Some(None);
        }
        let mut n = s.split('-');
        let d = Ymd::new(n.next()?.parse().ok()?, n.next()?.parse().ok()?, n.next()?.parse().ok()?)?;
        n.next().is_none().then_some(Some(d))
    }
    let f: Vec<&str> = s.split_whitespace().collect();
    Some(match f.as_slice() {
        ["today"] => DateSpec::Today,
        ["yesterday"] => DateSpec::Yesterday,
        ["week"] => DateSpec::ThisWeek,
        ["month"] => DateSpec::ThisMonth,
        ["year"] => DateSpec::ThisYear,
        ["days", n] => DateSpec::LastDays(n.parse().ok().filter(|n| *n > 0)?),
        ["range", a, b] => DateSpec::Range { from: endpoint(a)?, to: endpoint(b)? },
        _ => return None,
    })
}

fn encode_size(s: SizeRange) -> String {
    if s.max == u64::MAX {
        format!("{} -", s.min)
    } else {
        format!("{} {}", s.min, s.max)
    }
}

fn parse_size(s: &str) -> Option<SizeRange> {
    let mut f = s.split_whitespace();
    let min = f.next()?.parse().ok()?;
    let max = match f.next()? {
        "-" => u64::MAX,
        m => m.parse().ok()?,
    };
    (f.next().is_none() && min < max).then_some(SizeRange { min, max })
}

/// 寫進設定檔的 JSON（陣列）
pub fn to_json(list: &[RecentSearch]) -> String {
    if list.is_empty() {
        return "[]".into();
    }
    let items: Vec<String> = list
        .iter()
        .map(|r| {
            let date = r.date.map_or("null".into(), |d| json::quote(&encode_date(d)));
            let size = r.size.map_or("null".into(), |s| json::quote(&encode_size(s)));
            format!(
                "    {{\"Text\": {}, \"Global\": {}, \"Recursive\": {}, \"Category\": {}, \"Date\": {date}, \"Size\": {size}}}",
                json::quote(&r.text),
                r.global,
                r.recursive,
                json::quote(category_name(r.category))
            )
        })
        .collect();
    format!("[\n{}\n  ]", items.join(",\n"))
}

/// 從設定檔讀回來。看不懂的那一筆略過（不讓整個設定檔讀不進來），最多 [`MAX`] 筆。
pub fn from_json(value: Option<&Value>) -> Vec<RecentSearch> {
    let Some(Value::Arr(items)) = value else { return Vec::new() };
    let mut out = Vec::new();
    for item in items {
        let text = match item.get("Text") {
            Some(Value::Str(s)) if s.chars().count() <= MAX_TEXT => s.clone(),
            _ => continue,
        };
        let flag = |k: &str, default: bool| match item.get(k) {
            Some(Value::Bool(b)) => *b,
            _ => default,
        };
        let category = match item.get("Category") {
            Some(Value::Str(s)) => match parse_category(s) {
                Some(c) => c,
                None => continue,
            },
            _ => Category::All,
        };
        let date = match item.get("Date") {
            None | Some(Value::Null) => None,
            Some(Value::Str(s)) => match parse_date(s) {
                Some(d) => Some(d),
                None => continue,
            },
            Some(_) => continue,
        };
        let size = match item.get("Size") {
            None | Some(Value::Null) => None,
            Some(Value::Str(s)) => match parse_size(s) {
                Some(r) => Some(r),
                None => continue,
            },
            Some(_) => continue,
        };
        let r = RecentSearch { text, global: flag("Global", false), recursive: flag("Recursive", true), category, date, size };
        if !out.contains(&r) {
            out.push(r);
        }
        if out.len() >= MAX {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::searchquery::MB;

    fn r(text: &str) -> RecentSearch {
        RecentSearch { text: text.into(), global: false, recursive: true, category: Category::All, date: None, size: None }
    }

    #[test]
    fn newest_first_without_duplicates() {
        let mut list = Vec::new();
        assert!(remember(&mut list, r("報告"), false));
        assert!(remember(&mut list, r("發票"), false));
        // 同一筆再搜一次：移到最前面
        assert!(remember(&mut list, r("報告"), false));
        assert_eq!(list.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["報告", "發票"]);
        // 已經在最前面：沒有變
        assert!(!remember(&mut list, r("報告"), false));
        // 文字一樣、條件不同是另一筆
        let docs = RecentSearch { category: Category::Document, ..r("報告") };
        assert!(remember(&mut list, docs.clone(), false));
        assert_eq!(list.len(), 3);
        // 最多 10 筆
        for i in 0..20 {
            remember(&mut list, r(&format!("q{i}")), false);
        }
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].text, "q19");
    }

    #[test]
    fn refining_replaces_the_latest() {
        let mut list = vec![r("發票")];
        remember(&mut list, r("報告"), false);
        let week = RecentSearch { date: Some(DateSpec::ThisWeek), ..r("報告") };
        assert!(remember(&mut list, week.clone(), true));
        let big = RecentSearch { size: Some(SizeRange { min: 10 * MB, max: u64::MAX }), ..week.clone() };
        assert!(remember(&mut list, big.clone(), true));
        assert_eq!(list, vec![big, r("發票")]);
        // 文字不同：不取代
        assert!(remember(&mut list, r("合約"), true));
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn json_round_trip() {
        let list = vec![
            RecentSearch {
                text: "\"年度 報告\" -草稿".into(),
                global: true,
                recursive: true,
                category: Category::Document,
                date: Some(DateSpec::Range { from: Ymd::new(2026, 11, 1), to: None }),
                size: Some(SizeRange { min: MB, max: 100 * MB }),
            },
            RecentSearch { recursive: false, date: Some(DateSpec::LastDays(7)), size: Some(SizeRange { min: 0, max: MB }), ..r("") },
            RecentSearch { date: Some(DateSpec::ThisWeek), size: Some(SizeRange { min: MB, max: u64::MAX }), ..r("照片") },
        ];
        let text = format!("{{\"RecentSearches\": {}}}", to_json(&list));
        let root = json::parse(&text).unwrap();
        assert_eq!(from_json(root.get("RecentSearches")), list);
        assert_eq!(to_json(&[]), "[]");
    }

    #[test]
    fn bad_entries_are_skipped() {
        let text = r#"[{"Text": "ok"}, {"Text": 3}, {"Text": "bad date", "Date": "someday"}, {"Text": "bad size", "Size": "9 1"},
            {"Text": "bad kind", "Category": "Music"}, {"Text": "ok"}, {"Text": "old", "Date": "days 0"}]"#;
        let v = json::parse(text).unwrap();
        assert_eq!(from_json(Some(&v)), vec![r("ok")]);
        assert_eq!(from_json(Some(&Value::Str("x".into()))), Vec::new());
        assert_eq!(from_json(None), Vec::new());
    }

    #[test]
    fn labels() {
        assert_eq!(r("  報告 ").title(), "報告");
        assert!(!r("").title().is_empty());
        assert_eq!(r("x").conditions(), "");
        let all = RecentSearch { global: true, category: Category::Image, date: Some(DateSpec::Today), ..r("x") };
        assert_eq!(all.conditions().matches(" · ").count(), 2);
        let here = RecentSearch { recursive: false, ..r("x") };
        assert!(!here.conditions().is_empty());
    }
}

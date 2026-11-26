//! 搜尋條件：把搜尋框的文字解析成 [`Query`]，換算好日期後變成搜尋執行緒用的 [`Filter`]；
//! 另外是篩選籤的日期、大小條件與搜尋結果的相關性分數。
//!
//! 寫法盡量和 Everything、檔案總管相同：
//!
//! | 輸入 | 意思 |
//! |---|---|
//! | `報告 2025` | 檔名同時有「報告」和「2025」，順序不限 |
//! | `"年度 報告"` | 檔名有完整的「年度 報告」（也接受 “ ” 與「 」） |
//! | `-草稿` | 排除檔名有「草稿」的 |
//! | `*.pdf`、`ext:pdf;docx` | 依副檔名（`*`、`?` 是比對整個檔名的萬用字元） |
//! | `size:>100mb`、`size:1mb..10mb` | 依大小（資料夾沒有大小，不會符合） |
//! | `dm:today`、`dm:7d`、`dm:2026-11-01..2026-11-20` | 依修改日期（也接受 `datemodified:`） |
//! | `folder:`、`file:` | 只要資料夾，或只要檔案 |
//!
//! 用注音輸入法時冒號常常變成全形的「：」，`ext：pdf` 一樣認得；但只有前面是認得的條件名稱時才算條件，
//! 檔名本身可以有全形冒號（`藍色星原：旅謠`）。
//!
//! 日期以本地時間的日界線計算（「今天」是本地的今天 0 點到明天 0 點）：這裡只做曆法（[`DateSpec::local_days`]），
//! 換成 UTC 的 FILETIME 由介面執行緒呼叫 Windows 處理，所以這個模組在 Linux 上也能跑單元測試。

use crate::namematch::{fold, Pattern};

/// 說明視窗「搜尋語法」的表格（寫法、意思）；README 的表格內容相同
pub fn syntax_rows() -> Vec<(&'static str, &'static str)> {
    vec![
        ("報告 2025", tr!("檔名同時有這些詞，順序不限", "Names containing all of these words, in any order")),
        ("\"年度 報告\"", tr!("完整的詞組（也可以用「」）", "The exact phrase (“ ” also works)")),
        ("-草稿", tr!("排除檔名有「草稿」的", "Exclude names containing the word")),
        ("*.pdf", tr!("萬用字元比對整個檔名：* 是任意個字，? 是一個字", "Wildcards match the whole name: * is any text, ? is one character")),
        ("ext:pdf;docx", tr!("依副檔名（多個用 ; 分開）", "By extension (separate several with ;)")),
        ("size:>100mb", tr!("大於 100 MB（也可以 <、>=、<=；資料夾不會顯示）", "Larger than 100 MB (also <, >=, <=; folders are hidden)")),
        ("size:1mb..10mb", tr!("1～10 MB 之間（size:empty 是空檔案）", "Between 1 and 10 MB (size:empty for empty files)")),
        ("dm:today", tr!("今天修改的（yesterday、thisweek、thismonth、thisyear）", "Modified today (yesterday, thisweek, thismonth, thisyear)")),
        ("dm:7d", tr!("最近 7 天修改的（2w 是最近兩週）", "Modified in the last 7 days (2w for two weeks)")),
        ("dm:2026-11-01..2026-11-20", tr!("這段日期修改的（也可以寫 datemodified:）", "Modified between these dates (datemodified: also works)")),
        ("folder:  file:", tr!("只要資料夾／只要檔案（folder:src 是名稱有 src 的資料夾）", "Folders only / files only (folder:src: folders named with src)")),
    ]
}

// ---------------------------------------------------------------- 日期

/// 本地日期（年、月、日）
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ymd {
    pub y: i32,
    pub m: u32,
    pub d: u32,
}

fn leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

impl Ymd {
    /// 不存在的日期（2 月 30 日）回傳 None
    pub fn new(y: i32, m: u32, d: u32) -> Option<Ymd> {
        ((1601..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then_some(Ymd { y, m, d })
    }

    /// 自 1970-01-01 起的天數（Howard Hinnant 的 days_from_civil）
    fn days(self) -> i64 {
        let y = self.y as i64 - if self.m <= 2 { 1 } else { 0 };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = self.m as i64;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + self.d as i64 - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    fn from_days(z: i64) -> Ymd {
        let z = z + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
        Ymd { y, m, d }
    }

    pub fn add_days(self, n: i64) -> Ymd {
        Ymd::from_days(self.days() + n)
    }

    /// 星期幾：0 是星期一……6 是星期日（同 Windows 的 LOCALE_IFIRSTDAYOFWEEK）
    pub fn weekday(self) -> u32 {
        // 1970-01-01 是星期四
        ((self.days() + 3).rem_euclid(7)) as u32
    }

    fn first_of_month(self) -> Ymd {
        Ymd { d: 1, ..self }
    }

    fn next_month(self) -> Ymd {
        if self.m == 12 {
            Ymd { y: self.y + 1, m: 1, d: 1 }
        } else {
            Ymd { y: self.y, m: self.m + 1, d: 1 }
        }
    }

    pub fn text(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }
}

/// 修改日期的條件（還沒換算成時間）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateSpec {
    Today,
    Yesterday,
    /// 最近 N 天（含今天）
    LastDays(u32),
    /// 本週：從本地設定的一週第一天開始
    ThisWeek,
    ThisMonth,
    ThisYear,
    /// 兩個日期之間（都含）；None 是不限
    Range { from: Option<Ymd>, to: Option<Ymd> },
}

/// 篩選籤「日期」選單的固定選項（不限與自訂範圍另外處理）
pub const DATE_PRESETS: [DateSpec; 5] = [DateSpec::Today, DateSpec::Yesterday, DateSpec::ThisWeek, DateSpec::ThisMonth, DateSpec::ThisYear];

impl DateSpec {
    /// 本地日期的範圍：[開始, 結束)，None 是不限。`first_day_of_week`：0 是星期一……6 是星期日。
    pub fn local_days(&self, today: Ymd, first_day_of_week: u32) -> (Option<Ymd>, Option<Ymd>) {
        let tomorrow = today.add_days(1);
        match *self {
            DateSpec::Today => (Some(today), Some(tomorrow)),
            DateSpec::Yesterday => (Some(today.add_days(-1)), Some(today)),
            DateSpec::LastDays(n) => (Some(today.add_days(-(n.max(1) as i64 - 1))), Some(tomorrow)),
            DateSpec::ThisWeek => {
                let back = (today.weekday() + 7 - first_day_of_week % 7) % 7;
                let start = today.add_days(-(back as i64));
                (Some(start), Some(start.add_days(7)))
            }
            DateSpec::ThisMonth => (Some(today.first_of_month()), Some(today.next_month())),
            DateSpec::ThisYear => (Ymd::new(today.y, 1, 1), Ymd::new(today.y + 1, 1, 1)),
            DateSpec::Range { from, to } => {
                let (from, to) = match (from, to) {
                    (Some(a), Some(b)) if a > b => (Some(b), Some(a)),
                    other => other,
                };
                (from, to.map(|d| d.add_days(1)))
            }
        }
    }

    /// 篩選籤與狀態列上的文字
    pub fn label(&self) -> String {
        match *self {
            DateSpec::Today => tr!("今天", "Today").into(),
            DateSpec::Yesterday => tr!("昨天", "Yesterday").into(),
            DateSpec::LastDays(n) => trf!("最近 {n} 天", "Last {n} days"),
            DateSpec::ThisWeek => tr!("本週", "This week").into(),
            DateSpec::ThisMonth => tr!("本月", "This month").into(),
            DateSpec::ThisYear => tr!("今年", "This year").into(),
            DateSpec::Range { from: Some(a), to: Some(b) } if a == b => a.text(),
            DateSpec::Range { from: Some(a), to: Some(b) } => format!("{}～{}", a.text(), b.text()),
            DateSpec::Range { from: Some(a), to: None } => trf!("{} 之後", "Since {}", a.text()),
            DateSpec::Range { from: None, to: Some(b) } => trf!("{} 之前", "Until {}", b.text()),
            DateSpec::Range { from: None, to: None } => tr!("不限", "Any time").into(),
        }
    }
}

/// 換算好的時間範圍：FILETIME（UTC），[from, to)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    pub from: u64,
    pub to: u64,
}

impl TimeRange {
    pub fn contains(&self, t: u64) -> bool {
        self.from <= t && t < self.to
    }

    pub fn intersect(self, other: TimeRange) -> TimeRange {
        TimeRange { from: self.from.max(other.from), to: self.to.min(other.to) }
    }

    /// 符合 `other` 的一定也符合這個範圍
    pub fn covers(&self, other: &TimeRange) -> bool {
        self.from <= other.from && other.to <= self.to
    }
}

// ---------------------------------------------------------------- 大小

pub const KB: u64 = 1024;
pub const MB: u64 = 1024 * KB;
pub const GB: u64 = 1024 * MB;
pub const TB: u64 = 1024 * GB;

/// 大小範圍（位元組）：[min, max)；max 是 u64::MAX 時不限
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeRange {
    pub min: u64,
    pub max: u64,
}

/// 篩選籤「大小」選單的固定選項：小於 1 MB、1～100 MB、100 MB～1 GB、大於 1 GB
pub const SIZE_PRESETS: [SizeRange; 4] = [
    SizeRange { min: 0, max: MB },
    SizeRange { min: MB, max: 100 * MB },
    SizeRange { min: 100 * MB, max: GB },
    SizeRange { min: GB, max: u64::MAX },
];

impl SizeRange {
    pub fn contains(&self, n: u64) -> bool {
        self.min <= n && n < self.max
    }

    pub fn intersect(self, other: SizeRange) -> SizeRange {
        SizeRange { min: self.min.max(other.min), max: self.max.min(other.max) }
    }

    pub fn covers(&self, other: &SizeRange) -> bool {
        self.min <= other.min && other.max <= self.max
    }

    /// 篩選籤與狀態列上的文字：「< 1 MB」「1～100 MB」「100 MB～1 GB」「> 1 GB」
    pub fn label(&self) -> String {
        let open = self.max == u64::MAX;
        match (self.min, open) {
            (0, true) => tr!("不限", "Any size").into(),
            (0, false) => format!("< {}", format_bytes(self.max)),
            (min, true) => format!("> {}", format_bytes(min)),
            (min, false) => {
                let (a, b) = (format_bytes(min), format_bytes(self.max));
                // 同一個單位只寫一次：1～100 MB
                match (a.rsplit_once(' '), b.rsplit_once(' ')) {
                    (Some((an, au)), Some((_, bu))) if au == bu => format!("{an}～{b}"),
                    _ => format!("{a}～{b}"),
                }
            }
        }
    }
}

/// 「1.5 GB」「100 MB」「512 KB」「10 B」（1024 進位，最多一位小數，整數不寫小數點）
pub fn format_bytes(n: u64) -> String {
    let units = [(TB, "TB"), (GB, "GB"), (MB, "MB"), (KB, "KB")];
    for (unit, name) in units {
        if n >= unit {
            let tenths = ((n as u128 * 10 + unit as u128 / 2) / unit as u128) as u64;
            return if tenths.is_multiple_of(10) { format!("{} {name}", tenths / 10) } else { format!("{}.{} {name}", tenths / 10, tenths % 10) };
        }
    }
    format!("{n} B")
}

/// 「100mb」「1.5 GB」「512k」「2048」（沒有單位是位元組；1024 進位）。認不得時回傳 None。
pub fn parse_size(text: &str) -> Option<u64> {
    let t: String = text.chars().filter(|c| !c.is_whitespace() && *c != ',').collect::<String>().to_ascii_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    if num.is_empty() || num.matches('.').count() > 1 {
        return None;
    }
    let mult = match unit {
        "" | "b" => 1,
        "k" | "kb" | "kib" => KB,
        "m" | "mb" | "mib" => MB,
        "g" | "gb" | "gib" => GB,
        "t" | "tb" | "tib" => TB,
        _ => return None,
    };
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    let whole: u64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let mut value = whole.checked_mul(mult)?;
    if !frac.is_empty() {
        let digits = frac.len().min(6) as u32;
        let f: u64 = frac[..digits as usize].parse().ok()?;
        value = value.checked_add((f as u128 * mult as u128 / 10u128.pow(digits)) as u64)?;
    }
    Some(value)
}

// ---------------------------------------------------------------- 搜尋框的條件

/// 只要資料夾或只要檔案
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Any,
    Folders,
    Files,
}

/// 搜尋框文字解析的結果（日期還沒換算）
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// 都要有的詞（順序不限）；引號內的整串是一個詞
    pub terms: Vec<String>,
    /// 要排除的詞（`-草稿`）
    pub exclude: Vec<String>,
    /// 副檔名（小寫、不含句點）；空的是不限
    pub exts: Vec<String>,
    pub exclude_exts: Vec<String>,
    pub size: Option<SizeRange>,
    /// 多個 dm: 條件要同時符合
    pub dates: Vec<DateSpec>,
    pub kind: Kind,
}

/// 認得的條件名稱
const KEYS: [&str; 6] = ["ext", "size", "dm", "datemodified", "folder", "file"];

struct Token {
    /// 前面有 `-`（在引號外）
    neg: bool,
    text: String,
    /// 有任何一段在引號裡：不當成條件，整串比對
    quoted: bool,
}

fn is_open_quote(c: char) -> Option<char> {
    match c {
        '"' => Some('"'),
        '“' => Some('”'),
        '「' => Some('」'),
        '『' => Some('』'),
        _ => None,
    }
}

fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut neg = false;
        if matches!(chars[i], '-' | '－') && chars.get(i + 1).is_some_and(|c| !c.is_whitespace()) {
            neg = true;
            i += 1;
        }
        let mut s = String::new();
        let mut quoted = false;
        let mut close: Option<char> = None;
        while i < chars.len() {
            let c = chars[i];
            if let Some(end) = close {
                if c == end {
                    close = None;
                } else {
                    s.push(c);
                }
            } else if let Some(end) = is_open_quote(c) {
                close = Some(end);
                quoted = true;
            } else if c.is_whitespace() {
                break;
            } else {
                s.push(c);
            }
            i += 1;
        }
        if !s.is_empty() {
            out.push(Token { neg, text: s, quoted });
        } else if neg && !quoted {
            out.push(Token { neg: false, text: "-".into(), quoted: false });
        }
    }
    out
}

/// 全形的比較符號、波浪號換成半形（只用在條件的值）
fn half_width(v: &str) -> String {
    v.chars()
        .map(|c| match c {
            '＞' => '>',
            '＜' => '<',
            '＝' => '=',
            '～' | '〜' => '~',
            '．' => '.',
            '／' => '/',
            '－' => '-',
            c if ('０'..='９').contains(&c) => char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c),
            c => c,
        })
        .collect()
}

/// `a..b`、`a~b`：拆成兩邊（任一邊可以空著）
fn split_range(v: &str) -> Option<(&str, &str)> {
    v.split_once("..").or_else(|| v.split_once('~'))
}

fn parse_size_condition(raw: &str) -> Result<SizeRange, String> {
    let v = half_width(raw).to_ascii_lowercase();
    let bad = || trf!("看不懂的大小條件：size:{raw}（例如 size:>100mb、size:1mb..10mb）", "Can't read the size condition size:{raw} (e.g. size:>100mb, size:1mb..10mb)");
    let num = |s: &str| parse_size(s).ok_or_else(bad);
    if v == "empty" || v == "空" {
        return Ok(SizeRange { min: 0, max: 1 });
    }
    let r = if let Some(rest) = v.strip_prefix(">=") {
        SizeRange { min: num(rest)?, max: u64::MAX }
    } else if let Some(rest) = v.strip_prefix("<=") {
        SizeRange { min: 0, max: num(rest)?.saturating_add(1) }
    } else if let Some(rest) = v.strip_prefix('>') {
        SizeRange { min: num(rest)?.saturating_add(1), max: u64::MAX }
    } else if let Some(rest) = v.strip_prefix('<') {
        SizeRange { min: 0, max: num(rest)? }
    } else if let Some((a, b)) = split_range(&v) {
        let min = if a.is_empty() { 0 } else { num(a)? };
        let max = if b.is_empty() { u64::MAX } else { num(b)?.saturating_add(1) };
        let (min, max) = if max != u64::MAX && min >= max { (max.saturating_sub(1), min.saturating_add(1)) } else { (min, max) };
        SizeRange { min, max }
    } else {
        let n = num(v.strip_prefix('=').unwrap_or(&v))?;
        SizeRange { min: n, max: n.saturating_add(1) }
    };
    Ok(r)
}

/// `2026-11-01`、`2026/11/1`、`2026.11.01`：一天；`2026-11`：整個月；`2026`：整年。回傳 [第一天, 最後一天]。
fn parse_date_span(s: &str) -> Option<(Ymd, Ymd)> {
    let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
    let n = |p: &str| -> Option<u32> { (!p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok()).flatten() };
    match parts.as_slice() {
        [y] if y.len() == 4 => {
            let y = n(y)? as i32;
            Some((Ymd::new(y, 1, 1)?, Ymd::new(y, 12, 31)?))
        }
        [y, m] if y.len() == 4 => {
            let (y, m) = (n(y)? as i32, n(m)?);
            Some((Ymd::new(y, m, 1)?, Ymd::new(y, m, days_in_month(y, m))?))
        }
        [y, m, d] if y.len() == 4 => {
            let day = Ymd::new(n(y)? as i32, n(m)?, n(d)?)?;
            Some((day, day))
        }
        _ => None,
    }
}

fn parse_date_condition(raw: &str) -> Result<DateSpec, String> {
    let v = half_width(raw).to_lowercase();
    let bad = || {
        trf!(
            "看不懂的日期條件：dm:{raw}（例如 dm:today、dm:7d、dm:2026-11-01..2026-11-20）",
            "Can't read the date condition dm:{raw} (e.g. dm:today, dm:7d, dm:2026-11-01..2026-11-20)"
        )
    };
    let named = match v.as_str() {
        "today" | "今天" | "今日" => Some(DateSpec::Today),
        "yesterday" | "昨天" | "昨日" => Some(DateSpec::Yesterday),
        "thisweek" | "本週" | "本周" | "這週" | "这周" => Some(DateSpec::ThisWeek),
        "thismonth" | "本月" | "這個月" => Some(DateSpec::ThisMonth),
        "thisyear" | "今年" => Some(DateSpec::ThisYear),
        _ => None,
    };
    if let Some(d) = named {
        return Ok(d);
    }
    // 7d、2w、30天
    for (suffix, mult) in [("d", 1u32), ("天", 1), ("w", 7), ("週", 7), ("周", 7)] {
        if let Some(num) = v.strip_suffix(suffix) {
            if let Ok(n) = num.parse::<u32>() {
                if (1..=36500).contains(&n) {
                    return Ok(DateSpec::LastDays(n * mult));
                }
            }
        }
    }
    let span = |s: &str| parse_date_span(s).ok_or_else(bad);
    let r = if let Some(rest) = v.strip_prefix(">=") {
        DateSpec::Range { from: Some(span(rest)?.0), to: None }
    } else if let Some(rest) = v.strip_prefix("<=") {
        DateSpec::Range { from: None, to: Some(span(rest)?.1) }
    } else if let Some(rest) = v.strip_prefix('>') {
        DateSpec::Range { from: Some(span(rest)?.1.add_days(1)), to: None }
    } else if let Some(rest) = v.strip_prefix('<') {
        DateSpec::Range { from: None, to: Some(span(rest)?.0.add_days(-1)) }
    } else if let Some((a, b)) = split_range(&v) {
        let from = if a.is_empty() { None } else { Some(span(a)?.0) };
        let to = if b.is_empty() { None } else { Some(span(b)?.1) };
        if from.is_none() && to.is_none() {
            return Err(bad());
        }
        DateSpec::Range { from, to }
    } else {
        let (a, b) = span(&v)?;
        DateSpec::Range { from: Some(a), to: Some(b) }
    };
    Ok(r)
}

fn parse_exts(v: &str) -> Vec<String> {
    v.split([';', ',', '|', '；', '，'])
        .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty())
        .collect()
}

impl Query {
    /// 解析搜尋框的文字。看不懂的條件（`size:abc`）回傳說明要怎麼寫的訊息。
    pub fn parse(text: &str) -> Result<Query, String> {
        let mut q = Query::default();
        for tok in tokenize(text) {
            // 條件：認得的名稱加上冒號（半形或全形）；在引號裡的不算
            let op = if tok.quoted {
                None
            } else {
                tok.text.find([':', '：']).and_then(|i| {
                    let key = tok.text[..i].to_ascii_lowercase();
                    let colon = tok.text[i..].chars().next().map_or(1, char::len_utf8);
                    KEYS.contains(&key.as_str()).then(|| (key, tok.text[i + colon..].to_string()))
                })
            };
            let Some((key, value)) = op else {
                if tok.neg {
                    q.exclude.push(tok.text);
                } else {
                    q.terms.push(tok.text);
                }
                continue;
            };
            match key.as_str() {
                "ext" => {
                    let exts = parse_exts(&value);
                    if exts.is_empty() {
                        return Err(tr!("ext: 後面要寫副檔名，例如 ext:pdf;docx", "Write extensions after ext:, e.g. ext:pdf;docx").into());
                    }
                    if tok.neg {
                        q.exclude_exts.extend(exts);
                    } else {
                        q.exts.extend(exts);
                    }
                }
                "size" if !tok.neg => {
                    let r = parse_size_condition(&value)?;
                    q.size = Some(q.size.map_or(r, |old| old.intersect(r)));
                }
                "dm" | "datemodified" if !tok.neg => q.dates.push(parse_date_condition(&value)?),
                "folder" | "file" => {
                    let folders = (key == "folder") != tok.neg;
                    q.kind = if folders { Kind::Folders } else { Kind::Files };
                    // folder:src：只要資料夾，而且名稱有 src
                    if !value.trim().is_empty() {
                        q.terms.push(value);
                    }
                }
                _ => return Err(trf!("「-{key}:」不能排除，請改寫成範圍", "“-{key}:” can't be excluded; write a range instead")),
            }
        }
        Ok(q)
    }

    /// 檔名以外的條件（副檔名、大小、日期、只要資料夾或檔案）
    pub fn has_conditions(&self) -> bool {
        !self.exts.is_empty() || !self.exclude_exts.is_empty() || self.size.is_some() || !self.dates.is_empty() || self.kind != Kind::Any
    }

    /// 什麼都沒寫（只有空白）
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.exclude.is_empty() && !self.has_conditions()
    }
}

// ---------------------------------------------------------------- 搜尋執行緒用的條件

/// 日期已經換算好的條件：搜尋執行緒在建立項目之前先用它判斷（只看檔名、大小、時間與屬性）
pub struct Filter {
    terms: Vec<Pattern>,
    exclude: Vec<Pattern>,
    exts: Vec<Vec<u16>>,
    exclude_exts: Vec<Vec<u16>>,
    pub kind: Kind,
    pub size: Option<SizeRange>,
    pub time: Option<TimeRange>,
}

fn folded_units(s: &str) -> Vec<u16> {
    s.encode_utf16().map(fold).collect()
}

/// 檔名最後一個句點之後（不含句點）；沒有副檔名時是空的
fn ext_of(name: &[u16]) -> &[u16] {
    match name.iter().rposition(|c| *c == b'.' as u16) {
        Some(i) if i + 1 < name.len() => &name[i + 1..],
        _ => &[],
    }
}

fn same_folded(a: &[u16], folded: &[u16]) -> bool {
    a.len() == folded.len() && a.iter().zip(folded).all(|(x, y)| fold(*x) == *y)
}

impl Filter {
    /// `time`：搜尋框的日期條件換算好的範圍（多個時已經取交集）
    pub fn new(q: &Query, time: Option<TimeRange>) -> Filter {
        Filter {
            terms: q.terms.iter().filter_map(|t| Pattern::new(t)).collect(),
            exclude: q.exclude.iter().filter_map(|t| Pattern::new(t)).collect(),
            exts: q.exts.iter().map(|e| folded_units(e)).collect(),
            exclude_exts: q.exclude_exts.iter().map(|e| folded_units(e)).collect(),
            kind: q.kind,
            size: q.size,
            time,
        }
    }

    /// 再加上篩選籤的大小、日期條件（和搜尋框的條件取交集）
    pub fn and(mut self, size: Option<SizeRange>, time: Option<TimeRange>) -> Filter {
        if let Some(s) = size {
            self.size = Some(self.size.map_or(s, |old| old.intersect(s)));
        }
        if let Some(t) = time {
            self.time = Some(self.time.map_or(t, |old| old.intersect(t)));
        }
        self
    }

    /// 什麼條件都沒有：每個項目都符合（搜尋時要求至少有一個條件）
    pub fn is_unrestricted(&self) -> bool {
        self.terms.is_empty() && self.exclude.is_empty() && self.exts.is_empty() && self.exclude_exts.is_empty() && self.kind == Kind::Any && self.size.is_none() && self.time.is_none()
    }

    /// `name` 是 UTF-16 檔名（不含結尾的 0）；`modified` 是 FILETIME（UTC）。不配置記憶體。
    pub fn matches(&self, name: &[u16], is_dir: bool, size: u64, modified: u64) -> bool {
        match self.kind {
            Kind::Folders if !is_dir => return false,
            Kind::Files if is_dir => return false,
            _ => {}
        }
        if !self.exts.is_empty() || !self.exclude_exts.is_empty() {
            let ext = if is_dir { &[][..] } else { ext_of(name) };
            if !self.exts.is_empty() && (is_dir || !self.exts.iter().any(|e| same_folded(ext, e))) {
                return false;
            }
            if self.exclude_exts.iter().any(|e| same_folded(ext, e)) {
                return false;
            }
        }
        // 資料夾沒有大小：有大小條件時不顯示資料夾（同檔案總管）
        if let Some(r) = self.size {
            if is_dir || !r.contains(size) {
                return false;
            }
        }
        if let Some(t) = self.time {
            if !t.contains(modified) {
                return false;
            }
        }
        crate::namematch::all_match(&self.terms, name) && !self.exclude.iter().any(|p| p.matches(name))
    }
}

// ---------------------------------------------------------------- 相關性

/// 相關性的等級：檔名（不算副檔名）完全相同、開頭相同、在詞的開頭出現、中間有出現
pub const LEVEL_EXACT: u32 = 0;
pub const LEVEL_PREFIX: u32 = 1;
pub const LEVEL_WORD: u32 = 2;
pub const LEVEL_CONTAINS: u32 = 3;

/// 算搜尋結果的相關性分數（越小越前面）：等級在高位、路徑深度在低位。
/// 同分時由排序再比修改日期（新的在前）與名稱。
pub struct Scorer {
    /// 沒有萬用字元的詞（已轉小寫）；萬用字元的詞不影響等級
    needles: Vec<Vec<u16>>,
}

fn is_separator(c: u16) -> bool {
    matches!(char::from_u32(c as u32), Some(' ' | '_' | '-' | '.' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '+' | '&' | '@' | '#' | '!' | '~' | '\'' | '　' | '（' | '）' | '【' | '】' | '「' | '」' | '・' | '、' | '，'))
}

fn is_ascii_alnum(c: u16) -> bool {
    c < 0x80 && (c as u8).is_ascii_alphanumeric()
}

/// `name[i]` 是不是一個詞的開頭：最前面、前一個是分隔符號、小寫接大寫（camelCase）、字母和數字交界、
/// 英數和中日韓文字交界
fn word_start(name: &[u16], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let (p, c) = (name[i - 1], name[i]);
    if is_separator(p) {
        return true;
    }
    let lower = |x: u16| (b'a' as u16..=b'z' as u16).contains(&x);
    let upper = |x: u16| (b'A' as u16..=b'Z' as u16).contains(&x);
    let digit = |x: u16| (b'0' as u16..=b'9' as u16).contains(&x);
    (lower(p) && upper(c)) || (digit(p) != digit(c) && is_ascii_alnum(p) && is_ascii_alnum(c)) || (is_ascii_alnum(p) != is_ascii_alnum(c) && !is_separator(c))
}

impl Scorer {
    pub fn new(q: &Query) -> Scorer {
        Scorer { needles: q.terms.iter().map(|t| t.trim()).filter(|t| !t.is_empty() && !t.contains(['*', '?'])).map(folded_units).collect() }
    }

    /// 檔名對所有詞中最差的那一級（每個詞都要至少這麼好）；沒有詞時都是 LEVEL_EXACT
    pub fn level(&self, name: &str, is_dir: bool) -> u32 {
        if self.needles.is_empty() {
            return LEVEL_EXACT;
        }
        let units: Vec<u16> = name.encode_utf16().collect();
        let folded: Vec<u16> = units.iter().map(|c| fold(*c)).collect();
        // 不算副檔名的部分（資料夾、以句點開頭的名稱用整個名稱）
        let stem_len = match (is_dir, units.iter().rposition(|c| *c == b'.' as u16)) {
            (false, Some(i)) if i > 0 => i,
            _ => units.len(),
        };
        self.needles
            .iter()
            .map(|n| {
                if n.is_empty() || n.len() > folded.len() {
                    return LEVEL_CONTAINS;
                }
                if folded[..stem_len] == n[..] {
                    return LEVEL_EXACT;
                }
                if folded.starts_with(n) {
                    return LEVEL_PREFIX;
                }
                let mut best = LEVEL_CONTAINS;
                for i in 0..=folded.len() - n.len() {
                    if folded[i..i + n.len()] == n[..] && word_start(&units, i) {
                        best = LEVEL_WORD;
                        break;
                    }
                }
                best
            })
            .max()
            .unwrap_or(LEVEL_EXACT)
    }

    /// 分數：等級 × 2^20 + 路徑深度（所在資料夾的層數，越淺越前面）
    pub fn score(&self, name: &str, is_dir: bool, dir: &str) -> u32 {
        let depth = dir.trim_end_matches(['\\', '/']).matches(['\\', '/']).count().min(0xF_FFFF) as u32;
        (self.level(name, is_dir) << 20) | depth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> Query {
        Query::parse(text).unwrap()
    }

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn terms_quotes_and_exclusions() {
        let x = q("報告 2025");
        assert_eq!(x.terms, ["報告", "2025"]);
        let x = q("\"年度 報告\" -草稿");
        assert_eq!(x.terms, ["年度 報告"]);
        assert_eq!(x.exclude, ["草稿"]);
        // 全形引號、全形空白、排除整串
        let x = q("「年度　報告」　-“舊 版”");
        assert_eq!(x.terms, ["年度　報告"]);
        assert_eq!(x.exclude, ["舊 版"]);
        assert!(q("   ").is_empty());
        // 單獨的減號是一般的詞
        assert_eq!(q("a - b").terms, ["a", "-", "b"]);
    }

    #[test]
    fn conditions_accept_full_width_colon() {
        let x = q("ext：pdf;DOCX 報告");
        assert_eq!(x.exts, ["pdf", "docx"]);
        assert_eq!(x.terms, ["報告"]);
        let x = q("EXT:.pdf,*.txt -ext:tmp");
        assert_eq!(x.exts, ["pdf", "txt"]);
        assert_eq!(x.exclude_exts, ["tmp"]);
        // 檔名裡的全形冒號不是條件
        assert_eq!(q("藍色星原：旅謠").terms, ["藍色星原：旅謠"]);
        // 引號裡的不是條件
        assert_eq!(q("\"ext:pdf\"").terms, ["ext:pdf"]);
        assert_eq!(q("folder:").kind, Kind::Folders);
        let x = q("file： 報告");
        assert_eq!(x.kind, Kind::Files);
        assert_eq!(x.terms, ["報告"]);
        let x = q("folder:src");
        assert_eq!((x.kind, x.terms.clone()), (Kind::Folders, vec!["src".to_string()]));
        assert_eq!(q("-folder:").kind, Kind::Files);
        assert!(q("ext:pdf").has_conditions() && !q("ext:pdf").is_empty());
        assert!(Query::parse("ext:").is_err());
        assert!(Query::parse("-size:>1mb").is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("100mb"), Some(100 * MB));
        assert_eq!(parse_size("1.5 GB"), Some(GB + GB / 2));
        assert_eq!(parse_size("512k"), Some(512 * KB));
        assert_eq!(parse_size("2048"), Some(2048));
        assert_eq!(parse_size("１０MB".chars().map(|c| if ('０'..='９').contains(&c) { char::from_u32(c as u32 - 0xFEE0).unwrap() } else { c }).collect::<String>().as_str()), Some(10 * MB));
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("1.2.3mb"), None);
        assert_eq!(q("size:>100mb").size, Some(SizeRange { min: 100 * MB + 1, max: u64::MAX }));
        assert_eq!(q("size:>=100mb").size, Some(SizeRange { min: 100 * MB, max: u64::MAX }));
        assert_eq!(q("size:<1mb").size, Some(SizeRange { min: 0, max: MB }));
        assert_eq!(q("size:1mb..10mb").size, Some(SizeRange { min: MB, max: 10 * MB + 1 }));
        assert_eq!(q("size：１mb～10mb").size, Some(SizeRange { min: MB, max: 10 * MB + 1 }));
        assert_eq!(q("size:10mb..").size, Some(SizeRange { min: 10 * MB, max: u64::MAX }));
        assert_eq!(q("size:empty").size, Some(SizeRange { min: 0, max: 1 }));
        // 兩個大小條件取交集
        assert_eq!(q("size:>1mb size:<10mb").size, Some(SizeRange { min: MB + 1, max: 10 * MB }));
        let err = Query::parse("size:abc").unwrap_err();
        assert!(err.contains("size:abc"), "{err}");
    }

    #[test]
    fn size_labels() {
        let labels: Vec<String> = SIZE_PRESETS.iter().map(|r| r.label()).collect();
        assert_eq!(labels, ["< 1 MB", "1～100 MB", "100 MB～1 GB", "> 1 GB"]);
        assert_eq!(format_bytes(1536 * MB), "1.5 GB");
        assert_eq!(format_bytes(10), "10 B");
        assert_eq!(SizeRange { min: 10 * MB, max: u64::MAX }.label(), "> 10 MB");
    }

    #[test]
    fn calendar() {
        let d = Ymd::new(2026, 11, 26).unwrap();
        assert_eq!(d.weekday(), 3); // 星期四
        assert_eq!(Ymd::new(1970, 1, 1).unwrap().weekday(), 3);
        assert_eq!(Ymd::new(2024, 2, 29).unwrap().add_days(1), Ymd::new(2024, 3, 1).unwrap());
        assert_eq!(Ymd::new(2026, 1, 1).unwrap().add_days(-1), Ymd::new(2025, 12, 31).unwrap());
        assert!(Ymd::new(2025, 2, 29).is_none());
        for n in [-1000i64, -1, 0, 1, 59, 365, 10_000] {
            let x = d.add_days(n);
            assert_eq!(x.add_days(-n), d);
        }
    }

    #[test]
    fn date_ranges_follow_local_days() {
        let today = Ymd::new(2026, 11, 26).unwrap(); // 星期四
        let day = |m, d| Ymd::new(2026, m, d);
        assert_eq!(DateSpec::Today.local_days(today, 0), (day(11, 26), day(11, 27)));
        assert_eq!(DateSpec::Yesterday.local_days(today, 0), (day(11, 25), day(11, 26)));
        assert_eq!(DateSpec::LastDays(7).local_days(today, 0), (day(11, 20), day(11, 27)));
        // 本週：一週從星期一開始，或（美國等）從星期日開始
        assert_eq!(DateSpec::ThisWeek.local_days(today, 0), (day(11, 23), day(11, 30)));
        assert_eq!(DateSpec::ThisWeek.local_days(today, 6), (day(11, 22), day(11, 29)));
        assert_eq!(DateSpec::ThisMonth.local_days(today, 0), (day(11, 1), day(12, 1)));
        assert_eq!(DateSpec::ThisYear.local_days(today, 0), (day(1, 1), Ymd::new(2027, 1, 1)));
        let range = DateSpec::Range { from: day(11, 20), to: day(11, 1) };
        assert_eq!(range.local_days(today, 0), (day(11, 1), day(11, 21)));
    }

    #[test]
    fn date_conditions() {
        let day = |m, d| Ymd::new(2026, m, d);
        assert_eq!(q("dm:today").dates, [DateSpec::Today]);
        assert_eq!(q("datemodified：本週").dates, [DateSpec::ThisWeek]);
        assert_eq!(q("dm:7d").dates, [DateSpec::LastDays(7)]);
        assert_eq!(q("dm:2w").dates, [DateSpec::LastDays(14)]);
        assert_eq!(q("dm:2026-11-01..2026-11-20").dates, [DateSpec::Range { from: day(11, 1), to: day(11, 20) }]);
        assert_eq!(q("dm:2026/11/1").dates, [DateSpec::Range { from: day(11, 1), to: day(11, 1) }]);
        assert_eq!(q("dm:2026-11").dates, [DateSpec::Range { from: day(11, 1), to: day(11, 30) }]);
        assert_eq!(q("dm:>2026-11-01").dates, [DateSpec::Range { from: day(11, 2), to: None }]);
        assert_eq!(q("dm:<2026-11-01").dates, [DateSpec::Range { from: None, to: day(10, 31) }]);
        assert_eq!(q("dm:2026-11-01..").dates, [DateSpec::Range { from: day(11, 1), to: None }]);
        assert!(Query::parse("dm:2026-13-01").is_err());
        assert!(Query::parse("dm:soon").is_err());
        assert_eq!(DateSpec::Range { from: day(11, 1), to: day(11, 20) }.label(), "2026-11-01～2026-11-20");
    }

    #[test]
    fn filter_checks_everything_without_entries() {
        let f = Filter::new(&q("報告 ext:pdf;docx -草稿"), None);
        assert!(f.matches(&units("2025 年度報告.PDF"), false, 10, 0));
        assert!(!f.matches(&units("年度報告 草稿.pdf"), false, 10, 0));
        assert!(!f.matches(&units("年度報告.txt"), false, 10, 0));
        // 資料夾沒有副檔名
        assert!(!f.matches(&units("報告.pdf"), true, 0, 0));
        // 大小條件不顯示資料夾
        let f = Filter::new(&q("size:>1mb"), None);
        assert!(f.matches(&units("big.iso"), false, 2 * MB, 0));
        assert!(!f.matches(&units("small.txt"), false, 10, 0));
        assert!(!f.matches(&units("folder"), true, 0, 0));
        // 篩選籤的條件取交集
        let f = Filter::new(&q(""), Some(TimeRange { from: 100, to: 200 })).and(Some(SizeRange { min: 0, max: 50 }), Some(TimeRange { from: 150, to: 300 }));
        assert!(f.matches(&units("a"), false, 10, 160));
        assert!(!f.matches(&units("a"), false, 10, 120));
        assert!(!f.matches(&units("a"), false, 60, 160));
        assert!(Filter::new(&q(""), None).is_unrestricted() && !Filter::new(&q("a"), None).is_unrestricted());
        let f = Filter::new(&q("folder:"), None);
        assert!(f.matches(&units("src"), true, 0, 0) && !f.matches(&units("a.txt"), false, 0, 0));
        let f = Filter::new(&q("*.pdf"), None);
        assert!(f.matches(&units("a.PDF"), false, 0, 0) && !f.matches(&units("a.pdf.bak"), false, 0, 0));
    }

    #[test]
    fn relevance_levels() {
        let s = Scorer::new(&q("report"));
        assert_eq!(s.level("Report.pdf", false), LEVEL_EXACT);
        assert_eq!(s.level("report", true), LEVEL_EXACT);
        assert_eq!(s.level("Report 2025.pdf", false), LEVEL_PREFIX);
        assert_eq!(s.level("Q3 report.pdf", false), LEVEL_WORD);
        assert_eq!(s.level("annualReport.xlsx", false), LEVEL_WORD);
        assert_eq!(s.level("2025report.pdf", false), LEVEL_WORD);
        assert_eq!(s.level("misreported.txt", false), LEVEL_CONTAINS);
        // 多個詞：取最差的那個
        let s = Scorer::new(&q("報告 2025"));
        assert_eq!(s.level("2025 報告.pdf", false), LEVEL_WORD);
        assert_eq!(s.level("年度報告2025.pdf", false), LEVEL_CONTAINS);
        // 沒有詞（只有條件）：都一樣，交給深度與日期
        assert_eq!(Scorer::new(&q("ext:pdf")).level("x.pdf", false), LEVEL_EXACT);
        // 淺的在前
        let s = Scorer::new(&q("a"));
        assert!(s.score("a.txt", false, "C:\\x") < s.score("a.txt", false, "C:\\x\\y\\z"));
        assert!(s.score("a.txt", false, "C:\\x\\y\\z") < s.score("ba.txt", false, "C:\\"));
    }
}

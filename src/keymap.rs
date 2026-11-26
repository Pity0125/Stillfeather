//! 快速鍵表：每一筆是「按鍵、修飾鍵、在哪裡有效、對應的命令」。
//!
//! 主視窗的按鍵處理（`src/win/app/keys.rs` 的 `handle_key`）查這張表，說明裡的快速鍵一覽也從這張表產生，
//! 兩邊不會對不起來。按住不放的空白鍵預覽、輸入框裡的 Enter／Esc、Tab 換焦點、清單的打字跳到項目、
//! 選擇器與麵包屑選單自己的按鍵不在表裡（要看按下、放開或輸入法的狀態）。
//!
//! 修飾鍵必須完全相同才算（Ctrl+Shift+W 不等於 Ctrl+W）。單元測試確認同一個地方沒有兩筆一樣的組合。
//!
//! 預設的表是 [`BINDINGS`]；主視窗用的是 [`Keymap`]（預設就是這張表），之後開放自訂快速鍵時以
//! [`Keymap::with_overrides`] 套上使用者的設定：命令用不會變的英文代號（[`Cmd::id`]，例如 `new-tab`），
//! 按鍵用說明裡的寫法（`Ctrl+Shift+T`，見 [`parse_combo`]），衝突在 [`Keymap::conflicts`] 一次列出來。

/// 虛擬按鍵碼（與 Win32 的 VK_* 相同；核心不依賴 Windows 的宣告）
pub mod vk {
    pub const BACK: u32 = 0x08;
    pub const TAB: u32 = 0x09;
    pub const RETURN: u32 = 0x0D;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const PRIOR: u32 = 0x21;
    pub const NEXT: u32 = 0x22;
    pub const END: u32 = 0x23;
    pub const HOME: u32 = 0x24;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
    pub const INSERT: u32 = 0x2D;
    pub const DELETE: u32 = 0x2E;
    pub const F1: u32 = 0x70;
    pub const F2: u32 = 0x71;
    pub const F5: u32 = 0x74;
    pub const F12: u32 = 0x7B;
    pub const F24: u32 = 0x87;
    pub const BROWSER_BACK: u32 = 0xA6;
    pub const BROWSER_FORWARD: u32 = 0xA7;
    pub const OEM_COMMA: u32 = 0xBC;
    /// 美式鍵盤的 \ 與 | 鍵
    pub const OEM_5: u32 = 0xDC;

    pub const fn letter(c: u8) -> u32 {
        c as u32
    }

    pub const fn digit(d: u8) -> u32 {
        0x30 + d as u32
    }
}

/// 在哪裡有效
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// 整個主視窗，包括路徑框與搜尋框
    Global,
    /// 不在路徑框或搜尋框裡（那裡的 Ctrl+C、Ctrl+Z 交給輸入框自己）
    NotEdit,
    /// 焦點在檔案清單
    List,
}

/// 命令
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    /// 跳到第 N 個分頁（1 起算）；9 是最後一個
    GoToTab(u8),
    MoveTabLeft,
    MoveTabRight,
    /// 重新開啟剛關掉的分頁
    ReopenTab,
    ToggleDual,
    /// 舊的切換雙欄快速鍵（Ctrl+Shift+D）：什麼都不做，只提示改成 Ctrl+\。保留一兩個版本後拿掉
    OldDualShortcut,
    /// 切換檢視（1 超大圖示…8 內容）
    View(u8),
    Settings,
    EditPath,
    /// 舊的編輯路徑快速鍵（Ctrl+L）：什麼都不做，只提示改成 Ctrl+E。保留一兩個版本後拿掉
    OldPathShortcut,
    Search,
    Refresh,
    Back,
    Forward,
    Up,
    Properties,
    Undo,
    Copy,
    Cut,
    Paste,
    CopyPath,
    NewFolder,
    SelectAll,
    Open,
    Rename,
    Delete,
    DeletePermanently,
    /// 說明頁
    Help,
}

impl Cmd {
    /// 只有主視窗有的功能（分頁、雙欄、設定）：選擇器（其他程式的開檔視窗）裡按了什麼都不做
    pub fn main_window_only(self) -> bool {
        matches!(self, NewTab | CloseTab | NextTab | PrevTab | GoToTab(_) | MoveTabLeft | MoveTabRight | ReopenTab | ToggleDual | OldDualShortcut | Settings)
    }

    /// 可以自訂的命令（舊快速鍵的提示不算）
    pub const CUSTOMIZABLE: &'static [Cmd] = &[
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        GoToTab(1),
        GoToTab(2),
        GoToTab(3),
        GoToTab(4),
        GoToTab(5),
        GoToTab(6),
        GoToTab(7),
        GoToTab(8),
        GoToTab(9),
        MoveTabLeft,
        MoveTabRight,
        ReopenTab,
        ToggleDual,
        View(1),
        View(2),
        View(3),
        View(4),
        View(5),
        View(6),
        View(7),
        View(8),
        Settings,
        EditPath,
        Search,
        Refresh,
        Back,
        Forward,
        Up,
        Properties,
        Undo,
        Copy,
        Cut,
        Paste,
        CopyPath,
        NewFolder,
        SelectAll,
        Open,
        Rename,
        Delete,
        DeletePermanently,
        Help,
    ];

    /// 不會變的英文代號（設定檔用）：`new-tab`、`go-to-tab-3`、`view-2`……
    pub fn section(self) -> Section { section_of(group(self)) }

    pub fn display_name(self) -> String {
        match self {
            NextTab => tr!("下一個分頁", "Next tab").into(),
            PrevTab => tr!("上一個分頁", "Previous tab").into(),
            GoToTab(n) => trf!("跳到分頁 {n}", "Go to tab {n}"),
            MoveTabLeft => tr!("分頁向左移", "Move tab left").into(),
            MoveTabRight => tr!("分頁向右移", "Move tab right").into(),
            View(n) => trf!("檢視模式 {n}", "View mode {n}"),
            Back => tr!("上一頁", "Back").into(),
            Forward => tr!("下一頁", "Forward").into(),
            Up => tr!("上一層", "Up").into(),
            Copy => tr!("複製", "Copy").into(),
            Cut => tr!("剪下", "Cut").into(),
            Paste => tr!("貼上", "Paste").into(),
            _ => group_text(group(self)).into(),
        }
    }

    pub fn id(self) -> String {
        let fixed = match self {
            NewTab => "new-tab",
            CloseTab => "close-tab",
            NextTab => "next-tab",
            PrevTab => "previous-tab",
            GoToTab(n) => return format!("go-to-tab-{n}"),
            MoveTabLeft => "move-tab-left",
            MoveTabRight => "move-tab-right",
            ReopenTab => "reopen-tab",
            ToggleDual => "toggle-dual-pane",
            OldDualShortcut => "old-dual-pane-shortcut",
            View(n) => return format!("view-{n}"),
            Settings => "settings",
            EditPath => "edit-path",
            OldPathShortcut => "old-edit-path-shortcut",
            Search => "search",
            Refresh => "refresh",
            Back => "back",
            Forward => "forward",
            Up => "up",
            Properties => "properties",
            Undo => "undo",
            Copy => "copy",
            Cut => "cut",
            Paste => "paste",
            CopyPath => "copy-path",
            NewFolder => "new-folder",
            SelectAll => "select-all",
            Open => "open",
            Rename => "rename",
            Delete => "delete",
            DeletePermanently => "delete-permanently",
            Help => "help",
        };
        fixed.to_string()
    }

    /// 由英文代號找回可以自訂的命令（不分大小寫）
    pub fn from_id(id: &str) -> Option<Cmd> {
        let id = id.trim().to_ascii_lowercase();
        Cmd::CUSTOMIZABLE.iter().copied().find(|c| c.id() == id)
    }

    /// 自訂的按法放在哪裡有效：沿用預設按法的範圍（刪除、重新命名只在清單上）
    fn scope(self) -> Scope {
        BINDINGS.iter().find(|b| b.cmd == self).map_or(Global, |b| b.scope)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Binding {
    pub key: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub scope: Scope,
    pub cmd: Cmd,
    /// 列在說明的快速鍵一覽
    pub help: bool,
}

const fn b(key: u32, ctrl: bool, shift: bool, alt: bool, scope: Scope, cmd: Cmd) -> Binding {
    Binding { key, ctrl, shift, alt, scope, cmd, help: true }
}

/// 不列在說明裡的（同一個命令的其他按法，例如鍵盤上的「上一頁」鍵）
const fn quiet(key: u32, ctrl: bool, shift: bool, alt: bool, scope: Scope, cmd: Cmd) -> Binding {
    Binding { key, ctrl, shift, alt, scope, cmd, help: false }
}

use vk::*;
use Cmd::*;
use Scope::*;

/// 快速鍵表。順序就是說明裡的順序（同一個命令的幾種按法合成一列）。
pub const BINDINGS: &[Binding] = &[
    b(letter(b'T'), true, false, false, Global, NewTab),
    b(letter(b'N'), true, false, false, Global, NewTab),
    b(letter(b'W'), true, false, false, Global, CloseTab),
    b(TAB, true, false, false, Global, NextTab),
    b(NEXT, true, false, false, Global, NextTab),
    b(TAB, true, true, false, Global, PrevTab),
    b(PRIOR, true, false, false, Global, PrevTab),
    b(digit(1), true, false, false, Global, GoToTab(1)),
    b(digit(2), true, false, false, Global, GoToTab(2)),
    b(digit(3), true, false, false, Global, GoToTab(3)),
    b(digit(4), true, false, false, Global, GoToTab(4)),
    b(digit(5), true, false, false, Global, GoToTab(5)),
    b(digit(6), true, false, false, Global, GoToTab(6)),
    b(digit(7), true, false, false, Global, GoToTab(7)),
    b(digit(8), true, false, false, Global, GoToTab(8)),
    b(digit(9), true, false, false, Global, GoToTab(9)),
    b(PRIOR, true, true, false, Global, MoveTabLeft),
    b(NEXT, true, true, false, Global, MoveTabRight),
    b(letter(b'T'), true, true, false, Global, ReopenTab),
    b(OEM_5, true, false, false, Global, ToggleDual),
    quiet(letter(b'D'), true, true, false, Global, OldDualShortcut),
    b(digit(1), true, true, false, Global, View(1)),
    b(digit(2), true, true, false, Global, View(2)),
    b(digit(3), true, true, false, Global, View(3)),
    b(digit(4), true, true, false, Global, View(4)),
    b(digit(5), true, true, false, Global, View(5)),
    b(digit(6), true, true, false, Global, View(6)),
    b(digit(7), true, true, false, Global, View(7)),
    b(digit(8), true, true, false, Global, View(8)),
    b(letter(b'E'), true, false, false, Global, EditPath),
    quiet(letter(b'L'), true, false, false, Global, OldPathShortcut),
    b(letter(b'D'), false, false, true, Global, EditPath),
    b(letter(b'F'), true, false, false, Global, Search),
    b(LEFT, false, false, true, Global, Back),
    quiet(BROWSER_BACK, false, false, false, Global, Back),
    b(RIGHT, false, false, true, Global, Forward),
    quiet(BROWSER_FORWARD, false, false, false, Global, Forward),
    b(UP, false, false, true, Global, Up),
    quiet(BACK, false, false, false, List, Back),
    b(F5, false, false, false, Global, Refresh),
    b(letter(b'Z'), true, false, false, NotEdit, Undo),
    quiet(RETURN, false, false, false, List, Open),
    b(F2, false, false, false, List, Rename),
    b(DELETE, false, false, false, List, Delete),
    b(letter(b'D'), true, false, false, List, Delete),
    b(DELETE, false, true, false, List, DeletePermanently),
    b(letter(b'C'), true, false, false, NotEdit, Copy),
    b(letter(b'X'), true, false, false, NotEdit, Cut),
    b(letter(b'V'), true, false, false, NotEdit, Paste),
    b(letter(b'C'), true, true, false, NotEdit, CopyPath),
    b(letter(b'N'), true, true, false, NotEdit, NewFolder),
    quiet(letter(b'A'), true, false, false, List, SelectAll),
    b(RETURN, false, false, true, NotEdit, Properties),
    b(F12, false, false, false, Global, Settings),
    b(OEM_COMMA, true, false, false, Global, Settings),
    b(F1, false, false, false, Global, Help),
];

/// 查預設的表：按下的鍵在目前的位置對應哪個命令。`in_edit`：焦點在路徑框或搜尋框；`in_list`：焦點在檔案清單。
pub fn lookup(key: u32, ctrl: bool, shift: bool, alt: bool, in_edit: bool, in_list: bool) -> Option<Cmd> {
    lookup_in(BINDINGS, key, ctrl, shift, alt, in_edit, in_list)
}

fn lookup_in(bindings: &[Binding], key: u32, ctrl: bool, shift: bool, alt: bool, in_edit: bool, in_list: bool) -> Option<Cmd> {
    bindings
        .iter()
        .find(|b| {
            b.key == key
                && b.ctrl == ctrl
                && b.shift == shift
                && b.alt == alt
                && match b.scope {
                    Global => true,
                    NotEdit => !in_edit,
                    List => in_list,
                }
        })
        .map(|b| b.cmd)
}

/// 有名字的按鍵（字母、數字與 F1～F24 另外處理）。第一個是說明裡的寫法，後面是解析時也接受的別名。
const KEY_NAMES: &[(u32, &[&str])] = &[
    (BACK, &["Backspace"]),
    (TAB, &["Tab"]),
    (RETURN, &["Enter", "Return"]),
    (ESCAPE, &["Esc", "Escape"]),
    (SPACE, &["Space"]),
    (PRIOR, &["PgUp", "PageUp"]),
    (NEXT, &["PgDn", "PageDown"]),
    (END, &["End"]),
    (HOME, &["Home"]),
    (LEFT, &["←", "Left"]),
    (UP, &["↑", "Up"]),
    (RIGHT, &["→", "Right"]),
    (DOWN, &["↓", "Down"]),
    (INSERT, &["Insert", "Ins"]),
    (DELETE, &["Delete", "Del"]),
    (OEM_COMMA, &[","]),
    (OEM_5, &["\\"]),
    (BROWSER_BACK, &["BrowserBack"]),
    (BROWSER_FORWARD, &["BrowserForward"]),
];

fn key_name(key: u32) -> String {
    if let Some((_, names)) = KEY_NAMES.iter().find(|(k, _)| *k == key) {
        return names[0].to_string();
    }
    match key {
        F1..=F24 => format!("F{}", key - F1 + 1),
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(key).map(String::from).unwrap_or_default(),
        _ => format!("0x{key:02X}"),
    }
}

/// 一組按鍵：按鍵與修飾鍵
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub key: u32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// 「Ctrl+Shift+T」「Alt+←」「F5」「ctrl + pgdn」：說明裡的寫法（不分大小寫、可以有空白）。認不得時回傳 None。
pub fn parse_combo(text: &str) -> Option<Combo> {
    let parts: Vec<String> = text.split('+').map(|p| p.trim().to_string()).collect();
    // 「Ctrl++」：最後一個是加號鍵（不支援），空的部分不合法
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (last, mods) = parts.split_last()?;
    let mut c = Combo { key: 0, ctrl: false, shift: false, alt: false };
    for m in mods {
        let flag = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => &mut c.ctrl,
            "shift" => &mut c.shift,
            "alt" => &mut c.alt,
            _ => return None,
        };
        if *flag {
            return None; // 同一個修飾鍵寫兩次
        }
        *flag = true;
    }
    let lower = last.to_lowercase();
    c.key = if let Some((k, _)) = KEY_NAMES.iter().find(|(_, names)| names.iter().any(|n| n.to_lowercase() == lower)) {
        *k
    } else if last.chars().count() == 1 && last.chars().all(|ch| ch.is_ascii_alphanumeric()) {
        last.to_ascii_uppercase().chars().next()? as u32
    } else {
        // F1～F24
        F1 + lower.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()).filter(|n| (1..=24).contains(n))? - 1
    };
    Some(c)
}

impl Combo {
    /// 不能拿來自訂的按法：沒有 Ctrl 或 Alt 的字母、數字、空白鍵、Esc、Tab 與標點（打字跳到項目、空白鍵預覽、
    /// 離開輸入框、換焦點要用）
    pub fn reserved(&self) -> bool {
        !self.ctrl && !self.alt && (matches!(self.key, 0x30..=0x39 | 0x41..=0x5A | SPACE | ESCAPE | TAB | OEM_COMMA | OEM_5))
    }

    pub fn label(&self) -> String {
        label(&Binding { key: self.key, ctrl: self.ctrl, shift: self.shift, alt: self.alt, scope: Global, cmd: Refresh, help: true })
    }
}

/// 一筆快速鍵的寫法，例如「Ctrl+Shift+D」
pub fn label(b: &Binding) -> String {
    let mut s = String::new();
    if b.ctrl {
        s.push_str("Ctrl+");
    }
    if b.shift {
        s.push_str("Shift+");
    }
    if b.alt {
        s.push_str("Alt+");
    }
    s.push_str(&key_name(b.key));
    s
}

/// 說明裡一列的分組：同一組的命令合成一列
fn group(cmd: Cmd) -> u8 {
    match cmd {
        NewTab => 1,
        CloseTab => 2,
        NextTab | PrevTab => 3,
        GoToTab(_) => 4,
        MoveTabLeft | MoveTabRight => 5,
        ReopenTab => 6,
        ToggleDual | OldDualShortcut => 7,
        View(_) => 8,
        EditPath | OldPathShortcut => 9,
        Search => 10,
        Back | Forward | Up => 11,
        Refresh => 12,
        Undo => 13,
        Open => 14,
        Rename => 15,
        Delete => 16,
        DeletePermanently => 17,
        Copy | Cut | Paste => 18,
        CopyPath => 19,
        NewFolder => 20,
        SelectAll => 21,
        Properties => 22,
        Settings => 23,
        Help => 24,
    }
}

/// 說明頁的快速鍵一覽分成幾段（依說明一列的分組）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Tabs,
    View,
    Navigate,
    Files,
    Other,
}

fn section_of(group: u8) -> Section {
    match group {
        1..=7 => Section::Tabs,
        8 => Section::View,
        9..=12 => Section::Navigate,
        13..=22 => Section::Files,
        _ => Section::Other,
    }
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Tabs => tr!("分頁與雙欄", "Tabs and panes"),
            Section::View => tr!("檢視", "View"),
            Section::Navigate => tr!("移動與搜尋", "Getting around"),
            Section::Files => tr!("檔案與資料夾", "Files and folders"),
            Section::Other => tr!("其他", "Other"),
        }
    }
}

fn group_text(group: u8) -> &'static str {
    match group {
        1 => tr!("新分頁", "New tab"),
        2 => tr!("關閉分頁（分頁上按中鍵也可以）", "Close tab (or middle-click the tab)"),
        3 => tr!("下一個／上一個分頁", "Next/previous tab"),
        4 => tr!("跳到第 1～8 個分頁，Ctrl+9 是最後一個", "Go to tab 1–8; Ctrl+9 goes to the last tab"),
        5 => tr!("調整分頁順序（也可以直接拖曳分頁）", "Reorder tabs (or drag a tab)"),
        6 => tr!("重新開啟剛關掉的分頁", "Reopen the tab you just closed"),
        7 => tr!("單欄／雙欄", "Single/dual pane"),
        8 => tr!("超大圖示、大圖示…詳細資料、並排、內容", "Extra large icons, large icons… details, tiles, content"),
        9 => tr!("編輯路徑", "Edit path"),
        10 => tr!("搜尋檔名", "Search file names"),
        11 => tr!("上一頁／下一頁／上一層", "Back/forward/up"),
        12 => tr!("重新整理", "Refresh"),
        13 => tr!("復原重新命名、移動或刪除", "Undo rename, move or delete"),
        14 => tr!("開啟", "Open"),
        15 => tr!("重新命名", "Rename"),
        16 => tr!("刪除到資源回收筒", "Delete to Recycle Bin"),
        17 => tr!("永久刪除（會先確認）", "Delete permanently (asks first)"),
        18 => tr!("複製／剪下／貼上", "Copy/cut/paste"),
        19 => tr!("複製路徑", "Copy path"),
        20 => tr!("新增資料夾", "New folder"),
        21 => tr!("全選", "Select all"),
        22 => tr!("內容", "Properties"),
        23 => tr!("設定", "Settings"),
        _ => tr!("說明（這一頁）", "Help (this page)"),
    }
}

/// 一組按鍵的寫法：連續的數字寫成範圍（Ctrl+Shift+1～8），其他用「、」隔開
fn keys_text(bindings: &[&Binding]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bindings.len() {
        let first = bindings[i];
        let is_digit = |b: &Binding| (0x30..=0x39).contains(&b.key);
        let mut j = i;
        while is_digit(first)
            && j + 1 < bindings.len()
            && is_digit(bindings[j + 1])
            && bindings[j + 1].key == bindings[j].key + 1
            && (bindings[j + 1].ctrl, bindings[j + 1].shift, bindings[j + 1].alt) == (first.ctrl, first.shift, first.alt)
        {
            j += 1;
        }
        if j > i + 1 {
            out.push(format!("{}～{}", label(first), key_name(bindings[j].key)));
        } else {
            for b in &bindings[i..=j] {
                out.push(label(b));
            }
        }
        i = j + 1;
    }
    out.join(tr!("、", ", "))
}

/// 預設的表的說明一覽（按鍵, 說明）
pub fn help_rows() -> Vec<(String, String)> {
    help_rows_in(BINDINGS)
}

/// 說明頁的快速鍵一覽，分段：(段落, [(按鍵, 說明)])，段落的順序依第一次出現在表裡的順序
fn help_sections_in(bindings: &[Binding]) -> Vec<(Section, Vec<(String, String)>)> {
    let mut groups: Vec<u8> = Vec::new();
    for b in bindings.iter().filter(|b| b.help) {
        let g = group(b.cmd);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let rows = help_rows_in(bindings);
    let mut out: Vec<(Section, Vec<(String, String)>)> = Vec::new();
    for (g, row) in groups.into_iter().zip(rows) {
        let section = section_of(g);
        match out.iter_mut().find(|(s, _)| *s == section) {
            Some((_, list)) => list.push(row),
            None => out.push((section, vec![row])),
        }
    }
    out
}

/// 說明的快速鍵一覽（按鍵, 說明），依快速鍵表的順序
fn help_rows_in(bindings: &[Binding]) -> Vec<(String, String)> {
    let mut groups: Vec<u8> = Vec::new();
    for b in bindings.iter().filter(|b| b.help) {
        let g = group(b.cmd);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    groups
        .into_iter()
        .map(|g| {
            let list: Vec<&Binding> = bindings.iter().filter(|b| b.help && group(b.cmd) == g).collect();
            // 「跳到第 N 個分頁」的 Ctrl+9 寫在說明文字裡
            let list: Vec<&Binding> = if g == 4 { list.into_iter().filter(|b| b.cmd != GoToTab(9)).collect() } else { list };
            (keys_text(&list), group_text(g).to_string())
        })
        .collect()
}

/// 兩筆按法會不會在同一個地方同時成立（按鍵與修飾鍵相同，範圍會同時成立：全域和任何地方、不在輸入框和清單）
fn clash(a: &Binding, b: &Binding) -> bool {
    let overlaps = a.scope == b.scope || a.scope == Global || b.scope == Global || matches!((a.scope, b.scope), (NotEdit, List) | (List, NotEdit));
    a.key == b.key && (a.ctrl, a.shift, a.alt) == (b.ctrl, b.shift, b.alt) && overlaps
}

/// 主視窗用的快速鍵表：預設是 [`BINDINGS`]，可以套上自訂的按法
#[derive(Clone, Debug)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    fn default() -> Keymap {
        Keymap { bindings: BINDINGS.to_vec() }
    }
}

impl Keymap {
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// 按下的鍵在目前的位置對應哪個命令（同 [`lookup`]）
    pub fn lookup(&self, key: u32, ctrl: bool, shift: bool, alt: bool, in_edit: bool, in_list: bool) -> Option<Cmd> {
        lookup_in(&self.bindings, key, ctrl, shift, alt, in_edit, in_list)
    }

    /// 說明的快速鍵一覽（同 [`help_rows`]，自訂的按法也列進去）
    pub fn help_rows(&self) -> Vec<(String, String)> {
        help_rows_in(&self.bindings)
    }

    /// 說明頁的快速鍵一覽，分成分頁、檢視、移動與搜尋、檔案、其他幾段
    pub fn help_sections(&self) -> Vec<(Section, Vec<(String, String)>)> {
        help_sections_in(&self.bindings)
    }

    pub fn with_hint(&self, name: &str, cmd: Cmd, separator: &str) -> String {
        let keys = self.bindings.iter().filter(|b| b.cmd == cmd && b.help).map(label).collect::<Vec<_>>().join(" / ");
        if keys.is_empty() { name.into() } else { format!("{name}{separator}{keys}") }
    }

    /// 某個命令目前的按法
    pub fn combos(&self, cmd: Cmd) -> Vec<Combo> {
        self.bindings.iter().filter(|b| b.cmd == cmd).map(|b| Combo { key: b.key, ctrl: b.ctrl, shift: b.shift, alt: b.alt }).collect()
    }

    /// 同一個地方有兩筆一樣按法的（預設的表一定沒有；自訂後用來提醒）
    pub fn conflicts(&self) -> Vec<(Binding, Binding)> {
        let mut out = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for b in &self.bindings[i + 1..] {
                if a.cmd != b.cmd && clash(a, b) {
                    out.push((*a, *b));
                }
            }
        }
        out
    }

    /// 套上自訂的按法：(命令代號, 按法)，按法空著表示這個命令不用快速鍵。自訂的命令拿掉原本所有的按法
    /// （包括不列在說明裡的），換成新的；新的按法和別的命令撞在一起時，自訂的優先，原本那個命令的這個按法拿掉。
    /// 回傳新的表與要告訴使用者的訊息（看不懂的命令或按法、不能用的按法、被拿掉的預設按法）。
    pub fn with_overrides(overrides: &[(String, Vec<String>)]) -> (Keymap, Vec<String>) {
        let mut map = Keymap::default();
        let mut notes = Vec::new();
        for (id, combos) in overrides {
            let Some(cmd) = Cmd::from_id(id) else {
                notes.push(trf!("快速鍵：不認得的命令「{}」", "Shortcuts: unknown command “{}”", id));
                continue;
            };
            let mut parsed = Vec::new();
            for text in combos {
                match parse_combo(text) {
                    Some(c) if c.reserved() => notes.push(trf!("快速鍵：「{}」不能自訂（打字、預覽或換焦點要用）", "Shortcuts: “{}” can't be customized (used for typing, preview or focus)", text)),
                    Some(c) => parsed.push(c),
                    None => notes.push(trf!("快速鍵：看不懂「{}」（例如 Ctrl+Shift+T、Alt+←、F5）", "Shortcuts: can't read “{}” (e.g. Ctrl+Shift+T, Alt+←, F5)", text)),
                }
            }
            let scope = cmd.scope();
            map.bindings.retain(|b| b.cmd != cmd);
            for c in parsed {
                let new = Binding { key: c.key, ctrl: c.ctrl, shift: c.shift, alt: c.alt, scope, cmd, help: true };
                let before = map.bindings.len();
                let taken: Vec<Cmd> = map.bindings.iter().filter(|b| clash(b, &new)).map(|b| b.cmd).collect();
                map.bindings.retain(|b| !clash(b, &new));
                if map.bindings.len() != before {
                    for other in taken {
                        notes.push(trf!("快速鍵：{} 改給「{}」，原本的「{}」不再用這個按法", "Shortcuts: {} now runs “{}” instead of “{}”", c.label(), cmd.id(), other.id()));
                    }
                }
                map.bindings.push(new);
            }
        }
        (map, notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一個地方不能有兩筆一樣的按鍵組合（全域的組合在任何地方都會先被它接走）
    #[test]
    fn no_duplicate_combinations() {
        for (i, a) in BINDINGS.iter().enumerate() {
            for b in &BINDINGS[i + 1..] {
                assert!(!clash(a, b), "{} 同時是 {:?} 與 {:?}", label(a), a.cmd, b.cmd);
            }
        }
    }

    /// D 鍵：Ctrl+D 刪除（清單）、Alt+D 編輯路徑、Ctrl+Shift+D 只提示雙欄改成 Ctrl+\，不再切換雙欄
    #[test]
    fn d_key_does_not_overlap() {
        let d = vk::letter(b'D');
        assert_eq!(lookup(d, true, false, false, false, true), Some(Delete));
        // 不在清單裡時 Ctrl+D 什麼都不做（不會刪到清單裡選著的項目）
        assert_eq!(lookup(d, true, false, false, false, false), None);
        assert_eq!(lookup(d, false, false, true, false, false), Some(EditPath));
        assert_eq!(lookup(d, true, true, false, false, true), Some(OldDualShortcut));
        let commands: Vec<Cmd> = BINDINGS.iter().filter(|b| b.key == d).map(|b| b.cmd).collect();
        assert_eq!(commands.len(), 3, "{commands:?}");
        assert_eq!(lookup(vk::OEM_5, true, false, false, true, false), Some(ToggleDual));
    }

    /// 編輯路徑是 Ctrl+E（和 Alt+D）；舊的 Ctrl+L 只提示改成 Ctrl+E，不列在說明裡
    #[test]
    fn edit_path_is_ctrl_e() {
        assert_eq!(lookup(vk::letter(b'E'), true, false, false, false, true), Some(EditPath));
        assert_eq!(lookup(vk::letter(b'E'), true, false, false, true, false), Some(EditPath));
        assert_eq!(lookup(vk::letter(b'L'), true, false, false, false, true), Some(OldPathShortcut));
        let help = help_rows();
        assert!(help.iter().any(|(k, _)| k.contains("Ctrl+E") && k.contains("Alt+D")), "{help:?}");
        assert!(!help.iter().any(|(k, _)| k.contains("Ctrl+L")), "{help:?}");
    }

    #[test]
    fn lookups_respect_scope_and_modifiers() {
        // 輸入框裡：Ctrl+Z、Ctrl+C 交給輸入框；Ctrl+T、F5 照常
        assert_eq!(lookup(vk::letter(b'Z'), true, false, false, true, false), None);
        assert_eq!(lookup(vk::letter(b'C'), true, false, false, true, false), None);
        assert_eq!(lookup(vk::letter(b'T'), true, false, false, true, false), Some(NewTab));
        assert_eq!(lookup(vk::F5, false, false, false, true, false), Some(Refresh));
        // 修飾鍵要完全相同
        assert_eq!(lookup(vk::letter(b'T'), true, true, false, false, true), Some(ReopenTab));
        assert_eq!(lookup(vk::letter(b'N'), true, true, false, false, true), Some(NewFolder));
        assert_eq!(lookup(vk::letter(b'W'), true, true, false, false, true), None);
        assert_eq!(lookup(vk::DELETE, false, true, false, false, true), Some(DeletePermanently));
        assert_eq!(lookup(vk::digit(9), true, false, false, false, true), Some(GoToTab(9)));
        assert_eq!(lookup(vk::digit(3), true, true, false, false, true), Some(View(3)));
    }

    #[test]
    fn command_ids_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for c in Cmd::CUSTOMIZABLE {
            assert_eq!(Cmd::from_id(&c.id()), Some(*c), "{c:?}");
            assert!(seen.insert(c.id()), "重複的代號 {}", c.id());
        }
        // 每個在表裡的命令（舊快速鍵的提示除外）都可以自訂
        for b in BINDINGS.iter().filter(|b| !matches!(b.cmd, OldDualShortcut | OldPathShortcut)) {
            assert!(Cmd::CUSTOMIZABLE.contains(&b.cmd), "{:?}", b.cmd);
        }
        assert_eq!(Cmd::from_id(" New-Tab "), Some(NewTab));
        assert_eq!(Cmd::from_id("old-dual-pane-shortcut"), None);
    }

    #[test]
    fn combos_parse_like_the_help_writes_them() {
        for b in BINDINGS {
            let c = parse_combo(&label(b)).unwrap_or_else(|| panic!("{}", label(b)));
            assert_eq!((c.key, c.ctrl, c.shift, c.alt), (b.key, b.ctrl, b.shift, b.alt), "{}", label(b));
        }
        assert_eq!(parse_combo("ctrl + shift + t"), Some(Combo { key: vk::letter(b'T'), ctrl: true, shift: true, alt: false }));
        assert_eq!(parse_combo("Alt+Left"), Some(Combo { key: vk::LEFT, ctrl: false, shift: false, alt: true }));
        assert_eq!(parse_combo("Ctrl+PageDown").map(|c| c.key), Some(vk::NEXT));
        assert_eq!(parse_combo("F11").map(|c| c.key), Some(vk::F1 + 10));
        assert_eq!(parse_combo("Ctrl+Ctrl+T"), None);
        assert_eq!(parse_combo("Hyper+T"), None);
        assert_eq!(parse_combo("Ctrl+"), None);
        assert_eq!(parse_combo("Ctrl+F25"), None);
        assert!(parse_combo("Space").is_some_and(|c| c.reserved()));
        assert!(parse_combo("A").is_some_and(|c| c.reserved()));
        assert!(!parse_combo("Ctrl+A").is_some_and(|c| c.reserved()));
        assert!(!parse_combo("F7").is_some_and(|c| c.reserved()));
    }

    #[test]
    fn overrides_replace_and_report_conflicts() {
        let map = Keymap::default();
        assert!(map.conflicts().is_empty());
        let o = |id: &str, keys: &[&str]| (id.to_string(), keys.iter().map(|k| k.to_string()).collect::<Vec<_>>());
        // 換成 Ctrl+Shift+N 開新分頁：原本的 Ctrl+T、Ctrl+N 都不再開新分頁；新增資料夾讓出 Ctrl+Shift+N
        let (map, notes) = Keymap::with_overrides(&[o("new-tab", &["Ctrl+Shift+N"])]);
        assert_eq!(map.lookup(vk::letter(b'N'), true, true, false, false, true), Some(NewTab));
        assert_eq!(map.lookup(vk::letter(b'T'), true, false, false, false, true), None);
        assert_eq!(map.combos(NewFolder), vec![]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(map.conflicts().is_empty());
        // 自訂的刪除只在清單上有效（沿用預設的範圍）
        let (map, _) = Keymap::with_overrides(&[o("delete", &["Ctrl+Shift+X"])]);
        assert_eq!(map.lookup(vk::letter(b'X'), true, true, false, false, true), Some(Delete));
        assert_eq!(map.lookup(vk::letter(b'X'), true, true, false, false, false), None);
        assert_eq!(map.lookup(vk::DELETE, false, false, false, false, true), None);
        // 空著：不用快速鍵；說明裡就沒有那一列
        let (map, notes) = Keymap::with_overrides(&[o("refresh", &[])]);
        assert!(notes.is_empty());
        assert_eq!(map.lookup(vk::F5, false, false, false, false, true), None);
        assert!(!map.help_rows().iter().any(|(_, t)| t == tr!("重新整理", "Refresh")));
        // 看不懂、不能用的都說明，不影響其他的
        let (map, notes) = Keymap::with_overrides(&[o("teleport", &["Ctrl+Q"]), o("search", &["Q", "Ctrl+Q", "Ctrl+Nope"])]);
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert_eq!(map.lookup(vk::letter(b'Q'), true, false, false, false, true), Some(Search));
        assert_eq!(map.lookup(vk::letter(b'Q'), false, false, false, false, true), None);
        assert!(map.help_rows().iter().any(|(k, t)| k == "Ctrl+Q" && t == tr!("搜尋檔名", "Search file names")));
    }

    #[test]
    fn help_sections_cover_every_row() {
        let map = Keymap::default();
        let sections = map.help_sections();
        let total: usize = sections.iter().map(|(_, rows)| rows.len()).sum();
        assert_eq!(total, map.help_rows().len());
        let titles: Vec<Section> = sections.iter().map(|(s, _)| *s).collect();
        assert_eq!(titles, [Section::Tabs, Section::View, Section::Navigate, Section::Files, Section::Other]);
        let other = &sections.last().unwrap().1;
        assert!(other.iter().any(|(k, _)| k == "F1"), "{other:?}");
        assert!(other.iter().any(|(k, _)| k.contains("F12")), "{other:?}");
    }

    #[test]
    fn help_lists_the_table() {
        let rows = help_rows();
        let find = |text: &str| rows.iter().find(|(_, t)| t.contains(text)).map(|(k, _)| k.clone());
        assert_eq!(find("單欄").as_deref(), Some("Ctrl+\\"));
        assert_eq!(find("超大圖示").as_deref(), Some("Ctrl+Shift+1～8"));
        assert_eq!(find("跳到第").as_deref(), Some("Ctrl+1～8"));
        assert_eq!(find("新分頁").as_deref(), Some("Ctrl+T、Ctrl+N"));
        assert_eq!(find("永久刪除").as_deref(), Some("Shift+Delete"));
        // 不列出的：舊的 Ctrl+Shift+D、鍵盤上的上一頁鍵、Backspace
        assert!(rows.iter().all(|(k, _)| !k.contains("Ctrl+Shift+D") && !k.contains("Backspace") && !k.contains("0xA6")));
    }
}

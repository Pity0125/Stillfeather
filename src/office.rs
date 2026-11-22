//! Office 文件的內容預覽：Word、Excel、PowerPoint 與 OpenDocument（.odt／.ods／.odp）。
//!
//! 這些檔案都是 ZIP 裡面放 XML，這裡直接讀出內容，不需要安裝 Office，也不執行文件裡的任何東西：
//! - Word／文字文件：段落、標題（依樣式判斷層級）、項目清單與表格。
//! - PowerPoint／簡報：每張投影片的標題、內文（含層級）與表格；頁首頁尾、日期、頁碼不顯示。
//! - Excel／試算表：第一個顯示中的工作表，儲存格內容（共用字串、日期、百分比、布林值）。
//!
//! 只取文字與表格，不還原版面、字型與圖片。每一份內容都有大小與數量上限，任何不合法的輸入都回傳錯誤，不會 panic。

use crate::xml::{local, Event, Reader};
use crate::zip::Zip;
use std::collections::HashMap;

/// 單一 XML 檔案解壓後的上限
const MAX_PART: usize = 48 * 1024 * 1024;
/// 主要內容（文件本文、工作表）只解壓前面這麼多：只顯示開頭，大檔案不必整個解開
const MAX_BODY_PREFIX: usize = 16 * 1024 * 1024;
const MAX_BLOCKS: usize = 2_000;
const MAX_CHARS: usize = 400_000;
const MAX_CELL_CHARS: usize = 500;
/// 文件裡的表格最多幾列
const MAX_DOC_TABLE_ROWS: usize = 200;
/// 試算表最多顯示幾列、幾欄
pub const MAX_SHEET_ROWS: usize = 500;
pub const MAX_COLS: usize = 40;
const MAX_SLIDES: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Word,
    Excel,
    PowerPoint,
    OdText,
    OdSheet,
    OdPresentation,
}

impl Format {
    pub fn from_ext(ext: &str) -> Option<Format> {
        Some(match ext {
            "docx" | "docm" | "dotx" | "dotm" => Format::Word,
            "xlsx" | "xlsm" | "xltx" | "xltm" => Format::Excel,
            "pptx" | "pptm" | "ppsx" | "ppsm" | "potx" | "potm" => Format::PowerPoint,
            "odt" | "ott" => Format::OdText,
            "ods" | "ots" => Format::OdSheet,
            "odp" | "otp" => Format::OdPresentation,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Word => "Word 文件",
            Format::Excel => "Excel 活頁簿",
            Format::PowerPoint => "PowerPoint 簡報",
            Format::OdText => "OpenDocument 文字",
            Format::OdSheet => "OpenDocument 試算表",
            Format::OdPresentation => "OpenDocument 簡報",
        }
    }

    pub fn is_presentation(self) -> bool {
        matches!(self, Format::PowerPoint | Format::OdPresentation)
    }
}

/// 舊版 Office 二進位格式（沒有內容預覽）
pub fn is_legacy_ext(ext: &str) -> bool {
    matches!(ext, "doc" | "dot" | "xls" | "xlt" | "xlsb" | "ppt" | "pps" | "pot")
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph(String),
    Item { level: u8, text: String },
    /// 投影片的分隔（從 1 開始）
    Slide { number: usize },
    Table(Table),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
    pub cols: usize,
    /// 還有沒顯示的列或欄
    pub more_rows: bool,
    pub more_cols: bool,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// 內容太多，只取了前面
    pub truncated: bool,
    /// 簡報的投影片總數
    pub slides: usize,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Workbook {
    pub sheets: Vec<String>,
    /// 顯示的是第幾個工作表
    pub shown: usize,
    pub table: Table,
    /// 工作表實際用到的列數與欄數（可能比顯示的多）
    pub used_rows: usize,
    pub used_cols: usize,
    /// 只讀了前面的列，又不知道總列數：`used_rows` 是至少的列數
    pub rows_at_least: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Office {
    Document(Document),
    Workbook(Workbook),
}

/// 讀出文件內容。
pub fn extract(format: Format, data: &[u8]) -> Result<Office, String> {
    let zip = match Zip::open(data) {
        Ok(z) => z,
        Err(e) if e == "ole" => return Err("這個檔案有密碼保護，或是舊版 Office 格式，沒有內容預覽。按 Enter 開啟。".into()),
        Err(_) => return Err("檔案不是有效的 Office 文件，或已經損壞。".into()),
    };
    let result = match format {
        Format::Word => word(&zip).map(Office::Document),
        Format::PowerPoint => powerpoint(&zip).map(Office::Document),
        Format::Excel => excel(&zip).map(Office::Workbook),
        Format::OdText | Format::OdPresentation => {
            let (xml, complete) = zip.read_string_prefix("content.xml", MAX_BODY_PREFIX)?;
            let mut doc = if format == Format::OdText { odf_text(&xml) } else { odf_presentation(&xml) };
            doc.truncated |= !complete;
            Ok(Office::Document(doc))
        }
        Format::OdSheet => {
            let (xml, complete) = zip.read_string_prefix("content.xml", MAX_BODY_PREFIX)?;
            let mut book = odf_sheet(&xml);
            if !complete {
                book.table.more_rows = true;
                book.rows_at_least = true;
            }
            Ok(Office::Workbook(book))
        }
    };
    result.map_err(|e| format!("無法讀取文件內容：{e}"))
}

/// 文件裡有沒有存縮圖（PowerPoint 預設會存第 1 張投影片；OpenDocument 一律有）。
/// 有的話 Windows 的 Office 縮圖處理程式才可能產生影像。
pub fn has_thumbnail(data: &[u8]) -> bool {
    Zip::open(data).is_ok_and(|z| z.entries().iter().any(|e| {
        let n = e.name.to_ascii_lowercase();
        n.starts_with("docprops/thumbnail.") || n.starts_with("thumbnails/thumbnail.")
    }))
}

impl Office {
    /// 大約占用的記憶體（預覽快取用）
    pub fn approx_bytes(&self) -> usize {
        let table = |t: &Table| t.rows.iter().flatten().map(|c| c.len() + 24).sum::<usize>() + 64;
        match self {
            Office::Document(d) => d
                .blocks
                .iter()
                .map(|b| match b {
                    Block::Heading { text, .. } | Block::Paragraph(text) | Block::Item { text, .. } => text.len() + 40,
                    Block::Slide { .. } => 40,
                    Block::Table(t) => table(t),
                })
                .sum(),
            Office::Workbook(w) => table(&w.table) + w.sheets.iter().map(|s| s.len() + 24).sum::<usize>(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// 試算表儲存格的對齊（像 Excel：數字、百分比、日期靠右，TRUE／FALSE 置中，文字靠左）
pub fn cell_align(text: &str) -> Align {
    let t = text.trim();
    if t == "TRUE" || t == "FALSE" {
        return Align::Center;
    }
    let number = t.trim_end_matches('%').replace(',', "");
    if !number.is_empty() && number.parse::<f64>().is_ok_and(f64::is_finite) {
        return Align::Right;
    }
    let date_like = t.starts_with(|c: char| c.is_ascii_digit())
        && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '/' | '-' | ':' | ' ' | '.'))
        && t.contains(['/', ':', '-']);
    if date_like {
        Align::Right
    } else {
        Align::Left
    }
}

/// 表格的欄寬放不下時：窄的欄維持原本寬度，寬的欄平分剩下的空間（不小於 `min`）。
pub fn fit_columns(natural: &[i32], avail: i32, min: i32) -> Vec<i32> {
    let n = natural.len();
    let mut widths = natural.to_vec();
    if natural.iter().sum::<i32>() <= avail {
        return widths;
    }
    let mut fixed = vec![false; n];
    loop {
        let free: Vec<usize> = (0..n).filter(|i| !fixed[*i]).collect();
        if free.is_empty() {
            break;
        }
        let used: i32 = (0..n).filter(|i| fixed[*i]).map(|i| widths[i]).sum();
        let share = ((avail - used) / free.len() as i32).max(min);
        let mut changed = false;
        for &i in &free {
            if natural[i] <= share {
                widths[i] = natural[i];
                fixed[i] = true;
                changed = true;
            }
        }
        if !changed {
            for &i in &free {
                widths[i] = share;
            }
            break;
        }
    }
    widths
}

/// 底部的說明文字
pub fn caption(format: Format, office: &Office) -> String {
    match office {
        Office::Document(d) => {
            let mut parts = vec![format.label().to_string()];
            if format.is_presentation() {
                parts.push(format!("{} 張投影片", d.slides));
            } else {
                let paras = d.blocks.iter().filter(|b| !matches!(b, Block::Table(_) | Block::Slide { .. })).count();
                parts.push(format!("{paras} 段"));
                let tables = d.blocks.iter().filter(|b| matches!(b, Block::Table(_))).count();
                if tables > 0 {
                    parts.push(format!("{tables} 個表格"));
                }
            }
            if d.truncated {
                parts.push("只顯示前面的內容".into());
            }
            parts.join(" · ")
        }
        Office::Workbook(w) => {
            let name = w.sheets.get(w.shown).cloned().unwrap_or_default();
            let mut parts = vec![if name.is_empty() { format.label().to_string() } else { name }];
            if w.sheets.len() > 1 {
                parts.push(format!("共 {} 個工作表", w.sheets.len()));
            }
            parts.push(if w.rows_at_least { format!("超過 {} 列 × {} 欄", w.used_rows, w.used_cols) } else { format!("{} 列 × {} 欄", w.used_rows, w.used_cols) });
            if w.table.more_rows || w.table.more_cols {
                parts.push(format!("只顯示前 {} 列、{} 欄", w.table.rows.len(), w.table.cols));
            }
            parts.join(" · ")
        }
    }
}

// ---------------------------------------------------------------- 共用

/// 收集區塊（有上限）
#[derive(Default)]
struct Out {
    doc: Document,
    chars: usize,
}

impl Out {
    fn full(&self) -> bool {
        self.doc.blocks.len() >= MAX_BLOCKS || self.chars >= MAX_CHARS
    }

    fn push(&mut self, b: Block) {
        if self.full() {
            self.doc.truncated = true;
            return;
        }
        self.chars += match &b {
            Block::Heading { text, .. } | Block::Paragraph(text) | Block::Item { text, .. } => text.len(),
            Block::Table(t) => t.rows.iter().flatten().map(String::len).sum(),
            Block::Slide { .. } => 0,
        };
        self.doc.blocks.push(b);
    }
}

/// 表格的建立過程（Word、PowerPoint 與 OpenDocument 共用）
#[derive(Default)]
struct TableBuild {
    rows: Vec<Vec<String>>,
    row: Option<Vec<String>>,
    cell: Option<String>,
    more_rows: bool,
    more_cols: bool,
}

impl TableBuild {
    fn start_row(&mut self) {
        self.row = Some(Vec::new());
    }

    fn start_cell(&mut self) {
        self.cell = Some(String::new());
    }

    /// 把一段文字加進目前的儲存格
    fn add_text(&mut self, text: &str) {
        if let Some(c) = &mut self.cell {
            if c.chars().count() >= MAX_CELL_CHARS {
                return;
            }
            if !c.is_empty() {
                c.push('\n');
            }
            c.push_str(text);
        }
    }

    fn end_cell(&mut self, repeat: usize) {
        let cell = self.cell.take().unwrap_or_default();
        if let Some(row) = &mut self.row {
            for _ in 0..repeat.max(1) {
                if row.len() >= MAX_COLS {
                    if !cell.trim().is_empty() {
                        self.more_cols = true;
                    }
                    break;
                }
                row.push(cell.clone());
            }
        }
    }

    fn end_row(&mut self, repeat: usize, max_rows: usize) {
        let row = self.row.take().unwrap_or_default();
        let empty = row.iter().all(|c| c.trim().is_empty());
        // 重複的空白列（試算表結尾常有上百萬列）只算位置，不產生內容
        let times = if empty { repeat.clamp(1, 2) } else { repeat.clamp(1, max_rows) };
        for _ in 0..times {
            if self.rows.len() >= max_rows {
                if !empty {
                    self.more_rows = true;
                }
                return;
            }
            self.rows.push(row.clone());
        }
    }

    /// 完成：去掉結尾的空白列與空白欄
    fn finish(self) -> Table {
        finish_rows(self.rows, self.more_rows, self.more_cols)
    }
}

fn finish_rows(mut rows: Vec<Vec<String>>, more_rows: bool, more_cols: bool) -> Table {
    while rows.last().is_some_and(|r| r.iter().all(|c| c.trim().is_empty())) {
        rows.pop();
    }
    let cols = rows.iter().map(|r| r.iter().rposition(|c| !c.trim().is_empty()).map_or(0, |i| i + 1)).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(cols, String::new());
    }
    Table { rows, cols, more_rows, more_cols }
}

/// 套件關聯（.rels）：Id → (類型, 目標)
fn rels(zip: &Zip, part: &str) -> HashMap<String, (String, String)> {
    let (dir, file) = match part.rfind('/') {
        Some(i) => (&part[..i + 1], &part[i + 1..]),
        None => ("", part),
    };
    let path = format!("{dir}_rels/{file}.rels");
    let mut out = HashMap::new();
    let Ok(xml) = zip.read_string(&path, MAX_PART) else { return out };
    for ev in Reader::new(&xml) {
        if let Event::Start { name, attrs, .. } = ev {
            if local(name) == "Relationship" {
                if let (Some(id), Some(target)) = (attrs.get("Id"), attrs.get("Target")) {
                    let kind = attrs.get("Type").unwrap_or_default().to_string();
                    out.insert(id.to_string(), (kind, resolve(dir, &target)));
                }
            }
        }
    }
    out
}

/// 關聯的目標路徑換成 ZIP 裡的完整名稱（處理 / 開頭與 ../）
fn resolve(dir: &str, target: &str) -> String {
    let joined = if let Some(abs) = target.strip_prefix('/') { abs.to_string() } else { format!("{dir}{target}") };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(p),
        }
    }
    parts.join("/")
}

/// 主文件的位置（_rels/.rels 裡的 officeDocument），找不到就用預設位置
fn main_part(zip: &Zip, default: &str) -> String {
    let r = rels(zip, "");
    r.values().find(|(kind, _)| kind.ends_with("/officeDocument")).map(|(_, t)| t.clone()).filter(|t| zip.contains(t)).unwrap_or_else(|| default.to_string())
}

fn dir_of(part: &str) -> &str {
    part.rfind('/').map_or("", |i| &part[..i + 1])
}

/// 段落的文字整理：去掉結尾空白；整段空白就不顯示
fn clean(text: &str) -> Option<String> {
    let t = text.trim_end();
    (!t.trim().is_empty()).then(|| t.to_string())
}

// ---------------------------------------------------------------- Word

/// styles.xml 裡的段落樣式
#[derive(Default)]
struct WordStyles {
    /// 樣式編號 → 標題層級（「heading 1」「Title」，或大綱層級）
    headings: HashMap<String, u8>,
    /// 樣式編號 → 清單層級（樣式本身帶編號，例如「List Bullet 2」）
    lists: HashMap<String, u8>,
}

fn word_styles(xml: &str) -> WordStyles {
    let mut out = WordStyles::default();
    // 目前的樣式：編號、名稱、是否帶編號、編號層級
    let mut current: Option<(String, String, bool, Option<u8>)> = None;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, .. } => match (local(name), &mut current) {
                ("style", _) => current = attrs.get("styleId").map(|s| (s.to_string(), String::new(), false, None)),
                ("name", Some(c)) => {
                    c.1 = attrs.get("val").unwrap_or_default().to_lowercase();
                    let level = if c.1 == "title" {
                        Some(1)
                    } else {
                        c.1.strip_prefix("heading ").and_then(|n| n.trim().parse::<u8>().ok()).filter(|n| (1..=9).contains(n))
                    };
                    if let Some(l) = level {
                        out.headings.insert(c.0.clone(), l);
                    }
                }
                ("outlineLvl", Some(c)) => {
                    if let Some(v) = attrs.get("val").and_then(|v| v.parse::<u8>().ok()).filter(|v| *v < 9) {
                        out.headings.entry(c.0.clone()).or_insert(v + 1);
                    }
                }
                ("numId", Some(c)) => c.2 = attrs.get("val").as_deref() != Some("0"),
                ("ilvl", Some(c)) => c.3 = attrs.get("val").and_then(|v| v.parse::<u8>().ok()),
                _ => {}
            },
            Event::End { name } if local(name) == "style" => {
                if let Some((id, name, true, ilvl)) = current.take() {
                    // 沒寫層級時看名稱結尾的數字（「List Bullet 2」是第 2 層）
                    let from_name = name.rsplit(' ').next().and_then(|n| n.parse::<u8>().ok()).map(|n| n.saturating_sub(1));
                    out.lists.insert(id, ilvl.or(from_name).unwrap_or(0).min(8));
                }
            }
            _ => {}
        }
    }
    out
}

fn builtin_heading(style: &str) -> Option<u8> {
    let s = style.to_lowercase();
    if s == "title" {
        return Some(1);
    }
    s.strip_prefix("heading").and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=9).contains(n))
}

#[derive(Default)]
struct Para {
    text: String,
    style: Option<String>,
    list: Option<u8>,
    /// 段落明確取消編號（numId 0），樣式帶的編號也不算
    no_list: bool,
    outline: Option<u8>,
}

fn word(zip: &Zip) -> Result<Document, String> {
    let part = main_part(zip, "word/document.xml");
    let (xml, complete) = zip.read_string_prefix(&part, MAX_BODY_PREFIX)?;
    let styles = zip.read_string(&format!("{}styles.xml", dir_of(&part)), MAX_PART).map(|s| word_styles(&s)).unwrap_or_default();
    let mut out = Out::default();
    let mut paras: Vec<Para> = Vec::new();
    let mut tables: Vec<TableBuild> = Vec::new();
    let mut in_t = false;
    let mut in_ppr = 0usize;
    for ev in Reader::new(&xml) {
        if out.full() {
            out.doc.truncated = true;
            break;
        }
        match ev {
            Event::Start { name, attrs, empty } => {
                let p = paras.last_mut();
                match (local(name), p) {
                    ("p", _) if !empty => paras.push(Para::default()),
                    ("pPr", _) if !empty => in_ppr += 1,
                    ("pStyle", Some(p)) => p.style = attrs.get("val").map(|s| s.to_string()),
                    ("numPr", Some(p)) if in_ppr > 0 => {
                        p.list.get_or_insert(0);
                    }
                    ("ilvl", Some(p)) if in_ppr > 0 => p.list = Some(attrs.get("val").and_then(|v| v.parse().ok()).unwrap_or(0).min(8)),
                    // numId 0 表示取消編號
                    ("numId", Some(p)) if attrs.get("val").as_deref() == Some("0") => {
                        p.list = None;
                        p.no_list = true;
                    }
                    ("outlineLvl", Some(p)) => p.outline = attrs.get("val").and_then(|v| v.parse::<u8>().ok()).filter(|v| *v < 9).map(|v| v + 1),
                    ("t", Some(_)) if !empty => in_t = true,
                    ("tab", Some(p)) if in_ppr == 0 => p.text.push('\t'),
                    ("br" | "cr", Some(p)) if in_ppr == 0 => p.text.push('\n'),
                    ("noBreakHyphen", Some(p)) => p.text.push('-'),
                    ("tbl", _) if !empty => tables.push(TableBuild::default()),
                    ("tr", _) if !empty => {
                        if let Some(t) = tables.last_mut() {
                            t.start_row();
                        }
                    }
                    ("tc", _) if !empty => {
                        if let Some(t) = tables.last_mut() {
                            t.start_cell();
                        }
                    }
                    _ => {}
                }
            }
            Event::End { name } => match local(name) {
                "t" => in_t = false,
                "pPr" => in_ppr = in_ppr.saturating_sub(1),
                "p" => {
                    let Some(p) = paras.pop() else { continue };
                    let Some(text) = clean(&p.text) else { continue };
                    if let Some(t) = tables.last_mut().filter(|t| t.cell.is_some()) {
                        t.add_text(&text);
                        continue;
                    }
                    let level = p.style.as_deref().and_then(|s| styles.headings.get(s).copied().or_else(|| builtin_heading(s))).or(p.outline);
                    let list = if p.no_list { None } else { p.list.or_else(|| p.style.as_deref().and_then(|s| styles.lists.get(s).copied())) };
                    out.push(match (level, list) {
                        (Some(level), _) => Block::Heading { level, text },
                        (None, Some(level)) => Block::Item { level, text },
                        (None, None) => Block::Paragraph(text),
                    });
                }
                "tc" => {
                    if let Some(t) = tables.last_mut() {
                        t.end_cell(1);
                    }
                }
                "tr" => {
                    if let Some(t) = tables.last_mut() {
                        t.end_row(1, MAX_DOC_TABLE_ROWS);
                    }
                }
                "tbl" => {
                    let Some(t) = tables.pop() else { continue };
                    let table = t.finish();
                    if let Some(parent) = tables.last_mut().filter(|t| t.cell.is_some()) {
                        // 表格裡的表格：攤平成文字
                        let text: Vec<String> = table.rows.iter().map(|r| r.join(" · ")).collect();
                        parent.add_text(&text.join("\n"));
                    } else if table.cols > 0 {
                        out.push(Block::Table(table));
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if in_t {
                    if let Some(p) = paras.last_mut() {
                        p.text.push_str(&t);
                    }
                }
            }
        }
    }
    out.doc.truncated |= !complete;
    Ok(out.doc)
}

// ---------------------------------------------------------------- PowerPoint

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Title,
    Subtitle,
    Body,
    Other,
    /// 頁首頁尾、日期、頁碼：不顯示
    Skip,
}

fn powerpoint(zip: &Zip) -> Result<Document, String> {
    let part = main_part(zip, "ppt/presentation.xml");
    let xml = zip.read_string(&part, MAX_PART)?;
    let rel = rels(zip, &part);
    let mut slides: Vec<String> = Vec::new();
    for ev in Reader::new(&xml) {
        if let Event::Start { name, attrs, .. } = ev {
            if local(name) == "sldId" {
                if let Some((_, target)) = attrs.get("r:id").and_then(|id| rel.get(id.as_ref())) {
                    slides.push(target.clone());
                }
            }
        }
    }
    if slides.is_empty() {
        // 沒有投影片清單時，依檔名順序找
        let mut found: Vec<(usize, String)> = zip
            .entries()
            .iter()
            .filter_map(|e| e.name.strip_prefix("ppt/slides/slide").and_then(|n| n.strip_suffix(".xml")).and_then(|n| n.parse().ok()).map(|n| (n, e.name.clone())))
            .collect();
        found.sort();
        slides = found.into_iter().map(|(_, n)| n).collect();
    }
    let mut out = Out::default();
    out.doc.slides = slides.len();
    for (i, slide) in slides.iter().enumerate() {
        if i >= MAX_SLIDES || out.full() {
            out.doc.truncated = true;
            break;
        }
        out.push(Block::Slide { number: i + 1 });
        let Ok(xml) = zip.read_string(slide, MAX_PART) else {
            out.push(Block::Paragraph("（無法讀取這張投影片）".into()));
            continue;
        };
        let blocks = slide_blocks(&xml);
        if blocks.is_empty() {
            out.push(Block::Paragraph("（這張投影片沒有文字）".into()));
        }
        for b in blocks {
            out.push(b);
        }
    }
    Ok(out.doc)
}

/// 一張投影片：標題在前，其他文字與表格依出現順序
fn slide_blocks(xml: &str) -> Vec<Block> {
    let mut titles = Vec::new();
    let mut rest = Vec::new();
    let mut shape = Shape::Other;
    let mut para: Option<(u8, String)> = None;
    let mut tables: Vec<TableBuild> = Vec::new();
    let mut in_t = false;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => match local(name) {
                "sp" if !empty => shape = Shape::Other,
                "ph" => {
                    shape = match attrs.get("type").as_deref() {
                        Some("title" | "ctrTitle") => Shape::Title,
                        Some("subTitle") => Shape::Subtitle,
                        Some("dt" | "ftr" | "sldNum" | "hdr") => Shape::Skip,
                        Some("body" | "obj") | None => Shape::Body,
                        Some(_) => Shape::Other,
                    }
                }
                "p" if !empty => para = Some((0, String::new())),
                "pPr" => {
                    if let (Some(p), Some(l)) = (&mut para, attrs.get("lvl").and_then(|v| v.parse::<u8>().ok())) {
                        p.0 = l.min(8);
                    }
                }
                "t" if !empty => in_t = true,
                "br" => {
                    if let Some(p) = &mut para {
                        p.1.push('\n');
                    }
                }
                "tbl" if !empty => tables.push(TableBuild::default()),
                "tr" if !empty => {
                    if let Some(t) = tables.last_mut() {
                        t.start_row();
                    }
                }
                "tc" if !empty => {
                    if let Some(t) = tables.last_mut() {
                        t.start_cell();
                    }
                }
                _ => {}
            },
            Event::End { name } => match local(name) {
                "t" => in_t = false,
                "sp" => shape = Shape::Other,
                "p" => {
                    let Some((level, text)) = para.take() else { continue };
                    let Some(text) = clean(&text) else { continue };
                    if let Some(t) = tables.last_mut().filter(|t| t.cell.is_some()) {
                        t.add_text(&text);
                        continue;
                    }
                    match shape {
                        Shape::Skip => {}
                        Shape::Title => titles.push(Block::Heading { level: 2, text }),
                        Shape::Subtitle | Shape::Other => rest.push(Block::Paragraph(text)),
                        Shape::Body => rest.push(Block::Item { level, text }),
                    }
                }
                "tc" => {
                    if let Some(t) = tables.last_mut() {
                        t.end_cell(1);
                    }
                }
                "tr" => {
                    if let Some(t) = tables.last_mut() {
                        t.end_row(1, MAX_DOC_TABLE_ROWS);
                    }
                }
                "tbl" => {
                    if let Some(t) = tables.pop() {
                        let table = t.finish();
                        if table.cols > 0 {
                            rest.push(Block::Table(table));
                        }
                    }
                }
                _ => {}
            },
            Event::Text(t) => {
                if in_t {
                    if let Some(p) = &mut para {
                        p.1.push_str(&t);
                    }
                }
            }
        }
    }
    titles.extend(rest);
    titles
}

// ---------------------------------------------------------------- Excel

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumKind {
    General,
    DateTime { date: bool, time: bool },
    Percent(u8),
}

/// 數字格式代碼判斷日期、時間或百分比（忽略引號裡的文字與 [] 裡的色彩、條件）
fn classify_format(code: &str) -> NumKind {
    let mut plain = String::new();
    let mut chars = code.chars();
    let mut in_quote = false;
    let mut in_bracket = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => in_quote = !in_quote,
            '[' if !in_quote => in_bracket = true,
            ']' if in_bracket => in_bracket = false,
            '\\' => {
                chars.next();
            }
            _ if in_quote || in_bracket => {}
            c => plain.push(c.to_ascii_lowercase()),
        }
    }
    // 「General」與科學記號（0.00E+00）裡的 e 不是日期的年份代碼
    let plain = plain.replace("general", "").replace("e+", "").replace("e-", "");
    let date = plain.contains('y') || plain.contains('d') || plain.contains('e');
    let time = plain.contains('h') || plain.contains('s');
    if date || time {
        return NumKind::DateTime { date: date || (plain.contains('m') && !time), time };
    }
    if plain.contains('%') {
        let decimals = plain.split('.').nth(1).map_or(0, |d| d.chars().take_while(|c| *c == '0').count()) as u8;
        return NumKind::Percent(decimals);
    }
    NumKind::General
}

fn builtin_format(id: u32) -> NumKind {
    match id {
        9 => NumKind::Percent(0),
        10 => NumKind::Percent(2),
        14..=17 | 27..=31 | 34..=36 | 50..=58 => NumKind::DateTime { date: true, time: false },
        18..=21 | 32 | 33 | 45..=47 => NumKind::DateTime { date: false, time: true },
        22 => NumKind::DateTime { date: true, time: true },
        _ => NumKind::General,
    }
}

/// styles.xml：每個儲存格樣式（cellXfs 的順序）對應的數字格式
fn excel_styles(xml: &str) -> Vec<NumKind> {
    let mut custom: HashMap<u32, NumKind> = HashMap::new();
    let mut kinds = Vec::new();
    let mut in_xfs = false;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => match local(name) {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (attrs.get("numFmtId").and_then(|v| v.parse().ok()), attrs.get("formatCode")) {
                        custom.insert(id, classify_format(&code));
                    }
                }
                "cellXfs" if !empty => in_xfs = true,
                "xf" if in_xfs => {
                    let id: u32 = attrs.get("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
                    kinds.push(custom.get(&id).copied().unwrap_or_else(|| builtin_format(id)));
                }
                _ => {}
            },
            Event::End { name } if local(name) == "cellXfs" => in_xfs = false,
            _ => {}
        }
    }
    kinds
}

fn shared_strings(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut in_t = false;
    let mut in_phonetic = false;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, empty, .. } => match local(name) {
                "si" if !empty => current = Some(String::new()),
                "si" => out.push(String::new()),
                "rPh" if !empty => in_phonetic = true,
                "t" if !empty => in_t = true,
                _ => {}
            },
            Event::End { name } => match local(name) {
                "si" => out.push(current.take().unwrap_or_default()),
                "rPh" => in_phonetic = false,
                "t" => in_t = false,
                _ => {}
            },
            Event::Text(t) => {
                if in_t && !in_phonetic {
                    if let Some(c) = &mut current {
                        c.push_str(&t);
                    }
                }
            }
        }
        if out.len() > 2_000_000 {
            break;
        }
    }
    out
}

/// 「AB12」→（欄 27, 列 11），從 0 開始
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let digits = &r[letters.len()..];
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let col = letters.bytes().fold(0usize, |acc, b| acc * 26 + (b.to_ascii_uppercase() - b'A' + 1) as usize) - 1;
    let row = digits.parse::<usize>().ok()?.checked_sub(1)?;
    Some((col, row))
}

/// 欄名：0 → A、26 → AA
pub fn column_name(mut col: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// 一般數字：整數照原樣，小數最多約 10 位有效數字（像 Excel 的「通用格式」）
fn general_number(v: &str) -> String {
    let Ok(f) = v.trim().parse::<f64>() else { return v.to_string() };
    if !f.is_finite() {
        return v.to_string();
    }
    if f.fract() == 0.0 && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let a = f.abs();
    if !(1e-9..1e15).contains(&a) {
        return format!("{f:.4e}");
    }
    let int_digits = if a >= 1.0 { a.log10().floor() as i32 + 1 } else { 0 };
    let decimals = (10 - int_digits).clamp(0, 10) as usize;
    let s = format!("{f:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// 由 1970-01-01 起的天數換成年月日（Howard Hinnant 的演算法）
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Excel 的日期序號換成文字（1900 系統含當年不存在的 2/29；1904 系統從 1904/1/1 起算）
fn excel_date(serial: f64, date1904: bool, date: bool, time: bool) -> String {
    if !serial.is_finite() || !(0.0..3_000_000.0).contains(&serial) {
        return general_number(&serial.to_string());
    }
    let days = serial.floor() as i64;
    let mut secs = ((serial - serial.floor()) * 86_400.0).round() as i64;
    let mut days_from_1899_12_31 = if date1904 { days + 1461 } else if days < 60 { days } else { days - 1 };
    if secs >= 86_400 {
        secs -= 86_400;
        days_from_1899_12_31 += 1;
    }
    let mut out = String::new();
    if date {
        let (y, m, d) = civil_from_days(days_from_1899_12_31 - 25_568);
        out = format!("{y}/{m}/{d}");
    }
    if time {
        if !out.is_empty() {
            out.push(' ');
        }
        let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        out.push_str(&if s != 0 { format!("{h:02}:{mi:02}:{s:02}") } else { format!("{h:02}:{mi:02}") });
    }
    out
}

fn format_number(v: &str, kind: NumKind, date1904: bool) -> String {
    match (kind, v.trim().parse::<f64>()) {
        (NumKind::DateTime { date, time }, Ok(f)) => excel_date(f, date1904, date, time),
        (NumKind::Percent(d), Ok(f)) => format!("{:.*}%", d as usize, f * 100.0),
        _ => general_number(v),
    }
}

/// 已經收集到的儲存格（列, 欄, 內容）組成表格
fn assemble(cells: Vec<(usize, usize, String)>, max_rows: usize) -> (Table, usize, usize) {
    let used_rows = cells.iter().map(|c| c.0 + 1).max().unwrap_or(0);
    let used_cols = cells.iter().map(|c| c.1 + 1).max().unwrap_or(0);
    let rows_n = used_rows.min(max_rows);
    let cols_n = used_cols.min(MAX_COLS);
    let mut rows = vec![vec![String::new(); cols_n]; rows_n];
    for (r, c, v) in cells {
        if r < rows_n && c < cols_n {
            rows[r][c] = v;
        }
    }
    let table = finish_rows(rows, used_rows > max_rows, used_cols > MAX_COLS);
    (table, used_rows, used_cols)
}

fn excel(zip: &Zip) -> Result<Workbook, String> {
    let part = main_part(zip, "xl/workbook.xml");
    let xml = zip.read_string(&part, MAX_PART)?;
    let rel = rels(zip, &part);
    let mut sheets: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut date1904 = false;
    for ev in Reader::new(&xml) {
        if let Event::Start { name, attrs, .. } = ev {
            match local(name) {
                "sheet" => {
                    let hidden = matches!(attrs.get("state").as_deref(), Some("hidden" | "veryHidden"));
                    let target = attrs.get("r:id").and_then(|id| rel.get(id.as_ref()).map(|(_, t)| t.clone()));
                    sheets.push((attrs.get("name").unwrap_or_default().to_string(), target, hidden));
                }
                "workbookPr" => date1904 = matches!(attrs.get("date1904").as_deref(), Some("1" | "true")),
                _ => {}
            }
        }
    }
    let by_type = |suffix: &str| rel.values().find(|(k, _)| k.ends_with(suffix)).map(|(_, t)| t.clone());
    let strings = by_type("/sharedStrings").and_then(|p| zip.read_string(&p, MAX_PART).ok()).map(|x| shared_strings(&x)).unwrap_or_default();
    let styles = by_type("/styles").and_then(|p| zip.read_string(&p, MAX_PART).ok()).map(|x| excel_styles(&x)).unwrap_or_default();
    let Some(shown) = sheets.iter().position(|s| !s.2 && s.1.is_some()) else {
        return Ok(Workbook { sheets: sheets.into_iter().map(|s| s.0).collect(), ..Default::default() });
    };
    // 先解壓開頭 2 MiB（500 列通常就在裡面）；還不夠才解壓到 16 MiB
    let path = sheets[shown].1.clone().unwrap_or_default();
    let mut scan = SheetScan { cells: Vec::new(), dimension: None, stopped: false };
    for max in [2 * 1024 * 1024, MAX_BODY_PREFIX] {
        let (sheet_xml, complete) = zip.read_string_prefix(&path, max)?;
        scan = sheet_cells(&sheet_xml, &strings, &styles, date1904, MAX_SHEET_ROWS);
        if complete || scan.stopped {
            break;
        }
        scan.stopped = true;
    }
    let (mut table, mut used_rows, mut used_cols) = assemble(scan.cells, MAX_SHEET_ROWS);
    // 工作表開頭的使用範圍（Excel 一定會寫）比只看讀到的列準確
    if let Some((r, c)) = scan.dimension {
        used_rows = used_rows.max(r);
        used_cols = used_cols.max(c);
    }
    table.more_rows |= scan.stopped || used_rows > table.rows.len().max(MAX_SHEET_ROWS);
    table.more_cols |= used_cols > MAX_COLS;
    let rows_at_least = scan.stopped && scan.dimension.is_none_or(|(r, _)| r <= MAX_SHEET_ROWS);
    Ok(Workbook { sheets: sheets.into_iter().map(|s| s.0).collect(), shown, table, used_rows, used_cols, rows_at_least })
}

struct SheetScan {
    cells: Vec<(usize, usize, String)>,
    /// `<dimension ref="A1:J50000">` 的列數與欄數
    dimension: Option<(usize, usize)>,
    /// 讀到 `max_rows` 列就停了（後面還有）
    stopped: bool,
}

/// 讀出前 `max_rows` 列的儲存格（列依序排列，超過就停，不必解析整個大型工作表）
fn sheet_cells(xml: &str, strings: &[String], styles: &[NumKind], date1904: bool, max_rows: usize) -> SheetScan {
    let mut cells = Vec::new();
    let mut dimension = None;
    let mut stopped = false;
    let mut row = 0usize;
    let mut next_row = 0usize;
    let mut col = 0usize;
    let mut cell: Option<(String, usize)> = None; // (類型, 樣式)
    let mut value = String::new();
    let mut in_value = false;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => match local(name) {
                "dimension" => {
                    let last = attrs.get("ref").and_then(|r| r.rsplit(':').next().and_then(cell_ref));
                    dimension = last.map(|(c, r)| (r + 1, c + 1));
                }
                "row" => {
                    row = attrs.get("r").and_then(|r| r.parse::<usize>().ok()).and_then(|r| r.checked_sub(1)).unwrap_or(next_row);
                    if row >= max_rows {
                        stopped = true;
                        break;
                    }
                    next_row = row + 1;
                    col = 0;
                }
                "c" => {
                    if let Some((c, r)) = attrs.get("r").and_then(|r| cell_ref(&r)) {
                        col = c;
                        row = r;
                    }
                    let kind = attrs.get("t").unwrap_or_default().to_string();
                    let style = attrs.get("s").and_then(|s| s.parse().ok()).unwrap_or(0);
                    value.clear();
                    if empty {
                        col += 1;
                    } else {
                        cell = Some((kind, style));
                    }
                }
                "v" | "t" if !empty && cell.is_some() => in_value = true,
                _ => {}
            },
            Event::End { name } => match local(name) {
                "v" | "t" => in_value = false,
                "c" => {
                    if let Some((kind, style)) = cell.take() {
                        let text = match kind.as_str() {
                            "s" => value.trim().parse::<usize>().ok().and_then(|i| strings.get(i).cloned()).unwrap_or_default(),
                            "b" => (if value.trim() == "1" { "TRUE" } else { "FALSE" }).to_string(),
                            "inlineStr" | "str" | "e" => value.clone(),
                            _ => {
                                if value.trim().is_empty() {
                                    String::new()
                                } else {
                                    format_number(&value, styles.get(style).copied().unwrap_or(NumKind::General), date1904)
                                }
                            }
                        };
                        if !text.trim().is_empty() && row < 1_048_576 && col < 16_384 {
                            let text: String = text.chars().take(MAX_CELL_CHARS).collect();
                            cells.push((row, col, text));
                        }
                    }
                    col += 1;
                }
                _ => {}
            },
            Event::Text(t) => {
                if in_value {
                    value.push_str(&t);
                }
            }
        }
        if cells.len() > 200_000 {
            stopped = true;
            break;
        }
    }
    SheetScan { cells, dimension, stopped }
}

// ---------------------------------------------------------------- OpenDocument

/// 這些元素裡的文字不顯示（註解、追蹤修訂的刪除內容、備忘稿）
fn odf_skipped(local_name: &str) -> bool {
    matches!(local_name, "annotation" | "tracked-changes" | "notes" | "note-body" | "sequence-decls")
}

fn repeat_attr(attrs: &crate::xml::Attrs, key: &str) -> usize {
    attrs.get(key).and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).max(1)
}

/// OpenDocument 文字段落的共用讀法：回傳（區塊, 表格）事件交給呼叫端
/// OpenDocument 的段落樣式是不是標題：「Title」，或以它為基礎的自動樣式
fn odf_title(style: &str, parents: &HashMap<String, String>) -> bool {
    let base = parents.get(style).map_or(style, String::as_str);
    base == "Title"
}

fn odf_text(xml: &str) -> Document {
    let mut out = Out::default();
    // 自動樣式（P1、P2…）→ 基礎樣式；content.xml 裡的自動樣式在內文之前
    let mut parents: HashMap<String, String> = HashMap::new();
    let mut para: Option<(Option<u8>, Option<u8>, String)> = None; // (標題層級, 清單層級, 文字)
    let mut list_depth = 0usize;
    let mut tables: Vec<TableBuild> = Vec::new();
    let mut skip = 0usize;
    for ev in Reader::new(xml) {
        if out.full() {
            out.doc.truncated = true;
            break;
        }
        match ev {
            Event::Start { name, attrs, empty } => {
                let l = local(name);
                if skip > 0 || odf_skipped(l) {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                match l {
                    "style" => {
                        if let (Some(n), Some(parent)) = (attrs.get("style:name"), attrs.get("parent-style-name")) {
                            parents.insert(n.to_string(), parent.to_string());
                        }
                    }
                    "h" if !empty => para = Some((Some(attrs.get("outline-level").and_then(|v| v.parse().ok()).unwrap_or(1u8).clamp(1, 9)), None, String::new())),
                    "p" if !empty && attrs.get("text:style-name").is_some_and(|st| odf_title(&st, &parents)) => para = Some((Some(1), None, String::new())),
                    "p" if !empty => para = Some((None, list_depth.checked_sub(1).map(|d| d.min(8) as u8), String::new())),
                    "list" if !empty => list_depth += 1,
                    "s" => {
                        if let Some(p) = &mut para {
                            p.2.push_str(&" ".repeat(repeat_attr(&attrs, "c").min(100)));
                        }
                    }
                    "tab" => {
                        if let Some(p) = &mut para {
                            p.2.push('\t');
                        }
                    }
                    "line-break" => {
                        if let Some(p) = &mut para {
                            p.2.push('\n');
                        }
                    }
                    "table" if !empty => tables.push(TableBuild::default()),
                    "table-row" if !empty => {
                        if let Some(t) = tables.last_mut() {
                            t.start_row();
                        }
                    }
                    "table-cell" | "covered-table-cell" => {
                        if let Some(t) = tables.last_mut() {
                            t.start_cell();
                            if empty {
                                t.end_cell(repeat_attr(&attrs, "number-columns-repeated").min(MAX_COLS));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::End { name } => {
                let l = local(name);
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match l {
                    "h" | "p" => {
                        let Some((heading, list, text)) = para.take() else { continue };
                        let Some(text) = clean(&text) else { continue };
                        if let Some(t) = tables.last_mut().filter(|t| t.cell.is_some()) {
                            t.add_text(&text);
                            continue;
                        }
                        out.push(match (heading, list) {
                            (Some(level), _) => Block::Heading { level, text },
                            (None, Some(level)) => Block::Item { level, text },
                            _ => Block::Paragraph(text),
                        });
                    }
                    "list" => list_depth = list_depth.saturating_sub(1),
                    "table-cell" | "covered-table-cell" => {
                        if let Some(t) = tables.last_mut() {
                            t.end_cell(1);
                        }
                    }
                    "table-row" => {
                        if let Some(t) = tables.last_mut() {
                            t.end_row(1, MAX_DOC_TABLE_ROWS);
                        }
                    }
                    "table" => {
                        if let Some(t) = tables.pop() {
                            let table = t.finish();
                            if table.cols > 0 {
                                out.push(Block::Table(table));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if skip == 0 {
                    if let Some(p) = &mut para {
                        p.2.push_str(&t);
                    }
                }
            }
        }
    }
    out.doc
}

fn odf_presentation(xml: &str) -> Document {
    let mut out = Out::default();
    let mut slide_blocks: Vec<Block> = Vec::new();
    let mut titles: Vec<Block> = Vec::new();
    let mut frame = Shape::Other;
    let mut para: Option<String> = None;
    let mut list_depth = 0usize;
    let mut skip = 0usize;
    let mut slides = 0usize;
    let mut tables: Vec<TableBuild> = Vec::new();
    let flush = |out: &mut Out, titles: &mut Vec<Block>, rest: &mut Vec<Block>, number: usize| {
        out.push(Block::Slide { number });
        if titles.is_empty() && rest.is_empty() {
            out.push(Block::Paragraph("（這張投影片沒有文字）".into()));
        }
        for b in titles.drain(..).chain(rest.drain(..)) {
            out.push(b);
        }
    };
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => {
                let l = local(name);
                if skip > 0 || odf_skipped(l) {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                match l {
                    "page" if !empty => {
                        slides += 1;
                        titles.clear();
                        slide_blocks.clear();
                    }
                    "frame" => {
                        frame = match attrs.get("presentation:class").as_deref() {
                            Some("title") => Shape::Title,
                            Some("subtitle") => Shape::Subtitle,
                            Some("outline") => Shape::Body,
                            Some("header" | "footer" | "date-time" | "page-number") => Shape::Skip,
                            _ => Shape::Other,
                        }
                    }
                    "list" if !empty => list_depth += 1,
                    "p" | "h" if !empty => para = Some(String::new()),
                    "table" if !empty => tables.push(TableBuild::default()),
                    "table-row" if !empty => {
                        if let Some(t) = tables.last_mut() {
                            t.start_row();
                        }
                    }
                    "table-cell" | "covered-table-cell" => {
                        if let Some(t) = tables.last_mut() {
                            t.start_cell();
                            if empty {
                                t.end_cell(repeat_attr(&attrs, "number-columns-repeated").min(MAX_COLS));
                            }
                        }
                    }
                    "s" => {
                        if let Some(p) = &mut para {
                            p.push(' ');
                        }
                    }
                    "line-break" => {
                        if let Some(p) = &mut para {
                            p.push('\n');
                        }
                    }
                    _ => {}
                }
            }
            Event::End { name } => {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match local(name) {
                    "page" => {
                        if slides <= MAX_SLIDES {
                            flush(&mut out, &mut titles, &mut slide_blocks, slides);
                        } else {
                            out.doc.truncated = true;
                        }
                    }
                    "frame" => frame = Shape::Other,
                    "list" => list_depth = list_depth.saturating_sub(1),
                    "table-cell" | "covered-table-cell" => {
                        if let Some(t) = tables.last_mut() {
                            t.end_cell(1);
                        }
                    }
                    "table-row" => {
                        if let Some(t) = tables.last_mut() {
                            t.end_row(1, MAX_DOC_TABLE_ROWS);
                        }
                    }
                    "table" => {
                        if let Some(t) = tables.pop() {
                            let table = t.finish();
                            if table.cols > 0 {
                                slide_blocks.push(Block::Table(table));
                            }
                        }
                    }
                    "p" | "h" => {
                        let Some(text) = para.take().and_then(|t| clean(&t)) else { continue };
                        if let Some(t) = tables.last_mut().filter(|t| t.cell.is_some()) {
                            t.add_text(&text);
                            continue;
                        }
                        let level = list_depth.saturating_sub(1).min(8) as u8;
                        match frame {
                            Shape::Skip => {}
                            Shape::Title => titles.push(Block::Heading { level: 2, text }),
                            Shape::Body => slide_blocks.push(Block::Item { level, text }),
                            _ => slide_blocks.push(Block::Paragraph(text)),
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if skip == 0 {
                    if let Some(p) = &mut para {
                        p.push_str(&t);
                    }
                }
            }
        }
    }
    out.doc.slides = slides;
    out.doc
}

fn odf_sheet(xml: &str) -> Workbook {
    let mut sheets: Vec<String> = Vec::new();
    let mut depth = 0usize; // 表格層數（只讀第一個工作表，表格裡的表格不算工作表）
    let mut cells: Vec<(usize, usize, String)> = Vec::new();
    let (mut row, mut col) = (0usize, 0usize);
    let mut cell: Option<(String, usize)> = None; // (文字, 重複次數)
    let mut row_repeat = 1usize;
    let mut row_has_content = false;
    let mut row_cells: Vec<(usize, String)> = Vec::new();
    let mut para: Option<String> = None;
    let mut skip = 0usize;
    for ev in Reader::new(xml) {
        match ev {
            Event::Start { name, attrs, empty } => {
                let l = local(name);
                if skip > 0 || odf_skipped(l) {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                match l {
                    "table" if !empty => {
                        depth += 1;
                        if depth == 1 {
                            sheets.push(attrs.get("name").unwrap_or_default().to_string());
                        }
                    }
                    "table-row" if depth == 1 && sheets.len() == 1 => {
                        row_repeat = repeat_attr(&attrs, "number-rows-repeated");
                        row_cells.clear();
                        row_has_content = false;
                        col = 0;
                        if empty {
                            row += row_repeat;
                        }
                    }
                    "table-cell" | "covered-table-cell" if depth == 1 && sheets.len() == 1 => {
                        let repeat = repeat_attr(&attrs, "number-columns-repeated");
                        if empty {
                            col += repeat;
                        } else {
                            cell = Some((String::new(), repeat));
                        }
                    }
                    "p" if !empty && cell.is_some() => para = Some(String::new()),
                    "s" => {
                        if let Some(p) = &mut para {
                            p.push_str(&" ".repeat(repeat_attr(&attrs, "c").min(100)));
                        }
                    }
                    "line-break" => {
                        if let Some(p) = &mut para {
                            p.push('\n');
                        }
                    }
                    _ => {}
                }
            }
            Event::End { name } => {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match local(name) {
                    "table" => depth = depth.saturating_sub(1),
                    "p" => {
                        if let (Some(text), Some(c)) = (para.take(), &mut cell) {
                            if !c.0.is_empty() {
                                c.0.push('\n');
                            }
                            c.0.push_str(&text);
                        }
                    }
                    "table-cell" | "covered-table-cell" if depth == 1 && sheets.len() == 1 => {
                        if let Some((text, repeat)) = cell.take() {
                            if !text.trim().is_empty() {
                                row_has_content = true;
                                for k in 0..repeat.min(MAX_COLS) {
                                    row_cells.push((col + k, text.chars().take(MAX_CELL_CHARS).collect()));
                                }
                            }
                            col += repeat;
                        }
                    }
                    "table-row" if depth == 1 && sheets.len() == 1 => {
                        let times = if row_has_content { row_repeat.min(MAX_SHEET_ROWS) } else { 1 };
                        for k in 0..times {
                            for (c, t) in &row_cells {
                                cells.push((row + k, *c, t.clone()));
                            }
                        }
                        row += row_repeat;
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if skip == 0 {
                    if let Some(p) = &mut para {
                        p.push_str(&t);
                    }
                }
            }
        }
        if cells.len() > 200_000 {
            break;
        }
    }
    let (table, used_rows, used_cols) = assemble(cells, MAX_SHEET_ROWS);
    Workbook { sheets, shown: 0, table, used_rows, used_cols, rows_at_least: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(format: Format, data: &[u8]) -> Document {
        match extract(format, data).unwrap() {
            Office::Document(d) => d,
            other => panic!("not a document: {other:?}"),
        }
    }

    fn book(format: Format, data: &[u8]) -> Workbook {
        match extract(format, data).unwrap() {
            Office::Workbook(w) => w,
            other => panic!("not a workbook: {other:?}"),
        }
    }

    fn texts(d: &Document) -> Vec<String> {
        d.blocks
            .iter()
            .map(|b| match b {
                Block::Heading { level, text } => format!("H{level} {text}"),
                Block::Paragraph(t) => format!("P {t}"),
                Block::Item { level, text } => format!("L{level} {text}"),
                Block::Slide { number } => format!("== {number}"),
                Block::Table(t) => format!("T {}", t.rows.iter().map(|r| r.join("|")).collect::<Vec<_>>().join(" / ")),
            })
            .collect()
    }

    #[test]
    fn word_document() {
        let d = doc(Format::Word, include_bytes!("../testdata/office/sample.docx"));
        assert_eq!(
            texts(&d),
            [
                "H1 專案報告",
                "P 第一段：A & B\t有定位點",
                "H2 第二章",
                "L0 項目一",
                "L1 子項目",
                "P 取消編號的段落",
                "T 名稱|數量 / 蘋果|3 / 香蕉\n（熟）|12",
                "P 換行前\n換行後",
                "H3 Heading3 內建樣式",
                "L1 樣式清單第二層",
                "P 樣式清單但取消編號",
            ]
        );
        assert!(!d.truncated);
        assert_eq!(caption(Format::Word, &Office::Document(d)), "Word 文件 · 10 段 · 1 個表格");
    }

    #[test]
    fn powerpoint_slides_in_presentation_order() {
        let d = doc(Format::PowerPoint, include_bytes!("../testdata/office/sample.pptx"));
        assert_eq!(d.slides, 2);
        assert_eq!(
            texts(&d),
            ["== 1", "H2 開場", "P 副標題文字", "== 2", "H2 重點", "L0 第一點", "L1 細節", "T 季|營收 / Q1|100", "P 文字方塊"]
        );
        assert_eq!(caption(Format::PowerPoint, &Office::Document(d)), "PowerPoint 簡報 · 2 張投影片");
    }

    #[test]
    fn excel_first_visible_sheet() {
        let w = book(Format::Excel, include_bytes!("../testdata/office/sample.xlsx"));
        assert_eq!(w.sheets, ["隱藏", "銷售", "說明"]);
        assert_eq!(w.shown, 1);
        let t = &w.table;
        assert_eq!(t.rows[0], ["日期", "品項", "金額", "比例", "已付", "備註"]);
        assert_eq!(t.rows[1], ["2023/1/1", "蘋果", "1234.5", "12.50%", "TRUE", "直接輸入"]);
        assert_eq!(t.rows[2], ["2024/2/29 13:45", "香蕉", "0.3", "", "FALSE", ""]);
        assert_eq!(t.rows[3], ["", "", "", "", "", ""], "空白列保留位置");
        assert_eq!(t.rows[4][1], "芭樂");
        assert_eq!(t.rows[4][2], "1000000");
        assert_eq!((w.used_rows, w.used_cols, t.cols), (5, 6, 6));
        assert_eq!(caption(Format::Excel, &Office::Workbook(w)), "銷售 · 共 3 個工作表 · 5 列 × 6 欄");
        // 只讀前面的列；總列數取自 <dimension>
        let xml = r#"<worksheet><dimension ref="A1:C9000"/><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="2"><c r="B2"><v>2</v></c></row><row r="3"><c r="C3"><v>3</v></c></row></sheetData></worksheet>"#;
        let scan = sheet_cells(xml, &[], &[], false, 2);
        assert!(scan.stopped);
        assert_eq!(scan.dimension, Some((9000, 3)));
        assert_eq!(scan.cells.len(), 2);
    }

    #[test]
    fn opendocument() {
        let d = doc(Format::OdText, include_bytes!("../testdata/office/sample.odt"));
        assert_eq!(texts(&d), ["H1 文件標題", "H1 標題", "P 內文  兩個空格", "L0 清單一", "L1 巢狀清單", "T A|B / 1|2"]);
        let p = doc(Format::OdPresentation, include_bytes!("../testdata/office/sample.odp"));
        assert_eq!(p.slides, 2);
        assert_eq!(texts(&p), ["== 1", "H2 簡報標題", "L0 大綱一", "T 甲|乙", "== 2", "P （這張投影片沒有文字）"]);
        let s = book(Format::OdSheet, include_bytes!("../testdata/office/sample.ods"));
        assert_eq!(s.sheets, ["工作表一", "工作表二"]);
        assert_eq!(s.table.rows, [vec!["名稱", "", "值"], vec!["x", "", "1"], vec!["x", "", "1"], vec!["", "", "末"]]);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(extract(Format::Word, b"not a zip").is_err());
        let mut ole = crate::zip::OLE_SIGNATURE.to_vec();
        ole.resize(1024, 0);
        assert!(extract(Format::Excel, &ole).unwrap_err().contains("密碼保護"));
        // 少了主文件的 ZIP
        assert!(extract(Format::Word, include_bytes!("../testdata/sample.zip")).is_err());
    }

    #[test]
    fn corrupted_documents_never_panic() {
        let files: [(Format, &[u8]); 6] = [
            (Format::Word, include_bytes!("../testdata/office/sample.docx")),
            (Format::PowerPoint, include_bytes!("../testdata/office/sample.pptx")),
            (Format::Excel, include_bytes!("../testdata/office/sample.xlsx")),
            (Format::OdText, include_bytes!("../testdata/office/sample.odt")),
            (Format::OdPresentation, include_bytes!("../testdata/office/sample.odp")),
            (Format::OdSheet, include_bytes!("../testdata/office/sample.ods")),
        ];
        let mut x: u32 = 3;
        for (format, data) in files {
            for round in 0..150 {
                let mut d = data.to_vec();
                for _ in 0..(1 + round % 5) {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    let i = (x as usize) % d.len();
                    d[i] = (x >> 24) as u8;
                }
                let _ = extract(format, &d);
            }
        }
        // XML 層級：亂七八糟的標籤
        for xml in ["<w:p><w:t>a", "</w:p></w:tbl></w:tc>", "<w:tbl><w:tr><w:tc><w:p><w:t>x</w:t></w:p>", "<c r='ZZZZ99'><v>1</v></c>", "<row r='0'><c r='A0'/></row>"] {
            let _ = sheet_cells(xml, &[], &[], false, 10);
            let _ = slide_blocks(xml);
            let _ = odf_text(xml);
            let _ = odf_sheet(xml);
            let _ = odf_presentation(xml);
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(cell_ref("A1"), Some((0, 0)));
        assert_eq!(cell_ref("AB12"), Some((27, 11)));
        assert_eq!(cell_ref("XFD1048576"), Some((16383, 1_048_575)));
        assert_eq!(cell_ref("A0"), None);
        assert_eq!(cell_ref("12"), None);
        assert_eq!((column_name(0), column_name(25), column_name(26), column_name(701), column_name(702)), ("A".into(), "Z".into(), "AA".into(), "ZZ".into(), "AAA".into()));
        assert_eq!(general_number("0.30000000000000004"), "0.3");
        assert_eq!(general_number("1234.5"), "1234.5");
        assert_eq!(general_number("42"), "42");
        assert_eq!(general_number("1E+20"), "1.0000e20");
        assert_eq!(general_number("abc"), "abc");
        assert_eq!(excel_date(1.0, false, true, false), "1900/1/1");
        assert_eq!(excel_date(59.0, false, true, false), "1900/2/28");
        assert_eq!(excel_date(61.0, false, true, false), "1900/3/1");
        assert_eq!(excel_date(44927.0, false, true, false), "2023/1/1");
        assert_eq!(excel_date(45351.572916666664, false, true, true), "2024/2/29 13:45");
        assert_eq!(excel_date(0.0, true, true, false), "1904/1/1");
        assert_eq!(excel_date(0.5, false, false, true), "12:00");
        assert_eq!(classify_format("yyyy/mm/dd hh:mm"), NumKind::DateTime { date: true, time: true });
        assert_eq!(classify_format("[$-404]e/m/d"), NumKind::DateTime { date: true, time: false });
        assert_eq!(classify_format("0.00%"), NumKind::Percent(2));
        assert_eq!(classify_format("#,##0\"天\""), NumKind::General);
        assert_eq!(classify_format("[Red]0.0"), NumKind::General);
        assert_eq!(classify_format("General"), NumKind::General);
        assert_eq!(classify_format("0.00E+00"), NumKind::General);
        assert_eq!(classify_format("yyyy\\-mm\\-dd"), NumKind::DateTime { date: true, time: false });
        assert_eq!(classify_format("[h]:mm:ss"), NumKind::DateTime { date: false, time: true });
        assert_eq!(resolve("ppt/", "slides/slide1.xml"), "ppt/slides/slide1.xml");
        assert_eq!(resolve("xl/", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("ppt/slides/", "../media/a.png"), "ppt/media/a.png");
        assert_eq!(cell_align("1234.5"), Align::Right);
        assert_eq!(cell_align("12.50%"), Align::Right);
        assert_eq!(cell_align("1,000"), Align::Right);
        assert_eq!(cell_align("2024/2/29 13:45"), Align::Right);
        assert_eq!(cell_align("TRUE"), Align::Center);
        assert_eq!(cell_align("蘋果"), Align::Left);
        assert_eq!(cell_align("12 個"), Align::Left);
        assert_eq!(cell_align("inf"), Align::Left);
        assert_eq!(cell_align(""), Align::Left);
        assert_eq!(fit_columns(&[50, 60], 200, 30), [50, 60]);
        assert_eq!(fit_columns(&[50, 400, 300], 400, 30), [50, 175, 175]);
        assert_eq!(fit_columns(&[100, 100, 100, 100], 100, 40), [40, 40, 40, 40]);
        assert_eq!(fit_columns(&[], 100, 40), Vec::<i32>::new());
        assert!(has_thumbnail(include_bytes!("../testdata/office/sample.pptx")));
        assert!(!has_thumbnail(include_bytes!("../testdata/office/sample.docx")));
        assert!(extract(Format::Word, include_bytes!("../testdata/sample.zip")).unwrap_err().starts_with("無法讀取文件內容"));
        assert_eq!(Format::from_ext("pptx"), Some(Format::PowerPoint));
        assert_eq!(Format::from_ext("doc"), None);
        assert!(is_legacy_ext("xls"));
    }
}

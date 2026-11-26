//! Transfer a tab over private pipes. The source closes only after the destination has loaded it.
use super::*;
use feather_core::json::{self, Value};
use feather_core::searchquery::Ymd;
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};

pub(super) struct Transfer {
    pub x: i32,
    pub y: i32,
    path: String,
    back: Vec<String>,
    forward: Vec<String>,
    selected: Vec<String>,
    query: String,
    search: bool,
    global: bool,
    recursive: bool,
    sort: usize,
    desc: bool,
    category: Category,
    date: Option<DateSpec>,
    size: Option<SizeRange>,
}

fn encode_date(date: Option<DateSpec>) -> String {
    match date {
        None => "-".into(),
        Some(DateSpec::LastDays(n)) => format!("days {n}"),
        Some(DateSpec::Range { from, to }) => {
            let endpoint = |d: Option<Ymd>| d.map_or("-".into(), |d| d.text());
            format!("range {} {}", endpoint(from), endpoint(to))
        }
        Some(d) => DATE_PRESETS.iter().position(|p| *p == d).unwrap().to_string(),
    }
}

fn decode_date(text: &str) -> Option<Option<DateSpec>> {
    fn endpoint(s: &str) -> Option<Option<Ymd>> {
        if s == "-" { return Some(None) }
        let mut n = s.split('-');
        Ymd::new(n.next()?.parse().ok()?, n.next()?.parse().ok()?, n.next()?.parse().ok()?).map(Some)
    }
    if text == "-" { return Some(None) }
    let fields: Vec<_> = text.split_whitespace().collect();
    Some(Some(match fields.as_slice() {
        ["range", a, b] => DateSpec::Range { from: endpoint(a)?, to: endpoint(b)? },
        ["days", n] => DateSpec::LastDays(n.parse().ok()?),
        [n] => *DATE_PRESETS.get(n.parse::<usize>().ok()?)?,
        _ => return None,
    }))
}

impl Transfer {
    fn encode(&self) -> String {
        let strings = |v: &[String]| format!("[{}]", v.iter().map(|s| json::quote(s)).collect::<Vec<_>>().join(","));
        let flags = format!("{} {} {} {} {} {} {} {}", self.x, self.y, self.search, self.global, self.recursive, self.sort, self.desc, self.category as usize);
        let size = self.size.map_or("-".into(), |s| format!("{} {}", s.min, s.max));
        [flags, size, encode_date(self.date), json::quote(&self.path), json::quote(&self.query),
            strings(&self.back), strings(&self.forward), strings(&self.selected)].join("\n")
    }

    fn decode(data: &str) -> Option<Self> {
        let mut lines = data.lines();
        let mut flags = lines.next()?.split_whitespace();
        let mut sizes = lines.next()?.split_whitespace();
        let min = sizes.next()?;
        let size = if min == "-" { None } else { Some(SizeRange { min: min.parse().ok()?, max: sizes.next()?.parse().ok()? }) };
        let date = decode_date(lines.next()?)?;
        let string = |s: &str| match json::parse(s).ok()? { Value::Str(s) => Some(s), _ => None };
        let list = |s: &str| {
            let Value::Arr(v) = json::parse(s).ok()? else { return None };
            v.into_iter().map(|v| match v { Value::Str(s) => Some(s), _ => None }).collect::<Option<Vec<_>>>()
        };
        Some(Self { x: flags.next()?.parse().ok()?, y: flags.next()?.parse().ok()?,
            search: flags.next()?.parse().ok()?, global: flags.next()?.parse().ok()?, recursive: flags.next()?.parse().ok()?,
            sort: flags.next()?.parse().ok()?, desc: flags.next()?.parse().ok()?,
            category: *category::ALL.get(flags.next()?.parse::<usize>().ok()?)?, size, date,
            path: string(lines.next()?)?, query: string(lines.next()?)?, back: list(lines.next()?)?,
            forward: list(lines.next()?)?, selected: list(lines.next()?)?,
        })
    }
}

pub(in crate::win) fn run_detached() -> i32 {
    let mut data = String::new();
    if std::io::stdin().read_to_string(&mut data).is_err() { return 1 }
    let Some(t) = Transfer::decode(&data) else { return 1 };
    run_window(Vec::new(), None, Some(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_search_tab_with_unc_history_and_selection() {
        let t = Transfer {
            x: -1500, y: 20, path: r"\\nas\專案".into(), back: vec![r"C:\舊資料".into()], forward: vec![],
            selected: vec![r"\\nas\專案\新檔案.txt".into()], query: "word \"two words\"".into(),
            search: true, global: false, recursive: true, sort: 2, desc: true, category: Category::Document,
            date: Some(DateSpec::Range { from: Ymd::new(2026, 11, 1), to: None }), size: Some(SizeRange { min: 1, max: u64::MAX }),
        };
        let data = t.encode();
        assert_eq!(Transfer::decode(&data).unwrap().encode(), data);
        assert!(Transfer::decode(&data[..data.find('\n').unwrap()]).is_none());
    }
}

fn launch(transfer: Transfer) -> Result<(), String> {
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--detached-tab").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    child.stdin.take().unwrap().write_all(transfer.encode().as_bytes()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut reply).map_err(|e| e.to_string())?;
    if reply.trim() == "ready" { Ok(()) } else {
        Err(tr!("新視窗未能載入分頁。", "The new window could not load the tab.").into())
    }
}

impl App {
    pub(super) fn detach_tab(&mut self, side: Side, index: usize, pt: POINT) {
        let Some(&id) = self.ws.group(side).tabs.get(index) else { return };
        let Some(tab) = self.tabs.get(&id) else { return };
        if self.detaching.contains(&id) { return }
        let x = pt.x - self.s(50);
        let y = pt.y - self.s(15);
        // A sole tab already owns its window. Moving it keeps the primary shell endpoint alive.
        if self.ws.group(Side::Left).tabs.len() == 1 && !self.ws.dual {
            unsafe {
                if IsZoomed(self.hwnd) != 0 { ShowWindow(self.hwnd, SW_RESTORE); }
                SetWindowPos(self.hwnd, 0, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);
            }
            return;
        }
        let generation = tab.generation;
        let transfer = Transfer { x, y, path: tab.display_path(), back: tab.back.clone(), forward: tab.forward.clone(),
            selected: self.bound_selection_paths(id), query: tab.query.clone(), search: tab.search_mode, global: tab.search_global,
            recursive: tab.recursive, sort: tab.sort_col, desc: tab.desc, category: tab.filter, date: tab.date.map(|d| d.0), size: tab.size };
        self.detaching.push(id);
        std::thread::spawn(move || post(WorkerMsg::Detached(id, generation, launch(transfer))));
    }

    pub(super) fn finish_detach(&mut self, id: TabId, generation: u64, result: Result<(), String>) {
        self.detaching.retain(|t| *t != id);
        if let Err(error) = result { self.show_error(&error); return }
        if self.tabs.get(&id).is_none_or(|t| t.generation != generation) { return }
        if let Some(side) = self.ws.side_of(id) {
            if let Some(i) = self.ws.group(side).tabs.iter().position(|t| *t == id) { self.close_tab(side, i); }
        }
    }

    pub(super) fn receive_tab(&mut self, mut transfer: Transfer) {
        let id = self.active_tab_id();
        let tab = self.tabs.get_mut(&id).unwrap();
        tab.back = std::mem::take(&mut transfer.back);
        tab.forward = std::mem::take(&mut transfer.forward);
        tab.recursive = transfer.recursive;
        if !transfer.search { tab.sort_col = transfer.sort; tab.desc = transfer.desc; }
        let path = transfer.path.clone();
        self.incoming_tab = Some((id, transfer));
        self.navigate(id, &path, NavKind::NoHistory, Vec::new());
        if !self.tabs[&id].loading { self.finish_incoming_tab(id, false); }
    }

    pub(super) fn finish_incoming_tab(&mut self, id: TabId, ok: bool) {
        if self.incoming_tab.as_ref().is_none_or(|(incoming, _)| *incoming != id) { return }
        let (_, transfer) = self.incoming_tab.take().unwrap();
        if ok {
            let tab = self.tabs.get_mut(&id).unwrap();
            tab.filter = transfer.category;
            tab.date = transfer.date.map(|d| (d, date_range(&d)));
            tab.size = transfer.size;
            tab.refilter();
            tab.resummarize();
            if transfer.search {
                tab.sort_col = transfer.sort;
                tab.desc = transfer.desc;
                self.run_search(id, transfer.query, transfer.global, true, None);
                self.tabs.get_mut(&id).unwrap().pending_select = transfer.selected;
            } else {
                tab.pending_select = transfer.selected;
                self.refresh_list(Side::Left, true);
            }
            self.update_chrome();
        }
        let _ = writeln!(std::io::stdout(), "{}", if ok { "ready" } else { "failed" });
    }
}

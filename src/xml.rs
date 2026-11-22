//! 寬鬆的 XML 讀取器（逐一產生開始標籤、結束標籤與文字），給 Office 文件預覽用。只用標準函式庫。
//!
//! 不驗證結構、不處理 DTD；註解、處理指令與 DOCTYPE 直接略過，CDATA 當成文字。
//! 文字與屬性值會解開 `&amp;` 等實體與 `&#…;` 字元參照。格式錯誤時就停止，不會 panic。

use std::borrow::Cow;

#[derive(Debug, PartialEq)]
pub enum Event<'a> {
    /// `<name …>`；`empty` 表示 `<name …/>`（不會再有對應的 End）
    Start { name: &'a str, attrs: Attrs<'a>, empty: bool },
    End { name: &'a str },
    Text(Cow<'a, str>),
}

/// 標籤裡的屬性（原始字串，需要時才解析）
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct Attrs<'a>(&'a str);

impl<'a> Attrs<'a> {
    /// 依名稱取屬性值（完整名稱相同，或冒號後的本地名稱相同）
    pub fn get(&self, key: &str) -> Option<Cow<'a, str>> {
        let mut rest = self.0;
        loop {
            rest = rest.trim_start();
            let eq = rest.find('=')?;
            let name = rest[..eq].trim();
            let after = rest[eq + 1..].trim_start();
            let quote = after.chars().next()?;
            if quote != '"' && quote != '\'' {
                return None;
            }
            let body = &after[1..];
            let end = body.find(quote)?;
            let value = &body[..end];
            if name == key || local(name) == key {
                return Some(unescape(value));
            }
            rest = &body[end + 1..];
        }
    }
}

/// 冒號後面的本地名稱（`w:t` → `t`）
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub struct Reader<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(s: &'a str) -> Reader<'a> {
        Reader { s, pos: 0 }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Event<'a>> {
        loop {
            let rest = self.s.get(self.pos..)?;
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.pos += end;
                return Some(Event::Text(unescape(&rest[..end])));
            }
            if let Some(body) = rest.strip_prefix("<!--") {
                let end = body.find("-->")?;
                self.pos += 4 + end + 3;
                continue;
            }
            if let Some(body) = rest.strip_prefix("<![CDATA[") {
                let end = body.find("]]>")?;
                self.pos += 9 + end + 3;
                return Some(Event::Text(Cow::Borrowed(&body[..end])));
            }
            if rest.starts_with("<?") || rest.starts_with("<!") {
                let end = rest.find('>')?;
                self.pos += end + 1;
                continue;
            }
            // 一般標籤：找到不在引號裡的 >
            let bytes = rest.as_bytes();
            let mut quote = 0u8;
            let mut end = None;
            for (i, &b) in bytes.iter().enumerate().skip(1) {
                if quote != 0 {
                    if b == quote {
                        quote = 0;
                    }
                } else if b == b'"' || b == b'\'' {
                    quote = b;
                } else if b == b'>' {
                    end = Some(i);
                    break;
                }
            }
            let end = end?;
            let inner = &rest[1..end];
            self.pos += end + 1;
            if let Some(name) = inner.strip_prefix('/') {
                return Some(Event::End { name: name.trim() });
            }
            let (inner, empty) = match inner.strip_suffix('/') {
                Some(i) => (i, true),
                None => (inner, false),
            };
            let split = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
            let name = &inner[..split];
            if name.is_empty() {
                continue;
            }
            return Some(Event::Start { name, attrs: Attrs(&inner[split..]), empty });
        }
    }
}

/// 解開實體與字元參照；沒有 & 時直接借用原字串。
pub fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(semi) = after.find(';').filter(|&n| n <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let entity = &after[..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(s: &str) -> Vec<String> {
        Reader::new(s)
            .map(|e| match e {
                Event::Start { name, empty, .. } => format!("<{name}{}>", if empty { "/" } else { "" }),
                Event::End { name } => format!("</{name}>"),
                Event::Text(t) => format!("'{t}'"),
            })
            .collect()
    }

    #[test]
    fn basic_document() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<!-- 註解 --><w:document xmlns:w="x"><w:p><w:r><w:t xml:space="preserve">A &amp; B &lt;中&gt; &#x4E2D;&#25991;</w:t></w:r><w:br/></w:p></w:document>"#;
        assert_eq!(
            events(xml),
            ["'\n'", "<w:document>", "<w:p>", "<w:r>", "<w:t>", "'A & B <中> 中文'", "</w:t>", "</w:r>", "<w:br/>", "</w:p>", "</w:document>"]
        );
    }

    #[test]
    fn attributes() {
        let mut r = Reader::new(r#"<c r="B12" t='s' s = "3" title="a &quot;b&quot; > c"/>"#);
        let Some(Event::Start { name, attrs, empty }) = r.next() else { panic!() };
        assert_eq!((name, empty), ("c", true));
        assert_eq!(attrs.get("r").as_deref(), Some("B12"));
        assert_eq!(attrs.get("t").as_deref(), Some("s"));
        assert_eq!(attrs.get("s").as_deref(), Some("3"));
        assert_eq!(attrs.get("title").as_deref(), Some("a \"b\" > c"));
        assert_eq!(attrs.get("missing"), None);
        let mut r = Reader::new(r#"<w:pStyle w:val="Heading1"/>"#);
        let Some(Event::Start { attrs, .. }) = r.next() else { panic!() };
        assert_eq!(attrs.get("w:val").as_deref(), Some("Heading1"));
        assert_eq!(attrs.get("val").as_deref(), Some("Heading1"));
    }

    #[test]
    fn cdata_and_bad_entities() {
        assert_eq!(events("<a><![CDATA[x < y & z]]></a>"), ["<a>", "'x < y & z'", "</a>"]);
        assert_eq!(unescape("AT&T &bogus; &#xZZ; & end"), "AT&T &bogus; &#xZZ; & end");
        assert_eq!(local("a:t"), "t");
        assert_eq!(local("t"), "t");
    }

    #[test]
    fn malformed_never_panics() {
        for s in ["<", "<a", "<a b='>", "<!--", "<![CDATA[x", "</", "<>", "<a/><", "&", "&#99999999999;", "<中 文='x'>字</中>", "<a b=\"\u{FFFF}\">"] {
            let _: Vec<_> = Reader::new(s).collect();
        }
        let mut x: u32 = 5;
        let alphabet = ['<', '>', '/', '"', '\'', '=', '&', ';', '#', 'a', ' ', '中', '!', '-', '[', ']', '?'];
        for _ in 0..2000 {
            let s: String = (0..30)
                .map(|_| {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    alphabet[(x >> 24) as usize % alphabet.len()]
                })
                .collect();
            for e in Reader::new(&s) {
                if let Event::Start { attrs, .. } = e {
                    let _ = attrs.get("a");
                }
            }
        }
    }
}

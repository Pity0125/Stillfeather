//! 小型 JSON 解析／輸出，只用於設定檔。
//!
//! 需要讀得懂 C# System.Text.Json 寫出的檔案：它預設會把中文等非 ASCII
//! 字元寫成 `\uXXXX`（含代理對），所以這裡完整處理跳脫序列。

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// 以不分大小寫的鍵取值（System.Text.Json 預設區分大小寫，但寬鬆讀取較安全）。
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(items) => items
                .iter()
                .find(|(k, _)| k == key)
                .or_else(|| items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
                .map(|(_, v)| v),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 16;

pub fn parse(text: &str) -> Result<Value, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut p = Parser { s: text.as_bytes(), i: 0, text };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("JSON 第 {} 個位元組之後有多餘內容", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    text: &'a str,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\r' | b'\n') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON 格式錯誤（位置 {}）：{}", self.i, what))
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return self.err("巢狀層數過深");
        }
        match self.s.get(self.i) {
            None => self.err("內容意外結束"),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => self.err("無法辨識的值"),
        }
    }

    fn literal(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            self.err("無法辨識的常值")
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'e' | b'E' | b'+' | b'-'))
        {
            self.i += 1;
        }
        let slice = &self.text[start..self.i];
        slice.parse::<f64>().map(Value::Num).or_else(|_| self.err("數字格式錯誤"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let Some(digits) = self.s.get(self.i..self.i + 4) else {
            return self.err("\\u 跳脫不完整");
        };
        // 逐位元組檢查：`\u` 後面若接中文等多位元組字元，直接切字串會落在字元中間而閃退。
        let mut v = 0u32;
        for &b in digits {
            let Some(d) = (b as char).to_digit(16) else { return self.err("\\u 跳脫不是十六進位") };
            v = v * 16 + d;
        }
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(&c) = self.s.get(self.i) else { return self.err("字串未結束") };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else { return self.err("跳脫序列不完整") };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                if self.s.get(self.i) == Some(&b'\\') && self.s.get(self.i + 1) == Some(&b'u') {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return self.err("代理對不完整");
                                    }
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    return self.err("代理對不完整");
                                }
                            } else {
                                hi
                            };
                            match char::from_u32(code) {
                                Some(ch) => out.push(ch),
                                None => return self.err("無效的 Unicode 字元"),
                            }
                        }
                        _ => return self.err("未知的跳脫字元"),
                    }
                }
                c if c < 0x20 => return self.err("字串內含控制字元"),
                _ => {
                    // 複製一段連續的一般 UTF-8 位元組
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' && self.s[self.i] >= 0x20 {
                        self.i += 1;
                    }
                    out.push_str(&self.text[start..self.i]);
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return self.err("陣列缺少 , 或 ]"),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Obj(items));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return self.err("物件鍵必須是字串");
            }
            let key = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return self.err("物件缺少 :");
            }
            self.i += 1;
            self.ws();
            let v = self.value(depth + 1)?;
            items.push((key, v));
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Obj(items));
                }
                _ => return self.err("物件缺少 , 或 }"),
            }
        }
    }
}

/// 輸出 JSON 字串常值；非 ASCII 字元直接以 UTF-8 保留。
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csharp_style_output() {
        let text = "\u{FEFF}{\r\n  \"ThemeMode\": \"Dark\",\r\n  \"AccentColor\": null,\r\n  \"SidebarWidth\": 230,\r\n  \"CompactRows\": true,\r\n  \"Favorites\": [\"C:\\\\Users\\\\pity0\\\\\\u684C\\u9762\", \"D:\\\\\\uD83D\\uDE00\"]\r\n}";
        let v = parse(text).unwrap();
        assert_eq!(v.get("ThemeMode"), Some(&Value::Str("Dark".into())));
        assert_eq!(v.get("accentcolor"), Some(&Value::Null));
        assert_eq!(v.get("SidebarWidth"), Some(&Value::Num(230.0)));
        assert_eq!(v.get("CompactRows"), Some(&Value::Bool(true)));
        match v.get("Favorites") {
            Some(Value::Arr(items)) => {
                assert_eq!(items[0], Value::Str("C:\\Users\\pity0\\桌面".into()));
                assert_eq!(items[1], Value::Str("D:\\😀".into()));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{").is_err());
        assert!(parse("{\"a\":1,}").is_err());
        assert!(parse("[1 2]").is_err());
        assert!(parse("\"\\uD800\"").is_err());
        assert!(parse("{} x").is_err());
        let deep = "[".repeat(40) + &"]".repeat(40);
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn bad_unicode_escape_is_an_error_not_a_crash() {
        // 設定檔被手動改壞時：`\u` 後面接中文、不足四碼或不是十六進位
        assert!(parse("\"\\u中文\"").is_err());
        assert!(parse("\"\\u00中\"").is_err());
        assert!(parse("\"\\u12\"").is_err());
        assert!(parse("\"\\uZZZZ\"").is_err());
        assert!(parse("\"\\u+123\"").is_err());
        assert!(parse("{\"a\":\"\\uD83D\\u中\"}").is_err());
        assert_eq!(parse("\"\\u00e9\\u00C9\"").unwrap(), Value::Str("éÉ".into()));
    }

    #[test]
    fn quote_roundtrip() {
        let s = "a\"b\\c\n中文\u{1}";
        let q = quote(s);
        assert_eq!(parse(&q).unwrap(), Value::Str(s.into()));
    }
}

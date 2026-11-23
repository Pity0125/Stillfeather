//! 快速預覽（空白鍵）的純邏輯：依檔名決定預覽方式、文字解碼、程式碼語法上色。
//!
//! 語法上色是輕量的逐行掃描（不是完整的語法分析）：關鍵字、型別、函式呼叫、字串、數字、
//! 註解、常數、變數，以及標記語言的標籤與屬性。跨行的區塊註解與多行字串會延續到下一行。

use crate::paths;

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "jpe", "jfif", "png", "bmp", "dib", "gif", "tif", "tiff", "ico", "webp", "heic", "heif", "avif"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "avi", "wmv", "mkv", "webm", "mpg", "mpeg", "3gp", "3g2", "mts", "m2ts", "flv", "asf"];

/// 預覽方式
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Pdf,
    Text(Lang),
    /// Word、Excel、PowerPoint 與 OpenDocument（讀出文字與表格）
    Office(crate::office::Format),
    /// 舊版 Office 二進位格式（.doc／.xls／.ppt）：沒有內容預覽，顯示說明
    LegacyOffice,
    /// 沒有內容預覽（顯示資訊卡）；檔名沒有副檔名時，讀取端可再試著當純文字
    Other,
}

/// 文字與程式碼的語言（決定上色規則）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Plain,
    Rust,
    C,
    CSharp,
    Java,
    JavaScript,
    TypeScript,
    Go,
    Swift,
    Kotlin,
    Php,
    Css,
    Python,
    Ruby,
    Shell,
    PowerShell,
    Batch,
    Sql,
    Lua,
    Html,
    Xml,
    Json,
    Yaml,
    Toml,
    Ini,
    Markdown,
}

impl Lang {
    pub fn label(self) -> &'static str {
        match self {
            Lang::Plain => tr!("純文字", "Plain text"),
            Lang::Rust => "Rust",
            Lang::C => tr!("C／C++", "C/C++"),
            Lang::CSharp => "C#",
            Lang::Java => "Java",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Go => "Go",
            Lang::Swift => "Swift",
            Lang::Kotlin => "Kotlin",
            Lang::Php => "PHP",
            Lang::Css => "CSS",
            Lang::Python => "Python",
            Lang::Ruby => "Ruby",
            Lang::Shell => "Shell",
            Lang::PowerShell => "PowerShell",
            Lang::Batch => tr!("批次檔", "Batch file"),
            Lang::Sql => "SQL",
            Lang::Lua => "Lua",
            Lang::Html => "HTML",
            Lang::Xml => "XML",
            Lang::Json => "JSON",
            Lang::Yaml => "YAML",
            Lang::Toml => "TOML",
            Lang::Ini => tr!("設定檔", "Settings file"),
            Lang::Markdown => "Markdown",
        }
    }
}

/// 依檔名決定預覽方式。
pub fn classify(name: &str) -> Kind {
    let lower = name.to_lowercase();
    let special = match lower.as_str() {
        "dockerfile" | "makefile" | "gnumakefile" | ".bashrc" | ".zshrc" | ".profile" | ".gitignore" | ".gitattributes" | ".dockerignore"
        | ".npmignore" | ".env" => Some(Lang::Shell),
        ".editorconfig" | ".gitconfig" | ".npmrc" | "desktop.ini" => Some(Lang::Ini),
        "cargo.lock" => Some(Lang::Toml),
        _ => None,
    };
    if let Some(l) = special {
        return Kind::Text(l);
    }
    let ext = paths::extension_lower(name);
    let ext = ext.as_str();
    if IMAGE_EXTS.contains(&ext) {
        return Kind::Image;
    }
    if VIDEO_EXTS.contains(&ext) {
        return Kind::Video;
    }
    if ext == "pdf" {
        return Kind::Pdf;
    }
    if let Some(f) = crate::office::Format::from_ext(ext) {
        return Kind::Office(f);
    }
    if crate::office::is_legacy_ext(ext) {
        return Kind::LegacyOffice;
    }
    let lang = match ext {
        "txt" | "text" | "log" | "csv" | "tsv" | "nfo" | "srt" | "vtt" | "lst" | "diff" | "patch" => Lang::Plain,
        "rs" => Lang::Rust,
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino" | "m" | "mm" => Lang::C,
        "cs" | "csx" => Lang::CSharp,
        "java" => Lang::Java,
        "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
        "ts" | "tsx" | "mts" | "cts" => Lang::TypeScript,
        "go" => Lang::Go,
        "swift" => Lang::Swift,
        "kt" | "kts" => Lang::Kotlin,
        "php" => Lang::Php,
        "css" | "scss" | "less" => Lang::Css,
        "py" | "pyw" | "pyi" => Lang::Python,
        "rb" | "rake" | "gemspec" => Lang::Ruby,
        "sh" | "bash" | "zsh" | "fish" => Lang::Shell,
        "ps1" | "psm1" | "psd1" => Lang::PowerShell,
        "bat" | "cmd" => Lang::Batch,
        "sql" => Lang::Sql,
        "lua" => Lang::Lua,
        "html" | "htm" | "xhtml" | "vue" | "svelte" => Lang::Html,
        "xml" | "xaml" | "svg" | "csproj" | "vbproj" | "fsproj" | "vcxproj" | "props" | "targets" | "resx" | "manifest" | "plist" | "config" | "nuspec" => Lang::Xml,
        "json" | "jsonc" | "json5" | "geojson" | "webmanifest" | "ipynb" => Lang::Json,
        "yml" | "yaml" => Lang::Yaml,
        "toml" => Lang::Toml,
        "ini" | "cfg" | "conf" | "inf" | "properties" | "reg" => Lang::Ini,
        "md" | "markdown" | "mdx" => Lang::Markdown,
        _ => return Kind::Other,
    };
    Kind::Text(lang)
}

// ---------------------------------------------------------------- 文字解碼

/// 解碼結果
#[derive(Debug, PartialEq, Eq)]
pub enum Decoded {
    Text(String),
    /// 看起來是二進位檔
    Binary,
    /// 不是 UTF-8／UTF-16，交給系統預設字碼頁（例如 Big5）解碼
    Legacy,
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| if big_endian { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/// 依 BOM 與內容判斷編碼。`bytes` 可能是讀取上限截斷後的前段，結尾不完整的 UTF-8 字元會被捨去。
pub fn decode(bytes: &[u8]) -> Decoded {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Decoded::Text(String::from_utf8_lossy(rest).into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Decoded::Text(utf16(rest, false));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Decoded::Text(utf16(rest, true));
    }
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        // 沒有 BOM 的 UTF-16：英數字元的另一半位元組都是 0
        let pairs = head.len() / 2;
        let odd = head.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
        let even = head.iter().step_by(2).filter(|b| **b == 0).count();
        if pairs >= 2 && odd * 10 >= pairs * 6 && even * 10 < pairs {
            return Decoded::Text(utf16(bytes, false));
        }
        if pairs >= 2 && even * 10 >= pairs * 6 && odd * 10 < pairs {
            return Decoded::Text(utf16(bytes, true));
        }
        return Decoded::Binary;
    }
    let control = head.iter().filter(|b| matches!(**b, 1..=8 | 0x0E..=0x1A | 0x1C..=0x1F)).count();
    if control * 20 > head.len().max(1) {
        return Decoded::Binary;
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Decoded::Text(s.to_string()),
        Err(e) if e.error_len().is_none() => Decoded::Text(String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()),
        Err(_) => Decoded::Legacy,
    }
}

// ---------------------------------------------------------------- 語法上色

/// 片段的種類（決定顏色）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Constant,
    Variable,
    Tag,
    Attr,
    Heading,
}

/// 一行中的上色片段（位元組位置）；沒有片段覆蓋的部分是一般文字。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub tok: Tok,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub lines: Vec<Line>,
    /// 超過 MAX_LINES，只保留前段
    pub truncated_lines: bool,
    /// 有過長的行被截斷
    pub long_lines: bool,
}

pub const MAX_LINES: usize = 5000;
pub const MAX_LINE_CHARS: usize = 1000;
const TAB: usize = 4;

/// 展開定位字元、把其他控制字元換成空白、截斷過長的行。
fn prepare(raw: &str) -> (String, bool) {
    let mut out = String::with_capacity(raw.len());
    let mut col = 0usize;
    for c in raw.chars() {
        if col >= MAX_LINE_CHARS {
            out.push('…');
            return (out, true);
        }
        match c {
            '\t' => {
                let n = TAB - col % TAB;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            c if c.is_control() => {
                out.push(' ');
                col += 1;
            }
            c => {
                out.push(c);
                col += 1;
            }
        }
    }
    (out, false)
}

/// 拆行並上色。
pub fn highlight(text: &str, lang: Lang) -> Doc {
    let mut doc = Doc::default();
    for (n, raw) in text.split('\n').enumerate() {
        if n >= MAX_LINES {
            doc.truncated_lines = true;
            break;
        }
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let (t, cut) = prepare(raw);
        doc.long_lines |= cut;
        doc.lines.push(Line { text: t, spans: Vec::new() });
    }
    if doc.lines.len() > 1 && !doc.truncated_lines && doc.lines.last().is_some_and(|l| l.text.is_empty()) {
        doc.lines.pop();
    }
    match lang {
        Lang::Plain => {}
        Lang::Html | Lang::Xml => {
            let mut st = Markup::Text;
            for l in &mut doc.lines {
                markup_line(&l.text, &mut st, &mut l.spans);
            }
        }
        Lang::Yaml => doc.lines.iter_mut().for_each(|l| yaml_line(&l.text, &mut l.spans)),
        Lang::Toml => doc.lines.iter_mut().for_each(|l| ini_line(&l.text, &['#'], &mut l.spans)),
        Lang::Ini => doc.lines.iter_mut().for_each(|l| ini_line(&l.text, &[';', '#'], &mut l.spans)),
        Lang::Batch => doc.lines.iter_mut().for_each(|l| batch_line(&l.text, &mut l.spans)),
        Lang::Markdown => {
            let mut fence = false;
            for l in &mut doc.lines {
                markdown_line(&l.text, &mut fence, &mut l.spans);
            }
        }
        _ => {
            let syn = syntax(lang);
            let mut st = Open::Normal;
            for l in &mut doc.lines {
                code_line(&l.text, &syn, &mut st, &mut l.spans);
            }
        }
    }
    doc
}

fn push(spans: &mut Vec<Span>, start: usize, end: usize, tok: Tok) {
    if end > start {
        spans.push(Span { start, end, tok });
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 程式語言的上色規則
struct Syntax {
    line: &'static [&'static str],
    block: &'static [(&'static str, &'static str)],
    long_strings: &'static [(&'static str, &'static str)],
    quotes: &'static [char],
    escape: Option<char>,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    constants: &'static [&'static str],
    nocase: bool,
    /// 變數前綴（$name、@name、#id）
    sigils: &'static [char],
    /// 大寫開頭的名稱當作型別
    caps_are_types: bool,
    /// 名稱可以含「-」（CSS、PowerShell 的 Get-ChildItem）
    dash_in_names: bool,
    /// 緊接「:」的名稱是屬性（CSS 的屬性名稱）
    attr_before_colon: bool,
    /// 字串後面接「:」是鍵（JSON）
    string_keys: bool,
    /// name! 是巨集、'a 是生命週期（Rust）
    rust: bool,
    /// # 開頭的前置處理指令（C／C++、C#）
    preprocessor: bool,
    /// # 註解前面必須是空白或行首（Shell）
    hash_needs_space: bool,
}

const fn base() -> Syntax {
    Syntax {
        line: &[],
        block: &[],
        long_strings: &[],
        quotes: &['"', '\''],
        escape: Some('\\'),
        keywords: &[],
        types: &[],
        constants: &[],
        nocase: false,
        sigils: &[],
        caps_are_types: false,
        dash_in_names: false,
        attr_before_colon: false,
        string_keys: false,
        rust: false,
        preprocessor: false,
        hash_needs_space: false,
    }
}

const C_COMMENTS: (&[&str], &[(&str, &str)]) = (&["//"], &[("/*", "*/")]);

fn syntax(lang: Lang) -> Syntax {
    let (cl, cb) = C_COMMENTS;
    match lang {
        Lang::Rust => Syntax {
            line: cl,
            block: cb,
            quotes: &['"'],
            keywords: &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop",
                "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
                "yield", "macro_rules",
            ],
            types: &[
                "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str", "Self",
            ],
            constants: &["true", "false", "None", "Some", "Ok", "Err"],
            caps_are_types: true,
            rust: true,
            ..base()
        },
        Lang::C => Syntax {
            line: cl,
            block: cb,
            keywords: &[
                "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto", "if", "inline", "register", "restrict",
                "return", "sizeof", "static", "struct", "switch", "typedef", "union", "volatile", "while", "class", "namespace", "template", "typename", "public",
                "private", "protected", "virtual", "override", "new", "delete", "this", "using", "try", "catch", "throw", "constexpr", "noexcept", "operator",
                "friend", "explicit", "mutable", "final", "co_await", "co_return", "decltype", "static_cast", "dynamic_cast", "reinterpret_cast", "const_cast",
            ],
            types: &[
                "void", "char", "short", "int", "long", "float", "double", "signed", "unsigned", "bool", "size_t", "wchar_t", "int8_t", "int16_t", "int32_t",
                "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "auto", "std", "string", "vector",
            ],
            constants: &["true", "false", "NULL", "nullptr", "TRUE", "FALSE"],
            preprocessor: true,
            ..base()
        },
        Lang::CSharp => Syntax {
            line: cl,
            block: cb,
            long_strings: &[("\"\"\"", "\"\"\"")],
            keywords: &[
                "abstract", "as", "async", "await", "base", "break", "case", "catch", "checked", "class", "const", "continue", "default", "delegate", "do", "else",
                "enum", "event", "explicit", "extern", "finally", "fixed", "for", "foreach", "goto", "if", "implicit", "in", "interface", "internal", "is", "lock",
                "namespace", "new", "operator", "out", "override", "params", "private", "protected", "public", "readonly", "ref", "return", "sealed", "sizeof",
                "stackalloc", "static", "struct", "switch", "this", "throw", "try", "typeof", "unchecked", "unsafe", "using", "var", "virtual", "volatile",
                "while", "get", "set", "init", "record", "yield", "where", "when", "with", "required", "partial",
            ],
            types: &[
                "bool", "byte", "char", "decimal", "double", "float", "int", "long", "object", "sbyte", "short", "string", "uint", "ulong", "ushort", "void",
                "nint", "nuint", "dynamic",
            ],
            constants: &["true", "false", "null"],
            caps_are_types: true,
            preprocessor: true,
            ..base()
        },
        Lang::Java | Lang::Kotlin => Syntax {
            line: cl,
            block: cb,
            long_strings: &[("\"\"\"", "\"\"\"")],
            keywords: &[
                "abstract", "assert", "break", "case", "catch", "class", "continue", "default", "do", "else", "enum", "extends", "final", "finally", "for", "if",
                "implements", "import", "instanceof", "interface", "native", "new", "package", "private", "protected", "public", "return", "static", "super",
                "switch", "synchronized", "this", "throw", "throws", "transient", "try", "volatile", "while", "var", "record", "sealed", "permits", "yield",
                "fun", "val", "when", "is", "in", "object", "companion", "data", "override", "open", "internal", "suspend", "lateinit", "by", "init", "typealias",
            ],
            types: &["boolean", "byte", "char", "double", "float", "int", "long", "short", "void", "Int", "Long", "Double", "Float", "Boolean", "String", "Unit", "Any"],
            constants: &["true", "false", "null"],
            caps_are_types: true,
            ..base()
        },
        Lang::JavaScript | Lang::TypeScript => Syntax {
            line: cl,
            block: cb,
            long_strings: &[("`", "`")],
            keywords: &[
                "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "finally", "for",
                "function", "if", "import", "in", "instanceof", "let", "new", "return", "super", "switch", "this", "throw", "try", "typeof", "var", "void",
                "while", "with", "yield", "async", "await", "of", "static", "get", "set", "from", "as", "interface", "type", "enum", "implements", "private",
                "public", "protected", "readonly", "declare", "namespace", "abstract", "keyof", "infer", "is", "satisfies",
            ],
            types: &["number", "string", "boolean", "any", "unknown", "never", "object", "symbol", "bigint"],
            constants: &["true", "false", "null", "undefined", "NaN", "Infinity"],
            sigils: &['@'],
            caps_are_types: lang == Lang::TypeScript,
            ..base()
        },
        Lang::Go => Syntax {
            line: cl,
            block: cb,
            long_strings: &[("`", "`")],
            keywords: &[
                "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface",
                "map", "package", "range", "return", "select", "struct", "switch", "type", "var",
            ],
            types: &[
                "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "uintptr", "float32", "float64", "complex64",
                "complex128", "string", "bool", "byte", "rune", "error", "any",
            ],
            constants: &["true", "false", "nil", "iota"],
            ..base()
        },
        Lang::Swift => Syntax {
            line: cl,
            block: cb,
            long_strings: &[("\"\"\"", "\"\"\"")],
            quotes: &['"'],
            keywords: &[
                "class", "deinit", "enum", "extension", "func", "import", "init", "let", "protocol", "struct", "subscript", "typealias", "var", "break", "case",
                "continue", "default", "defer", "do", "else", "fallthrough", "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as",
                "catch", "is", "rethrows", "throw", "throws", "try", "async", "await", "self", "static", "private", "public", "internal", "fileprivate", "open",
                "override", "mutating", "some", "any", "actor",
            ],
            constants: &["true", "false", "nil"],
            sigils: &['@'],
            caps_are_types: true,
            ..base()
        },
        Lang::Php => Syntax {
            line: &["//", "#"],
            block: cb,
            keywords: &[
                "abstract", "and", "array", "as", "break", "callable", "case", "catch", "class", "clone", "const", "continue", "declare", "default", "do", "echo",
                "else", "elseif", "empty", "enddeclare", "endfor", "endforeach", "endif", "endswitch", "endwhile", "extends", "final", "finally", "fn", "for",
                "foreach", "function", "global", "goto", "if", "implements", "include", "include_once", "instanceof", "interface", "isset", "list", "match",
                "namespace", "new", "or", "print", "private", "protected", "public", "readonly", "require", "require_once", "return", "static", "switch",
                "throw", "trait", "try", "unset", "use", "var", "while", "yield",
            ],
            constants: &["true", "false", "null"],
            nocase: true,
            sigils: &['$'],
            ..base()
        },
        Lang::Css => Syntax {
            block: cb,
            sigils: &['#', '.', '@'],
            constants: &["important", "inherit", "initial", "unset", "none", "auto"],
            dash_in_names: true,
            attr_before_colon: true,
            ..base()
        },
        Lang::Python => Syntax {
            line: &["#"],
            long_strings: &[("\"\"\"", "\"\"\""), ("'''", "'''")],
            keywords: &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
                "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match", "case", "self",
            ],
            types: &["int", "str", "float", "list", "dict", "set", "tuple", "bool", "bytes", "object", "type"],
            constants: &["True", "False", "None"],
            sigils: &['@'],
            caps_are_types: true,
            ..base()
        },
        Lang::Ruby => Syntax {
            line: &["#"],
            keywords: &[
                "alias", "and", "begin", "break", "case", "class", "def", "defined?", "do", "else", "elsif", "end", "ensure", "for", "if", "in", "module", "next",
                "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "undef", "unless", "until", "when", "while", "yield", "require",
                "attr_accessor", "attr_reader", "attr_writer", "puts", "lambda",
            ],
            constants: &["true", "false", "nil"],
            sigils: &['@', '$', ':'],
            caps_are_types: true,
            ..base()
        },
        Lang::Shell => Syntax {
            line: &["#"],
            quotes: &['"', '\'', '`'],
            keywords: &[
                "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done", "in", "function", "return", "exit", "export", "local",
                "readonly", "declare", "unset", "shift", "break", "continue", "echo", "source", "alias", "set", "cd", "test",
            ],
            constants: &["true", "false"],
            sigils: &['$'],
            hash_needs_space: true,
            ..base()
        },
        Lang::PowerShell => Syntax {
            line: &["#"],
            block: &[("<#", "#>")],
            escape: Some('`'),
            keywords: &[
                "begin", "break", "catch", "class", "continue", "data", "do", "dynamicparam", "else", "elseif", "end", "exit", "filter", "finally", "for",
                "foreach", "from", "function", "if", "in", "param", "process", "return", "switch", "throw", "trap", "try", "until", "using", "while", "-eq",
                "-ne", "-gt", "-ge", "-lt", "-le", "-like", "-notlike", "-match", "-notmatch", "-and", "-or", "-not", "-contains", "-in", "-band", "-bor",
            ],
            nocase: true,
            sigils: &['$'],
            dash_in_names: true,
            ..base()
        },
        Lang::Sql => Syntax {
            line: &["--"],
            block: cb,
            escape: None,
            keywords: &[
                "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "alter", "drop", "index", "view", "join",
                "inner", "left", "right", "outer", "full", "cross", "on", "as", "and", "or", "not", "is", "in", "exists", "between", "like", "order", "by",
                "group", "having", "limit", "offset", "union", "all", "distinct", "case", "when", "then", "else", "end", "primary", "key", "foreign",
                "references", "default", "constraint", "unique", "begin", "commit", "rollback", "transaction", "declare", "procedure", "function", "return",
                "returns", "with", "if", "top", "asc", "desc", "go", "exec", "use", "database",
            ],
            types: &[
                "int", "integer", "bigint", "smallint", "tinyint", "varchar", "nvarchar", "char", "nchar", "text", "date", "datetime", "datetime2", "timestamp",
                "boolean", "bit", "decimal", "numeric", "float", "real", "uniqueidentifier", "blob",
            ],
            constants: &["true", "false", "null"],
            nocase: true,
            sigils: &['@'],
            ..base()
        },
        Lang::Lua => Syntax {
            // 區塊註解要先於單行註解檢查（兩者都以 -- 開頭）
            line: &["--"],
            block: &[("--[[", "]]")],
            long_strings: &[("[[", "]]")],
            keywords: &[
                "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in", "local", "not", "or", "repeat", "return", "then", "until",
                "while", "self",
            ],
            constants: &["true", "false", "nil"],
            ..base()
        },
        Lang::Json => Syntax { line: cl, block: cb, quotes: &['"'], constants: &["true", "false", "null"], string_keys: true, ..base() },
        _ => base(),
    }
}

/// 跨行延續中的區塊：結束符號與種類
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Normal,
    Until(&'static str, Tok),
}

/// 從 `from` 開始找結束符號（字串會略過跳脫字元），回傳結束符號之後的位置。
fn find_close(line: &str, from: usize, close: &str, escape: Option<char>) -> Option<usize> {
    let mut it = line[from..].char_indices();
    while let Some((k, c)) = it.next() {
        let at = from + k;
        if escape == Some(c) {
            it.next();
            continue;
        }
        if line[at..].starts_with(close) {
            return Some(at + close.len());
        }
    }
    None
}

fn scan_ident(line: &str, start: usize, dash: bool) -> usize {
    line[start..]
        .char_indices()
        .find(|(k, c)| !(is_word_char(*c) || (dash && *c == '-' && *k > 0)))
        .map(|(k, _)| start + k)
        .unwrap_or(line.len())
}

fn scan_number(line: &str, start: usize) -> usize {
    let b = line.as_bytes();
    let mut i = start;
    while i < b.len() {
        let c = b[i];
        let decimal_point = c == b'.' && b.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_alphanumeric() || c == b'_' || decimal_point {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn word_tok(word: &str, syn: &Syntax, next: Option<char>) -> Option<Tok> {
    let has = |list: &[&str]| if syn.nocase { list.iter().any(|k| k.eq_ignore_ascii_case(word)) } else { list.contains(&word) };
    if has(syn.keywords) {
        return Some(Tok::Keyword);
    }
    if has(syn.constants) {
        return Some(Tok::Constant);
    }
    if has(syn.types) {
        return Some(Tok::Type);
    }
    if syn.attr_before_colon && next == Some(':') {
        return Some(Tok::Attr);
    }
    if next == Some('(') || (syn.rust && next == Some('!')) {
        return Some(Tok::Function);
    }
    let mut chars = word.chars();
    let first_upper = chars.next().is_some_and(|c| c.is_uppercase());
    if syn.caps_are_types && first_upper && word.chars().any(|c| c.is_lowercase()) {
        return Some(Tok::Type);
    }
    if syn.dash_in_names && first_upper && word.contains('-') {
        // PowerShell 指令（Get-ChildItem）
        return Some(Tok::Function);
    }
    None
}

fn code_line(line: &str, syn: &Syntax, state: &mut Open, spans: &mut Vec<Span>) {
    let mut i = 0;
    if let Open::Until(close, tok) = *state {
        let esc = if tok == Tok::String { syn.escape } else { None };
        match find_close(line, 0, close, esc) {
            Some(e) => {
                push(spans, 0, e, tok);
                i = e;
                *state = Open::Normal;
            }
            None => {
                push(spans, 0, line.len(), tok);
                return;
            }
        }
    }
    if syn.preprocessor && i == 0 && line.trim_start().starts_with('#') {
        let s = line.len() - line.trim_start().len();
        let e = scan_ident(line, s + 1, false);
        push(spans, s, e, Tok::Keyword);
        i = i.max(e);
    }
    let mut prev: Option<char> = line[..i].chars().next_back();
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap_or(' ');
        // 區塊註解（Lua 的 --[[ 要先於 --）
        if let Some((open, close)) = syn.block.iter().find(|(o, _)| rest.starts_with(o)) {
            match find_close(line, i + open.len(), close, None) {
                Some(e) => {
                    push(spans, i, e, Tok::Comment);
                    i = e;
                    prev = Some(' ');
                    continue;
                }
                None => {
                    push(spans, i, line.len(), Tok::Comment);
                    *state = Open::Until(close, Tok::Comment);
                    return;
                }
            }
        }
        if let Some(lc) = syn.line.iter().find(|l| rest.starts_with(*l)) {
            let ok = !(syn.hash_needs_space && *lc == "#") || prev.is_none_or(char::is_whitespace);
            if ok {
                push(spans, i, line.len(), Tok::Comment);
                return;
            }
        }
        if let Some((open, close)) = syn.long_strings.iter().find(|(o, _)| rest.starts_with(o)) {
            match find_close(line, i + open.len(), close, syn.escape) {
                Some(e) => {
                    push(spans, i, e, Tok::String);
                    i = e;
                    prev = Some('"');
                    continue;
                }
                None => {
                    push(spans, i, line.len(), Tok::String);
                    *state = Open::Until(close, Tok::String);
                    return;
                }
            }
        }
        if syn.rust && c == '\'' {
            // 字元常值 'x'、'\n'；否則是生命週期 'a
            let b = rest.as_bytes();
            let close = if b.get(1) == Some(&b'\\') { rest[2..].find('\'').map(|p| p + 3) } else { rest.char_indices().nth(2).filter(|(_, ch)| *ch == '\'').map(|(k, _)| k + 1) };
            match close {
                Some(e) => push(spans, i, i + e, Tok::String),
                None => {
                    let e = scan_ident(line, i + 1, false);
                    push(spans, i, e, Tok::Variable);
                    i = e.max(i + 1);
                    prev = Some('a');
                    continue;
                }
            }
            i += close.unwrap_or(1);
            prev = Some('\'');
            continue;
        }
        if syn.quotes.contains(&c) {
            let e = find_close(line, i + c.len_utf8(), &c.to_string(), syn.escape).unwrap_or(line.len());
            let key = syn.string_keys && line[e..].trim_start().starts_with(':');
            push(spans, i, e, if key { Tok::Attr } else { Tok::String });
            i = e;
            prev = Some(c);
            continue;
        }
        let after_word = prev.is_some_and(is_word_char);
        if c.is_ascii_digit() && !after_word {
            let e = scan_number(line, i);
            push(spans, i, e, Tok::Number);
            i = e;
            prev = Some('0');
            continue;
        }
        if syn.sigils.contains(&c) && !after_word {
            let next = rest[c.len_utf8()..].chars().next();
            if next.is_some_and(|n| is_word_char(n) || n == '{') {
                let from = i + c.len_utf8();
                let e = if next == Some('{') { line[from..].find('}').map(|p| from + p + 1).unwrap_or(line.len()) } else { scan_ident(line, from, syn.dash_in_names) };
                let tok = if c == '@' && syn.keywords.is_empty() { Tok::Keyword } else { Tok::Variable };
                push(spans, i, e, tok);
                i = e;
                prev = Some('a');
                continue;
            }
        }
        if (c.is_alphabetic() || c == '_' || (syn.dash_in_names && c == '-' && syn.nocase)) && !after_word {
            let e = scan_ident(line, i + c.len_utf8(), syn.dash_in_names);
            let word = &line[i..e];
            if let Some(t) = word_tok(word, syn, line[e..].chars().next()) {
                push(spans, i, e, t);
            }
            prev = word.chars().next_back();
            i = e;
            continue;
        }
        prev = Some(c);
        i += c.len_utf8();
    }
}

/// HTML／XML 的掃描狀態
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Markup {
    Text,
    Comment,
    /// 在標籤裡（屬性可能跨行）
    Tag,
}

fn markup_line(line: &str, st: &mut Markup, spans: &mut Vec<Span>) {
    let mut i = 0;
    while i < line.len() {
        let rest = &line[i..];
        match *st {
            Markup::Comment => match rest.find("-->") {
                Some(p) => {
                    push(spans, i, i + p + 3, Tok::Comment);
                    i += p + 3;
                    *st = Markup::Text;
                }
                None => {
                    push(spans, i, line.len(), Tok::Comment);
                    return;
                }
            },
            Markup::Tag => {
                let c = rest.chars().next().unwrap_or(' ');
                if rest.starts_with("/>") || rest.starts_with("?>") {
                    push(spans, i, i + 2, Tok::Tag);
                    i += 2;
                    *st = Markup::Text;
                } else if c == '>' {
                    push(spans, i, i + 1, Tok::Tag);
                    i += 1;
                    *st = Markup::Text;
                } else if c == '"' || c == '\'' {
                    let e = find_close(line, i + 1, &c.to_string(), None).unwrap_or(line.len());
                    push(spans, i, e, Tok::String);
                    i = e;
                } else if c.is_alphabetic() || c == '_' || c == ':' || c == '@' {
                    let e = line[i..].char_indices().find(|(_, ch)| !(is_word_char(*ch) || matches!(ch, '-' | ':' | '.' | '@'))).map(|(k, _)| i + k).unwrap_or(line.len());
                    push(spans, i, e.max(i + c.len_utf8()), Tok::Attr);
                    i = e.max(i + c.len_utf8());
                } else {
                    i += c.len_utf8();
                }
            }
            Markup::Text => {
                let Some(p) = rest.find(['<', '&']) else { return };
                i += p;
                let rest = &line[i..];
                if rest.starts_with("<!--") {
                    *st = Markup::Comment;
                    push(spans, i, i + 4, Tok::Comment);
                    i += 4;
                } else if let Some(after) = rest.strip_prefix('<') {
                    let name_start = i + 1 + after.chars().next().filter(|c| matches!(c, '/' | '!' | '?')).map(|c| c.len_utf8()).unwrap_or(0);
                    let e = line[name_start..]
                        .char_indices()
                        .find(|(_, ch)| !(is_word_char(*ch) || matches!(ch, '-' | ':' | '.')))
                        .map(|(k, _)| name_start + k)
                        .unwrap_or(line.len());
                    if e > name_start || after.starts_with('/') {
                        push(spans, i, e, Tok::Tag);
                        *st = Markup::Tag;
                        i = e;
                    } else {
                        i += 1;
                    }
                } else {
                    // 字元實體 &amp; &#123;
                    match rest.char_indices().take(12).find(|(_, c)| *c == ';' || c.is_whitespace()) {
                        Some((k, ';')) if k > 1 => {
                            push(spans, i, i + k + 1, Tok::Constant);
                            i += k + 1;
                        }
                        _ => i += 1,
                    }
                }
            }
        }
    }
}

/// 值的上色（YAML、TOML、INI 共用）：字串、數字、常數
fn value_spans(line: &str, from: usize, constants: &[&str], spans: &mut Vec<Span>) {
    let v = line[from..].trim_start();
    let s = line.len() - v.len();
    if v.is_empty() {
        return;
    }
    let first = v.chars().next().unwrap_or(' ');
    if first == '"' || first == '\'' {
        let e = find_close(line, s + 1, &first.to_string(), if first == '"' { Some('\\') } else { None }).unwrap_or(line.len());
        push(spans, s, e, Tok::String);
        return;
    }
    let word = v.trim_end();
    if constants.iter().any(|c| c.eq_ignore_ascii_case(word)) {
        push(spans, s, s + word.len(), Tok::Constant);
    } else if word.parse::<f64>().is_ok() || (first.is_ascii_digit() && word.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-+".contains(c))) {
        push(spans, s, s + word.len(), Tok::Number);
    }
}

/// 找出不在引號內的註解起點
fn comment_start(line: &str, marks: &[char], need_space: bool) -> Option<usize> {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (k, c) in line.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if marks.contains(&c) && (!need_space || prev.is_whitespace() || k == 0) => return Some(k),
            None => {}
        }
        prev = c;
    }
    None
}

fn ini_line(line: &str, comments: &[char], spans: &mut Vec<Span>) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with(comments) {
        push(spans, indent, line.len(), Tok::Comment);
        return;
    }
    let cut = comment_start(line, comments, true).unwrap_or(line.len());
    if trimmed.starts_with('[') {
        push(spans, indent, cut, Tok::Heading);
    } else if let Some(eq) = line[..cut].find(['=', ':']).filter(|p| *p > indent) {
        push(spans, indent, line[..eq].trim_end().len(), Tok::Attr);
        value_spans(&line[..cut], eq + 1, &["true", "false", "yes", "no", "on", "off"], spans);
    }
    push(spans, cut, line.len(), Tok::Comment);
}

fn yaml_line(line: &str, spans: &mut Vec<Span>) {
    let trimmed = line.trim_start();
    let mut i = line.len() - trimmed.len();
    if trimmed.starts_with('#') {
        push(spans, i, line.len(), Tok::Comment);
        return;
    }
    if trimmed == "---" || trimmed == "..." {
        push(spans, i, line.len(), Tok::Keyword);
        return;
    }
    let cut = comment_start(line, &['#'], true).unwrap_or(line.len());
    if line[i..].starts_with("- ") {
        push(spans, i, i + 1, Tok::Keyword);
        i += 2;
    }
    let body = &line[i..cut];
    let key_end = body.find(": ").or_else(|| body.strip_suffix(':').map(|b| b.len()));
    let value_from = match key_end {
        Some(k) if !body.starts_with(['"', '\'', '{', '[']) || body[..k].ends_with(['"', '\'']) => {
            push(spans, i, i + k, Tok::Attr);
            i + k + 1
        }
        _ => i,
    };
    let v = line[value_from..cut].trim_start();
    if v.starts_with(['&', '*']) {
        let s = cut - line[value_from..cut].trim_start().len();
        push(spans, s, s + v.split_whitespace().next().map(str::len).unwrap_or(0), Tok::Variable);
    } else {
        value_spans(&line[..cut], value_from, &["true", "false", "null", "yes", "no", "on", "off", "~"], spans);
    }
    push(spans, cut, line.len(), Tok::Comment);
}

fn batch_line(line: &str, spans: &mut Vec<Span>) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let lower = trimmed.to_ascii_lowercase();
    let body = lower.strip_prefix('@').unwrap_or(&lower);
    if body.starts_with("::") || body == "rem" || body.starts_with("rem ") {
        push(spans, indent, line.len(), Tok::Comment);
        return;
    }
    if trimmed.starts_with(':') {
        push(spans, indent, line.len(), Tok::Heading);
        return;
    }
    const KEYWORDS: &[&str] = &[
        "echo", "set", "if", "else", "for", "in", "do", "goto", "call", "exit", "not", "exist", "defined", "errorlevel", "setlocal", "endlocal", "shift",
        "pause", "cd", "pushd", "popd", "start", "title", "equ", "neq", "lss", "leq", "gtr", "geq", "off", "on",
    ];
    let b = line.as_bytes();
    let mut i = indent;
    while i < line.len() {
        let c = line[i..].chars().next().unwrap_or(' ');
        if c == '"' {
            let e = line[i + 1..].find('"').map(|p| i + p + 2).unwrap_or(line.len());
            push(spans, i, e, Tok::String);
            i = e;
        } else if c == '%' || c == '!' {
            // %var%、%%i、%1、!var!
            let rest = &line[i + 1..];
            let e = if rest.starts_with('%') {
                Some(i + 2 + scan_ident(rest, 1, false) - 1)
            } else if rest.starts_with(|ch: char| ch.is_ascii_digit() || ch == '~') && c == '%' {
                Some(i + 2)
            } else {
                rest.find(c).filter(|p| *p > 0 && !rest[..*p].contains(' ')).map(|p| i + p + 2)
            };
            match e {
                Some(e) => {
                    push(spans, i, e, Tok::Variable);
                    i = e;
                }
                None => i += 1,
            }
        } else if c.is_ascii_alphabetic() && (i == 0 || !is_word_char(b[i - 1] as char)) {
            let e = scan_ident(line, i, false);
            if KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(&line[i..e])) {
                push(spans, i, e, Tok::Keyword);
            }
            i = e;
        } else {
            i += c.len_utf8();
        }
    }
}

fn markdown_line(line: &str, fence: &mut bool, spans: &mut Vec<Span>) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        *fence = !*fence;
        push(spans, indent, line.len(), Tok::Comment);
        return;
    }
    if *fence {
        push(spans, 0, line.len(), Tok::String);
        return;
    }
    if trimmed.starts_with('#') && trimmed.trim_start_matches('#').starts_with(' ') {
        push(spans, indent, line.len(), Tok::Heading);
        return;
    }
    if trimmed.starts_with('>') {
        push(spans, indent, line.len(), Tok::Comment);
        return;
    }
    let mut i = indent;
    // 清單符號
    let marker = if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        Some(1)
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        (digits > 0 && trimmed[digits..].starts_with(". ")).then_some(digits + 1)
    };
    if let Some(m) = marker {
        push(spans, i, i + m, Tok::Keyword);
        i += m;
    }
    while i < line.len() {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap_or(' ');
        if c == '`' {
            let e = rest[1..].find('`').map(|p| i + p + 2).unwrap_or(line.len());
            push(spans, i, e, Tok::String);
            i = e;
        } else if c == '[' {
            // [文字](網址)
            match rest.find("](") {
                Some(p) => {
                    let close = rest[p + 2..].find(')').map(|q| i + p + 3 + q).unwrap_or(line.len());
                    push(spans, i, i + p + 1, Tok::Attr);
                    push(spans, i + p + 1, close, Tok::Variable);
                    i = close;
                }
                None => i += 1,
            }
        } else if rest.starts_with("**") || rest.starts_with("__") {
            let mark = &rest[..2];
            let e = rest[2..].find(mark).map(|p| i + p + 4).unwrap_or(i + 2);
            push(spans, i, e, Tok::Keyword);
            i = e;
        } else {
            i += c.len_utf8();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一行中上色的片段文字與種類
    fn toks(line: &str, lang: Lang) -> Vec<(String, Tok)> {
        let doc = highlight(line, lang);
        let l = &doc.lines[0];
        l.spans.iter().map(|s| (l.text[s.start..s.end].to_string(), s.tok)).collect()
    }

    fn has(v: &[(String, Tok)], text: &str, tok: Tok) -> bool {
        v.iter().any(|(t, k)| t == text && *k == tok)
    }

    #[test]
    fn classifies_by_name() {
        assert_eq!(classify("相片.JPG"), Kind::Image);
        assert_eq!(classify("clip.mp4"), Kind::Video);
        assert_eq!(classify("報告.pdf"), Kind::Pdf);
        assert_eq!(classify("簡報.PPTX"), Kind::Office(crate::office::Format::PowerPoint));
        assert_eq!(classify("帳.xlsm"), Kind::Office(crate::office::Format::Excel));
        assert_eq!(classify("a.odt"), Kind::Office(crate::office::Format::OdText));
        assert_eq!(classify("舊.doc"), Kind::LegacyOffice);
        assert_eq!(classify("main.rs"), Kind::Text(Lang::Rust));
        assert_eq!(classify("index.tsx"), Kind::Text(Lang::TypeScript));
        assert_eq!(classify("README.md"), Kind::Text(Lang::Markdown));
        assert_eq!(classify(".gitignore"), Kind::Text(Lang::Shell));
        assert_eq!(classify("Dockerfile"), Kind::Text(Lang::Shell));
        assert_eq!(classify("notes.txt"), Kind::Text(Lang::Plain));
        assert_eq!(classify("icon.svg"), Kind::Text(Lang::Xml));
        assert_eq!(classify("setup.exe"), Kind::Other);
        assert_eq!(classify("LICENSE"), Kind::Other);
        assert_eq!(Lang::Batch.label(), "批次檔");
    }

    #[test]
    fn decodes_encodings() {
        assert_eq!(decode(b"\xEF\xBB\xBFhi"), Decoded::Text("hi".into()));
        assert_eq!(decode("中文".as_bytes()), Decoded::Text("中文".into()));
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("ab".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
        assert_eq!(decode(&le), Decoded::Text("ab".into()));
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("ab".encode_utf16().flat_map(|u| u.to_be_bytes())).collect();
        assert_eq!(decode(&be), Decoded::Text("ab".into()));
        let no_bom: Vec<u8> = "hello world".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&no_bom), Decoded::Text("hello world".into()));
        assert_eq!(decode(&[0x4D, 0x5A, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0xFF, 0xFF]), Decoded::Binary);
        // Big5 的「中文」不是合法 UTF-8
        assert_eq!(decode(&[0xA4, 0xA4, 0xA4, 0xE5]), Decoded::Legacy);
        // 讀取上限切在多位元組字元中間：捨去不完整的尾端
        let mut cut = "測試".as_bytes().to_vec();
        cut.pop();
        assert_eq!(decode(&cut), Decoded::Text("測".into()));
    }

    #[test]
    fn prepares_lines() {
        let doc = highlight("a\tb\r\nline2\n", Lang::Plain);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0].text, "a   b");
        assert_eq!(doc.lines[1].text, "line2");
        let long = "x".repeat(MAX_LINE_CHARS + 50);
        let doc = highlight(&long, Lang::Plain);
        assert!(doc.long_lines);
        assert_eq!(doc.lines[0].text.chars().count(), MAX_LINE_CHARS + 1);
        let many = "a\n".repeat(MAX_LINES + 10);
        let doc = highlight(&many, Lang::Plain);
        assert!(doc.truncated_lines);
        assert_eq!(doc.lines.len(), MAX_LINES);
    }

    #[test]
    fn rust_tokens() {
        let v = toks(r#"pub fn main() { let s: String = "hi\"x"; println!("{}", 42); } // done"#, Lang::Rust);
        assert!(has(&v, "pub", Tok::Keyword));
        assert!(has(&v, "fn", Tok::Keyword));
        assert!(has(&v, "main", Tok::Function));
        assert!(has(&v, "String", Tok::Type));
        assert!(has(&v, r#""hi\"x""#, Tok::String));
        assert!(has(&v, "println", Tok::Function));
        assert!(has(&v, "42", Tok::Number));
        assert!(has(&v, "// done", Tok::Comment));
        let v = toks("fn f<'a>(c: char) -> bool { c == 'x' || c == '\\n' }", Lang::Rust);
        assert!(has(&v, "'a", Tok::Variable));
        assert!(has(&v, "'x'", Tok::String));
        assert!(has(&v, "'\\n'", Tok::String));
        // 名稱中的數字不是數字常值
        let v = toks("let x2 = u8::MAX;", Lang::Rust);
        assert!(!v.iter().any(|(t, k)| *k == Tok::Number && t == "2"));
    }

    #[test]
    fn block_comments_span_lines() {
        let doc = highlight("int a; /* start\nmiddle\nend */ int b;", Lang::C);
        assert_eq!(doc.lines[1].spans, vec![Span { start: 0, end: 6, tok: Tok::Comment }]);
        let l2 = &doc.lines[2];
        assert_eq!(&l2.text[l2.spans[0].start..l2.spans[0].end], "end */");
        assert!(l2.spans.iter().any(|s| &l2.text[s.start..s.end] == "int" && s.tok == Tok::Type));
        let v = toks("#include <stdio.h>", Lang::C);
        assert!(has(&v, "#include", Tok::Keyword));
    }

    #[test]
    fn python_and_shell() {
        let doc = highlight("def f():\n    \"\"\"doc\n    more\"\"\"\n    return None  # x", Lang::Python);
        assert!(doc.lines[0].spans.iter().any(|s| s.tok == Tok::Keyword));
        assert_eq!(doc.lines[1].spans[0].tok, Tok::String);
        assert_eq!(doc.lines[2].spans[0].tok, Tok::String);
        let last = &doc.lines[3];
        assert!(last.spans.iter().any(|s| &last.text[s.start..s.end] == "None" && s.tok == Tok::Constant));
        assert!(last.spans.iter().any(|s| &last.text[s.start..s.end] == "# x" && s.tok == Tok::Comment));
        let v = toks("echo $HOME ${PATH} $# # comment", Lang::Shell);
        assert!(has(&v, "$HOME", Tok::Variable));
        assert!(has(&v, "${PATH}", Tok::Variable));
        assert!(has(&v, "# comment", Tok::Comment));
    }

    #[test]
    fn powershell_sql_lua() {
        let v = toks("Get-ChildItem $env:TEMP -Recurse | Where-Object { $_.Length -gt 10 } # list", Lang::PowerShell);
        assert!(has(&v, "Get-ChildItem", Tok::Function));
        assert!(has(&v, "-gt", Tok::Keyword));
        assert!(has(&v, "# list", Tok::Comment));
        let v = toks("SELECT name FROM users WHERE id = 3 -- c", Lang::Sql);
        assert!(has(&v, "SELECT", Tok::Keyword));
        assert!(has(&v, "3", Tok::Number));
        assert!(has(&v, "-- c", Tok::Comment));
        let doc = highlight("--[[ block\nstill ]] local x = 1 -- end", Lang::Lua);
        assert_eq!(doc.lines[0].spans[0].tok, Tok::Comment);
        let l = &doc.lines[1];
        assert!(l.spans.iter().any(|s| &l.text[s.start..s.end] == "local" && s.tok == Tok::Keyword));
    }

    #[test]
    fn json_keys_and_values() {
        let v = toks(r#"{"name": "檔案", "count": 3, "ok": true}"#, Lang::Json);
        assert!(has(&v, "\"name\"", Tok::Attr));
        assert!(has(&v, "\"檔案\"", Tok::String));
        assert!(has(&v, "3", Tok::Number));
        assert!(has(&v, "true", Tok::Constant));
    }

    #[test]
    fn markup() {
        let doc = highlight("<div class=\"a\" id='b'>text &amp; more <!-- note\n--> <br/>", Lang::Html);
        let l = &doc.lines[0];
        let t: Vec<(String, Tok)> = l.spans.iter().map(|s| (l.text[s.start..s.end].to_string(), s.tok)).collect();
        assert!(has(&t, "<div", Tok::Tag));
        assert!(has(&t, "class", Tok::Attr));
        assert!(has(&t, "\"a\"", Tok::String));
        assert!(has(&t, "'b'", Tok::String));
        assert!(has(&t, ">", Tok::Tag));
        assert!(has(&t, "&amp;", Tok::Constant));
        assert!(t.iter().any(|(s, k)| s.starts_with("<!--") && *k == Tok::Comment));
        let l = &doc.lines[1];
        assert_eq!(l.spans[0].tok, Tok::Comment);
        assert!(l.spans.iter().any(|s| &l.text[s.start..s.end] == "<br" && s.tok == Tok::Tag));
        let v = toks("<?xml version=\"1.0\"?>", Lang::Xml);
        assert!(has(&v, "<?xml", Tok::Tag));
        assert!(has(&v, "?>", Tok::Tag));
    }

    #[test]
    fn config_formats() {
        let v = toks("[package] # c", Lang::Toml);
        assert!(has(&v, "[package] ", Tok::Heading));
        let v = toks("name = \"feather\" # x", Lang::Toml);
        assert!(has(&v, "name", Tok::Attr));
        assert!(has(&v, "\"feather\"", Tok::String));
        assert!(has(&v, "# x", Tok::Comment));
        let v = toks("; comment", Lang::Ini);
        assert!(has(&v, "; comment", Tok::Comment));
        let v = toks("  - name: build # step", Lang::Yaml);
        assert!(has(&v, "-", Tok::Keyword));
        assert!(has(&v, "name", Tok::Attr));
        assert!(has(&v, "# step", Tok::Comment));
        let v = toks("enabled: true", Lang::Yaml);
        assert!(has(&v, "true", Tok::Constant));
        let v = toks("port: 8080", Lang::Yaml);
        assert!(has(&v, "8080", Tok::Number));
    }

    #[test]
    fn batch_and_markdown() {
        let v = toks("@echo off", Lang::Batch);
        assert!(has(&v, "echo", Tok::Keyword));
        let v = toks("REM note", Lang::Batch);
        assert_eq!(v[0].1, Tok::Comment);
        let v = toks("set X=%PATH% & for %%i in (*) do echo !Y!", Lang::Batch);
        assert!(has(&v, "%PATH%", Tok::Variable));
        assert!(has(&v, "%%i", Tok::Variable));
        assert!(has(&v, "!Y!", Tok::Variable));
        let doc = highlight("# 標題\n- 項目 `code` [連結](https://x)\n```\nlet x\n```", Lang::Markdown);
        assert_eq!(doc.lines[0].spans[0].tok, Tok::Heading);
        let l = &doc.lines[1];
        let t: Vec<(String, Tok)> = l.spans.iter().map(|s| (l.text[s.start..s.end].to_string(), s.tok)).collect();
        assert!(has(&t, "-", Tok::Keyword));
        assert!(has(&t, "`code`", Tok::String));
        assert!(has(&t, "[連結]", Tok::Attr));
        assert!(has(&t, "(https://x)", Tok::Variable));
        assert_eq!(doc.lines[3].spans[0].tok, Tok::String);
    }

    #[test]
    fn spans_are_valid_byte_ranges() {
        // 各語言混合中文與特殊字元，片段必須落在字元邊界上且依序不重疊
        let sample = "中文 \"字串\" // 註解 /* x */ 'a' $v @d #h 12.5 <a b=\"c\"> -- z ::\n[x] = 1 # y\n";
        for lang in [
            Lang::Rust, Lang::C, Lang::CSharp, Lang::Java, Lang::JavaScript, Lang::TypeScript, Lang::Go, Lang::Swift, Lang::Kotlin, Lang::Php, Lang::Css,
            Lang::Python, Lang::Ruby, Lang::Shell, Lang::PowerShell, Lang::Batch, Lang::Sql, Lang::Lua, Lang::Html, Lang::Xml, Lang::Json, Lang::Yaml,
            Lang::Toml, Lang::Ini, Lang::Markdown,
        ] {
            let doc = highlight(sample, lang);
            for l in &doc.lines {
                let mut last = 0;
                for s in &l.spans {
                    assert!(s.start >= last && s.end > s.start && s.end <= l.text.len(), "{lang:?} {s:?} in {:?}", l.text);
                    assert!(l.text.is_char_boundary(s.start) && l.text.is_char_boundary(s.end), "{lang:?}");
                    last = s.end;
                }
            }
        }
    }
}

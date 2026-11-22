//! 自動更新的純邏輯：版本比較、更新資訊（update.json）的解析與檢查、何時再檢查、SHA-256。
//! 網路與執行安裝程式在 `src/win/update.rs`。
//!
//! 發佈方式（見 README「自動更新」）：每個版本在 GitHub Releases 建立 `v<版本>` 的發行版，
//! 上傳 StillfeatherSetup.exe 與 `build.ps1 -Setup` 產生的 update.json。程式讀取
//! `<repository>/releases/latest/download/update.json`（GitHub 會轉到最新正式版的那一份），
//! 有新版就提醒；下載的安裝程式要大小與 SHA-256 都和 update.json 相符才會執行。

use crate::json::{self, Value};

pub const MANIFEST_FILE: &str = "update.json";
pub const SETUP_FILE: &str = "StillfeatherSetup.exe";
/// 自動檢查的間隔：每天開啟時最多檢查一次
pub const CHECK_INTERVAL_SECS: u64 = 20 * 3600;
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_SETUP_BYTES: u64 = 256 * 1024 * 1024;
const MAX_NOTES_CHARS: usize = 1200;
const MAX_URL_LEN: usize = 2048;

// ---------------------------------------------------------------- 網址

/// GitHub 專案網址（Cargo.toml 的 `repository`）→ 最新正式版的 update.json 網址。不是 GitHub 專案網址時回傳 None。
pub fn manifest_url(repository: &str) -> Option<String> {
    let (owner, repo) = github_repo(repository)?;
    Some(format!("https://github.com/{owner}/{repo}/releases/latest/download/{MANIFEST_FILE}"))
}

/// 某個版本的安裝程式網址（發佈時寫進 update.json）
pub fn setup_url(repository: &str, version: &str) -> Option<String> {
    let (owner, repo) = github_repo(repository)?;
    Some(format!("https://github.com/{owner}/{repo}/releases/download/v{version}/{SETUP_FILE}"))
}

fn github_repo(repository: &str) -> Option<(&str, &str)> {
    let r = repository.trim().trim_end_matches('/');
    let r = r.strip_suffix(".git").unwrap_or(r);
    let rest = r.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    let valid = |s: &str| !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (parts.next().is_none() && valid(owner) && valid(repo)).then_some((owner, repo))
}

/// 拆開 https 網址：（主機, 連接埠, 路徑與查詢字串）。只接受 https，不接受帳號密碼。
pub fn split_https_url(url: &str) -> Option<(String, u16, String)> {
    if url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_control() || c == ' ') {
        return None;
    }
    let rest = url.strip_prefix("https://").or_else(|| url.get(..8).filter(|p| p.eq_ignore_ascii_case("https://")).map(|_| &url[8..]))?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.contains('@') {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p != 0)?),
        None => (authority, 443),
    };
    if host.is_empty() || host.len() > 253 || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.')) || host.starts_with(['.', '-']) {
        return None;
    }
    // 片段（# 之後）不送給伺服器
    let tail = tail.split('#').next().unwrap_or("");
    let path = if tail.starts_with('/') { tail.to_string() } else { format!("/{tail}") };
    Some((host.to_ascii_lowercase(), port, path))
}

// ---------------------------------------------------------------- 版本

/// 版本號：`1.2.3`、`v1.2`、`1.2.3-beta.2`（正式版比同號的預覽版新；`+` 之後的組建資訊不比較）
#[derive(Clone, Debug)]
pub struct Version {
    parts: Vec<u64>,
    pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let parts: Vec<u64> = core.split('.').map(|p| if !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None }).collect::<Option<_>>()?;
        if parts.is_empty() || parts.len() > 4 {
            return None;
        }
        let pre: Vec<String> = match pre {
            Some(p) => {
                let ids: Vec<&str> = p.split('.').collect();
                if ids.iter().any(|i| i.is_empty() || !i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
                    return None;
                }
                ids.into_iter().map(str::to_string).collect()
            }
            None => Vec::new(),
        };
        Some(Version { parts, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        let n = self.parts.len().max(other.parts.len());
        for i in 0..n {
            let (a, b) = (self.parts.get(i).copied().unwrap_or(0), other.parts.get(i).copied().unwrap_or(0));
            if a != b {
                return a.cmp(&b);
            }
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        for (a, b) in self.pre.iter().zip(&other.pre) {
            let ord = match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                _ => a.cmp(b),
            };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }
}

/// `1.2` 與 `1.2.0` 是同一個版本
impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `candidate` 是不是比 `current` 新（任何一個不是有效版本就當作不是）
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((Version::parse(candidate), Version::parse(current)), (Some(a), Some(b)) if a > b)
}

// ---------------------------------------------------------------- update.json

/// 一個版本的更新資訊
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    /// 安裝程式的下載網址（https）
    pub url: String,
    pub sha256: [u8; 32],
    pub size: u64,
    /// 這一版的更新說明（可以是空的）
    pub notes: String,
}

/// 解析並檢查 update.json：
/// `{"version":"0.2.0","url":"https://…/StillfeatherSetup.exe","sha256":"<64 個十六進位字元>","size":1773056,"notes":"…"}`
pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let bad = |what: &str| format!("更新資訊格式不正確：{what}。");
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(bad("檔案太大"));
    }
    let root = json::parse(text.trim_start_matches('\u{FEFF}')).map_err(|_| bad("不是有效的 JSON"))?;
    let string = |key: &str| match root.get(key) {
        Some(Value::Str(s)) => Ok(s.trim().to_string()),
        _ => Err(bad(&format!("缺少 {key}"))),
    };
    let version = string("version")?;
    if Version::parse(&version).is_none() {
        return Err(bad("version 不是有效的版本號"));
    }
    let url = string("url")?;
    if split_https_url(&url).is_none() {
        return Err(bad("url 必須是 https 網址"));
    }
    let sha256 = parse_hex32(&string("sha256")?).ok_or_else(|| bad("sha256 必須是 64 個十六進位字元"))?;
    let size = match root.get("size") {
        Some(Value::Num(n)) if n.fract() == 0.0 && *n >= 1.0 && *n <= MAX_SETUP_BYTES as f64 => *n as u64,
        _ => return Err(bad("size 必須是安裝程式的位元組數")),
    };
    let notes = match root.get("notes") {
        None | Some(Value::Null) => String::new(),
        Some(Value::Str(s)) => clean_notes(s),
        Some(_) => return Err(bad("notes 必須是文字")),
    };
    Ok(Manifest { version, url, sha256, size, notes })
}

/// 更新說明：統一換行、去掉控制字元，太長就截斷
fn clean_notes(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let mut out: String = s.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES_CHARS).collect();
    if s.chars().count() > MAX_NOTES_CHARS {
        out.push('…');
    }
    out.trim().to_string()
}

/// 要不要提醒這個版本
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    UpToDate,
    Newer,
    /// 使用者選過「略過這個版本」（手動檢查時不算）
    Skipped,
}

pub fn offer(current: &str, m: &Manifest, skipped: Option<&str>, manual: bool) -> Offer {
    if !is_newer(&m.version, current) {
        Offer::UpToDate
    } else if !manual && skipped.is_some_and(|s| Version::parse(s) == Version::parse(&m.version)) {
        Offer::Skipped
    } else {
        Offer::Newer
    }
}

/// 啟動新的安裝程式做更新的參數：裝回同一個位置，維持原本的捷徑與預設檔案總管設定，完成後重新開啟
pub fn setup_args(install_dir: &str) -> Vec<String> {
    vec!["--update".into(), "--dir".into(), install_dir.into()]
}

// ---------------------------------------------------------------- 狀態（update-state.json）

/// 上次檢查的時間與略過的版本（和設定檔放在同一個資料夾，Rust 版專用）
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Unix 時間（秒）
    pub last_check: Option<u64>,
    pub skipped: Option<String>,
}

impl State {
    /// 讀不懂的內容一律當作沒有記錄（最壞只是多檢查一次）
    pub fn from_json(text: &str) -> State {
        let Ok(root) = json::parse(text) else { return State::default() };
        let last_check = match root.get("LastCheck") {
            Some(Value::Num(n)) if n.fract() == 0.0 && *n >= 0.0 && *n < 1e12 => Some(*n as u64),
            _ => None,
        };
        let skipped = match root.get("SkippedVersion") {
            Some(Value::Str(s)) if Version::parse(s).is_some() => Some(s.trim().to_string()),
            _ => None,
        };
        State { last_check, skipped }
    }

    pub fn to_json(&self) -> String {
        let last = self.last_check.map_or("null".to_string(), |t| t.to_string());
        let skipped = self.skipped.as_deref().map_or("null".to_string(), json::quote);
        format!("{{\n  \"LastCheck\": {last},\n  \"SkippedVersion\": {skipped}\n}}")
    }

    /// 該自動檢查了嗎（從沒檢查過、超過間隔，或時鐘被調回去）
    pub fn due(&self, now: u64) -> bool {
        match self.last_check {
            None => true,
            Some(t) if t > now.saturating_add(3600) => true,
            Some(t) => now - t.min(now) >= CHECK_INTERVAL_SECS,
        }
    }
}

// ---------------------------------------------------------------- SHA-256

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// 可以分段餵資料的 SHA-256
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    used: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Sha256 { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], buf: [0; 64], used: 0, total: 0 }
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256::default()
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let n = (64 - self.used).min(data.len());
            self.buf[self.used..self.used + n].copy_from_slice(&data[..n]);
            self.used += n;
            data = &data[n..];
            if self.used < 64 {
                return;
            }
            let block = self.buf;
            self.block(&block);
            self.used = 0;
        }
        let mut chunks = data.chunks_exact(64);
        for c in &mut chunks {
            let mut block = [0u8; 64];
            block.copy_from_slice(c);
            self.block(&block);
        }
        let rest = chunks.remainder();
        self.buf[..rest.len()].copy_from_slice(rest);
        self.used = rest.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        let len_after = (self.used + 1) % 64;
        pad.resize(1 + if len_after <= 56 { 56 - len_after } else { 120 - len_after }, 0);
        pad.extend_from_slice(&bits.to_be_bytes());
        let total = self.total;
        self.update(&pad);
        self.total = total;
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn block(&mut self, b: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b2, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b2) ^ (a & c) ^ (b2 & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b2;
            b2 = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.h.iter_mut().zip([a, b2, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finish()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answers() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha256(&million)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
        // 分段餵入（跨區塊邊界的各種長度）結果相同
        for len in [0usize, 1, 55, 56, 63, 64, 65, 119, 120, 1000] {
            let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let whole = sha256(&data);
            for split in [0, 1, len / 3, len / 2, len.saturating_sub(1)] {
                let split = split.min(len);
                let mut s = Sha256::new();
                s.update(&data[..split]);
                s.update(&data[split..]);
                assert_eq!(s.finish(), whole, "len {len} split {split}");
            }
        }
    }

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("v1.0", "0.9.9"));
        assert!(is_newer("0.10.0", "0.9.0"), "數字比較，不是字串比較");
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(is_newer("1.0.0-beta.10", "1.0.0-beta.2"));
        assert!(is_newer("1.0.0-rc.1", "1.0.0-beta.9"));
        assert!(is_newer("1.0.0-beta", "1.0.0-alpha.5"));
        assert!(is_newer("1.0.0-alpha.1", "1.0.0-alpha"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(!is_newer("1.0.0+build5", "1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        for bad in ["", "abc", "1..2", "1.2.3.4.5", "1.-2", "1.2-", "1.2-beta..1", "１.２"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
            assert!(!is_newer(bad, "0.0.1"));
        }
    }

    fn manifest_json(version: &str, url: &str, sha: &str, size: &str, notes: &str) -> String {
        format!(r#"{{"version":"{version}","url":"{url}","sha256":"{sha}","size":{size}{notes}}}"#)
    }

    const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn manifest_parsing() {
        let ok = manifest_json("0.2.0", "https://github.com/a/b/releases/download/v0.2.0/StillfeatherSetup.exe", SHA, "1773056", r#","notes":"新增 Office 預覽\r\n修正 Win+E""#);
        let m = parse_manifest(&ok).unwrap();
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.size, 1773056);
        assert_eq!(hex(&m.sha256), SHA);
        assert_eq!(m.notes, "新增 Office 預覽\n修正 Win+E");
        assert_eq!(parse_manifest(&format!("\u{FEFF}{ok}")).unwrap(), m, "開頭的 BOM 沒關係");
        let bad = [
            manifest_json("x", "https://a/b", SHA, "1", ""),
            manifest_json("0.2.0", "http://github.com/a.exe", SHA, "1", ""),
            manifest_json("0.2.0", "https://user:pw@github.com/a.exe", SHA, "1", ""),
            manifest_json("0.2.0", "https://a/b", &SHA[..63], "1", ""),
            manifest_json("0.2.0", "https://a/b", &SHA.replace('e', "g"), "1", ""),
            manifest_json("0.2.0", "https://a/b", SHA, "0", ""),
            manifest_json("0.2.0", "https://a/b", SHA, "1.5", ""),
            manifest_json("0.2.0", "https://a/b", SHA, "999999999999", ""),
            manifest_json("0.2.0", "https://a/b", SHA, "1", r#","notes":5"#),
            r#"{"version":"0.2.0"}"#.to_string(),
            "not json".to_string(),
            "x".repeat(70_000),
        ];
        for b in &bad {
            assert!(parse_manifest(b).is_err(), "{}", &b[..b.len().min(80)]);
        }
        let long = manifest_json("0.2.0", "https://a/b", SHA, "1", &format!(r#","notes":"{}""#, "字".repeat(2000)));
        let notes = parse_manifest(&long).unwrap().notes;
        assert_eq!(notes.chars().count(), MAX_NOTES_CHARS + 1);
        assert!(notes.ends_with('…'));
    }

    #[test]
    fn reads_powershell_output() {
        // build.ps1 用 Windows PowerShell 5.1 的 ConvertTo-Json 寫出 update.json：縮排、冒號後兩個空白、& 與 ' 會跳脫
        let text = "{\r\n    \"version\":  \"0.2.0\",\r\n    \"url\":  \"https://github.com/pity0/stillfeather/releases/download/v0.2.0/StillfeatherSetup.exe\",\r\n    \"sha256\":  \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\",\r\n    \"size\":  1773056,\r\n    \"notes\":  \"新增 Office 預覽\\r\\nWin+E \\u0026 設定\\u0027s\"\r\n}";
        let m = parse_manifest(text).unwrap();
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.size, 1773056);
        assert_eq!(m.notes, "新增 Office 預覽\nWin+E & 設定's");
        assert_eq!(Some(m.url.as_str()), setup_url("https://github.com/pity0/stillfeather", "0.2.0").as_deref());
    }

    #[test]
    fn offers() {
        let m = parse_manifest(&manifest_json("0.2.0", "https://a/b", SHA, "1", "")).unwrap();
        assert_eq!(offer("0.1.0", &m, None, false), Offer::Newer);
        assert_eq!(offer("0.2.0", &m, None, false), Offer::UpToDate);
        assert_eq!(offer("0.3.0", &m, None, true), Offer::UpToDate);
        assert_eq!(offer("0.1.0", &m, Some("0.2.0"), false), Offer::Skipped);
        assert_eq!(offer("0.1.0", &m, Some("v0.2"), false), Offer::Skipped, "同一個版本的不同寫法");
        assert_eq!(offer("0.1.0", &m, Some("0.2.0"), true), Offer::Newer, "手動檢查不管略過");
        assert_eq!(offer("0.1.0", &m, Some("0.1.5"), false), Offer::Newer, "略過的是舊的版本");
    }

    #[test]
    fn urls() {
        assert_eq!(manifest_url("https://github.com/pity0/stillfeather").as_deref(), Some("https://github.com/pity0/stillfeather/releases/latest/download/update.json"));
        assert_eq!(manifest_url(" https://github.com/pity0/still-feather.git/ ").as_deref(), Some("https://github.com/pity0/still-feather/releases/latest/download/update.json"));
        for bad in ["", "https://gitlab.com/a/b", "http://github.com/a/b", "https://github.com/a", "https://github.com/a/b/c", "https://github.com/../b", "https://github.com/a b/c"] {
            assert_eq!(manifest_url(bad), None, "{bad}");
        }
        assert_eq!(setup_url("https://github.com/a/b", "0.2.0").as_deref(), Some("https://github.com/a/b/releases/download/v0.2.0/StillfeatherSetup.exe"));
        assert_eq!(split_https_url("https://github.com/a/b?x=1#frag"), Some(("github.com".into(), 443, "/a/b?x=1".into())));
        assert_eq!(split_https_url("HTTPS://Example.COM:8443"), Some(("example.com".into(), 8443, "/".into())));
        assert_eq!(split_https_url("https://example.com?q"), Some(("example.com".into(), 443, "/?q".into())));
        for bad in ["http://a/b", "https://", "https://a b/", "https://u@h/", "https://h:0/", "https://h:99999/", "https://-h/", "ftp://h/", "https://h/\n"] {
            assert_eq!(split_https_url(bad), None, "{bad:?}");
        }
        assert_eq!(setup_args("C:\\Program Files\\Stillfeather"), ["--update", "--dir", "C:\\Program Files\\Stillfeather"]);
    }

    #[test]
    fn state_and_schedule() {
        let s = State { last_check: Some(1_000_000), skipped: Some("0.2.0".into()) };
        assert_eq!(State::from_json(&s.to_json()), s);
        assert_eq!(State::from_json(&State::default().to_json()), State::default());
        assert_eq!(State::from_json("garbage"), State::default());
        assert_eq!(State::from_json(r#"{"LastCheck":-5,"SkippedVersion":"bad version"}"#), State::default());
        assert!(State::default().due(5));
        assert!(!s.due(1_000_000 + 3600));
        assert!(s.due(1_000_000 + CHECK_INTERVAL_SECS));
        assert!(s.due(10), "時鐘被調回去");
    }
}

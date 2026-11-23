//! 最小 Base64 編碼與解碼（PowerShell -EncodedCommand、更新資訊的簽章與公鑰）。

const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// 解碼標準 Base64（含 `=` 補齊；忽略空白與換行）。有不合法的字元或長度時回傳 None。
pub fn decode(input: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if clean.len() & 3 != 0 {
        return None;
    }
    let value = |c: u8| TABLE.iter().position(|t| *t == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    let chunks = clean.len() / 4;
    for (i, chunk) in clean.chunks(4).enumerate() {
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && i + 1 != chunks) {
            return None;
        }
        let mut n = 0u32;
        for (j, c) in chunk.iter().enumerate() {
            let v = if j >= 4 - pad { 0 } else { value(*c)? };
            n = (n << 6) | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// PowerShell 的 -EncodedCommand 需要 UTF-16LE 位元組再做 Base64。
pub fn encode_utf16le(text: &str) -> String {
    let bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    encode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn decoding() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar", &[0, 255, 128, 7, 1]] {
            assert_eq!(decode(&encode(s)).as_deref(), Some(s));
        }
        assert_eq!(decode(" Zm9v\r\nYmFy \n").as_deref(), Some(&b"foobar"[..]));
        assert_eq!(decode("Zm9"), None);
        assert_eq!(decode("Zm9v!mFy"), None);
        assert_eq!(decode("Zg==Zm9v"), None, "補齊只能在最後");
        assert_eq!(decode("Z==="), None);
        assert_eq!(decode("Zm=v"), None);
    }

    #[test]
    fn utf16le() {
        // "A" => 41 00 => "QQA="
        assert_eq!(encode_utf16le("A"), "QQA=");
        assert_eq!(encode_utf16le("中"), "LU4=");
    }
}

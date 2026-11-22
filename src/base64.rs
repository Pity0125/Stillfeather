//! 最小 Base64 編碼器（PowerShell -EncodedCommand 用）。

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
    fn utf16le() {
        // "A" => 41 00 => "QQA="
        assert_eq!(encode_utf16le("A"), "QQA=");
        assert_eq!(encode_utf16le("中"), "LU4=");
    }
}

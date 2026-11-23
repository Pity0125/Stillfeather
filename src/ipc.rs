//! 單一實例：第二次啟動把路徑交給既有主視窗的資料格式（WM_COPYDATA 內容）。
//!
//! 版面：u16 版本、u16 路徑數，接著每個路徑為 UTF-16 並以 0 結尾。

pub const VERSION: u16 = 1;
pub const MAX_PATHS: usize = 128;
pub const MAX_PATH_CHARS: usize = 32_767;
pub const MAX_BYTES: usize = 256 * 1024;
/// WM_COPYDATA 的 dwData 識別值（"FFR1"）。
pub const COPYDATA_TAG: usize = 0x4646_5231;

pub fn validate(paths: &[String]) -> Result<(), String> {
    if paths.len() > MAX_PATHS {
        return Err(trf!("一次最多只能傳送 {MAX_PATHS} 個路徑。", "Up to {MAX_PATHS} paths can be sent at once."));
    }
    for p in paths {
        if p.trim().is_empty() || p.contains('\0') || p.encode_utf16().count() > MAX_PATH_CHARS {
            return Err(tr!("路徑為空、過長或含有 NUL 字元。", "A path is empty, too long or contains a NUL character.").to_string());
        }
    }
    Ok(())
}

pub fn encode(paths: &[String]) -> Result<Vec<u16>, String> {
    validate(paths)?;
    let mut out = vec![VERSION, paths.len() as u16];
    for p in paths {
        out.extend(p.encode_utf16());
        out.push(0);
    }
    if out.len() * 2 > MAX_BYTES {
        return Err(tr!("開啟要求超過 256 KiB。", "The open request is larger than 256 KiB.").to_string());
    }
    Ok(out)
}

pub fn decode(data: &[u16]) -> Result<Vec<String>, String> {
    if data.len() * 2 > MAX_BYTES || data.len() < 2 {
        return Err(tr!("開啟要求大小不正確。", "The open request has the wrong size.").to_string());
    }
    if data[0] != VERSION {
        return Err(tr!("開啟要求版本不相符。", "The open request version doesn't match.").to_string());
    }
    let count = data[1] as usize;
    let mut paths = Vec::with_capacity(count);
    let mut rest = &data[2..];
    for _ in 0..count {
        let end = rest.iter().position(|u| *u == 0).ok_or(tr!("開啟要求缺少結尾。", "The open request has no terminator."))?;
        paths.push(String::from_utf16(&rest[..end]).map_err(|_| tr!("開啟要求含無效字元。", "The open request contains invalid characters."))?);
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        return Err(tr!("開啟要求含多餘資料。", "The open request contains extra data.").to_string());
    }
    validate(&paths)?;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let paths = vec!["C:\\Users\\pity0\\桌面".to_string(), "D:\\😀\\a.txt".to_string()];
        let enc = encode(&paths).unwrap();
        assert_eq!(decode(&enc).unwrap(), paths);
        assert_eq!(decode(&encode(&[]).unwrap()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(encode(&["".to_string()]).is_err());
        assert!(encode(&["a\0b".to_string()]).is_err());
        let many: Vec<String> = (0..129).map(|i| format!("C:\\{i}")).collect();
        assert!(encode(&many).is_err());
        assert!(decode(&[2, 0]).is_err());
        assert!(decode(&[1, 1, 65]).is_err());
        assert!(decode(&[1, 0, 65]).is_err());
        assert!(decode(&[1]).is_err());
    }
}

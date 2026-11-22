//! 把 .ico 轉成 Windows 資源檔（.res），讓建置腳本不需要資源編譯器（rc.exe／windres）就能把圖示嵌進 exe。
//! 這個模組同時給 build.rs（以 `#[path]` 引入）與單元測試使用，只用標準函式庫。
//!
//! .res 格式：一連串資源，每個都是 32 位元組起的標頭（資料大小、標頭大小、類型、名稱、旗標、語言…）
//! 加上 4 位元組對齊的資料；檔案開頭固定是一個空資源。
//! 圖示會拆成每個尺寸一個 RT_ICON，再加一個 RT_GROUP_ICON 目錄（編號 1）指向它們。

pub const RT_ICON: u16 = 3;
pub const RT_GROUP_ICON: u16 = 14;
pub const RT_VERSION: u16 = 16;
pub const RT_MANIFEST: u16 = 24;
/// exe 圖示的資源編號（程式執行時以 LoadImage(…, 1, …) 載入）。
pub const ICON_ID: u16 = 1;

/// .ico 裡的一個尺寸。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcoImage<'a> {
    pub width: u8,
    pub height: u8,
    pub colors: u8,
    pub planes: u16,
    pub bit_count: u16,
    pub data: &'a [u8],
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

/// 解析 .ico；格式不對時回傳 None。
pub fn parse_ico(ico: &[u8]) -> Option<Vec<IcoImage<'_>>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = u16_at(ico, 4)? as usize;
    if count == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let e = 6 + 16 * i;
        let size = u32_at(ico, e + 8)? as usize;
        let offset = u32_at(ico, e + 12)? as usize;
        let data = ico.get(offset..offset.checked_add(size)?)?;
        out.push(IcoImage {
            width: *ico.get(e)?,
            height: *ico.get(e + 1)?,
            colors: *ico.get(e + 2)?,
            planes: u16_at(ico, e + 4)?,
            bit_count: u16_at(ico, e + 6)?,
            data,
        });
    }
    Some(out)
}

fn push_resource(res: &mut Vec<u8>, type_id: u16, name_id: u16, flags: u16, data: &[u8]) {
    res.extend_from_slice(&(data.len() as u32).to_le_bytes()); // DataSize
    res.extend_from_slice(&32u32.to_le_bytes()); // HeaderSize
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // TYPE：以編號表示
    res.extend_from_slice(&type_id.to_le_bytes());
    res.extend_from_slice(&0xFFFFu16.to_le_bytes()); // NAME：以編號表示
    res.extend_from_slice(&name_id.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    res.extend_from_slice(&flags.to_le_bytes()); // MemoryFlags
    res.extend_from_slice(&0u16.to_le_bytes()); // LanguageId（中性）
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    while res.len() % 4 != 0 {
        res.push(0);
    }
}

/// 新的 .res：只有開頭的空資源（類型與名稱皆為 0），再用 `add_*` 加入資源。
pub fn empty_res() -> Vec<u8> {
    let mut res = Vec::new();
    push_resource(&mut res, 0, 0, 0, &[]);
    res
}

/// 由 .ico 產生 .res 內容。
pub fn icon_res(ico: &[u8]) -> Option<Vec<u8>> {
    let mut res = empty_res();
    add_icon(&mut res, ico)?;
    Some(res)
}

/// 加入程式圖示（編號 [`ICON_ID`]）。.ico 格式不對時回傳 None，`res` 不變。
pub fn add_icon(res: &mut Vec<u8>, ico: &[u8]) -> Option<()> {
    let images = parse_ico(ico)?;
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, img) in images.iter().enumerate() {
        let id = (i + 1) as u16;
        push_resource(res, RT_ICON, id, 0x1010, img.data);
        // GRPICONDIRENTRY：14 位元組，最後以資源編號取代 .ico 裡的檔案位移
        group.extend_from_slice(&[img.width, img.height, img.colors, 0]);
        group.extend_from_slice(&img.planes.to_le_bytes());
        group.extend_from_slice(&img.bit_count.to_le_bytes());
        group.extend_from_slice(&(img.data.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(res, RT_GROUP_ICON, ICON_ID, 0x1030, &group);
    Some(())
}

/// 加入應用程式資訊清單（編號 1，Windows 啟動程式時自動套用）。
pub fn add_manifest(res: &mut Vec<u8>, xml: &str) {
    push_resource(res, RT_MANIFEST, 1, 0x1030, xml.as_bytes());
}

/// exe 的版本資訊（檔案內容 › 詳細資料、工作管理員的名稱欄都會顯示）
#[derive(Debug, Clone)]
pub struct VersionInfo<'a> {
    pub version: [u16; 4],
    pub description: &'a str,
    pub product: &'a str,
    pub company: &'a str,
    pub copyright: &'a str,
    pub original_filename: &'a str,
    pub internal_name: &'a str,
}

/// 「0.1.0」這種版本字串轉成四段數字（缺的補 0，無法解析的當 0）。
pub fn parse_version(s: &str) -> [u16; 4] {
    let mut v = [0u16; 4];
    for (i, part) in s.split(['.', '-', '+']).take(4).enumerate() {
        v[i] = part.parse().unwrap_or(0);
    }
    v
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect()
}

fn pad4(b: &mut Vec<u8>) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
}

/// VS_VERSIONINFO 的一個節點：長度、值的長度、類型、鍵（UTF-16）、對齊、值、對齊後的子節點。
fn version_node(key: &str, kind: u16, value: &[u8], value_len: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0u8, 0];
    b.extend_from_slice(&value_len.to_le_bytes());
    b.extend_from_slice(&kind.to_le_bytes());
    b.extend_from_slice(&utf16z(key));
    pad4(&mut b);
    b.extend_from_slice(value);
    for c in children {
        pad4(&mut b);
        b.extend_from_slice(c);
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b
}

/// VS_VERSIONINFO 的內容（繁體中文、Unicode）。
pub fn version_block(info: &VersionInfo) -> Vec<u8> {
    let v = info.version;
    let ms = ((v[0] as u32) << 16) | v[1] as u32;
    let ls = ((v[2] as u32) << 16) | v[3] as u32;
    let mut fixed = Vec::with_capacity(52);
    for x in [0xFEEF_04BDu32, 0x0001_0000, ms, ls, ms, ls, 0x3F, 0, 0x0004_0004, 1, 0, 0, 0] {
        fixed.extend_from_slice(&x.to_le_bytes());
    }
    let version_text = format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]);
    let strings: Vec<Vec<u8>> = [
        ("CompanyName", info.company),
        ("FileDescription", info.description),
        ("FileVersion", version_text.as_str()),
        ("InternalName", info.internal_name),
        ("LegalCopyright", info.copyright),
        ("OriginalFilename", info.original_filename),
        ("ProductName", info.product),
        ("ProductVersion", version_text.as_str()),
    ]
    .iter()
    .map(|(k, text)| version_node(k, 1, &utf16z(text), (text.encode_utf16().count() + 1) as u16, &[]))
    .collect();
    let table = version_node("040404B0", 1, &[], 0, &strings);
    let string_info = version_node("StringFileInfo", 1, &[], 0, &[table]);
    // 語言 0x0404（繁體中文）、字碼頁 1200（Unicode）
    let translation = version_node("Translation", 0, &[0x04, 0x04, 0xB0, 0x04], 4, &[]);
    let var_info = version_node("VarFileInfo", 1, &[], 0, &[translation]);
    version_node("VS_VERSION_INFO", 0, &fixed, 52, &[string_info, var_info])
}

/// 加入版本資訊（編號 1）。
pub fn add_version(res: &mut Vec<u8>, info: &VersionInfo) {
    push_resource(res, RT_VERSION, 1, 0x0030, &version_block(info));
}

/// 執行時從 .ico 資料挑一個尺寸：最小但不小於 `size` 的；都太小就用最大的。
pub fn pick_size(images: &[IcoImage], size: u32) -> Option<usize> {
    let px = |img: &IcoImage| if img.width == 0 { 256 } else { img.width as u32 };
    images
        .iter()
        .enumerate()
        .filter(|(_, img)| px(img) >= size)
        .min_by_key(|(_, img)| px(img))
        .or_else(|| images.iter().enumerate().max_by_key(|(_, img)| px(img)))
        .map(|(i, _)| i)
}

/// .ico 裡是否剛好有 `size` 這個邊長。
pub fn has_size(images: &[IcoImage], size: u32) -> bool {
    images.iter().any(|img| (if img.width == 0 { 256 } else { img.width as u32 }) == size)
}

/// 以面積平均把 sw×sh 的 0xAARRGGBB 像素（未預乘）縮成 dw×dh。
/// 顏色以 alpha 加權，半透明的邊緣不會被透明像素拉出黑邊；
/// 比系統縮放圖示的最近點取樣平滑，用在沒有剛好尺寸的程式圖示上。
pub fn downscale_argb(src: &[u32], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u32> {
    let mut out = vec![0u32; dw * dh];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh {
        return out;
    }
    let fx = sw as f64 / dw as f64;
    let fy = sh as f64 / dh as f64;
    for y in 0..dh {
        let y0 = y as f64 * fy;
        let y1 = y0 + fy;
        for x in 0..dw {
            let x0 = x as f64 * fx;
            let x1 = x0 + fx;
            let (mut a, mut r, mut g, mut b, mut area) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
            let mut sy = y0.floor() as usize;
            while (sy as f64) < y1 && sy < sh {
                let wy = (y1.min(sy as f64 + 1.0) - y0.max(sy as f64)).max(0.0);
                let mut sx = x0.floor() as usize;
                while (sx as f64) < x1 && sx < sw {
                    let w = (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)).max(0.0) * wy;
                    let p = src[sy * sw + sx];
                    let pa = (p >> 24) as f64 * w;
                    a += pa;
                    r += ((p >> 16) & 0xFF) as f64 * pa;
                    g += ((p >> 8) & 0xFF) as f64 * pa;
                    b += (p & 0xFF) as f64 * pa;
                    area += w;
                    sx += 1;
                }
                sy += 1;
            }
            if a <= 0.0 || area <= 0.0 {
                continue;
            }
            let ch = |v: f64| (v / a).round().clamp(0.0, 255.0) as u32;
            let alpha = (a / area).round().clamp(0.0, 255.0) as u32;
            out[y * dw + x] = (alpha << 24) | (ch(r) << 16) | (ch(g) << 8) | ch(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ico() -> Vec<u8> {
        let blobs: [&[u8]; 3] = [b"AAAA", b"BBBBBB", b"CCCCCCCC"];
        let sizes = [16u8, 32, 0];
        let mut out = vec![0, 0, 1, 0, 3, 0];
        let mut offset = 6 + 16 * 3;
        for (s, b) in sizes.iter().zip(blobs) {
            out.extend_from_slice(&[*s, *s, 0, 0]);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += b.len();
        }
        for b in blobs {
            out.extend_from_slice(b);
        }
        out
    }

    #[test]
    fn parses_ico_entries() {
        let ico = sample_ico();
        let images = parse_ico(&ico).unwrap();
        assert_eq!(images.len(), 3);
        assert_eq!(images[1].width, 32);
        assert_eq!(images[2].data, b"CCCCCCCC");
        assert!(parse_ico(&ico[..10]).is_none());
        assert!(parse_ico(b"not an icon").is_none());
    }

    #[test]
    fn builds_res_layout() {
        let res = icon_res(&sample_ico()).unwrap();
        assert_eq!(res.len() % 4, 0);
        // 開頭的空資源
        assert_eq!(u32_at(&res, 0), Some(0));
        assert_eq!(u32_at(&res, 4), Some(32));
        // 第一個 RT_ICON 緊接在後：大小 4、類型 3、編號 1
        assert_eq!(u32_at(&res, 32), Some(4));
        assert_eq!(u16_at(&res, 40), Some(0xFFFF));
        assert_eq!(u16_at(&res, 42), Some(RT_ICON));
        assert_eq!(u16_at(&res, 46), Some(1));
        assert_eq!(&res[64..68], b"AAAA");
        // 最後一個是群組目錄：6 + 3×14 位元組，編號 1
        let tail = res.len() - 48 - 32;
        assert_eq!(u32_at(&res, tail), Some(48));
        assert_eq!(u16_at(&res, tail + 10), Some(RT_GROUP_ICON));
        assert_eq!(u16_at(&res, tail + 14), Some(ICON_ID));
        let group = &res[tail + 32..tail + 32 + 48];
        assert_eq!(u16_at(group, 4), Some(3));
        // 第 3 筆（256 px，寬度記為 0）指向編號 3
        assert_eq!(group[6 + 28], 0);
        assert_eq!(u16_at(group, 6 + 28 + 12), Some(3));
    }

    /// 子節點：(鍵, 類型, 值)
    type Child = (String, u16, Vec<u8>);

    /// 讀回 VS_VERSIONINFO 的節點：(鍵, 類型, 值, 子節點, 長度)
    fn read_node(b: &[u8]) -> (String, u16, Vec<u8>, Vec<Child>, usize) {
        let len = u16::from_le_bytes([b[0], b[1]]) as usize;
        let value_len = u16::from_le_bytes([b[2], b[3]]) as usize;
        let kind = u16::from_le_bytes([b[4], b[5]]);
        let mut i = 6;
        let mut key = Vec::new();
        loop {
            let c = u16::from_le_bytes([b[i], b[i + 1]]);
            i += 2;
            if c == 0 {
                break;
            }
            key.push(c);
        }
        while i % 4 != 0 {
            i += 1;
        }
        let bytes = if kind == 1 { value_len * 2 } else { value_len };
        let value = b[i..i + bytes].to_vec();
        i += bytes;
        let mut children = Vec::new();
        while i < len {
            while i % 4 != 0 {
                i += 1;
            }
            if i >= len {
                break;
            }
            let (k, t, v, _, n) = read_node(&b[i..]);
            children.push((k, t, v));
            i += n;
        }
        assert_eq!(i.max(len), len, "節點長度與內容不符");
        (String::from_utf16(&key).unwrap(), kind, value, children, len)
    }

    #[test]
    fn version_info_layout() {
        assert_eq!(parse_version("0.1.0"), [0, 1, 0, 0]);
        assert_eq!(parse_version("1.2.3-beta"), [1, 2, 3, 0]);
        let info = VersionInfo {
            version: [1, 2, 3, 4],
            description: "Stillfeather 安裝程式",
            product: "Stillfeather",
            company: "Stillfeather",
            copyright: "© 2026",
            original_filename: "StillfeatherSetup.exe",
            internal_name: "StillfeatherSetup",
        };
        let b = version_block(&info);
        let (key, kind, fixed, children, len) = read_node(&b);
        assert_eq!((key.as_str(), kind, fixed.len(), len), ("VS_VERSION_INFO", 0, 52, b.len()));
        assert_eq!(u32::from_le_bytes([fixed[0], fixed[1], fixed[2], fixed[3]]), 0xFEEF_04BD);
        assert_eq!(u32::from_le_bytes([fixed[8], fixed[9], fixed[10], fixed[11]]), 0x0001_0002);
        assert_eq!(children.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["StringFileInfo", "VarFileInfo"]);
        // 字串表：讀出 FileDescription
        let sfi_start = b.windows(28).position(|w| w == &utf16z("StringFileInfo")[..28]).unwrap() - 6;
        let (_, _, _, tables, _) = read_node(&b[sfi_start..]);
        assert_eq!(tables[0].0, "040404B0");
        let table_start = b.windows(18).position(|w| w == &utf16z("040404B0")[..]).unwrap() - 6;
        let (_, _, _, strings, _) = read_node(&b[table_start..]);
        let desc = strings.iter().find(|s| s.0 == "FileDescription").unwrap();
        let text: Vec<u16> = desc.2.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&text[..text.len() - 1]).unwrap(), "Stillfeather 安裝程式");
        assert!(strings.iter().any(|s| s.0 == "FileVersion"));
    }

    #[test]
    fn res_with_manifest_and_version_is_aligned() {
        let mut res = empty_res();
        add_icon(&mut res, &sample_ico()).unwrap();
        add_version(&mut res, &VersionInfo { version: [0, 1, 0, 0], description: "d", product: "p", company: "c", copyright: "", original_filename: "o", internal_name: "i" });
        add_manifest(&mut res, "<assembly/>");
        // 逐一走過每個資源：標頭 32 位元組、資料 4 位元組對齊，最後剛好結束
        let mut i = 0;
        let mut types = Vec::new();
        while i < res.len() {
            let size = u32::from_le_bytes(res[i..i + 4].try_into().unwrap()) as usize;
            let header = u32::from_le_bytes(res[i + 4..i + 8].try_into().unwrap()) as usize;
            types.push(u16::from_le_bytes([res[i + 10], res[i + 11]]));
            i += header + size;
            while i % 4 != 0 {
                i += 1;
            }
        }
        assert_eq!(i, res.len());
        assert!(types.contains(&RT_VERSION) && types.contains(&RT_MANIFEST) && types.contains(&RT_GROUP_ICON));
    }

    #[test]
    fn picks_nearest_size() {
        let ico = sample_ico();
        let images = parse_ico(&ico).unwrap();
        assert_eq!(pick_size(&images, 16), Some(0));
        assert_eq!(pick_size(&images, 20), Some(1));
        assert_eq!(pick_size(&images, 48), Some(2));
        assert_eq!(pick_size(&images, 512), Some(2));
        assert!(has_size(&images, 32));
        assert!(has_size(&images, 256));
        assert!(!has_size(&images, 40));
    }

    #[test]
    fn downscale_keeps_flat_color() {
        let src = vec![0xFF_D2_76_56u32; 256 * 256];
        let out = downscale_argb(&src, 256, 256, 50, 50);
        assert_eq!(out.len(), 50 * 50);
        assert!(out.iter().all(|&p| p == 0xFF_D2_76_56));
    }

    #[test]
    fn downscale_averages_by_area() {
        // 2×2 → 1×1：兩白兩黑 → 中灰
        let src = [0xFF_FF_FF_FFu32, 0xFF_00_00_00, 0xFF_00_00_00, 0xFF_FF_FF_FF];
        let out = downscale_argb(&src, 2, 2, 1, 1);
        assert_eq!(out[0], 0xFF_80_80_80);
    }

    #[test]
    fn downscale_transparent_edges_do_not_darken() {
        // 一個不透明的紅點旁邊三個全透明（黑）像素：顏色維持紅色，只有 alpha 變成四分之一
        let src = [0xFF_FF_00_00u32, 0, 0, 0];
        let out = downscale_argb(&src, 2, 2, 1, 1);
        assert_eq!(out[0] & 0x00FF_FFFF, 0x00FF_0000);
        assert_eq!(out[0] >> 24, 64);
        // 完全透明的區域保持透明
        assert_eq!(downscale_argb(&[0, 0, 0, 0], 2, 2, 1, 1)[0], 0);
    }

    #[test]
    fn downscale_handles_fractional_ratio() {
        // 3 → 2：每個目標像素涵蓋 1.5 個來源像素
        let src = [0xFF_00_00_00u32, 0xFF_FF_FF_FF, 0xFF_00_00_00];
        let out = downscale_argb(&src, 3, 1, 2, 1);
        // 左：黑 1 + 白 0.5 → 255/3 ≈ 85；右對稱
        assert_eq!(out[0], 0xFF_55_55_55);
        assert_eq!(out[1], 0xFF_55_55_55);
    }
}

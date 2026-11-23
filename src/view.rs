//! 清單的檢視模式（與檔案總管「檢視」選單相同的八種），以及大圖示的縮放與置中。

/// 檔案清單的檢視模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    ExtraLarge,
    Large,
    Medium,
    Small,
    List,
    Details,
    Tiles,
    Content,
}

/// 選單上的順序（與檔案總管相同），也是 Ctrl+Shift+1～8 的順序。
pub const ALL: [ViewMode; 8] = [
    ViewMode::ExtraLarge,
    ViewMode::Large,
    ViewMode::Medium,
    ViewMode::Small,
    ViewMode::List,
    ViewMode::Details,
    ViewMode::Tiles,
    ViewMode::Content,
];

impl ViewMode {
    /// 設定檔中的名稱
    pub fn as_str(self) -> &'static str {
        match self {
            ViewMode::ExtraLarge => "ExtraLargeIcons",
            ViewMode::Large => "LargeIcons",
            ViewMode::Medium => "MediumIcons",
            ViewMode::Small => "SmallIcons",
            ViewMode::List => "List",
            ViewMode::Details => "Details",
            ViewMode::Tiles => "Tiles",
            ViewMode::Content => "Content",
        }
    }

    pub fn parse(s: &str) -> Option<ViewMode> {
        ALL.into_iter().find(|m| m.as_str().eq_ignore_ascii_case(s))
    }

    /// 選單上的名稱
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::ExtraLarge => tr!("超大圖示", "Extra large icons"),
            ViewMode::Large => tr!("大圖示", "Large icons"),
            ViewMode::Medium => tr!("中圖示", "Medium icons"),
            ViewMode::Small => tr!("小圖示", "Small icons"),
            ViewMode::List => tr!("清單", "List"),
            ViewMode::Details => tr!("詳細資料", "Details"),
            ViewMode::Tiles => tr!("並排", "Tiles"),
            ViewMode::Content => tr!("內容", "Content"),
        }
    }

    /// 大圖示的邊長（100% 縮放時的像素）。`None` 表示這個模式只用小圖示。
    pub fn big_icon_px(self) -> Option<i32> {
        match self {
            ViewMode::ExtraLarge => Some(256),
            ViewMode::Large => Some(96),
            ViewMode::Medium | ViewMode::Tiles => Some(48),
            ViewMode::Content => Some(32),
            ViewMode::Small | ViewMode::List | ViewMode::Details => None,
        }
    }

    /// 這個檢視會不會顯示縮圖（圖示類的檢視，和檔案總管相同）
    pub fn shows_thumbnails(self) -> bool {
        self.big_icon_px().is_some()
    }

    /// Ctrl+Shift+數字 的數字（1～8）
    pub fn digit(self) -> u8 {
        ALL.iter().position(|m| *m == self).unwrap_or(0) as u8 + 1
    }

    pub fn from_digit(d: u8) -> Option<ViewMode> {
        (1..=8).contains(&d).then(|| ALL[(d - 1) as usize])
    }
}

/// 清單刪除選取的項目之後要選取哪一列：選取範圍之後第一個沒被選取的列；
/// 選取已到清單結尾就從最後一個選取項目往前找第一個沒被選取的列。`selected` 是選取的列號（順序不拘）。
pub fn next_unselected(selected: &[usize], count: usize) -> Option<usize> {
    let mut sorted = selected.to_vec();
    sorted.sort_unstable();
    let last = *sorted.last()?;
    let taken = |i: usize| sorted.binary_search(&i).is_ok();
    if last + 1 < count {
        return Some(last + 1);
    }
    (0..last.min(count)).rev().find(|i| !taken(*i))
}

/// 依 DPI 換算大圖示邊長，上限是系統最大的 256 像素圖示。
pub fn scaled_icon_px(base: i32, dpi: u32) -> i32 {
    ((base as i64 * dpi as i64 + 48) / 96).clamp(16, 256) as i32
}

/// 要放大成 `target` 時，應該取哪一種系統圖示：16、32、48 或 256（取不小於目標的最小者）。
pub fn source_px(target: i32) -> i32 {
    [16, 32, 48, 256].into_iter().find(|s| *s >= target).unwrap_or(256)
}

/// 預乘 ARGB 像素（0xAARRGGBB）的正方形影像，以面積平均縮放到 `dst` 邊長。
/// 縮小時每個目標像素是它所覆蓋的來源範圍的加權平均；放大時等同最近鄰。
pub fn resample(src: &[u32], src_px: usize, dst_px: usize) -> Vec<u32> {
    if src_px == 0 || dst_px == 0 || src.len() < src_px * src_px {
        return vec![0; dst_px * dst_px];
    }
    if src_px == dst_px {
        return src[..src_px * src_px].to_vec();
    }
    let scale = src_px as f64 / dst_px as f64;
    // 每個目標座標涵蓋的來源格與權重（兩軸相同）
    let spans: Vec<Vec<(usize, f64)>> = (0..dst_px)
        .map(|d| {
            let a = d as f64 * scale;
            let b = ((d + 1) as f64 * scale).min(src_px as f64);
            let mut v = Vec::new();
            let mut i = a.floor() as usize;
            while (i as f64) < b && i < src_px {
                let w = (b.min(i as f64 + 1.0) - a.max(i as f64)).max(0.0);
                if w > 0.0 {
                    v.push((i, w));
                }
                i += 1;
            }
            if v.is_empty() {
                v.push(((a as usize).min(src_px - 1), 1.0));
            }
            v
        })
        .collect();
    let mut out = vec![0u32; dst_px * dst_px];
    for (dy, ys) in spans.iter().enumerate() {
        for (dx, xs) in spans.iter().enumerate() {
            let mut acc = [0f64; 4];
            let mut total = 0f64;
            for &(sy, wy) in ys {
                for &(sx, wx) in xs {
                    let w = wx * wy;
                    let p = src[sy * src_px + sx];
                    for (k, a) in acc.iter_mut().enumerate() {
                        *a += ((p >> (8 * k)) & 0xFF) as f64 * w;
                    }
                    total += w;
                }
            }
            let mut px = 0u32;
            for (k, a) in acc.iter().enumerate() {
                let v = (a / total).round().clamp(0.0, 255.0) as u32;
                px |= v << (8 * k);
            }
            out[dy * dst_px + dx] = px;
        }
    }
    out
}

/// 不透明像素的外框（含左上、不含右下）；全透明時回傳 `None`。
pub fn opaque_bounds(px: &[u32], size: usize) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for y in 0..size {
        for x in 0..size {
            if px.get(y * size + x).is_some_and(|p| p >> 24 > 8) {
                b = Some(match b {
                    None => (x, y, x + 1, y + 1),
                    Some((l, t, r, bt)) => (l.min(x), t.min(y), r.max(x + 1), bt.max(y + 1)),
                });
            }
        }
    }
    b
}

/// 系統的 256 像素圖示若其實只在角落畫了一個小圖（沒有大尺寸版本的舊圖示），就不該整張縮放。
pub fn is_small_in_big(px: &[u32], size: usize) -> bool {
    match opaque_bounds(px, size) {
        None => true,
        Some((l, t, r, b)) => r - l <= size / 4 + 2 && b - t <= size / 4 + 2,
    }
}

/// 把 `src` 置中放進 `dst` 邊長的透明畫布（比畫布大時先縮小）。
pub fn center_into(src: &[u32], src_px: usize, dst_px: usize) -> Vec<u32> {
    if src_px > dst_px {
        return resample(src, src_px, dst_px);
    }
    let mut out = vec![0u32; dst_px * dst_px];
    let off = (dst_px - src_px) / 2;
    for y in 0..src_px {
        for x in 0..src_px {
            if let Some(p) = src.get(y * src_px + x) {
                out[(y + off) * dst_px + x + off] = *p;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roundtrip_and_digits() {
        for (i, m) in ALL.into_iter().enumerate() {
            assert_eq!(ViewMode::parse(m.as_str()), Some(m));
            assert_eq!(m.digit() as usize, i + 1);
            assert_eq!(ViewMode::from_digit(m.digit()), Some(m));
        }
        assert_eq!(ViewMode::parse("details"), Some(ViewMode::Details));
    }

    #[test]
    fn row_after_deleting_selection() {
        // 刪掉中間一段：選下一個
        assert_eq!(next_unselected(&[2, 3, 4], 10), Some(5));
        // 跳過選取之間沒被選的列不算「下一個」：從選取範圍的最後一項往後找
        assert_eq!(next_unselected(&[1, 3], 10), Some(4));
        // 選到最後一列：往前找
        assert_eq!(next_unselected(&[8, 9], 10), Some(7));
        assert_eq!(next_unselected(&[0, 9], 10), Some(8));
        // 全部選取或清單空白：沒有可選的
        assert_eq!(next_unselected(&[0, 1, 2], 3), None);
        assert_eq!(next_unselected(&[], 5), None);
        // 順序不一定排好
        assert_eq!(next_unselected(&[4, 2, 3], 6), Some(5));
        assert_eq!(ViewMode::parse("Huge"), None);
        assert_eq!(ViewMode::from_digit(0), None);
        assert_eq!(ViewMode::from_digit(9), None);
        assert_eq!(ViewMode::Large.label(), "大圖示");
    }

    #[test]
    fn icon_sizes() {
        assert_eq!(ViewMode::Details.big_icon_px(), None);
        assert_eq!(ViewMode::ExtraLarge.big_icon_px(), Some(256));
        assert_eq!(scaled_icon_px(96, 96), 96);
        assert_eq!(scaled_icon_px(96, 144), 144);
        assert_eq!(scaled_icon_px(256, 144), 256);
        assert_eq!(source_px(32), 32);
        assert_eq!(source_px(40), 48);
        assert_eq!(source_px(96), 256);
        assert_eq!(source_px(300), 256);
    }

    #[test]
    fn resample_averages() {
        // 4×4：左半白（不透明）、右半透明 → 2×2：左白右透明
        let w = 0xFFFF_FFFFu32;
        let src: Vec<u32> = (0..16).map(|i| if i % 4 < 2 { w } else { 0 }).collect();
        assert_eq!(resample(&src, 4, 2), vec![w, 0, w, 0]);
        // 2×2 交錯 → 1×1 取平均（預乘值各半）
        let avg = resample(&[w, 0, 0, w], 2, 1);
        assert_eq!(avg, vec![0x8080_8080]);
        // 非整數倍：256 → 96，均勻影像不變
        let solid = vec![0xFF10_2030u32; 256 * 256];
        let out = resample(&solid, 256, 96);
        assert_eq!(out.len(), 96 * 96);
        assert!(out.iter().all(|p| *p == 0xFF10_2030));
        // 相同大小直接複製
        assert_eq!(resample(&[1, 2, 3, 4], 2, 2), vec![1, 2, 3, 4]);
    }

    #[test]
    fn small_icon_in_jumbo_canvas() {
        let mut big = vec![0u32; 256 * 256];
        for y in 0..48 {
            for x in 0..48 {
                big[y * 256 + x] = 0xFF00_0000;
            }
        }
        assert_eq!(opaque_bounds(&big, 256), Some((0, 0, 48, 48)));
        assert!(is_small_in_big(&big, 256));
        let full = vec![0xFF00_0000u32; 256 * 256];
        assert!(!is_small_in_big(&full, 256));
        assert!(is_small_in_big(&vec![0u32; 64], 8));
    }

    #[test]
    fn centering() {
        let out = center_into(&[7], 1, 3);
        assert_eq!(out, vec![0, 0, 0, 0, 7, 0, 0, 0, 0]);
        assert_eq!(center_into(&[9, 9, 9, 9], 2, 1).len(), 1);
    }
}

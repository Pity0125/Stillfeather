//! 清單的框選（按住滑鼠拖出方框選取項目）：不碰 Windows 的計算部分。
//!
//! 座標都是「內容座標」：以第一列的左上角為原點，捲動時不變，所以拖到一半捲動清單，起點仍然固定在原本的項目上。
//! 一列的「項目本身」橫向只到 `span`（圖示左邊一點到最後一欄的右緣），左右兩側的空白不算，同檔案總管。

/// 框選和原本的選取怎麼合併（依按下滑鼠時的按鍵，同檔案總管）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// 沒按按鍵：只選方框裡的
    Replace,
    /// Shift：方框裡的加進原本的選取
    Add,
    /// Ctrl：方框裡的反轉（原本選了的取消，沒選的選取）
    Toggle,
}

impl Combine {
    pub fn from_keys(ctrl: bool, shift: bool) -> Combine {
        if ctrl {
            Combine::Toggle
        } else if shift {
            Combine::Add
        } else {
            Combine::Replace
        }
    }

    /// 這一列最後該不該選取：`before` 按下滑鼠前是否選取，`inside` 是否在方框裡
    pub fn wanted(self, before: bool, inside: bool) -> bool {
        match self {
            Combine::Replace => inside,
            Combine::Add => before || inside,
            Combine::Toggle => before != inside,
        }
    }
}

/// 兩點圍成的方框（左、上、右、下；左 ≤ 右、上 ≤ 下）
pub fn span(a: (i32, i32), b: (i32, i32)) -> (i32, i32, i32, i32) {
    (a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1))
}

/// 詳細資料檢視中方框碰到的列（第一列、最後一列，含）。
/// `row_h` 列高，`span` 項目本身的橫向範圍（左含右不含），`count` 列數；
/// 方框只在項目左右兩側的空白、或在所有列的上下方時沒有。
pub fn rows_touched(rect: (i32, i32, i32, i32), row_h: i32, span: (i32, i32), count: usize) -> Option<(usize, usize)> {
    let (left, top, right, bottom) = rect;
    if row_h <= 0 || count == 0 || right < span.0 || left >= span.1 {
        return None;
    }
    let total = row_h as i64 * count as i64;
    let (top, bottom) = (top as i64, bottom as i64);
    if bottom < 0 || top >= total {
        return None;
    }
    let first = top.max(0) / row_h as i64;
    let last = bottom.min(total - 1) / row_h as i64;
    Some((first as usize, last as usize))
}

/// 碰到的列從 `old` 變成 `new` 時，要重新設定選取的列（在其中一邊、不在另一邊的），由小到大
pub fn changed_rows(old: Option<(usize, usize)>, new: Option<(usize, usize)>) -> Vec<usize> {
    let inside = |r: Option<(usize, usize)>, i: usize| r.is_some_and(|(a, b)| a <= i && i <= b);
    let lo = [old, new].iter().flatten().map(|r| r.0).min();
    let hi = [old, new].iter().flatten().map(|r| r.1).max();
    let (Some(lo), Some(hi)) = (lo, hi) else { return Vec::new() };
    (lo..=hi).filter(|&i| inside(old, i) != inside(new, i)).collect()
}

/// 拖到清單外面時每次自動捲動多少像素：離邊緣越遠捲得越快（以列為單位，最多一次 8 列）
pub fn scroll_step(outside: i32, row_h: i32) -> i32 {
    if outside <= 0 || row_h <= 0 {
        return 0;
    }
    (1 + outside / row_h).min(8) * row_h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_follows_explorer() {
        assert_eq!(Combine::from_keys(false, false), Combine::Replace);
        assert_eq!(Combine::from_keys(false, true), Combine::Add);
        assert_eq!(Combine::from_keys(true, false), Combine::Toggle);
        assert_eq!(Combine::from_keys(true, true), Combine::Toggle);
        assert!(!Combine::Replace.wanted(true, false));
        assert!(Combine::Replace.wanted(false, true));
        assert!(Combine::Add.wanted(true, false));
        assert!(Combine::Add.wanted(false, true));
        assert!(!Combine::Add.wanted(false, false));
        assert!(!Combine::Toggle.wanted(true, true));
        assert!(Combine::Toggle.wanted(false, true));
        assert!(Combine::Toggle.wanted(true, false));
    }

    #[test]
    fn span_orders_corners() {
        assert_eq!(span((10, 50), (5, 20)), (5, 20, 10, 50));
        assert_eq!(span((1, 2), (3, 4)), (1, 2, 3, 4));
    }

    #[test]
    fn rows_touched_by_rect() {
        // 列高 20、10 列，項目本身橫向 16～300（左邊 16 是圖示左側的空白）
        let span = (16, 300);
        assert_eq!(rows_touched((20, 0, 30, 0), 20, span, 10), Some((0, 0)));
        assert_eq!(rows_touched((20, 19, 30, 20), 20, span, 10), Some((0, 1)));
        assert_eq!(rows_touched((20, 25, 30, 79), 20, span, 10), Some((1, 3)));
        // 從上方拖進來、拖到最後一列下面
        assert_eq!(rows_touched((20, -40, 30, 5), 20, span, 10), Some((0, 0)));
        assert_eq!(rows_touched((20, 150, 30, 900), 20, span, 10), Some((7, 9)));
        // 全在列的上方或下方
        assert_eq!(rows_touched((20, -40, 30, -1), 20, span, 10), None);
        assert_eq!(rows_touched((20, 200, 30, 260), 20, span, 10), None);
        // 只在欄位右側或圖示左側的空白裡上下拖：沒碰到項目
        assert_eq!(rows_touched((300, 0, 400, 50), 20, span, 10), None);
        assert_eq!(rows_touched((2, 0, 15, 50), 20, span, 10), None);
        // 從空白處拖進項目
        assert_eq!(rows_touched((280, 0, 400, 50), 20, span, 10), Some((0, 2)));
        assert_eq!(rows_touched((2, 0, 16, 50), 20, span, 10), Some((0, 2)));
        // 沒有列
        assert_eq!(rows_touched((20, 0, 30, 10), 20, span, 0), None);
        assert_eq!(rows_touched((20, 0, 30, 10), 0, span, 5), None);
    }

    #[test]
    fn changed_rows_between_ranges() {
        assert_eq!(changed_rows(None, None), Vec::<usize>::new());
        assert_eq!(changed_rows(None, Some((2, 4))), vec![2, 3, 4]);
        assert_eq!(changed_rows(Some((2, 4)), None), vec![2, 3, 4]);
        assert_eq!(changed_rows(Some((2, 4)), Some((2, 6))), vec![5, 6]);
        assert_eq!(changed_rows(Some((2, 6)), Some((4, 6))), vec![2, 3]);
        assert_eq!(changed_rows(Some((0, 1)), Some((5, 6))), vec![0, 1, 5, 6]);
        assert_eq!(changed_rows(Some((3, 3)), Some((3, 3))), Vec::<usize>::new());
    }

    #[test]
    fn scroll_speeds_up_with_distance() {
        assert_eq!(scroll_step(0, 28), 0);
        assert_eq!(scroll_step(-5, 28), 0);
        assert_eq!(scroll_step(1, 28), 28);
        assert_eq!(scroll_step(30, 28), 56);
        assert_eq!(scroll_step(10_000, 28), 8 * 28);
        assert_eq!(scroll_step(10, 0), 0);
    }
}

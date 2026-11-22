//! 數字、大小與日期的顯示格式（對齊 C# 版的 N0／N1 輸出）。

pub fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn one_decimal(value: f64) -> String {
    let tenths = (value * 10.0).round() as u64;
    format!("{}.{}", group_digits(tenths / 10), tenths % 10)
}

/// `—`、`39 B`、`2.3 KB`、`102.3 KB`、`1.5 MB`、`1,234.5 GB`。
pub fn format_size(size: Option<u64>) -> String {
    match size {
        None => "—".to_string(),
        Some(s) if s < 1024 => format!("{s} B"),
        Some(s) if s < 1_048_576 => format!("{} KB", one_decimal(s as f64 / 1024.0)),
        Some(s) if s < 1_073_741_824 => format!("{} MB", one_decimal(s as f64 / 1_048_576.0)),
        Some(s) => format!("{} GB", one_decimal(s as f64 / 1_073_741_824.0)),
    }
}

pub fn format_date(year: u16, month: u16, day: u16, hour: u16, minute: u16) -> String {
    format!("{year:04}/{month:02}/{day:02} {hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(50_000), "50,000");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(None), "—");
        assert_eq!(format_size(Some(39)), "39 B");
        assert_eq!(format_size(Some(946)), "946 B");
        assert_eq!(format_size(Some(2313)), "2.3 KB");
        assert_eq!(format_size(Some(104_763)), "102.3 KB");
        assert_eq!(format_size(Some(1_048_575)), "1,024.0 KB");
        assert_eq!(format_size(Some(5 * 1_048_576)), "5.0 MB");
        assert_eq!(format_size(Some(3 * 1_073_741_824)), "3.0 GB");
    }

    #[test]
    fn dates() {
        assert_eq!(format_date(2026, 10, 27, 18, 53), "2026/10/27 18:53");
        assert_eq!(format_date(2026, 1, 2, 3, 4), "2026/01/02 03:04");
    }
}

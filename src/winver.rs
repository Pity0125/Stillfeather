//! 依 Windows 版本（組建編號）決定要用哪些系統功能。不呼叫 Win32，取得組建編號的部分在 `win::util::windows_build`。

/// 第一個支援 `DWMWA_USE_IMMERSIVE_DARK_MODE` = 20 的組建（Windows 10 20H1 的測試版）
const DARK_MODE_ATTR_20: u32 = 18985;
/// Windows 10 1809：標題列深色模式第一次出現，當時屬性編號是 19（沒有公開文件）
const DARK_MODE_ATTR_19: u32 = 17763;
/// Windows 11：自訂標題列與外框顏色（`DWMWA_CAPTION_COLOR` 等）、改了屬性後標題列自己會重畫
pub const WINDOWS_11: u32 = 22000;

/// 標題列深色模式的 DWM 屬性編號：20（18985 以後）、19（1809～1909），更舊的 Windows 10 沒有（None）。
/// 用錯編號時 DwmSetWindowAttribute 只會失敗，標題列維持淺色。
pub fn dark_mode_attribute(build: u32) -> Option<u32> {
    if build >= DARK_MODE_ATTR_20 {
        Some(20)
    } else if build >= DARK_MODE_ATTR_19 {
        Some(19)
    } else {
        None
    }
}

/// Windows 10 改了深淺色後，標題列要等視窗重新啟用（或改變大小）才重畫：需要自己觸發重畫
pub fn caption_needs_nudge(build: u32) -> bool {
    build < WINDOWS_11
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_title_bar_attribute_by_build() {
        assert_eq!(dark_mode_attribute(26100), Some(20)); // Windows 11 24H2
        assert_eq!(dark_mode_attribute(22000), Some(20));
        assert_eq!(dark_mode_attribute(19045), Some(20)); // Windows 10 22H2
        assert_eq!(dark_mode_attribute(18985), Some(20));
        assert_eq!(dark_mode_attribute(18363), Some(19)); // Windows 10 1909
        assert_eq!(dark_mode_attribute(17763), Some(19)); // Windows 10 1809
        assert_eq!(dark_mode_attribute(17134), None); // Windows 10 1803
        assert!(caption_needs_nudge(19045));
        assert!(!caption_needs_nudge(22631));
    }
}

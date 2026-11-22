//! 「攔截 Win+E」的純邏輯（實際的鍵盤攔截在 `src/win/wine.rs`）。
//!
//! 新版 Windows 11（25H2／26H2，2026 年 10 月起陸續推出）按 Win+E 時直接開 Windows 檔案總管，
//! 不再看登錄裡「本機」的 opennewwindow 命令，所以「設為預設檔案總管」對 Win+E 沒有作用。
//! Stillfeather 改在背景執行一個沒有視窗的小程序（`Stillfeather.exe --win-e`），以低階鍵盤攔截
//! 看到 Win+E 就吃掉，改開 Stillfeather。這裡決定每個按鍵要放行、吃掉，還是吃掉並開啟 Stillfeather。

/// 背景攔截程序的參數
pub const WIN_E_ARG: &str = "--win-e";
/// 使用者的選擇（`install::APP_KEY` 底下的值）："1" 開、"0" 關；沒有這個值表示沒選過，預設開啟
pub const CHOICE_VALUE: &str = "WinEHook";
/// 登入時啟動：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 底下的值名稱
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const RUN_VALUE: &str = "Stillfeather";

/// E 鍵的虛擬鍵碼
pub const VK_E: u32 = 0x45;
/// 遮罩鍵：未指派的虛擬鍵碼。吃掉 E 之後送出一次，Windows 才不會在放開 Win 時以為只按了 Win 而跳出開始功能表
pub const VK_MASK: u16 = 0xE8;
/// 吃掉的 E 一直沒看到放開時，隔這麼久（毫秒）之後的按下當成新的一次（例如放開發生在鎖定畫面上，攔截看不到）
const STUCK_MS: u32 = 1500;

/// 要不要攔截：預設開啟，使用者關掉（"0"）才不要
pub fn wanted(choice: Option<&str>) -> bool {
    choice.is_none_or(|c| c.trim() != "0")
}

/// 登入時啟動的命令
pub fn run_command(exe: &str) -> String {
    format!("\"{exe}\" {WIN_E_ARG}")
}

/// 這個登入啟動命令是不是 Stillfeather 的背景攔截（任何一份）
pub fn is_run_command(cmd: &str) -> bool {
    let program = crate::install::command_program(cmd);
    let name = crate::paths::file_name(program);
    let ours = name.eq_ignore_ascii_case(crate::install::EXE_NAME) || name.eq_ignore_ascii_case("FeatherFiles.exe");
    ours && cmd.split_whitespace().any(|a| a == WIN_E_ARG)
}

/// 背景程序的訊息視窗類別名稱（安裝程式也用它找到並關閉背景程序）。`user` 是已經去掉特殊字元的使用者名稱。
pub fn window_class(user: &str) -> String {
    format!("Stillfeather.WinE.{user}")
}

/// 單一實例的互斥鎖名稱
pub fn mutex_name(user: &str) -> String {
    format!("Local\\Stillfeather.WinE.{user}")
}

/// 一次按鍵事件（E 鍵；其他鍵一律放行）
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyEvent {
    pub vk: u32,
    pub down: bool,
    /// 程式送出的按鍵（SendInput），不是使用者按的
    pub injected: bool,
    /// 事件時間（毫秒，會繞回）
    pub time: u32,
    /// 有按住 Win
    pub win: bool,
    /// 有按住 Ctrl、Alt 或 Shift（Win+Shift+E 之類的組合不攔截）
    pub mods: bool,
    /// 這個事件之前 E 已經是按下的（系統的按鍵狀態；放行的按鍵才會更新）
    pub was_down: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// 交給 Windows
    Pass,
    /// 吃掉（長按的重複與放開）
    Swallow,
    /// 吃掉，送出遮罩鍵並開啟 Stillfeather
    Open,
}

/// 追蹤 E 鍵：吃掉了按下，就連長按的重複與放開也吃掉，而且只開一次
#[derive(Debug, Default)]
pub struct WinE {
    swallowed: bool,
    last: u32,
}

impl WinE {
    pub fn key(&mut self, ev: KeyEvent) -> KeyAction {
        if ev.vk != VK_E || ev.injected {
            return KeyAction::Pass;
        }
        if !ev.down {
            let was = std::mem::take(&mut self.swallowed);
            return if was { KeyAction::Swallow } else { KeyAction::Pass };
        }
        if self.swallowed && ev.time.wrapping_sub(self.last) < STUCK_MS {
            // 長按：系統沒看到按下（被吃掉了），重複的按下也吃掉
            self.last = ev.time;
            return KeyAction::Swallow;
        }
        self.swallowed = false;
        if ev.was_down {
            // 放行的按下之後的長按重複（例如先按住 E 再按 Win）
            return KeyAction::Pass;
        }
        if ev.win && !ev.mods {
            self.swallowed = true;
            self.last = ev.time;
            return KeyAction::Open;
        }
        KeyAction::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(down: bool, time: u32, win: bool) -> KeyEvent {
        KeyEvent { vk: VK_E, down, time, win, ..Default::default() }
    }

    #[test]
    fn win_e_opens_once_and_swallows_repeats_and_release() {
        let mut k = WinE::default();
        assert_eq!(k.key(e(true, 1000, true)), KeyAction::Open);
        // 長按：重複的按下只吃掉，不會一直開
        assert_eq!(k.key(e(true, 1500, true)), KeyAction::Swallow);
        assert_eq!(k.key(e(true, 1533, true)), KeyAction::Swallow);
        // 先放開 Win 再放開 E：E 的重複與放開仍然吃掉
        assert_eq!(k.key(e(true, 1566, false)), KeyAction::Swallow);
        assert_eq!(k.key(e(false, 1600, false)), KeyAction::Swallow);
        // 再按一次 Win+E 又會開
        assert_eq!(k.key(e(true, 3000, true)), KeyAction::Open);
        assert_eq!(k.key(e(false, 3080, true)), KeyAction::Swallow);
    }

    #[test]
    fn other_keys_and_combinations_pass() {
        let mut k = WinE::default();
        // 只按 E
        assert_eq!(k.key(e(true, 0, false)), KeyAction::Pass);
        assert_eq!(k.key(e(false, 50, false)), KeyAction::Pass);
        // 其他鍵
        assert_eq!(k.key(KeyEvent { vk: 0x46, down: true, win: true, ..Default::default() }), KeyAction::Pass);
        // Win+Shift+E、Ctrl+Win+E 等組合不攔截
        assert_eq!(k.key(KeyEvent { mods: true, ..e(true, 100, true) }), KeyAction::Pass);
        assert_eq!(k.key(e(false, 150, true)), KeyAction::Pass);
        // 程式送出的按鍵（包括自己送的遮罩鍵）一律放行
        assert_eq!(k.key(KeyEvent { injected: true, ..e(true, 200, true) }), KeyAction::Pass);
    }

    #[test]
    fn holding_e_before_win_is_not_a_shortcut() {
        let mut k = WinE::default();
        assert_eq!(k.key(e(true, 0, false)), KeyAction::Pass);
        // 按住 E 之後才按 Win：E 的長按重複不算 Win+E
        assert_eq!(k.key(KeyEvent { was_down: true, ..e(true, 600, true) }), KeyAction::Pass);
        assert_eq!(k.key(e(false, 700, true)), KeyAction::Pass);
    }

    #[test]
    fn missed_release_recovers() {
        let mut k = WinE::default();
        assert_eq!(k.key(e(true, 1000, true)), KeyAction::Open);
        // 放開發生在鎖定畫面（攔截看不到）：很久之後只按 E，要放行
        assert_eq!(k.key(e(true, 60_000, false)), KeyAction::Pass);
        assert_eq!(k.key(e(false, 60_050, false)), KeyAction::Pass);
        // 時間繞回也不會卡住
        let mut k = WinE::default();
        assert_eq!(k.key(e(true, u32::MAX - 10, true)), KeyAction::Open);
        assert_eq!(k.key(e(true, 20, true)), KeyAction::Swallow);
    }

    #[test]
    fn choice_and_run_command() {
        assert!(wanted(None));
        assert!(wanted(Some("1")));
        assert!(!wanted(Some("0")));
        let exe = r"C:\Users\王小明\AppData\Local\Programs\Stillfeather\Stillfeather.exe";
        let cmd = run_command(exe);
        assert_eq!(cmd, format!("\"{exe}\" --win-e"));
        assert!(is_run_command(&cmd));
        assert!(is_run_command(r#""C:\Stillfeather\app\FeatherFiles.exe" --win-e"#));
        assert!(!is_run_command(&format!("\"{exe}\"")));
        assert!(!is_run_command(r#""C:\Other\Tool.exe" --win-e"#));
        assert_eq!(window_class("pity0"), "Stillfeather.WinE.pity0");
        assert_eq!(mutex_name("pity0"), r"Local\Stillfeather.WinE.pity0");
    }
}

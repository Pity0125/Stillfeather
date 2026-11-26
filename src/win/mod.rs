//! Windows 介面層（原生 Win32，無第三方套件）。

pub mod access;
pub mod anim;
pub mod app;
pub mod chips;
pub mod clipboard;
pub mod crashlog;
pub mod controls;
pub mod crumbmenu;
pub mod crumbs;
pub mod dialog;
pub mod dragdrop;
pub mod help;
pub mod fileicons;
pub mod fileops;
pub mod fs;
pub mod icons;
pub mod instance;
pub mod marquee;
pub mod menu;
pub mod menusite;
pub mod pane;
pub mod perf;
pub mod picker;
pub mod preview;
pub mod rescue;
pub mod settings_dialog;
pub mod shellmenu;
pub mod shellreg;
pub mod shellwin;
pub mod sys;
pub mod tabs;
pub mod theme;
pub mod tree;
pub mod undo;
pub mod update;
pub mod util;
pub mod viewmenu;
pub mod thumbs;
pub mod watch;
pub mod wine;

use sys::*;
use util::*;

const COMCTL6_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

/// 啟用 Common Controls v6（圓角按鈕、檔案總管樣式清單、輸入框提示文字）。
/// 不需要資源編譯器：執行期以啟用內容（activation context）載入 v6，失敗時退回 shell32 內建的資訊清單。
fn activate_visual_styles() {
    unsafe {
        let mut cookie: usize = 0;
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            let dir = feather_core::install::data_dir(&base);
            let file = feather_core::paths::join(&dir, "Stillfeather.comctl6.manifest");
            let current = std::fs::read_to_string(&file).ok();
            if current.as_deref() != Some(COMCTL6_MANIFEST) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&file, COMCTL6_MANIFEST);
            }
            let source = wide(&file);
            let ctx = ACTCTXW {
                cbSize: std::mem::size_of::<ACTCTXW>() as u32,
                dwFlags: 0,
                lpSource: source.as_ptr(),
                wProcessorArchitecture: 0,
                wLangId: 0,
                lpAssemblyDirectory: std::ptr::null(),
                lpResourceName: std::ptr::null(),
                lpApplicationName: std::ptr::null(),
                hModule: 0,
            };
            let h = CreateActCtxW(&ctx);
            if h != INVALID_HANDLE_VALUE && ActivateActCtx(h, &mut cookie) != 0 {
                return;
            }
        }
        let shell = wide("shell32.dll");
        let ctx = ACTCTXW {
            cbSize: std::mem::size_of::<ACTCTXW>() as u32,
            dwFlags: ACTCTX_FLAG_RESOURCE_NAME_VALID,
            lpSource: shell.as_ptr(),
            wProcessorArchitecture: 0,
            wLangId: 0,
            lpAssemblyDirectory: std::ptr::null(),
            lpResourceName: 124 as *const u16,
            lpApplicationName: std::ptr::null(),
            hModule: 0,
        };
        let h = CreateActCtxW(&ctx);
        if h != INVALID_HANDLE_VALUE {
            ActivateActCtx(h, &mut cookie);
        }
    }
}

/// 發生未預期錯誤時顯示訊息並寫入記錄，方便回報。
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let text = format!("{info}\n\n{}", std::backtrace::Backtrace::force_capture());
        if let Ok(base) = std::env::var("LOCALAPPDATA") {
            let dir = feather_core::install::data_dir(&base);
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(feather_core::paths::join(&dir, "rust-crash.log"), &text);
        }
        // 記錄資料夾也留一份（開發版在 exe 旁，方便從專案資料夾取得；安裝版在 %LOCALAPPDATA%\Stillfeather\logs）
        if let Some(dir) = crashlog::log_dir() {
            let _ = std::fs::write(feather_core::paths::join(&dir, "crash.log"), &text);
        }
        let first: String = text.lines().take(6).collect::<Vec<_>>().join("\n");
        message_box(
            0,
            &trf!("Stillfeather 發生未預期的錯誤，即將關閉。\n\n{first}\n\n詳細內容已寫入 %LOCALAPPDATA%\\Stillfeather\\rust-crash.log", "Stillfeather ran into an unexpected error and will close.\n\n{first}\n\nDetails were written to %LOCALAPPDATA%\\Stillfeather\\rust-crash.log"),
            MB_OK | MB_ICONERROR,
        );
    }));
}

/// 程式開始執行的時間（效能記錄用）
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// 程式開始執行到現在的毫秒數
pub fn since_start_ms() -> u64 {
    STARTED.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0)
}

/// 重新開啟時要等的程序：`--wait-pid=<pid>`
pub const WAIT_PID_ARG: &str = "--wait-pid=";

fn args_wait_pid() -> Option<u32> {
    std::env::args().skip(1).find_map(|a| a.strip_prefix(WAIT_PID_ARG).and_then(|p| p.parse().ok()))
}

pub fn entry() -> i32 {
    STARTED.get_or_init(std::time::Instant::now);
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    // 介面語言（設定的語言，或跟隨 Windows 的顯示語言）：之後的文字都照它。
    // 工作程序也要：它們的錯誤訊息會顯示在預覽裡
    app::apply_language();
    // 清單縮圖的工作程序（見 thumbs.rs）：沒有視窗，從標準輸入讀要求
    if args.first().map(String::as_str) == Some(feather_core::thumbs::WORKER_ARG) {
        return thumbs::worker_main();
    }
    if args.first().map(String::as_str) == Some("--preview-worker") {
        return match args.len() {
            2 => preview::worker_main(&args[1], None),
            3 => preview::worker_main(&args[1], Some(&args[2])),
            _ => 2,
        };
    }
    install_panic_hook();
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // 發佈前的檢查（tools\Sign-Update.ps1）：用這一版內建的公鑰驗證 update.json 的簽章，不開視窗
    if args.first().map(String::as_str) == Some("--verify-update") {
        return match args.get(1) {
            Some(manifest) => update::verify_release_files(manifest),
            None => 2,
        };
    }
    // 背景攔截 Win+E（新版 Windows 11 的 Win+E 不看登錄，見 wine.rs）：沒有視窗，不載入介面
    if args.first().map(String::as_str) == Some(feather_core::wine::WIN_E_ARG) {
        return wine::run();
    }
    // 舊版的設定在 %LOCALAPPDATA%\FeatherFiles：讀設定之前搬到 %LOCALAPPDATA%\Stillfeather
    rescue::migrate_data_dir();
    activate_visual_styles();
    if let Err(e) = unsafe { load_comctl() } {
        message_box(0, &trf!("無法初始化 Windows 通用控制項：{e}", "Couldn't initialize the Windows common controls: {e}"), MB_OK | MB_ICONERROR);
        return 1;
    }
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | ICC_BAR_CLASSES | ICC_TAB_CLASSES | ICC_STANDARD_CLASSES | ICC_DATE_CLASSES,
    };
    unsafe {
        (comctl().init_common_controls_ex)(&icc);
    }
    // 選擇器：其他程式要開啟檔案或選擇資料夾時（見 picker.rs）。另一個程序，不佔用單一實例
    if args.first().map(String::as_str) == Some(feather_core::picker::PICK_ARG) {
        return match args.get(1) {
            Some(request) => picker::run(request),
            None => 2,
        };
    }
    // 效能量測：不開視窗、不佔用單一實例，Stillfeather 開著時也可以跑
    if args.first().map(String::as_str) == Some("--perf") {
        return perf::run(&args[1..]);
    }
    if args.first().is_some_and(|s| s == "--detached-tab") {
        crashlog::install();
        unsafe {
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            OleInitialize(std::ptr::null_mut());
        }
        return app::run_detached();
    }
    // Win+E（設為預設檔案總管時）帶 --open-home：在新分頁開首頁
    let open_home = args.iter().any(|a| a == feather_core::install::OPEN_HOME_ARG);
    let mut paths: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();
    if open_home && paths.is_empty() {
        paths.push(feather_core::install::HOME_MARKER.to_string());
    }
    // 換了語言後重新開啟（見 App::restart）：等原本的 Stillfeather 關閉再接手，不要把位置交給正在關閉的它
    if let Some(pid) = args_wait_pid() {
        unsafe {
            let h = OpenProcess(SYNCHRONIZE, 0, pid);
            if h != 0 {
                WaitForSingleObject(h, 15_000);
                CloseHandle(h);
            }
        }
    }
    // 前面一分鐘內連續三次沒能正常啟動：改回 Windows 檔案總管並用它開這次的位置（見 rescue.rs）
    if rescue::rescue_if_crash_loop(&paths) {
        return 0;
    }
    // 這次啟動的記號：正常結束（離開這個函式）或撐過 20 秒時拿掉；當掉就留著，下次啟動據此判斷
    let _startup = rescue::Marker::start();
    let instance = instance::Instance::acquire();
    let full: Vec<String> = paths.iter().map(|p| if p == feather_core::install::HOME_MARKER { p.clone() } else { full_path(p).unwrap_or_else(|_| p.clone()) }).collect();
    // 「在資料夾中顯示」（瀏覽器的下載等）：其他程式以單一資料夾啟動時，先登記這個資料夾等 Windows 交來要選取的檔案，
    // 收到就改成開啟那個檔案（開它所在的資料夾並選取）。要在開始處理訊息之前做（見 shellwin.rs 的 Handshake）。
    let mut handshake = shellwin::Handshake::wanted(&full).and_then(|folder| shellwin::Handshake::begin(&folder));
    let mut selected = Vec::new();
    if let Some(h) = handshake.as_mut() {
        selected = h.wait(std::time::Duration::from_millis(500));
        if !selected.is_empty() {
            handshake = None;
        }
    }
    if !instance.primary {
        let target = if selected.is_empty() { &full } else { &selected };
        if !instance.send_to_primary(target) {
            // 已開啟的 Stillfeather 沒有回應（例如卡在第三方的右鍵選單擴充）：不要讓使用者什麼都打不開，
            // 這次的位置改用 Windows 檔案總管開。救援只算當掉的次數，卡住不會觸發，所以在這裡處理。
            rescue::open_in_explorer(target);
            return 0;
        }
        // 還沒收到選取：資料夾已經交出去了，再等一下（最多 10 秒），晚到的選取也交給原本的 Stillfeather
        if let Some(mut h) = handshake.take() {
            let late = h.wait(std::time::Duration::from_secs(10));
            if !late.is_empty() {
                instance.send_to_primary(&late);
            }
        }
        return 0;
    }
    // 主程式：交接結束（之後由主視窗自己的登記接手）；收到選取時改成開啟那個檔案
    drop(handshake);
    let paths = if selected.is_empty() { paths } else { selected };
    crashlog::install();
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // 拖放（RegisterDragDrop、SHDoDragDrop）需要 OLE
        OleInitialize(std::ptr::null_mut());
    }
    let code = app::run(paths, None);
    crashlog::crumb("訊息迴圈結束，正常關閉");
    drop(instance);
    code
}

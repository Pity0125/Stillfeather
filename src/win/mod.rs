//! Windows 介面層（原生 Win32，無第三方套件）。

pub mod anim;
pub mod app;
pub mod chips;
pub mod clipboard;
pub mod crashlog;
pub mod controls;
pub mod crumbs;
pub mod dialog;
pub mod dragdrop;
pub mod fileops;
pub mod fs;
pub mod icons;
pub mod instance;
pub mod menu;
pub mod pane;
pub mod perf;
pub mod preview;
pub mod settings_dialog;
pub mod shellmenu;
pub mod shellreg;
pub mod sys;
pub mod tabs;
pub mod theme;
pub mod tree;
pub mod undo;
pub mod update;
pub mod util;
pub mod viewmenu;

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
            let dir = feather_core::paths::join(&base, "FeatherFiles");
            let file = feather_core::paths::join(&dir, "FeatherFiles.rust.manifest");
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
            let dir = feather_core::paths::join(&base, "FeatherFiles");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(feather_core::paths::join(&dir, "rust-crash.log"), &text);
        }
        // 記錄資料夾也留一份（開發版在 exe 旁，方便從專案資料夾取得；安裝版在 %LOCALAPPDATA%\FeatherFiles\logs）
        if let Some(dir) = crashlog::log_dir() {
            let _ = std::fs::write(feather_core::paths::join(&dir, "crash.log"), &text);
        }
        let first: String = text.lines().take(6).collect::<Vec<_>>().join("\n");
        message_box(
            0,
            &format!("Stillfeather 發生未預期的錯誤，即將關閉。\n\n{first}\n\n詳細內容已寫入 %LOCALAPPDATA%\\FeatherFiles\\rust-crash.log"),
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

pub fn entry() -> i32 {
    STARTED.get_or_init(std::time::Instant::now);
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
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
    activate_visual_styles();
    if let Err(e) = unsafe { load_comctl() } {
        message_box(0, &format!("無法初始化 Windows 通用控制項：{e}"), MB_OK | MB_ICONERROR);
        return 1;
    }
    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | ICC_BAR_CLASSES | ICC_TAB_CLASSES | ICC_STANDARD_CLASSES,
    };
    unsafe {
        (comctl().init_common_controls_ex)(&icc);
    }
    // 效能量測：不開視窗、不佔用單一實例，Stillfeather 開著時也可以跑
    if args.first().map(String::as_str) == Some("--perf") {
        return perf::run(&args[1..]);
    }
    // Win+E（設為預設檔案總管時）帶 --open-home：在新分頁開首頁
    let open_home = args.iter().any(|a| a == feather_core::install::OPEN_HOME_ARG);
    let mut paths: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();
    if open_home && paths.is_empty() {
        paths.push(feather_core::install::HOME_MARKER.to_string());
    }
    let instance = instance::Instance::acquire();
    if !instance.primary {
        let full: Vec<String> = paths.iter().map(|p| if p == feather_core::install::HOME_MARKER { p.clone() } else { full_path(p).unwrap_or_else(|_| p.clone()) }).collect();
        if !instance.send_to_primary(&full) {
            message_box(0, "無法連線至已開啟的 Stillfeather。請稍後重試。", MB_OK | MB_ICONINFORMATION);
        }
        return 0;
    }
    crashlog::install();
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        // 拖放（RegisterDragDrop、SHDoDragDrop）需要 OLE
        OleInitialize(std::ptr::null_mut());
    }
    let code = app::run(paths);
    crashlog::crumb("訊息迴圈結束，正常關閉");
    drop(instance);
    code
}

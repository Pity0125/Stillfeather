//! 建置腳本：把圖示（assets/FeatherFiles.ico）與版本資訊嵌入 exe，讓檔案總管、捷徑、工作列與工作管理員
//! 都顯示 Stillfeather 的圖示和名稱。安裝程式（StillfeatherSetup，`--features setup`）另外嵌入應用程式資訊清單
//! （以一般權限執行、支援 Windows 10／11、每個螢幕各自的 DPI、Common Controls v6）。
//!
//! 不需要資源編譯器：直接產生 .res 檔（見 src/winres.rs）。
//! - MSVC 工具鏈：連結器可以直接連結 .res。
//! - GNU 工具鏈：需要 windres 把 .res 轉成目的檔；找不到時略過，只有 exe 檔案本身沒有圖示，
//!   視窗、工作列與 Alt+Tab 仍會顯示（程式執行時會自己載入圖示）。
//!
//! 設定環境變數 FEATHER_NO_ICON_RES=1 可以完全略過（build.ps1 在連結資源失敗時會自動這樣重試）。

#[path = "src/winres.rs"]
#[allow(dead_code)]
mod winres;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/FeatherFiles.ico");
    println!("cargo:rerun-if-changed=src/winres.rs");
    println!("cargo:rerun-if-changed=src/setup/setup.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FEATHER_NO_ICON_RES");
    println!("cargo:rerun-if-env-changed=STILLFEATHER_PAYLOAD");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || env::var_os("FEATHER_NO_ICON_RES").is_some() {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let ico = fs::read("assets/FeatherFiles.ico").ok();
    if ico.is_none() {
        println!("cargo:warning=找不到 assets/FeatherFiles.ico，exe 不會有圖示");
    }
    let version = winres::parse_version(&env::var("CARGO_PKG_VERSION").unwrap_or_default());
    let copyright = "© 2026 Stillfeather";

    // 應用程式：圖示＋版本資訊
    let mut app = winres::empty_res();
    if let Some(ico) = &ico {
        if winres::add_icon(&mut app, ico).is_none() {
            println!("cargo:warning=assets/FeatherFiles.ico 格式無法辨識，exe 不會有圖示");
        }
    }
    winres::add_version(
        &mut app,
        &winres::VersionInfo {
            version,
            description: "Stillfeather",
            product: "Stillfeather",
            company: "Stillfeather",
            copyright,
            original_filename: "Stillfeather.exe",
            internal_name: "Stillfeather",
        },
    );
    if let Some(arg) = link_res(&out, "feather_icon", &app) {
        println!("cargo:rustc-link-arg-bin=FeatherFiles={arg}");
    }

    // 安裝程式：圖示＋版本資訊＋資訊清單（只在建置安裝程式時）
    if env::var_os("CARGO_FEATURE_SETUP").is_some() {
        let mut setup = winres::empty_res();
        if let Some(ico) = &ico {
            let _ = winres::add_icon(&mut setup, ico);
        }
        winres::add_version(
            &mut setup,
            &winres::VersionInfo {
                version,
                description: "Stillfeather 安裝程式",
                product: "Stillfeather",
                company: "Stillfeather",
                copyright,
                original_filename: "StillfeatherSetup.exe",
                internal_name: "StillfeatherSetup",
            },
        );
        winres::add_manifest(&mut setup, include_str!("src/setup/setup.manifest"));
        if let Some(arg) = link_res(&out, "feather_setup", &setup) {
            println!("cargo:rustc-link-arg-bin=StillfeatherSetup={arg}");
        }
    }
}

/// 寫出 .res 並回傳要交給連結器的檔案路徑（GNU 工具鏈先轉成目的檔）；無法連結時回傳 None。
fn link_res(out: &Path, name: &str, res: &[u8]) -> Option<String> {
    let res_path = out.join(format!("{name}.res"));
    fs::write(&res_path, res).ok()?;
    match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => Some(res_path.display().to_string()),
        Ok("gnu") => {
            let obj = out.join(format!("{name}.o"));
            let converted = ["windres", "x86_64-w64-mingw32-windres"].iter().any(|tool| {
                Command::new(tool)
                    .args(["-J", "res", "-O", "coff", "-i"])
                    .arg(&res_path)
                    .arg("-o")
                    .arg(&obj)
                    .status()
                    .is_ok_and(|s| s.success())
            });
            if converted {
                Some(obj.display().to_string())
            } else {
                println!("cargo:warning=找不到 windres：exe 檔案本身不會有圖示與版本資訊，視窗與工作列仍會顯示圖示");
                None
            }
        }
        _ => None,
    }
}

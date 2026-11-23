//! 建置腳本：
//! - 嵌入版本資訊（檔案描述、版本、著作權）。這個 DLL 會被載入到其他程式裡，在檔案內容、工作管理員的模組清單與
//!   防毒軟體看來，有名稱與版本的元件比沒有任何資訊的 DLL 好辨認。.res 由 `src/winres.rs` 直接產生，不需要資源編譯器；
//!   GNU 工具鏈找不到 windres 時略過。設定 FEATHER_NO_ICON_RES=1 可以完全略過（build.ps1 連結資源失敗時會這樣重試）。
//! - MSVC 連結器對 COM 的標準匯出（DllGetClassObject、DllCanUnloadNow）會提醒「應為 PRIVATE」（LNK4104）。
//!   那只影響匯入程式庫，對其他程式載入這個 DLL 沒有影響；關掉這個提醒，讓 build.log 保持乾淨。

#[path = "../src/winres.rs"]
#[allow(dead_code)]
mod winres;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../src/winres.rs");
    println!("cargo:rerun-if-env-changed=FEATHER_NO_ICON_RES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-cdylib-link-arg=/IGNORE:4104");
    }
    if env::var_os("FEATHER_NO_ICON_RES").is_some() {
        return;
    }
    let mut res = winres::empty_res();
    winres::add_version(
        &mut res,
        &winres::VersionInfo {
            version: winres::parse_version(&env::var("CARGO_PKG_VERSION").unwrap_or_default()),
            description: "Stillfeather 開啟檔案視窗元件",
            product: "Stillfeather",
            company: "Stillfeather",
            copyright: "© 2026 Stillfeather",
            original_filename: "StillfeatherPicker.dll",
            internal_name: "StillfeatherPicker",
        },
    );
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    if let Some(arg) = link_res(&out, "picker_version", &res) {
        println!("cargo:rustc-cdylib-link-arg={arg}");
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
                println!("cargo:warning=找不到 windres：StillfeatherPicker.dll 不會有版本資訊（不影響功能）");
                None
            }
        }
        _ => None,
    }
}

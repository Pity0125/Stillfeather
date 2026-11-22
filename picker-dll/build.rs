//! MSVC 連結器對 COM 的標準匯出（DllGetClassObject、DllCanUnloadNow）會提醒「應為 PRIVATE」（LNK4104）。
//! 那只影響匯入程式庫，對其他程式載入這個 DLL 沒有影響；關掉這個提醒，讓 build.log 保持乾淨。

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-cdylib-link-arg=/IGNORE:4104");
    }
}

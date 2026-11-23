//! Stillfeather（Rust／原生 Win32 版）程式入口。
#![windows_subsystem = "windows"]

// tr!／trf!（介面語言，見 src/i18n.rs）在每個模組都能直接用
#[macro_use]
extern crate feather_core;

mod win;

fn main() {
    std::process::exit(win::entry());
}

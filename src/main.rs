//! Stillfeather（Rust／原生 Win32 版）程式入口。
#![windows_subsystem = "windows"]

mod win;

fn main() {
    std::process::exit(win::entry());
}

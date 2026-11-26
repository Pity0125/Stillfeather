//! Stillfeather 的平台無關核心。
//!
//! 這裡只放純邏輯（設定、路徑、排序、分頁模型、檔案作業檢查等），
//! 不呼叫任何 Win32 API，因此可以在任何平台執行 `cargo test --lib`。
//! 視窗與檔案系統呼叫位於 `src/win/`，只在 Windows 上建置。

// 第一個宣告：tr!／trf! 在之後的每個模組都能直接用
#[macro_use]
pub mod i18n;
pub mod base64;
pub mod anim;
pub mod category;
pub mod crashguard;
pub mod entry;
pub mod fileicons;
pub mod keymap;
pub mod folderviews;
pub mod format;
pub mod inflate;
pub mod install;
pub mod ipc;
pub mod json;
pub mod lru;
pub mod marquee;
pub mod namematch;
pub mod office;
pub mod paths;
pub mod picker;
pub mod plan;
pub mod quicklook;
pub mod searchquery;
pub mod session;
pub mod settings;
pub mod sorting;
pub mod terminal;
pub mod themes;
pub mod thumbs;
pub mod typeahead;
pub mod undo;
pub mod update;
pub mod view;
pub mod watch;
pub mod wine;
pub mod winres;
pub mod winver;
pub mod wordmark;
pub mod workspace;
pub mod xml;
pub mod zip;

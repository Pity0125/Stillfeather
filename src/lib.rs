//! Stillfeather 的平台無關核心。
//!
//! 這裡只放純邏輯（設定、路徑、排序、分頁模型、檔案作業檢查等），
//! 不呼叫任何 Win32 API，因此可以在任何平台執行 `cargo test --lib`。
//! 視窗與檔案系統呼叫位於 `src/win/`，只在 Windows 上建置。

pub mod base64;
pub mod anim;
pub mod category;
pub mod entry;
pub mod folderviews;
pub mod format;
pub mod inflate;
pub mod install;
pub mod ipc;
pub mod json;
pub mod lru;
pub mod namematch;
pub mod office;
pub mod paths;
pub mod plan;
pub mod quicklook;
pub mod settings;
pub mod sorting;
pub mod typeahead;
pub mod undo;
pub mod update;
pub mod view;
pub mod watch;
pub mod winres;
pub mod wordmark;
pub mod workspace;
pub mod xml;
pub mod zip;

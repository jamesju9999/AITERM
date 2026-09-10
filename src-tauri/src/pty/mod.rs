//! GUI 專屬的 PTY 接線。核心實作住在 `aiterm-core`——見那邊的 `pty` 模組。
//!
//! 這裡只留下依賴 Tauri 的東西：`#[tauri::command]` 進入點，以及把 PTY 輸出
//! 接到 Tauri 事件的 `manager`。

pub mod commands;
pub mod manager;

pub use aiterm_core::pty::{ansi, cd_parser, error, events, session, shell};
pub use aiterm_core::pty::{PtyError, PtyResult};
pub use manager::PtyManager;

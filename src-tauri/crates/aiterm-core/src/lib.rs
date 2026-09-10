//! AITerm 的無 GUI 核心：PTY 生命週期與遠端終端機共享協定。
//!
//! **這個 crate 不依賴 Tauri，也不該依賴。** 它同時被 GUI（`app` crate）與
//! headless 的 `aiterm-host` CLI 使用，而後者要能在一台沒有任何 GUI 函式庫
//! 的伺服器上編譯並執行。任何 `tauri` 的 import 都會讓那件事失效。

pub mod appimage_env;
pub mod pty;

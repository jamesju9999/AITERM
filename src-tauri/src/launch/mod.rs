//! 讓 AITerm 被當成終端機使用：把「開這個資料夾／跑這個指令」的請求，
//! 從冷啟動 argv、第二次啟動、macOS Opened 三個入口收斂成同一個佇列。

pub mod parse;

pub use parse::{args_from_file_urls, parse_args, LaunchRequest};

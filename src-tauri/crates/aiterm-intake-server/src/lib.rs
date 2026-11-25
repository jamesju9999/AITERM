//! AITerm 問題收集 Server。資料與規則在 `aiterm-intake-core`，這裡是常駐的收訊、認證與 API。
pub mod ai;
pub mod audit;
pub mod channels;
pub mod cli;
pub mod db;
pub mod events;
pub mod imap;
pub mod mail;
pub mod members;
pub mod secrets;
pub mod pairing;
pub mod reply;
pub mod runner;
pub mod orchestrator;
pub mod scheduler;
pub mod state;
pub mod telegram;
pub mod tls;
pub mod routes;
pub mod routes_data;
pub mod routes_board;
pub mod server;
#[doc(hidden)]
pub mod testkit;

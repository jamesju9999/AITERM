//! 遠端終端機共享：讓同區網的同事用一組 6 位短碼連進本機的某個終端機分頁，
//! 看畫面並（經同意後）接手控制。
//!
//! 與 `crate::bridge`（Claude Code 的 Anthropic API 相容層）和
//! `crate::mcp_server`（AITerm 對外的 MCP 工具 server）並列但**刻意分開**：
//! 那兩個綁 127.0.0.1，這一個綁區網介面。把路由混進 mcp_server 等於順手把
//! `execute_query`（允許任意 SQL）暴露到辦公室網路。
//!
//! 設計文件：`docs/superpowers/specs/2026-08-26-remote-terminal-sharing-design.md`

pub mod protocol;
pub mod registry;
pub mod server;
pub mod tls;

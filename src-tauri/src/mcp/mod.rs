// src-tauri/src/mcp/mod.rs
pub mod protocol;
pub mod transport;
pub mod types;

pub use types::{McpToolInfo, McpToolResult, McpServerStatus, encode_tool_name, decode_tool_name};

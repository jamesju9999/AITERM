// src-tauri/src/mcp/types.rs

use serde::{Deserialize, Serialize};

/// A tool exposed by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub server_id: String,
    pub server_name: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Text content returned by the tool.
    pub content: String,
    /// True if the tool reported an error in its result.
    pub is_error: bool,
}

/// Connection status of a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpServerStatus {
    Connecting,
    Connected { tool_count: usize },
    Error { message: String },
    Disabled,
}

/// Encoded tool name: `{server_id_sanitized}__{tool_name}`.
/// `server_id` chars that are not `[a-zA-Z0-9]` are replaced with `_`.
pub fn encode_tool_name(server_id: &str, tool_name: &str) -> String {
    let safe: String = server_id
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("{safe}__{tool_name}")
}

/// Decode an encoded tool name back to `(server_id_sanitized, tool_name)`.
/// Returns `None` if the encoded name doesn't contain `__`.
pub fn decode_tool_name(encoded: &str) -> Option<(&str, &str)> {
    encoded.split_once("__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let encoded = encode_tool_name("my-server", "read_file");
        assert_eq!(encoded, "my_server__read_file");
        let (sid, tname) = decode_tool_name(&encoded).unwrap();
        assert_eq!(sid, "my_server");
        assert_eq!(tname, "read_file");
    }

    #[test]
    fn encode_with_special_chars() {
        let encoded = encode_tool_name("brave.search@v2", "web_search");
        assert_eq!(encoded, "brave_search_v2__web_search");
    }

    #[test]
    fn decode_returns_none_without_separator() {
        assert!(decode_tool_name("noseparator").is_none());
    }

    #[test]
    fn mcp_tool_result_serializes() {
        let r = McpToolResult { content: "hello".into(), is_error: false };
        let j = serde_json::to_value(&r).unwrap();
        assert_eq!(j["content"], "hello");
        assert_eq!(j["is_error"], false);
    }
}

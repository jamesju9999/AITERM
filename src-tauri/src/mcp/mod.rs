// src-tauri/src/mcp/mod.rs
pub mod protocol;
pub mod transport;
pub mod types;

pub use types::{McpToolInfo, McpToolResult, McpServerStatus, encode_tool_name, decode_tool_name};

use std::collections::HashMap;
use protocol::{JsonRpcRequest, initialize_params, McpToolDescriptor, McpContentItem};
use transport::McpTransport;
use crate::config::types::McpServerConfig;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("Transport error: {0}")]
    Transport(#[from] transport::TransportError),
    #[error("Server not found: {0}")]
    ServerNotFound(String),
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
    #[error("JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },
    #[error("Missing result in response")]
    MissingResult,
}

struct ServerConnection {
    config: McpServerConfig,
    transport: McpTransport,
    tools: Vec<McpToolDescriptor>,
    status: McpServerStatus,
}

/// Manages connections to all configured MCP servers.
pub struct McpManager {
    connections: HashMap<String, ServerConnection>,
}

impl McpManager {
    pub fn new() -> Self {
        Self { connections: HashMap::new() }
    }

    /// Connect to all enabled servers in `configs`. Errors per-server are
    /// recorded in status; they don't abort the whole initialization.
    pub async fn connect_all(&mut self, configs: &[McpServerConfig]) {
        for cfg in configs {
            if !cfg.enabled {
                self.connections.insert(cfg.id.clone(), ServerConnection {
                    config: cfg.clone(),
                    transport: McpTransport::new_http(String::new()),
                    tools: vec![],
                    status: McpServerStatus::Disabled,
                });
                continue;
            }
            self.connections.insert(cfg.id.clone(), ServerConnection {
                config: cfg.clone(),
                transport: McpTransport::new_http(String::new()),
                tools: vec![],
                status: McpServerStatus::Connecting,
            });
            if let Err(e) = self.connect_one(&cfg.id).await {
                if let Some(conn) = self.connections.get_mut(&cfg.id) {
                    conn.status = McpServerStatus::Error { message: e.to_string() };
                }
            }
        }
    }

    async fn connect_one(&mut self, id: &str) -> Result<(), McpError> {
        use crate::config::types::McpTransport as CfgTransport;

        let cfg = self.connections[id].config.clone();

        let mut transport = match cfg.transport {
            CfgTransport::Stdio => {
                let cmd = cfg.command.as_deref().unwrap_or("");
                let t = transport::StdioTransport::spawn(cmd, &cfg.args, &cfg.env).await?;
                McpTransport::Stdio(std::sync::Arc::new(t))
            }
            CfgTransport::Http | CfgTransport::Sse => {
                let url = cfg.url.clone().unwrap_or_default();
                McpTransport::new_http(url)
            }
        };

        // MCP handshake: initialize
        let init_req = JsonRpcRequest::new(0, "initialize", initialize_params());
        let init_resp = transport.send_request(init_req).await?;
        if let Some(err) = init_resp.error {
            return Err(McpError::JsonRpc { code: err.code, message: err.message });
        }

        // List tools
        let list_req = JsonRpcRequest::new(0, "tools/list", serde_json::json!({}));
        let list_resp = transport.send_request(list_req).await?;
        if let Some(err) = list_resp.error {
            return Err(McpError::JsonRpc { code: err.code, message: err.message });
        }

        let tools: Vec<McpToolDescriptor> = list_resp.result
            .ok_or(McpError::MissingResult)
            .and_then(|v| {
                serde_json::from_value(v["tools"].clone())
                    .map_err(|e| McpError::Transport(transport::TransportError::Json(e)))
            })?;

        let tool_count = tools.len();
        let conn = self.connections.get_mut(id).unwrap();
        conn.transport = transport;
        conn.tools = tools;
        conn.status = McpServerStatus::Connected { tool_count };

        Ok(())
    }

    /// Returns all tools from all connected servers.
    pub fn list_tool_infos(&self) -> Vec<McpToolInfo> {
        let mut result = Vec::new();
        for conn in self.connections.values() {
            if !matches!(conn.status, McpServerStatus::Connected { .. }) { continue; }
            for tool in &conn.tools {
                result.push(McpToolInfo {
                    server_id: conn.config.id.clone(),
                    server_name: conn.config.name.clone(),
                    name: encode_tool_name(&conn.config.id, &tool.name),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                });
            }
        }
        result
    }

    /// Execute a tool call on the appropriate server.
    pub async fn call_tool(
        &mut self,
        encoded_name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let (server_id_sanitized, raw_tool_name) = decode_tool_name(encoded_name)
            .ok_or_else(|| McpError::ToolNotFound(encoded_name.to_string()))?;

        // Find the connection whose sanitized id matches
        let conn_id = self.connections.keys()
            .find(|id| {
                let sanitized: String = id.chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                sanitized == server_id_sanitized
            })
            .cloned()
            .ok_or_else(|| McpError::ServerNotFound(server_id_sanitized.to_string()))?;

        let conn = self.connections.get_mut(&conn_id)
            .ok_or_else(|| McpError::ServerNotFound(conn_id.clone()))?;

        let req = JsonRpcRequest::new(0, "tools/call", serde_json::json!({
            "name": raw_tool_name,
            "arguments": args
        }));

        let resp = conn.transport.send_request(req).await?;

        if let Some(err) = resp.error {
            return Err(McpError::JsonRpc { code: err.code, message: err.message });
        }

        let result = resp.result.ok_or(McpError::MissingResult)?;
        let is_error = result["isError"].as_bool().unwrap_or(false);
        let content = result["content"]
            .as_array()
            .map(|items| {
                items.iter()
                    .filter_map(|item| {
                        let c: McpContentItem = serde_json::from_value(item.clone()).ok()?;
                        Some(c.text)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        Ok(McpToolResult { content, is_error })
    }

    /// Get status of all connections (for UI display).
    pub fn list_server_statuses(&self) -> Vec<(String, McpServerStatus)> {
        self.connections.iter()
            .map(|(id, conn)| (id.clone(), conn.status.clone()))
            .collect()
    }

    /// Add or reconnect a single server without restarting all.
    pub async fn reconnect(&mut self, cfg: &McpServerConfig) {
        self.connections.remove(&cfg.id);
        self.connections.insert(cfg.id.clone(), ServerConnection {
            config: cfg.clone(),
            transport: McpTransport::new_http(String::new()),
            tools: vec![],
            status: McpServerStatus::Connecting,
        });
        if let Err(e) = self.connect_one(&cfg.id).await {
            if let Some(conn) = self.connections.get_mut(&cfg.id) {
                conn.status = McpServerStatus::Error { message: e.to_string() };
            }
        }
    }

    /// Disconnect and remove a server.
    pub fn remove_server(&mut self, id: &str) {
        self.connections.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_starts_empty() {
        let manager = McpManager::new();
        assert!(manager.list_tool_infos().is_empty());
    }

    #[test]
    fn encode_decode_preserved_in_tool_info() {
        use crate::mcp::types::encode_tool_name;
        let encoded = encode_tool_name("my-server", "read_file");
        assert_eq!(encoded, "my_server__read_file");
    }
}

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
    /// 每個連線各自一把鎖。
    ///
    /// **不要把它併回 manager 的那把鎖。** `execute_mcp_tool` 曾經寫成
    /// `mcp.lock().await.call_tool(...).await`，那個 MutexGuard 會活到整個
    /// 陳述式結束——也就是整個工具呼叫期間都握著 manager 的鎖。只要有一次
    /// 呼叫卡住不回（server 沒回應、stdio 塞住），`list_mcp_servers` 就再也
    /// 拿不到鎖，設定頁的 MCP 清單會永遠讀不出來，看起來就像「設定被刪了」。
    transport: std::sync::Arc<tokio::sync::Mutex<McpTransport>>,
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
                    transport: std::sync::Arc::new(tokio::sync::Mutex::new(McpTransport::new_http(String::new()))),
                    tools: vec![],
                    status: McpServerStatus::Disabled,
                });
                continue;
            }
            self.connections.insert(cfg.id.clone(), ServerConnection {
                config: cfg.clone(),
                transport: std::sync::Arc::new(tokio::sync::Mutex::new(McpTransport::new_http(String::new()))),
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
        conn.transport = std::sync::Arc::new(tokio::sync::Mutex::new(transport));
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
    /// 查出這個工具屬於哪個連線，回傳該連線的 transport 與解碼後的工具名。
    ///
    /// 刻意與實際送請求分成兩段：呼叫端拿到 `Arc` 之後就該**放掉 manager 的
    /// 鎖**，再去鎖那一個連線。合在一起寫的話，整個工具呼叫期間都握著
    /// manager 的鎖，任何一次卡住的呼叫都會讓 `list_mcp_servers` 永遠等下去。
    pub fn resolve_tool(
        &self,
        encoded_name: &str,
    ) -> Result<(std::sync::Arc<tokio::sync::Mutex<McpTransport>>, String), McpError> {
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

        let conn = self.connections.get(&conn_id)
            .ok_or_else(|| McpError::ServerNotFound(conn_id.clone()))?;
        Ok((conn.transport.clone(), raw_tool_name.to_string()))
    }

    /// 對已解析出的連線送出 `tools/call`。此時 manager 的鎖應該已經放掉了。
    pub async fn call_tool_on(
        transport: std::sync::Arc<tokio::sync::Mutex<McpTransport>>,
        raw_tool_name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let req = JsonRpcRequest::new(0, "tools/call", serde_json::json!({
            "name": raw_tool_name,
            "arguments": args
        }));

        let resp = transport.lock().await.send_request(req).await?;

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
            transport: std::sync::Arc::new(tokio::sync::Mutex::new(McpTransport::new_http(String::new()))),
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

    /// **送請求時不可以還握著 manager 的鎖。**
    ///
    /// 曾經寫成 `mcp.lock().await.call_tool(...).await`——那個 MutexGuard 會活
    /// 到整個陳述式結束，也就是整個工具呼叫期間都握著 manager 的鎖。只要一次
    /// 呼叫卡住不回（server 沒回應、stdio 塞住），`list_mcp_servers` 就再也拿
    /// 不到鎖，設定頁的 MCP 清單永遠讀不出來，看起來像「設定被刪了」。
    ///
    /// 型別本身就擋住這件事：`resolve_tool` 只需要 `&self`、且**不是** async，
    /// 所以它拿到 `Arc` 就結束，guard 沒有理由跨過任何 await；真正會 await 的
    /// `call_tool_on` 是關聯函式，簽名裡根本沒有 `self`，拿不到 manager 的鎖。
    ///
    /// 這條測試把那個性質釘住：如果有人把 `call_tool_on` 改回 `&mut self` 的
    /// 方法，這裡就編不過。
    #[test]
    fn sending_a_tool_call_does_not_require_the_manager() {
        let transport =
            std::sync::Arc::new(tokio::sync::Mutex::new(McpTransport::new_http(String::new())));
        // 建立 future 但不 await——async 函式的本體在被 poll 之前不會執行，
        // 所以這裡不會真的送出任何請求。能編譯就證明送請求只需要一個
        // transport 的 Arc，完全不需要 McpManager（也就拿不到它的鎖）。
        let _fut = McpManager::call_tool_on(transport, "some_tool", serde_json::json!({}));
    }

    /// `resolve_tool` 不是 async——呼叫端拿到 `Arc` 之後鎖就能放掉。
    #[test]
    fn resolving_a_tool_is_synchronous() {
        let manager = McpManager::new();
        // 沒有連線時回 ServerNotFound / ToolNotFound，重點是這行不需要 .await。
        let r = manager.resolve_tool("nonexistent__tool");
        assert!(r.is_err());
    }
}

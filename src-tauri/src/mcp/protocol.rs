// src-tauri/src/mcp/protocol.rs
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'static str,
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &'static str, params: Value) -> Self {
        Self { jsonrpc: "2.0", id, method, params }
    }
}

/// JSON-RPC 2.0 response (either result or error).
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub id: Option<u64>,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// MCP `initialize` params.
pub fn initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": {} },
        "clientInfo": { "name": "AITerm", "version": env!("CARGO_PKG_VERSION") }
    })
}

/// A single tool from `tools/list` response.
#[derive(Debug, Deserialize)]
pub struct McpToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
}

/// A single content item from a `tools/call` result.
#[derive(Debug, Deserialize)]
pub struct McpContentItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_request_serializes_correctly() {
        let req = JsonRpcRequest::new(1, "tools/list", serde_json::json!({}));
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["jsonrpc"], "2.0");
        assert_eq!(j["id"], 1);
        assert_eq!(j["method"], "tools/list");
    }

    #[test]
    fn json_rpc_response_parses_result() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn json_rpc_response_parses_error() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let resp: JsonRpcResponse = serde_json::from_str(raw).unwrap();
        assert!(resp.result.is_none());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}

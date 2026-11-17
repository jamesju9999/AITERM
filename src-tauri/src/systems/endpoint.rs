//! 動態 MCP 端點：把 `sys_*` 工具（代理 Playwright MCP）以 MCP（JSON-RPC over HTTP）提供給
//! Ask AI 迴圈與 Claude Code。手寫的原因：工具清單是動態代理來的，`rmcp` 的 `#[tool]` 是靜態方法。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::state::SystemsState;
use crate::bridge::auth as bridge_auth;

/// 鑰匙圈裡這個端點的 bearer token。
pub const ENDPOINT_TOKEN_KEY: &str = "systems-endpoint:token";
pub const DEFAULT_PORT: u16 = 8320;
/// 在 `AppConfig.mcp_servers` 裡自動維護的那一筆的 id。
pub const MCP_SERVER_ID: &str = "aiterm-systems";

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// 讓 AI 知道有哪些系統可用（id 要填進每個 sys_* 工具的 `system`）。
fn list_tool() -> Value {
    json!({
        "name": "sys_list",
        "description": "列出可操作的外部系統（id、名稱、權限模式）。其他 sys_* 工具的 system 參數要填這裡的 id。",
        "inputSchema": { "type": "object", "properties": {} }
    })
}

fn describe_systems(state: &SystemsState, config_conns: Vec<super::types::SystemConnection>) -> String {
    let _ = state;
    if config_conns.is_empty() {
        return "目前沒有設定任何系統連線。".into();
    }
    let list: Vec<Value> = config_conns
        .iter()
        .map(|c| json!({ "id": c.id, "name": c.name, "start_url": c.start_url, "mode": c.mode, "enabled": c.enabled }))
        .collect();
    serde_json::to_string_pretty(&list).unwrap_or_default()
}

/// 處理一則 JSON-RPC 訊息；通知（沒有 id）回 `None`。
pub async fn handle_rpc(state: &Arc<SystemsState>, req: Value) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let method = req["method"].as_str().unwrap_or("");
    Some(match method {
        "initialize" => rpc_result(
            &id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "aiterm-systems", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),
        "tools/list" => match state.catalog().await {
            Ok(mut tools) => {
                tools.insert(0, list_tool());
                rpc_result(&id, json!({ "tools": tools }))
            }
            Err(e) => rpc_error(&id, -32603, &e),
        },
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("").to_string();
            let args = req["params"]["arguments"].clone();
            if name == "sys_list" {
                rpc_result(&id, tool_text(describe_systems(state, state.connections()), false))
            } else {
                let system = args["system"].as_str().unwrap_or("").to_string();
                if system.is_empty() {
                    rpc_result(&id, tool_text("缺少 system 參數（先用 sys_list 查看可用的系統 id）".into(), true))
                } else {
                    match state.call(&system, &name, args).await {
                        Ok(v) => rpc_result(&id, v),
                        Err(e) => rpc_result(&id, tool_text(e, true)),
                    }
                }
            }
        }
        "ping" => rpc_result(&id, json!({})),
        _ => rpc_error(&id, -32601, "Method not found"),
    })
}

#[derive(Clone)]
struct AppState {
    systems: Arc<SystemsState>,
    token: Arc<String>,
}

async fn mcp_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    match bridge_auth::extract_token(authorization, None) {
        Some(p) if bridge_auth::token_matches(&app.token, &p) => {}
        _ => return Err(StatusCode::UNAUTHORIZED),
    }
    match handle_rpc(&app.systems, req).await {
        Some(resp) => Ok((StatusCode::OK, Json(resp))),
        None => Ok((StatusCode::ACCEPTED, Json(Value::Null))),
    }
}

pub struct EndpointState {
    running: Mutex<Option<(u16, tokio::sync::oneshot::Sender<()>)>>,
}

impl Default for EndpointState {
    fn default() -> Self {
        Self { running: Mutex::new(None) }
    }
}

impl EndpointState {
    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.0)
    }

    /// 啟動（已啟動就直接成功）。只綁 127.0.0.1；埠被占用就報錯，不自己換埠（換了的話已註冊的客戶端會指向死位址）。
    pub async fn start(&self, systems: Arc<SystemsState>, token: String, port: u16) -> Result<(), String> {
        if self.port() == Some(port) {
            return Ok(());
        }
        self.stop();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("無法綁定 127.0.0.1:{port}（{e}）"))?;
        let app = Router::new()
            .route("/systems/mcp", post(mcp_handler))
            .with_state(AppState { systems, token: Arc::new(token) });
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await;
        });
        *self.running.lock() = Some((port, tx));
        Ok(())
    }

    pub fn stop(&self) {
        if let Some((_, tx)) = self.running.lock().take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;

    fn state_with_catalog() -> Arc<SystemsState> {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        let s = Arc::new(SystemsState::new(cfg));
        s.set_catalog_for_test(vec![json!({"name":"sys_snapshot"})]);
        s
    }

    #[tokio::test]
    async fn initialize_and_notifications() {
        let s = state_with_catalog();
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})).await.unwrap();
        assert_eq!(r["result"]["serverInfo"]["name"], "aiterm-systems");
        assert!(handle_rpc(&s, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.is_none());
    }

    #[tokio::test]
    async fn tools_list_has_sys_list_first_then_the_catalog() {
        let s = state_with_catalog();
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.unwrap();
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["sys_list", "sys_snapshot"]);
    }

    #[tokio::test]
    async fn unknown_method_and_missing_system_are_reported() {
        let s = state_with_catalog();
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":3,"method":"nope"})).await.unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"sys_click","arguments":{}}})).await.unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("system"));
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"sys_snapshot","arguments":{"system":"ghost"}}})).await.unwrap();
        assert_eq!(r["result"]["isError"], true, "不存在的系統回工具層錯誤，不是 JSON-RPC 錯誤");
    }

    #[tokio::test]
    async fn the_http_endpoint_requires_the_bearer_token() {
        let s = state_with_catalog();
        let ep = EndpointState::default();
        let port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(s, "secret".into(), port).await.unwrap();
        let url = format!("http://127.0.0.1:{port}/systems/mcp");
        let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        let c = reqwest::Client::new();
        assert_eq!(c.post(&url).json(&body).send().await.unwrap().status(), 401);
        assert_eq!(c.post(&url).bearer_auth("wrong").json(&body).send().await.unwrap().status(), 401);
        let ok = c.post(&url).bearer_auth("secret").json(&body).send().await.unwrap();
        assert_eq!(ok.status(), 200);
        ep.stop();
    }

    /// 整條後端：HTTP（bearer）→ MCP 端點 → 狀態 → 代理 → 閘門 → 真 Chrome。
    /// `cargo test -p app --lib -- --ignored --test-threads=1 systems::endpoint`
    #[tokio::test]
    #[ignore]
    async fn over_http_a_write_is_refused_and_the_server_sees_nothing() {
        use crate::systems::testsite::{conn, fake_site};
        use crate::systems::types::Mode;
        use std::sync::atomic::Ordering;

        let (site_port, posts) = fake_site().await;
        let dir = tempfile::tempdir().unwrap();
        let cfg = Arc::new(ConfigStore::new_at(dir.path().join("config.json")));
        cfg.update(|a| a.system_connections.push(conn(site_port, Mode::ReadOnly))).unwrap();
        let st = Arc::new(SystemsState::new(cfg));
        let ep = EndpointState::default();
        let port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(st.clone(), "tok".into(), port).await.unwrap();

        let url = format!("http://127.0.0.1:{port}/systems/mcp");
        let c = reqwest::Client::new();
        let rpc = |id: u64, method: &str, params: Value| {
            let c = c.clone();
            let url = url.clone();
            let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
            async move { c.post(&url).bearer_auth("tok").json(&body).send().await.unwrap().json::<Value>().await.unwrap() }
        };
        let text = |v: &Value| v["result"]["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n");

        let list = rpc(1, "tools/list", json!({})).await;
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"sys_click") && names.contains(&"sys_list") && !names.contains(&"sys_evaluate"), "{names:?}");

        let listed = rpc(2, "tools/call", json!({"name":"sys_list","arguments":{}})).await;
        assert!(text(&listed).contains("\"id\": \"t\""));

        let site = format!("http://127.0.0.1:{site_port}/");
        rpc(3, "tools/call", json!({"name":"sys_navigate","arguments":{"system":"t","url":site,"intent":"讀：開首頁"}})).await;
        let snap = rpc(4, "tools/call", json!({"name":"sys_snapshot","arguments":{"system":"t"}})).await;
        let t = text(&snap);
        let r = t.split("button \"Save\" [ref=").nth(1).and_then(|s| s.split(']').next()).expect(&t).to_string();
        let no_intent = rpc(5, "tools/call", json!({"name":"sys_click","arguments":{"system":"t","target":r}})).await;
        assert_eq!(no_intent["result"]["isError"], true, "沒帶 intent 要被拒絕");
        let click = rpc(6, "tools/call", json!({"name":"sys_click","arguments":{"system":"t","target":r,"intent":"寫：按儲存"}})).await;
        assert!(text(&click).contains("被閘門拒絕"), "{}", text(&click));
        assert_eq!(posts.load(Ordering::SeqCst), 0);

        st.close("t").await;
        ep.stop();
    }
}

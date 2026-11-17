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
pub const DEFAULT_PORT: u16 = 48320;
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

/// 配方相關的工具（不經 Playwright，由我們自己處理）。
fn recipe_tools() -> Vec<Value> {
    let sys = json!({ "type": "string", "description": "系統連線的 id" });
    vec![
        json!({
            "name": "sys_recipe_list",
            "description": "列出這個系統已存的配方（名稱、說明、參數、步數、成功／失敗次數）。",
            "inputSchema": { "type": "object", "properties": { "system": sys }, "required": ["system"] }
        }),
        json!({
            "name": "sys_recipe_save",
            "description": "把你剛剛成功完成的一串操作存成配方，之後可用 sys_recipe_run 直接重播。只會存成功的動作；存完後操作日誌清空。\
                            有會變的值（例如單號）就用 params 宣告：name 是參數名稱，example 是你剛才實際用的值。",
            "inputSchema": { "type": "object", "properties": {
                "system": sys,
                "name": { "type": "string", "description": "配方名稱，同系統內唯一，同名會覆蓋" },
                "description": { "type": "string", "description": "這個配方做什麼" },
                "params": { "type": "array", "items": { "type": "object", "properties": {
                    "name": { "type": "string" }, "example": { "type": "string" } }, "required": ["name", "example"] } },
                "last_n": { "type": "integer", "description": "只取最後幾步；不填就是上次存配方之後的全部" }
            }, "required": ["system", "name"] }
        }),
        json!({
            "name": "sys_recipe_run",
            "description": "重播一個配方。每一步仍照該系統的寫入權限判定，需要核准的照樣等使用者。任何一步失敗就停下並回報是哪一步。",
            "inputSchema": { "type": "object", "properties": {
                "system": sys,
                "name": { "type": "string" },
                "params": { "type": "object", "description": "參數名稱 → 值", "additionalProperties": { "type": "string" } }
            }, "required": ["system", "name"] }
        }),
    ]
}

/// 處理配方工具；不是配方工具就回 `None`。
async fn handle_recipe_tool(state: &Arc<SystemsState>, name: &str, args: &Value) -> Option<Result<String, String>> {
    let system = args["system"].as_str().unwrap_or("").to_string();
    let rname = args["name"].as_str().unwrap_or("").to_string();
    match name {
        "sys_recipe_list" => Some(state.recipe_list(&system).await.map(|l| {
            if l.is_empty() {
                "這個系統還沒有配方。".to_string()
            } else {
                serde_json::to_string_pretty(&l.iter().map(|r| json!({
                    "name": r.name, "description": r.description, "params": r.params,
                    "steps": r.steps.len(), "ok": r.ok_count, "failed": r.fail_count
                })).collect::<Vec<_>>()).unwrap_or_default()
            }
        })),
        "sys_recipe_save" => {
            let params: Vec<super::recipes::Param> = serde_json::from_value(args["params"].clone()).unwrap_or_default();
            let last_n = args["last_n"].as_u64().map(|n| n as usize);
            let desc = args["description"].as_str().unwrap_or("");
            Some(state.recipe_save(&system, &rname, desc, params, last_n).await)
        }
        "sys_recipe_run" => {
            let values: std::collections::HashMap<String, String> = args["params"]
                .as_object()
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect())
                .unwrap_or_default();
            Some(state.recipe_run(&system, &rname, values).await)
        }
        _ => None,
    }
}

fn describe_systems(config_conns: Vec<super::types::SystemConnection>) -> String {
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
pub async fn handle_rpc(state: &Arc<SystemsState>, req: Value, scope: &Scope) -> Option<Value> {
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
                tools.splice(0..0, std::iter::once(list_tool()).chain(recipe_tools()));
                rpc_result(&id, json!({ "tools": tools }))
            }
            Err(e) => rpc_error(&id, -32603, &e),
        },
        "tools/call" => {
            let name = req["params"]["name"].as_str().unwrap_or("").to_string();
            let args = req["params"]["arguments"].clone();
            if name == "sys_list" {
                rpc_result(&id, tool_text(describe_systems(scope.filter(state.connections())), false))
            } else {
                let system = args["system"].as_str().unwrap_or("").to_string();
                if !system.is_empty() && !scope.allows(&system) {
                    return Some(rpc_result(&id, tool_text(format!("這個工作沒有被授權使用系統連線「{system}」（先用 sys_list 看可用的系統）"), true)));
                }
                if let Some(r) = if system.is_empty() { None } else { handle_recipe_tool(state, &name, &args).await } {
                    match r {
                        Ok(t) => rpc_result(&id, tool_text(t, false)),
                        Err(e) => rpc_result(&id, tool_text(e, true)),
                    }
                } else if system.is_empty() {
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

/// 呼叫者能碰哪些系統：主 token＝全部；Agent 卡片拿到的是只含被授權系統的限定 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Only(Vec<String>),
}

impl Scope {
    pub fn allows(&self, system: &str) -> bool {
        match self {
            Scope::All => true,
            Scope::Only(ids) => ids.iter().any(|i| i == system),
        }
    }

    pub fn filter(&self, conns: Vec<super::types::SystemConnection>) -> Vec<super::types::SystemConnection> {
        conns.into_iter().filter(|c| self.allows(&c.id)).collect()
    }
}

#[derive(Clone)]
struct AppState {
    systems: Arc<SystemsState>,
    token: Arc<String>,
    scoped: Arc<Mutex<std::collections::HashMap<String, Vec<String>>>>,
}

async fn mcp_handler(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<Value>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    let scope = match bridge_auth::extract_token(authorization, None) {
        Some(p) if bridge_auth::token_matches(&app.token, &p) => Scope::All,
        Some(p) => match app.scoped.lock().get(&p) {
            Some(ids) => Scope::Only(ids.clone()),
            None => return Err(StatusCode::UNAUTHORIZED),
        },
        None => return Err(StatusCode::UNAUTHORIZED),
    };
    match handle_rpc(&app.systems, req, &scope).await {
        Some(resp) => Ok((StatusCode::OK, Json(resp))),
        None => Ok((StatusCode::ACCEPTED, Json(Value::Null))),
    }
}

pub struct EndpointState {
    running: Mutex<Option<(u16, String, tokio::sync::oneshot::Sender<()>)>>,
    /// 限定 token → 可用的系統 id。給 Agent 卡片用，只存在記憶體，App 重啟就失效。
    scoped: Arc<Mutex<std::collections::HashMap<String, Vec<String>>>>,
}

impl Default for EndpointState {
    fn default() -> Self {
        Self { running: Mutex::new(None), scoped: Arc::new(Mutex::new(std::collections::HashMap::new())) }
    }
}

impl EndpointState {
    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.0)
    }

    /// 目前這個端點實際在用的 token。要以它為準：鑰匙圈不可用時 token 只存在記憶體，
    /// 重新產生一個會讓端點與已寫進設定的 token 對不上。
    pub fn token(&self) -> Option<String> {
        self.running.lock().as_ref().map(|r| r.1.clone())
    }

    /// 啟動（已啟動就直接成功）。只綁 127.0.0.1；埠被占用就報錯，不自己換埠（換了的話已註冊的客戶端會指向死位址）。
    pub async fn start(&self, systems: Arc<SystemsState>, token: String, port: u16) -> Result<(), String> {
        if self.port() == Some(port) {
            return Ok(());
        }
        self.stop();
        let token_for_state = token.clone();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("無法綁定 127.0.0.1:{port}（{e}）"))?;
        let app = Router::new()
            .route("/systems/mcp", post(mcp_handler))
            .with_state(AppState { systems, token: Arc::new(token), scoped: self.scoped.clone() });
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await;
        });
        *self.running.lock() = Some((port, token_for_state, tx));
        Ok(())
    }

    /// 發一個只能用這幾個系統的 token（Agent 卡片派工時用）。同一組系統重複要會拿到同一個。
    pub fn issue_scoped_token(&self, systems: &[String]) -> String {
        let mut ids: Vec<String> = systems.to_vec();
        ids.sort();
        ids.dedup();
        let mut map = self.scoped.lock();
        if let Some((tok, _)) = map.iter().find(|(_, v)| **v == ids) {
            return tok.clone();
        }
        let tok = crate::bridge::auth::generate_token();
        map.insert(tok.clone(), ids);
        tok
    }

    pub fn stop(&self) {
        if let Some((_, _, tx)) = self.running.lock().take() {
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
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}), &Scope::All).await.unwrap();
        assert_eq!(r["result"]["serverInfo"]["name"], "aiterm-systems");
        assert!(handle_rpc(&s, json!({"jsonrpc":"2.0","method":"notifications/initialized"}), &Scope::All).await.is_none());
    }

    #[tokio::test]
    async fn tools_list_has_sys_list_and_the_recipe_tools_first_then_the_catalog() {
        let s = state_with_catalog();
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), &Scope::All).await.unwrap();
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["sys_list", "sys_recipe_list", "sys_recipe_save", "sys_recipe_run", "sys_snapshot"]);
    }

    #[tokio::test]
    async fn unknown_method_and_missing_system_are_reported() {
        let s = state_with_catalog();
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":3,"method":"nope"}), &Scope::All).await.unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"sys_click","arguments":{}}}), &Scope::All).await.unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("system"));
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"sys_snapshot","arguments":{"system":"ghost"}}}), &Scope::All).await.unwrap();
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
        assert_eq!(ep.token().as_deref(), Some("secret"), "要能問到端點實際在用的 token");
        ep.stop();
        assert_eq!(ep.token(), None);
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
        let st = Arc::new(SystemsState::with_pool(cfg, crate::systems::audit::open_in_memory().await));
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

    #[tokio::test]
    async fn a_scoped_caller_only_sees_and_uses_the_systems_it_was_granted() {
        let s = state_with_catalog();
        let only_a = Scope::Only(vec!["a".into()]);
        assert!(only_a.allows("a") && !only_a.allows("b") && Scope::All.allows("anything"));
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"sys_snapshot","arguments":{"system":"b"}}}), &only_a).await.unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("沒有被授權"), "別的系統一律擋下");
        // 配方工具也一樣。
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sys_recipe_list","arguments":{"system":"b"}}}), &only_a).await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("沒有被授權"));
        // sys_list 只列被授權的。
        let r = handle_rpc(&s, json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sys_list","arguments":{}}}), &only_a).await.unwrap();
        assert!(!r["result"]["content"][0]["text"].as_str().unwrap().contains("\"b\""));
    }

    #[tokio::test]
    async fn scoped_tokens_authenticate_but_only_for_their_systems() {
        let s = state_with_catalog();
        let ep = EndpointState::default();
        let port = crate::systems::browser::pick_free_port().unwrap();
        ep.start(s, "master".into(), port).await.unwrap();
        let tok = ep.issue_scoped_token(&["a".to_string(), "b".to_string()]);
        assert_eq!(tok, ep.issue_scoped_token(&["b".to_string(), "a".to_string()]), "同一組系統拿到同一個 token");
        assert_ne!(tok, ep.issue_scoped_token(&["a".to_string()]));
        let url = format!("http://127.0.0.1:{port}/systems/mcp");
        let c = reqwest::Client::new();
        let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        assert_eq!(c.post(&url).bearer_auth(&tok).json(&body).send().await.unwrap().status(), 200);
        assert_eq!(c.post(&url).bearer_auth("made-up").json(&body).send().await.unwrap().status(), 401);
        let denied = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"sys_snapshot","arguments":{"system":"z"}}});
        let r: Value = c.post(&url).bearer_auth(&tok).json(&denied).send().await.unwrap().json().await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("沒有被授權"), "{r}");
        ep.stop();
    }
}

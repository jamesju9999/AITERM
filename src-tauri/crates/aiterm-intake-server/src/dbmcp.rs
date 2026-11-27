//! 給 Claude Code 用的資料庫工具：server 內建的 MCP 端點。
//!
//! 只聽 127.0.0.1 的純 HTTP 埠（Claude 是本機子行程，不必走 TLS）。每次執行發一個隨機 token，對應
//! {卡片、Agent、專案、資料庫技能}，卡結束（`McpGuard` 被丟掉）就作廢。協定是 MCP 的 Streamable HTTP：
//! 一個 POST 一個 JSON-RPC 訊息，回單一 JSON 回應（通知回 202）。
//! 權限在這裡擋（不靠提示詞）：唯讀擋寫入、寫入依模式放行／等管理員核准。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aiterm_db::skill::{authorize, DbSkill, Decision};
use aiterm_intake_core::board_store::{self as bs, NewDbQuery};
use aiterm_intake_core::host::Notifier;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::state::AppState;

/// MCP 伺服器在 Claude 設定裡的名字（`--allowedTools` 用 `mcp__aiterm-db`）。
pub const SERVER_NAME: &str = "aiterm-db";
/// 系統連線工具的 MCP 伺服器名字（`--allowedTools` 用 `mcp__aiterm-systems`）。
pub const SYSTEMS_SERVER_NAME: &str = "aiterm-systems";
/// 「中途問人」工具的 MCP 伺服器名字（`--allowedTools` 用 `mcp__aiterm-ask`）；每張卡都有。
pub const ASK_SERVER_NAME: &str = "aiterm-ask";
/// 等管理員核准最久多久；超過就當作拒絕。
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(24 * 3600);
const APPROVAL_POLL: Duration = Duration::from_millis(500);
/// 單句查詢最久執行多久。
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
/// Claude 的工具呼叫逾時要比等核准久（毫秒），否則等核准時工具呼叫會先逾時。
pub const TOOL_TIMEOUT_ENV: (&str, &str) = ("MCP_TOOL_TIMEOUT", "86400000");

/// 一個 token 對應的執行環境。
#[derive(Clone)]
pub struct McpCtx {
    pub card_id: String,
    pub agent_id: Option<String>,
    pub project_id: String,
    /// 資料庫技能（沒裝就是 `None`）。
    pub skill: Option<DbSkill>,
    /// 這張卡能用的系統連線 id（server 系統）。
    pub systems: Vec<String>,
}

#[derive(Default)]
pub struct McpState {
    tokens: parking_lot::Mutex<HashMap<String, McpCtx>>,
    addr: parking_lot::Mutex<Option<SocketAddr>>,
    task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl McpState {
    pub fn addr(&self) -> Option<SocketAddr> {
        *self.addr.lock()
    }
    pub fn shutdown(&self) {
        if let Some(t) = self.task.lock().take() {
            t.abort();
        }
    }
    fn get(&self, token: &str) -> Option<McpCtx> {
        self.tokens.lock().get(token).cloned()
    }
    pub(crate) fn alive(&self, token: &str) -> bool {
        self.tokens.lock().contains_key(token)
    }
}

/// 啟動 MCP 端點（127.0.0.1 的隨機埠）。重複呼叫不會再開一個。
pub fn start(st: &Arc<AppState>) -> std::io::Result<SocketAddr> {
    if let Some(a) = st.mcp.addr() {
        return Ok(a);
    }
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    std_listener.set_nonblocking(true)?;
    let addr = std_listener.local_addr()?;
    let listener = tokio::net::TcpListener::from_std(std_listener)?;
    let app = Router::new().route("/mcp/{token}", post(handle)).route("/mcp/{token}/systems", post(handle_sys)).route("/mcp/{token}/ask", post(handle_ask)).with_state(st.clone());
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    *st.mcp.addr.lock() = Some(addr);
    *st.mcp.task.lock() = Some(task);
    Ok(addr)
}

/// 這次執行的 MCP 註冊。被丟掉時 token 作廢，還在等核准的查詢標成拒絕（沒有人在等了）。
pub struct McpGuard {
    st: Arc<AppState>,
    token: String,
    card_id: String,
    /// 寫給 Claude 的 `--mcp-config` 檔。
    pub config_path: std::path::PathBuf,
}

impl std::fmt::Debug for McpGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpGuard").field("card_id", &self.card_id).finish()
    }
}

impl Drop for McpGuard {
    fn drop(&mut self) {
        self.st.mcp.tokens.lock().remove(&self.token);
        // 在背景把這張卡還在等的查詢收掉（Drop 不能 await）。
        let st = self.st.clone();
        let card = self.card_id.clone();
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move {
                crate::syscore::close_card(&st, &card).await;
                // 還在等人回答的問題：沒有行程在等了。
                if aiterm_intake_core::board_questions::close_card(&st.pools.intake, &card).await.unwrap_or(0) > 0 {
                    crate::asknotify::closed(&st, &card).await;
                }
                let _ = sqlx::query("UPDATE board_db_queries SET decision = 'denied', finished_at = ?, error = '卡片已結束，沒有人在等這個查詢' WHERE card_id = ? AND decision = 'pending'")
                    .bind(crate::db::now())
                    .bind(&card)
                    .execute(&st.pools.intake)
                    .await;
                st.events.changed();
            });
        }
    }
}

/// 為一張卡註冊資料庫工具：發 token、寫 MCP 設定檔。MCP 端點還沒啟動（或設定檔寫不出來）回 `None`。
pub fn register(st: &Arc<AppState>, ctx: McpCtx, dir: &std::path::Path) -> Option<McpGuard> {
    let addr = st.mcp.addr()?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let mut servers = serde_json::Map::new();
    servers.insert(ASK_SERVER_NAME.into(), json!({"type": "http", "url": format!("http://{addr}/mcp/{token}/ask")}));
    if ctx.skill.is_some() {
        servers.insert(SERVER_NAME.into(), json!({"type": "http", "url": format!("http://{addr}/mcp/{token}")}));
    }
    if !ctx.systems.is_empty() {
        servers.insert(SYSTEMS_SERVER_NAME.into(), json!({"type": "http", "url": format!("http://{addr}/mcp/{token}/systems")}));
    }
    let config = json!({"mcpServers": servers});
    std::fs::create_dir_all(dir).ok()?;
    let config_path = dir.join("db-mcp.json");
    std::fs::write(&config_path, config.to_string()).ok()?;
    let card_id = ctx.card_id.clone();
    st.mcp.tokens.lock().insert(token.clone(), ctx);
    Some(McpGuard { st: st.clone(), token, card_id, config_path })
}

// ---- JSON-RPC ----

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn tools() -> Value {
    let conn = json!({"type": "string", "description": "連線 id（db_list_connections 看得到）"});
    json!([
        {"name": "db_list_connections", "description": "列出你可以用的資料庫連線（id、名稱、類型）。", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "db_list_schemas", "description": "列出連線裡的 schema。", "inputSchema": {"type": "object", "properties": {"connection_id": conn}, "required": ["connection_id"]}},
        {"name": "db_list_tables", "description": "列出 schema 裡的資料表與檢視表。", "inputSchema": {"type": "object", "properties": {"connection_id": conn, "schema": {"type": "string"}}, "required": ["connection_id", "schema"]}},
        {"name": "db_table_schema", "description": "列出資料表的欄位（名稱、型別、是否可空、預設值）。", "inputSchema": {"type": "object", "properties": {"connection_id": conn, "schema": {"type": "string"}, "table": {"type": "string"}}, "required": ["connection_id", "schema", "table"]}},
        {"name": "db_query", "description": "執行一句 SQL。唯讀查詢（SELECT／WITH／SHOW／DESCRIBE／EXPLAIN）一律可以；寫入依這個連線的權限：被擋下、要等管理員核准，或直接執行。", "inputSchema": {"type": "object", "properties": {"connection_id": conn, "sql": {"type": "string"}}, "required": ["connection_id", "sql"]}}
    ])
}

/// 資料庫工具端點（`/mcp/<token>`）。
async fn handle(State(st): State<Arc<AppState>>, Path(token): Path<String>, Json(msg): Json<Value>) -> Response {
    serve(&st, &token, msg, Kind::Db).await
}

/// 中途問人工具端點（`/mcp/<token>/ask`）。
async fn handle_ask(State(st): State<Arc<AppState>>, Path(token): Path<String>, Json(msg): Json<Value>) -> Response {
    serve(&st, &token, msg, Kind::Ask).await
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Db,
    Sys,
    Ask,
}

/// 系統連線工具端點（`/mcp/<token>/systems`）。
async fn handle_sys(State(st): State<Arc<AppState>>, Path(token): Path<String>, Json(msg): Json<Value>) -> Response {
    serve(&st, &token, msg, Kind::Sys).await
}

async fn serve(st: &Arc<AppState>, token: &str, msg: Value, kind: Kind) -> Response {
    let systems = kind == Kind::Sys;
    let Some(ctx) = st.mcp.get(token) else { return (StatusCode::UNAUTHORIZED, "token 無效或已作廢").into_response() };
    let id = msg.get("id").cloned();
    let method = msg["method"].as_str().unwrap_or("");
    // 沒有 id 的是通知（例如 notifications/initialized）：不回內容。
    let Some(id) = id else { return StatusCode::ACCEPTED.into_response() };
    let reply = match method {
        "initialize" => rpc_result(
            &id,
            json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": match kind { Kind::Sys => SYSTEMS_SERVER_NAME, Kind::Ask => ASK_SERVER_NAME, Kind::Db => SERVER_NAME }, "version": st.version.clone()}
            }),
        ),
        "ping" => rpc_result(&id, json!({})),
        "tools/list" if systems => match sys_tools(st, &ctx).await {
            Ok(t) => rpc_result(&id, json!({"tools": t})),
            Err(e) => rpc_error(&id, -32603, &e),
        },
        "tools/list" if kind == Kind::Ask => rpc_result(&id, json!({"tools": crate::askmcp::tools()})),
        "tools/list" => rpc_result(&id, json!({"tools": tools()})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or("").to_string();
            let args = msg["params"]["arguments"].clone();
            if kind == Kind::Ask {
                rpc_result(&id, crate::askmcp::call(st, token, &ctx, &name, &args).await)
            } else if systems {
                rpc_result(&id, call_sys_tool(st, &ctx, &name, args).await)
            } else {
                rpc_result(&id, call_tool(st, token, &ctx, &name, &args).await)
            }
        }
        _ => rpc_error(&id, -32601, "不支援的方法"),
    };
    Json(reply).into_response()
}

/// `sys_list` ＋ Playwright 的 `sys_*` 工具目錄。
async fn sys_tools(st: &Arc<AppState>, ctx: &McpCtx) -> Result<Value, String> {
    if ctx.systems.is_empty() {
        return Ok(json!([]));
    }
    let mut tools = vec![json!({"name": "sys_list", "description": "列出你可以操作的外部系統（id、名稱、起始網址、權限模式）。其他 sys_* 工具的 system 參數要填這裡的 id。", "inputSchema": {"type": "object", "properties": {}}})];
    tools.extend(crate::syscore::catalog(st).await?);
    Ok(Value::Array(tools))
}

async fn call_sys_tool(st: &Arc<AppState>, ctx: &McpCtx, name: &str, args: Value) -> Value {
    if ctx.systems.is_empty() {
        return tool_text("這位 Agent 沒有系統連線技能".into(), true);
    }
    if name == "sys_list" {
        return match crate::syscore::list(st, ctx).await {
            Ok(t) => tool_text(t, false),
            Err(e) => tool_text(e, true),
        };
    }
    if !name.starts_with("sys_") {
        return tool_text(format!("沒有這個工具：{name}"), true);
    }
    match crate::syscore::call(st, ctx, name, args).await {
        Ok(v) => v,
        Err(e) => tool_text(e, true),
    }
}

fn arg<'a>(args: &'a Value, k: &str) -> &'a str {
    args[k].as_str().unwrap_or("")
}

async fn call_tool(st: &Arc<AppState>, token: &str, ctx: &McpCtx, name: &str, args: &Value) -> Value {
    let r = match name {
        "db_list_connections" => list_connections(st, ctx).await,
        "db_list_schemas" | "db_list_tables" | "db_table_schema" => meta(st, ctx, name, args).await,
        "db_query" => query(st, token, ctx, arg(args, "connection_id"), arg(args, "sql")).await,
        other => Err(format!("沒有這個工具：{other}")),
    };
    match r {
        Ok(text) => tool_text(text, false),
        Err(why) => tool_text(why, true),
    }
}

fn skill_of(ctx: &McpCtx) -> Result<&DbSkill, String> {
    ctx.skill.as_ref().ok_or_else(|| "這位 Agent 沒有資料庫技能".to_string())
}

fn allowed(ctx: &McpCtx, conn_id: &str) -> Result<(), String> {
    if skill_of(ctx)?.connections.iter().any(|c| c == conn_id) {
        Ok(())
    } else {
        Err("這個連線沒有開放給你，先用 db_list_connections 看可以用哪些。".into())
    }
}

async fn list_connections(st: &AppState, ctx: &McpCtx) -> Result<String, String> {
    let mut out = Vec::new();
    for id in &skill_of(ctx)?.connections {
        if let Some(c) = bs::get_db_conn(&st.pools.intake, id).await.map_err(|e| e.to_string())? {
            out.push(json!({"id": c.id, "name": c.name, "db_type": c.db_type, "database": c.database}));
        }
    }
    if out.is_empty() {
        return Ok("沒有可用的資料庫連線。".into());
    }
    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
}

async fn meta(st: &Arc<AppState>, ctx: &McpCtx, tool: &str, args: &Value) -> Result<String, String> {
    let conn_id = arg(args, "connection_id");
    allowed(ctx, conn_id)?;
    let a = crate::dbconn::adapter_for(st, conn_id).await?;
    let run = async {
        match tool {
            "db_list_schemas" => serde_json::to_string_pretty(&a.list_schemas().await.map_err(|e| e.to_string())?),
            "db_list_tables" => serde_json::to_string_pretty(&a.list_tables(arg(args, "schema")).await.map_err(|e| e.to_string())?),
            _ => serde_json::to_string_pretty(&a.get_table_schema(arg(args, "schema"), arg(args, "table")).await.map_err(|e| e.to_string())?),
        }
        .map_err(|e| e.to_string())
    };
    tokio::time::timeout(QUERY_TIMEOUT, run).await.map_err(|_| "查詢逾時".to_string())?
}

/// 執行並記錄。`decision` 是這筆記錄的結論（allowed／approved）。
async fn run_and_log(st: &Arc<AppState>, ctx: &McpCtx, conn_name: &str, conn_id: &str, sql: &str, row_id: Option<i64>) -> Result<String, String> {
    let id = match row_id {
        Some(id) => id,
        None => bs::start_db_query(&st.pools.intake, &NewDbQuery { project_id: &ctx.project_id, card_id: &ctx.card_id, agent_id: ctx.agent_id.as_deref(), connection_id: conn_id, connection_name: conn_name, sql, decision: "allowed" }).await.map_err(|e| e.to_string())?,
    };
    st.events.changed();
    let started = Instant::now();
    let outcome = async {
        let a = crate::dbconn::adapter_for(st, conn_id).await?;
        let res = tokio::time::timeout(QUERY_TIMEOUT, a.execute(sql)).await.map_err(|_| format!("查詢逾時（超過 {} 秒）", QUERY_TIMEOUT.as_secs()))?.map_err(|e| e.to_string())?;
        aiterm_db::connect::format_result(&res)
    }
    .await;
    let ms = started.elapsed().as_millis() as i64;
    match &outcome {
        Ok((_, rows)) => bs::finish_db_query(&st.pools.intake, id, Some(*rows as i64), ms, None).await.ok(),
        Err(e) => bs::finish_db_query(&st.pools.intake, id, None, ms, Some(e)).await.ok(),
    };
    st.events.changed();
    outcome.map(|(text, _)| text)
}

async fn query(st: &Arc<AppState>, token: &str, ctx: &McpCtx, conn_id: &str, sql: &str) -> Result<String, String> {
    let conn = bs::get_db_conn(&st.pools.intake, conn_id).await.map_err(|e| e.to_string())?;
    let conn_name = conn.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| conn_id.to_string());
    let q = NewDbQuery { project_id: &ctx.project_id, card_id: &ctx.card_id, agent_id: ctx.agent_id.as_deref(), connection_id: conn_id, connection_name: &conn_name, sql, decision: "allowed" };
    let skill = skill_of(ctx)?;
    if conn.is_none() && skill.connections.iter().any(|c| c == conn_id) {
        return Err("這個連線已經不存在（管理員可能刪掉了）。".into());
    }
    match authorize(skill, conn_id, sql) {
        Decision::Deny(why) => {
            bs::record_denied_db_query(&st.pools.intake, &q, &why).await.ok();
            st.events.changed();
            Err(why)
        }
        Decision::Allow => run_and_log(st, ctx, &conn_name, conn_id, sql, None).await,
        Decision::NeedApproval => {
            let id = bs::start_db_query(&st.pools.intake, &NewDbQuery { decision: "pending", ..q }).await.map_err(|e| e.to_string())?;
            st.events.changed();
            let started = Instant::now();
            loop {
                tokio::time::sleep(APPROVAL_POLL).await;
                match bs::get_db_query(&st.pools.intake, id).await.map_err(|e| e.to_string())? {
                    Some(row) if row.decision == "approved" => return run_and_log(st, ctx, &conn_name, conn_id, sql, Some(id)).await,
                    Some(row) if row.decision == "denied" => return Err("管理員拒絕了這句寫入，不要硬做；需要的話請在待確認事項說明原因。".into()),
                    None => return Err("找不到這筆查詢記錄".into()),
                    _ => {}
                }
                if !st.mcp.alive(token) {
                    bs::decide_db_query(&st.pools.intake, id, false, "system").await.ok();
                    return Err("這張卡已經結束，寫入沒有執行。".into());
                }
                if started.elapsed() > APPROVAL_TIMEOUT {
                    bs::decide_db_query(&st.pools.intake, id, false, "system").await.ok();
                    return Err("等管理員核准等太久，寫入沒有執行。".into());
                }
            }
        }
    }
}

/// 提示詞附註：連線名稱與權限（沿用桌面版的說明文字）。
pub async fn prompt_note(st: &AppState, skill: &DbSkill) -> String {
    let mut names = Vec::new();
    for id in &skill.connections {
        if let Ok(Some(c)) = bs::get_db_conn(&st.pools.intake, id).await {
            names.push((c.id, c.name));
        }
    }
    aiterm_db::skill::prompt_note(skill, &names)
}

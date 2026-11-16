# VCS 團隊狀態 MCP Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓終端機分頁裡的 Claude Code CLI 也能查詢「團隊目前在做什麼功能、會不會跟我要改的檔案重疊」，不用切去版本控制分頁。

**Architecture:** AITerm 常駐一個綁 `127.0.0.1` 的本機 HTTP MCP server（比照既有的 `bridge` 模組——同一種「本機常駐 HTTP server 給 Claude Code CLI 連」的模式，但這是完全獨立的 server/port，不是同一個），開放 `list_active_features`、`check_file_overlap` 兩個工具，內部直接重用 [`vcs-team-visibility` plan](./2026-08-17-vcs-team-visibility.md) 已經做好的 `GitClient::list_active_features` 與 `overlap::find_overlaps`。AITerm 會把這個 server 的位址寫進 Claude Code 的使用者層級專案設定（`~/.claude.json` 的 `projects.<repo路徑>.mcpServers`），不寫進 repo 本身，所以不會有「不同人 AITerm 埠不一樣、寫進 git 的設定檔對不上」的問題。

**Tech Stack:** `axum`（本機常駐已有這個 crate 的既有慣例）、`reqwest`／`serde_json`、`wiremock` 或直接用 `reqwest` 打自己的伺服器做整合測試。

**依賴：** 本 plan 假設 `docs/superpowers/plans/2026-08-17-vcs-team-visibility.md` 已經完整執行完（需要 `GitClient::list_active_features`、`overlap::find_overlaps`、`StartFeatureDialog.tsx` 都已存在）。

**前置技術查證（已驗證，非猜測）：**
- Claude Code 的專案層級 MCP server 可以用 `type: "http"` + `url`，且能寫在使用者層級的 `~/.claude.json` 的 `projects.<絕對路徑>.mcpServers` 底下，不需要寫進 repo 的 `.mcp.json`。
- 這個機制在互動式 session 裡第一次使用仍然會跳「信任這個工作區」的提示——這是 Claude Code 自己的安全機制，AITerm 沒有辦法繞過，本 plan 不假裝能做到完全零感。
- MCP 的 Streamable HTTP transport 允許伺服器完全不用 SSE、每個請求都直接回 JSON——因為這個 server 沒有任何需要主動推播給 client 的情境（工具呼叫都是同步的），本 plan 採用這個最簡形式，不做 session 管理。

---

## 檔案總覽

**新增（Rust）：**
- `src-tauri/src/vcs/mcp_server/mod.rs` — `VcsMcpState`（啟動/停止/取得埠，仿照 `bridge::BridgeState`）
- `src-tauri/src/vcs/mcp_server/server.rs` — axum router + JSON-RPC 2.0 handler
- `src-tauri/src/vcs/mcp_server/tools.rs` —兩個工具的實作與 schema
- `src-tauri/src/vcs/mcp_server/registration.rs` — `~/.claude.json` 合併寫入邏輯（純函式 + 檔案 I/O 分開）
- `src-tauri/tests/vcs_mcp_server.rs` — 對真的跑起來的 server 打 HTTP 請求的整合測試

**修改（Rust）：**
- `src-tauri/src/vcs/mod.rs` — 新增 `pub mod mcp_server;`
- `src-tauri/src/commands/vcs.rs` — 新增 `vcs_ensure_mcp_registered` 指令
- `src-tauri/src/lib.rs` — 註冊新指令、`.manage()` 新 state、`.setup()` 裡啟動 server

**修改（前端）：**
- `src/ipc/vcs.ts` — 新增 `vcsEnsureMcpRegistered` 包裝
- `src/components/VcsView/StartFeatureDialog.tsx` — 開始功能成功後呼叫註冊

---

## Task 1: `VcsMcpState` 生命週期管理

**Files:**
- Create: `src-tauri/src/vcs/mcp_server/mod.rs`

- [ ] **Step 1: 建立 state 管理（仿照 `bridge::BridgeState`，不需要先寫測試——這是純粹的生命週期管理，行為在 Task 4 的整合測試裡會被驗證到）**

Create `src-tauri/src/vcs/mcp_server/mod.rs`：

```rust
//! 本機常駐的 MCP server：讓終端機分頁裡的 Claude Code CLI 能查詢團隊目前
//! 進行中的功能與檔案重疊狀況。跟 `bridge` 模組是同一種「本機 HTTP server
//! 給 Claude Code CLI 連」的模式，但這是完全獨立的 server/port——bridge
//! 是可選功能（`ClaudeBridgeConfig.enabled`），這個則是 app 一啟動就常駐，
//! 兩者互相獨立，其中一個關掉不影響另一個。

pub mod registration;
pub mod server;
pub mod tools;

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::ConfigStore;
use crate::secret::SecretStore;

pub struct VcsMcpState {
    running: Mutex<Option<Running>>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for VcsMcpState {
    fn default() -> Self {
        Self { running: Mutex::new(None) }
    }
}

impl VcsMcpState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// 啟動 server，優先嘗試 `preferred_port`，被占用就往上找最多 10 個埠。
    /// 這個 server 沒有使用者可見的「設定埠」介面（不像 bridge），
    /// 所以用自動找空埠而非直接回錯——被占用不該讓整個功能不可用。
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        preferred_port: u16,
    ) -> anyhow::Result<u16> {
        self.stop();

        let mut last_err = None;
        for candidate in preferred_port..preferred_port.saturating_add(10) {
            let addr = SocketAddr::from(([127, 0, 0, 1], candidate));
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let app = server::router(server::McpAppState {
                        config: config.clone(),
                        secrets: secrets.clone(),
                    });
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    tokio::spawn(async move {
                        let served = axum::serve(listener, app)
                            .with_graceful_shutdown(async { let _ = rx.await; })
                            .await;
                        if let Err(e) = served {
                            log::error!("vcs mcp server 結束於錯誤：{e}");
                        }
                    });
                    *self.running.lock() = Some(Running { port: candidate, shutdown: tx });
                    return Ok(candidate);
                }
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow::anyhow!(
            "無法在 {preferred_port}-{} 範圍內找到可用的埠：{last_err:?}",
            preferred_port.saturating_add(9)
        ))
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}
```

- [ ] **Step 2: 編譯確認（`server` 模組還沒建立，預期會失敗）**

Run: `cd src-tauri && cargo check`
Expected: 編譯失敗，`server`/`registration`/`tools` 模組找不到——這是預期的，下面幾個 task 會補上。

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/vcs/mcp_server/mod.rs
git commit -m "feat(vcs-mcp): add VcsMcpState lifecycle (server module comes next)"
```

（這個 commit 本身編不過，是刻意的中間狀態——下一個 task 馬上補上 `server` 模組讓它能編。若團隊慣例不允許不能編譯的中間 commit，可以把 Task 1-2 合併成一個 commit。）

---

## Task 2: JSON-RPC handler 骨架（`initialize` + `tools/list`）

**Files:**
- Create: `src-tauri/src/vcs/mcp_server/server.rs`
- Create: `src-tauri/src/vcs/mcp_server/tools.rs`
- Create: `src-tauri/tests/vcs_mcp_server.rs`

- [ ] **Step 1: 寫整合測試（先確認會紅）**

Create `src-tauri/tests/vcs_mcp_server.rs`：

```rust
use std::sync::Arc;
use aiterm_lib::config::ConfigStore;
use aiterm_lib::secret::SecretStore;
use aiterm_lib::vcs::mcp_server::VcsMcpState;

async fn start_test_server() -> (Arc<VcsMcpState>, u16) {
    let state = Arc::new(VcsMcpState::new());
    let config = Arc::new(ConfigStore::from_config(Default::default()));
    let secrets = Arc::new(SecretStore::new());
    let port = state.start(config, secrets, 0).await.expect("server should start on an ephemeral port");
    (state, port)
}

#[tokio::test]
async fn initialize_returns_protocol_version_and_tool_capability() {
    let (_state, port) = start_test_server().await;
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
        .send().await.unwrap()
        .json().await.unwrap();

    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn tools_list_returns_both_tools_with_schemas() {
    let (_state, port) = start_test_server().await;
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
        .send().await.unwrap()
        .json().await.unwrap();

    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"list_active_features"));
    assert!(names.contains(&"check_file_overlap"));
}

#[tokio::test]
async fn unknown_method_returns_json_rpc_error() {
    let (_state, port) = start_test_server().await;
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "nonexistent/method", "params": {}}))
        .send().await.unwrap()
        .json().await.unwrap();

    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn initialized_notification_returns_202_with_no_body() {
    let (_state, port) = start_test_server().await;
    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .send().await.unwrap();

    assert_eq!(resp.status(), 202);
}
```

（`ConfigStore::from_config`／`SecretStore::new` 已經是既有專案測試裡常見的建構方式，見 `src-tauri/src/commands/vcs.rs` 裡的 `resolve_embedder_config_tests`。`state.start(..., 0)` 傳入 port `0`——`TcpListener::bind` 對 `0` 會配一個作業系統挑的空埠，但目前的 `start` 實作是从 `preferred_port` 逐一往上找，傳 `0` 會先試 `0` 本身失敗一次才往上找到真的可用的埠；這在多數作業系統上 `bind` 到埠 `0` 其實會成功並配發隨機埠，所以第一次嘗試就會成功。）

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_mcp_server`
Expected: 編譯失敗，`server`/`tools` 模組不存在。

- [ ] **Step 3: 建立 `tools.rs`（先只放 schema，工具邏輯留給 Task 3）**

Create `src-tauri/src/vcs/mcp_server/tools.rs`：

```rust
//! 兩個 MCP 工具的定義與（Task 3 補上的）實作。

use serde_json::{json, Value};

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "list_active_features",
            "description": "List all team members' in-progress features (open GitHub PRs) for a repo, with the files each one has changed. Call this before starting new work to see what teammates are doing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Absolute path to the git repository (use your current working directory)."
                    }
                },
                "required": ["repo_path"]
            }
        },
        {
            "name": "check_file_overlap",
            "description": "Check whether the files you're about to edit overlap with any teammate's in-progress feature. Call this before making changes to avoid conflicts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Absolute path to the git repository (use your current working directory)."
                    },
                    "files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Repo-relative file paths you're about to edit."
                    }
                },
                "required": ["repo_path", "files"]
            }
        }
    ])
}
```

- [ ] **Step 4: 建立 `server.rs`**

Create `src-tauri/src/vcs/mcp_server/server.rs`：

```rust
//! MCP 的 Streamable HTTP transport 裡最簡單的合法形式：單一 POST 端點，
//! 每個請求同步回一個 JSON-RPC 回應，不用 SSE、不用 session id——這個
//! server 的兩個工具呼叫都是「問一次、答一次」，沒有需要伺服器主動推播
//! 給 client 的情境，所以不需要 transport 規格裡更複雜的那些部分。

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use super::tools;

#[derive(Clone)]
pub struct McpAppState {
    pub config: Arc<ConfigStore>,
    pub secrets: Arc<SecretStore>,
}

#[derive(Deserialize)]
struct JsonRpcIn {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

pub fn router(state: McpAppState) -> Router {
    Router::new()
        .route("/mcp", post(handle))
        .with_state(state)
}

async fn handle(State(state): State<McpAppState>, Json(req): Json<JsonRpcIn>) -> Response {
    // 通知（沒有 id）不需要回應內容，只要 202 讓 client 知道收到了。
    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    let result = match req.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "aiterm-vcs-team", "version": env!("CARGO_PKG_VERSION") }
        })),
        "tools/list" => Ok(json!({ "tools": tools::tool_definitions() })),
        "tools/call" => tools::handle_call(&state, &req.params).await,
        other => Err((-32601i64, format!("Method not found: {other}"))),
    };

    let body = match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": req.id, "result": value }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": req.id, "error": { "code": code, "message": message } }),
    };
    Json(body).into_response()
}
```

- [ ] **Step 5: 補上 `tools::handle_call` 的暫時骨架（回傳 not-implemented，讓上面能編譯過；Task 3 會補上真正的實作）**

在 `tools.rs` 的 `tool_definitions` 後面加入：

```rust
use super::server::McpAppState;

pub async fn handle_call(_state: &McpAppState, _params: &Value) -> Result<Value, (i64, String)> {
    Err((-32601, "tool not yet implemented".to_string()))
}
```

- [ ] **Step 6: 註冊模組**

`src-tauri/src/vcs/mod.rs` 加入：

```rust
pub mod mcp_server;
```

- [ ] **Step 7: 執行測試**

Run: `cd src-tauri && cargo test --test vcs_mcp_server`
Expected: `initialize_returns_protocol_version_and_tool_capability`、`tools_list_returns_both_tools_with_schemas`、`unknown_method_returns_json_rpc_error`、`initialized_notification_returns_202_with_no_body` 全部 `ok`。

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/vcs/mcp_server/ src-tauri/src/vcs/mod.rs src-tauri/tests/vcs_mcp_server.rs
git commit -m "feat(vcs-mcp): add JSON-RPC handler skeleton (initialize, tools/list)"
```

---

## Task 3: 兩個工具的真正實作（`tools/call`）

**Files:**
- Modify: `src-tauri/src/vcs/mcp_server/tools.rs`
- Modify: `src-tauri/tests/vcs_mcp_server.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/tests/vcs_mcp_server.rs` 加入（需要一個真的 git repo 加假的 remote，跟 `vcs_git_client.rs` 裡的 helper 手法一樣；由於這裡沒有真的 GitHub token，測試改成驗證「沒有 token 時回傳看得懂的錯誤」，而不是打真的 GitHub API）：

```rust
async fn init_repo_with_origin(dir: &std::path::Path) {
    std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/acme/widget.git"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn list_active_features_tool_call_without_token_returns_readable_error() {
    let (_state, port) = start_test_server().await;
    let dir = tempfile::tempdir().unwrap();
    init_repo_with_origin(dir.path()).await;

    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "list_active_features", "arguments": { "repo_path": dir.path().to_string_lossy() } }
        }))
        .send().await.unwrap()
        .json().await.unwrap();

    // 沒設定任何 VCS 連線，預期回傳 isError 內容而不是整個 HTTP 失敗——
    // 讓呼叫端（Claude Code）能把錯誤內容當成一般的工具結果處理。
    assert_eq!(resp["result"]["isError"], true);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("尚未設定"), "unexpected message: {text}");
}

#[tokio::test]
async fn unknown_tool_name_returns_json_rpc_error() {
    let (_state, port) = start_test_server().await;
    let resp: serde_json::Value = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/mcp"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": { "name": "does_not_exist", "arguments": {} }
        }))
        .send().await.unwrap()
        .json().await.unwrap();

    assert!(resp["error"]["code"].is_i64());
}
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `cd src-tauri && cargo test --test vcs_mcp_server list_active_features_tool_call`
Expected: 目前的骨架永遠回 `-32601 tool not yet implemented`，斷言不符，測試失敗。

- [ ] **Step 3: 實作 `handle_call` 與連線解析**

把 `tools.rs` 最下面暫時的 `handle_call` 換成：

```rust
use crate::config::types::VcsType;
use crate::vcs::git::GitClient;
use crate::vcs::overlap::find_overlaps;

/// MVP 簡化：假設使用者只設定了一個 Git 類型的 VCS 連線（小團隊的典型情況），
/// 用清單裡第一個當 token 來源。若之後要支援多個 GitHub 帳號/組織，
/// 這裡需要改成依 repo remote 的 owner 比對到正確的連線。
fn resolve_token(state: &McpAppState) -> Option<String> {
    let connection = state.config.get().vcs_connections.into_iter().find(|c| c.vcs_type == VcsType::Git)?;
    state.secrets.get(&format!("vcs:{}", connection.id)).ok().flatten()
}

fn text_result(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn error_result(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

pub async fn handle_call(state: &McpAppState, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"].as_str().unwrap_or("");
    let args = &params["arguments"];

    match name {
        "list_active_features" => {
            let repo_path = args["repo_path"].as_str().unwrap_or("").to_string();
            if repo_path.is_empty() {
                return Ok(error_result("缺少 repo_path 參數".to_string()));
            }
            let Some(token) = resolve_token(state) else {
                return Ok(error_result("尚未設定 GitHub 連線的 VCS token，請先在 AITerm 設定頁新增".to_string()));
            };
            let client = GitClient::new(repo_path, Some(token));
            match client.list_active_features().await {
                Ok(features) if features.is_empty() => Ok(text_result("目前沒有任何隊友有進行中的功能。".to_string())),
                Ok(features) => {
                    let lines: Vec<String> = features.iter().map(|f| {
                        format!("「{}」— {}（{}）改動: {}", f.title, f.author,
                            if f.draft { "進行中" } else { "待審核" }, f.files.join(", "))
                    }).collect();
                    Ok(text_result(lines.join("\n")))
                }
                Err(e) => Ok(error_result(format!("查詢失敗：{e}"))),
            }
        }
        "check_file_overlap" => {
            let repo_path = args["repo_path"].as_str().unwrap_or("").to_string();
            let files: Vec<String> = args["files"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if repo_path.is_empty() || files.is_empty() {
                return Ok(error_result("缺少 repo_path 或 files 參數".to_string()));
            }
            let Some(token) = resolve_token(state) else {
                return Ok(error_result("尚未設定 GitHub 連線的 VCS token，請先在 AITerm 設定頁新增".to_string()));
            };
            let client = GitClient::new(repo_path, Some(token));
            match client.list_active_features().await {
                Ok(all_features) => {
                    let hits = find_overlaps(&files, &all_features);
                    if hits.is_empty() {
                        Ok(text_result("沒有跟任何進行中的功能重疊。".to_string()))
                    } else {
                        let lines: Vec<String> = hits.iter()
                            .map(|f| format!("跟 {} 的「{}」重疊", f.author, f.title))
                            .collect();
                        Ok(text_result(lines.join("\n")))
                    }
                }
                Err(e) => Ok(error_result(format!("查詢失敗：{e}"))),
            }
        }
        other => Err((-32602, format!("Unknown tool: {other}"))),
    }
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `cd src-tauri && cargo test --test vcs_mcp_server`
Expected: 全部通過（累積 6 個測試）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/mcp_server/tools.rs src-tauri/tests/vcs_mcp_server.rs
git commit -m "feat(vcs-mcp): implement list_active_features and check_file_overlap tool calls"
```

---

## Task 4: `~/.claude.json` 註冊邏輯

**Files:**
- Create: `src-tauri/src/vcs/mcp_server/registration.rs`

- [ ] **Step 1: 寫純函式的失敗測試**

Create `src-tauri/src/vcs/mcp_server/registration.rs`：

```rust
//! 把這個 MCP server 的位址寫進 Claude Code 的使用者層級專案設定
//! （`~/.claude.json` 的 `projects.<repo路徑>.mcpServers`），不寫進 repo
//! 本身——這個位址（埠）是每台機器各自的，寫進 repo 對其他隊友沒有意義，
//! 也會造成「设定檔裡的埠對不上」的困惑。
//!
//! `~/.claude.json` 的完整 schema 不是 AITerm 管的（Claude Code 自己還會
//! 存其他東西在裡面），所以用 `serde_json::Value` 做「只改我關心的那個
//! 欄位，其他原封不動」的合併，不能整份反序列化成自訂 struct 再寫回去
//! （那樣任何 AITerm 不認得的欄位都會在寫回時消失）。

use serde_json::{json, Value};

pub const MCP_SERVER_KEY: &str = "aiterm-team";

/// 把一筆 MCP server 註冊合併進既有的 `~/.claude.json` 內容。
/// `existing` 若不是物件（例如檔案是空的、或內容壞掉）會被重置成空物件；
/// 除了 `projects.<repo_root>.mcpServers.aiterm-team` 這一個欄位，
/// 其他既有內容一律保留原樣。
pub fn merge_registration(existing: &mut Value, repo_root: &str, port: u16) {
    if !existing.is_object() {
        *existing = json!({});
    }
    let root = existing.as_object_mut().unwrap();

    let projects = root.entry("projects").or_insert_with(|| json!({}));
    if !projects.is_object() {
        *projects = json!({});
    }

    let project_entry = projects
        .as_object_mut()
        .unwrap()
        .entry(repo_root.to_string())
        .or_insert_with(|| json!({}));
    if !project_entry.is_object() {
        *project_entry = json!({});
    }

    let mcp_servers = project_entry
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if !mcp_servers.is_object() {
        *mcp_servers = json!({});
    }

    mcp_servers.as_object_mut().unwrap().insert(
        MCP_SERVER_KEY.to_string(),
        json!({ "type": "http", "url": format!("http://127.0.0.1:{port}/mcp") }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_full_structure_from_empty_file() {
        let mut existing = json!({});
        merge_registration(&mut existing, "/repo/widget", 47823);
        assert_eq!(
            existing["projects"]["/repo/widget"]["mcpServers"]["aiterm-team"]["url"],
            "http://127.0.0.1:47823/mcp"
        );
    }

    #[test]
    fn preserves_unrelated_top_level_fields() {
        let mut existing = json!({ "numStartups": 42, "theme": "dark" });
        merge_registration(&mut existing, "/repo/widget", 47823);
        assert_eq!(existing["numStartups"], 42);
        assert_eq!(existing["theme"], "dark");
    }

    #[test]
    fn preserves_other_projects_and_other_mcp_servers_on_the_same_project() {
        let mut existing = json!({
            "projects": {
                "/repo/other": { "mcpServers": { "something-else": { "type": "stdio" } } },
                "/repo/widget": { "mcpServers": { "unrelated-tool": { "type": "http", "url": "http://x" } } }
            }
        });
        merge_registration(&mut existing, "/repo/widget", 47823);

        assert_eq!(existing["projects"]["/repo/other"]["mcpServers"]["something-else"]["type"], "stdio");
        assert_eq!(existing["projects"]["/repo/widget"]["mcpServers"]["unrelated-tool"]["type"], "http");
        assert_eq!(
            existing["projects"]["/repo/widget"]["mcpServers"]["aiterm-team"]["url"],
            "http://127.0.0.1:47823/mcp"
        );
    }

    #[test]
    fn rerunning_with_a_new_port_updates_the_url_idempotently() {
        let mut existing = json!({});
        merge_registration(&mut existing, "/repo/widget", 47823);
        merge_registration(&mut existing, "/repo/widget", 47824);

        assert_eq!(
            existing["projects"]["/repo/widget"]["mcpServers"]["aiterm-team"]["url"],
            "http://127.0.0.1:47824/mcp"
        );
    }
}
```

- [ ] **Step 2: 執行測試確認通過**

Run: `cd src-tauri && cargo test --lib vcs::mcp_server::registration`
Expected: 4 個測試全過。

（這裡直接給出正確實作——邏輯是機械式的巢狀 `entry().or_insert_with()`，不需要刻意先看紅燈；若要嚴格照紅燈流程，可以先讓 `merge_registration` 函式體是空的 `{}` 跑一次確認全部失敗，再貼上面的實作。）

- [ ] **Step 3: 加入讀寫真實檔案的部分（不寫測試——檔案 I/O 邊界，Task 5 的 Tauri 指令會是這個功能唯一的呼叫端，行為由 Task 5 的手動驗證步驟確認）**

在 `registration.rs` 最下面（`#[cfg(test)]` 之前）加入：

```rust
/// 讀取、合併、寫回 `~/.claude.json`。檔案不存在或內容不是合法 JSON
/// 都當成「從空的開始」，不視為錯誤——使用者可能從來沒建立過這個檔案。
pub fn write_registration(repo_root: &str, port: u16) -> Result<(), String> {
    let path = dirs::home_dir()
        .ok_or_else(|| "找不到使用者家目錄".to_string())?
        .join(".claude.json");

    let mut existing: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));

    merge_registration(&mut existing, repo_root, port);

    let pretty = serde_json::to_string_pretty(&existing).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| format!("寫入 {} 失敗：{e}", path.display()))
}
```

（已知限制：若 Claude Code CLI 當下正在寫這個檔案，理論上存在競爭寫入風險——兩邊都是「整份讀取、整份寫回」，幾乎同時發生時後寫的會覆蓋先寫的。這個機率極低（使用者要剛好在 AITerm 呼叫這支函式的瞬間也在跑一個會寫這個檔案的 Claude Code 操作），本 plan 不特別處理鎖，是已知、可接受的限制。）

- [ ] **Step 4: 編譯確認**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/mcp_server/registration.rs
git commit -m "feat(vcs-mcp): merge MCP server registration into ~/.claude.json without clobbering other fields"
```

---

## Task 5: Tauri 指令、app 啟動時常駐、前端接線

**Files:**
- Modify: `src-tauri/src/commands/vcs.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/ipc/vcs.ts`
- Modify: `src/components/VcsView/StartFeatureDialog.tsx`

- [ ] **Step 1: 新增 Tauri 指令**

在 `src-tauri/src/commands/vcs.rs` 檔案最後加入：

```rust
#[tauri::command]
pub async fn vcs_ensure_mcp_registered(
    repo_root: String,
    mcp_state: State<'_, Arc<crate::vcs::mcp_server::VcsMcpState>>,
) -> Result<(), String> {
    let port = mcp_state.port().ok_or_else(|| "MCP server 尚未啟動".to_string())?;
    crate::vcs::mcp_server::registration::write_registration(&repo_root, port)
}
```

- [ ] **Step 2: 註冊指令與 state、app 啟動時常駐**

`src-tauri/src/lib.rs` 的 import 清單裡把 `vcs_ensure_mcp_registered` 加進既有 vcs 指令那組 `use` 陳述式。

`generate_handler!` 清單裡加入 `vcs_ensure_mcp_registered,`。

在既有的 `.manage(Arc::new(bridge::BridgeState::new()))`（第 277 行）附近加入：

```rust
        .manage(Arc::new(vcs::mcp_server::VcsMcpState::new()))
```

在 `.setup()` 裡、既有的 bridge 啟動區塊（第 290-316 行）後面加入：

```rust
            // VCS 團隊狀態 MCP server：跟 bridge 不同，這個沒有「啟用開關」，
            // app 一開就常駐，讓終端機分頁的 Claude Code CLI 隨時能查。
            // 埠被占用只記 log，不擋 app 啟動。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let mcp_state = handle.state::<Arc<vcs::mcp_server::VcsMcpState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    const DEFAULT_VCS_MCP_PORT: u16 = 47823;
                    if let Err(e) = mcp_state.start(config, secrets, DEFAULT_VCS_MCP_PORT).await {
                        log::error!("VCS MCP server 啟動失敗：{e}");
                    }
                });
            }
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo check`
Expected: 編譯成功。

- [ ] **Step 4: 前端 IPC 包裝**

在 `src/ipc/vcs.ts` 加入：

```typescript
export function vcsEnsureMcpRegistered(repoRoot: string): Promise<void> {
  return invoke<void>("vcs_ensure_mcp_registered", { repoRoot });
}
```

- [ ] **Step 5: 在 `StartFeatureDialog` 開始功能成功後呼叫註冊**

`src/components/VcsView/StartFeatureDialog.tsx` 頂部 import 加入：

```typescript
import { vcsCheckOverlap, vcsEnsureMcpRegistered, vcsStartFeature, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";
```

把 `doStart` 函式：

```tsx
  const doStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await vcsStartFeature(repoInfo, name, baseBranch, parseFileList(filesText));
      void outcome;
      onStarted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
```

改成：

```tsx
  const doStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await vcsStartFeature(repoInfo, name, baseBranch, parseFileList(filesText));
      void outcome;
      // 讓這個 repo 底下的 Claude Code CLI 能查團隊狀態；註冊失敗不影響
      // 「開始新功能」本身已經成功這件事，只記錄下來，不擋流程。
      vcsEnsureMcpRegistered(repoInfo.root).catch((e) => {
        console.warn("MCP registration failed:", e);
      });
      onStarted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
```

- [ ] **Step 6: 更新 `StartFeatureDialog.test.tsx` 的 mock**

`src/components/VcsView/StartFeatureDialog.test.tsx` 檔案上方的 `vi.mock("../../ipc/vcs", ...)` 加入 `vcsEnsureMcpRegistered` 的 mock：

```typescript
const checkOverlapMock = vi.fn();
const startFeatureMock = vi.fn();
const ensureMcpRegisteredMock = vi.fn();
vi.mock("../../ipc/vcs", () => ({
  vcsCheckOverlap: (...args: unknown[]) => checkOverlapMock(...args),
  vcsStartFeature: (...args: unknown[]) => startFeatureMock(...args),
  vcsEnsureMcpRegistered: (...args: unknown[]) => ensureMcpRegisteredMock(...args),
}));
```

`beforeEach` 加入 `ensureMcpRegisteredMock.mockReset();`，並讓它預設 resolve：

```typescript
beforeEach(() => {
  checkOverlapMock.mockReset();
  startFeatureMock.mockReset();
  ensureMcpRegisteredMock.mockReset();
  ensureMcpRegisteredMock.mockResolvedValue(undefined);
});
```

- [ ] **Step 7: 執行前端測試確認沒有回歸**

Run: `npx vitest run src/components/VcsView/StartFeatureDialog.test.tsx`
Expected: 原本 3 個測試依然全過（這個 task 沒有新增前端測試案例，只是確保既有測試在多了一個 mock 呼叫的情況下不會壞）。

- [ ] **Step 8: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 9: 手動驗證整個鏈路**

Run: `npm run tauri:dev`

1. app 開起來後，確認 log 沒有「VCS MCP server 啟動失敗」。
2. 在一個設定好 GitHub token 的 repo 走一次「開始新功能」。
3. 檢查 `~/.claude.json` 確實多了 `projects.<repo路徑>.mcpServers.aiterm-team`，`url` 指向 `http://127.0.0.1:47823/mcp`。
4. 在同一個 repo 目錄下開一個終端機分頁跑 `claude`，問它「幫我查一下團隊目前有哪些進行中的功能」——第一次應該會跳工作區信任的提示，同意後應該能看到它呼叫 `list_active_features` 並讀到剛剛開的那個功能。
5. 關掉 AITerm 整個 app，在系統終端機（不透過 AITerm）直接跑 `claude` 問同樣的問題——確認它會說查不到/工具呼叫失敗，但**不會整個卡住或報錯終止**，仍然能正常對話。這一步驗證的是 spec 裡承諾的「AITerm 沒開著就優雅降級」。

- [ ] **Step 10: Commit**

```bash
git add src-tauri/src/commands/vcs.rs src-tauri/src/lib.rs src/ipc/vcs.ts src/components/VcsView/StartFeatureDialog.tsx src/components/VcsView/StartFeatureDialog.test.tsx
git commit -m "feat(vcs-mcp): start MCP server on app launch and auto-register repos on feature start"
```

---

## 完成後驗證

- [ ] `cd src-tauri && cargo test --lib vcs::mcp_server && cargo test --test vcs_mcp_server` 全綠
- [ ] `npx vitest run` 全綠
- [ ] `npx tsc -b` 無錯誤
- [ ] Task 5 Step 9 的五步手動驗證都做過，包含「AITerm 沒開著」的降級情境

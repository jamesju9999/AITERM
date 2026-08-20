//! Integration test: hits the real axum router (via `tower::ServiceExt::oneshot`,
//! no TCP listener) to verify auth gating and a full MCP protocol round trip
//! (initialize -> notifications/initialized -> tools/call).

use std::sync::Arc;

use aiterm_lib::mcp_server::server::router;
use aiterm_lib::config::ConfigStore;
use aiterm_lib::db::db2_sidecar::Db2SidecarState;
use aiterm_lib::db::manager::DbManager;
use aiterm_lib::mcp_server::coordination_ops::CoordinationRegistry;
use aiterm_lib::pty::manager::PtyManager;
use aiterm_lib::secret::SecretStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// `coordination_enabled: false` (the real default) unless
/// `coordination_enabled` is true — matching this codebase's actual default
/// config, so most tests exercise the disabled-by-default gate without any
/// special setup.
async fn test_router_with(coordination_enabled: bool) -> (axum::Router, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
    if coordination_enabled {
        config.update(|c| c.mcp_tool_server.coordination_enabled = true).unwrap();
    }
    let secrets = Arc::new(SecretStore::new());
    let db_manager = Arc::new(DbManager::new());
    let sidecar = Arc::new(Db2SidecarState::new(dir.path().to_path_buf()));
    let pty_manager = Arc::new(PtyManager::new());
    let coordination_registry = Arc::new(CoordinationRegistry::new());

    let kb_pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE notebooks (
            id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, folder_path TEXT NOT NULL,
            embed_provider_id TEXT, embed_model TEXT, embed_dim INTEGER,
            last_synced_at INTEGER, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&kb_pool).await.unwrap();

    let app = router(
        Arc::new("t0ken".to_string()),
        db_manager,
        config,
        secrets,
        sidecar,
        kb_pool.clone(),
        pty_manager,
        None,
        coordination_registry,
    );
    (app, kb_pool)
}

async fn test_router() -> (axum::Router, sqlx::SqlitePool) {
    test_router_with(false).await
}

fn post(uri: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    // rmcp's StreamableHttpService does DNS-rebinding protection and rejects
    // requests with no Host header (default allowed_hosts: localhost,
    // 127.0.0.1, ::1) — see rmcp-3.1.3 src/transport/streamable_http_server/tower.rs.
    let mut b = Request::builder().method("POST").uri(uri)
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn health_check_does_not_require_auth() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn mcp_endpoint_rejects_missing_token() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(post(
        "/mcp", None,
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_endpoint_rejects_wrong_token() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(post(
        "/mcp", Some("nope"),
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Full protocol round trip: initialize (capture the session id header) ->
/// notifications/initialized -> tools/call(list_connections). Proves the
/// rmcp wiring actually speaks MCP correctly end to end, not just that the
/// axum plumbing compiles.
#[tokio::test]
async fn full_round_trip_lists_connections() {
    let (app, _pool) = test_router().await;

    let init_resp = app.clone().oneshot(post(
        "/mcp", Some("t0ken"),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.1"}
            }
        }),
    )).await.unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let session_id = init_resp.headers().get("mcp-session-id").expect("server should assign a session id").to_str().unwrap().to_string();

    let mut notify_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    notify_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let notify_resp = app.clone().oneshot(notify_req).await.unwrap();
    assert!(notify_resp.status().is_success(), "{:?}", notify_resp.status());

    let mut call_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "list_connections", "arguments": {}}
    }));
    call_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let call_resp = app.oneshot(call_req).await.unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(call_resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("No DB connections configured"), "{text}");
}

/// Performs the initialize -> notifications/initialized -> tools/call
/// handshake (see `full_round_trip_lists_connections` above) and returns the
/// tools/call response body as text. Factored out because the coordination
/// tool tests below need this same handshake repeatedly.
async fn call_tool(app: axum::Router, name: &str, arguments: serde_json::Value) -> String {
    let init_resp = app.clone().oneshot(post(
        "/mcp", Some("t0ken"),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.1"}
            }
        }),
    )).await.unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let session_id = init_resp.headers().get("mcp-session-id").expect("server should assign a session id").to_str().unwrap().to_string();

    let mut notify_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    notify_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let notify_resp = app.clone().oneshot(notify_req).await.unwrap();
    assert!(notify_resp.status().is_success(), "{:?}", notify_resp.status());

    let mut call_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    }));
    call_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let call_resp = app.oneshot(call_req).await.unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(call_resp.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8_lossy(&body).to_string()
}

/// Coordination tools are disabled by default (`coordination_enabled: false`
/// in the real default config) even when the MCP tool server itself is
/// running — confirms `require_coordination_enabled()`'s gate is reachable
/// through the full router, not just in `tools.rs`'s own unit tests.
#[tokio::test]
async fn coordination_tools_are_disabled_by_default() {
    let (app, _pool) = test_router().await;
    let text = call_tool(app, "get_tab_status", serde_json::json!({"tab_id": "whatever"})).await;
    assert!(text.contains("disabled"), "{text}");
}

/// With coordination enabled, a `tab_id` this server never spawned (via
/// `spawn_tab`) must still be rejected — proves the router-level plumbing
/// reaches `CoordinationRegistry`'s rejection path, not just the
/// enabled/disabled gate.
#[tokio::test]
async fn coordination_tool_rejects_an_unknown_tab_id() {
    let (app, _pool) = test_router_with(true).await;
    let text = call_tool(app, "send_input", serde_json::json!({"tab_id": "nonexistent-tab-id", "text": "hi"})).await;
    assert!(text.contains("was not created by spawn_tab"), "{text}");
}

/// `router()`'s `app: Option<AppHandle>` is `None` in every test in this
/// file (no real Tauri app is available under `cargo test`) — confirms
/// `spawn_tab` returns a clear tool-level error instead of panicking when
/// that's the case, proving the `Option` handling added to unblock this
/// whole test file also works end to end through the router.
#[tokio::test]
async fn spawn_tab_without_an_app_handle_returns_a_clear_error() {
    let (app, _pool) = test_router_with(true).await;
    let text = call_tool(app, "spawn_tab", serde_json::json!({})).await;
    assert!(text.contains("requires a running AITerm app instance"), "{text}");
}

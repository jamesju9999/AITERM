//! axum router for the MCP tool server: mounts the `rmcp` Streamable HTTP
//! service at `/mcp` behind bearer-token auth, plus an unauthenticated
//! `/health`. Auth middleware shape follows the official `rmcp` example at
//! `https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/simple_auth_streamhttp.rs`
//! (public routes built separately from the token-gated ones, then merged) —
//! token comparison itself reuses `bridge::auth`, which is generic bearer-token
//! logic, not bridge-specific.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;
use tauri::AppHandle;

use crate::bridge::auth as bridge_auth;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::pty::manager::PtyManager;
use crate::secret::SecretStore;

use super::coordination_ops::CoordinationRegistry;
use super::tools::AiTermTools;

#[derive(Clone)]
struct AuthState {
    token: Arc<String>,
}

async fn auth_middleware(
    State(state): State<AuthState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    match bridge_auth::extract_token(authorization, None) {
        Some(provided) if bridge_auth::token_matches(&state.token, &provided) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn router(
    token: Arc<String>,
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    pty_manager: Arc<PtyManager>,
    app: AppHandle,
    coordination_registry: Arc<CoordinationRegistry>,
) -> Router {
    let service = StreamableHttpService::new(
        move || {
            Ok(AiTermTools::new(
                db_manager.clone(),
                config.clone(),
                secrets.clone(),
                sidecar.clone(),
                kb_pool.clone(),
                pty_manager.clone(),
                app.clone(),
                coordination_registry.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let protected = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(AuthState { token }, auth_middleware));

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(protected)
}

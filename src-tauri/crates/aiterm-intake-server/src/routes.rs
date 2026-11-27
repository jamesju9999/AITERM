//! `/v1` 路由：公開端點（hello、pair）與需要 token 的端點。
use crate::members::{self, Me, Role};
use crate::state::AppState;
use aiterm_intake_core::api::{ApiError, Hello, PROTOCOL};
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{header, request::Parts, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use std::sync::Arc;

pub fn api_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(ApiError { error: msg.to_string() })).into_response()
}

/// 需要 token。失敗一律 401 且不說明原因（不透露 token 是否存在或已撤銷）。
async fn require_member(State(st): State<Arc<AppState>>, mut req: Request, next: Next) -> Response {
    let token = req.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    match members::verify_token(&st.pools.server, token).await {
        Some(me) => {
            req.extensions_mut().insert(me);
            next.run(req).await
        }
        None => api_error(StatusCode::UNAUTHORIZED, "unauthorized"),
    }
}

/// 只有管理員過得了的 extractor；成員得 403。
pub struct Admin(pub Me);

impl<S: Send + Sync> FromRequestParts<S> for Admin {
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<Me>() {
            Some(me) if me.role == Role::Admin => Ok(Admin(me.clone())),
            Some(_) => Err(api_error(StatusCode::FORBIDDEN, "admin only")),
            None => Err(api_error(StatusCode::UNAUTHORIZED, "unauthorized")),
        }
    }
}

async fn hello(State(st): State<Arc<AppState>>) -> Json<Hello> {
    Json(Hello { server_version: st.version.clone(), protocol: PROTOCOL })
}

async fn me(Extension(me): Extension<Me>) -> Json<Me> {
    Json(me)
}

/// 已登入的路由（各功能模組在這裡掛上去）。
fn protected() -> Router<Arc<AppState>> {
    Router::new().route("/v1/me", get(me)).route("/v1/events", get(crate::events::events)).merge(crate::routes_data::routes()).merge(crate::routes_board::routes()).merge(crate::routes_verify::routes()).merge(crate::routes_skills::routes()).merge(crate::routes_db::routes()).merge(crate::routes_sys::routes())
}

pub fn build(state: Arc<AppState>) -> Router {
    let guarded = protected().layer(middleware::from_fn_with_state(state.clone(), require_member));
    Router::new().route("/v1/hello", get(hello)).route("/v1/pair", post(crate::pairing::pair)).merge(guarded).with_state(state)
}

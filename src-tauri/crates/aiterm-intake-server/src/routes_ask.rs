//! 中途問人的端點：待回答清單（成員可讀）、回答（任何成員）、取消（管理員）、通知設定（管理員）。
use crate::members::Me;
use crate::routes::Admin;
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use aiterm_intake_core::board_questions as bq;
use aiterm_intake_core::host::Notifier;
use aiterm_intake_core::store;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

type St = State<Arc<AppState>>;

pub const NOTIFY_EMAIL_KEY: &str = "ask_notify_email";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/board/projects/{pid}/questions", get(list))
        .route("/v1/board/questions/{id}/answer", post(answer))
        .route("/v1/board/questions/{id}/cancel", post(cancel))
        .route("/v1/ask/settings", get(get_settings).put(put_settings))
}

async fn list(State(st): St, Path(pid): Path<String>) -> Response {
    let open = bq::open_questions(&st.pools.intake, &pid).await;
    let closed = bq::recent_closed(&st.pools.intake, &pid, 20).await;
    match (open, closed) {
        (Ok(open), Ok(closed)) => Json(json!({"open": open, "closed": closed})).into_response(),
        (Err(e), _) | (_, Err(e)) => internal(e),
    }
}

#[derive(Deserialize)]
struct AnswerBody {
    answer: String,
}

async fn answer(State(st): St, Extension(me): Extension<Me>, Path(id): Path<i64>, Json(b): Json<AnswerBody>) -> Response {
    let text = b.answer.trim();
    if text.is_empty() {
        return bad("回答不能是空的");
    }
    if text.chars().count() > bq::MAX_QUESTION_CHARS {
        return bad("回答太長");
    }
    let Ok(Some(q)) = bq::get(&st.pools.intake, id).await else { return not_found() };
    match bq::answer(&st.pools.intake, id, text, &me.name, "app").await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "board.question.answer", &q.label()).await;
            st.events.changed();
            let row = bq::get(&st.pools.intake, id).await.ok().flatten();
            if let Some(row) = &row {
                crate::asknotify::closed_one(&st, row).await;
            }
            Json(row).into_response()
        }
        Ok(false) => conflict("這個問題已經有人回答了，或已被取消"),
        Err(e) => internal(e),
    }
}

async fn cancel(State(st): St, Admin(me): Admin, Path(id): Path<i64>) -> Response {
    let Ok(Some(q)) = bq::get(&st.pools.intake, id).await else { return not_found() };
    match bq::cancel(&st.pools.intake, id, &me.name).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "board.question.cancel", &q.label()).await;
            st.events.changed();
            let row = bq::get(&st.pools.intake, id).await.ok().flatten();
            if let Some(row) = &row {
                crate::asknotify::closed_one(&st, row).await;
            }
            Json(row).into_response()
        }
        Ok(false) => conflict("這個問題已經有人回答了，或已被取消"),
        Err(e) => internal(e),
    }
}

async fn get_settings(State(st): St, Admin(_): Admin) -> Response {
    let email = store::get_setting(&st.pools.intake, NOTIFY_EMAIL_KEY).await.ok().flatten().unwrap_or_default();
    Json(json!({"notify_email": email})).into_response()
}

#[derive(Deserialize)]
struct SettingsBody {
    notify_email: String,
}

async fn put_settings(State(st): St, Admin(me): Admin, Json(b): Json<SettingsBody>) -> Response {
    let email = b.notify_email.trim();
    if !email.is_empty() && !(email.contains('@') && !email.contains(char::is_whitespace)) {
        return bad("Email 格式不對");
    }
    match store::set_setting(&st.pools.intake, NOTIFY_EMAIL_KEY, email).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, "ask.settings", "notify_email").await;
            Json(json!({"notify_email": email})).into_response()
        }
        Err(e) => internal(e),
    }
}

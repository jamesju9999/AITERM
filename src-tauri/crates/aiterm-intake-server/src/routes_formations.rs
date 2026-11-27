//! 團隊編制的端點：讀取（成員）、建立／另存／改名／刪除／切換（管理員）。規則在 core 的 `board_formations`。

use crate::routes::Admin;
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use aiterm_intake_core::board_formations::{self as bf, FormationAgent, SwitchOutcome};
use aiterm_intake_core::host::Notifier;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

type St = State<Arc<AppState>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/board/projects/{pid}/formations", get(state).post(create))
        .route("/v1/board/projects/{pid}/formations/save-as", post(save_as))
        .route("/v1/board/projects/{pid}/formations/cancel-switch", post(cancel_switch))
        .route("/v1/board/projects/{pid}/formations/retired", get(retired))
        .route("/v1/board/projects/{pid}/formations/{id}", put(rename).delete(remove))
        .route("/v1/board/projects/{pid}/formations/{id}/switch", post(switch))
        .route("/v1/board/projects/{pid}/formations/{id}/request-switch", post(request_switch))
}

async fn project_exists(st: &AppState, pid: &str) -> bool {
    crate::routes_data::project_list(st).await.iter().any(|p| p.id == pid)
}

#[derive(Deserialize)]
struct StateQuery {
    #[serde(default)]
    default_name: String,
}

/// 舊專案（桌布上有 Agent、還沒有任何編制）第一次讀取時先用目前的團隊建立 `default_name`。
async fn state(State(st): St, Path(pid): Path<String>, Query(q): Query<StateQuery>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    if let Err(e) = bf::ensure_default(&st.pools.intake, &pid, &q.default_name).await {
        return internal(e);
    }
    match bf::state(&st.pools.intake, &pid).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => internal(e),
    }
}

async fn retired(State(st): St, Path(pid): Path<String>) -> Response {
    match bf::retired(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    #[serde(default)]
    agents: Vec<FormationAgent>,
}

async fn create(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<CreateBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    match bf::create(&st.pools.intake, &pid, &b.name, &b.agents, &me.name).await {
        Ok(Ok(id)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.formation.create", &id).await;
            st.events.changed();
            Json(json!({"id": id})).into_response()
        }
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn save_as(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<NameBody>) -> Response {
    match bf::save_as(&st.pools.intake, &pid, &b.name, &me.name).await {
        Ok(Ok(id)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.formation.save_as", &id).await;
            st.events.changed();
            Json(json!({"id": id})).into_response()
        }
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

async fn rename(State(st): St, Admin(me): Admin, Path((pid, id)): Path<(String, String)>, Json(b): Json<NameBody>) -> Response {
    match bf::rename(&st.pools.intake, &pid, &id, &b.name).await {
        Ok(Ok(())) => {
            crate::audit::record(&st.pools.server, &me.name, "board.formation.rename", &id).await;
            st.events.changed();
            Json(json!({})).into_response()
        }
        Ok(Err(why)) if why.contains("找不到") => not_found(),
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

async fn remove(State(st): St, Admin(me): Admin, Path((pid, id)): Path<(String, String)>) -> Response {
    match bf::delete(&st.pools.intake, &pid, &id).await {
        Ok(Ok(())) => {
            crate::audit::record(&st.pools.server, &me.name, "board.formation.delete", &id).await;
            st.events.changed();
            Json(json!({})).into_response()
        }
        Ok(Err(why)) => conflict(&why),
        Err(e) => internal(e),
    }
}

/// 切換編制。有人在工作時回 200 與 `{"status":"busy","names":[…]}`，由桌面版決定要不要改成「做完再換」。
async fn switch(State(st): St, Admin(me): Admin, Path((pid, id)): Path<(String, String)>) -> Response {
    match bf::switch_to(&st.pools.intake, &pid, &id, &me.name).await {
        Ok(Ok(outcome)) => {
            if outcome == SwitchOutcome::Switched {
                crate::audit::record(&st.pools.server, &me.name, "board.formation.switch", &id).await;
                st.events.changed();
            }
            Json(outcome).into_response()
        }
        Ok(Err(why)) if why.contains("找不到") => not_found(),
        Ok(Err(why)) => conflict(&why),
        Err(e) => internal(e),
    }
}

async fn request_switch(State(st): St, Admin(me): Admin, Path((pid, id)): Path<(String, String)>) -> Response {
    match bf::request_switch(&st.pools.intake, &pid, &id).await {
        Ok(Ok(())) => {
            crate::audit::record(&st.pools.server, &me.name, "board.formation.request_switch", &id).await;
            st.events.changed();
            Json(json!({})).into_response()
        }
        Ok(Err(why)) => not_found_or_bad(&why),
        Err(e) => internal(e),
    }
}

fn not_found_or_bad(why: &str) -> Response {
    if why.contains("找不到") {
        not_found()
    } else {
        bad(why)
    }
}

async fn cancel_switch(State(st): St, Admin(_): Admin, Path(pid): Path<String>) -> Response {
    match bf::cancel_switch(&st.pools.intake, &pid).await {
        Ok(()) => {
            st.events.changed();
            Json(json!({})).into_response()
        }
        Err(e) => internal(e),
    }
}


//! 資料庫技能的端點：連線管理（管理員）、設定 Agent 的資料庫技能（管理員）、查詢記錄與待核准清單（成員可讀）、核准／拒絕（管理員）。
use crate::dbconn::{self, ConnInput};
use crate::members::{Me, Role};
use crate::routes::Admin;
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use aiterm_db::skill::DbSkill;
use aiterm_intake_core::board_store as bs;
use aiterm_intake_core::host::Notifier;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

type St = State<Arc<AppState>>;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/db/connections", get(list).post(upsert))
        .route("/v1/db/connections/{id}", delete(remove))
        .route("/v1/db/connections/{id}/test", post(test))
        .route("/v1/board/agents/{id}/db-skill", put(set_agent_db_skill))
        .route("/v1/board/projects/{pid}/db/queries", get(queries))
        .route("/v1/board/projects/{pid}/db/approvals", get(approvals))
        .route("/v1/board/db/approvals/{id}/decide", post(decide))
}

/// 成員只看得到能選哪個連線所需的資訊；主機、帳號只給管理員。
async fn list(State(st): St, Extension(me): Extension<Me>) -> Response {
    match bs::list_db_conns(&st.pools.intake).await {
        Ok(v) if me.role == Role::Admin => Json(v).into_response(),
        Ok(v) => Json(v.into_iter().map(|c| json!({"id": c.id, "name": c.name, "db_type": c.db_type, "database": c.database})).collect::<Vec<_>>()).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct ConnBody {
    name: String,
    db_type: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: i64,
    #[serde(default)]
    database: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    default_schema: Option<String>,
    /// 不給＝沿用已存的密碼（同名取代時）。
    #[serde(default)]
    password: Option<String>,
}

async fn upsert(State(st): St, Admin(me): Admin, Json(b): Json<ConnBody>) -> Response {
    let input = ConnInput { name: &b.name, db_type: &b.db_type, host: &b.host, port: b.port, database: &b.database, username: &b.username, default_schema: b.default_schema.as_deref(), password: b.password.as_deref() };
    match dbconn::upsert(&st, &input, &me.name).await {
        Ok(conn) => {
            crate::audit::record(&st.pools.server, &me.name, "db.connection.upsert", &conn.name).await;
            st.events.changed();
            Json(conn).into_response()
        }
        Err(why) => bad(&why),
    }
}

async fn remove(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let name = bs::get_db_conn(&st.pools.intake, &id).await.ok().flatten().map(|c| c.name).unwrap_or_default();
    match dbconn::remove(&st, &id).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "db.connection.delete", &name).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}

async fn test(State(st): St, Admin(_me): Admin, Path(id): Path<String>) -> Response {
    if bs::get_db_conn(&st.pools.intake, &id).await.ok().flatten().is_none() {
        return not_found();
    }
    match dbconn::test(&st, &id).await {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(why) => bad(&why),
    }
}

#[derive(Deserialize)]
struct SkillBody {
    /// `null`＝移除。
    skill: Option<DbSkill>,
}

async fn set_agent_db_skill(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<SkillBody>) -> Response {
    // 沒選任何連線等於移除（同桌面版）。
    let skill = b.skill.filter(|s| !s.connections.is_empty());
    let (raw, conns) = match &skill {
        Some(s) => (Some(serde_json::to_string(s).unwrap_or_default()), s.connections.clone()),
        None => (None, Vec::new()),
    };
    match bs::set_agent_db_skill(&st.pools.intake, &id, raw.as_deref(), &conns).await {
        Ok(Ok(agent)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.db_skill", &id).await;
            st.events.changed();
            Json(agent).into_response()
        }
        Ok(Err(why)) if why.contains("找不到這個 Agent") => not_found(),
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct Limit {
    #[serde(default)]
    limit: Option<i64>,
}

async fn queries(State(st): St, Path(pid): Path<String>, Query(q): Query<Limit>) -> Response {
    match bs::recent_db_queries(&st.pools.intake, &pid, q.limit.unwrap_or(100)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

async fn approvals(State(st): St, Path(pid): Path<String>) -> Response {
    match bs::pending_db_queries(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct Decide {
    approve: bool,
}

async fn decide(State(st): St, Admin(me): Admin, Path(id): Path<i64>, Json(b): Json<Decide>) -> Response {
    let Ok(Some(q)) = bs::get_db_query(&st.pools.intake, id).await else { return not_found() };
    match bs::decide_db_query(&st.pools.intake, id, b.approve, &me.name).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, if b.approve { "db.approval.approve" } else { "db.approval.deny" }, &format!("{id} {}", q.connection_name)).await;
            st.events.changed();
            Json(bs::get_db_query(&st.pools.intake, id).await.ok().flatten()).into_response()
        }
        Ok(false) => conflict("這筆已經決定過了，或不是等核准的查詢"),
        Err(e) => internal(e),
    }
}

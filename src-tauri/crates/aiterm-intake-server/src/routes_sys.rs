//! 系統連線的端點：設定與登入 session（管理員）、Agent 勾選（管理員）、待核准與審計（成員可讀、管理員決定）。
use crate::members::{Me, Role};
use crate::routes::Admin;
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use crate::sysconf::{self, SystemInput};
use aiterm_intake_core::board_store as bs;
use aiterm_intake_core::host::Notifier;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

type St = State<Arc<AppState>>;

/// session 的請求主體上限（cookie 本身另有 1 MB 上限，這裡留餘裕給 JSON 外殼）。
const SESSION_BODY_LIMIT: usize = 4 * 1024 * 1024;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/systems", get(list).post(upsert))
        .route("/v1/systems/{id}", delete(remove))
        .route("/v1/systems/{id}/session", put(put_session).layer(DefaultBodyLimit::max(SESSION_BODY_LIMIT)).delete(delete_session))
        .route("/v1/systems/{id}/audit", get(audit))
        .route("/v1/board/agents/{id}/systems", put(set_agent_systems))
        .route("/v1/board/projects/{pid}/sys/approvals", get(approvals))
        .route("/v1/board/sys/approvals/{id}/decide", post(decide))
}

/// 成員看得到有哪些系統、誰分享了 session；白名單網域只給管理員。cookie 本身永遠不回傳。
async fn list(State(st): St, Extension(me): Extension<Me>) -> Response {
    let systems = match bs::list_systems(&st.pools.intake).await {
        Ok(v) => v,
        Err(e) => return internal(e),
    };
    let sessions: HashMap<String, bs::SystemSessionMeta> = bs::all_system_sessions(&st.pools.intake).await.unwrap_or_default().into_iter().map(|s| (s.system_id.clone(), s)).collect();
    let out: Vec<Value> = systems
        .into_iter()
        .map(|s| {
            let sess = sessions.get(&s.id).map(|m| json!({"exported_by": m.exported_by, "exported_at": m.exported_at, "cookie_count": m.cookie_count}));
            let mut v = json!({"id": s.id, "name": s.name, "start_url": s.start_url, "mode": s.mode, "enabled": s.enabled, "session": sess});
            if me.role == Role::Admin {
                v["allowed_hosts"] = json!(s.hosts());
            }
            v
        })
        .collect();
    Json(out).into_response()
}

#[derive(Deserialize)]
struct SystemBody {
    name: String,
    start_url: String,
    #[serde(default)]
    allowed_hosts: Vec<String>,
    #[serde(default = "read_only")]
    mode: String,
    #[serde(default = "yes")]
    enabled: bool,
}

fn read_only() -> String {
    "read_only".into()
}

fn yes() -> bool {
    true
}

async fn upsert(State(st): St, Admin(me): Admin, Json(b): Json<SystemBody>) -> Response {
    let input = SystemInput { name: &b.name, start_url: &b.start_url, allowed_hosts: &b.allowed_hosts, mode: &b.mode, enabled: b.enabled };
    match sysconf::upsert(&st, &input, &me.name).await {
        Ok(s) => {
            // 停用或改了設定：正在跑的瀏覽器下次呼叫會讀到新設定；停用的立刻關掉。
            if !s.enabled {
                crate::syscore::close_system(&st, &s.id).await;
            }
            crate::audit::record(&st.pools.server, &me.name, "system.upsert", &s.name).await;
            st.events.changed();
            Json(s).into_response()
        }
        Err(why) => bad(&why),
    }
}

async fn remove(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let name = bs::get_system(&st.pools.intake, &id).await.ok().flatten().map(|s| s.name).unwrap_or_default();
    match sysconf::remove(&st, &id).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "system.delete", &name).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct SessionBody {
    /// 桌面版從 Chrome 取出的 cookie（CDP `Network.getAllCookies` 的格式）。
    cookies: Vec<Value>,
}

async fn put_session(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<SessionBody>) -> Response {
    let Ok(Some(system)) = bs::get_system(&st.pools.intake, &id).await else { return not_found() };
    match sysconf::store_session(&st, &system, &b.cookies, &me.name).await {
        Ok(n) => {
            crate::audit::record(&st.pools.server, &me.name, "system.session.put", &system.name).await;
            st.events.changed();
            Json(json!({"cookie_count": n})).into_response()
        }
        Err(why) => bad(&why),
    }
}

async fn delete_session(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let name = bs::get_system(&st.pools.intake, &id).await.ok().flatten().map(|s| s.name).unwrap_or_default();
    match sysconf::remove_session(&st, &id).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "system.session.delete", &name).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct Limit {
    #[serde(default)]
    limit: Option<i64>,
}

async fn audit(State(st): St, Path(id): Path<String>, Query(q): Query<Limit>) -> Response {
    match aiterm_systems::audit::list(&st.pools.intake, &id, q.limit.unwrap_or(100).clamp(1, 500)).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct IdsBody {
    ids: Vec<String>,
}

async fn set_agent_systems(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<IdsBody>) -> Response {
    match bs::set_agent_systems(&st.pools.intake, &id, &b.ids).await {
        Ok(Ok(agent)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.systems", &id).await;
            st.events.changed();
            Json(agent).into_response()
        }
        Ok(Err(why)) if why.contains("找不到這個 Agent") => not_found(),
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

async fn approvals(State(st): St, Path(pid): Path<String>) -> Response {
    let names: HashMap<String, String> = bs::list_systems(&st.pools.intake).await.unwrap_or_default().into_iter().map(|s| (s.id, s.name)).collect();
    Json(st.sys.pending_for(&pid, &names)).into_response()
}

#[derive(Deserialize)]
struct Decide {
    approve: bool,
}

async fn decide(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<Decide>) -> Response {
    let Some(owner) = st.sys.owner_of(&id) else { return not_found() };
    if st.sys.approvals.decide(&id, b.approve) {
        crate::audit::record(&st.pools.server, &me.name, if b.approve { "system.approval.approve" } else { "system.approval.deny" }, &format!("{id} {}", owner.card_id)).await;
        st.events.changed();
        Json(json!({"ok": true})).into_response()
    } else {
        conflict("這筆已經決定過了或已逾時")
    }
}

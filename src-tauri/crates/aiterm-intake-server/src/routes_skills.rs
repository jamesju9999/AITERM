//! 技能庫的端點：清單（所有成員可讀）、上傳與刪除（管理員）、設定 Agent 的技能（管理員）。
use crate::routes::Admin;
use crate::routes_data::{bad, internal, not_found};
use crate::skills;
use crate::state::AppState;
use aiterm_intake_core::board_store as bs;
use aiterm_intake_core::host::Notifier;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;

type St = State<Arc<AppState>>;

/// 上傳主體上限：10 MB 的內容 base64 之後約 13.4 MB，留一點餘裕。
const UPLOAD_BODY_LIMIT: usize = 16 * 1024 * 1024;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/skills", get(list).post(upload).layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT)))
        .route("/v1/skills/{id}", delete(remove))
        .route("/v1/board/agents/{id}/skills", put(set_agent_skills))
}

async fn list(State(st): St) -> Response {
    match bs::list_skills(&st.pools.intake).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct UploadFile {
    path: String,
    /// base64 的檔案內容。
    data: String,
}

#[derive(Deserialize)]
struct UploadBody {
    /// 本機資料夾名稱（SKILL.md 沒有 name 時拿來當技能名稱）。
    #[serde(default)]
    folder_name: String,
    files: Vec<UploadFile>,
}

async fn upload(State(st): St, Admin(me): Admin, Json(b): Json<UploadBody>) -> Response {
    let mut files = Vec::with_capacity(b.files.len());
    for f in b.files {
        match base64::engine::general_purpose::STANDARD.decode(f.data.as_bytes()) {
            Ok(bytes) => files.push((f.path, bytes)),
            Err(_) => return bad(&format!("檔案內容不是有效的 base64：{}", f.path)),
        }
    }
    match skills::store_upload(&st, &b.folder_name, files, &me.name).await {
        Ok(skill) => {
            crate::audit::record(&st.pools.server, &me.name, "skill.upload", &skill.name).await;
            st.events.changed();
            (StatusCode::OK, Json(skill)).into_response()
        }
        Err(why) => bad(&why),
    }
}

async fn remove(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let name = bs::get_skill(&st.pools.intake, &id).await.ok().flatten().map(|s| s.name).unwrap_or_default();
    match skills::remove(&st, &id).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "skill.delete", &name).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct SetSkills {
    ids: Vec<String>,
}

async fn set_agent_skills(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<SetSkills>) -> Response {
    match bs::set_agent_skills(&st.pools.intake, &id, &b.ids).await {
        Ok(Ok(agent)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.skills", &id).await;
            st.events.changed();
            Json(agent).into_response()
        }
        Ok(Err(why)) if why.contains("找不到") => not_found(),
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

//! 共用看板的端點：卡片、附件、留言與領取。資料邏輯在 core 的 `board_store`，這裡只做收請求、權限、稽核與通知。
use crate::members::{Me, Role};
use crate::routes::{api_error, Admin};
use crate::routes_data::{bad, conflict, internal, not_found};
use crate::state::AppState;
use aiterm_intake_core::attach::{self, Raw};
use aiterm_intake_core::board_store::{self as bs, AgentChange, AgentEdit, Change, Edit, NewAgent, NewCard};
use aiterm_intake_core::host::Notifier;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;

type St = State<Arc<AppState>>;

fn attach_root(st: &AppState) -> std::path::PathBuf {
    st.data_dir.join("board-attachments")
}

async fn project_exists(st: &AppState, pid: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects WHERE id = ?").bind(pid).fetch_one(&st.pools.server).await.unwrap_or(0) > 0
}

fn changed_response(st: &AppState, c: Change) -> Response {
    match c {
        Change::Ok(card) => {
            st.events.changed();
            Json(*card).into_response()
        }
        Change::Conflict => conflict("這張卡片已被別人修改，請重新載入後再試"),
        Change::NotFound => not_found(),
        Change::Refused(why) => api_error(StatusCode::UNPROCESSABLE_ENTITY, &why),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    archived: bool,
}

async fn list(State(st): St, Path(pid): Path<String>, Query(q): Query<ListQuery>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    match bs::list_cards(&st.pools.intake, &pid, q.archived).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct CardBody {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default = "yes")]
    parallel_ok: bool,
    #[serde(default)]
    interactive: bool,
    #[serde(default)]
    label: Option<String>,
}

fn yes() -> bool {
    true
}

async fn get_one(State(st): St, Path(id): Path<String>) -> Response {
    match bs::get_card(&st.pools.intake, &id).await {
        Ok(Some(c)) => Json(c).into_response(),
        Ok(None) => not_found(),
        Err(e) => internal(e),
    }
}

async fn create(State(st): St, Extension(me): Extension<Me>, Path(pid): Path<String>, Json(b): Json<CardBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    if b.title.trim().is_empty() {
        return bad("標題不能是空的");
    }
    match bs::create_card(&st.pools.intake, NewCard { project_id: &pid, title: &b.title, body: &b.body, parallel_ok: b.parallel_ok, interactive: b.interactive, label: b.label.as_deref(), created_by: &me.name }).await {
        Ok(card) => {
            crate::audit::record(&st.pools.server, &me.name, "board.create", &card.id).await;
            st.events.changed();
            (StatusCode::CREATED, Json(card)).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct UpdateBody {
    version: i64,
    #[serde(flatten)]
    card: CardBody,
}

async fn update(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<UpdateBody>) -> Response {
    let r = bs::update_card(&st.pools.intake, &id, b.version, Edit { title: &b.card.title, body: &b.card.body, parallel_ok: b.card.parallel_ok, interactive: b.card.interactive, label: b.card.label.as_deref() }).await;
    match r {
        Ok(c) => {
            if matches!(c, Change::Ok(_)) {
                crate::audit::record(&st.pools.server, &me.name, "board.update", &id).await;
            }
            changed_response(&st, c)
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct MoveBody {
    version: i64,
    to_status: String,
    #[serde(default)]
    after_id: Option<String>,
    #[serde(default)]
    before_id: Option<String>,
    /// AITerm 依畫面上的鄰居算好的排序值；有就直接用。
    #[serde(default)]
    sort_order: Option<f64>,
}

async fn move_card(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<MoveBody>) -> Response {
    match bs::move_card_at(&st.pools.intake, &id, b.version, &b.to_status, b.after_id.as_deref(), b.before_id.as_deref(), b.sort_order).await {
        Ok(c) => {
            if matches!(c, Change::Ok(_)) {
                crate::audit::record(&st.pools.server, &me.name, "board.move", &format!("{id} → {}", b.to_status)).await;
            }
            changed_response(&st, c)
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct ArchiveBody {
    archived: bool,
}

async fn archive(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<ArchiveBody>) -> Response {
    match bs::set_archived(&st.pools.intake, &id, b.archived).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, if b.archived { "board.archive" } else { "board.unarchive" }, &id).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => conflict(if b.archived { "只有已完成、還沒封存的卡片可以封存" } else { "這張卡片沒有被封存" }),
        Err(e) => internal(e),
    }
}

/// 把整欄已完成的卡片封存（管理員）。
async fn archive_done(State(st): St, Admin(me): Admin, Path(pid): Path<String>) -> Response {
    match bs::archive_all_done(&st.pools.intake, &pid).await {
        Ok(n) => {
            crate::audit::record(&st.pools.server, &me.name, "board.archive_done", &pid).await;
            st.events.changed();
            Json(n).into_response()
        }
        Err(e) => internal(e),
    }
}

/// 刪除：卡片建立者或管理員；處理中的不能刪。
async fn remove(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    let Ok(Some(card)) = bs::get_card(&st.pools.intake, &id).await else { return not_found() };
    if me.role != Role::Admin && card.created_by != me.name {
        return api_error(StatusCode::FORBIDDEN, "只有卡片的建立者或管理員可以刪除");
    }
    match bs::delete_card(&st.pools.intake, &id).await {
        Ok(Some(paths)) => {
            for p in paths {
                let _ = std::fs::remove_file(p);
            }
            let _ = std::fs::remove_dir(attach_root(&st).join(&id));
            crate::audit::record(&st.pools.server, &me.name, "board.delete", &id).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => conflict("處理中的卡片不能刪除"),
        Err(e) => internal(e),
    }
}

// ---- 附件與留言 ----

#[derive(Deserialize)]
struct UploadBody {
    name: String,
    data_b64: String,
}

async fn upload(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<UploadBody>) -> Response {
    if !matches!(bs::get_card(&st.pools.intake, &id).await, Ok(Some(_))) {
        return not_found();
    }
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b.data_b64.as_bytes()) else { return bad("檔案內容不是有效的 base64") };
    let existing = bs::list_attachments(&st.pools.intake, &id).await.map(|v| v.iter().map(|a| a.size as usize).sum::<usize>()).unwrap_or(0);
    if existing + bytes.len() > attach::MAX_TOTAL_BYTES {
        return bad("這張卡片的附件總量太大");
    }
    let (saved, skipped) = attach::save_all(&attach_root(&st), &id, vec![Raw { name: b.name, bytes }]);
    let Some(s) = saved.first() else { return bad(if skipped > 0 { "檔案太大、是空的，或附件數量已達上限" } else { "無法儲存附件" }) };
    match bs::add_attachment(&st.pools.intake, &id, s).await {
        Ok(a) => {
            crate::audit::record(&st.pools.server, &me.name, "board.attach", &id).await;
            st.events.changed();
            (StatusCode::CREATED, Json(a)).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn attachment_file(State(st): St, Path(aid): Path<String>) -> Response {
    let Ok(Some(a)) = bs::get_attachment(&st.pools.intake, &aid).await else { return not_found() };
    // 只讀 server 的看板附件目錄底下的檔案。
    let (Ok(root), Ok(path)) = (attach_root(&st).canonicalize(), std::path::Path::new(&a.path).canonicalize()) else { return not_found() };
    if !path.starts_with(&root) {
        return bad("附件路徑不在允許的目錄內");
    }
    match std::fs::read(&path) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(_) => not_found(),
    }
}

async fn remove_attachment(State(st): St, Extension(me): Extension<Me>, Path(aid): Path<String>) -> Response {
    match bs::remove_attachment(&st.pools.intake, &aid).await {
        Ok(Some((_, path))) => {
            let _ = std::fs::remove_file(path);
            crate::audit::record(&st.pools.server, &me.name, "board.detach", &aid).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => not_found(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct CommentBody {
    text: String,
}

async fn comments(State(st): St, Path(id): Path<String>) -> Response {
    match bs::list_comments(&st.pools.intake, &id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

async fn add_comment(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<CommentBody>) -> Response {
    if b.text.trim().is_empty() {
        return bad("留言不能是空的");
    }
    if !matches!(bs::get_card(&st.pools.intake, &id).await, Ok(Some(_))) {
        return not_found();
    }
    match bs::add_comment(&st.pools.intake, &id, &me.name, &b.text).await {
        Ok(c) => {
            st.events.changed();
            (StatusCode::CREATED, Json(c)).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- 領取 ----

/// 領這張卡片：只有「待派送」的領得到；已被領走回 409。
async fn claim(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    // 這個專案開了「由 server 執行」：有指派 Agent 的卡由 server 的編排處理，成員不能搶。
    if let Ok(Some(c)) = bs::get_card(&st.pools.intake, &id).await {
        if c.agent_id.is_some() && crate::orchestrator::enabled(&st.pools.intake, &c.project_id).await {
            return conflict("這張卡由 server 的編排執行，不能手動領取");
        }
    }
    match bs::claim_card(&st.pools.intake, &id, &me.name, crate::db::now()).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "board.claim", &id).await;
            st.events.changed();
            match bs::get_card(&st.pools.intake, &id).await {
                Ok(Some(c)) => Json(c).into_response(),
                _ => not_found(),
            }
        }
        Ok(false) => match bs::get_card(&st.pools.intake, &id).await {
            Ok(None) => not_found(),
            _ => conflict("這張卡片已被領取，或還不是待派送"),
        },
        Err(e) => internal(e),
    }
}

async fn heartbeat(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    match bs::renew_claim(&st.pools.intake, &id, &me.name, crate::db::now()).await {
        Ok(ok) => Json(ok).into_response(),
        Err(e) => internal(e),
    }
}

async fn release(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    let only_by = if me.role == Role::Admin { None } else { Some(me.name.as_str()) };
    match bs::release_claim(&st.pools.intake, &id, only_by, crate::db::now()).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "board.release", &id).await;
            st.events.changed();
            Json(true).into_response()
        }
        Ok(false) => conflict("這張卡片不是你領取的，或不在處理中"),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct ResultBody {
    /// `success`｜`failed`
    outcome: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// 領取者交結果。摘要與錯誤先去敏。
async fn result(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<ResultBody>) -> Response {
    if !matches!(b.outcome.as_str(), "success" | "failed") {
        return bad("outcome 必須是 success 或 failed");
    }
    let summary = b.summary.as_deref().map(aiterm_intake_core::filter::redact);
    let error = b.error.as_deref().map(aiterm_intake_core::filter::redact);
    match bs::finish_claim(&st.pools.intake, &id, &me.name, &b.outcome, summary.as_deref(), error.as_deref(), crate::db::now()).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "board.result", &id).await;
            st.events.changed();
            Json(true).into_response()
        }
        Ok(false) => conflict("這張卡片已不是你領取的（可能租約逾期被收回）"),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct SummaryBody {
    summary: String,
}

async fn summary(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<SummaryBody>) -> Response {
    let s = aiterm_intake_core::filter::redact(&b.summary);
    match bs::set_summary(&st.pools.intake, &id, &me.name, &s).await {
        Ok(true) => {
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => conflict("這張卡片不是你領取的，或不在處理中"),
        Err(e) => internal(e),
    }
}

// ---- Agent 協作：Agent、指派、記事本、簡介 ----

async fn list_agents(State(st): St, Path(pid): Path<String>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    match bs::list_agents(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct AgentBody {
    #[serde(default)]
    kind: String,
    name: String,
    #[serde(default)]
    role: String,
    #[serde(default)]
    prompt_prefix: String,
    #[serde(default)]
    canvas_x: f64,
    #[serde(default)]
    canvas_y: f64,
    #[serde(default)]
    produces_spec: bool,
    #[serde(default)]
    model: Option<String>,
    #[serde(default = "claude")]
    cli: String,
}

fn claude() -> String {
    "claude".into()
}

async fn create_agent(State(st): St, Extension(me): Extension<Me>, Path(pid): Path<String>, Json(b): Json<AgentBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    let r = bs::create_agent(&st.pools.intake, NewAgent { project_id: &pid, kind: &b.kind, name: &b.name, role: &b.role, prompt_prefix: &b.prompt_prefix, canvas_x: b.canvas_x, canvas_y: b.canvas_y, produces_spec: b.produces_spec, model: b.model.as_deref(), cli: &b.cli, created_by: &me.name }).await;
    match r {
        Ok(Ok(a)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.create", &a.id).await;
            st.events.changed();
            (StatusCode::CREATED, Json(a)).into_response()
        }
        Ok(Err(why)) => bad(&why),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct AgentUpdate {
    version: i64,
    #[serde(flatten)]
    agent: AgentBody,
}

async fn update_agent(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<AgentUpdate>) -> Response {
    let a = &b.agent;
    match bs::update_agent(&st.pools.intake, &id, b.version, AgentEdit { name: &a.name, role: &a.role, prompt_prefix: &a.prompt_prefix, produces_spec: a.produces_spec, model: a.model.as_deref(), cli: &a.cli }).await {
        Ok(Ok(agent)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.update", &id).await;
            st.events.changed();
            Json(agent).into_response()
        }
        Ok(Err(AgentChange::Conflict)) => conflict("這個 Agent 已被別人修改，請重新載入後再試"),
        Ok(Err(AgentChange::NotFound)) => not_found(),
        Ok(Err(AgentChange::Refused(why))) => api_error(StatusCode::UNPROCESSABLE_ENTITY, &why),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct PositionBody {
    x: f64,
    y: f64,
}

async fn agent_position(State(st): St, Path(id): Path<String>, Json(b): Json<PositionBody>) -> Response {
    match bs::set_agent_position(&st.pools.intake, &id, b.x, b.y).await {
        Ok(true) => {
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => bad("找不到這個 Agent，或位置不是有效的數字"),
        Err(e) => internal(e),
    }
}

/// 刪除 Agent：建立者或管理員；它還有處理中的卡片就不能刪。
async fn delete_agent(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    let Ok(Some(a)) = bs::get_agent(&st.pools.intake, &id).await else { return not_found() };
    if me.role != Role::Admin && a.created_by != me.name {
        return api_error(StatusCode::FORBIDDEN, "只有 Agent 的建立者或管理員可以刪除");
    }
    match bs::delete_agent(&st.pools.intake, &id).await {
        Ok(Ok(_)) => {
            crate::audit::record(&st.pools.server, &me.name, "board.agent.delete", &id).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Err(why)) => conflict(&why),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct AssignBody {
    version: i64,
    #[serde(default)]
    agent_id: Option<String>,
}

async fn assign(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<AssignBody>) -> Response {
    match bs::assign_card(&st.pools.intake, &id, b.version, b.agent_id.as_deref()).await {
        Ok(c) => {
            if matches!(c, Change::Ok(_)) {
                crate::audit::record(&st.pools.server, &me.name, "board.assign", &format!("{id} → {}", b.agent_id.as_deref().unwrap_or("-"))).await;
            }
            changed_response(&st, c)
        }
        Err(e) => internal(e),
    }
}

async fn notes(State(st): St, Path(pid): Path<String>) -> Response {
    match bs::list_notes(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct NoteBody {
    text: String,
    /// 署名（Agent 或空白）；成員自己加的用成員名稱。
    #[serde(default)]
    author: Option<String>,
}

async fn add_note(State(st): St, Extension(me): Extension<Me>, Path(pid): Path<String>, Json(b): Json<NoteBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    if b.text.trim().is_empty() {
        return bad("記事不能是空的");
    }
    let author = b.author.filter(|a| !a.trim().is_empty()).unwrap_or_else(|| me.name.clone());
    match bs::add_note(&st.pools.intake, &pid, &author, &b.text).await {
        Ok(n) => {
            st.events.changed();
            (StatusCode::CREATED, Json(n)).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn delete_note(State(st): St, Path(id): Path<i64>) -> Response {
    match bs::delete_note(&st.pools.intake, id).await {
        Ok(true) => {
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => not_found(),
        Err(e) => internal(e),
    }
}

fn brief_key(pid: &str) -> String {
    format!("board_brief:{pid}")
}

async fn get_brief(State(st): St, Path(pid): Path<String>) -> Response {
    let v = aiterm_intake_core::store::get_setting(&st.pools.intake, &brief_key(&pid)).await.ok().flatten();
    Json(v).into_response()
}

#[derive(Deserialize)]
struct BriefBody {
    brief: Option<String>,
}

/// 專案簡介（每張 Agent 卡派工時放在提示前面）。空白＝刪除。
async fn set_brief(State(st): St, Extension(me): Extension<Me>, Path(pid): Path<String>, Json(b): Json<BriefBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    let text = b.brief.unwrap_or_default();
    let r = if text.trim().is_empty() {
        sqlx::query("DELETE FROM intake_settings WHERE key = ?").bind(brief_key(&pid)).execute(&st.pools.intake).await.map(|_| ())
    } else {
        aiterm_intake_core::store::set_setting(&st.pools.intake, &brief_key(&pid), text.trim()).await
    };
    match r {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, "board.brief", &pid).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => internal(e),
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/board/projects/{pid}/cards", get(list).post(create))
        .route("/v1/board/projects/{pid}/archive-done", post(archive_done))
        .route("/v1/board/cards/{id}", get(get_one).put(update).delete(remove))
        .route("/v1/board/cards/{id}/move", post(move_card))
        .route("/v1/board/cards/{id}/archive", post(archive))
        .route("/v1/board/cards/{id}/attachments", post(upload))
        .route("/v1/board/cards/{id}/comments", get(comments).post(add_comment))
        .route("/v1/board/cards/{id}/claim", post(claim))
        .route("/v1/board/cards/{id}/heartbeat", post(heartbeat))
        .route("/v1/board/cards/{id}/release", post(release))
        .route("/v1/board/cards/{id}/result", post(result))
        .route("/v1/board/cards/{id}/summary", post(summary))
        .route("/v1/board/attachments/{aid}/file", get(attachment_file))
        .route("/v1/board/attachments/{aid}", delete(remove_attachment))
        .route("/v1/board/projects/{pid}/agents", get(list_agents).post(create_agent))
        .route("/v1/board/agents/{id}", put(update_agent).delete(delete_agent))
        .route("/v1/board/agents/{id}/position", post(agent_position))
        .route("/v1/board/cards/{id}/assign", post(assign))
        .route("/v1/board/projects/{pid}/notes", get(notes).post(add_note))
        .route("/v1/board/notes/{id}", delete(delete_note))
        .route("/v1/board/projects/{pid}/brief", get(get_brief).put(set_brief))
        .route("/v1/board/projects/{pid}/orchestration", get(get_orchestration).put(set_orchestration))
        .route("/v1/board/cards/{id}/merge", post(merge_card))
}

#[derive(serde::Serialize, Deserialize)]
struct OrchestrationBody {
    enabled: bool,
}

async fn get_orchestration(State(st): St, Path(pid): Path<String>) -> Response {
    Json(OrchestrationBody { enabled: crate::orchestrator::enabled(&st.pools.intake, &pid).await }).into_response()
}

/// 開關「由 server 執行」：管理員才能改。開之前要有專案資料夾，否則 server 不知道在哪裡做。
async fn set_orchestration(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<OrchestrationBody>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    if b.enabled {
        let wd: String = sqlx::query_scalar("SELECT work_dir FROM projects WHERE id = ?").bind(&pid).fetch_optional(&st.pools.server).await.ok().flatten().unwrap_or_default();
        if wd.trim().is_empty() {
            return bad("這個專案還沒設定工作資料夾（work_dir），無法由 server 執行");
        }
    }
    match crate::orchestrator::set_enabled(&st.pools.intake, &pid, b.enabled).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, if b.enabled { "board.orchestrate_on" } else { "board.orchestrate_off" }, &pid).await;
            st.events.changed();
            Json(b).into_response()
        }
        Err(e) => internal(e),
    }
}

/// 管理員核准合併：把各 Coder 的分支合併進專案資料夾。
async fn merge_card(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    let Ok(Some(card)) = bs::get_card(&st.pools.intake, &id).await else { return not_found() };
    match crate::orchestrator::merge_approved(&st, &card).await {
        Ok(summary) => {
            crate::audit::record(&st.pools.server, &me.name, "board.merge", &id).await;
            st.events.changed();
            Json(summary).into_response()
        }
        Err(e) => conflict(&e),
    }
}

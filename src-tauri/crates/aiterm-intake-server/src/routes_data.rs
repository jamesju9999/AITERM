//! 資料端點：專案、設定、白名單、問題清單與動作、附件、祕密、AI、稽核。
//! 處理邏輯盡量呼叫 core 的 `ops` 與 `store`（與 AITERM 的 Tauri 指令同一份），這裡只做收請求、權限與通知。
use crate::members::Me;
use crate::routes::{api_error, Admin};
use crate::state::AppState;
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::ops::{self, ConfigView, SenderDto};
use aiterm_intake_core::{attach, config, line_store, refs, store};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type St = State<Arc<AppState>>;

fn internal(e: impl ToString) -> Response {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

fn bad(msg: &str) -> Response {
    api_error(StatusCode::BAD_REQUEST, msg)
}

fn not_found() -> Response {
    api_error(StatusCode::NOT_FOUND, "not found")
}

// ---- 專案 ----

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub work_dir: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub work_dir: String,
    pub open_count: i64,
    pub total: i64,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct ProjectBody {
    pub name: String,
    #[serde(default)]
    pub work_dir: String,
}

pub async fn project_list(st: &AppState) -> Vec<ProjectRow> {
    sqlx::query_as("SELECT id, name, work_dir FROM projects ORDER BY name, id").fetch_all(&st.pools.server).await.unwrap_or_default()
}

async fn project_exists(st: &AppState, pid: &str) -> bool {
    project_list(st).await.iter().any(|p| p.id == pid)
}

async fn list_projects(State(st): St) -> Response {
    let counts = store::project_counts(&st.pools.intake).await.unwrap_or_default();
    let mut out = Vec::new();
    for p in project_list(&st).await {
        let (open_count, total) = counts.iter().find(|(id, _, _)| *id == p.id).map(|(_, o, t)| (*o, *t)).unwrap_or((0, 0));
        let enabled = config::load(&st.pools.intake, &p.id).await.enabled;
        out.push(ProjectSummary { id: p.id, name: p.name, work_dir: p.work_dir, open_count, total, enabled });
    }
    Json(out).into_response()
}

async fn create_project(State(st): St, Admin(me): Admin, Json(b): Json<ProjectBody>) -> Response {
    let name = b.name.trim();
    if name.is_empty() {
        return bad("專案名稱不能是空的");
    }
    let id = uuid::Uuid::new_v4().to_string();
    let r = sqlx::query("INSERT INTO projects(id, name, work_dir, created_at) VALUES (?,?,?,?)").bind(&id).bind(name).bind(b.work_dir.trim()).bind(crate::db::now()).execute(&st.pools.server).await;
    if let Err(e) = r {
        return internal(e);
    }
    crate::audit::record(&st.pools.server, &me.name, "project.create", &id).await;
    crate::channels::reload(&st).await;
    st.events.changed();
    (StatusCode::CREATED, Json(ProjectRow { id, name: name.to_string(), work_dir: b.work_dir.trim().to_string() })).into_response()
}

async fn update_project(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<ProjectBody>) -> Response {
    let name = b.name.trim();
    if name.is_empty() {
        return bad("專案名稱不能是空的");
    }
    match sqlx::query("UPDATE projects SET name = ?, work_dir = ? WHERE id = ?").bind(name).bind(b.work_dir.trim()).bind(&pid).execute(&st.pools.server).await {
        Ok(r) if r.rows_affected() == 1 => {
            crate::audit::record(&st.pools.server, &me.name, "project.update", &pid).await;
            crate::channels::reload(&st).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(_) => not_found(),
        Err(e) => internal(e),
    }
}

async fn delete_project(State(st): St, Admin(me): Admin, Path(pid): Path<String>) -> Response {
    match sqlx::query("DELETE FROM projects WHERE id = ?").bind(&pid).execute(&st.pools.server).await {
        Ok(r) if r.rows_affected() == 1 => {
            crate::audit::record(&st.pools.server, &me.name, "project.delete", &pid).await;
            crate::channels::reload(&st).await;
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(_) => not_found(),
        Err(e) => internal(e),
    }
}

// ---- 設定與白名單 ----

async fn get_config(State(st): St, Path(pid): Path<String>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    Json(ops::get_config(&st.pools.intake, &pid).await).into_response()
}

#[derive(Deserialize)]
pub struct SetConfig {
    pub config: config::Config,
    #[serde(default)]
    pub telegram_default: bool,
}

async fn set_config(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<SetConfig>) -> Response {
    if !project_exists(&st, &pid).await {
        return not_found();
    }
    let projects: Vec<(String, String)> = project_list(&st).await.into_iter().map(|p| (p.id, p.name)).collect();
    if let Err(e) = ops::set_config(&st.pools.intake, &pid, b.config, b.telegram_default, &projects).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "config.set", &pid).await;
    crate::channels::reload(&st).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

async fn list_senders(State(st): St, Path(pid): Path<String>) -> Response {
    match ops::list_senders(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

async fn upsert_sender(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(s): Json<SenderDto>) -> Response {
    let addr = s.address.clone();
    if let Err(e) = ops::upsert_sender(&st.pools.intake, &pid, s).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "sender.upsert", &addr).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
pub struct AddressBody {
    pub address: String,
    #[serde(default)]
    pub label: String,
}

async fn set_label(State(st): St, Admin(_): Admin, Path(pid): Path<String>, Json(b): Json<AddressBody>) -> Response {
    match store::set_sender_label(&st.pools.intake, &pid, &b.address, &b.label).await {
        Ok(()) => {
            st.events.changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => internal(e),
    }
}

async fn delete_sender(State(st): St, Admin(me): Admin, Path(pid): Path<String>, Json(b): Json<AddressBody>) -> Response {
    match store::delete_sender(&st.pools.intake, &pid, &b.address).await {
        Ok(r) => {
            crate::audit::record(&st.pools.server, &me.name, "sender.delete", &b.address).await;
            st.events.changed();
            Json(r).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn dropped(State(st): St, Path(pid): Path<String>) -> Response {
    match store::dropped_counts(&st.pools.intake, &pid).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct Paths {
    pub paths: Vec<String>,
}

/// 參考資料夾檢查：路徑是 server 檔案系統上的路徑，所以在 server 上檢查。
async fn check_folders(State(st): St, Path(pid): Path<String>, Json(b): Json<Paths>) -> Response {
    let base = project_list(&st).await.into_iter().find(|p| p.id == pid).map(|p| p.work_dir).filter(|d| !d.is_empty());
    let base = base.map(std::path::PathBuf::from);
    Json(b.paths.iter().map(|p| refs::check(p, base.as_deref())).collect::<Vec<_>>()).into_response()
}

async fn line_unknown(State(st): St) -> Response {
    match line_store::list_unknown(&st.pools.intake).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

async fn forget_unknown(State(st): St, Admin(_): Admin, Path(id): Path<String>) -> Response {
    match line_store::forget_unknown(&st.pools.intake, &id).await {
        Ok(r) => {
            st.events.changed();
            Json(r).into_response()
        }
        Err(e) => internal(e),
    }
}

// ---- 問題清單與動作 ----

#[derive(Deserialize)]
pub struct ListQuery {
    pub status: Option<String>,
}

async fn list_items(State(st): St, Path(pid): Path<String>, Query(q): Query<ListQuery>) -> Response {
    match store::list_items(&st.pools.intake, &pid, q.status.as_deref()).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

/// 題目以 id 定址（UUID）；所有成員看得到所有專案，所以不再要求網址帶專案。
async fn item_of(st: &AppState, id: &str) -> Option<store::IntakeItem> {
    store::get_item(&st.pools.intake, id).await.ok().flatten()
}

async fn get_item(State(st): St, Path(id): Path<String>) -> Response {
    match item_of(&st, &id).await {
        Some(i) => Json(i).into_response(),
        None => not_found(),
    }
}

async fn act(st: &AppState, me: &Me, id: &str, action: &str, r: impl std::future::Future<Output = Result<bool, sqlx::Error>>) -> Response {
    if item_of(st, id).await.is_none() {
        return not_found();
    }
    match r.await {
        Ok(done) => {
            crate::audit::record(&st.pools.server, &me.name, action, id).await;
            st.events.changed();
            Json(done).into_response()
        }
        Err(e) => internal(e),
    }
}

async fn approve(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    act(&st, &me, &id, "item.approve", store::approve(&st.pools.intake, &id)).await
}

async fn reject(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    act(&st, &me, &id, "item.reject", store::reject(&st.pools.intake, &id)).await
}

async fn delete_item(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    let r = act(&st, &me, &id, "item.delete", store::delete_item(&st.pools.intake, &id)).await;
    attach::remove_dir(&st.data_dir.join("attachments"), &id);
    r
}

#[derive(Deserialize)]
pub struct DraftBody {
    pub draft: String,
}

async fn update_draft(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<DraftBody>) -> Response {
    act(&st, &me, &id, "item.draft", store::update_draft(&st.pools.intake, &id, &b.draft)).await
}

async fn reply(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    if item_of(&st, &id).await.is_none() {
        return not_found();
    }
    match crate::reply::send(&st, &id).await {
        Ok(()) => {
            crate::audit::record(&st.pools.server, &me.name, "item.reply", &id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(msg) => {
            store::set_error(&st.pools.intake, &id, &format!("寄出失敗：{msg}")).await.ok();
            st.events.changed();
            bad(&msg)
        }
    }
}

// ---- 領取處理（成員用自己的電腦處理題目）----

fn conflict(msg: &str) -> Response {
    api_error(StatusCode::CONFLICT, msg)
}

/// 領這一題：只有「待派送」的題目領得到；已被領走、還沒核准都回 409。
async fn claim(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    if item_of(&st, &id).await.is_none() {
        return not_found();
    }
    match store::claim_item(&st.pools.intake, &id, &me.name, crate::db::now()).await {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, "item.claim", &id).await;
            st.events.changed();
            match item_of(&st, &id).await {
                Some(i) => Json(i).into_response(),
                None => not_found(),
            }
        }
        Ok(false) => conflict("這題已被領取，或還不能領取（要先核准、排進待派送）"),
        Err(e) => internal(e),
    }
}

/// 領這個專案最舊的待派送題目；沒有就回 `null`。
async fn claim_next(State(st): St, Extension(me): Extension<Me>, Path(pid): Path<String>) -> Response {
    match store::claim_next_for(&st.pools.intake, &pid, &me.name, crate::db::now()).await {
        Ok(Some(i)) => {
            crate::audit::record(&st.pools.server, &me.name, "item.claim", &i.id).await;
            st.events.changed();
            Json(Some(i)).into_response()
        }
        Ok(None) => Json(Option::<store::IntakeItem>::None).into_response(),
        Err(e) => internal(e),
    }
}

async fn heartbeat(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    match store::renew_claim(&st.pools.intake, &id, &me.name, crate::db::now()).await {
        Ok(ok) => Json(ok).into_response(),
        Err(e) => internal(e),
    }
}

/// 放回待派送：領取者本人，或管理員。
async fn release(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>) -> Response {
    let only_by = if me.role == crate::members::Role::Admin { None } else { Some(me.name.as_str()) };
    match store::release_claim(&st.pools.intake, &id, only_by, crate::db::now()).await {
        Ok(ok) => {
            if ok {
                crate::audit::record(&st.pools.server, &me.name, "item.release", &id).await;
                st.events.changed();
            }
            Json(ok).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
pub struct ResultBody {
    pub reply: Option<String>,
    pub error: Option<String>,
}

/// 領取者交結果：`reply` → 草稿（先去敏）；`error` → 回待核准並記原因。
async fn claim_result(State(st): St, Extension(me): Extension<Me>, Path(id): Path<String>, Json(b): Json<ResultBody>) -> Response {
    let now = crate::db::now();
    let (done, action) = match (b.reply, b.error) {
        (Some(reply), _) if !reply.trim().is_empty() => (store::finish_claim(&st.pools.intake, &id, &me.name, &aiterm_intake_core::filter::redact(&reply), now).await, "item.result"),
        (_, Some(error)) => (store::fail_claim(&st.pools.intake, &id, &me.name, &error, now).await, "item.fail"),
        _ => return bad("要帶 reply 或 error"),
    };
    match done {
        Ok(true) => {
            crate::audit::record(&st.pools.server, &me.name, action, &id).await;
            st.events.changed();
            Json(true).into_response()
        }
        Ok(false) => conflict("這題已不是你領取的（可能租約逾期被收回）"),
        Err(e) => internal(e),
    }
}

/// 附件原始內容：領取者要下載到自己的電腦。只讀 server 的附件目錄底下的檔案。
async fn attachment_file(State(st): St, Path(att): Path<String>) -> Response {
    let Some(row) = attachment_row(&st, &att).await else { return not_found() };
    let root = st.data_dir.join("attachments");
    let (Ok(root), Ok(path)) = (root.canonicalize(), std::path::Path::new(&row.path).canonicalize()) else { return not_found() };
    if !path.starts_with(&root) {
        return bad("附件路徑不在允許的目錄內");
    }
    match std::fs::read(&path) {
        Ok(bytes) => ([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Err(_) => not_found(),
    }
}

// ---- 附件 ----

async fn attachments(State(st): St, Path(id): Path<String>) -> Response {
    if item_of(&st, &id).await.is_none() {
        return not_found();
    }
    match store::list_attachments(&st.pools.intake, &id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => internal(e),
    }
}

async fn attachment_row(st: &AppState, att: &str) -> Option<store::AttachmentRow> {
    store::get_attachment(&st.pools.intake, att).await.ok().flatten()
}

async fn attachment_data(State(st): St, Path(att): Path<String>) -> Response {
    let Some(row) = attachment_row(&st, &att).await else { return not_found() };
    match attach::data_url(&st.data_dir.join("attachments"), &row) {
        Ok(u) => Json(u).into_response(),
        Err(e) => bad(&e),
    }
}

async fn attachment_text(State(st): St, Path(att): Path<String>) -> Response {
    let Some(row) = attachment_row(&st, &att).await else { return not_found() };
    match attach::text_preview(&st.data_dir.join("attachments"), &row) {
        Ok(t) => Json(t).into_response(),
        Err(e) => bad(&e),
    }
}

// ---- 祕密、AI、狀態、稽核 ----

/// 可以經由 API 設定的祕密（白名單，避免任意寫入）。
pub const SETTABLE_SECRETS: [&str; 5] = [aiterm_intake_core::line_server::LINE_SECRET_ID, aiterm_intake_core::line_server::LINE_TOKEN_ID, crate::ai::API_KEY_SECRET, crate::mail::PASSWORD_SECRET, crate::telegram::TOKEN_SECRET];

#[derive(Deserialize)]
pub struct SecretBody {
    pub value: String,
}

async fn set_secret(State(st): St, Admin(me): Admin, Path(id): Path<String>, Json(b): Json<SecretBody>) -> Response {
    if !SETTABLE_SECRETS.contains(&id.as_str()) {
        return bad("不認得這個祕密");
    }
    if b.value.trim().is_empty() {
        return bad("內容不能是空的");
    }
    if let Err(e) = st.secrets.set(&id, b.value.trim()) {
        return internal(e);
    }
    crate::audit::record(&st.pools.server, &me.name, "secret.set", &id).await;
    crate::channels::reload(&st).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

async fn clear_secret(State(st): St, Admin(me): Admin, Path(id): Path<String>) -> Response {
    if !SETTABLE_SECRETS.contains(&id.as_str()) {
        return bad("不認得這個祕密");
    }
    if let Err(e) = st.secrets.delete(&id) {
        return internal(e);
    }
    crate::audit::record(&st.pools.server, &me.name, "secret.clear", &id).await;
    crate::channels::reload(&st).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AiView {
    pub settings: crate::ai::AiSettings,
    pub configured: bool,
    /// 最近一次實際呼叫的結果（重啟後清空）。
    #[serde(default)]
    pub health: crate::ai::Health,
    /// 訂閱登入狀態：`none`｜`ok`｜`needs_login`（金鑰型供應商一律 `none`）。
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub login_error: Option<String>,
}

async fn get_ai(State(st): St) -> Response {
    let settings = crate::ai::load(&st.pools.intake).await;
    let configured = crate::ai::configured(&settings, st.secrets.as_ref());
    let (login, login_error) = if crate::ai::is_oauth(&settings.provider) { st.ai.login_state() } else { ("none", None) };
    Json(AiView { settings, configured, health: st.ai.health(), login: login.to_string(), login_error }).into_response()
}

/// 模型清單與測試連線共用的輸入：畫面上還沒儲存的值也能先測；`api_key` 空白就用已存的。
#[derive(Deserialize)]
pub struct AiProbe {
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
}

impl AiProbe {
    fn resolve(self, st: &AppState) -> (crate::ai::AiSettings, String) {
        let key = if self.api_key.trim().is_empty() { st.secrets.get(crate::ai::API_KEY_SECRET).unwrap_or_default() } else { self.api_key.trim().to_string() };
        (crate::ai::AiSettings { provider: self.provider, model: self.model, base_url: self.base_url }, key)
    }
}

async fn ai_models(State(st): St, Admin(_): Admin, Json(p): Json<AiProbe>) -> Response {
    let (settings, key) = p.resolve(&st);
    match st.ai.list_models(&settings, &key).await {
        Ok(models) => Json(models).into_response(),
        Err(e) => bad(&e),
    }
}

async fn ai_test(State(st): St, Admin(me): Admin, Json(p): Json<AiProbe>) -> Response {
    let (settings, key) = p.resolve(&st);
    let result = st.ai.test(&settings, &key).await;
    crate::audit::record(&st.pools.server, &me.name, "ai.test", if result.ok { "ok" } else { "failed" }).await;
    Json(result).into_response()
}

#[derive(Deserialize)]
pub struct OauthLogin {
    pub provider: String,
    pub access: String,
    pub refresh: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

/// 管理員的 AITerm 完成瀏覽器登入後，把憑證交給 server 存放與續期。
async fn put_oauth(State(st): St, Admin(me): Admin, Json(b): Json<OauthLogin>) -> Response {
    if !crate::ai::is_oauth(&b.provider) {
        return bad("這個供應商不用訂閱登入");
    }
    if let Err(e) = st.ai.store_login(&b.access, &b.refresh, b.expires_in) {
        return bad(&e);
    }
    // 切到這個供應商；模型與位址沿用已存的（沒有就留空，由畫面補）。
    let mut settings = crate::ai::load(&st.pools.intake).await;
    settings.provider = b.provider.clone();
    if let Err(e) = crate::ai::save(&st.pools.intake, &settings).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "ai.login", &b.provider).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

async fn delete_oauth(State(st): St, Admin(me): Admin) -> Response {
    st.ai.clear_login();
    crate::audit::record(&st.pools.server, &me.name, "ai.logout", "").await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

async fn set_ai(State(st): St, Admin(me): Admin, Json(s): Json<crate::ai::AiSettings>) -> Response {
    if let Err(e) = crate::ai::save(&st.pools.intake, &s).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "ai.set", &s.provider).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TelegramView {
    pub settings: crate::telegram::TelegramSettings,
    /// 已設定機器人金鑰。
    pub has_token: bool,
    pub listening: bool,
    pub error: Option<String>,
}

async fn telegram_view(st: &AppState) -> TelegramView {
    let health = st.channels.telegram_health.lock().clone();
    let has_token = st.secrets.get(crate::telegram::TOKEN_SECRET).is_some();
    TelegramView { settings: crate::telegram::load(&st.pools.intake).await, has_token, listening: has_token && health.last_ok.is_some() && health.error.is_none(), error: health.error }
}

async fn get_telegram(State(st): St) -> Response {
    Json(telegram_view(&st).await).into_response()
}

async fn set_telegram(State(st): St, Admin(me): Admin, Json(s): Json<crate::telegram::TelegramSettings>) -> Response {
    if let Err(e) = crate::telegram::save(&st.pools.intake, &s).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "telegram.set", "").await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MailView {
    pub settings: crate::mail::MailSettings,
    pub configured: bool,
}

async fn get_mail(State(st): St) -> Response {
    let settings = crate::mail::load(&st.pools.intake).await;
    let configured = crate::mail::configured(&settings, st.secrets.as_ref());
    Json(MailView { settings, configured }).into_response()
}

async fn set_mail(State(st): St, Admin(me): Admin, Json(s): Json<crate::mail::MailSettings>) -> Response {
    if let Err(e) = crate::mail::save(&st.pools.intake, &s).await {
        return bad(&e);
    }
    crate::audit::record(&st.pools.server, &me.name, "mail.set", &s.smtp_host).await;
    st.events.changed();
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StatusView {
    pub has_line_secret: bool,
    pub has_line_token: bool,
    pub line_listening: bool,
    pub line_port: u16,
    #[serde(default)]
    pub web_listening: bool,
    #[serde(default)]
    pub web_port: u16,
    #[serde(default)]
    pub mail_configured: bool,
    /// 有設定 IMAP 且會收信。
    #[serde(default)]
    pub email_receiving: bool,
    #[serde(default)]
    pub email_last_ok: Option<i64>,
    #[serde(default)]
    pub email_error: Option<String>,
    pub ai_configured: bool,
    pub projects: usize,
}

async fn status(State(st): St) -> Response {
    let health = st.channels.email_health.lock().clone();
    let settings = crate::ai::load(&st.pools.intake).await;
    Json(StatusView {
        has_line_secret: st.secrets.get(aiterm_intake_core::line_server::LINE_SECRET_ID).is_some(),
        has_line_token: st.secrets.get(aiterm_intake_core::line_server::LINE_TOKEN_ID).is_some(),
        line_listening: crate::channels::line_listening(&st),
        line_port: config::load_global(&st.pools.intake).await.line_port,
        web_listening: crate::channels::web_listening(&st),
        web_port: config::load_global(&st.pools.intake).await.web_port,
        mail_configured: crate::mail::configured(&crate::mail::load(&st.pools.intake).await, st.secrets.as_ref()),
        email_receiving: crate::imap::receiving(&crate::mail::load(&st.pools.intake).await, st.secrets.as_ref()),
        email_last_ok: health.last_ok,
        email_error: health.error,
        ai_configured: crate::ai::configured(&settings, st.secrets.as_ref()),
        projects: project_list(&st).await.len(),
    })
    .into_response()
}

async fn audit_log(State(st): St, Admin(_): Admin) -> Response {
    Json(crate::audit::recent(&st.pools.server, 200).await).into_response()
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/projects", get(list_projects).post(create_project))
        .route("/v1/projects/{pid}", put(update_project).delete(delete_project))
        .route("/v1/projects/{pid}/config", get(get_config).put(set_config))
        .route("/v1/projects/{pid}/senders", get(list_senders).put(upsert_sender).delete(delete_sender))
        .route("/v1/projects/{pid}/senders/label", put(set_label))
        .route("/v1/projects/{pid}/dropped", get(dropped))
        .route("/v1/projects/{pid}/check-folders", post(check_folders))
        .route("/v1/projects/{pid}/items", get(list_items))
        .route("/v1/items/{id}", get(get_item).delete(delete_item))
        .route("/v1/items/{id}/approve", post(approve))
        .route("/v1/items/{id}/reject", post(reject))
        .route("/v1/items/{id}/draft", put(update_draft))
        .route("/v1/items/{id}/reply", post(reply))
        .route("/v1/items/{id}/claim", post(claim))
        .route("/v1/items/{id}/release", post(release))
        .route("/v1/items/{id}/heartbeat", post(heartbeat))
        .route("/v1/items/{id}/result", post(claim_result))
        .route("/v1/projects/{pid}/claim-next", post(claim_next))
        .route("/v1/items/{id}/attachments", get(attachments))
        .route("/v1/attachments/{att}/data", get(attachment_data))
        .route("/v1/attachments/{att}/text", get(attachment_text))
        .route("/v1/attachments/{att}/file", get(attachment_file))
        .route("/v1/line/unknown", get(line_unknown))
        .route("/v1/line/unknown/{id}", delete(forget_unknown))
        .route("/v1/secrets/{id}", put(set_secret).delete(clear_secret))
        .route("/v1/ai", get(get_ai).put(set_ai))
        .route("/v1/ai/oauth", put(put_oauth).delete(delete_oauth))
        .route("/v1/ai/models", post(ai_models))
        .route("/v1/ai/test", post(ai_test))
        .route("/v1/mail", get(get_mail).put(set_mail))
        .route("/v1/telegram", get(get_telegram).put(set_telegram))
        .route("/v1/status", get(status))
        .route("/v1/audit", get(audit_log))
}

//! Client 模式：問題收集連到 aiterm-intake-server，看同一份清單與狀態。
//! 連線中時本機的收訊（LINE、網頁表單、信箱、Telegram）與派送都停止，避免同一封信被本機和 server 各收一次。
use super::commands::{ProjectSummary, ProjectsView, RefreshResult};
use super::store::{self, AttachmentRow, IntakeItem};
use super::CHANGED_EVENT;
use aiterm_intake_client::api::{decide_pin, Api};
use aiterm_intake_client::{pair, ClientError};
use aiterm_intake_core::api::Role;
use aiterm_intake_core::config::Config;
use aiterm_intake_core::ops::{ConfigView, SenderDto};
use parking_lot::Mutex;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub const CONNECTION_KEY: &str = "remote_connection";
pub const TOKEN_SECRET_ID: &str = "intake_remote_token";
pub const REMOTE_EVENT: &str = "intake://remote";

/// 存起來的連線資訊（token 不在這裡，在系統鑰匙圈）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Saved {
    pub addr: String,
    /// 自簽憑證釘住的指紋；正式憑證（公開網路）為 `None`。
    pub fingerprint: Option<String>,
    pub name: String,
    pub role: Role,
}

/// 給畫面顯示的連線狀態。
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct RemoteStatus {
    pub connected: bool,
    pub online: bool,
    pub addr: String,
    pub name: String,
    pub role: Option<Role>,
    /// 連線出問題的原因（已被移除、憑證改變、版本不相容…）；正常為 `None`。
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    saved: Option<Saved>,
    api: Option<Api>,
    online: bool,
    error: Option<String>,
    watcher: Option<tauri::async_runtime::JoinHandle<()>>,
}

/// 本機正在處理的、從 server 領來的題目（題目 id → 在哪個分頁、上次續約時間）。
pub struct Claim {
    pub tab: String,
    pub last_beat: std::time::Instant,
    /// 做完後要驗證時的計畫（只有「要改程式」的題目才有）。
    pub verify: Option<VerifyPlan>,
}

/// 領取時決定、做完後執行的驗證：驗證指令與逾時取自專案設定，`snapshot` 是派工前的 git HEAD。
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyPlan {
    pub command: String,
    pub timeout: Duration,
    pub snapshot: Option<String>,
}

#[derive(Default)]
pub struct RemoteState {
    inner: Mutex<Inner>,
    claims: Mutex<std::collections::HashMap<String, Claim>>,
    /// 領來的題目的附件下載到哪（測試用暫存資料夾；預設是 AITERM 的資料夾底下）。
    attach_root: Mutex<Option<std::path::PathBuf>>,
    /// 看板卡片最近一次看到的版本（卡片 id → 版本）；編輯與移動帶上它，別人先改過 server 會擋下。
    board_versions: Mutex<std::collections::HashMap<String, i64>>,
    /// 我領取、正在本機處理的看板卡片（server 卡片 id → 本機的影子卡片）。
    board_claims: Mutex<std::collections::HashMap<String, BoardClaim>>,
}

/// 領取的看板卡片在本機的對應：影子卡片放在哪個本機看板專案、id 是什麼。
#[derive(Debug, Clone)]
pub struct BoardClaim {
    pub local_project_id: String,
    pub local_task_id: String,
    pub last_beat: std::time::Instant,
}

impl RemoteState {
    pub fn set_board_version(&self, card_id: &str, version: i64) {
        self.board_versions.lock().insert(card_id.to_string(), version);
    }

    pub fn track_board_claim(&self, card_id: &str, local_project_id: &str, local_task_id: &str) {
        self.board_claims.lock().insert(card_id.to_string(), BoardClaim { local_project_id: local_project_id.to_string(), local_task_id: local_task_id.to_string(), last_beat: std::time::Instant::now() });
    }

    pub fn untrack_board_claim(&self, card_id: &str) -> Option<BoardClaim> {
        self.board_claims.lock().remove(card_id)
    }

    pub fn board_claims(&self) -> Vec<(String, BoardClaim)> {
        self.board_claims.lock().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// 該續約的看板卡片（距上次續約超過 `every`），並把它們的計時重設。
    pub fn board_due_for_heartbeat(&self, every: Duration) -> Vec<String> {
        let mut g = self.board_claims.lock();
        g.iter_mut()
            .filter(|(_, c)| c.last_beat.elapsed() >= every)
            .map(|(id, c)| {
                c.last_beat = std::time::Instant::now();
                id.clone()
            })
            .collect()
    }

    pub fn board_version(&self, card_id: &str) -> Option<i64> {
        self.board_versions.lock().get(card_id).copied()
    }

    /// 這位成員自己電腦上、某個 server 專案對應的專案資料夾（沒設定就空）。第 3 步才會有設定介面。
    pub fn local_dir_of(&self, _server_project_id: &str) -> String {
        String::new()
    }

    /// 連線中就回傳可呼叫 API 的 Client；本機模式回 `None`。
    pub fn active(&self) -> Option<Api> {
        self.inner.lock().api.clone()
    }

    pub fn status(&self) -> RemoteStatus {
        let g = self.inner.lock();
        match &g.saved {
            Some(s) => RemoteStatus { connected: true, online: g.online, addr: s.addr.clone(), name: s.name.clone(), role: Some(s.role), error: g.error.clone() },
            None => RemoteStatus::default(),
        }
    }

    pub fn attach_root(&self) -> std::path::PathBuf {
        self.attach_root.lock().clone().unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("AITERM").join("intake-remote"))
    }

    #[cfg(test)]
    pub fn set_attach_root(&self, p: std::path::PathBuf) {
        *self.attach_root.lock() = Some(p);
    }

    /// 記下這題是從 server 領來、正在某個分頁處理。
    pub fn track_claim(&self, item_id: &str, tab: &str) {
        self.claims.lock().insert(item_id.to_string(), Claim { tab: tab.to_string(), last_beat: std::time::Instant::now(), verify: None });
    }

    /// 這題做完後要驗證（要在 `track_claim` 之後呼叫）。
    pub fn set_verify_plan(&self, item_id: &str, plan: VerifyPlan) {
        if let Some(c) = self.claims.lock().get_mut(item_id) {
            c.verify = Some(plan);
        }
    }

    /// 不再處理這題；回傳它原本在哪個分頁。不是領來的題目回 `None`。
    pub fn untrack_claim(&self, item_id: &str) -> Option<String> {
        self.claims.lock().remove(item_id).map(|c| c.tab)
    }

    /// 同 `untrack_claim`，但連驗證計畫一起拿走（做完要驗證時用）。
    pub fn take_claim(&self, item_id: &str) -> Option<Claim> {
        self.claims.lock().remove(item_id)
    }

    pub fn is_claimed(&self, item_id: &str) -> bool {
        self.claims.lock().contains_key(item_id)
    }

    /// 該續約的題目（距上次續約超過 `every`），並把它們的計時重設。
    pub fn due_for_heartbeat(&self, every: Duration) -> Vec<String> {
        let mut g = self.claims.lock();
        g.iter_mut()
            .filter(|(_, c)| c.last_beat.elapsed() >= every)
            .map(|(id, c)| {
                c.last_beat = std::time::Instant::now();
                id.clone()
            })
            .collect()
    }

    /// 管理員才能改設定、白名單與祕密（server 端也會擋，這裡讓畫面能事先隱藏）。
    pub fn is_admin(&self) -> bool {
        self.inner.lock().saved.as_ref().is_some_and(|s| s.role == Role::Admin)
    }

    fn set_online(&self, online: bool, error: Option<String>) -> bool {
        let mut g = self.inner.lock();
        let changed = g.online != online || g.error != error;
        g.online = online;
        g.error = error;
        changed
    }

    /// 呼叫失敗時記下需要使用者處理的原因（被移除、憑證改變、版本不符）；一般的離線不算。
    fn note(&self, e: &ClientError) {
        if matches!(e, ClientError::Revoked | ClientError::FingerprintChanged | ClientError::Incompatible { .. }) {
            self.set_online(false, Some(e.to_string()));
        }
    }
}

/// 本機是不是處於「連到 server」模式。
pub fn is_remote<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<Arc<RemoteState>>().is_some_and(|r| r.active().is_some())
}

/// 連線中用這個取得 Client；沒連線回 `None`（呼叫端走本機路徑）。
pub fn active<R: Runtime>(app: &AppHandle<R>) -> Option<(Api, Arc<RemoteState>)> {
    let rs = app.try_state::<Arc<RemoteState>>()?.inner().clone();
    rs.active().map(|a| (a, rs))
}

/// 把 `ClientError` 轉成指令的字串錯誤，並記下需要使用者處理的原因。
pub fn fail(rs: &RemoteState, e: ClientError) -> String {
    rs.note(&e);
    e.to_string()
}

fn status_changed<R: Runtime>(app: &AppHandle<R>) {
    if let Some(rs) = app.try_state::<Arc<RemoteState>>() {
        let _ = app.emit(REMOTE_EVENT, rs.status());
    }
}

// ───────── 連線、中斷、還原 ─────────

async fn save<R: Runtime>(app: &AppHandle<R>, saved: &Saved, token: &str) -> Result<(), String> {
    let db = app.try_state::<super::IntakeDb>().ok_or("intake 未就緒")?;
    let secrets = app.try_state::<Arc<crate::secret::SecretStore>>().ok_or("密鑰庫尚未就緒")?;
    secrets.set(TOKEN_SECRET_ID, token).map_err(|e| e.to_string())?;
    store::set_setting(&db.pool, CONNECTION_KEY, &serde_json::to_string(saved).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// 用邀請碼配對並連線。成功後本機收訊停止。
pub async fn connect<R: Runtime>(app: &AppHandle<R>, addr: &str, code: &str) -> Result<RemoteStatus, String> {
    let paired = pair::pair(addr, code).await.map_err(|e| e.to_string())?;
    let fingerprint = decide_pin(addr, &paired.response.fingerprint).await;
    let api = Api::new(addr, &paired.token, fingerprint.clone()).map_err(|e| e.to_string())?;
    api.check_version().await.map_err(|e| e.to_string())?;
    let saved = Saved { addr: addr.trim().to_string(), fingerprint, name: paired.response.me.name.clone(), role: paired.response.me.role };
    save(app, &saved, &paired.token).await?;
    install(app, saved, api).await;
    Ok(app.state::<Arc<RemoteState>>().status())
}

pub(crate) async fn install<R: Runtime>(app: &AppHandle<R>, saved: Saved, api: Api) {
    let Some(rs) = app.try_state::<Arc<RemoteState>>().map(|s| s.inner().clone()) else { return };
    {
        let mut g = rs.inner.lock();
        if let Some(h) = g.watcher.take() {
            h.abort();
        }
        g.saved = Some(saved);
        g.api = Some(api.clone());
        g.online = false;
        g.error = None;
    }
    // 連到 server 之後，本機的收訊要停（切回本機再重開）。
    stop_local(app);
    let handle = {
        let app = app.clone();
        let rs = rs.clone();
        tauri::async_runtime::spawn(async move { watch(app, rs, api).await })
    };
    rs.inner.lock().watcher = Some(handle);
    status_changed(app);
    let _ = app.emit(CHANGED_EVENT, ());
    let _ = app.emit("tasks-updated", ());
    let _ = app.emit("tasks-updated", ()); // 共用看板也要重新載入（專案清單多了／少了 server 的專案）
}

fn stop_local<R: Runtime>(app: &AppHandle<R>) {
    if let Some(s) = app.try_state::<Arc<super::line_server::LineServer>>() {
        s.stop();
    }
    if let Some(s) = app.try_state::<Arc<super::web::WebServer>>() {
        s.stop();
    }
}

/// 中斷連線並切回本機：刪除 token（server 端的成員紀錄不動，撤銷是管理員的事）。
pub async fn disconnect<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if let Some(rs) = app.try_state::<Arc<RemoteState>>() {
        let mut g = rs.inner.lock();
        if let Some(h) = g.watcher.take() {
            h.abort();
        }
        *g = Inner::default();
    }
    if let Some(secrets) = app.try_state::<Arc<crate::secret::SecretStore>>() {
        secrets.delete(TOKEN_SECRET_ID).ok();
    }
    if let Some(db) = app.try_state::<super::IntakeDb>() {
        sqlx::query("DELETE FROM intake_settings WHERE key = ?").bind(CONNECTION_KEY).execute(&db.pool).await.ok();
    }
    status_changed(app);
    let _ = app.emit(CHANGED_EVENT, ());
    // 切回本機：重開本機的收訊。
    super::web::restart(app).await;
    super::line_server::restart(app).await;
    Ok(())
}

/// 啟動時還原上次的連線。
pub async fn restore<R: Runtime>(app: &AppHandle<R>) {
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    let Some(saved) = store::get_setting(&db.pool, CONNECTION_KEY).await.ok().flatten().and_then(|s| serde_json::from_str::<Saved>(&s).ok()) else { return };
    let Some(token) = app.try_state::<Arc<crate::secret::SecretStore>>().and_then(|s| s.get(TOKEN_SECRET_ID).ok().flatten()) else { return };
    match Api::new(&saved.addr, &token, saved.fingerprint.clone()) {
        Ok(api) => install(app, saved, api).await,
        Err(e) => log::warn!("intake: 無法還原 server 連線：{e}"),
    }
}

/// 連線期間持續收 server 的變動通知；斷線就退避重連。被移除、憑證改變等需要人處理的錯誤不重試。
async fn watch<R: Runtime>(app: AppHandle<R>, rs: Arc<RemoteState>, api: Api) {
    let mut delay = Duration::from_secs(1);
    loop {
        match api.events().await {
            Ok(mut stream) => {
                delay = Duration::from_secs(1);
                if rs.set_online(true, None) {
                    status_changed(&app);
                }
                // 連上先收到目前 seq：斷線期間錯過的變動靠它觸發重抓。
                while stream.next().await.is_some() {
                    let _ = app.emit(CHANGED_EVENT, ());
                    let _ = app.emit("tasks-updated", ()); // server 上的看板有變動
                }
                rs.set_online(false, None);
                status_changed(&app);
            }
            Err(e @ (ClientError::Revoked | ClientError::FingerprintChanged | ClientError::Incompatible { .. })) => {
                rs.note(&e);
                status_changed(&app);
                return;
            }
            Err(_) => {
                if rs.set_online(false, None) {
                    status_changed(&app);
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

// ───────── 資料操作：與本機指令一對一 ─────────

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub async fn get_config(api: &Api, pid: &str) -> Result<ConfigView, ClientError> {
    api.get(&format!("/v1/projects/{}/config", enc(pid))).await
}

pub async fn set_config(api: &Api, pid: &str, config: &Config, telegram_default: bool) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/projects/{}/config", enc(pid)), Some(json!({ "config": config, "telegram_default": telegram_default }))).await.map(|_| ())
}

#[derive(Deserialize)]
struct ServerProject {
    id: String,
    name: String,
    #[allow(dead_code)]
    work_dir: String,
    open_count: i64,
    total: i64,
    enabled: bool,
}

/// server 的專案清單轉成畫面用的形狀；server 沒有「未歸屬」的舊資料。
/// `repo_dir` 一律空：server 上的工作資料夾對這台電腦沒有意義，不能拿來判斷分頁在不在專案目錄裡。
pub async fn projects(api: &Api) -> Result<ProjectsView, ClientError> {
    let list: Vec<ServerProject> = api.get("/v1/projects").await?;
    Ok(ProjectsView {
        projects: list.into_iter().map(|p| ProjectSummary { id: p.id, name: p.name, repo_dir: None, open_count: p.open_count, total: p.total, enabled: p.enabled }).collect(),
        unassigned: 0,
        legacy_settings: false,
    })
}

pub async fn list(api: &Api, pid: &str, status: Option<&str>) -> Result<Vec<IntakeItem>, ClientError> {
    let q = status.map(|s| format!("?status={}", enc(s))).unwrap_or_default();
    api.get(&format!("/v1/projects/{}/items{q}", enc(pid))).await
}

pub async fn senders(api: &Api, pid: &str) -> Result<Vec<SenderDto>, ClientError> {
    api.get(&format!("/v1/projects/{}/senders", enc(pid))).await
}

pub async fn upsert_sender(api: &Api, pid: &str, s: &SenderDto) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/projects/{}/senders", enc(pid)), Some(json!(s))).await.map(|_| ())
}

pub async fn set_sender_label(api: &Api, pid: &str, address: &str, label: &str) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/projects/{}/senders/label", enc(pid)), Some(json!({ "address": address, "label": label }))).await.map(|_| ())
}

pub async fn delete_sender(api: &Api, pid: &str, address: &str) -> Result<bool, ClientError> {
    api.typed(Method::DELETE, &format!("/v1/projects/{}/senders", enc(pid)), Some(json!({ "address": address }))).await
}

pub async fn dropped(api: &Api, pid: &str) -> Result<Vec<(String, i64)>, ClientError> {
    api.get(&format!("/v1/projects/{}/dropped", enc(pid))).await
}

/// 核准、駁回、刪除這類回傳「有沒有做成」的動作。
pub async fn item_action(api: &Api, id: &str, action: &str) -> Result<bool, ClientError> {
    match action {
        "delete" => api.typed(Method::DELETE, &format!("/v1/items/{}", enc(id)), None).await,
        a => api.typed(Method::POST, &format!("/v1/items/{}/{a}", enc(id)), None).await,
    }
}

pub async fn update_draft(api: &Api, id: &str, draft: &str) -> Result<bool, ClientError> {
    api.typed(Method::PUT, &format!("/v1/items/{}/draft", enc(id)), Some(json!({ "draft": draft }))).await
}

pub async fn send_reply(api: &Api, id: &str) -> Result<(), ClientError> {
    api.request(Method::POST, &format!("/v1/items/{}/reply", enc(id)), None).await.map(|_| ())
}

pub async fn attachments(api: &Api, item_id: &str) -> Result<Vec<AttachmentRow>, ClientError> {
    api.get(&format!("/v1/items/{}/attachments", enc(item_id))).await
}

pub async fn attachment(api: &Api, id: &str, kind: &str) -> Result<String, ClientError> {
    api.get(&format!("/v1/attachments/{}/{kind}", enc(id))).await
}

pub async fn check_folders(api: &Api, pid: &str, paths: &[String]) -> Result<Vec<super::refs::FolderCheck>, ClientError> {
    api.typed(Method::POST, &format!("/v1/projects/{}/check-folders", enc(pid)), Some(json!({ "paths": paths }))).await
}

pub async fn line_unknown(api: &Api) -> Result<Vec<super::line_store::Unknown>, ClientError> {
    api.get("/v1/line/unknown").await
}

pub async fn forget_unknown(api: &Api, id: &str) -> Result<bool, ClientError> {
    api.typed(Method::DELETE, &format!("/v1/line/unknown/{}", enc(id)), None).await
}

#[derive(Deserialize)]
struct ServerStatus {
    has_line_secret: bool,
    has_line_token: bool,
    line_listening: bool,
    line_port: u16,
}

pub async fn line_status(api: &Api) -> Result<super::line_server::Status, ClientError> {
    let s: ServerStatus = api.get("/v1/status").await?;
    Ok(super::line_server::Status { has_secret: s.has_line_secret, has_token: s.has_line_token, port: s.line_port, listening: s.line_listening })
}

pub async fn set_secret(api: &Api, id: &str, value: &str) -> Result<(), ClientError> {
    api.request(Method::PUT, &format!("/v1/secrets/{}", enc(id)), Some(json!({ "value": value }))).await.map(|_| ())
}

fn default_imap_port() -> u16 {
    993
}

/// server 的寄信帳號（網頁表單寄驗證碼與回覆用）；密碼不會回傳。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerMailSettings {
    pub smtp_host: String,
    pub smtp_port: u16,
    /// 收信（IMAP）；舊版 server 沒有這兩個欄位。
    #[serde(default)]
    pub imap_host: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    pub username: String,
    pub from: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerMail {
    pub settings: ServerMailSettings,
    pub configured: bool,
}

pub async fn mail_status(api: &Api) -> Result<ServerMail, ClientError> {
    api.get("/v1/mail").await
}

/// 把設定與密碼寫到 server（只有管理員做得到；密碼只寫入）。
pub async fn mail_apply(api: &Api, settings: &ServerMailSettings, password: &str) -> Result<(), ClientError> {
    api.request(Method::PUT, "/v1/mail", Some(serde_json::to_value(settings).map_err(|e| ClientError::Server(e.to_string()))?)).await?;
    set_secret(api, "smtp_password", password).await
}

/// server 的 AI 設定（題目分類與群組「是不是新問題」用）；金鑰不會回傳。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ServerAiSettings {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub base_url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerAi {
    pub settings: ServerAiSettings,
    pub configured: bool,
    /// 最近一次實際呼叫的結果；舊版 server 沒有這個欄位。
    #[serde(default)]
    pub health: ServerAiHealth,
    /// 訂閱登入狀態：`none`｜`ok`｜`needs_login`。
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub login_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ServerAiHealth {
    pub last_ok: Option<i64>,
    pub last_ms: Option<u64>,
    pub error: Option<String>,
}

/// 「測試連線」的結果。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerAiTest {
    pub ok: bool,
    pub ms: u64,
    pub message: String,
}

/// 把為 server 登入取得的憑證交給 server（只有管理員做得到）。
pub async fn ai_login_upload(api: &Api, provider: &str, login: &super::ai_login::Login) -> Result<(), ClientError> {
    api.request(
        Method::PUT,
        "/v1/ai/oauth",
        Some(json!({ "provider": provider, "access": login.access, "refresh": login.refresh, "expires_in": login.expires_in, "account_id": login.account_id, "project_id": login.project_id })),
    )
    .await
    .map(|_| ())
}

pub async fn ai_logout(api: &Api) -> Result<(), ClientError> {
    api.request(Method::DELETE, "/v1/ai/oauth", None).await.map(|_| ())
}

/// 還沒儲存的值也能測；`api_key` 空白＝用 server 已存的。
pub async fn ai_models(api: &Api, settings: &ServerAiSettings, api_key: &str) -> Result<Vec<String>, ClientError> {
    api.typed(Method::POST, "/v1/ai/models", Some(ai_probe(settings, api_key))).await
}

pub async fn ai_test(api: &Api, settings: &ServerAiSettings, api_key: &str) -> Result<ServerAiTest, ClientError> {
    api.typed(Method::POST, "/v1/ai/test", Some(ai_probe(settings, api_key))).await
}

fn ai_probe(s: &ServerAiSettings, api_key: &str) -> serde_json::Value {
    json!({ "provider": s.provider, "model": s.model, "base_url": s.base_url, "api_key": api_key.trim() })
}

/// server 自動處理題目的執行器設定與狀態。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerRunner {
    pub settings: ServerRunnerSettings,
    pub availability: ServerRunnerAvailability,
    pub running: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerRunnerSettings {
    pub claude_path: String,
    pub max_concurrent: usize,
    pub timeout_secs: u64,
    pub max_turns: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ServerRunnerAvailability {
    #[serde(default)]
    pub checked: bool,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub async fn runner_status(api: &Api) -> Result<ServerRunner, ClientError> {
    api.get("/v1/runner").await
}

pub async fn runner_apply(api: &Api, s: &ServerRunnerSettings) -> Result<ServerRunner, ClientError> {
    api.typed(Method::PUT, "/v1/runner", Some(serde_json::to_value(s).map_err(|e| ClientError::Server(e.to_string()))?)).await
}

/// 管理員核准（合併）或駁回 server 自動處理的改動。
pub async fn merge_item(api: &Api, id: &str, approve: bool) -> Result<(), ClientError> {
    api.request(Method::POST, &format!("/v1/items/{}/merge", enc(id)), Some(json!({ "approve": approve }))).await.map(|_| ())
}

pub async fn ai_status(api: &Api) -> Result<ServerAi, ClientError> {
    api.get("/v1/ai").await
}

/// 寫入 AI 設定；`api_key` 非空才一併更新金鑰（空＝不變，畫面不會回填已存的金鑰）。
pub async fn ai_apply(api: &Api, settings: &ServerAiSettings, api_key: &str) -> Result<(), ClientError> {
    api.request(Method::PUT, "/v1/ai", Some(serde_json::to_value(settings).map_err(|e| ClientError::Server(e.to_string()))?)).await?;
    if !api_key.trim().is_empty() {
        set_secret(api, "ai_api_key", api_key.trim()).await?;
    }
    Ok(())
}

/// server 的 Telegram 狀態與設定；機器人金鑰不會回傳。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ServerTelegram {
    pub settings: ServerTelegramSettings,
    pub has_token: bool,
    pub listening: bool,
    /// 有金鑰但沒有專案啟用 Telegram：server 沒在收訊。
    #[serde(default)]
    pub idle: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ServerTelegramSettings {
    #[serde(default)]
    pub owner_chat_id: String,
}

pub async fn telegram_status(api: &Api) -> Result<ServerTelegram, ClientError> {
    api.get("/v1/telegram").await
}

/// 寫入主人聊天室與（有給才更新的）機器人金鑰；金鑰空字串＝不變。
pub async fn telegram_apply(api: &Api, owner_chat_id: &str, bot_token: &str) -> Result<(), ClientError> {
    api.request(Method::PUT, "/v1/telegram", Some(json!({ "owner_chat_id": owner_chat_id }))).await?;
    if !bot_token.trim().is_empty() {
        set_secret(api, "telegram_bot_token", bot_token.trim()).await?;
    }
    Ok(())
}

pub async fn clear_secret(api: &Api, id: &str) -> Result<(), ClientError> {
    api.request(Method::DELETE, &format!("/v1/secrets/{}", enc(id)), None).await.map(|_| ())
}

// ───────── 領取處理 ─────────

pub async fn get_item(api: &Api, id: &str) -> Result<IntakeItem, ClientError> {
    api.get(&format!("/v1/items/{}", enc(id))).await
}

pub async fn claim_item(api: &Api, id: &str) -> Result<IntakeItem, ClientError> {
    api.typed(Method::POST, &format!("/v1/items/{}/claim", enc(id)), None).await
}

pub async fn claim_next(api: &Api, pid: &str) -> Result<Option<IntakeItem>, ClientError> {
    api.typed(Method::POST, &format!("/v1/projects/{}/claim-next", enc(pid)), None).await
}

pub async fn heartbeat(api: &Api, id: &str) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/items/{}/heartbeat", enc(id)), None).await
}

pub async fn release(api: &Api, id: &str) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/items/{}/release", enc(id)), None).await
}

/// 交回結果；`verification` 是（報告文字、是否通過），沒驗證傳 `None`。
pub async fn submit_reply(api: &Api, id: &str, reply: &str, verification: Option<(&str, bool)>) -> Result<bool, ClientError> {
    let mut body = json!({ "reply": reply });
    if let Some((report, passed)) = verification {
        body["verification"] = json!(report);
        body["verified"] = json!(passed);
    }
    api.typed(Method::POST, &format!("/v1/items/{}/result", enc(id)), Some(body)).await
}

pub async fn submit_error(api: &Api, id: &str, error: &str) -> Result<bool, ClientError> {
    api.typed(Method::POST, &format!("/v1/items/{}/result", enc(id)), Some(json!({ "error": error }))).await
}

/// 把題目的附件下載到本機（Claude Code 讀得到的路徑），回傳路徑已換成本機的附件清單。
pub async fn download_attachments(api: &Api, item_id: &str, root: &std::path::Path) -> Vec<AttachmentRow> {
    let Ok(rows) = attachments(api, item_id).await else { return Vec::new() };
    let dir = root.join(item_id);
    let mut out = Vec::new();
    for mut row in rows {
        // 檔名只取最後一段，避免伺服器給的名稱把檔案寫到目錄外。
        let name = std::path::Path::new(&row.filename).file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
        let Ok(bytes) = api.download(&format!("/v1/attachments/{}/file", enc(&row.id))).await else { continue };
        if std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(&name), &bytes)).is_err() {
            continue;
        }
        row.path = dir.join(&name).to_string_lossy().into_owned();
        out.push(row);
    }
    out
}

/// Remote 模式下「重新整理」：server 自己收信、有變動就推播，不需要手動收；回報「沒去收信」即可。
pub fn refresh_result() -> RefreshResult {
    RefreshResult { polled: false, ok: true, new_items: 0, error: None }
}

pub const LOCAL_ONLY: &str = "連線到 server 時不能使用這個功能；請先在「連線」中斷連線。";

#[cfg(test)]
mod tests {
    use super::*;
    use aiterm_intake_core::rules::Level;
    use aiterm_intake_core::store::NewItem;
    use aiterm_intake_server::testkit::TestServer;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::Listener;

    async fn local_pool() -> sqlx::SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }

    async fn server_item(srv: &TestServer, pid: &str, subject: &str) -> IntakeItem {
        store::insert_item(&srv.state().pools.intake, NewItem { project_id: pid, id: None, source: "line", sender: "line:u0123456789abcdef0123456789abcdef", level: Level::External, subject, body: "內容", message_id: subject, status: store::PENDING })
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn server_ai_settings_round_trip_and_the_key_is_written_only_when_given() {
        use aiterm_intake_core::host::Secrets;
        let srv = TestServer::start().await;
        let api = srv.paired("小華", Role::Admin).await;
        assert!(!ai_status(&api).await.unwrap().configured);
        let settings = ServerAiSettings { provider: "openai".into(), model: "gpt-x".into(), base_url: String::new() };
        ai_apply(&api, &settings, "  sk-secret  ").await.unwrap();
        let v = ai_status(&api).await.unwrap();
        assert_eq!((v.settings.clone(), v.configured), (settings.clone(), true));
        assert_eq!(srv.state().secrets.get("ai_api_key").as_deref(), Some("sk-secret"), "金鑰會去掉前後空白");
        assert!(!serde_json::to_string(&v).unwrap().contains("sk-secret"), "金鑰不會被讀回");
        // 空金鑰＝不變：只改模型，已存的金鑰還在。
        ai_apply(&api, &ServerAiSettings { model: "gpt-y".into(), ..settings }, "").await.unwrap();
        assert_eq!(srv.state().secrets.get("ai_api_key").as_deref(), Some("sk-secret"));
        assert_eq!(ai_status(&api).await.unwrap().settings.model, "gpt-y");
    }

    #[tokio::test]
    async fn applying_a_mail_account_writes_both_directions_and_the_password() {
        use aiterm_intake_core::host::Secrets;
        let srv = TestServer::start().await;
        let api = srv.paired("小華", Role::Admin).await;
        let s = ServerMailSettings { smtp_host: "smtp.example.com".into(), smtp_port: 587, imap_host: "imap.example.com".into(), imap_port: 993, username: "bot@example.com".into(), from: "bot@example.com".into() };
        mail_apply(&api, &s, "pw-123").await.unwrap();
        let v = mail_status(&api).await.unwrap();
        assert_eq!((v.settings, v.configured), (s, true));
        assert_eq!(srv.state().secrets.get("smtp_password").as_deref(), Some("pw-123"));
    }

    #[tokio::test]
    async fn server_telegram_round_trips_and_the_token_is_written_only_when_given() {
        use aiterm_intake_core::host::Secrets;
        let srv = TestServer::start().await;
        let api = srv.paired("小華", Role::Admin).await;
        let v = telegram_status(&api).await.unwrap();
        assert_eq!((v.has_token, v.settings.owner_chat_id.as_str()), (false, ""));
        telegram_apply(&api, "111", "  123:ABC  ").await.unwrap();
        let v = telegram_status(&api).await.unwrap();
        assert_eq!((v.has_token, v.settings.owner_chat_id.as_str()), (true, "111"));
        assert_eq!(srv.state().secrets.get("telegram_bot_token").as_deref(), Some("123:ABC"));
        assert!(!serde_json::to_string(&v).unwrap().contains("123:ABC"), "金鑰不會被讀回");
        telegram_apply(&api, "222", "").await.unwrap();
        assert_eq!(srv.state().secrets.get("telegram_bot_token").as_deref(), Some("123:ABC"), "空金鑰＝不變");
        assert_eq!(telegram_status(&api).await.unwrap().settings.owner_chat_id, "222");
        assert!(telegram_apply(&api, "不是數字", "").await.is_err());
    }

    #[tokio::test]
    async fn uploading_a_server_login_makes_the_server_ai_usable_and_logout_removes_it() {
        use aiterm_intake_core::host::Secrets;
        let srv = TestServer::start().await;
        let api = srv.paired("小華", Role::Admin).await;
        assert_eq!(ai_status(&api).await.unwrap().login, "none");
        ai_apply(&api, &ServerAiSettings { provider: "anthropic_oauth".into(), model: "claude-x".into(), base_url: String::new() }, "").await.unwrap();
        ai_login_upload(&api, "anthropic_oauth", &crate::intake::ai_login::Login { access: "ACCESS".into(), refresh: "REFRESH".into(), expires_in: Some(3600), account_id: None, project_id: None }).await.unwrap();
        let v = ai_status(&api).await.unwrap();
        assert_eq!((v.configured, v.login.as_str(), v.settings.provider.as_str()), (true, "ok", "anthropic_oauth"));
        assert!(!serde_json::to_string(&v).unwrap().contains("REFRESH"), "憑證不會被讀回");
        assert_eq!(srv.state().secrets.get("ai_oauth_refresh").as_deref(), Some("REFRESH"));
        ai_logout(&api).await.unwrap();
        let v = ai_status(&api).await.unwrap();
        assert_eq!((v.configured, v.login.as_str()), (false, "none"));
        assert!(srv.state().secrets.get("ai_oauth_refresh").is_none());
    }

    #[tokio::test]
    async fn the_runner_settings_round_trip_report_availability_and_only_admins_change_them_or_merge() {
        let srv = TestServer::start().await;
        let admin = srv.paired("小華", Role::Admin).await;
        let member = srv.paired("小明", Role::Member).await;
        let s = ServerRunnerSettings { claude_path: "/不存在/claude".into(), max_concurrent: 2, timeout_secs: 120, max_turns: 9 };
        let v = runner_apply(&admin, &s).await.unwrap();
        assert_eq!(v.settings, s);
        assert!(!v.availability.available && v.availability.error.as_deref().unwrap_or("").contains("/不存在/claude"), "{v:?}");
        assert_eq!(runner_status(&member).await.unwrap().settings, s, "成員看得到目前的設定");
        assert!(runner_apply(&member, &s).await.is_err(), "成員不能改");
        assert!(runner_apply(&admin, &ServerRunnerSettings { timeout_secs: 5, ..s.clone() }).await.is_err(), "不合理的值會被拒絕");
        // 沒有等待核准的改動，核准會被拒絕（成員更不行）。
        assert!(merge_item(&admin, "不存在的題目", true).await.is_err());
        assert!(merge_item(&member, "不存在的題目", false).await.is_err());
    }

    #[tokio::test]
    async fn a_member_cannot_change_the_servers_ai_or_mail() {
        let srv = TestServer::start().await;
        let member = srv.paired("小明", Role::Member).await;
        assert!(ai_apply(&member, &ServerAiSettings { provider: "openai".into(), model: "m".into(), base_url: String::new() }, "k").await.is_err());
        let s = ServerMailSettings { smtp_host: "h".into(), smtp_port: 25, imap_host: String::new(), imap_port: 993, username: "u".into(), from: String::new() };
        assert!(mail_apply(&member, &s, "p").await.is_err());
        assert!(ai_login_upload(&member, "anthropic_oauth", &crate::intake::ai_login::Login { access: "A".into(), refresh: "R".into(), expires_in: None, account_id: None, project_id: None }).await.is_err());
        assert!(ai_logout(&member).await.is_err());
    }

    #[test]
    fn ids_are_percent_encoded_in_paths() {
        assert_eq!(enc("a-b_c.d~1"), "a-b_c.d~1");
        assert_eq!(enc("a/b c?"), "a%2Fb%20c%3F");
        assert_eq!(enc("中"), "%E4%B8%AD");
    }

    #[tokio::test]
    async fn every_data_operation_maps_onto_the_servers_api() {
        let srv = TestServer::start().await;
        let api = srv.paired("小華", Role::Admin).await;
        let pid = srv.project("範例科技").await;

        let view = projects(&api).await.unwrap();
        assert_eq!((view.projects.len(), view.unassigned, view.legacy_settings), (1, 0, false));
        assert_eq!(view.projects[0].name, "範例科技");

        let cfg = Config { tag: "[X]".into(), ..Config::default() };
        set_config(&api, &pid, &cfg, false).await.unwrap();
        assert_eq!(get_config(&api, &pid).await.unwrap().config.tag, "[X]");

        let s = SenderDto { address: "bob@corp.com".into(), level: Level::Internal, skip_auth: false, label: "小明".into() };
        upsert_sender(&api, &pid, &s).await.unwrap();
        assert_eq!(senders(&api, &pid).await.unwrap(), vec![s.clone()]);
        set_sender_label(&api, &pid, &s.address, "改名").await.unwrap();
        assert_eq!(senders(&api, &pid).await.unwrap()[0].label, "改名");
        assert!(delete_sender(&api, &pid, &s.address).await.unwrap());

        let item = server_item(&srv, &pid, "電腦壞了").await;
        let listed = list(&api, &pid, None).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(list(&api, &pid, Some("done")).await.unwrap().len(), 0, "狀態篩選有帶到 server");
        assert!(item_action(&api, &item.id, "approve").await.unwrap());
        assert!(attachments(&api, &item.id).await.unwrap().is_empty());
        assert!(!update_draft(&api, &item.id, "草稿").await.unwrap(), "還不是草稿的題目不能改草稿，這個判斷在 server");
        let drafting = store::insert_item(&srv.state().pools.intake, NewItem { project_id: &pid, id: None, source: "line", sender: "line:u0123456789abcdef0123456789abcdef", level: Level::External, subject: "草稿題", body: "b", message_id: "d", status: store::DRAFT }).await.unwrap().unwrap();
        assert!(update_draft(&api, &drafting.id, "新草稿").await.unwrap());
        assert!(item_action(&api, &item.id, "delete").await.unwrap());
        assert!(item_action(&api, &drafting.id, "delete").await.unwrap());
        assert!(list(&api, &pid, None).await.unwrap().is_empty());

        assert_eq!(check_folders(&api, &pid, &["/definitely/not/here".into()]).await.unwrap()[0].exists, false);
        assert!(line_unknown(&api).await.unwrap().is_empty());
        set_secret(&api, "line_channel_secret", "s").await.unwrap();
        let st = line_status(&api).await.unwrap();
        assert!(st.has_secret && !st.has_token);
        clear_secret(&api, "line_channel_secret").await.unwrap();
        assert!(!line_status(&api).await.unwrap().has_secret);
    }

    /// 把 mock app 接上本機資料庫與遠端狀態，並連到測試 server。
    async fn connected(srv: &TestServer, role: Role) -> (tauri::App<tauri::test::MockRuntime>, sqlx::SqlitePool, Arc<RemoteState>) {
        let app = tauri::test::mock_app();
        let local = local_pool().await;
        app.manage(super::super::IntakeDb { pool: local.clone() });
        app.manage(Arc::new(RemoteState::default()));
        app.manage(Arc::new(super::super::line_server::LineServer::default()));
        app.manage(Arc::new(super::super::web::WebServer::default()));
        let code = srv.invite("小華", role).await;
        let paired = pair::pair(&srv.addr(), &code).await.unwrap();
        let api = Api::new(&srv.addr(), &paired.token, Some(paired.response.fingerprint.clone())).unwrap();
        let saved = Saved { addr: srv.addr(), fingerprint: Some(paired.response.fingerprint), name: "小華".into(), role };
        install(app.handle(), saved, api).await;
        let rs = app.state::<Arc<RemoteState>>().inner().clone();
        (app, local, rs)
    }

    #[tokio::test]
    async fn while_connected_commands_read_the_server_and_never_the_local_database() {
        let srv = TestServer::start().await;
        let pid = srv.project("範例科技").await;
        server_item(&srv, &pid, "server 上的題目").await;
        let (app, local, rs) = connected(&srv, Role::Admin).await;
        // 本機也有一題，內容不同：連線時不應該出現。
        store::insert_item(&local, NewItem { project_id: &pid, id: None, source: "email", sender: "a@b.co", level: Level::External, subject: "本機的題目", body: "b", message_id: "m", status: store::PENDING }).await.unwrap();

        assert!(is_remote(app.handle()));
        let items = super::super::commands::intake_list(app.state(), pid.clone(), None, app.state()).await.unwrap();
        assert_eq!(items.iter().map(|i| i.subject.as_str()).collect::<Vec<_>>(), vec!["server 上的題目"]);
        let senders = super::super::commands::intake_senders(app.state(), pid.clone(), app.state()).await.unwrap();
        assert!(senders.is_empty(), "白名單也是 server 的");
        assert_eq!(rs.status().role, Some(Role::Admin));
        assert_eq!(store::list_items(&local, &pid, None).await.unwrap().len(), 1, "本機資料庫沒被動到");
    }

    #[tokio::test]
    async fn the_servers_change_notifications_become_local_changed_events() {
        let srv = TestServer::start().await;
        let (app, _local, _rs) = connected(&srv, Role::Admin).await;
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        app.listen(CHANGED_EVENT, move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });
        let wait = |n: usize| {
            let count = count.clone();
            async move {
                for _ in 0..100 {
                    if count.load(Ordering::SeqCst) >= n {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                false
            }
        };
        assert!(wait(1).await, "連上時先收到目前的 seq，要觸發一次重抓");
        let before = count.load(Ordering::SeqCst);
        srv.project("新專案").await;
        srv.state().events.changed_for_test();
        assert!(wait(before + 1).await, "server 的變動要變成本機的 changed 事件");
    }

    #[tokio::test]
    async fn disconnecting_returns_to_local_mode() {
        let srv = TestServer::start().await;
        let (app, _local, rs) = connected(&srv, Role::Member).await;
        assert!(rs.status().connected && !rs.is_admin());
        disconnect(app.handle()).await.unwrap();
        assert!(!is_remote(app.handle()));
        assert_eq!(rs.status(), RemoteStatus::default());
    }

    #[tokio::test]
    async fn a_revoked_member_is_told_so_and_not_retried_forever() {
        let srv = TestServer::start().await;
        let (app, _local, rs) = connected(&srv, Role::Member).await;
        aiterm_intake_server::members::revoke(&srv.state().pools.server, "小華", aiterm_intake_server::db::now()).await.unwrap();
        // 下一個請求就是 401：指令回覆明確的原因，狀態記下來讓畫面顯示。
        let err = super::super::commands::intake_list(app.state(), "p".into(), None, app.state()).await.unwrap_err();
        assert!(err.contains("已被移除"), "{err}");
        assert!(rs.status().error.is_some_and(|e| e.contains("已被移除")));
    }
}

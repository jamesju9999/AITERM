//! LINE webhook 的本機伺服器，與回覆用的 push API。
//! 預設只綁 127.0.0.1；要讓 LINE 連得到，由使用者自備 Cloudflare Tunnel／ngrok 指向這個埠。
//! 設計：docs/superpowers/specs/2026-11-23-intake-line-design.md
use super::line_source::{self, GroupMsg, MAX_PER_REQUEST, SEGMENT_CHARS};
use super::line_store;
use super::queue::Ingested;
use super::telegram_source::Candidate;
use super::attach;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use parking_lot::Mutex;
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

pub(crate) const LINE_SECRET_ID: &str = "line_channel_secret";
pub(crate) const LINE_TOKEN_ID: &str = "line_channel_access_token";
pub const WEBHOOK_PATH: &str = "/line/webhook";
const API_BASE: &str = "https://api.line.me";
/// LINE 的 webhook 本文很小；1 MB 綽綽有餘，也擋掉灌爆。
const MAX_BODY: usize = 1024 * 1024;

type Candidates = Pin<Box<dyn Future<Output = Vec<Candidate>> + Send>>;

pub struct LineState {
    pub pool: SqlitePool,
    pub secret: String,
    pub attach_root: std::path::PathBuf,
    /// 查成員／群組名稱用的 Channel access token（沒有就只用簡短編號）。
    pub token: Option<String>,
    pub api_base: String,
    pub candidates: Box<dyn Fn() -> Candidates + Send + Sync>,
    pub on_change: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
pub struct LineServer {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    listening: AtomicBool,
}

impl LineServer {
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::SeqCst)
    }
}

async fn webhook(State(st): State<Arc<LineState>>, headers: HeaderMap, body: Bytes) -> StatusCode {
    // 簽章是唯一的入口驗證：沒過就連 body 都不看。
    let sig = headers.get("x-line-signature").and_then(|v| v.to_str().ok());
    if !line_source::verify_signature(&st.secret, &body, sig) {
        return StatusCode::UNAUTHORIZED;
    }
    let parsed = line_source::parse_webhook(&body);
    if parsed.messages.is_empty() && parsed.joined.is_empty() {
        return StatusCode::OK;
    }
    let cands = (st.candidates)().await;
    // 回非 2xx 會讓 LINE 重送，所以資料庫暫時出錯時回 500 是對的；重送由去重擋下。
    match handle(&st, &cands, parsed).await {
        Ok(true) => (st.on_change)(),
        Ok(false) => {}
        Err(e) => {
            log::warn!("intake: LINE ingest failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }
    StatusCode::OK
}

/// 逐則處理；有新題目或有變動（存了群組訊息）回 `true`。
async fn handle(st: &LineState, cands: &[Candidate], parsed: line_source::Parsed) -> Result<bool, sqlx::Error> {
    let now = chrono::Utc::now().timestamp();
    let retention = super::config::load_global(&st.pool).await.line_retention_days;
    let mut changed = false;
    for ev in parsed.messages {
        let result = match &ev.chat {
            Some(chat) => {
                let key = line_source::sender_key(chat);
                if !line_source::is_whitelisted(&st.pool, cands, &key).await? {
                    // 白名單外的群組：只記編號與名稱供使用者加入，不存任何訊息。
                    note_unknown(st, chat, "group", now).await?;
                    continue;
                }
                let name = member_name(st, chat, &ev.user_id).await;
                let group_name = chat_name(st, chat).await;
                let m = GroupMsg { chat, user_id: &ev.user_id, name: &name, message_id: &ev.message_id, text: &ev.text, ts: ev.ts, mentioned: ev.mentioned };
                line_source::route_group(&st.pool, cands, &m, &group_name, retention, now, &st.attach_root).await?
            }
            None => {
                let r = line_source::route(&st.pool, cands, &ev.user_id, &ev.message_id, &ev.text, &st.attach_root).await?;
                if r == Ingested::Dropped("not_allowed") {
                    note_unknown(st, &ev.user_id, "user", now).await?;
                }
                r
            }
        };
        changed |= matches!(result, Ingested::Queued(_) | Ingested::Dropped("recorded"));
    }
    for chat in parsed.joined {
        if !line_source::is_whitelisted(&st.pool, cands, &line_source::sender_key(&chat)).await? {
            note_unknown(st, &chat, "group", now).await?;
        }
    }
    // 保存期限到了就刪（保存天數 0＝不保存）。
    line_store::purge_expired(&st.pool, retention, now).await?;
    Ok(changed)
}

/// 記下一個還沒加入白名單的來源（名稱取得失敗就只記編號）。
async fn note_unknown(st: &LineState, id: &str, kind: &str, now: i64) -> Result<(), sqlx::Error> {
    let id = id.to_lowercase();
    let name = if kind == "group" { chat_name(st, &id).await } else { user_profile_name(st, &id).await };
    line_store::note_unknown(&st.pool, &id, kind, &name, now).await
}

fn short_id(id: &str) -> String {
    format!("成員{}", &id[id.len().saturating_sub(4)..])
}

async fn get_json(st: &LineState, path: &str) -> Option<serde_json::Value> {
    let token = st.token.as_deref()?;
    let res = reqwest::Client::new().get(format!("{}{path}", st.api_base)).bearer_auth(token).timeout(std::time::Duration::from_secs(3)).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json().await.ok()
}

/// 群組成員的顯示名稱：先看快取，再問 LINE；都沒有就用簡短編號。
async fn member_name(st: &LineState, chat: &str, user_id: &str) -> String {
    if user_id.is_empty() {
        return String::new();
    }
    if let Ok(Some(n)) = line_store::get_name(&st.pool, user_id).await {
        return n;
    }
    let kind = if chat.starts_with(['R', 'r']) { "room" } else { "group" };
    if let Some(n) = get_json(st, &format!("/v2/bot/{kind}/{chat}/member/{user_id}")).await.and_then(|v| v.get("displayName")?.as_str().map(str::to_string)) {
        line_store::set_name(&st.pool, user_id, &n).await.ok();
        return line_store::get_name(&st.pool, user_id).await.ok().flatten().unwrap_or(n);
    }
    short_id(user_id)
}

/// 好友的顯示名稱（一對一）。
async fn user_profile_name(st: &LineState, user_id: &str) -> String {
    if let Ok(Some(n)) = line_store::get_name(&st.pool, user_id).await {
        return n;
    }
    match get_json(st, &format!("/v2/bot/profile/{user_id}")).await.and_then(|v| v.get("displayName")?.as_str().map(str::to_string)) {
        Some(n) => {
            line_store::set_name(&st.pool, user_id, &n).await.ok();
            n
        }
        None => String::new(),
    }
}

/// 群組名稱（聊天室沒有名稱）。查不到回空字串。
async fn chat_name(st: &LineState, chat: &str) -> String {
    if chat.starts_with(['R', 'r']) {
        return String::new();
    }
    if let Ok(Some(n)) = line_store::get_name(&st.pool, chat).await {
        return n;
    }
    match get_json(st, &format!("/v2/bot/group/{chat}/summary")).await.and_then(|v| v.get("groupName")?.as_str().map(str::to_string)) {
        Some(n) => {
            line_store::set_name(&st.pool, chat, &n).await.ok();
            n
        }
        None => String::new(),
    }
}

pub fn router(st: Arc<LineState>) -> Router {
    Router::new().route(WEBHOOK_PATH, post(webhook).layer(DefaultBodyLimit::max(MAX_BODY))).with_state(st)
}

fn secret_of<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<String> {
    app.try_state::<Arc<crate::secret::SecretStore>>()?.get(id).ok().flatten().filter(|s| !s.trim().is_empty())
}

/// 依目前設定（重新）啟動 webhook 伺服器：任一專案啟用 LINE 且已設 Channel secret 才開。
pub async fn restart<R: Runtime>(app: &AppHandle<R>) {
    let Some(server) = app.try_state::<Arc<LineServer>>() else { return };
    if let Some(h) = server.handle.lock().take() {
        h.abort();
    }
    server.listening.store(false, Ordering::SeqCst);
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    let any_enabled = super::all_configs(app, &db.pool).await.into_iter().any(|(_, _, c)| c.line_enabled);
    let Some(secret) = secret_of(app, LINE_SECRET_ID) else { return };
    if !any_enabled {
        return;
    }
    let port = super::config::load_global(&db.pool).await.line_port;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("intake line: 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (a_cands, a_change) = (app.clone(), app.clone());
    let st = Arc::new(LineState {
        pool: db.pool.clone(),
        secret,
        attach_root: attach::root(),
        token: secret_of(app, LINE_TOKEN_ID),
        api_base: API_BASE.to_string(),
        candidates: Box::new(move || {
            let app = a_cands.clone();
            Box::pin(async move {
                let Some(db) = app.try_state::<super::IntakeDb>() else { return Vec::new() };
                super::all_configs(&app, &db.pool).await.into_iter().map(|(id, name, cfg)| Candidate { id, name, cfg }).collect()
            })
        }),
        on_change: Box::new(move || {
            use tauri::Emitter;
            let _ = a_change.emit(super::CHANGED_EVENT, ());
        }),
    });
    let flag: Arc<LineServer> = server.inner().clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router(st)).await {
            log::warn!("intake line: 伺服器結束：{e}");
        }
        flag.listening.store(false, Ordering::SeqCst);
    });
    server.listening.store(true, Ordering::SeqCst);
    *server.handle.lock() = Some(task);
}

/// 用 LINE push API 回覆。內容切段（每則不超過 `SEGMENT_CHARS`），每次請求最多 `MAX_PER_REQUEST` 則。
pub async fn push(token: &str, to: &str, text: &str) -> Result<(), String> {
    push_to(API_BASE, token, to, text).await
}

pub async fn push_to(base: &str, token: &str, to: &str, text: &str) -> Result<(), String> {
    // 只剩空白的段落（例如被切開的換行）LINE 會以 400 拒收，直接略過。
    let parts: Vec<String> = line_source::split_message(text, SEGMENT_CHARS).into_iter().filter(|p| !p.trim().is_empty()).collect();
    if parts.is_empty() {
        return Err("回覆內容是空的".into());
    }
    let client = reqwest::Client::new();
    for batch in parts.chunks(MAX_PER_REQUEST) {
        let messages: Vec<_> = batch.iter().map(|t| serde_json::json!({ "type": "text", "text": t })).collect();
        let res = client
            .post(format!("{base}/v2/bot/message/push"))
            .bearer_auth(token)
            .json(&serde_json::json!({ "to": to, "messages": messages }))
            .send()
            .await
            .map_err(|e| format!("LINE 回覆失敗：{e}"))?;
        let status = res.status();
        if !status.is_success() {
            let detail = res.json::<serde_json::Value>().await.ok().and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string)).unwrap_or_default();
            let hint = if status.as_u16() == 429 { "（LINE 官方帳號本月訊息額度已用完）" } else { "" };
            return Err(format!("LINE 回覆失敗：HTTP {status}{hint} {detail}").trim_end().to_string());
        }
    }
    Ok(())
}

/// 回覆用憑證；沒設定時回人看得懂的錯誤。
pub fn token<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    secret_of(app, LINE_TOKEN_ID).ok_or_else(|| "尚未設定 LINE 的 Channel access token".to_string())
}

/// 供設定頁顯示：憑證是否已設定（憑證本身不外傳）與監聽狀態。
#[derive(serde::Serialize)]
pub struct Status {
    pub has_secret: bool,
    pub has_token: bool,
    pub port: u16,
    pub listening: bool,
}

pub async fn status<R: Runtime>(app: &AppHandle<R>) -> Status {
    let port = match app.try_state::<super::IntakeDb>() {
        Some(db) => super::config::load_global(&db.pool).await.line_port,
        None => super::config::GlobalConfig::default().line_port,
    };
    Status {
        has_secret: secret_of(app, LINE_SECRET_ID).is_some(),
        has_token: secret_of(app, LINE_TOKEN_ID).is_some(),
        port,
        listening: app.try_state::<Arc<LineServer>>().is_some_and(|s| s.listening()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::store;
    use crate::intake::config::Config;
    use crate::intake::rules::{Level, Sender};
    use axum::body::Body;
    use axum::http::Request;
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const UID: &str = "U0123456789abcdef0123456789abcdef";
    const SECRET: &str = "s3cret";

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    async fn app() -> (Router, SqlitePool, Arc<std::sync::atomic::AtomicUsize>) {
        app_with(None, String::new()).await
    }

    async fn app_with(token: Option<String>, api_base: String) -> (Router, SqlitePool, Arc<std::sync::atomic::AtomicUsize>) {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        store::upsert_sender(&pool, "", &Sender { address: format!("line:{UID}"), level: Level::External, skip_auth: false }).await.unwrap();
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = changes.clone();
        let st = Arc::new(LineState {
            pool: pool.clone(),
            secret: SECRET.into(),
            attach_root: std::env::temp_dir().join("aiterm-line-test"),
            token,
            api_base,
            candidates: Box::new(|| Box::pin(async { vec![Candidate { id: String::new(), name: String::new(), cfg: Config { line_enabled: true, ..Config::default() } }] })),
            on_change: Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        });
        (router(st), pool, changes)
    }

    async fn post_hook(r: &Router, body: &str, sig: Option<&str>) -> StatusCode {
        let mut req = Request::post(WEBHOOK_PATH).header("content-type", "application/json");
        if let Some(s) = sig {
            req = req.header("x-line-signature", s);
        }
        r.clone().oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap().status()
    }

    fn text_event(id: &str, text: &str) -> String {
        format!(r#"{{"events":[{{"type":"message","source":{{"type":"user","userId":"{UID}"}},"message":{{"type":"text","id":"{id}","text":"{text}"}}}}]}}"#)
    }

    #[tokio::test]
    async fn unsigned_or_wrongly_signed_requests_are_rejected_and_store_nothing() {
        let (r, pool, changes) = app().await;
        let body = text_event("1", "電腦壞了");
        assert_eq!(post_hook(&r, &body, None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(post_hook(&r, &body, Some(&sign("wrong", body.as_bytes()))).await, StatusCode::UNAUTHORIZED);
        assert_eq!(post_hook(&r, &body, Some(&sign(SECRET, b"another body"))).await, StatusCode::UNAUTHORIZED);
        assert!(store::list_items(&pool, "", None).await.unwrap().is_empty());
        assert_eq!(changes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_signed_message_from_an_allowlisted_user_is_queued_and_announced() {
        let (r, pool, changes) = app().await;
        let body = text_event("1", "電腦壞了");
        assert_eq!(post_hook(&r, &body, Some(&sign(SECRET, body.as_bytes()))).await, StatusCode::OK);
        let items = store::list_items(&pool, "", None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].source.as_str(), items[0].body.as_str()), ("line", "電腦壞了"));
        assert_eq!(changes.load(Ordering::SeqCst), 1);
        // LINE 重送同一則：仍回 200，但不重複入列、也不再通知。
        assert_eq!(post_hook(&r, &body, Some(&sign(SECRET, body.as_bytes()))).await, StatusCode::OK);
        assert_eq!(store::list_items(&pool, "", None).await.unwrap().len(), 1);
        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn the_verify_button_and_unknown_senders_get_200_but_queue_nothing() {
        let (r, pool, _) = app().await;
        let empty = r#"{"events":[]}"#;
        assert_eq!(post_hook(&r, empty, Some(&sign(SECRET, empty.as_bytes()))).await, StatusCode::OK);
        let stranger = text_event("2", "hi").replace(UID, "Uffffffffffffffffffffffffffffffff");
        assert_eq!(post_hook(&r, &stranger, Some(&sign(SECRET, stranger.as_bytes()))).await, StatusCode::OK);
        assert!(store::list_items(&pool, "", None).await.unwrap().is_empty());
        assert_eq!(store::dropped_counts(&pool, "").await.unwrap(), vec![("not_allowed".to_string(), 1)]);
    }

    #[tokio::test]
    async fn oversized_bodies_are_refused_before_any_processing() {
        let (r, pool, _) = app().await;
        let big = "x".repeat(MAX_BODY + 1);
        let status = post_hook(&r, &big, Some(&sign(SECRET, big.as_bytes()))).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(store::list_items(&pool, "", None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn other_paths_and_methods_are_not_found() {
        let (r, _, _) = app().await;
        let resp = r.clone().oneshot(Request::get(WEBHOOK_PATH).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        let resp = r.oneshot(Request::post("/").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn push_sends_bearer_token_target_and_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/bot/message/push"))
            .and(header("authorization", "Bearer tok"))
            .and(body_partial_json(serde_json::json!({ "to": UID, "messages": [{ "type": "text", "text": "請先重開機" }] })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;
        push_to(&server.uri(), "tok", UID, "請先重開機").await.unwrap();
    }

    #[tokio::test]
    async fn a_long_reply_is_split_and_sent_in_batches_of_five() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v2/bot/message/push")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({}))).expect(2).mount(&server).await;
        // 7 段（每行剛好一段）→ 5 + 2 兩次請求。
        let text = (0..7).map(|_| "a".repeat(SEGMENT_CHARS - 100)).collect::<Vec<_>>().join("\n");
        push_to(&server.uri(), "tok", UID, &text).await.unwrap();
        let reqs = server.received_requests().await.unwrap();
        let counts: Vec<usize> = reqs.iter().map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap()["messages"].as_array().unwrap().len()).collect();
        assert_eq!(counts, vec![5, 2]);
    }

    #[tokio::test]
    async fn quota_exhaustion_and_other_failures_surface_as_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({ "message": "You have reached your monthly limit." }))).mount(&server).await;
        let err = push_to(&server.uri(), "tok", UID, "hi").await.unwrap_err();
        assert!(err.contains("429") && err.contains("額度") && err.contains("monthly limit"), "{err}");
        for blank in ["", "  \n "] {
            let empty = push_to(&server.uri(), "tok", UID, blank).await.unwrap_err();
            assert!(empty.contains("空"), "{empty}");
        }
    }

    // ───────── 群組 ─────────

    const GROUP: &str = "Cb345e5d492c48c02a3eaa279924c52d2";
    const MEMBER: &str = "U2222222222222222222222222222222b";

    fn group_text(id: &str, text: &str, mention_self: bool) -> String {
        let mention = if mention_self { r#","mention":{"mentionees":[{"index":0,"length":3,"type":"user","isSelf":true}]}"# } else { "" };
        let now_ms = chrono::Utc::now().timestamp_millis();
        format!(r#"{{"events":[{{"type":"message","timestamp":{now_ms},"source":{{"type":"group","groupId":"{GROUP}","userId":"{MEMBER}"}},"message":{{"type":"text","id":"{id}","text":"{text}"{mention}}}}}]}}"#)
    }
    async fn signed(r: &Router, body: &str) -> StatusCode {
        post_hook(r, body, Some(&sign(SECRET, body.as_bytes()))).await
    }
    async fn allow_group(pool: &SqlitePool) {
        store::upsert_sender(pool, "", &Sender { address: format!("line:{GROUP}"), level: Level::External, skip_auth: false }).await.unwrap();
    }
    fn gkey() -> String {
        format!("line:{}", GROUP.to_lowercase())
    }

    #[tokio::test]
    async fn a_group_that_is_not_whitelisted_is_only_noted_by_id_and_nothing_is_stored() {
        let (r, pool, _) = app().await;
        assert_eq!(signed(&r, &group_text("1", "機密的內部討論", true)).await, StatusCode::OK);
        assert_eq!(line_store::message_count(&pool, &gkey()).await.unwrap(), 0, "白名單外的群組不存訊息");
        assert!(store::list_items(&pool, "", None).await.unwrap().is_empty());
        let unknown = line_store::list_unknown(&pool).await.unwrap();
        assert_eq!((unknown.len(), unknown[0].id.as_str(), unknown[0].kind.as_str(), unknown[0].count), (1, GROUP.to_lowercase().as_str(), "group", 1));
        signed(&r, &group_text("2", "又一則", false)).await;
        assert_eq!(line_store::list_unknown(&pool).await.unwrap()[0].count, 2);
        // 被拉進群組（join）也會記下，但不會重複建立。
        let join = format!(r#"{{"events":[{{"type":"join","source":{{"type":"group","groupId":"{GROUP}"}}}}]}}"#);
        assert_eq!(signed(&r, &join).await, StatusCode::OK);
        assert_eq!(line_store::list_unknown(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_whitelisted_group_is_recorded_without_opening_and_a_mention_opens_an_item() {
        let (r, pool, changes) = app().await;
        allow_group(&pool).await;
        assert_eq!(signed(&r, &group_text("1", "電腦開不了機", false)).await, StatusCode::OK);
        assert_eq!(line_store::message_count(&pool, &gkey()).await.unwrap(), 1);
        assert!(store::list_items(&pool, "", None).await.unwrap().is_empty(), "沒 @ 不會直接開題");
        assert_eq!(changes.load(Ordering::SeqCst), 1, "存了訊息也要通知畫面");
        assert!(line_store::list_unknown(&pool).await.unwrap().is_empty());
        assert_eq!(signed(&r, &group_text("2", "@AI 請看一下", true)).await, StatusCode::OK);
        let items = store::list_items(&pool, "", None).await.unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].body.contains("電腦開不了機") && items[0].body.contains("請看一下"), "{}", items[0].body);
        // 重送：仍 200、不重複。
        assert_eq!(signed(&r, &group_text("2", "@AI 請看一下", true)).await, StatusCode::OK);
        assert_eq!(store::list_items(&pool, "", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retention_zero_keeps_no_group_messages_and_old_ones_are_purged() {
        let (r, pool, _) = app().await;
        allow_group(&pool).await;
        // 一則很舊的訊息（ts=1），保存期限 30 天，下次 webhook 處理後就被清掉。
        line_store::insert_message(&pool, &line_store::NewMessage { chat: &gkey(), msg_id: "ancient", user_id: "u", name: "x", text: "古早", ts: 1, outgoing: false }).await.unwrap();
        signed(&r, &group_text("1", "新的", false)).await;
        assert!(line_store::get_by_msg_id(&pool, &gkey(), "ancient").await.unwrap().is_none());
        assert!(line_store::get_by_msg_id(&pool, &gkey(), "1").await.unwrap().is_some(), "新的留著");
        // 保存天數 0：根本不存。
        let mut g = crate::intake::config::load_global(&pool).await;
        g.line_retention_days = 0;
        crate::intake::config::save_global(&pool, &g).await.unwrap();
        signed(&r, &group_text("2", "不該被存", false)).await;
        assert_eq!(line_store::message_count(&pool, &gkey()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn member_and_group_names_come_from_line_and_are_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path(format!("/v2/bot/group/{GROUP}/member/{MEMBER}"))).and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "displayName": "介琳" }))).expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path(format!("/v2/bot/group/{GROUP}/summary")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "groupName": "兆強客服群" }))).expect(1).mount(&server).await;
        let (r, pool, _) = app_with(Some("tok".into()), server.uri()).await;
        allow_group(&pool).await;
        assert_eq!(signed(&r, &group_text("1", "請看", false)).await, StatusCode::OK);
        assert_eq!(signed(&r, &group_text("2", "請看", true)).await, StatusCode::OK);
        let m = line_store::get_by_msg_id(&pool, &gkey(), "1").await.unwrap().unwrap();
        assert_eq!(m.name, "介琳");
        let it = &store::list_items(&pool, "", None).await.unwrap()[0];
        assert_eq!(it.sender_name.as_deref(), Some("兆強客服群 / 介琳"));
        assert!(it.body.contains("兆強客服群"), "{}", it.body);
        // wiremock 的 expect(1)：第二則沒有再查名稱（快取生效）。
    }

    #[tokio::test]
    async fn names_fall_back_to_a_short_id_when_line_cannot_be_asked() {
        let (r, pool, _) = app().await;
        allow_group(&pool).await;
        signed(&r, &group_text("1", "請看", false)).await;
        let m = line_store::get_by_msg_id(&pool, &gkey(), "1").await.unwrap().unwrap();
        assert_eq!(m.name, "成員222b");
    }
}

//! LINE webhook 的本機伺服器，與回覆用的 push API。
//! 預設只綁 127.0.0.1；要讓 LINE 連得到，由使用者自備 Cloudflare Tunnel／ngrok 指向這個埠。
//! 設計：docs/superpowers/specs/2026-11-23-intake-line-design.md
use super::line_source::{self, MAX_PER_REQUEST, SEGMENT_CHARS};
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
    let events = line_source::parse_events(&body);
    if events.is_empty() {
        return StatusCode::OK;
    }
    let cands = (st.candidates)().await;
    let mut queued = false;
    for ev in events {
        match line_source::route(&st.pool, &cands, &ev.user_id, &ev.message_id, &ev.text, &st.attach_root).await {
            Ok(Ingested::Queued(_)) => queued = true,
            Ok(_) => {}
            // 回非 2xx 會讓 LINE 重送，所以資料庫暫時出錯時回 500 是對的；重送由去重擋下。
            Err(e) => {
                log::warn!("intake: LINE ingest failed: {e}");
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
        }
    }
    if queued {
        (st.on_change)();
    }
    StatusCode::OK
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
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        store::upsert_sender(&pool, "", &Sender { address: format!("line:{UID}"), level: Level::External, skip_auth: false }).await.unwrap();
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = changes.clone();
        let st = Arc::new(LineState {
            pool: pool.clone(),
            secret: SECRET.into(),
            attach_root: std::env::temp_dir().join("aiterm-line-test"),
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
}

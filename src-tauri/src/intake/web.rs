//! 網頁表單：用電子郵件一次性驗證碼證明身分，才符合「來源必須在白名單」的規則。
//!
//! - `POST /api/code`：白名單內的信箱會收到 6 位數驗證碼。**不論信箱在不在白名單回應都一樣**，
//!   外人無法藉此探測白名單。
//! - `POST /api/submit`：信箱＋驗證碼＋問題。驗證碼 10 分鐘有效、單次使用、最多試 5 次。
use super::attach::{self, Raw};
use super::config::{Config, WebMode};
use super::queue::{self, Ingested};
use super::rules;
use super::store;
use parking_lot::Mutex;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;

pub const CODE_TTL_SECS: i64 = 600;
pub const MAX_ATTEMPTS: u32 = 5;
pub const MAX_CODE_REQUESTS: usize = 3;
pub const MAX_BODY_CHARS: usize = 20_000;
pub const MAX_SUBMIT_BYTES: usize = 16 * 1024 * 1024;

struct Entry {
    code: String,
    expires: i64,
    attempts: u32,
}

#[derive(Default)]
pub struct Otps {
    codes: Mutex<HashMap<String, Entry>>,
    /// 每個信箱最近的發碼時間，用來限制頻率（驗證成功也不清掉，否則可以靠重複成功繞過）。
    history: Mutex<HashMap<String, Vec<i64>>>,
}

impl Otps {
    /// 發一組新碼；10 分鐘內發太多次回 `None`。新碼會讓舊碼失效。
    pub fn issue(&self, email: &str, now: i64, code: String) -> Option<String> {
        {
            let mut h = self.history.lock();
            let v = h.entry(email.to_string()).or_default();
            v.retain(|t| now - *t < CODE_TTL_SECS);
            if v.len() >= MAX_CODE_REQUESTS {
                return None;
            }
            v.push(now);
        }
        self.codes.lock().insert(email.to_string(), Entry { code: code.clone(), expires: now + CODE_TTL_SECS, attempts: 0 });
        Some(code)
    }

    /// 驗證成功會消耗掉驗證碼；失敗累計次數，超過上限整組作廢。
    pub fn verify(&self, email: &str, code: &str, now: i64) -> bool {
        let mut m = self.codes.lock();
        let Some(e) = m.get_mut(email) else { return false };
        if now > e.expires {
            m.remove(email);
            return false;
        }
        e.attempts += 1;
        if e.code == code.trim() {
            m.remove(email);
            return true;
        }
        if e.attempts >= MAX_ATTEMPTS {
            m.remove(email);
        }
        false
    }
}

pub fn random_code() -> String {
    let n = u32::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap());
    format!("{:06}", n % 1_000_000)
}

/// 發碼的前置檢查。回傳要寄出的 `(收件地址, 驗證碼)`；不在白名單或被限流回 `None`
/// （呼叫端對外一律回同樣的成功訊息）。
pub async fn prepare_code(pool: &SqlitePool, cfg: &Config, otps: &Otps, email: &str, now: i64) -> Result<Option<(String, String)>, sqlx::Error> {
    if cfg.web_mode == WebMode::Off || !rules::valid_address(email) || email.trim().starts_with("tg:") {
        return Ok(None);
    }
    let addr = rules::normalize(email);
    let senders = store::list_senders(pool).await?;
    if !senders.iter().any(|s| rules::normalize(&s.address) == addr) {
        store::bump_dropped(pool, "web_not_allowed").await?;
        return Ok(None);
    }
    Ok(otps.issue(&addr, now, random_code()).map(|c| (addr, c)))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Submit {
    Accepted,
    BadCode,
    Invalid(&'static str),
    Disabled,
}

#[allow(clippy::too_many_arguments)]
pub async fn submit(pool: &SqlitePool, cfg: &Config, otps: &Otps, now: i64, email: &str, code: &str, subject: &str, body: &str, attach_root: &Path, images: Vec<Raw>) -> Result<Submit, sqlx::Error> {
    if cfg.web_mode == WebMode::Off {
        return Ok(Submit::Disabled);
    }
    let body = body.trim();
    if body.is_empty() && images.is_empty() {
        return Ok(Submit::Invalid("內容不可為空"));
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Ok(Submit::Invalid("內容太長"));
    }
    let addr = rules::normalize(email);
    // 先驗證碼、後查白名單：兩者任一不符都回 BadCode，不洩漏白名單。
    let senders = store::list_senders(pool).await?;
    let Some(sender) = senders.iter().find(|s| rules::normalize(&s.address) == addr) else { return Ok(Submit::BadCode) };
    if !otps.verify(&addr, code, now) {
        return Ok(Submit::BadCode);
    }
    let subject: String = subject.trim().chars().take(200).collect();
    let mid = format!("web:{}", uuid::Uuid::new_v4());
    let body = if body.is_empty() { "（僅附圖）" } else { body };
    let r = queue::enqueue(pool, cfg, sender.level, "web", &addr, &subject, body, &mid, attach_root, images).await?;
    Ok(match r {
        Ingested::Queued(_) | Ingested::Duplicate => Submit::Accepted,
        Ingested::Dropped("rate_limited") => Submit::Invalid("送出太頻繁，請稍後再試"),
        Ingested::Dropped(_) => Submit::Invalid("目前無法接收"),
    })
}

pub const PAGE: &str = r#"<!doctype html><html lang="zh-Hant"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>提問</title>
<style>body{font:16px system-ui;max-width:560px;margin:2rem auto;padding:0 1rem}label{display:block;margin:.8rem 0 .2rem}
input,textarea,button{font:inherit;width:100%;box-sizing:border-box;padding:.5rem}button{margin-top:1rem;cursor:pointer}
#msg{margin-top:1rem;min-height:1.5em}.ok{color:#15803d}.err{color:#b91c1c}[hidden]{display:none}</style></head><body>
<h1>提問</h1>
<form id="f1"><label for="email">你的電子郵件（必須是已授權的信箱）</label>
<input id="email" type="email" required autocomplete="email"><button>寄送驗證碼</button></form>
<form id="f2" hidden><label for="code">收到的 6 位數驗證碼</label><input id="code" inputmode="numeric" maxlength="6" required>
<label for="subject">主旨</label><input id="subject" maxlength="200">
<label for="body">問題內容</label><textarea id="body" rows="8"></textarea>
<label for="imgs">附圖（選填，最多 5 張，每張 10MB 內）</label><input id="imgs" type="file" accept="image/png,image/jpeg,image/gif,image/webp" multiple><button>送出</button></form>
<div id="msg" role="status"></div>
<script>
const $=id=>document.getElementById(id),m=(t,c)=>{$("msg").textContent=t;$("msg").className=c||""};
async function post(u,d){const r=await fetch(u,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(d)});
let j={};try{j=await r.json()}catch(e){}return{ok:r.ok,j}}
$("f1").onsubmit=async e=>{e.preventDefault();const r=await post("/api/code",{email:$("email").value});
m(r.ok?"若此信箱已獲授權，驗證碼已寄出（10 分鐘內有效）。":(r.j.error||"失敗"),r.ok?"ok":"err");if(r.ok)$("f2").hidden=false};
const b64=f=>new Promise((ok,no)=>{const r=new FileReader();r.onload=()=>ok(String(r.result).split(",")[1]||"");r.onerror=no;r.readAsDataURL(f)});
$("f2").onsubmit=async e=>{e.preventDefault();const fs=[...$("imgs").files].slice(0,5);
const images=await Promise.all(fs.map(async f=>({name:f.name,data:await b64(f)})));
const r=await post("/api/submit",{email:$("email").value,code:$("code").value,subject:$("subject").value,body:$("body").value,images});
m(r.ok?"已收到，處理完成後會回信給你。":(r.j.error||"失敗"),r.ok?"ok":"err");if(r.ok){$("f2").reset()}};
</script></body></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::rules::{Level, Sender};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        store::upsert_sender(&p, &Sender { address: "bob@corp.com".into(), level: Level::External, skip_auth: false }).await.unwrap();
        p
    }
    fn cfg() -> Config {
        Config { web_mode: WebMode::Local, ..Config::default() }
    }

    #[test]
    fn code_is_single_use_expires_and_locks_after_too_many_guesses() {
        let o = Otps::default();
        o.issue("a@b.co", 0, "111111".into()).unwrap();
        assert!(!o.verify("a@b.co", "000000", 1));
        assert!(o.verify("a@b.co", "111111", 2));
        assert!(!o.verify("a@b.co", "111111", 3), "單次使用");

        o.issue("c@d.co", 0, "222222".into()).unwrap();
        assert!(!o.verify("c@d.co", "222222", CODE_TTL_SECS + 1), "過期");

        let o = Otps::default();
        o.issue("e@f.co", 0, "333333".into()).unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert!(!o.verify("e@f.co", "999999", 1));
        }
        assert!(!o.verify("e@f.co", "333333", 2), "猜錯 5 次後整組作廢，即使這次對了");
    }

    #[test]
    fn issuing_is_rate_limited_per_address_and_a_new_code_replaces_the_old() {
        let o = Otps::default();
        for i in 0..MAX_CODE_REQUESTS {
            assert!(o.issue("a@b.co", i as i64, format!("{i:06}")).is_some());
        }
        assert!(o.issue("a@b.co", 10, "999999".into()).is_none());
        assert!(o.issue("other@b.co", 10, "888888".into()).is_some(), "別人不受影響");
        assert!(o.issue("a@b.co", CODE_TTL_SECS + 5, "777777".into()).is_some(), "視窗過後恢復");
        assert!(!o.verify("a@b.co", "000002", CODE_TTL_SECS + 6), "舊碼失效");
        assert!(o.verify("a@b.co", "777777", CODE_TTL_SECS + 6));
    }

    #[test]
    fn random_codes_are_six_digits() {
        for _ in 0..50 {
            let c = random_code();
            assert!(c.len() == 6 && c.chars().all(|x| x.is_ascii_digit()), "{c}");
        }
    }

    #[tokio::test]
    async fn only_allowlisted_addresses_get_a_code_and_strangers_are_counted() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let got = prepare_code(&p, &cfg(), &o, "Bob <BOB@corp.com>", 0).await.unwrap().unwrap();
        assert_eq!(got.0, "bob@corp.com");
        assert!(prepare_code(&p, &cfg(), &o, "evil@x.com", 0).await.unwrap().is_none());
        assert!(prepare_code(&p, &cfg(), &o, "tg:123", 0).await.unwrap().is_none());
        assert_eq!(store::dropped_counts(&p).await.unwrap(), vec![("web_not_allowed".to_string(), 1)]);
        let off = Config { web_mode: WebMode::Off, ..Config::default() };
        assert!(prepare_code(&p, &off, &o, "bob@corp.com", 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn submit_needs_the_right_code_and_lands_in_the_queue_with_the_senders_level() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        assert_eq!(submit(&p, &cfg(), &o, 1, &addr, "000000", "s", "內容", tmp.path(), vec![]).await.unwrap(), Submit::BadCode);
        assert!(store::list_items(&p, None).await.unwrap().is_empty());
        assert_eq!(submit(&p, &cfg(), &o, 2, &addr, &code, "壞掉了", "請修", tmp.path(), vec![]).await.unwrap(), Submit::Accepted);
        let it = &store::list_items(&p, None).await.unwrap()[0];
        assert_eq!((it.source.as_str(), it.sender.as_str(), it.sender_level.as_str(), it.status.as_str()), ("web", "bob@corp.com", "external", store::PENDING));
        assert_eq!(submit(&p, &cfg(), &o, 3, &addr, &code, "s", "again", tmp.path(), vec![]).await.unwrap(), Submit::BadCode, "同一組碼不能再用");
    }

    #[tokio::test]
    async fn submit_rejects_empty_oversized_and_unlisted_without_leaking_which() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        assert!(matches!(submit(&p, &cfg(), &o, 1, &addr, &code, "s", "   ", tmp.path(), vec![]).await.unwrap(), Submit::Invalid(_)));
        let big = "字".repeat(MAX_BODY_CHARS + 1);
        assert!(matches!(submit(&p, &cfg(), &o, 1, &addr, &code, "s", &big, tmp.path(), vec![]).await.unwrap(), Submit::Invalid(_)));
        // 內容無效時不應該消耗驗證碼
        assert_eq!(submit(&p, &cfg(), &o, 2, &addr, &code, "s", "ok", tmp.path(), vec![]).await.unwrap(), Submit::Accepted);
        assert_eq!(submit(&p, &cfg(), &o, 2, "evil@x.com", "123456", "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::BadCode);
        let off = Config { web_mode: WebMode::Off, ..Config::default() };
        assert_eq!(submit(&p, &off, &o, 2, &addr, &code, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::Disabled);
    }

    #[test]
    fn page_posts_json_to_the_two_endpoints() {
        assert!(PAGE.contains("/api/code") && PAGE.contains("/api/submit") && PAGE.contains("application/json"));
    }
}

// ---- HTTP 伺服器 ----

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager};


/// 伺服器需要的外部能力都從這裡注入：測試不必起一個完整的 Tauri 應用。
pub struct WebState {
    pub pool: SqlitePool,
    pub otps: Otps,
    /// 附圖存放的根目錄。
    pub attach_root: std::path::PathBuf,
    /// 佇列有新項目時通知前端。
    pub on_change: Box<dyn Fn() + Send + Sync>,
    /// 寄驗證碼：`(收件地址, 主旨, 內文)`，在背景執行。
    pub send_mail: Box<dyn Fn(String, String, String) + Send + Sync>,
}

#[derive(Default)]
pub struct WebServer {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

#[derive(Deserialize)]
struct CodeReq {
    email: String,
}

#[derive(Deserialize)]
struct SubmitReq {
    email: String,
    code: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
    /// 圖片以 base64 傳送；格式與大小由 `attach` 依內容驗證。
    #[serde(default)]
    images: Vec<ImageReq>,
}

#[derive(Deserialize)]
struct ImageReq {
    #[serde(default)]
    name: String,
    data: String,
}

/// base64 解不開的圖片留成空內容，之後會被當成「略過」計數，而不是讓整個請求失敗。
fn decode_images(images: Vec<ImageReq>) -> Vec<Raw> {
    use base64::Engine;
    images
        .into_iter()
        .take(attach::MAX_FILES + 5)
        .map(|i| Raw { name: i.name, bytes: base64::engine::general_purpose::STANDARD.decode(i.data.trim()).unwrap_or_default() })
        .collect()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn json(status: StatusCode, v: serde_json::Value) -> impl IntoResponse {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(v))
}

async fn page() -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store"), (header::X_FRAME_OPTIONS, "DENY")], Html(PAGE))
}

async fn api_code(State(st): State<Arc<WebState>>, Json(req): Json<CodeReq>) -> impl IntoResponse {
    let cfg = super::config::load(&st.pool).await;
    if let Ok(Some((to, code))) = prepare_code(&st.pool, &cfg, &st.otps, &req.email, now()).await {
        // 寄信放背景：回應時間不能洩漏「這個信箱有沒有在白名單」。
        let body = format!("你的驗證碼是 {code}，10 分鐘內有效。如果不是你本人操作，請忽略這封信。");
        (st.send_mail)(to, "提問驗證碼".into(), body);
    }
    json(StatusCode::OK, serde_json::json!({ "ok": true }))
}

async fn api_submit(State(st): State<Arc<WebState>>, Json(req): Json<SubmitReq>) -> impl IntoResponse {
    let cfg = super::config::load(&st.pool).await;
    match submit(&st.pool, &cfg, &st.otps, now(), &req.email, &req.code, &req.subject, &req.body, &st.attach_root, decode_images(req.images)).await {
        Ok(Submit::Accepted) => {
            (st.on_change)();
            json(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Ok(Submit::BadCode) => json(StatusCode::FORBIDDEN, serde_json::json!({ "error": "驗證碼錯誤或已過期" })),
        Ok(Submit::Invalid(m)) => json(StatusCode::BAD_REQUEST, serde_json::json!({ "error": m })),
        Ok(Submit::Disabled) => json(StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({ "error": "目前未開放" })),
        Err(e) => {
            log::warn!("intake web: submit failed: {e}");
            json(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({ "error": "伺服器錯誤" }))
        }
    }
}

pub fn router(st: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(page))
        // 要驗證碼的請求只有一個信箱，小上限；送出問題可能帶 5 張圖（base64 約 13MB）。
        .route("/api/code", post(api_code).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/api/submit", post(api_submit).layer(DefaultBodyLimit::max(MAX_SUBMIT_BYTES)))
        .with_state(st)
}

/// 依目前設定（重新）啟動網頁表單伺服器。
pub async fn restart(app: &AppHandle) {
    let Some(server) = app.try_state::<Arc<WebServer>>() else { return };
    if let Some(h) = server.handle.lock().take() {
        h.abort();
    }
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    let cfg = super::config::load(&db.pool).await;
    let host = match cfg.web_mode {
        WebMode::Off => return,
        WebMode::Local => [127, 0, 0, 1],
        WebMode::Lan => [0, 0, 0, 0],
    };
    let addr = std::net::SocketAddr::from((host, cfg.web_port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("intake web: 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (a1, a2) = (app.clone(), app.clone());
    let st = Arc::new(WebState {
        pool: db.pool.clone(),
        otps: Otps::default(),
        attach_root: attach::root(),
        on_change: Box::new(move || {
            use tauri::Emitter;
            let _ = a1.emit(super::CHANGED_EVENT, ());
        }),
        send_mail: Box::new(move |to, subject, body| {
            let app = a2.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = super::runtime::send_plain_mail(&app, &to, &subject, &body).await {
                    log::warn!("intake web: 寄驗證碼失敗：{e}");
                }
            });
        }),
    });
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router(st)).await {
            log::warn!("intake web: 伺服器結束：{e}");
        }
    });
    *server.handle.lock() = Some(task);
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::intake::rules::{Level, Sender};
    use axum::body::Body;
    use axum::http::Request;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

    type Sent = Arc<Mutex<Vec<(String, String, String)>>>;

    async fn app(mode: WebMode) -> (Router, SqlitePool, Sent, Arc<std::sync::atomic::AtomicUsize>) {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&pool).await.unwrap();
        store::upsert_sender(&pool, &Sender { address: "bob@corp.com".into(), level: Level::External, skip_auth: false }).await.unwrap();
        super::super::config::save(&pool, &Config { web_mode: mode, ..Config::default() }).await.unwrap();
        let sent: Sent = Arc::default();
        let (s2, changes) = (sent.clone(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let c2 = changes.clone();
        let root2 = std::env::temp_dir().join(format!("intake-web-test-{}", uuid::Uuid::new_v4()));
        let st = Arc::new(WebState {
            pool: pool.clone(),
            otps: Otps::default(),
            attach_root: root2,
            on_change: Box::new(move || { c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }),
            send_mail: Box::new(move |to, sub, body| s2.lock().push((to, sub, body))),
        });
        (router(st), pool, sent, changes)
    }

    async fn post(r: &Router, path: &str, body: serde_json::Value) -> (StatusCode, String) {
        let resp = r
            .clone()
            .oneshot(Request::post(path).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn full_flow_code_by_mail_then_submit() {
        let (r, pool, sent, changes) = app(WebMode::Local).await;
        let (st, _) = post(&r, "/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        assert_eq!(st, StatusCode::OK);
        let (to, _, body) = sent.lock()[0].clone();
        assert_eq!(to, "bob@corp.com");
        // 內文是「你的驗證碼是 123456，10 分鐘內有效…」：取第一段連續 6 位數字。
        let chars: Vec<char> = body.chars().collect();
        let code: String = chars.windows(6).find(|w| w.iter().all(|c| c.is_ascii_digit())).unwrap().iter().collect();

        let (st, _) = post(&r, "/api/submit", serde_json::json!({"email":"bob@corp.com","code":"000000","body":"hi"})).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = post(&r, "/api/submit", serde_json::json!({"email":"bob@corp.com","code":code,"subject":"s","body":"請修"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(store::list_items(&pool, None).await.unwrap().len(), 1);
        assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn code_endpoint_answers_identically_for_listed_and_unlisted_addresses() {
        let (r, _, sent, _) = app(WebMode::Local).await;
        let a = post(&r, "/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        let b = post(&r, "/api/code", serde_json::json!({"email": "stranger@x.com"})).await;
        assert_eq!(a, b, "狀態碼與內容必須一樣，否則可以拿來探測白名單");
        assert_eq!(sent.lock().len(), 1, "只有白名單內的信箱真的收到信");
    }

    #[tokio::test]
    async fn disabled_server_refuses_submissions_and_sends_no_codes() {
        let (r, _, sent, _) = app(WebMode::Off).await;
        post(&r, "/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        assert!(sent.lock().is_empty());
        let (st, _) = post(&r, "/api/submit", serde_json::json!({"email":"bob@corp.com","code":"123456","body":"x"})).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn oversized_request_bodies_are_rejected_before_parsing() {
        let (r, pool, _, _) = app(WebMode::Local).await;
        let huge = "x".repeat(MAX_SUBMIT_BYTES + 1024);
        let (st, _) = post(&r, "/api/submit", serde_json::json!({"email":"bob@corp.com","code":"123456","body":huge})).await;
        assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(store::list_items(&pool, None).await.unwrap().is_empty());
        // 要驗證碼的端點只該收到一個信箱，上限小得多。
        let (st, _) = post(&r, "/api/code", serde_json::json!({"email":"bob@corp.com","pad":"x".repeat(8 * 1024)})).await;
        assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn images_in_a_submission_are_saved_and_attached() {
        use base64::Engine;
        let (r, pool, sent, _) = app(WebMode::Local).await;
        post(&r, "/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        let body = sent.lock()[0].2.clone();
        let chars: Vec<char> = body.chars().collect();
        let code: String = chars.windows(6).find(|w| w.iter().all(|c| c.is_ascii_digit())).unwrap().iter().collect();
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 7, 7];
        let ok = base64::engine::general_purpose::STANDARD.encode(png);
        let (st, _) = post(&r, "/api/submit", serde_json::json!({
            "email":"bob@corp.com","code":code,"subject":"s","body":"",
            "images":[{"name":"畫面.png","data":ok},{"name":"bad.png","data":"!!!not base64!!!"},{"name":"x.txt","data":base64::engine::general_purpose::STANDARD.encode(b"hello")}]
        })).await;
        assert_eq!(st, StatusCode::OK);
        let it = &store::list_items(&pool, None).await.unwrap()[0];
        let atts = store::list_attachments(&pool, &it.id).await.unwrap();
        assert_eq!(atts.len(), 1);
        assert!(it.body.contains("僅附圖") && it.body.contains("2 個附件"), "{}", it.body);
    }

    #[tokio::test]
    async fn page_is_served_with_no_store_and_frame_protection() {
        let (r, _, _, _) = app(WebMode::Local).await;
        let resp = r.clone().oneshot(Request::get("/").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(resp.headers()[header::X_FRAME_OPTIONS], "DENY");
    }
}


//! 網頁表單：用電子郵件一次性驗證碼證明身分，才符合「來源必須在白名單」的規則。
//!
//! - `POST /api/code`：白名單內的信箱會收到 6 位數驗證碼。**不論信箱在不在白名單回應都一樣**，
//!   外人無法藉此探測白名單。
//! - `POST /api/login`：信箱＋驗證碼換一個工作階段 token。驗證碼 10 分鐘有效、單次使用、最多試 5 次。
//! - `POST /api/submit`：token＋問題。token 只存在頁面記憶體，閒置 30 分鐘失效，可連續提交多題。
//! - `POST /api/logout`：作廢 token。
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
/// 驗證通過後的工作階段閒置多久失效；每次成功提交都會重新計時。
pub const SESSION_TTL_SECS: i64 = 1800;
const MAX_SESSIONS: usize = 500;
pub const MAX_ATTEMPTS: u32 = 5;
pub const MAX_CODE_REQUESTS: usize = 3;
pub const MAX_BODY_CHARS: usize = 20_000;
pub const MAX_SUBMIT_BYTES: usize = 48 * 1024 * 1024;

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
    /// 驗證通過後的工作階段：token → (專案, 信箱, 到期時間)。只放記憶體，重啟就全部作廢。
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    project_id: String,
    addr: String,
    expires: i64,
    /// 已上傳、還沒隨題目送出的附件（只放記憶體，登出或過期就丟掉）。
    staged: Vec<(String, Raw)>,
}

/// 一個工作階段暫存的附件總量上限（與頁面上標示的「合計約 35MB」一致）。
pub const MAX_STAGED_BYTES: usize = 35 * 1024 * 1024;

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

    /// 驗證碼正確才開一個工作階段，回傳 token。
    pub fn login(&self, project_id: &str, addr: &str, code: &str, now: i64) -> Option<String> {
        if !self.verify(&otp_key(project_id, addr), code, now) {
            return None;
        }
        let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let mut m = self.sessions.lock();
        m.retain(|_, s| s.expires > now);
        if m.len() >= MAX_SESSIONS {
            return None;
        }
        m.insert(token.clone(), Session { project_id: project_id.to_string(), addr: addr.to_string(), expires: now + SESSION_TTL_SECS, staged: vec![] });
        Some(token)
    }

    /// token 屬於這個專案且沒過期才回信箱；通過就順延到期時間（閒置計時）。
    pub fn touch(&self, token: &str, project_id: &str, now: i64) -> Option<String> {
        let mut m = self.sessions.lock();
        let s = m.get_mut(token)?;
        if s.project_id != project_id || s.expires <= now {
            if s.expires <= now {
                m.remove(token);
            }
            return None;
        }
        s.expires = now + SESSION_TTL_SECS;
        Some(s.addr.clone())
    }

    /// 同 `touch`，但不延長到期時間：給輪詢進度用，免得開著頁面就永遠不過期。
    pub fn peek(&self, token: &str, project_id: &str, now: i64) -> Option<String> {
        let m = self.sessions.lock();
        let s = m.get(token)?;
        (s.project_id == project_id && s.expires > now).then(|| s.addr.clone())
    }

    /// 暫存一個附件，回傳它的 id。超過個數、單檔或總量上限、空檔都拒絕。
    pub fn stage(&self, token: &str, project_id: &str, now: i64, name: &str, bytes: Vec<u8>) -> Result<String, &'static str> {
        let mut m = self.sessions.lock();
        let s = m.get_mut(token).filter(|s| s.project_id == project_id && s.expires > now).ok_or("session")?;
        if bytes.is_empty() {
            return Err("empty");
        }
        if bytes.len() > attach::MAX_BYTES {
            return Err("too_big");
        }
        if s.staged.len() >= attach::MAX_FILES || s.staged.iter().map(|(_, r)| r.bytes.len()).sum::<usize>() + bytes.len() > MAX_STAGED_BYTES {
            return Err("too_many");
        }
        s.expires = now + SESSION_TTL_SECS;
        let id = uuid::Uuid::new_v4().simple().to_string();
        s.staged.push((id.clone(), Raw { name: name.chars().take(200).collect(), bytes }));
        Ok(id)
    }

    pub fn unstage(&self, token: &str, id: &str) {
        if let Some(s) = self.sessions.lock().get_mut(token) {
            s.staged.retain(|(i, _)| i != id);
        }
    }

    /// 取出（複製）要隨題目送出的暫存附件；找不到的 id 略過。
    pub fn staged_for(&self, token: &str, ids: &[String]) -> Vec<Raw> {
        let m = self.sessions.lock();
        let Some(s) = m.get(token) else { return vec![] };
        ids.iter().filter_map(|id| s.staged.iter().find(|(i, _)| i == id).map(|(_, r)| Raw { name: r.name.clone(), bytes: r.bytes.clone() })).collect()
    }

    pub fn clear_staged(&self, token: &str, ids: &[String]) {
        if let Some(s) = self.sessions.lock().get_mut(token) {
            s.staged.retain(|(i, _)| !ids.contains(i));
        }
    }

    pub fn logout(&self, token: &str) {
        self.sessions.lock().remove(token);
    }
}

fn otp_key(project_id: &str, addr: &str) -> String {
    format!("{project_id}|{addr}")
}

pub fn random_code() -> String {
    let n = u32::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap());
    format!("{:06}", n % 1_000_000)
}

/// 發碼的前置檢查。回傳要寄出的 `(收件地址, 驗證碼)`；不在白名單或被限流回 `None`
/// （呼叫端對外一律回同樣的成功訊息）。
pub async fn prepare_code(pool: &SqlitePool, project_id: &str, cfg: &Config, otps: &Otps, email: &str, now: i64) -> Result<Option<(String, String)>, sqlx::Error> {
    if cfg.web_mode == WebMode::Off || !rules::valid_address(email) || email.trim().starts_with("tg:") {
        return Ok(None);
    }
    let addr = rules::normalize(email);
    let senders = store::list_senders(pool, project_id).await?;
    if !senders.iter().any(|s| rules::normalize(&s.address) == addr) {
        store::bump_dropped(pool, project_id, "web_not_allowed").await?;
        return Ok(None);
    }
    // 驗證碼綁定專案：同一個信箱在另一個專案拿到的碼不能拿來投這個專案。
    Ok(otps.issue(&otp_key(project_id, &addr), now, random_code()).map(|c| (addr, c)))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Submit {
    Accepted,
    /// 沒有有效的工作階段（沒登入、過期、被登出，或信箱已不在白名單）。
    NoSession,
    Invalid(&'static str),
    Disabled,
}

/// 信箱＋驗證碼換 token。不在白名單或碼不對一律回 `None`，不洩漏是哪一個。
pub async fn login(pool: &SqlitePool, project_id: &str, cfg: &Config, otps: &Otps, now: i64, email: &str, code: &str) -> Result<Option<(String, String)>, sqlx::Error> {
    if cfg.web_mode == WebMode::Off || !rules::valid_address(email) {
        return Ok(None);
    }
    let addr = rules::normalize(email);
    let senders = store::list_senders(pool, project_id).await?;
    if !senders.iter().any(|s| rules::normalize(&s.address) == addr) {
        return Ok(None);
    }
    Ok(otps.login(project_id, &addr, code, now).map(|t| (t, addr)))
}

#[allow(clippy::too_many_arguments)]
pub async fn submit(pool: &SqlitePool, project_id: &str, cfg: &Config, otps: &Otps, now: i64, token: &str, subject: &str, body: &str, attach_root: &Path, images: Vec<Raw>) -> Result<Submit, sqlx::Error> {
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
    let Some(addr) = otps.touch(token, project_id, now) else { return Ok(Submit::NoSession) };
    // 每次提交都重查白名單：信箱被移除後，已開的工作階段立刻失效。
    let senders = store::list_senders(pool, project_id).await?;
    let Some(sender) = senders.iter().find(|s| rules::normalize(&s.address) == addr) else {
        otps.logout(token);
        return Ok(Submit::NoSession);
    };
    let subject: String = subject.trim().chars().take(200).collect();
    let mid = format!("web:{}", uuid::Uuid::new_v4());
    let body = if body.is_empty() { "（僅附件）" } else { body };
    let r = queue::enqueue(pool, cfg, project_id, sender.level, "web", &addr, &subject, body, &mid, attach_root, images).await?;
    Ok(match r {
        Ingested::Queued(_) | Ingested::Duplicate => Submit::Accepted,
        Ingested::Dropped("rate_limited") => Submit::Invalid("送出太頻繁，請稍後再試"),
        Ingested::Dropped(_) => Submit::Invalid("目前無法接收"),
    })
}

pub const PAGE: &str = include_str!("web_page.html");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::rules::{Level, Sender};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        store::upsert_sender(&p, "", &Sender { address: "bob@corp.com".into(), level: Level::External, skip_auth: false }).await.unwrap();
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
        let got = prepare_code(&p, "", &cfg(), &o, "Bob <BOB@corp.com>", 0).await.unwrap().unwrap();
        assert_eq!(got.0, "bob@corp.com");
        assert!(prepare_code(&p, "", &cfg(), &o, "evil@x.com", 0).await.unwrap().is_none());
        assert!(prepare_code(&p, "", &cfg(), &o, "tg:123", 0).await.unwrap().is_none());
        assert_eq!(store::dropped_counts(&p, "").await.unwrap(), vec![("web_not_allowed".to_string(), 1)]);
        let off = Config { web_mode: WebMode::Off, ..Config::default() };
        assert!(prepare_code(&p, "", &off, &o, "bob@corp.com", 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn login_needs_the_right_code_then_one_session_submits_many_questions() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, "", &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        assert!(login(&p, "", &cfg(), &o, 1, &addr, "000000").await.unwrap().is_none());
        assert!(login(&p, "", &cfg(), &o, 1, "evil@x.com", &code).await.unwrap().is_none(), "不在白名單");
        let (token, who) = login(&p, "", &cfg(), &o, 2, &addr, &code).await.unwrap().unwrap();
        assert_eq!(who, "bob@corp.com");
        assert!(login(&p, "", &cfg(), &o, 3, &addr, &code).await.unwrap().is_none(), "同一組碼不能再用");
        for (i, body) in ["一", "二", "三"].iter().enumerate() {
            assert_eq!(submit(&p, "", &cfg(), &o, 10 + i as i64, &token, "s", body, tmp.path(), vec![]).await.unwrap(), Submit::Accepted);
        }
        let items = store::list_items(&p, "", None).await.unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|it| (it.source.as_str(), it.sender.as_str(), it.sender_level.as_str(), it.status.as_str()) == ("web", "bob@corp.com", "external", store::PENDING)));
    }

    #[tokio::test]
    async fn a_session_expires_after_idle_but_each_submission_extends_it() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, "", &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        let (token, _) = login(&p, "", &cfg(), &o, 0, &addr, &code).await.unwrap().unwrap();
        // 每次間隔 20 分鐘（< 30 分鐘閒置上限），累計遠超 30 分鐘仍有效。
        for i in 1..=3 {
            assert_eq!(submit(&p, "", &cfg(), &o, i * 1200, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::Accepted, "第 {i} 次");
        }
        assert_eq!(submit(&p, "", &cfg(), &o, 3 * 1200 + SESSION_TTL_SECS + 1, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::NoSession);
    }

    #[tokio::test]
    async fn a_session_is_bound_to_its_project_and_dies_on_logout_or_allowlist_removal() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, "", &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        let (token, _) = login(&p, "", &cfg(), &o, 1, &addr, &code).await.unwrap().unwrap();
        assert_eq!(submit(&p, "other", &cfg(), &o, 2, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::NoSession, "不能拿去投別的專案");
        assert_eq!(submit(&p, "", &cfg(), &o, 3, "亂猜的token", "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::NoSession);
        o.logout(&token);
        assert_eq!(submit(&p, "", &cfg(), &o, 4, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::NoSession, "登出後失效");

        let (addr, code) = prepare_code(&p, "", &cfg(), &o, "bob@corp.com", 5).await.unwrap().unwrap();
        let (token, _) = login(&p, "", &cfg(), &o, 6, &addr, &code).await.unwrap().unwrap();
        store::delete_sender(&p, "", "bob@corp.com").await.unwrap();
        assert_eq!(submit(&p, "", &cfg(), &o, 7, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::NoSession, "白名單移除後立刻失效");
        assert!(store::list_items(&p, "", None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn submit_rejects_empty_oversized_and_disabled() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let o = Otps::default();
        let (addr, code) = prepare_code(&p, "", &cfg(), &o, "bob@corp.com", 0).await.unwrap().unwrap();
        let (token, _) = login(&p, "", &cfg(), &o, 1, &addr, &code).await.unwrap().unwrap();
        assert!(matches!(submit(&p, "", &cfg(), &o, 1, &token, "s", "   ", tmp.path(), vec![]).await.unwrap(), Submit::Invalid(_)));
        let big = "字".repeat(MAX_BODY_CHARS + 1);
        assert!(matches!(submit(&p, "", &cfg(), &o, 1, &token, "s", &big, tmp.path(), vec![]).await.unwrap(), Submit::Invalid(_)));
        assert_eq!(submit(&p, "", &cfg(), &o, 2, &token, "s", "ok", tmp.path(), vec![]).await.unwrap(), Submit::Accepted, "內容無效不會弄壞工作階段");
        let off = Config { web_mode: WebMode::Off, ..Config::default() };
        assert_eq!(submit(&p, "", &off, &o, 2, &token, "s", "x", tmp.path(), vec![]).await.unwrap(), Submit::Disabled);
    }

    #[test]
    fn staged_files_obey_limits_belong_to_the_session_and_vanish_with_it() {
        let o = Otps::default();
        o.issue(&otp_key("p", "a@b.co"), 0, "111111".into()).unwrap();
        let t = o.login("p", "a@b.co", "111111", 0).unwrap();
        assert_eq!(o.stage("nope", "p", 1, "x", vec![1]), Err("session"));
        assert_eq!(o.stage(&t, "other", 1, "x", vec![1]), Err("session"), "別的專案不能用");
        assert_eq!(o.stage(&t, "p", 1, "x", vec![]), Err("empty"));
        assert_eq!(o.stage(&t, "p", 1, "x", vec![0; attach::MAX_BYTES + 1]), Err("too_big"));
        let id = o.stage(&t, "p", 1, "a.log", vec![1, 2, 3]).unwrap();
        assert_eq!(o.staged_for(&t, &[id.clone(), "不存在".into()]).len(), 1);
        o.unstage(&t, &id);
        assert!(o.staged_for(&t, &[id]).is_empty());
        for _ in 0..attach::MAX_FILES {
            o.stage(&t, "p", 1, "f", vec![1]).unwrap();
        }
        assert_eq!(o.stage(&t, "p", 1, "f", vec![1]), Err("too_many"), "個數上限");
        let o2 = Otps::default();
        o2.issue(&otp_key("p", "a@b.co"), 0, "111111".into()).unwrap();
        let t2 = o2.login("p", "a@b.co", "111111", 0).unwrap();
        for _ in 0..2 {
            o2.stage(&t2, "p", 1, "big", vec![0; attach::MAX_BYTES]).unwrap_or_default();
        }
        assert_eq!(o2.stage(&t2, "p", 1, "big", vec![0; attach::MAX_BYTES]), Err("too_many"), "總量上限 35MB");
        assert!(o.peek(&t, "p", SESSION_TTL_SECS + 5).is_none() && o.stage(&t, "p", SESSION_TTL_SECS + 5, "x", vec![1]).is_err(), "過期後不能再傳");
    }

    #[test]
    fn peek_does_not_extend_the_session_but_touch_does() {
        let o = Otps::default();
        o.issue(&otp_key("p", "a@b.co"), 0, "111111".into()).unwrap();
        let t = o.login("p", "a@b.co", "111111", 0).unwrap();
        assert!(o.peek(&t, "p", SESSION_TTL_SECS - 1).is_some());
        assert!(o.peek(&t, "p", SESSION_TTL_SECS + 1).is_none(), "輪詢不能讓登入永遠不過期");
        let t = { o.issue(&otp_key("p", "a@b.co"), 5000, "222222".into()).unwrap(); o.login("p", "a@b.co", "222222", 5000).unwrap() };
        assert!(o.touch(&t, "p", 5000 + SESSION_TTL_SECS - 1).is_some());
        assert!(o.peek(&t, "p", 5000 + 2 * SESSION_TTL_SECS - 5).is_some(), "touch 之後到期時間往後延");
    }

    #[test]
    fn page_posts_json_to_the_two_endpoints() {
        assert!(PAGE.contains("/api/code") && PAGE.contains("/api/login") && PAGE.contains("/api/submit") && PAGE.contains("/api/logout") && PAGE.contains("application/json"));
    }
}

// ---- HTTP 伺服器 ----

use axum::extract::{DefaultBodyLimit, Path as UrlPath, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};


/// 伺服器需要的外部能力都從這裡注入：測試不必起一個完整的 Tauri 應用。
pub struct WebState {
    pub pool: SqlitePool,
    pub otps: Otps,
    /// 附圖存放的根目錄。
    pub attach_root: std::path::PathBuf,
    /// 佇列有新項目時通知前端。
    pub on_change: Box<dyn Fn() + Send + Sync>,
    /// 寄驗證碼：`(專案 id, 收件地址, 主旨, 內文)`，在背景執行。
    pub send_mail: Box<dyn Fn(String, String, String, String) + Send + Sync>,
    /// 目前開啟中的專案（id、名稱）；網址裡的短代碼靠它對回專案。
    pub projects: Box<dyn Fn() -> Vec<(String, String)> + Send + Sync>,
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
struct LoginReq {
    email: String,
    code: String,
}

#[derive(Deserialize)]
struct TokenReq {
    token: String,
}

#[derive(Deserialize)]
struct SubmitReq {
    token: String,
    #[serde(default)]
    subject: String,
    #[serde(default)]
    body: String,
    /// 檔案以 base64 傳送；大小與個數由 `attach` 限制，圖片與否由內容判斷。舊版頁面送的欄位叫 `images`，一併接受。
    #[serde(default, alias = "images")]
    files: Vec<FileReq>,
    /// 先用 `/api/attach` 上傳好的附件 id。
    #[serde(default)]
    uploads: Vec<String>,
}

#[derive(Deserialize)]
struct UnattachReq {
    token: String,
    id: String,
}

#[derive(Deserialize)]
struct FileReq {
    #[serde(default)]
    name: String,
    data: String,
}

/// base64 解不開的圖片留成空內容，之後會被當成「略過」計數，而不是讓整個請求失敗。
fn decode_files(files: Vec<FileReq>) -> Vec<Raw> {
    use base64::Engine;
    files
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

/// 網址上的專案短代碼＝專案 id 的前 8 個字元；「未歸屬」的舊設定用 `legacy`。
pub fn project_code(project_id: &str) -> String {
    if project_id.is_empty() { "legacy".to_string() } else { project_id.chars().take(8).collect() }
}

/// 依短代碼找專案，回傳 `(專案 id, 名稱, 設定)`；找不到或沒啟用網頁表單回 `None`。
async fn resolve(st: &WebState, code: &str) -> Option<(String, String, Config)> {
    let mut all = (st.projects)();
    if super::config::exists(&st.pool, "").await {
        all.push((String::new(), "未歸屬".to_string()));
    }
    let (id, name) = all.into_iter().find(|(id, _)| project_code(id) == code)?;
    let cfg = super::config::load(&st.pool, &id).await;
    (cfg.web_mode != WebMode::Off).then_some((id, name, cfg))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn no_store_html(body: String) -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store"), (header::X_FRAME_OPTIONS, "DENY")], Html(body))
}

const BASE_CSS: &str = include_str!("web_base.css");

/// 非表單頁（首頁、找不到）共用的外框，樣式與表單頁同一份。
fn shell(title: &str, inner: &str) -> String {
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>{title}</title><style>{BASE_CSS}</style></head><body><main>{inner}</main></body></html>")
}

fn empty_card(icon: &str, title: &str, hint: &str, back: Option<&str>) -> String {
    let btn = back.map(|t| format!("<a class=\"btn\" href=\"/\">{t}</a>")).unwrap_or_default();
    format!("<section class=\"card empty\"><div class=\"ico\">{icon}</div><h2>{title}</h2><p class=\"hint\">{hint}</p>{btn}</section>")
}

/// 瀏覽器偏好中文（或沒說）就用繁體中文，否則英文。表單頁自己在前端依 `navigator.language` 切換。
fn zh(headers: &axum::http::HeaderMap) -> bool {
    headers.get(header::ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()).map(|v| v.trim().to_ascii_lowercase()).map_or(true, |v| v.is_empty() || v.starts_with("zh"))
}

fn not_found(zh: bool) -> axum::response::Response {
    let (title, hint, back, page) = if zh {
        ("找不到這個頁面", "這個專案不存在，或網頁提問尚未開放。請向管理者確認網址。", "回到專案列表", "找不到頁面")
    } else {
        ("Page not found", "This project doesn't exist, or web questions aren't open yet. Please check the address with the administrator.", "Back to projects", "Not found")
    };
    let body = shell(page, &empty_card("🔎", title, hint, Some(back)));
    (StatusCode::NOT_FOUND, [(header::CACHE_CONTROL, "no-store"), (header::X_FRAME_OPTIONS, "DENY")], Html(body)).into_response()
}

/// 首頁：列出已啟用網頁表單的專案。
async fn index(State(st): State<Arc<WebState>>, headers: axum::http::HeaderMap) -> impl IntoResponse {
    let zh = zh(&headers);
    let mut all = (st.projects)();
    if super::config::exists(&st.pool, "").await {
        all.push((String::new(), "未歸屬".to_string()));
    }
    let go = if zh { "點選後驗證信箱並提問" } else { "Verify your email and ask" };
    let mut items = String::new();
    for (id, name) in all {
        if super::config::load(&st.pool, &id).await.web_mode != WebMode::Off {
            items.push_str(&format!("<li><a class=\"item\" href=\"/p/{}\"><span>{}<small>{go}</small></span><span class=\"go\">→</span></a></li>", project_code(&id), escape_html(&name)));
        }
    }
    let (title, pick, none, none_hint) = if zh { ("提問", "請選擇要提問的專案", "目前沒有開放的專案", "管理者尚未開放網頁提問。") } else { ("Ask a question", "Choose a project", "No projects are open", "The administrator hasn't opened web questions yet.") };
    let inner = if items.is_empty() {
        empty_card("📭", none, none_hint, None)
    } else {
        format!("<header><h1>{title}</h1><p class=\"hint\" style=\"margin:6px 0 0\">{pick}</p></header><ul class=\"list\">{items}</ul>")
    };
    no_store_html(shell(title, &inner))
}

async fn page(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, headers: axum::http::HeaderMap) -> axum::response::Response {
    match resolve(&st, &code).await {
        Some((id, name, _)) => no_store_html(PAGE.replace("__BASE_CSS__", BASE_CSS).replace("__TITLE__", &escape_html(&name)).replace("__BASE__", &format!("/p/{}", project_code(&id)))).into_response(),
        None => not_found(zh(&headers)),
    }
}

async fn api_code(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, Json(req): Json<CodeReq>) -> impl IntoResponse {
    if let Some((pid, _, cfg)) = resolve(&st, &code).await {
        if let Ok(Some((to, c))) = prepare_code(&st.pool, &pid, &cfg, &st.otps, &req.email, now()).await {
            // 寄信放背景：回應時間不能洩漏「這個信箱有沒有在白名單」。
            let body = format!("你的驗證碼是 {c}，10 分鐘內有效。如果不是你本人操作，請忽略這封信。");
            (st.send_mail)(pid, to, "提問驗證碼".into(), body);
        }
    }
    // 專案不存在、未開放、不在白名單都回一樣的內容，不洩漏哪些專案存在。
    json(StatusCode::OK, serde_json::json!({ "ok": true }))
}

async fn api_login(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, Json(req): Json<LoginReq>) -> impl IntoResponse {
    let bad = || json(StatusCode::FORBIDDEN, serde_json::json!({ "error": "驗證碼錯誤或已過期", "code": "bad_code" }));
    let Some((pid, _, cfg)) = resolve(&st, &code).await else { return bad() };
    match login(&st.pool, &pid, &cfg, &st.otps, now(), &req.email, &req.code).await {
        Ok(Some((token, email))) => json(StatusCode::OK, serde_json::json!({ "token": token, "email": email, "ttl": SESSION_TTL_SECS })),
        Ok(None) => bad(),
        Err(e) => {
            log::warn!("intake web: login failed: {e}");
            json(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({ "error": "伺服器錯誤" }))
        }
    }
}

async fn api_logout(State(st): State<Arc<WebState>>, UrlPath(_code): UrlPath<String>, Json(req): Json<TokenReq>) -> impl IntoResponse {
    st.otps.logout(&req.token);
    json(StatusCode::OK, serde_json::json!({ "ok": true }))
}

fn err(status: StatusCode, code: &str, msg: &str) -> axum::response::Response {
    json(status, serde_json::json!({ "error": msg, "code": code })).into_response()
}

async fn api_submit(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, Json(req): Json<SubmitReq>) -> axum::response::Response {
    let Some((pid, _, cfg)) = resolve(&st, &code).await else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "disabled", "目前未開放");
    };
    let mut files = decode_files(req.files);
    files.extend(st.otps.staged_for(&req.token, &req.uploads));
    match submit(&st.pool, &pid, &cfg, &st.otps, now(), &req.token, &req.subject, &req.body, &st.attach_root, files).await {
        Ok(Submit::Accepted) => {
            st.otps.clear_staged(&req.token, &req.uploads);
            (st.on_change)();
            json(StatusCode::OK, serde_json::json!({ "ok": true, "ttl": SESSION_TTL_SECS })).into_response()
        }
        Ok(Submit::NoSession) => err(StatusCode::UNAUTHORIZED, "session", "登入已失效，請重新驗證"),
        Ok(Submit::Invalid(m)) => {
            let c = match m {
                "內容不可為空" => "empty",
                "內容太長" => "too_long",
                "送出太頻繁，請稍後再試" => "rate",
                _ => "unavailable",
            };
            err(StatusCode::BAD_REQUEST, c, m)
        }
        Ok(Submit::Disabled) => err(StatusCode::SERVICE_UNAVAILABLE, "disabled", "目前未開放"),
        Err(e) => {
            log::warn!("intake web: submit failed: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "server", "伺服器錯誤")
        }
    }
}

#[derive(Deserialize)]
struct AttachQuery {
    token: String,
    #[serde(default)]
    name: String,
}

/// 附件單獨上傳（原始位元組，不用 base64）：每個檔案各一個請求，頁面才能顯示進度、失敗時只重傳那一個。
async fn api_attach(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, axum::extract::Query(q): axum::extract::Query<AttachQuery>, body: axum::body::Bytes) -> axum::response::Response {
    let Some((pid, _, cfg)) = resolve(&st, &code).await else { return err(StatusCode::SERVICE_UNAVAILABLE, "disabled", "目前未開放") };
    if cfg.web_mode == WebMode::Off {
        return err(StatusCode::SERVICE_UNAVAILABLE, "disabled", "目前未開放");
    }
    let size = body.len();
    match st.otps.stage(&q.token, &pid, now(), &q.name, body.to_vec()) {
        Ok(id) => json(StatusCode::OK, serde_json::json!({ "id": id, "size": size })).into_response(),
        Err("session") => err(StatusCode::UNAUTHORIZED, "session", "登入已失效，請重新驗證"),
        Err("empty") => err(StatusCode::BAD_REQUEST, "file_empty", "這個檔案是空的"),
        Err("too_big") => err(StatusCode::PAYLOAD_TOO_LARGE, "file_big", "單一檔案超過 20MB"),
        Err(_) => err(StatusCode::BAD_REQUEST, "file_many", "附件個數或總量超過上限"),
    }
}

async fn api_unattach(State(st): State<Arc<WebState>>, UrlPath(_code): UrlPath<String>, Json(req): Json<UnattachReq>) -> impl IntoResponse {
    st.otps.unstage(&req.token, &req.id);
    json(StatusCode::OK, serde_json::json!({ "ok": true }))
}

/// 提問者自己的題目與處理進度。只給粗略階段，不外露內部狀態與回覆內容（草稿還沒經人確認）。
fn public_status(status: &str) -> &'static str {
    match status {
        store::PENDING | store::QUEUED => "received",
        store::DONE => "replied",
        store::REJECTED => "rejected",
        _ => "working",
    }
}

async fn api_status(State(st): State<Arc<WebState>>, UrlPath(code): UrlPath<String>, Json(req): Json<TokenReq>) -> axum::response::Response {
    let Some((pid, _, _)) = resolve(&st, &code).await else { return err(StatusCode::SERVICE_UNAVAILABLE, "disabled", "目前未開放") };
    let Some(addr) = st.otps.peek(&req.token, &pid, now()) else { return err(StatusCode::UNAUTHORIZED, "session", "登入已失效，請重新驗證") };
    match store::list_web_items(&st.pool, &pid, &addr, 20).await {
        Ok(rows) => {
            let items: Vec<_> = rows.into_iter().map(|(id, subject, status, at)| serde_json::json!({ "id": id, "subject": subject, "status": public_status(&status), "at": at })).collect();
            json(StatusCode::OK, serde_json::json!({ "items": items })).into_response()
        }
        Err(e) => {
            log::warn!("intake web: status failed: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, "server", "伺服器錯誤")
        }
    }
}

pub fn router(st: Arc<WebState>) -> Router {
    Router::new()
        .route("/", get(index))
        .fallback(|headers: axum::http::HeaderMap| async move { not_found(zh(&headers)) })
        .route("/p/{code}", get(page))
        // 要驗證碼的請求只有一個信箱，小上限；送出問題可能帶 5 張圖（base64 約 13MB）。
        .route("/p/{code}/api/code", post(api_code).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/p/{code}/api/login", post(api_login).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/p/{code}/api/attach", post(api_attach).layer(DefaultBodyLimit::max(attach::MAX_BYTES + 1024)))
        .route("/p/{code}/api/unattach", post(api_unattach).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/p/{code}/api/status", post(api_status).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/p/{code}/api/logout", post(api_logout).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/p/{code}/api/submit", post(api_submit).layer(DefaultBodyLimit::max(MAX_SUBMIT_BYTES)))
        .with_state(st)
}

/// 依目前設定（重新）啟動網頁表單伺服器。
pub async fn restart<R: Runtime>(app: &AppHandle<R>) {
    let Some(server) = app.try_state::<Arc<WebServer>>() else { return };
    if let Some(h) = server.handle.lock().take() {
        h.abort();
    }
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    // 任一專案啟用網頁表單就開伺服器；任一專案設為區網就綁 0.0.0.0。
    let modes: Vec<WebMode> = super::all_configs(app, &db.pool).await.into_iter().map(|(_, _, c)| c.web_mode).collect();
    let host = if modes.contains(&WebMode::Lan) {
        [0, 0, 0, 0]
    } else if modes.contains(&WebMode::Local) {
        [127, 0, 0, 1]
    } else {
        return;
    };
    let port = super::config::load_global(&db.pool).await.web_port;
    let addr = std::net::SocketAddr::from((host, port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("intake web: 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let st = Arc::new(WebState {
        pool: db.pool.clone(),
        otps: Otps::default(),
        attach_root: attach::root(),
        on_change: Box::new(move || {
            use tauri::Emitter;
            let _ = a1.emit(super::CHANGED_EVENT, ());
        }),
        projects: Box::new(move || super::open_projects(&a3)),
        send_mail: Box::new(move |project_id, to, subject, body| {
            let app = a2.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = super::runtime::send_plain_mail(&app, &project_id, &to, &subject, &body).await {
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
        for pid in ["proj1", "proj2"] {
            store::upsert_sender(&pool, pid, &Sender { address: "bob@corp.com".into(), level: Level::External, skip_auth: false }).await.unwrap();
            super::super::config::save(&pool, pid, &Config { web_mode: mode, ..Config::default() }).await.unwrap();
        }
        let sent: Sent = Arc::default();
        let (s2, changes) = (sent.clone(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let c2 = changes.clone();
        let root2 = std::env::temp_dir().join(format!("intake-web-test-{}", uuid::Uuid::new_v4()));
        let st = Arc::new(WebState {
            pool: pool.clone(),
            otps: Otps::default(),
            attach_root: root2,
            on_change: Box::new(move || { c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }),
            send_mail: Box::new(move |_pid, to, sub, body| s2.lock().push((to, sub, body))),
            projects: Box::new(|| vec![("proj1".to_string(), "專案一".to_string()), ("proj2".to_string(), "專案二".to_string())]),
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

    /// 請求驗證碼、從「寄出的信」取出 6 位數，再換 token。
    async fn login_token(r: &Router, sent: &Sent, project: &str) -> String {
        post(r, &format!("/p/{project}/api/code"), serde_json::json!({"email": "bob@corp.com"})).await;
        let body = sent.lock().last().unwrap().2.clone();
        let chars: Vec<char> = body.chars().collect();
        let code: String = chars.windows(6).find(|w| w.iter().all(|c| c.is_ascii_digit())).unwrap().iter().collect();
        let (st, out) = post(r, &format!("/p/{project}/api/login"), serde_json::json!({"email":"bob@corp.com","code":code})).await;
        assert_eq!(st, StatusCode::OK, "{out}");
        serde_json::from_str::<serde_json::Value>(&out).unwrap()["token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn full_flow_code_by_mail_then_submit() {
        let (r, pool, sent, changes) = app(WebMode::Local).await;
        let (st, _) = post(&r, "/p/proj1/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(sent.lock()[0].0, "bob@corp.com");
        let (st, _) = post(&r, "/p/proj1/api/login", serde_json::json!({"email":"bob@corp.com","code":"000000"})).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":"nope","body":"hi"})).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        // 前面那次寄的信裡的碼；重新發一組以便用 helper 取碼。
        let token = login_token(&r, &sent, "proj1").await;
        for q in ["請修", "再一題", "第三題"] {
            let (st, out) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"subject":"s","body":q})).await;
            assert_eq!(st, StatusCode::OK, "{out}");
            assert!(out.contains("ttl"));
        }
        let (st, _) = post(&r, "/p/proj1/api/logout", serde_json::json!({"token":token})).await;
        assert_eq!(st, StatusCode::OK);
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"body":"登出後"})).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert_eq!(store::list_items(&pool, "proj1", None).await.unwrap().len(), 3);
        assert_eq!(changes.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
    #[tokio::test]
    async fn code_endpoint_answers_identically_for_listed_and_unlisted_addresses() {
        let (r, _, sent, _) = app(WebMode::Local).await;
        let a = post(&r, "/p/proj1/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        let b = post(&r, "/p/proj1/api/code", serde_json::json!({"email": "stranger@x.com"})).await;
        assert_eq!(a, b, "狀態碼與內容必須一樣，否則可以拿來探測白名單");
        assert_eq!(sent.lock().len(), 1, "只有白名單內的信箱真的收到信");
    }

    #[tokio::test]
    async fn disabled_server_refuses_submissions_and_sends_no_codes() {
        let (r, _, sent, _) = app(WebMode::Off).await;
        post(&r, "/p/proj1/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        assert!(sent.lock().is_empty());
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":"abc","body":"x"})).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
        let (st, _) = post(&r, "/p/proj1/api/login", serde_json::json!({"email":"bob@corp.com","code":"123456"})).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn oversized_request_bodies_are_rejected_before_parsing() {
        let (r, pool, _, _) = app(WebMode::Local).await;
        let huge = "x".repeat(MAX_SUBMIT_BYTES + 1024);
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":"abc","body":huge})).await;
        assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(store::list_items(&pool, "proj1", None).await.unwrap().is_empty());
        // 要驗證碼的端點只該收到一個信箱，上限小得多。
        let (st, _) = post(&r, "/p/proj1/api/code", serde_json::json!({"email":"bob@corp.com","pad":"x".repeat(8 * 1024)})).await;
        assert_eq!(st, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn files_in_a_submission_are_saved_and_attached() {
        use base64::Engine;
        let (r, pool, sent, _) = app(WebMode::Local).await;
        let token = login_token(&r, &sent, "proj1").await;
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 7, 7];
        let ok = base64::engine::general_purpose::STANDARD.encode(png);
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({
            "token":token,"subject":"s","body":"",
            "files":[{"name":"畫面.png","data":ok},{"name":"bad.png","data":"!!!not base64!!!"},{"name":"x.txt","data":base64::engine::general_purpose::STANDARD.encode(b"hello")}]
        })).await;
        assert_eq!(st, StatusCode::OK);
        let it = &store::list_items(&pool, "proj1", None).await.unwrap()[0];
        let atts = store::list_attachments(&pool, &it.id).await.unwrap();
        assert_eq!(atts.len(), 2, "圖片與文字檔都收；解不開的 base64 被略過");
        assert!(it.body.contains("僅附件") && it.body.contains("1 個附件被略過"), "{}", it.body);
    }

    async fn post_raw(r: &Router, path: &str, bytes: Vec<u8>) -> (StatusCode, String) {
        let resp = r.clone().oneshot(Request::post(path).header("content-type", "application/octet-stream").body(Body::from(bytes)).unwrap()).await.unwrap();
        let status = resp.status();
        let b = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&b).into_owned())
    }

    #[tokio::test]
    async fn files_can_be_uploaded_one_by_one_then_attached_to_the_question() {
        let (r, pool, sent, _) = app(WebMode::Local).await;
        let token = login_token(&r, &sent, "proj1").await;
        let (st, _) = post_raw(&r, "/p/proj1/api/attach?token=bad&name=a.txt", b"hello".to_vec()).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        let (st, out) = post_raw(&r, &format!("/p/proj1/api/attach?token={token}&name=a.txt"), b"hello".to_vec()).await;
        assert_eq!(st, StatusCode::OK, "{out}");
        let id1 = serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"].as_str().unwrap().to_string();
        let (_, out) = post_raw(&r, &format!("/p/proj1/api/attach?token={token}&name=b.txt"), b"world".to_vec()).await;
        let id2 = serde_json::from_str::<serde_json::Value>(&out).unwrap()["id"].as_str().unwrap().to_string();
        let (st, _) = post_raw(&r, &format!("/p/proj1/api/attach?token={token}&name=empty.txt"), vec![]).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        post(&r, "/p/proj1/api/unattach", serde_json::json!({"token":token,"id":id2})).await;
        let (st, out) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"body":"附檔","uploads":[id1,id2]})).await;
        assert_eq!(st, StatusCode::OK, "{out}");
        let it = &store::list_items(&pool, "proj1", None).await.unwrap()[0];
        assert_eq!(store::list_attachments(&pool, &it.id).await.unwrap().len(), 1, "移除的那個不會被附上");
    }

    #[tokio::test]
    async fn status_lists_only_the_askers_own_web_questions_with_coarse_stages() {
        let (r, pool, sent, _) = app(WebMode::Local).await;
        let token = login_token(&r, &sent, "proj1").await;
        post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"subject":"第一題","body":"a"})).await;
        post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"subject":"第二題","body":"b"})).await;
        // 別人的題目不該出現
        store::insert_item(&pool, store::NewItem { project_id: "proj1", id: None, source: "web", sender: "other@corp.com", level: Level::External, subject: "別人的", body: "x", message_id: "m-other", status: store::PENDING }).await.unwrap();
        let first = store::list_items(&pool, "proj1", None).await.unwrap().into_iter().find(|i| i.sender == "bob@corp.com" && i.subject == "第一題").unwrap();
        sqlx::query("UPDATE intake_items SET status = ? WHERE id = ?").bind(store::DRAFT).bind(&first.id).execute(&pool).await.unwrap();
        let (st, out) = post(&r, "/p/proj1/api/status", serde_json::json!({"token":token})).await;
        assert_eq!(st, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "{out}");
        assert!(!out.contains("別人的") && !out.contains("draft") && !out.contains("reply"), "不外露別人的題目、內部狀態與回覆內容：{out}");
        let by = |s: &str| items.iter().find(|i| i["subject"] == s).unwrap()["status"].as_str().unwrap().to_string();
        assert_eq!(by("第一題"), "working", "草稿（還沒經人確認）只顯示處理中");
        assert_eq!(by("第二題"), "received");
        let (st, _) = post(&r, "/p/proj1/api/status", serde_json::json!({"token":"bad"})).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        let (st, _) = post(&r, "/p/proj2/api/status", serde_json::json!({"token":token})).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "別的專案不能查");
    }

    #[tokio::test]
    async fn errors_carry_a_machine_readable_code_for_the_page_to_translate() {
        let (r, _, sent, _) = app(WebMode::Local).await;
        let (_, out) = post(&r, "/p/proj1/api/login", serde_json::json!({"email":"bob@corp.com","code":"000000"})).await;
        assert!(out.contains("\"code\":\"bad_code\""), "{out}");
        let token = login_token(&r, &sent, "proj1").await;
        let (_, out) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"body":"  "})).await;
        assert!(out.contains("\"code\":\"empty\""), "{out}");
        let (_, out) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":"x","body":"hi"})).await;
        assert!(out.contains("\"code\":\"session\""), "{out}");
    }

    #[tokio::test]
    async fn page_is_served_with_no_store_and_frame_protection() {
        let (r, _, _, _) = app(WebMode::Local).await;
        let resp = r.clone().oneshot(Request::get("/p/proj1").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(resp.headers()[header::X_FRAME_OPTIONS], "DENY");
    }

    async fn get_text(r: &Router, path: &str) -> (StatusCode, String) {
        let resp = r.clone().oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn each_project_has_its_own_page_and_the_index_lists_them() {
        let (r, _, _, _) = app(WebMode::Local).await;
        let (st, home) = get_text(&r, "/").await;
        assert_eq!(st, StatusCode::OK);
        assert!(home.contains("/p/proj1") && home.contains("專案一") && home.contains("/p/proj2"), "{home}");
        let (_, page) = get_text(&r, "/p/proj2").await;
        assert!(page.contains("專案二") && page.contains("const BASE=\"/p/proj2\"") && !page.contains("__BASE__") && !page.contains("__BASE_CSS__"), "{page}");
        let (st, nf) = get_text(&r, "/p/nope").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(nf.contains("回到專案列表") && nf.contains("<style>"), "找不到頁面要有樣式與返回連結");
        assert_eq!(get_text(&r, "/whatever").await.0, StatusCode::NOT_FOUND);
        let resp = r.clone().oneshot(Request::get("/p/nope").header("accept-language", "en-US,en;q=0.9").body(Body::empty()).unwrap()).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Back to projects"), "英文瀏覽器看到英文");
        let resp = r.clone().oneshot(Request::get("/").header("accept-language", "en").body(Body::empty()).unwrap()).await.unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Choose a project"));
    }

    #[tokio::test]
    async fn an_unknown_project_looks_the_same_to_the_code_endpoint_and_refuses_submissions() {
        let (r, _, sent, _) = app(WebMode::Local).await;
        let a = post(&r, "/p/proj1/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        let b = post(&r, "/p/nope/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        assert_eq!(a, b);
        assert_eq!(sent.lock().len(), 1, "不存在的專案不寄信");
        let (st, _) = post(&r, "/p/nope/api/submit", serde_json::json!({"token":"abc","body":"x"})).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_code_or_token_from_one_project_cannot_be_used_in_another() {
        let (r, pool, sent, _) = app(WebMode::Local).await;
        post(&r, "/p/proj1/api/code", serde_json::json!({"email": "bob@corp.com"})).await;
        let body = sent.lock()[0].2.clone();
        let chars: Vec<char> = body.chars().collect();
        let code: String = chars.windows(6).find(|w| w.iter().all(|c| c.is_ascii_digit())).unwrap().iter().collect();
        let (st, _) = post(&r, "/p/proj2/api/login", serde_json::json!({"email":"bob@corp.com","code":code})).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "專案一的驗證碼不能登入專案二");
        let token = login_token(&r, &sent, "proj1").await;
        let (st, _) = post(&r, "/p/proj2/api/submit", serde_json::json!({"token":token,"body":"偷渡"})).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert!(store::list_items(&pool, "proj2", None).await.unwrap().is_empty());
        let (st, _) = post(&r, "/p/proj1/api/submit", serde_json::json!({"token":token,"body":"正常"})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(store::list_items(&pool, "proj1", None).await.unwrap().len(), 1);
    }
}

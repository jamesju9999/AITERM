//! Server 收信：定時用 IMAP 讀信箱的新信，交給與桌面版相同的規則（別名／主旨標籤、白名單）決定歸屬。
//!
//! - 只讀：`EXAMINE` 開啟、`BODY.PEEK[]` 取信，不改已讀、不搬、不刪。
//! - 第一次只記下目前最新的信件編號，**不收既有的舊信**；之後只收編號更大的新信。
//! - 信箱換了身分（UIDVALIDITY 改變）時同樣重新記基準，不補收。
//! - 只支援隱含 TLS 的 IMAP（通常是 993）。
use crate::mail::{self, MailSettings};
use crate::state::AppState;
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::mail_source::{self, Candidate};
use aiterm_intake_core::queue::Ingested;
use aiterm_intake_core::{config, store};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

const STATE_KEY: &str = "email_state";
/// 每次輪詢最多收幾封，避免一次塞太多；剩下的下一輪接著收。
pub const MAX_PER_POLL: usize = 100;
pub const POLL_EVERY: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(30);

/// 收到哪裡了。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct State {
    pub uid_validity: Option<u32>,
    pub last_uid: u32,
}

/// 這一輪要做什麼。
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// 第一次（或信箱換身分）：只記下目前最新編號，不收信。
    Baseline(u32),
    Fetch(Vec<u32>),
    Nothing,
}

/// 由已存的進度、信箱目前的 UIDVALIDITY 與搜尋到的編號決定這一輪的動作。
pub fn plan(state: Option<&State>, validity: Option<u32>, uids: &[u32]) -> Plan {
    let max = uids.iter().copied().max().unwrap_or(0);
    let Some(s) = state else { return Plan::Baseline(max) };
    // 只有「存的與目前的都有值而且不同」才算信箱換了身分；伺服器沒回報就不當成改變。
    if matches!((s.uid_validity, validity), (Some(a), Some(b)) if a != b) {
        return Plan::Baseline(max);
    }
    // `UID n:*` 在沒有新信時可能回傳最後一封（RFC 3501），所以一律再檢查一次。
    let mut v: Vec<u32> = uids.iter().copied().filter(|u| *u > s.last_uid).collect();
    v.sort_unstable();
    v.dedup();
    v.truncate(MAX_PER_POLL);
    if v.is_empty() { Plan::Nothing } else { Plan::Fetch(v) }
}

/// 目前的收信狀態，給 `status` 顯示。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Health {
    /// 最近一次成功輪詢的時間（Unix 秒）；從未成功為 `None`。
    pub last_ok: Option<i64>,
    /// 最近一次輪詢的錯誤；成功後清除。
    pub error: Option<String>,
}

async fn load_state(st: &AppState) -> Option<State> {
    store::get_setting(&st.pools.intake, STATE_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok())
}

async fn save_state(st: &AppState, s: &State) {
    if let Ok(v) = serde_json::to_string(s) {
        let _ = store::set_setting(&st.pools.intake, STATE_KEY, &v).await;
    }
}

/// 有設定 IMAP 主機與密碼才算會收信。
pub fn receiving(s: &MailSettings, secrets: &dyn Secrets) -> bool {
    !s.imap_host.trim().is_empty() && !s.username.trim().is_empty() && secrets.get(mail::PASSWORD_SECRET).is_some()
}

/// 已啟用問題收集的專案設定（歸屬判斷用）。
async fn candidates(st: &AppState) -> Vec<Candidate> {
    let mut out = Vec::new();
    for p in crate::routes_data::project_list(st).await {
        let cfg = config::load(&st.pools.intake, &p.id).await;
        if cfg.enabled {
            out.push((p.id, cfg));
        }
    }
    out
}

fn tls_config() -> tokio_rustls::rustls::ClientConfig {
    aiterm_core::share::ensure_crypto_provider();
    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()
}

type Tls = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;
type Session = async_imap::Session<Tls>;

async fn within<T>(what: &str, fut: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(TIMEOUT, fut).await.unwrap_or_else(|_| Err(format!("{what}逾時")))
}

async fn connect(s: &MailSettings, password: &str) -> Result<Session, String> {
    let host = s.imap_host.trim().to_string();
    let tcp = within("連線", async { tokio::net::TcpStream::connect((host.as_str(), s.imap_port)).await.map_err(|e| format!("連線失敗：{e}")) }).await?;
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| format!("主機名稱無效：{e}"))?;
    let tls = within("TLS 交握", async { tokio_rustls::TlsConnector::from(Arc::new(tls_config())).connect(name, tcp).await.map_err(|e| format!("TLS 交握失敗：{e}")) }).await?;
    let client = async_imap::Client::new(tls);
    within("登入", async { client.login(s.username.trim(), password).await.map_err(|(e, _)| format!("登入失敗：{e}")) }).await
}

/// 讀信箱、依 `plan` 取新信並入庫。回傳新開出的題目數。
pub async fn poll_once(st: &Arc<AppState>) -> Result<usize, String> {
    let settings = mail::load(&st.pools.intake).await;
    if !receiving(&settings, st.secrets.as_ref()) {
        return Ok(0);
    }
    let cands = candidates(st).await;
    if cands.is_empty() {
        return Ok(0);
    }
    let password = st.secrets.get(mail::PASSWORD_SECRET).unwrap_or_default();
    let mut session = connect(&settings, &password).await?;
    let result = poll_session(st, &mut session, &cands).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), session.logout()).await;
    result
}

async fn poll_session(st: &Arc<AppState>, session: &mut Session, cands: &[Candidate]) -> Result<usize, String> {
    let mailbox = within("開啟信箱", async { session.examine("INBOX").await.map_err(|e| format!("開啟信箱失敗：{e}")) }).await?;
    let validity = mailbox.uid_validity;
    let state = load_state(st).await;
    // 已有進度且信箱沒換身分時只搜新的；否則要看全部編號才知道基準。
    let known = matches!(&state, Some(s) if !matches!((s.uid_validity, validity), (Some(a), Some(b)) if a != b));
    let query = match (&state, known) {
        (Some(s), true) => format!("UID {}:*", s.last_uid.saturating_add(1)),
        _ => "ALL".to_string(),
    };
    let uids: Vec<u32> = within("搜尋", async { session.uid_search(&query).await.map_err(|e| format!("搜尋失敗：{e}")) }).await?.into_iter().collect();
    match plan(state.as_ref(), validity, &uids) {
        Plan::Baseline(max) => {
            save_state(st, &State { uid_validity: validity, last_uid: max }).await;
            Ok(0)
        }
        Plan::Nothing => Ok(0),
        Plan::Fetch(list) => {
            let mut opened = 0;
            let mut last = state.as_ref().map(|s| s.last_uid).unwrap_or(0);
            let root = st.data_dir.join("attachments");
            for uid in list {
                let fetched = within("取信", async {
                    let stream = session.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await.map_err(|e| format!("取信失敗：{e}"))?;
                    stream.try_collect::<Vec<_>>().await.map_err(|e| format!("取信失敗：{e}"))
                })
                .await?;
                // 取不到內文的信（例如這時剛被刪）直接略過，不讓它卡住之後的信。
                if let Some(raw) = fetched.iter().find_map(|f| f.body()) {
                    match mail_source::ingest_routed(&st.pools.intake, cands, raw, &root).await {
                        Ok(Ingested::Queued(_)) => opened += 1,
                        Ok(_) => {}
                        Err(e) => return Err(format!("入庫失敗：{e}")),
                    }
                }
                last = uid;
                save_state(st, &State { uid_validity: validity, last_uid: last }).await;
            }
            Ok(opened)
        }
    }
}

/// 輪詢一次並記錄結果；有新題目就通知所有連線中的 Client。
pub async fn tick(st: &Arc<AppState>) {
    let settings = mail::load(&st.pools.intake).await;
    if !receiving(&settings, st.secrets.as_ref()) {
        *st.channels.email_health.lock() = Health::default();
        return;
    }
    let outcome = poll_once(st).await;
    let mut h = st.channels.email_health.lock();
    match outcome {
        Ok(n) => {
            h.last_ok = Some(crate::db::now());
            h.error = None;
            drop(h);
            if n > 0 {
                log::info!("Email：收進 {n} 題");
                st.events.changed();
            }
        }
        Err(e) => {
            if h.error.as_deref() != Some(e.as_str()) {
                log::warn!("Email 收信失敗：{e}");
            }
            h.error = Some(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(v: Option<u32>, last: u32) -> State {
        State { uid_validity: v, last_uid: last }
    }

    #[test]
    fn the_first_poll_only_records_where_the_mailbox_is_and_collects_nothing() {
        assert_eq!(plan(None, Some(7), &[3, 9, 5]), Plan::Baseline(9));
        assert_eq!(plan(None, Some(7), &[]), Plan::Baseline(0));
    }

    #[test]
    fn only_messages_newer_than_the_saved_position_are_fetched_in_order() {
        assert_eq!(plan(Some(&st(Some(7), 5)), Some(7), &[9, 6, 5, 3]), Plan::Fetch(vec![6, 9]));
    }

    #[test]
    fn a_range_that_wraps_back_to_the_last_known_message_is_not_new_mail() {
        // `UID 6:*` 沒有新信時，伺服器可能回傳最後一封（編號 5）。
        assert_eq!(plan(Some(&st(Some(7), 5)), Some(7), &[5]), Plan::Nothing);
        assert_eq!(plan(Some(&st(Some(7), 5)), Some(7), &[]), Plan::Nothing);
    }

    #[test]
    fn a_mailbox_that_changed_identity_is_rebaselined_instead_of_re_collecting_everything() {
        assert_eq!(plan(Some(&st(Some(7), 5)), Some(8), &[1, 2, 3, 40]), Plan::Baseline(40));
    }

    #[test]
    fn a_server_that_does_not_report_uidvalidity_is_not_treated_as_changed() {
        assert_eq!(plan(Some(&st(Some(7), 5)), None, &[6]), Plan::Fetch(vec![6]));
        assert_eq!(plan(Some(&st(None, 5)), Some(7), &[6]), Plan::Fetch(vec![6]));
    }

    #[test]
    fn one_poll_never_takes_more_than_the_cap() {
        let uids: Vec<u32> = (1..=(MAX_PER_POLL as u32 + 50)).collect();
        match plan(Some(&st(Some(7), 0)), Some(7), &uids) {
            Plan::Fetch(v) => assert_eq!((v.len(), v[0]), (MAX_PER_POLL, 1)),
            other => panic!("{other:?}"),
        }
    }
}

//! LINE 來的問題（純邏輯，不碰網路與 Tauri）。
//!
//! 身分由 LINE 保證（userId 無法偽造），但 webhook 是公開網址，所以每個請求都要先驗簽章。
//! 沒有「主人」：所有寄件者都必須在白名單，等級依白名單。
//! 設計：docs/superpowers/specs/2026-11-23-intake-line-design.md
use super::attach;
use super::queue::{self, Ingested};
use super::rules::{self, Level};
use super::store;
use super::telegram_source::{subject_of, Candidate};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use sqlx::SqlitePool;
use std::path::Path;

/// 單則 LINE 文字訊息上限 5000 字元；留餘裕。
pub const SEGMENT_CHARS: usize = 4500;
/// 一次 push 請求最多 5 則訊息。
pub const MAX_PER_REQUEST: usize = 5;

/// LINE 的 userId：`U` 加 32 位十六進位。這裡不分大小寫（白名單會轉小寫）。
pub fn valid_user_id(id: &str) -> bool {
    id.len() == 33 && id.starts_with(['u', 'U']) && id[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn sender_key(user_id: &str) -> String {
    format!("line:{}", user_id.to_lowercase())
}

/// 從項目的 `sender`（`line:<小寫 userId>`）還原 push 要用的 userId（第一個字元改回大寫 `U`）。
pub fn reply_target(sender: &str) -> Option<String> {
    let id = sender.strip_prefix("line:")?;
    valid_user_id(id).then(|| format!("U{}", &id[1..].to_lowercase()))
}

/// 驗 `X-Line-Signature`：`base64(HMAC-SHA256(channel_secret, 原始 body))`，常數時間比對。
/// 沒設 secret、缺 header、不是合法 base64、對不上，一律 `false`。
pub fn verify_signature(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    if secret.is_empty() {
        return false;
    }
    let Some(h) = header else { return false };
    let Ok(sig) = base64::engine::general_purpose::STANDARD.decode(h.trim()) else { return false };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else { return false };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub user_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Deserialize)]
struct Webhook {
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type", default)]
    kind: String,
    source: Option<Source>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Source {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(rename = "userId")]
    user_id: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: String,
    text: Option<String>,
}

/// 只取一對一聊天裡的文字訊息；貼圖、圖片、加好友、群組等一律略過。壞掉的 JSON 當作沒有事件。
pub fn parse_events(body: &[u8]) -> Vec<Incoming> {
    let Ok(w) = serde_json::from_slice::<Webhook>(body) else { return Vec::new() };
    w.events
        .into_iter()
        .filter_map(|e| {
            let (src, msg) = (e.source?, e.message?);
            if e.kind != "message" || src.kind != "user" || msg.kind != "text" || msg.id.is_empty() {
                return None;
            }
            let user_id = src.user_id.filter(|u| valid_user_id(u))?;
            Some(Incoming { user_id, message_id: msg.id, text: msg.text? })
        })
        .collect()
}

/// 內容切成不超過 `max` 字元的段落（依行，再不行依字元）。
pub fn split_message(text: &str, max: usize) -> Vec<String> {
    super::telegram_source::split_message(text, max)
}

/// 決定訊息屬於哪個專案再入列：`line:<id>` 在哪個（已啟用 LINE 的）專案的白名單就進哪個。
/// 不在任何白名單、同時在多個專案、停用，都不入列（也不回覆，免得洩漏機器人在聽）。
pub async fn route(pool: &SqlitePool, cands: &[Candidate], user_id: &str, message_id: &str, text: &str, attach_root: &Path) -> Result<Ingested, sqlx::Error> {
    let text = text.trim();
    let enabled: Vec<&Candidate> = cands.iter().filter(|c| c.cfg.line_enabled).collect();
    if enabled.is_empty() || text.is_empty() {
        return Ok(Ingested::Dropped("not_for_us"));
    }
    let key = sender_key(user_id);
    let mut allowed: Vec<(&Candidate, Level)> = Vec::new();
    for c in &enabled {
        if let Some(s) = store::list_senders(pool, &c.id).await?.iter().find(|s| rules::normalize(&s.address) == key) {
            allowed.push((c, s.level));
        }
    }
    match allowed.as_slice() {
        [] => {
            // 只有一個專案時記在它名下，否則記在「無法判定」。
            let pid = if enabled.len() == 1 { enabled[0].id.as_str() } else { "" };
            store::bump_dropped(pool, pid, "not_allowed").await?;
            Ok(Ingested::Dropped("not_allowed"))
        }
        [(c, level)] => {
            let mid = format!("line:{}:{message_id}", user_id.to_lowercase());
            queue::enqueue(pool, &c.cfg, &c.id, *level, "line", &key, &subject_of(text), text, &mid, attach_root, Vec::<attach::Raw>::new()).await
        }
        _ => {
            store::bump_dropped(pool, "", "ambiguous_project").await?;
            Ok(Ingested::Dropped("ambiguous_project"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::config::Config;
    use crate::intake::rules::Sender;
    use sqlx::sqlite::SqlitePoolOptions;

    const UID: &str = "U0123456789abcdef0123456789abcdef";

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }
    fn cand(id: &str, enabled: bool) -> Candidate {
        Candidate { id: id.into(), name: id.into(), cfg: Config { line_enabled: enabled, ..Config::default() } }
    }
    async fn allow(p: &SqlitePool, project: &str, level: Level) {
        store::upsert_sender(p, project, &Sender { address: format!("line:{UID}"), level, skip_auth: false }).await.unwrap();
    }
    async fn go(p: &SqlitePool, cands: &[Candidate], mid: &str, text: &str) -> Ingested {
        let tmp = tempfile::tempdir().unwrap();
        route(p, cands, UID, mid, text, tmp.path()).await.unwrap()
    }
    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
    }

    #[test]
    fn signature_must_match_the_exact_body_and_secret() {
        let body = br#"{"events":[]}"#;
        let good = sign("s3cret", body);
        assert!(verify_signature("s3cret", body, Some(&good)));
        assert!(!verify_signature("s3cret", br#"{"events":[1]}"#, Some(&good)), "body 被竄改");
        assert!(!verify_signature("other", body, Some(&good)), "secret 不同");
        assert!(!verify_signature("s3cret", body, None), "缺 header");
        assert!(!verify_signature("s3cret", body, Some("not base64 !!")));
        assert!(!verify_signature("s3cret", body, Some("")));
        assert!(!verify_signature("", body, Some(&sign("", body))), "沒設 secret 一律拒絕，連對得上的空 secret 簽章也一樣");
    }

    #[test]
    fn user_ids_are_u_plus_32_hex_and_roundtrip_through_the_sender_key() {
        assert!(valid_user_id(UID));
        assert!(valid_user_id(&UID.to_lowercase()));
        for bad in ["", "U", "U123", &UID[..32], "X0123456789abcdef0123456789abcdef", "U0123456789abcdef0123456789abcdeg", &format!("{UID}0")] {
            assert!(!valid_user_id(bad), "{bad}");
        }
        assert_eq!(sender_key(UID), format!("line:{}", UID.to_lowercase()));
        assert_eq!(reply_target(&sender_key(UID)).as_deref(), Some(UID), "push 要用大寫 U 的原始 id");
        assert_eq!(reply_target("tg:5"), None);
        assert_eq!(reply_target("line:nope"), None);
        assert_eq!(reply_target("bob@corp.com"), None);
    }

    #[test]
    fn allowlist_accepts_line_ids_and_rejects_malformed_ones() {
        assert!(rules::valid_address(&format!("line:{UID}")));
        assert!(rules::valid_address(&format!("LINE:{}", UID.to_lowercase())));
        assert!(!rules::valid_address("line:"));
        assert!(!rules::valid_address("line:U123"));
    }

    #[test]
    fn only_one_to_one_text_messages_are_taken() {
        let body = format!(
            r#"{{"destination":"Ubot","events":[
              {{"type":"message","source":{{"type":"user","userId":"{UID}"}},"message":{{"type":"text","id":"100","text":"電腦開不了機"}}}},
              {{"type":"message","source":{{"type":"user","userId":"{UID}"}},"message":{{"type":"sticker","id":"101"}}}},
              {{"type":"message","source":{{"type":"group","groupId":"G1","userId":"{UID}"}},"message":{{"type":"text","id":"102","text":"群組"}}}},
              {{"type":"follow","source":{{"type":"user","userId":"{UID}"}}}},
              {{"type":"message","source":{{"type":"user","userId":"bad"}},"message":{{"type":"text","id":"103","text":"壞 id"}}}},
              {{"type":"message","source":{{"type":"user","userId":"{UID}"}},"message":{{"type":"text","id":"104","text":"第二則"}}}}
            ]}}"#
        );
        let got = parse_events(body.as_bytes());
        assert_eq!(got.iter().map(|i| (i.message_id.as_str(), i.text.as_str())).collect::<Vec<_>>(), vec![("100", "電腦開不了機"), ("104", "第二則")]);
        assert!(got.iter().all(|i| i.user_id == UID));
        assert!(parse_events(br#"{"events":[]}"#).is_empty(), "LINE 後台的 Verify 按鈕");
        assert!(parse_events(b"not json").is_empty());
        assert!(parse_events(b"{}").is_empty());
    }

    #[tokio::test]
    async fn strangers_are_dropped_and_counted_but_allowlisted_users_queue_with_their_level() {
        let p = pool().await;
        assert_eq!(go(&p, &[cand("", true)], "1", "我要入侵").await, Ingested::Dropped("not_allowed"));
        assert!(store::list_items(&p, "", None).await.unwrap().is_empty());
        assert_eq!(store::dropped_counts(&p, "").await.unwrap(), vec![("not_allowed".to_string(), 1)]);

        allow(&p, "", Level::External).await;
        assert!(matches!(go(&p, &[cand("", true)], "2", "電腦開不了機\n藍屏").await, Ingested::Queued(_)));
        let it = &store::list_items(&p, "", None).await.unwrap()[0];
        assert_eq!((it.source.as_str(), it.sender.as_str(), it.sender_level.as_str(), it.body.as_str()), ("line", sender_key(UID).as_str(), "external", "電腦開不了機\n藍屏"));
        assert_eq!(it.subject, "電腦開不了機");
        assert_eq!(it.status, store::PENDING, "外部預設手動派送");
    }

    #[tokio::test]
    async fn internal_users_dispatch_automatically() {
        let p = pool().await;
        allow(&p, "", Level::Internal).await;
        assert!(matches!(go(&p, &[cand("", true)], "1", "幫我看一下").await, Ingested::Queued(_)));
        assert_eq!(store::list_items(&p, "", None).await.unwrap()[0].status, store::QUEUED);
    }

    #[tokio::test]
    async fn a_redelivered_webhook_does_not_queue_twice() {
        let p = pool().await;
        allow(&p, "", Level::External).await;
        assert!(matches!(go(&p, &[cand("", true)], "7", "同一則").await, Ingested::Queued(_)));
        assert_eq!(go(&p, &[cand("", true)], "7", "同一則").await, Ingested::Duplicate);
        assert_eq!(store::list_items(&p, "", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn disabled_or_blank_means_nothing_is_stored() {
        let p = pool().await;
        allow(&p, "", Level::Internal).await;
        assert_eq!(go(&p, &[cand("", false)], "1", "hi").await, Ingested::Dropped("not_for_us"));
        assert_eq!(go(&p, &[cand("", true)], "2", "   \n ").await, Ingested::Dropped("not_for_us"));
        assert_eq!(go(&p, &[], "3", "hi").await, Ingested::Dropped("not_for_us"));
        assert!(store::list_items(&p, "", None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_user_goes_only_to_the_project_whose_allowlist_has_them() {
        let p = pool().await;
        let cands = [cand("a", true), cand("b", true)];
        allow(&p, "a", Level::External).await;
        assert!(matches!(go(&p, &cands, "1", "請問").await, Ingested::Queued(_)));
        assert_eq!(store::list_items(&p, "a", None).await.unwrap().len(), 1);
        assert!(store::list_items(&p, "b", None).await.unwrap().is_empty());
        // 專案 b 沒啟用 LINE 時，即使白名單有人也不收。
        let p2 = pool().await;
        allow(&p2, "b", Level::External).await;
        assert_eq!(go(&p2, &[cand("a", true), cand("b", false)], "1", "請問").await, Ingested::Dropped("not_allowed"));
    }

    #[tokio::test]
    async fn a_user_in_several_projects_is_ambiguous_and_nothing_is_queued() {
        let p = pool().await;
        let cands = [cand("a", true), cand("b", true)];
        allow(&p, "a", Level::External).await;
        allow(&p, "b", Level::External).await;
        assert_eq!(go(&p, &cands, "1", "你是哪個專案").await, Ingested::Dropped("ambiguous_project"));
        assert!(store::list_items(&p, "a", None).await.unwrap().is_empty());
        assert!(store::list_items(&p, "b", None).await.unwrap().is_empty());
    }

    #[test]
    fn splitting_keeps_every_segment_within_the_line_limit_and_loses_nothing() {
        let text = format!("{}\n{}", "字".repeat(4000), "文".repeat(4000));
        let parts = split_message(&text, SEGMENT_CHARS);
        assert!(parts.len() >= 2);
        assert!(parts.iter().all(|s| s.chars().count() <= SEGMENT_CHARS));
        assert_eq!(parts.concat(), text);
    }
}

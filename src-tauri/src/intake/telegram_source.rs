//! Telegram 來的問題。
//!
//! 身分由 Telegram 保證（chat id 無法偽造），所以不需要 SPF/DKIM 的對應物：
//! - 機器人主人的聊天室：訊息以 `/ask ` 開頭才算問題（其餘照舊轉送給作用中的分頁），視為內部。
//! - 其他聊天室：`tg:<chat_id>` 必須在白名單內，等級依白名單。
use super::attach::Raw;
use super::config::Config;
use super::queue::{self, Ingested};
use super::rules::{self, Level};
use super::store;
use sqlx::SqlitePool;
use std::future::Future;
use std::path::Path;

pub const OWNER_PREFIX: &str = "/ask";

pub fn sender_key(chat_id: i64) -> String {
    format!("tg:{chat_id}")
}

/// 主人訊息去掉 `/ask` 前綴；沒有前綴回 `None`。`/askew` 這種不算。
pub fn strip_owner_prefix(text: &str) -> Option<&str> {
    let t = text.trim_start();
    let rest = t.strip_prefix(OWNER_PREFIX)?;
    if rest.is_empty() {
        return Some("");
    }
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}

/// 從項目的 `sender`（`tg:<chat>`）與 `message_id`（`tg:<chat>:<msg>`）還原要回覆的對象。
pub fn reply_target(sender: &str, message_id: &str) -> Option<(i64, Option<i64>)> {
    let chat: i64 = sender.strip_prefix("tg:")?.parse().ok()?;
    let msg = message_id.rsplit(':').next().and_then(|m| m.parse().ok());
    Some((chat, msg))
}

/// 標題取第一行的前 40 字。
pub fn subject_of(body: &str) -> String {
    let line = body.lines().next().unwrap_or("").trim();
    let mut s: String = line.chars().take(40).collect();
    if line.chars().count() > 40 {
        s.push('…');
    }
    s
}

/// Telegram 單則訊息上限 4096 字元；超過就依行（再不行依字元）切開。
pub fn split_message(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        if cur.chars().count() + line.chars().count() > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if line.chars().count() > max {
            let chars: Vec<char> = line.chars().collect();
            for c in chars.chunks(max) {
                out.push(c.iter().collect());
            }
        } else {
            cur.push_str(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `has_files`：訊息帶了圖片。`fetch` 只有在確認寄件者被允許之後才會呼叫，
/// 陌生人不能藉由傳圖讓我們去下載東西。
#[allow(clippy::too_many_arguments)]
pub async fn ingest<F, Fut>(
    pool: &SqlitePool,
    cfg: &Config,
    chat_id: i64,
    message_id: i64,
    text: &str,
    is_owner: bool,
    attach_root: &Path,
    has_files: bool,
    fetch: F,
) -> Result<Ingested, sqlx::Error>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Vec<Raw>>,
{
    if !cfg.telegram_enabled {
        return Ok(Ingested::Dropped("not_for_us"));
    }
    let key = sender_key(chat_id);
    let (level, body) = if is_owner {
        match strip_owner_prefix(text) {
            Some(b) if !b.is_empty() || has_files => (Level::Internal, b),
            // 沒有前綴＝一般對話，留給原本的處理；只有 `/ask` 沒內容（也沒圖）不入列。
            _ => return Ok(Ingested::Dropped("not_for_us")),
        }
    } else {
        let senders = store::list_senders(pool).await?;
        match senders.iter().find(|s| rules::normalize(&s.address) == key) {
            Some(s) => (s.level, text.trim()),
            None => {
                store::bump_dropped(pool, "not_allowed").await?;
                return Ok(Ingested::Dropped("not_allowed"));
            }
        }
    };
    if body.is_empty() && !has_files {
        return Ok(Ingested::Dropped("not_for_us"));
    }
    let files = if has_files { fetch().await } else { Vec::new() };
    let body = if body.is_empty() { "（僅附圖）" } else { body };
    let mid = format!("tg:{chat_id}:{message_id}");
    queue::enqueue(pool, cfg, level, "telegram", &key, &subject_of(body), body, &mid, attach_root, files).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::rules::Sender;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }
    fn cfg() -> Config {
        Config { telegram_enabled: true, ..Config::default() }
    }

    #[test]
    fn owner_prefix_needs_a_word_boundary() {
        assert_eq!(strip_owner_prefix("/ask 修一下 bug"), Some("修一下 bug"));
        assert_eq!(strip_owner_prefix("  /ask\n多行\n問題"), Some("多行\n問題"));
        assert_eq!(strip_owner_prefix("/askew"), None);
        assert_eq!(strip_owner_prefix("hello /ask x"), None);
        assert_eq!(strip_owner_prefix("/ask"), Some(""));
    }

    #[test]
    fn splitting_respects_the_limit_and_loses_nothing() {
        let text = format!("{}\n{}\n{}", "a".repeat(30), "b".repeat(30), "c".repeat(100));
        let parts = split_message(&text, 50);
        assert!(parts.iter().all(|p| p.chars().count() <= 50), "{parts:?}");
        assert_eq!(parts.concat(), text);
        assert_eq!(split_message("短", 50), vec!["短".to_string()]);
        assert!(split_message("", 50).is_empty());
    }

    #[test]
    fn reply_target_roundtrips_and_rejects_non_telegram_senders() {
        assert_eq!(reply_target("tg:-1001", "tg:-1001:77"), Some((-1001, Some(77))));
        assert_eq!(reply_target("bob@corp.com", "m"), None);
        assert_eq!(reply_target("tg:abc", "tg:abc:1"), None);
    }

    #[test]
    fn subject_is_first_line_truncated() {
        assert_eq!(subject_of("第一行\n第二行"), "第一行");
        assert_eq!(subject_of(&"字".repeat(50)).chars().count(), 41);
    }

    #[tokio::test]
    async fn owner_needs_prefix_and_becomes_internal() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(ingest(&p, &cfg(), 1, 10, "普通聊天", true, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Dropped("not_for_us"));
        assert_eq!(ingest(&p, &cfg(), 1, 11, "/ask", true, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Dropped("not_for_us"));
        assert!(matches!(ingest(&p, &cfg(), 1, 12, "/ask 幫我看 log", true, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Queued(_)));
        let it = &store::list_items(&p, None).await.unwrap()[0];
        assert_eq!((it.source.as_str(), it.sender.as_str(), it.sender_level.as_str(), it.body.as_str()), ("telegram", "tg:1", "internal", "幫我看 log"));
        assert_eq!(it.status, store::QUEUED, "內部預設自動派送");
    }

    #[tokio::test]
    async fn strangers_are_dropped_and_counted_but_allowlisted_chats_pass() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(ingest(&p, &cfg(), 99, 1, "我要入侵", false, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Dropped("not_allowed"));
        assert!(store::list_items(&p, None).await.unwrap().is_empty());
        assert_eq!(store::dropped_counts(&p).await.unwrap(), vec![("not_allowed".to_string(), 1)]);
        store::upsert_sender(&p, &Sender { address: "tg:99".into(), level: Level::External, skip_auth: false }).await.unwrap();
        assert!(matches!(ingest(&p, &cfg(), 99, 2, "請問怎麼用", false, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Queued(_)));
        let it = &store::list_items(&p, None).await.unwrap()[0];
        assert_eq!((it.sender_level.as_str(), it.status.as_str()), ("external", store::PENDING), "外部預設手動派送");
        assert_eq!(ingest(&p, &cfg(), 99, 2, "請問怎麼用", false, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Duplicate);
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 9];

    #[tokio::test]
    async fn photos_are_downloaded_only_for_allowed_senders() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let called = std::sync::atomic::AtomicBool::new(false);
        let r = ingest(&p, &cfg(), 99, 1, "", false, tmp.path(), true, || async {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            vec![Raw { name: "p.png".into(), bytes: PNG.to_vec() }]
        })
        .await
        .unwrap();
        assert_eq!(r, Ingested::Dropped("not_allowed"));
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst), "陌生人傳圖不可觸發下載");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn owner_photo_with_ask_caption_is_saved_and_a_photo_alone_is_not_a_question() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let photo = || async { vec![Raw { name: "p.jpg".into(), bytes: PNG.to_vec() }] };
        // 沒有 /ask 的圖片是主人的一般聊天，不入列。
        assert_eq!(ingest(&p, &cfg(), 1, 1, "隨手拍", true, tmp.path(), true, photo).await.unwrap(), Ingested::Dropped("not_for_us"));
        let Ingested::Queued(id) = ingest(&p, &cfg(), 1, 2, "/ask 這個畫面為什麼壞", true, tmp.path(), true, photo).await.unwrap() else { panic!() };
        let atts = store::list_attachments(&p, &id).await.unwrap();
        assert_eq!(atts.len(), 1);
        assert!(atts[0].filename.ends_with(".png"), "副檔名依內容而非訊息裡的 p.jpg：{}", atts[0].filename);
    }

    #[tokio::test]
    async fn allowlisted_chat_can_send_a_photo_with_no_caption() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        store::upsert_sender(&p, &Sender { address: "tg:7".into(), level: Level::External, skip_auth: false }).await.unwrap();
        let Ingested::Queued(id) = ingest(&p, &cfg(), 7, 3, "", false, tmp.path(), true, || async { vec![Raw { name: "x".into(), bytes: PNG.to_vec() }] }).await.unwrap() else { panic!() };
        assert_eq!(store::get_item(&p, &id).await.unwrap().unwrap().body, "（僅附圖）");
        assert_eq!(store::list_attachments(&p, &id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn disabled_means_nothing_is_stored_even_for_the_owner() {
        let p = pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let off = Config { telegram_enabled: false, ..Config::default() };
        assert_eq!(ingest(&p, &off, 1, 1, "/ask hi", true, tmp.path(), false, || async { vec![] }).await.unwrap(), Ingested::Dropped("not_for_us"));
        assert!(store::list_items(&p, None).await.unwrap().is_empty());
    }
}

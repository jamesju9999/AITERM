//! LINE 來的問題（純邏輯，不碰網路與 Tauri）。
//!
//! 身分由 LINE 保證（userId 無法偽造），但 webhook 是公開網址，所以每個請求都要先驗簽章。
//! 沒有「主人」：所有寄件者都必須在白名單，等級依白名單。
//! 設計：docs/superpowers/specs/2026-11-23-intake-line-design.md
use super::attach;
use super::line_store::{self, LineMessage, NewMessage};
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
    valid_with_prefix(id, &['u', 'U'])
}

/// 白名單可放的 LINE 編號：使用者 `U`、群組 `C`、多人聊天室 `R`，後面都是 32 位十六進位。
pub fn valid_line_id(id: &str) -> bool {
    valid_with_prefix(id, &['u', 'U', 'c', 'C', 'r', 'R'])
}

fn valid_with_prefix(id: &str, prefixes: &[char]) -> bool {
    id.len() == 33 && id.starts_with(prefixes) && id[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// 這個白名單地址是群組或聊天室（`line:c…`／`line:r…`）。
pub fn is_chat_key(sender: &str) -> bool {
    sender.strip_prefix("line:").is_some_and(|id| valid_line_id(id) && id.starts_with(['c', 'C', 'r', 'R']))
}

pub fn sender_key(user_id: &str) -> String {
    format!("line:{}", user_id.to_lowercase())
}

/// 從項目的 `sender`（`line:<小寫編號>`）還原 push 要用的編號（第一個字元改回大寫 `U`／`C`／`R`）。
pub fn reply_target(sender: &str) -> Option<String> {
    let id = sender.strip_prefix("line:")?;
    valid_line_id(id).then(|| format!("{}{}", id[..1].to_ascii_uppercase(), id[1..].to_lowercase()))
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

/// 一則收到的文字訊息。`chat` 是群組／聊天室編號（原樣，開頭大寫），一對一時為 `None`。
/// 群組裡的 `user_id` 可能是空字串（對方沒同意提供）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub user_id: String,
    pub chat: Option<String>,
    pub message_id: String,
    pub text: String,
    /// 訊息時間（秒）。
    pub ts: i64,
    /// 這則有 @ 到官方帳號。
    pub mentioned: bool,
}

/// 解析結果：文字訊息，以及機器人被拉進哪些群組／聊天室。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub messages: Vec<Incoming>,
    pub joined: Vec<String>,
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
    /// 毫秒。
    #[serde(default)]
    timestamp: i64,
}

#[derive(Deserialize)]
struct Source {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(rename = "userId")]
    user_id: Option<String>,
    #[serde(rename = "groupId")]
    group_id: Option<String>,
    #[serde(rename = "roomId")]
    room_id: Option<String>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    id: String,
    text: Option<String>,
    mention: Option<Mention>,
}

#[derive(Deserialize)]
struct Mention {
    #[serde(default)]
    mentionees: Vec<Mentionee>,
}

#[derive(Deserialize)]
struct Mentionee {
    #[serde(rename = "isSelf", default)]
    is_self: bool,
}

/// 只取文字訊息（一對一、群組、多人聊天室）與 `join`；貼圖、圖片、加好友等一律略過。壞掉的 JSON 當作沒有事件。
pub fn parse_webhook(body: &[u8]) -> Parsed {
    let Ok(w) = serde_json::from_slice::<Webhook>(body) else { return Parsed::default() };
    let mut out = Parsed::default();
    for e in w.events {
        let Some(src) = e.source else { continue };
        let chat = match src.kind.as_str() {
            "group" => src.group_id.clone().filter(|g| valid_line_id(g)),
            "room" => src.room_id.clone().filter(|r| valid_line_id(r)),
            _ => None,
        };
        let in_chat = matches!(src.kind.as_str(), "group" | "room");
        if e.kind == "join" {
            if let Some(c) = chat {
                out.joined.push(c);
            }
            continue;
        }
        let Some(msg) = e.message else { continue };
        if e.kind != "message" || msg.kind != "text" || msg.id.is_empty() {
            continue;
        }
        let Some(text) = msg.text else { continue };
        let ts = if e.timestamp > 0 { e.timestamp / 1000 } else { chrono::Utc::now().timestamp() };
        let mentioned = msg.mention.is_some_and(|m| m.mentionees.iter().any(|x| x.is_self));
        if in_chat {
            let Some(chat) = chat else { continue };
            // 群組裡的 userId 可能沒有；有的話要合法。
            let user_id = match src.user_id {
                Some(u) if valid_user_id(&u) => u,
                Some(_) => continue,
                None => String::new(),
            };
            out.messages.push(Incoming { user_id, chat: Some(chat), message_id: msg.id, text, ts, mentioned });
        } else if src.kind == "user" {
            let Some(user_id) = src.user_id.filter(|u| valid_user_id(u)) else { continue };
            out.messages.push(Incoming { user_id, chat: None, message_id: msg.id, text, ts, mentioned: false });
        }
    }
    out
}

/// 只取文字訊息。
pub fn parse_events(body: &[u8]) -> Vec<Incoming> {
    parse_webhook(body).messages
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

/// 開題時帶的前情：最近幾則、多久以內、整段內文的字元上限。
pub const CONTEXT_LIMIT: i64 = 30;
pub const CONTEXT_WINDOW_SECS: i64 = 24 * 3600;
pub const BODY_MAX_CHARS: usize = 6000;
/// 客戶最後一則訊息過了這麼久沒有新訊息，才把這一批當成一個問題（讓客戶把話講完）。
pub const QUIET_SECS: i64 = 90;

fn stamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map(|d| d.with_timezone(&chrono::Local).format("%m/%d %H:%M").to_string()).unwrap_or_default()
}

fn line_of(m: &LineMessage) -> String {
    let who = if m.name.is_empty() { "（未知）" } else { m.name.as_str() };
    format!("[{} {}] {}", stamp(m.ts), who, m.text.replace('\n', "\n    "))
}

/// 去掉開頭的 `@名稱`，讓標題是真正的問題。
fn strip_mention(text: &str) -> &str {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix('@') {
        let after = rest.split_once(char::is_whitespace).map(|(_, r)| r.trim_start()).unwrap_or("");
        if !after.is_empty() {
            return after;
        }
    }
    t
}

/// 開題的內文：前情（舊的在前）＋需要處理的訊息。總長超過上限時，先捨棄最舊的前情。
pub fn build_body(group_name: &str, context: &[LineMessage], batch: &[LineMessage]) -> String {
    let who = if group_name.is_empty() { "（未命名）" } else { group_name };
    let ask: String = batch.iter().map(line_of).collect::<Vec<_>>().join("\n").chars().take(BODY_MAX_CHARS).collect();
    let mut room = BODY_MAX_CHARS.saturating_sub(ask.chars().count());
    let mut kept: Vec<String> = Vec::new();
    for m in context.iter().rev() {
        let l = line_of(m);
        let n = l.chars().count() + 1;
        if n > room {
            break;
        }
        room -= n;
        kept.push(l);
    }
    kept.reverse();
    let mut out = format!("（以下是 LINE 群組「{who}」最近的對話，舊的在前；最後一段才是需要處理的提問。）\n");
    if !kept.is_empty() {
        out.push_str("── 前情 ──\n");
        out.push_str(&kept.join("\n"));
        out.push('\n');
    }
    out.push_str("── 提問 ──\n");
    out.push_str(&ask);
    out
}

pub struct Opening<'a> {
    pub project: &'a Candidate,
    pub level: Level,
    /// 群組代號（`line:c…`）。
    pub key: &'a str,
    pub group_name: &'a str,
    pub context: Vec<LineMessage>,
    pub batch: Vec<LineMessage>,
    pub message_id: String,
}

/// 把一批群組訊息開成一題。成功後這批（資料列 id 最大者以前）標為已處理。
pub async fn open_item(pool: &SqlitePool, o: &Opening<'_>, attach_root: &Path) -> Result<Ingested, sqlx::Error> {
    let Some(first) = o.batch.first() else { return Ok(Ingested::Dropped("not_for_us")) };
    let body = build_body(o.group_name, &o.context, &o.batch);
    let subject = subject_of(strip_mention(&first.text));
    let r = queue::enqueue(pool, &o.project.cfg, &o.project.id, o.level, "line", o.key, &subject, &body, &o.message_id, attach_root, Vec::<attach::Raw>::new()).await?;
    if let Ingested::Queued(id) = &r {
        let label = match (o.group_name.is_empty(), first.name.is_empty()) {
            (false, false) => format!("{} / {}", o.group_name, first.name),
            (false, true) => o.group_name.to_string(),
            (true, _) => first.name.clone(),
        };
        store::set_sender_name(pool, id, &label, o.key).await?;
    }
    if let Some(last) = o.batch.iter().map(|m| m.id).filter(|id| *id > 0).max() {
        line_store::mark_handled(pool, o.key, last).await?;
    }
    Ok(r)
}

pub struct GroupMsg<'a> {
    /// 群組／聊天室編號（原樣）。
    pub chat: &'a str,
    pub user_id: &'a str,
    pub name: &'a str,
    pub message_id: &'a str,
    pub text: &'a str,
    pub ts: i64,
    pub mentioned: bool,
}

/// 這個群組／使用者代號（`line:…`）是否在任何啟用 LINE 的專案白名單內。
pub async fn is_whitelisted(pool: &SqlitePool, cands: &[Candidate], key: &str) -> Result<bool, sqlx::Error> {
    for c in cands.iter().filter(|c| c.cfg.line_enabled) {
        if store::list_senders(pool, &c.id).await?.iter().any(|s| rules::normalize(&s.address) == key) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 群組訊息：群組在（啟用 LINE 的）專案白名單內才保存；有 @ 到機器人就立即開題。
/// 沒 @ 的訊息只存起來，由 `triage_chat` 判斷要不要自動開題。
/// 白名單外一律不存也不回覆（編號由呼叫端記到「未知來源」）；`retention_days` 為 0 表示不保存，只剩 @ 的單則開題。
pub async fn route_group(pool: &SqlitePool, cands: &[Candidate], m: &GroupMsg<'_>, group_name: &str, retention_days: u32, now: i64, attach_root: &Path) -> Result<Ingested, sqlx::Error> {
    let text = m.text.trim();
    let enabled: Vec<&Candidate> = cands.iter().filter(|c| c.cfg.line_enabled).collect();
    if enabled.is_empty() || text.is_empty() {
        return Ok(Ingested::Dropped("not_for_us"));
    }
    let key = sender_key(m.chat);
    let mut allowed: Vec<(&Candidate, Level)> = Vec::new();
    for c in &enabled {
        if let Some(s) = store::list_senders(pool, &c.id).await?.iter().find(|s| rules::normalize(&s.address) == key) {
            allowed.push((c, s.level));
        }
    }
    let (project, level) = match allowed.as_slice() {
        [] => return Ok(Ingested::Dropped("not_allowed")),
        [(c, l)] => (*c, *l),
        _ => {
            store::bump_dropped(pool, "", "ambiguous_project").await?;
            return Ok(Ingested::Dropped("ambiguous_project"));
        }
    };
    let new = NewMessage { chat: &key, msg_id: m.message_id, user_id: &m.user_id.to_lowercase(), name: m.name, text, ts: m.ts, outgoing: false };
    let row = if retention_days > 0 {
        match line_store::insert_message(pool, &new).await? {
            Some(id) => Some(id),
            None => return Ok(Ingested::Duplicate),
        }
    } else {
        None
    };
    if !m.mentioned {
        return Ok(Ingested::Dropped(if row.is_some() { "recorded" } else { "not_for_us" }));
    }
    let this = LineMessage { id: row.unwrap_or(0), chat: key.clone(), msg_id: m.message_id.into(), user_id: m.user_id.to_lowercase(), name: m.name.into(), text: text.into(), ts: m.ts, outgoing: 0, handled: 0 };
    let context = match row {
        Some(id) => line_store::context_before(pool, &key, id, now - CONTEXT_WINDOW_SECS, CONTEXT_LIMIT).await?,
        None => Vec::new(),
    };
    let message_id = format!("{key}:{}", m.message_id);
    open_item(pool, &Opening { project, level, key: &key, group_name, context, batch: vec![this], message_id }, attach_root).await
}

/// 自動開題的批次判斷結果。
#[derive(Debug, PartialEq, Eq)]
pub enum Triage {
    /// 沒有待處理的訊息。
    Idle,
    /// 客戶還在講，等安靜之後再看。
    Wait,
    /// 全是我方人員的話，或人員／AI 已經在回覆：已標成已處理，不開題。
    Skipped,
    /// 客戶的一批訊息（舊的在前）等著判斷是不是新問題；`upto` 是這批與其間我方訊息的最大資料列 id。
    Candidate { messages: Vec<LineMessage>, upto: i64 },
}

/// 看一個群組有沒有該自動開題的客戶訊息。`staff` 是我方人員的 userId（小寫）。
/// 回傳 `Skipped` 時已把相關訊息標成已處理。
pub async fn triage_chat(pool: &SqlitePool, key: &str, staff: &std::collections::HashSet<String>, quiet_secs: i64, now: i64) -> Result<Triage, sqlx::Error> {
    let all = line_store::unhandled(pool, key).await?;
    let Some(last_all) = all.last().map(|m| m.id) else { return Ok(Triage::Idle) };
    let is_staff = |m: &LineMessage| !m.user_id.is_empty() && staff.contains(&m.user_id.to_lowercase());
    let customers: Vec<LineMessage> = all.iter().filter(|m| !is_staff(m)).cloned().collect();
    let (Some(first), Some(last)) = (customers.first(), customers.last()) else {
        line_store::mark_handled(pool, key, last_all).await?;
        return Ok(Triage::Skipped);
    };
    if now - last.ts < quiet_secs {
        return Ok(Triage::Wait);
    }
    // 這批客戶訊息之後，我方人員或 AI 已經回過 → 有人在處理了。
    if line_store::after(pool, key, first.id).await?.iter().any(|m| m.outgoing == 1 || is_staff(m)) {
        line_store::mark_handled(pool, key, last_all).await?;
        return Ok(Triage::Skipped);
    }
    Ok(Triage::Candidate { messages: customers, upto: last_all })
}

/// 判定為新問題後，把這批客戶訊息開成一題（前情＝這批之前的對話）。`upto` 以前的訊息全部標為已處理。
pub async fn open_candidate(pool: &SqlitePool, project: &Candidate, level: Level, key: &str, group_name: &str, messages: Vec<LineMessage>, upto: i64, now: i64, attach_root: &Path) -> Result<Ingested, sqlx::Error> {
    let Some(first) = messages.first() else { return Ok(Ingested::Dropped("not_for_us")) };
    let context = line_store::context_before(pool, key, first.id, now - CONTEXT_WINDOW_SECS, CONTEXT_LIMIT).await?;
    let message_id = format!("{key}:auto:{}", first.msg_id);
    let r = open_item(pool, &Opening { project, level, key, group_name, context, batch: messages, message_id }, attach_root).await?;
    line_store::mark_handled(pool, key, upto).await?;
    Ok(r)
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
        let group = "Cb345e5d492c48c02a3eaa279924c52d2";
        assert_eq!(reply_target(&sender_key(group)).as_deref(), Some(group), "群組編號開頭是大寫 C");
        assert!(is_chat_key(&sender_key(group)) && !is_chat_key(&sender_key(UID)));
        assert!(valid_line_id(group) && valid_line_id("Rb345e5d492c48c02a3eaa279924c52d2") && !valid_user_id(group));
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

    // ───────── 群組 ─────────

    const GROUP: &str = "Cb345e5d492c48c02a3eaa279924c52d2";
    const STAFF: &str = "U1111111111111111111111111111111a";
    const CUSTOMER: &str = "U2222222222222222222222222222222b";

    fn group_event(kind: &str, user: &str, id: &str, text: &str, mention_self: Option<bool>, ts_ms: i64) -> String {
        let mention = match mention_self {
            Some(me) => format!(r#","mention":{{"mentionees":[{{"index":0,"length":3,"type":"user","isSelf":{me}}}]}}"#),
            None => String::new(),
        };
        format!(r#"{{"type":"message","timestamp":{ts_ms},"source":{{"type":"{kind}","groupId":"{GROUP}","roomId":"{GROUP}","userId":"{user}"}},"message":{{"type":"text","id":"{id}","text":"{text}"{mention}}}}}"#)
    }

    #[test]
    fn group_events_carry_the_chat_the_time_and_whether_the_bot_was_mentioned() {
        let body = format!(
            r#"{{"events":[{},{},{},{{"type":"join","source":{{"type":"group","groupId":"{GROUP}"}}}},{{"type":"join","source":{{"type":"group","groupId":"bad"}}}},{{"type":"message","source":{{"type":"group","groupId":"{GROUP}"}},"message":{{"type":"text","id":"m9","text":"沒有 userId"}}}}]}}"#,
            group_event("group", CUSTOMER, "m1", "電腦開不了", None, 1_700_000_000_000),
            group_event("group", STAFF, "m2", "@AI 幫忙看", Some(true), 1_700_000_005_000),
            group_event("group", CUSTOMER, "m3", "@別人 你好", Some(false), 1_700_000_006_000),
        );
        let p = parse_webhook(body.as_bytes());
        assert_eq!(p.joined, vec![GROUP.to_string()], "壞編號的 join 略過");
        let m = &p.messages;
        assert_eq!(m.len(), 4);
        assert_eq!((m[0].chat.as_deref(), m[0].user_id.as_str(), m[0].ts, m[0].mentioned), (Some(GROUP), CUSTOMER, 1_700_000_000, false));
        assert!(m[1].mentioned && !m[2].mentioned, "只有 isSelf 才算 @ 到機器人");
        assert_eq!((m[3].user_id.as_str(), m[3].chat.as_deref()), ("", Some(GROUP)), "群組裡可以沒有 userId");
        // 一對一照舊：沒有 chat、不會被當成 @。
        let one = format!(r#"{{"events":[{{"type":"message","source":{{"type":"user","userId":"{UID}"}},"message":{{"type":"text","id":"1","text":"hi"}}}}]}}"#);
        let m = parse_webhook(one.as_bytes()).messages;
        assert_eq!((m[0].chat.as_deref(), m[0].mentioned), (None, false));
        // 多人聊天室（room）同樣處理，用 roomId。
        let room = r#"{"events":[{"type":"message","source":{"type":"room","roomId":"Rb345e5d492c48c02a3eaa279924c52d2","userId":"U2222222222222222222222222222222b"},"message":{"type":"text","id":"r1","text":"聊天室"}}]}"#;
        assert_eq!(parse_webhook(room.as_bytes()).messages[0].chat.as_deref(), Some("Rb345e5d492c48c02a3eaa279924c52d2"));
    }

    async fn allow_key(p: &SqlitePool, project: &str, addr: &str, level: Level) {
        store::upsert_sender(p, project, &Sender { address: addr.into(), level, skip_auth: false }).await.unwrap();
    }
    fn gm<'a>(id: &'a str, user: &'a str, name: &'a str, text: &'a str, ts: i64, mentioned: bool) -> GroupMsg<'a> {
        GroupMsg { chat: GROUP, user_id: user, name, message_id: id, text, ts, mentioned }
    }
    async fn grp(p: &SqlitePool, cands: &[Candidate], m: &GroupMsg<'_>, retention: u32, now: i64) -> Ingested {
        let tmp = tempfile::tempdir().unwrap();
        route_group(p, cands, m, "兆強客服群", retention, now, tmp.path()).await.unwrap()
    }
    fn gkey() -> String {
        sender_key(GROUP)
    }

    #[tokio::test]
    async fn a_group_that_is_not_whitelisted_stores_nothing_at_all() {
        let p = pool().await;
        let r = grp(&p, &[cand("", true)], &gm("1", CUSTOMER, "客戶", "我是陌生群組", 100, true), 30, 200).await;
        assert_eq!(r, Ingested::Dropped("not_allowed"));
        assert_eq!(line_store::message_count(&p, &gkey()).await.unwrap(), 0, "白名單外的群組不保存任何訊息");
        assert!(store::list_items(&p, "", None).await.unwrap().is_empty());
        assert!(!is_whitelisted(&p, &[cand("", true)], &gkey()).await.unwrap());
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        assert!(is_whitelisted(&p, &[cand("", true)], &gkey()).await.unwrap());
        assert!(!is_whitelisted(&p, &[cand("", false)], &gkey()).await.unwrap(), "LINE 沒啟用的專案不算");
    }

    #[tokio::test]
    async fn messages_without_a_mention_are_recorded_but_open_nothing_and_redelivery_is_ignored() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        let c = [cand("", true)];
        assert_eq!(grp(&p, &c, &gm("1", CUSTOMER, "客戶", "電腦開不了機", 100, false), 30, 200).await, Ingested::Dropped("recorded"));
        assert_eq!(grp(&p, &c, &gm("1", CUSTOMER, "客戶", "電腦開不了機", 100, false), 30, 200).await, Ingested::Duplicate);
        assert_eq!(line_store::message_count(&p, &gkey()).await.unwrap(), 1);
        assert!(store::list_items(&p, "", None).await.unwrap().is_empty());
        // 保存天數 0：什麼都不存，也不開題。
        let p2 = pool().await;
        allow_key(&p2, "", &format!("line:{GROUP}"), Level::External).await;
        assert_eq!(grp(&p2, &c, &gm("1", CUSTOMER, "客戶", "hi", 100, false), 0, 200).await, Ingested::Dropped("not_for_us"));
        assert_eq!(line_store::message_count(&p2, &gkey()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_mention_opens_an_item_with_the_earlier_conversation_and_marks_it_handled() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        let c = [cand("", true)];
        grp(&p, &c, &gm("1", CUSTOMER, "介琳", "ARESPP 開不了加密檔", 100, false), 30, 200).await;
        grp(&p, &c, &gm("2", STAFF, "旭婷", "請問版本是？", 110, false), 30, 200).await;
        let Ingested::Queued(id) = grp(&p, &c, &gm("3", CUSTOMER, "介琳", "@AI助理 6.0.9273，請幫我看", 120, true), 30, 200).await else { panic!() };
        let it = store::get_item(&p, &id).await.unwrap().unwrap();
        assert_eq!((it.source.as_str(), it.sender.as_str(), it.sender_level.as_str(), it.status.as_str()), ("line", gkey().as_str(), "external", store::PENDING));
        assert_eq!(it.subject, "6.0.9273，請幫我看", "標題去掉開頭的 @名稱");
        assert_eq!(it.sender_name.as_deref(), Some("兆強客服群 / 介琳"));
        assert_eq!(it.message_id, format!("{}:3", gkey()));
        let b = &it.body;
        let (ctx, ask) = b.split_once("── 提問 ──").unwrap();
        assert!(ctx.contains("兆強客服群") && ctx.contains("── 前情 ──") && ctx.contains("ARESPP 開不了加密檔") && ctx.contains("旭婷") && ctx.contains("請問版本是？"), "{b}");
        assert!(ask.contains("6.0.9273，請幫我看") && !ask.contains("開不了加密檔"), "{b}");
        // 這則之後，同一群組的這批已標成已處理。
        assert!(line_store::unhandled(&p, &gkey()).await.unwrap().iter().all(|m| m.msg_id != "3"));
        // 同一則 @ 重送：不重複開題。
        assert_eq!(grp(&p, &c, &gm("3", CUSTOMER, "介琳", "@AI助理 6.0.9273，請幫我看", 120, true), 30, 200).await, Ingested::Duplicate);
        assert_eq!(store::list_items(&p, "", None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn context_respects_the_time_window_the_count_limit_and_the_body_cap() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::Internal).await;
        let c = [cand("", true)];
        let now = 10 * 86_400;
        grp(&p, &c, &gm("old", CUSTOMER, "客戶", "三天前的話", now - 3 * 86_400, false), 30, now).await;
        for i in 0..40 {
            grp(&p, &c, &gm(&format!("m{i}"), CUSTOMER, "客戶", &format!("第{i}則"), now - 3600 + i, false), 30, now).await;
        }
        let Ingested::Queued(id) = grp(&p, &c, &gm("q", CUSTOMER, "客戶", "@AI 請看", now, true), 30, now).await else { panic!() };
        let b = store::get_item(&p, &id).await.unwrap().unwrap().body;
        assert!(!b.contains("三天前的話"), "超過 24 小時不帶");
        assert!(!b.contains("第9則") && b.contains("第10則") && b.contains("第39則"), "最近 30 則：{b}");
        // 內文上限：超長的前情先捨棄最舊的。
        let long = "字".repeat(2000);
        let m = |i: i64, t: &str| LineMessage { id: i, chat: gkey(), msg_id: format!("x{i}"), user_id: "u".into(), name: "客戶".into(), text: t.into(), ts: 1, outgoing: 0, handled: 0 };
        let ctx: Vec<LineMessage> = (1..=5).map(|i| m(i, &long)).collect();
        let body = build_body("群", &ctx, &[m(9, "最後的提問")]);
        assert!(body.chars().count() <= BODY_MAX_CHARS + 200, "{}", body.chars().count());
        assert!(body.contains("最後的提問"), "提問本身一定保留");
        assert!(body.matches(&long).count() < 5, "最舊的前情被捨棄");
    }

    #[tokio::test]
    async fn a_group_in_several_projects_is_ambiguous_and_a_disabled_project_does_not_count() {
        let p = pool().await;
        let cands = [cand("a", true), cand("b", true)];
        allow_key(&p, "a", &format!("line:{GROUP}"), Level::External).await;
        allow_key(&p, "b", &format!("line:{GROUP}"), Level::External).await;
        assert_eq!(grp(&p, &cands, &gm("1", CUSTOMER, "客戶", "誰的", 1, true), 30, 2).await, Ingested::Dropped("ambiguous_project"));
        assert_eq!(line_store::message_count(&p, &gkey()).await.unwrap(), 0);
        let only_a = [cand("a", true), cand("b", false)];
        assert!(matches!(grp(&p, &only_a, &gm("2", CUSTOMER, "客戶", "請看", 1, true), 30, 2).await, Ingested::Queued(_)));
        assert_eq!(store::list_items(&p, "a", None).await.unwrap().len(), 1);
    }

    // ───────── 自動開題：批次判斷 ─────────

    fn staff_set() -> std::collections::HashSet<String> {
        [STAFF.to_lowercase()].into_iter().collect()
    }
    async fn say(p: &SqlitePool, id: &str, user: &str, name: &str, text: &str, ts: i64) {
        let m = gm(id, user, name, text, ts, false);
        let r = grp(p, &[cand("", true)], &m, 30, ts).await;
        assert_eq!(r, Ingested::Dropped("recorded"));
    }

    #[tokio::test]
    async fn triage_waits_for_quiet_merges_a_burst_and_then_offers_it_as_a_candidate() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 1000).await.unwrap(), Triage::Idle);
        say(&p, "1", CUSTOMER, "客戶", "電腦開不了機", 1000).await;
        say(&p, "2", CUSTOMER, "客戶", "加密檔打不開", 1030).await;
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 1100).await.unwrap(), Triage::Wait, "最後一則才 70 秒前，客戶可能還在講");
        let Triage::Candidate { messages, upto } = triage_chat(&p, &gkey(), &staff_set(), 90, 1121).await.unwrap() else { panic!() };
        assert_eq!(messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["電腦開不了機", "加密檔打不開"], "一批合成一題");
        assert_eq!(upto, messages.last().unwrap().id);
        // 候選不會自己標成已處理：沒處理前下一輪仍然看得到。
        assert!(matches!(triage_chat(&p, &gkey(), &staff_set(), 90, 1200).await.unwrap(), Triage::Candidate { .. }));
    }

    #[tokio::test]
    async fn triage_skips_when_staff_or_the_ai_already_answered_or_when_only_staff_spoke() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        say(&p, "1", CUSTOMER, "客戶", "請問怎麼設定", 1000).await;
        say(&p, "2", STAFF, "旭婷", "我來看", 1010).await;
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 2000).await.unwrap(), Triage::Skipped, "人員已在回覆");
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 2000).await.unwrap(), Triage::Idle, "已標成已處理");
        // 只有人員講話。
        say(&p, "3", STAFF, "旭婷", "內部備忘", 3000).await;
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 4000).await.unwrap(), Triage::Skipped);
        // AI 自己已回覆（outgoing）也算有人處理。
        say(&p, "4", CUSTOMER, "客戶", "又一個問題", 5000).await;
        line_store::insert_message(&p, &line_store::NewMessage { chat: &gkey(), msg_id: "ai-1", user_id: "", name: "AI 助理", text: "已回覆", ts: 5010, outgoing: true }).await.unwrap();
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 6000).await.unwrap(), Triage::Skipped);
        // 沒有人員名單時，所有人都是客戶。
        say(&p, "5", STAFF, "旭婷", "我是誰", 7000).await;
        assert!(matches!(triage_chat(&p, &gkey(), &Default::default(), 90, 8000).await.unwrap(), Triage::Candidate { .. }));
    }

    #[tokio::test]
    async fn opening_a_candidate_uses_the_earlier_talk_as_context_and_marks_everything_up_to_it_handled() {
        let p = pool().await;
        allow_key(&p, "", &format!("line:{GROUP}"), Level::External).await;
        say(&p, "0", STAFF, "旭婷", "昨天已更新到最新版", 500).await;
        say(&p, "1", CUSTOMER, "介琳", "今天又打不開了", 1000).await;
        say(&p, "2", CUSTOMER, "介琳", "畫面一直轉圈", 1010).await;
        let Triage::Candidate { messages, upto } = triage_chat(&p, &gkey(), &staff_set(), 90, 2000).await.unwrap() else { panic!() };
        let tmp = tempfile::tempdir().unwrap();
        let project = cand("", true);
        let Ingested::Queued(id) = open_candidate(&p, &project, Level::External, &gkey(), "兆強客服群", messages, upto, 2000, tmp.path()).await.unwrap() else { panic!() };
        let it = store::get_item(&p, &id).await.unwrap().unwrap();
        assert_eq!(it.message_id, format!("{}:auto:1", gkey()));
        let (ctx, ask) = it.body.split_once("── 提問 ──").unwrap();
        assert!(ctx.contains("昨天已更新到最新版") && ask.contains("今天又打不開了") && ask.contains("畫面一直轉圈"), "{}", it.body);
        assert!(line_store::unhandled(&p, &gkey()).await.unwrap().is_empty());
        assert_eq!(triage_chat(&p, &gkey(), &staff_set(), 90, 3000).await.unwrap(), Triage::Idle);
        assert!(store::has_open_for_sender(&p, "", &gkey()).await.unwrap());
    }
}

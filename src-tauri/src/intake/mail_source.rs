//! 把郵件帳號收到的原始信件轉成佇列項目。
//!
//! 先只看標頭做判斷；通過才讀內文入庫。不符合的信不寫進 `intake.db`
//! （郵件分頁自己的快取不受影響），只累計丟棄計數。
use super::config::{Config, DispatchMode};
use super::rules::{self, Decision, Headers, Level};
use super::store::{self, NewItem};
use mail_parser::{HeaderValue, MessageParser};
use sqlx::SqlitePool;

pub struct Parsed {
    pub headers: Headers,
    pub message_id: String,
    pub body: String,
}

fn addrs(a: Option<&mail_parser::Address<'_>>) -> Vec<String> {
    a.map(|a| a.iter().filter_map(|x| x.address().map(|s| s.to_string())).collect()).unwrap_or_default()
}

pub fn parse(raw: &[u8]) -> Option<Parsed> {
    let m = MessageParser::default().parse(raw)?;
    let from = addrs(m.from()).into_iter().next().unwrap_or_default();
    let mut recipients = addrs(m.to());
    recipients.extend(addrs(m.cc()));
    for v in m.header_values("Delivered-To") {
        if let HeaderValue::Text(t) = v {
            recipients.push(t.to_string());
        }
    }
    let auth_results = m
        .header_values("Authentication-Results")
        .filter_map(|v| match v {
            HeaderValue::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect();
    let message_id = m.message_id().map(|s| s.to_string()).unwrap_or_default();
    let body = m.body_text(0).or_else(|| m.body_html(0)).map(|s| s.to_string()).unwrap_or_default();
    Some(Parsed {
        headers: Headers { from, recipients, subject: m.subject().unwrap_or("").to_string(), auth_results },
        message_id,
        body,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ingested {
    Queued(String),
    Duplicate,
    Dropped(&'static str),
}

fn drop_reason(d: &rules::Drop) -> &'static str {
    match d {
        rules::Drop::NotForUs => "not_for_us",
        rules::Drop::NotAllowed => "not_allowed",
        rules::Drop::AuthFailed => "auth_failed",
        rules::Drop::NoAuthHeader => "no_auth_header",
    }
}

pub async fn ingest(pool: &SqlitePool, cfg: &Config, raw: &[u8]) -> Result<Ingested, sqlx::Error> {
    let Some(p) = parse(raw) else { return Ok(Ingested::Dropped("unparsable")) };
    let senders = store::list_senders(pool).await?;
    let level = match rules::decide(&cfg.rules(), &senders, &p.headers) {
        Decision::Accept(l) => l,
        Decision::Drop(r) => {
            // 「不是給我們的信」不計入：那是信箱裡的其他所有信，數字沒有意義。
            if r != rules::Drop::NotForUs {
                store::bump_dropped(pool, drop_reason(&r)).await?;
            }
            return Ok(Ingested::Dropped(drop_reason(&r)));
        }
    };
    let sender = rules::normalize(&p.headers.from);
    let since = chrono::Utc::now().timestamp() - 3600;
    if cfg.rate_limit_per_hour > 0 && store::count_from_sender_since(pool, &sender, since).await? >= cfg.rate_limit_per_hour as i64 {
        store::bump_dropped(pool, "rate_limited").await?;
        return Ok(Ingested::Dropped("rate_limited"));
    }
    // 沒有 Message-ID 的信無法去重，用內容雜湊代替，避免重複輪詢造成重複項目。
    let mid = if p.message_id.is_empty() {
        format!("nomid:{}:{}", sender, fnv(raw))
    } else {
        p.message_id.clone()
    };
    let mode = match level {
        Level::Internal => cfg.dispatch_internal,
        Level::External => cfg.dispatch_external,
    };
    let status = if mode == DispatchMode::Auto { store::QUEUED } else { store::PENDING };
    let item = store::insert_item(pool, NewItem {
        source: "email", sender: &sender, level, subject: &p.headers.subject, body: &p.body, message_id: &mid, status,
    }).await?;
    Ok(match item {
        Some(i) => Ingested::Queued(i.id),
        None => Ingested::Duplicate,
    })
}

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325, |h, x| (h ^ *x as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::rules::{MatchMode, Sender};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let p = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        store::init_schema(&p).await.unwrap();
        p
    }
    fn cfg() -> Config {
        Config { mode: MatchMode::AliasOrTag, alias: "me+aiterm@gmail.com".into(), ..Config::default() }
    }
    fn mail(from: &str, to: &str, subject: &str, mid: &str, auth: Option<&str>) -> Vec<u8> {
        let a = auth.map(|a| format!("Authentication-Results: {a}\r\n")).unwrap_or_default();
        format!("{a}From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{mid}>\r\n\r\nbody text\r\n").into_bytes()
    }
    const PASS: &str = "mx.google.com; spf=pass smtp.mailfrom=bob@corp.com";

    async fn allow(p: &SqlitePool, a: &str, l: Level) {
        store::upsert_sender(p, &Sender { address: a.into(), level: l, skip_auth: false }).await.unwrap();
    }

    #[tokio::test]
    async fn internal_goes_straight_to_queued_external_waits_for_approval() {
        let p = pool().await;
        allow(&p, "me@home.com", Level::Internal).await;
        allow(&p, "bob@corp.com", Level::External).await;
        let a = ingest(&p, &cfg(), &mail("me@home.com", "x@y.z", "[AITerm] hi", "1@h", None)).await.unwrap();
        let b = ingest(&p, &cfg(), &mail("bob@corp.com", "me+aiterm@gmail.com", "q", "2@c", Some(PASS))).await.unwrap();
        assert!(matches!(a, Ingested::Queued(_)) && matches!(b, Ingested::Queued(_)));
        assert_eq!(store::list_items(&p, Some(store::QUEUED)).await.unwrap().len(), 1);
        assert_eq!(store::list_items(&p, Some(store::PENDING)).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn non_matching_and_unlisted_mail_is_never_stored() {
        let p = pool().await;
        allow(&p, "bob@corp.com", Level::External).await;
        let unrelated = ingest(&p, &cfg(), &mail("bob@corp.com", "x@y.z", "newsletter", "3@c", Some(PASS))).await.unwrap();
        assert_eq!(unrelated, Ingested::Dropped("not_for_us"));
        let stranger = ingest(&p, &cfg(), &mail("evil@x.com", "me+aiterm@gmail.com", "q", "4@x", Some(PASS))).await.unwrap();
        assert_eq!(stranger, Ingested::Dropped("not_allowed"));
        assert!(store::list_items(&p, None).await.unwrap().is_empty());
        // 只有「被點名但不被允許」才計數；其他信箱內容不計。
        assert_eq!(store::dropped_counts(&p).await.unwrap(), vec![("not_allowed".to_string(), 1)]);
    }

    #[tokio::test]
    async fn forged_external_is_dropped_and_duplicate_is_detected() {
        let p = pool().await;
        allow(&p, "bob@corp.com", Level::External).await;
        let forged = mail("bob@corp.com", "me+aiterm@gmail.com", "q", "5@c", Some("mx.google.com; spf=fail smtp.mailfrom=bob@corp.com"));
        assert_eq!(ingest(&p, &cfg(), &forged).await.unwrap(), Ingested::Dropped("auth_failed"));
        let ok = mail("bob@corp.com", "me+aiterm@gmail.com", "q", "6@c", Some(PASS));
        assert!(matches!(ingest(&p, &cfg(), &ok).await.unwrap(), Ingested::Queued(_)));
        assert_eq!(ingest(&p, &cfg(), &ok).await.unwrap(), Ingested::Duplicate);
    }

    #[tokio::test]
    async fn rate_limit_drops_excess() {
        let p = pool().await;
        allow(&p, "me@home.com", Level::Internal).await;
        let c = Config { rate_limit_per_hour: 2, ..cfg() };
        for i in 0..2 {
            assert!(matches!(ingest(&p, &c, &mail("me@home.com", "me+aiterm@gmail.com", "s", &format!("{i}@h"), None)).await.unwrap(), Ingested::Queued(_)));
        }
        assert_eq!(ingest(&p, &c, &mail("me@home.com", "me+aiterm@gmail.com", "s", "9@h", None)).await.unwrap(), Ingested::Dropped("rate_limited"));
    }

    #[test]
    fn parse_reads_headers_and_orders_auth_results() {
        let raw = b"Authentication-Results: own.example; spf=pass\r\nAuthentication-Results: spoof.example; spf=pass\r\nFrom: A <a@b.c>\r\nTo: x@y.z\r\nDelivered-To: me+aiterm@gmail.com\r\nSubject: [AITerm] t\r\nMessage-ID: <m@b>\r\n\r\nhello";
        let p = parse(raw).unwrap();
        assert!(p.headers.auth_results[0].starts_with("own.example"));
        assert_eq!(p.headers.from, "a@b.c");
        assert!(p.headers.recipients.contains(&"me+aiterm@gmail.com".to_string()));
        assert_eq!(p.message_id, "m@b");
    }
}

//! 各管道共用的入列：頻率限制、依層級決定起始狀態、去重。
use super::config::{Config, DispatchMode};
use super::rules::{self, Level};
use super::store::{self, NewItem};
use sqlx::SqlitePool;

#[derive(Debug, PartialEq, Eq)]
pub enum Ingested {
    Queued(String),
    Duplicate,
    Dropped(&'static str),
}

pub fn drop_reason(d: &rules::Drop) -> &'static str {
    match d {
        rules::Drop::NotForUs => "not_for_us",
        rules::Drop::NotAllowed => "not_allowed",
        rules::Drop::AuthFailed => "auth_failed",
        rules::Drop::NoAuthHeader => "no_auth_header",
    }
}


#[allow(clippy::too_many_arguments)]
pub async fn enqueue(
    pool: &SqlitePool,
    cfg: &Config,
    level: Level,
    source: &str,
    sender: &str,
    subject: &str,
    body: &str,
    message_id: &str,
) -> Result<Ingested, sqlx::Error> {
    let since = chrono::Utc::now().timestamp() - 3600;
    if cfg.rate_limit_per_hour > 0 && store::count_from_sender_since(pool, sender, since).await? >= cfg.rate_limit_per_hour as i64 {
        store::bump_dropped(pool, "rate_limited").await?;
        return Ok(Ingested::Dropped("rate_limited"));
    }
    let mode = match level {
        Level::Internal => cfg.dispatch_internal,
        Level::External => cfg.dispatch_external,
    };
    let status = if mode == DispatchMode::Auto { store::QUEUED } else { store::PENDING };
    let item = store::insert_item(pool, NewItem { source, sender, level, subject, body, message_id, status }).await?;
    Ok(match item {
        Some(i) => Ingested::Queued(i.id),
        None => Ingested::Duplicate,
    })
}

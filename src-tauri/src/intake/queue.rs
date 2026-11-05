//! 各管道共用的入列：頻率限制、依層級決定起始狀態、去重。
use super::attach::{self, Raw};
use super::config::{Config, DispatchMode};
use super::rules::{self, Level};
use super::store::{self, NewItem};
use sqlx::SqlitePool;
use std::path::Path;

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
    attach_root: &Path,
    files: Vec<Raw>,
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
    // 附圖要在項目入庫（可能立刻被領取）之前就存好，Agent 才一定看得到。
    let id = uuid::Uuid::new_v4().to_string();
    let (saved, skipped) = if files.is_empty() { (vec![], 0) } else { attach::save_all(attach_root, &id, files) };
    let body_owned;
    let body = if skipped > 0 {
        body_owned = format!("{body}\n\n（另有 {skipped} 個附件因格式不是圖片、太大或超過 {} 張上限而被略過）", attach::MAX_FILES);
        body_owned.as_str()
    } else {
        body
    };
    let item = store::insert_item(pool, NewItem { id: Some(&id), source, sender, level, subject, body, message_id, status }).await?;
    let Some(i) = item else {
        attach::remove_dir(attach_root, &id);
        return Ok(Ingested::Duplicate);
    };
    for a in &saved {
        store::add_attachment(pool, &i.id, a).await?;
    }
    Ok(Ingested::Queued(i.id))
}

// src-tauri/src/db/mail.rs
use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::fs;

pub struct MailDb {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct MailMessageRow {
    pub id: String,
    pub account_id: String,
    pub uid: i64,
    pub sender: String,
    pub subject: String,
    pub date: Option<String>,
    pub body_text: String,
    pub ai_summary: Option<String>,
    pub is_important: bool,
    pub is_promotional: bool,
    pub is_read_locally: bool,
    pub fetched_at: String,
}

/// Fields needed to insert a new message. Borrowed strings — the caller
/// (poller.rs) already owns the parsed data for the duration of the insert.
pub struct NewMessage<'a> {
    pub account_id: &'a str,
    pub uid: i64,
    pub sender: &'a str,
    pub subject: &'a str,
    pub date: Option<&'a str>,
    pub body_text: &'a str,
    pub ai_summary: Option<&'a str>,
    pub is_important: bool,
    pub is_promotional: bool,
}

impl MailDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("mail.db");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap_or_else(|_| {
            SqlitePool::connect_lazy("sqlite::memory:").unwrap()
        });
        let db = Self { pool };
        db.init().await.ok();
        db
    }

    pub async fn init(&self) -> Result<(), sqlx::Error> {
        init_schema(&self.pool).await
    }
}

/// Create the mail schema and bring an older one up to date. Idempotent, and a
/// free function rather than a method so the migration below can be exercised
/// against a hand-built legacy database in tests.
pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS mail_poll_state (
            account_id TEXT PRIMARY KEY NOT NULL,
            last_seen_uid INTEGER,
            uid_validity INTEGER,
            last_polled_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(pool).await?;

    // `uid_validity` was added after the first release. The CREATE above is
    // IF NOT EXISTS, so it is a no-op on an already-installed database and
    // that database would never gain the column — and every query naming it
    // would fail from then on. SQLite has no `ADD COLUMN IF NOT EXISTS`, so
    // check the current shape first rather than blanket-ignoring the error
    // (which would also swallow a genuinely broken database).
    if !column_exists(pool, "mail_poll_state", "uid_validity").await? {
        sqlx::query("ALTER TABLE mail_poll_state ADD COLUMN uid_validity INTEGER")
            .execute(pool).await?;
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS mail_messages (
            id TEXT PRIMARY KEY NOT NULL,
            account_id TEXT NOT NULL,
            uid INTEGER NOT NULL,
            sender TEXT NOT NULL,
            subject TEXT NOT NULL,
            date TEXT,
            body_text TEXT NOT NULL,
            ai_summary TEXT,
            is_important INTEGER NOT NULL DEFAULT 0,
            is_promotional INTEGER NOT NULL DEFAULT 0,
            is_read_locally INTEGER NOT NULL DEFAULT 0,
            fetched_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(account_id, uid)
        )"
    ).execute(pool).await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_mail_messages_account ON mail_messages(account_id)")
        .execute(pool).await?;

    Ok(())
}

async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    // `table` is a hard-coded literal at every call site, never user input.
    let columns: Vec<(i64, String)> = sqlx::query_as(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool).await?;
    Ok(columns.iter().any(|(_, name)| name == column))
}

pub async fn insert_message(pool: &SqlitePool, msg: NewMessage<'_>) -> Result<MailMessageRow, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mail_messages
            (id, account_id, uid, sender, subject, date, body_text, ai_summary, is_important, is_promotional)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(msg.account_id)
    .bind(msg.uid)
    .bind(msg.sender)
    .bind(msg.subject)
    .bind(msg.date)
    .bind(msg.body_text)
    .bind(msg.ai_summary)
    .bind(msg.is_important)
    .bind(msg.is_promotional)
    .execute(pool).await?;
    Ok(get_message(pool, &id).await?.expect("just-inserted row must exist"))
}

pub async fn get_message(pool: &SqlitePool, id: &str) -> Result<Option<MailMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MailMessageRow>(
        "SELECT id, account_id, uid, sender, subject, date, body_text, ai_summary,
                is_important, is_promotional, is_read_locally, fetched_at
         FROM mail_messages WHERE id = ?"
    ).bind(id).fetch_optional(pool).await
}

pub async fn list_messages(pool: &SqlitePool, account_id: &str) -> Result<Vec<MailMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MailMessageRow>(
        "SELECT id, account_id, uid, sender, subject, date, body_text, ai_summary,
                is_important, is_promotional, is_read_locally, fetched_at
         FROM mail_messages WHERE account_id = ? ORDER BY fetched_at DESC"
    ).bind(account_id).fetch_all(pool).await
}

pub async fn mark_read_locally(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mail_messages SET is_read_locally = 1 WHERE id = ?")
        .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn count_unread(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mail_messages WHERE is_read_locally = 0")
        .fetch_one(pool).await?;
    Ok(row.0)
}

pub async fn get_last_seen_uid(pool: &SqlitePool, account_id: &str) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT last_seen_uid FROM mail_poll_state WHERE account_id = ?"
    ).bind(account_id).fetch_optional(pool).await?;
    Ok(row.and_then(|(uid,)| uid))
}

pub async fn set_last_seen_uid(pool: &SqlitePool, account_id: &str, uid: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mail_poll_state (account_id, last_seen_uid, last_polled_at)
         VALUES (?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(account_id) DO UPDATE SET
            last_seen_uid = excluded.last_seen_uid,
            last_polled_at = CURRENT_TIMESTAMP"
    ).bind(account_id).bind(uid).execute(pool).await?;
    Ok(())
}

pub async fn get_uid_validity(pool: &SqlitePool, account_id: &str) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT uid_validity FROM mail_poll_state WHERE account_id = ?"
    ).bind(account_id).fetch_optional(pool).await?;
    Ok(row.and_then(|(v,)| v))
}

/// Record the mailbox's current UIDVALIDITY without disturbing `last_seen_uid`.
/// Used on every poll where the value did not change (including the first one,
/// and the first poll after upgrading a database that predates the column).
pub async fn set_uid_validity(pool: &SqlitePool, account_id: &str, uid_validity: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mail_poll_state (account_id, uid_validity, last_polled_at)
         VALUES (?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(account_id) DO UPDATE SET
            uid_validity = excluded.uid_validity,
            last_polled_at = CURRENT_TIMESTAMP"
    ).bind(account_id).bind(uid_validity).execute(pool).await?;
    Ok(())
}

/// The mailbox was recreated (UIDVALIDITY changed), so every UID we cached
/// refers to a message that no longer exists under that number. Drop the cached
/// messages *and* `last_seen_uid` together with the new UIDVALIDITY, in one
/// transaction: recording the new validity while leaving the old
/// `last_seen_uid` behind would look "already synced" on the next poll and
/// wedge the account at `UID {stale+1}:*` — nothing new, forever.
///
/// Returns how many cached messages were dropped, so the caller can tell the UI.
pub async fn reset_for_uid_validity(
    pool: &SqlitePool,
    account_id: &str,
    uid_validity: i64,
) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let dropped = sqlx::query("DELETE FROM mail_messages WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?.rows_affected();
    sqlx::query(
        "INSERT INTO mail_poll_state (account_id, last_seen_uid, uid_validity, last_polled_at)
         VALUES (?, NULL, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(account_id) DO UPDATE SET
            last_seen_uid = NULL,
            uid_validity = excluded.uid_validity,
            last_polled_at = CURRENT_TIMESTAMP"
    ).bind(account_id).bind(uid_validity).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(dropped)
}

pub async fn list_uids(pool: &SqlitePool, account_id: &str) -> Result<Vec<i64>, sqlx::Error> {
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT uid FROM mail_messages WHERE account_id = ?")
        .bind(account_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|(uid,)| uid).collect())
}

/// Which cached UIDs the server no longer has. Pure, and deliberately the only
/// place that decides what reconciliation destroys.
///
/// `server_uids` MUST be a complete `UID SEARCH ALL` result for the mailbox.
/// Feeding it a windowed or paged UID list would report everything outside that
/// window as deleted — which is why the caller never derives it from a
/// `UID {n}:*` search or from a fetch batch.
///
/// An empty `server_uids` returns nothing to delete, always. A mailbox the user
/// genuinely emptied therefore keeps its stale rows until one message arrives,
/// which is the deliberate trade: the alternative is that any server or parser
/// quirk that yields an empty SEARCH result silently erases the whole cache.
pub fn uids_absent_from_server(local_uids: &[i64], server_uids: &[i64]) -> Vec<i64> {
    if server_uids.is_empty() {
        return Vec::new();
    }
    let present: HashSet<i64> = server_uids.iter().copied().collect();
    local_uids.iter().copied().filter(|uid| !present.contains(uid)).collect()
}

/// Delete the account's cached messages that are gone from the server, and
/// return how many rows went. Never touches `mail_poll_state`: a message
/// disappearing from the server must not make us re-fetch anything.
///
/// Note the shape of the delete — it enumerates the doomed UIDs explicitly
/// rather than issuing `... WHERE uid NOT IN (<server set>)`. That is the
/// point: there is no statement here that could delete an account's mail by
/// being handed an empty or truncated server set, because an empty doomed list
/// runs no SQL at all.
pub async fn delete_messages_absent_from_server(
    pool: &SqlitePool,
    account_id: &str,
    server_uids: &[i64],
) -> Result<u64, sqlx::Error> {
    let local_uids = list_uids(pool, account_id).await?;
    let doomed = uids_absent_from_server(&local_uids, server_uids);
    if doomed.is_empty() {
        return Ok(0);
    }

    // Chunked so a bulk archive can't exceed SQLite's bound-parameter limit.
    let mut deleted = 0u64;
    for chunk in doomed.chunks(200) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!("DELETE FROM mail_messages WHERE account_id = ? AND uid IN ({placeholders})");
        let mut query = sqlx::query(&sql).bind(account_id);
        for uid in chunk {
            query = query.bind(*uid);
        }
        deleted += query.execute(pool).await?.rows_affected();
    }
    Ok(deleted)
}

pub async fn delete_account_data(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM mail_messages WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM mail_poll_state WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

// src-tauri/src/db/mail.rs
use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
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
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS mail_poll_state (
                account_id TEXT PRIMARY KEY NOT NULL,
                last_seen_uid INTEGER,
                last_polled_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

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
        ).execute(&self.pool).await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mail_messages_account ON mail_messages(account_id)")
            .execute(&self.pool).await?;

        Ok(())
    }
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

pub async fn delete_account_data(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM mail_messages WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM mail_poll_state WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

// src-tauri/src/db/design.rs
use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use std::path::PathBuf;
use std::fs;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DesignSession {
    pub id: String,
    pub title: String,
    pub current_proposal_draft: Option<String>,
    pub current_spec_draft: Option<String>,
    pub current_sdd_draft: Option<String>,
    pub current_plan_draft: Option<String>,
    pub context_summary: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DesignMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: Option<String>,
}

pub struct DesignDb {
    pub pool: SqlitePool,
}

impl DesignDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("design.db");
        // SqlitePool::connect does not create a missing file by default (sqlx
        // defaults create_if_missing to false) — it fails to open, and the
        // fallback below then silently swaps in an in-memory DB with no error
        // surfaced anywhere. Explicitly opt in to file creation so data persists.
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);

        let pool = SqlitePool::connect_with(options).await.unwrap_or_else(|_| {
            // Fallback to in-memory if file fails
            SqlitePool::connect_lazy("sqlite::memory:").unwrap()
        });

        let db = Self { pool };
        db.init().await.ok();
        db
    }

    async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS design_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL,
                current_spec_draft TEXT,
                current_sdd_draft TEXT,
                current_plan_draft TEXT,
                context_summary TEXT,
                status TEXT NOT NULL DEFAULT 'draft',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        // Migration: add proposal column for existing databases
        let _ = sqlx::query("ALTER TABLE design_sessions ADD COLUMN current_proposal_draft TEXT")
            .execute(&self.pool).await;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS design_messages (
                id TEXT PRIMARY KEY NOT NULL,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (session_id) REFERENCES design_sessions (id) ON DELETE CASCADE
            )"
        ).execute(&self.pool).await?;
        
        Ok(())
    }
}

pub async fn create_design_session(pool: &SqlitePool, title: &str) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO design_sessions (id, title, status) VALUES (?, ?, 'draft')")
        .bind(&id)
        .bind(title)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn get_design_session(pool: &SqlitePool, id: &str) -> Result<DesignSession, sqlx::Error> {
    let row = sqlx::query_as::<_, DesignSession>(
        "SELECT id, title, current_proposal_draft, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions WHERE id = ?"
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update_design_draft(
    pool: &SqlitePool,
    id: &str,
    field: &str,
    content: &str,
) -> Result<(), sqlx::Error> {
    let sql = match field {
        "proposal" => "UPDATE design_sessions SET current_proposal_draft = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        "spec" => "UPDATE design_sessions SET current_spec_draft = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        "sdd" => "UPDATE design_sessions SET current_sdd_draft = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        "plan" => "UPDATE design_sessions SET current_plan_draft = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        _ => return Err(sqlx::Error::ColumnNotFound(field.to_string())),
    };

    sqlx::query(sql)
        .bind(content)
        .bind(id)
        .execute(pool)
        .await?;
    
    Ok(())
}

pub async fn delete_design_session(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM design_messages WHERE session_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM design_sessions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn create_design_message(
    pool: &SqlitePool,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO design_messages (id, session_id, role, content) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(session_id)
        .bind(role)
        .bind(content)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_design_messages(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<DesignMessage>, sqlx::Error> {
    sqlx::query_as::<_, DesignMessage>(
        "SELECT id, session_id, role, content, created_at FROM design_messages WHERE session_id = ? ORDER BY created_at ASC, rowid ASC"
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
}

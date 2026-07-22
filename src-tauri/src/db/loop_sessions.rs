use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::fs;

pub struct LoopSessionDb {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct LoopSessionRow {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub iteration: i64,
    pub config_json: String,
    pub history_json: String,
    pub shared_context: String,
    pub trace_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct LoopSessionSummary {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub iteration: i64,
    pub created_at: String,
    pub updated_at: String,
}

impl LoopSessionDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("loop_sessions.db");
        // SqlitePool::connect does not create a missing file by default (sqlx
        // defaults create_if_missing to false) — it fails to open, and the
        // fallback below then silently swaps in an in-memory DB with no error
        // surfaced anywhere. Explicitly opt in to file creation so data persists.
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

    async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS loop_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                goal TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'running',
                iteration INTEGER NOT NULL DEFAULT 0,
                config_json TEXT NOT NULL DEFAULT '{}',
                history_json TEXT NOT NULL DEFAULT '[]',
                shared_context TEXT NOT NULL DEFAULT '',
                trace_json TEXT NOT NULL DEFAULT '[]',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;
        Ok(())
    }
}

pub async fn upsert_loop_session(
    pool: &SqlitePool,
    id: &str,
    goal: &str,
    status: &str,
    iteration: i64,
    config_json: &str,
    history_json: &str,
    shared_context: &str,
    trace_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO loop_sessions (id, goal, status, iteration, config_json, history_json, shared_context, trace_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
           goal = excluded.goal,
           status = excluded.status,
           iteration = excluded.iteration,
           config_json = excluded.config_json,
           history_json = excluded.history_json,
           shared_context = excluded.shared_context,
           trace_json = excluded.trace_json,
           updated_at = CURRENT_TIMESTAMP"
    )
    .bind(id).bind(goal).bind(status).bind(iteration)
    .bind(config_json).bind(history_json).bind(shared_context).bind(trace_json)
    .execute(pool).await?;
    Ok(())
}

pub async fn list_loop_sessions(pool: &SqlitePool) -> Result<Vec<LoopSessionSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, LoopSessionSummary>(
        "SELECT id, goal, status, iteration, created_at, updated_at
         FROM loop_sessions ORDER BY updated_at DESC"
    ).fetch_all(pool).await?;
    Ok(rows)
}

pub async fn load_loop_session(pool: &SqlitePool, id: &str) -> Result<LoopSessionRow, sqlx::Error> {
    sqlx::query_as::<_, LoopSessionRow>(
        "SELECT id, goal, status, iteration, config_json, history_json, shared_context, trace_json, created_at, updated_at
         FROM loop_sessions WHERE id = ?"
    ).bind(id).fetch_one(pool).await
}

pub async fn delete_loop_session(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM loop_sessions WHERE id = ?")
        .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn clear_all_loop_sessions(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM loop_sessions").execute(pool).await?;
    Ok(())
}

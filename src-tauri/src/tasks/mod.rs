//! Task board: a queued work list that dispatches cards to `claude`.
//! See `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`.
//!
//! - `store`     — `tasks.db` schema + CRUD (sqlx free functions over a pool)
//! - `dispatch`  — compose the prompt, spawn a visible PTY tab, type it in
//! - `monitor`   — watch one running task's session to a terminal outcome
//! - `scheduler` — pick the next runnable card; the long-lived dispatch loop

pub mod store;
pub mod dispatch;
pub mod monitor;
pub mod scheduler;

use std::fs;
use std::path::PathBuf;

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

/// Holds the connection pool for `tasks.db`. Managed by Tauri; free
/// functions in `store` take `&db.pool`. Same shape as `db::loop_sessions::LoopSessionDb`.
pub struct TasksDb {
    pub pool: SqlitePool,
}

/// `<data-dir>/AITERM` — the same app data directory every other
/// dedicated-SQLite module in this codebase uses.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM")
}

/// `<data-dir>/AITERM/tasks/<task_id>` — per-task scratch dir holding
/// `attachments/` and `transcript.txt`. Created lazily by dispatch/store.
pub fn task_dir(task_id: &str) -> PathBuf {
    app_data_dir().join("tasks").join(task_id)
}

impl TasksDb {
    pub async fn new() -> Self {
        let dir = app_data_dir();
        fs::create_dir_all(&dir).ok();
        let db_path = dir.join("tasks.db");
        // sqlx defaults create_if_missing to false; without this the open
        // fails and the fallback silently swaps in an in-memory DB that
        // loses every card on restart. See db/loop_sessions.rs.
        let options = SqliteConnectOptions::new().filename(&db_path).create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .unwrap_or_else(|_| SqlitePool::connect_lazy("sqlite::memory:").unwrap());
        let db = Self { pool };
        init_schema(&db.pool).await.ok();
        db
    }
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tasks (
            id              TEXT PRIMARY KEY NOT NULL,
            title           TEXT NOT NULL,
            body            TEXT NOT NULL DEFAULT '',
            project_dir     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'planning',
            parallel_ok     INTEGER NOT NULL DEFAULT 1,
            interactive     INTEGER NOT NULL DEFAULT 0,
            sort_order      REAL NOT NULL DEFAULT 0,
            outcome         TEXT,
            tab_id          TEXT,
            transcript_path TEXT,
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            dispatched_at   INTEGER,
            finished_at     INTEGER
        )",
    )
    .execute(pool)
    .await?;
    // Migration: existing databases created before `interactive` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status, sort_order)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_attachments (
            id          TEXT PRIMARY KEY NOT NULL,
            task_id     TEXT NOT NULL,
            filename    TEXT NOT NULL,
            stored_path TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_task_attachments_task ON task_attachments(task_id)")
        .execute(pool)
        .await?;
    Ok(())
}

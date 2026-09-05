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

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

/// `<data-dir>/AITERM` — the same app data directory every other
/// dedicated-SQLite module in this codebase uses.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM")
}

/// `<project-folder>/tasks/<task_id>` — 這張卡片的附件與對話記錄。
/// 由 dispatch/store 在需要時建立。
///
/// 根目錄是**專案資料夾**而非全域資料區——專案資料夾必須自成一體，
/// 這樣複製走就等於匯出。
pub fn task_dir(project_path: &Path, task_id: &str) -> PathBuf {
    project_path.join("tasks").join(task_id)
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
            finished_at     INTEGER,
            ai_summary      TEXT
        )",
    )
    .execute(pool)
    .await?;
    // Migration: existing databases created before `interactive` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    // Migration: existing databases created before `ai_summary` existed.
    // 跟上面的 `interactive` 同一個寫法——欄位已存在時 ALTER TABLE 會
    // 失敗，那是正常的，所以刻意丟掉錯誤。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN ai_summary TEXT")
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

#[cfg(test)]
mod task_dir_tests {
    use super::*;

    #[test]
    fn is_rooted_at_the_project_folder() {
        let project = std::path::Path::new("/projects/makemoney");
        assert_eq!(task_dir(project, "abc"), project.join("tasks").join("abc"));
    }
}

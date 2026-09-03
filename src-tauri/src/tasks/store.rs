//! `tasks.db` CRUD. Free functions over `&SqlitePool`, same style as
//! `db/loop_sessions.rs`. Status is a plain string column with four values:
//! `planning` → `queued` → `running` → `done`. `outcome` is NULL until
//! `done`, then one of `success` | `failed` | `cancelled`.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

pub const STATUS_PLANNING: &str = "planning";
pub const STATUS_QUEUED: &str = "queued";
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_DONE: &str = "done";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub status: String,
    // INTEGER column mapped straight to `bool` — the sqlite driver's native
    // Decode<bool> reads 0/1. Same pattern as `db/mail.rs` (`is_important` etc.).
    pub parallel_ok: bool,
    pub sort_order: f64,
    pub outcome: Option<String>,
    pub tab_id: Option<String>,
    pub transcript_path: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub dispatched_at: Option<i64>,
    pub finished_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub task_id: String,
    pub filename: String,
    pub stored_path: String,
}

#[allow(dead_code)] // consumed by dispatch/monitor in later tasks
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Insert a `planning` card at the bottom of the planning column
/// (`sort_order` = current max + 1). Returns the new id.
pub async fn create_task(
    pool: &SqlitePool,
    title: &str,
    body: &str,
    project_dir: &str,
    parallel_ok: bool,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    // CAST to REAL: with an empty table the expression is otherwise typed
    // INTEGER and query_scalar::<f64> fails to decode it.
    let next_order: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(MAX(sort_order), 0) + 1 AS REAL) FROM tasks WHERE status = ?",
    )
    .bind(STATUS_PLANNING)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, sort_order)
         VALUES (?, ?, ?, ?, 'planning', ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(body)
    .bind(project_dir)
    .bind(parallel_ok as i64)
    .bind(next_order)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_tasks(pool: &SqlitePool) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks ORDER BY status, sort_order")
        .fetch_all(pool)
        .await
}

pub async fn list_attachments(
    pool: &SqlitePool,
    task_id: &str,
) -> Result<Vec<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>(
        "SELECT * FROM task_attachments WHERE task_id = ? ORDER BY filename",
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_then_list_roundtrips_a_planning_card() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "Fix the flaky test", "make it deterministic", "/repo", true)
            .await
            .unwrap();
        let all = list_tasks(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].title, "Fix the flaky test");
        assert_eq!(all[0].status, "planning");
        assert_eq!(all[0].parallel_ok, true);
        assert!(all[0].outcome.is_none());
    }
}

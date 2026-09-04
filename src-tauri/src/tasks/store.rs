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
    // Set at creation time, editable only while `planning` (same rule as
    // `parallel_ok`) — see docs/superpowers/specs/2026-09-03-task-board-interactive-mode-design.md.
    pub interactive: bool,
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
    interactive: bool,
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
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, interactive, sort_order)
         VALUES (?, ?, ?, ?, 'planning', ?, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(body)
    .bind(project_dir)
    .bind(parallel_ok as i64)
    .bind(interactive as i64)
    .bind(next_order)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Create a fresh `planning` card copying `src`'s title/body/project_dir/
/// parallel_ok/interactive. Does NOT copy attachments (the command layer
/// does the file copy). Returns the new id. Err if `src_id` doesn't exist.
pub async fn clone_task_fields(pool: &SqlitePool, src_id: &str) -> Result<String, sqlx::Error> {
    let src = get_task(pool, src_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    create_task(pool, &src.title, &src.body, &src.project_dir, src.parallel_ok, src.interactive).await
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

/// Whether `from → to` is a legal user-driven column move. The scheduler and
/// monitor use `mark_dispatched`/`finish_task` for `queued→running` and
/// `→done`; the only moves a user makes by hand are among
/// planning/queued (either direction).
fn transition_ok(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        (STATUS_PLANNING, STATUS_QUEUED)
            | (STATUS_QUEUED, STATUS_PLANNING)
            | (STATUS_PLANNING, STATUS_PLANNING)
            | (STATUS_QUEUED, STATUS_QUEUED)
    )
}

/// Move a card to `to_status` at `sort_order`. Rejects illegal transitions.
/// `queued→running` / `→done` are NOT done through here — use
/// `mark_dispatched` / `finish_task`.
pub async fn move_task(
    pool: &SqlitePool,
    id: &str,
    to_status: &str,
    sort_order: f64,
) -> Result<(), sqlx::Error> {
    let current: Option<String> = sqlx::query_scalar("SELECT status FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let from = current.ok_or(sqlx::Error::RowNotFound)?;
    if !transition_ok(&from, to_status) {
        return Err(sqlx::Error::Protocol(format!(
            "illegal transition {from} → {to_status}"
        )));
    }
    sqlx::query("UPDATE tasks SET status = ?, sort_order = ? WHERE id = ?")
        .bind(to_status)
        .bind(sort_order)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The `sort_order` value that drops a card between `before` and `after`
/// (either may be `None` = top/bottom of the column).
pub async fn midpoint_between(
    pool: &SqlitePool,
    status: &str,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<f64, sqlx::Error> {
    async fn order_of(pool: &SqlitePool, id: &str) -> Result<f64, sqlx::Error> {
        sqlx::query_scalar("SELECT CAST(sort_order AS REAL) FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
    }
    let lo = match before_id {
        Some(id) => order_of(pool, id).await?,
        None => {
            let min: Option<f64> = sqlx::query_scalar(
                "SELECT CAST(MIN(sort_order) AS REAL) FROM tasks WHERE status = ?",
            )
            .bind(status)
            .fetch_one(pool)
            .await?;
            min.unwrap_or(1.0) - 1.0
        }
    };
    let hi = match after_id {
        Some(id) => order_of(pool, id).await?,
        None => {
            let max: Option<f64> = sqlx::query_scalar(
                "SELECT CAST(MAX(sort_order) AS REAL) FROM tasks WHERE status = ?",
            )
            .bind(status)
            .fetch_one(pool)
            .await?;
            max.unwrap_or(0.0) + 2.0
        }
    };
    Ok((lo + hi) / 2.0)
}

pub async fn set_parallel_ok(
    pool: &SqlitePool,
    id: &str,
    parallel_ok: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET parallel_ok = ? WHERE id = ?")
        .bind(parallel_ok as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_interactive(
    pool: &SqlitePool,
    id: &str,
    interactive: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET interactive = ? WHERE id = ?")
        .bind(interactive as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Edit title/body/project_dir. Caller (command layer) restricts this to `planning` cards.
pub async fn update_task_fields(
    pool: &SqlitePool,
    id: &str,
    title: &str,
    body: &str,
    project_dir: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET title = ?, body = ?, project_dir = ? WHERE id = ?")
        .bind(title)
        .bind(body)
        .bind(project_dir)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `queued → running`: record the spawned tab and dispatch time.
pub async fn mark_dispatched(pool: &SqlitePool, id: &str, tab_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tasks SET status = 'running', tab_id = ?, dispatched_at = ? WHERE id = ? AND status = 'queued'",
    )
    .bind(tab_id)
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// `running → done` with an outcome. `outcome` ∈ success | failed | cancelled.
pub async fn finish_task(
    pool: &SqlitePool,
    id: &str,
    outcome: &str,
    error_message: Option<&str>,
    transcript_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tasks SET status = 'done', outcome = ?, error_message = ?, transcript_path = ?, finished_at = ? WHERE id = ?",
    )
    .bind(outcome)
    .bind(error_message)
    .bind(transcript_path)
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Startup recovery: any card still `running` was orphaned when the app last
/// exited (its PTY died with the process). Returns how many were recovered.
pub async fn recover_orphaned_running(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE tasks SET status = 'done', outcome = 'cancelled',
             error_message = 'app 重啟，工作已中斷', finished_at = ?
         WHERE status = 'running'",
    )
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn add_attachment(
    pool: &SqlitePool,
    task_id: &str,
    filename: &str,
    stored_path: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO task_attachments (id, task_id, filename, stored_path) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(task_id)
    .bind(filename)
    .bind(stored_path)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn remove_attachment(pool: &SqlitePool, attachment_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_attachments WHERE id = ?")
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_attachment(
    pool: &SqlitePool,
    attachment_id: &str,
) -> Result<Option<AttachmentRow>, sqlx::Error> {
    sqlx::query_as::<_, AttachmentRow>("SELECT * FROM task_attachments WHERE id = ?")
        .bind(attachment_id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_task(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM task_attachments WHERE task_id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Rows the scheduler needs: everything of one status, oldest first.
pub async fn list_by_status(pool: &SqlitePool, status: &str) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>("SELECT * FROM tasks WHERE status = ? ORDER BY sort_order")
        .bind(status)
        .fetch_all(pool)
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
        let id =
            create_task(&pool, "Fix the flaky test", "make it deterministic", "/repo", true, false)
                .await
                .unwrap();
        let all = list_tasks(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].title, "Fix the flaky test");
        assert_eq!(all[0].status, "planning");
        assert!(all[0].parallel_ok);
        assert!(all[0].outcome.is_none());
    }

    #[tokio::test]
    async fn move_planning_to_queued_is_allowed_but_done_to_running_is_not() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().status, "queued");

        finish_task(&pool, &id, "success", None, None).await.unwrap();
        let err = move_task(&pool, &id, STATUS_RUNNING, 1.0).await.unwrap_err();
        assert!(err.to_string().contains("illegal transition"), "{err}");
    }

    #[tokio::test]
    async fn midpoint_sort_order_between_two_cards() {
        let pool = mem_pool().await;
        let a = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        let b = create_task(&pool, "b", "", "/r", true, false).await.unwrap();
        move_task(&pool, &a, STATUS_QUEUED, 1.0).await.unwrap();
        move_task(&pool, &b, STATUS_QUEUED, 2.0).await.unwrap();
        let c = create_task(&pool, "c", "", "/r", true, false).await.unwrap();
        let mid = midpoint_between(&pool, STATUS_QUEUED, Some(&a), Some(&b)).await.unwrap();
        assert!((mid - 1.5).abs() < 1e-9, "got {mid}");
        move_task(&pool, &c, STATUS_QUEUED, mid).await.unwrap();
    }

    #[tokio::test]
    async fn mark_dispatched_and_finish_set_the_right_columns() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        mark_dispatched(&pool, &id, "tab-xyz").await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "running");
        assert_eq!(row.tab_id.as_deref(), Some("tab-xyz"));
        assert!(row.dispatched_at.is_some());

        finish_task(&pool, &id, "failed", Some("boom"), Some("/p/transcript.txt")).await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("failed"));
        assert_eq!(row.error_message.as_deref(), Some("boom"));
        assert!(row.finished_at.is_some());
    }

    #[tokio::test]
    async fn recover_orphaned_running_marks_them_cancelled() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        mark_dispatched(&pool, &id, "tab-1").await.unwrap();
        let n = recover_orphaned_running(&pool).await.unwrap();
        assert_eq!(n, 1);
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("cancelled"));
    }

    #[tokio::test]
    async fn add_and_remove_attachment_rows() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let aid = add_attachment(&pool, &id, "spec.md", "/p/spec.md").await.unwrap();
        assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 1);
        remove_attachment(&pool, &aid).await.unwrap();
        assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn clone_task_fields_copies_the_core_fields_into_a_new_planning_card() {
        let pool = mem_pool().await;
        let src = create_task(&pool, "Ship it", "the body", "/repo/x", false, false)
            .await
            .unwrap();
        move_task(&pool, &src, STATUS_QUEUED, 1.0).await.unwrap();
        finish_task(&pool, &src, "success", None, None).await.unwrap();

        let new_id = clone_task_fields(&pool, &src).await.unwrap();
        assert_ne!(new_id, src);
        let row = get_task(&pool, &new_id).await.unwrap().unwrap();
        assert_eq!(row.status, STATUS_PLANNING);
        assert_eq!(row.title, "Ship it");
        assert_eq!(row.body, "the body");
        assert_eq!(row.project_dir, "/repo/x");
        assert!(!row.parallel_ok);
        assert!(row.outcome.is_none());
    }

    #[tokio::test]
    async fn clone_task_fields_errors_when_source_is_missing() {
        let pool = mem_pool().await;
        assert!(clone_task_fields(&pool, "nope").await.is_err());
    }

    #[tokio::test]
    async fn delete_task_also_deletes_its_attachment_rows() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        add_attachment(&pool, &id, "a", "/p/a").await.unwrap();
        delete_task(&pool, &id).await.unwrap();
        assert!(get_task(&pool, &id).await.unwrap().is_none());
        assert_eq!(list_attachments(&pool, &id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn interactive_flag_defaults_false_and_is_persisted_when_true() {
        let pool = mem_pool().await;
        let auto_id = create_task(&pool, "auto one", "", "/r", true, false).await.unwrap();
        let chat_id = create_task(&pool, "chat one", "", "/r", true, true).await.unwrap();
        assert!(!get_task(&pool, &auto_id).await.unwrap().unwrap().interactive);
        assert!(get_task(&pool, &chat_id).await.unwrap().unwrap().interactive);
    }

    #[tokio::test]
    async fn clone_task_fields_copies_the_interactive_flag() {
        let pool = mem_pool().await;
        let src = create_task(&pool, "chat one", "", "/r", true, true).await.unwrap();
        let new_id = clone_task_fields(&pool, &src).await.unwrap();
        assert!(get_task(&pool, &new_id).await.unwrap().unwrap().interactive);
    }
}

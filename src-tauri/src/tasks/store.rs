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
    /// 這張卡片的 AI 履行摘要（工作報告用）。只有 `done` 的卡片會有，
    /// 由前端在產生報告時補上。已完成的卡片不可變，所以這是永久快取；
    /// 重新派工時 `claim_for_dispatch` 會清掉它。
    pub ai_summary: Option<String>,
    /// 封存時間（Unix 秒）。有值代表這張卡已經從看板上收起來——資料完整
    /// 保留，只是不再出現在四欄裡，也不會被排程器或工作報告撿到。
    pub archived_at: Option<i64>,
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

/// 所有卡片，依欄位分組後排序。
///
/// 已完成的卡片依 `finished_at` 由新到舊——`finish_task` 不碰 `sort_order`，
/// 所以完成的卡片留著的是它還在佇列時的值，實機上那些值常常一模一樣，
/// 「已完成」欄的順序等於未定義。
///
/// 沒完成的卡片 `finished_at` 是 NULL，`COALESCE` 一律給 0，所以它們在
/// 第二個排序鍵上全部同分、順序仍然由使用者拖出來的 `sort_order` 決定。
pub async fn list_tasks(pool: &SqlitePool) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        "SELECT * FROM tasks WHERE archived_at IS NULL
         ORDER BY status, COALESCE(finished_at, 0) DESC, sort_order",
    )
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
/// monitor use `claim_for_dispatch`/`finish_task` for `queued→running` and
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
/// `claim_for_dispatch` / `finish_task`.
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

/// 寫入這張卡片的 AI 履行摘要（工作報告的第一階段產物）。
/// `done → 已封存`。只有已完成的卡片能封存：把計畫中或執行中的卡片
/// 藏起來等於讓工作憑空消失，而執行中的那張背後還有一個真的在跑的
/// Agent。`WHERE status = 'done'` 就是那道守門，沒有更新到任何列時
/// 回報錯誤而不是默默成功。
pub async fn archive_task(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    let res = sqlx::query(
        "UPDATE tasks SET archived_at = ? WHERE id = ? AND status = 'done' AND archived_at IS NULL",
    )
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(sqlx::Error::Protocol(format!(
            "只有已完成且尚未封存的卡片可以封存：{id}"
        )));
    }
    Ok(())
}

/// 把封存的卡片放回看板。它回到 `done` 欄——狀態從來沒有變過，封存只是
/// 一層可見性。
pub async fn unarchive_task(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET archived_at = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 一次收走整個「已完成」欄，回傳實際封存了幾張。
///
/// `archived_at IS NULL` 讓已經封存的不被重複計數，封存時間也不會被刷成
/// 新的——那會打亂封存清單的排序。
pub async fn archive_all_done(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE tasks SET archived_at = ? WHERE status = 'done' AND archived_at IS NULL",
    )
    .bind(now_secs())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 封存的卡片，新封存的在前。
pub async fn list_archived(pool: &SqlitePool) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        "SELECT * FROM tasks WHERE archived_at IS NOT NULL ORDER BY archived_at DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn set_summary(pool: &SqlitePool, id: &str, summary: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET ai_summary = ? WHERE id = ?")
        .bind(summary)
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
/// 原子地把一張 `queued` 卡片認領下來準備派工：標成 `running`、記下
/// 派工時間、清掉上一次執行留下的摘要。回傳是否真的認領到。
///
/// **必須在真正去 spawn 之前呼叫。** `dispatch::spawn_and_run` 要等
/// `claude` 的 TUI 起來，最久 30 秒；如果等到那之後才標記，卡片在整段
/// 期間都還是 `queued`，任何第二次派工掃描都會再派一次同一張卡——實機
/// 出現過同一張卡開出兩個 claude 行程。
///
/// `WHERE status = 'queued'` 讓這件事由資料庫保證：第二次呼叫影響 0 列，
/// 回傳 false。
pub async fn claim_for_dispatch(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE tasks SET status = 'running', dispatched_at = ?, ai_summary = NULL
         WHERE id = ? AND status = 'queued'",
    )
    .bind(now_secs())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// 記下這張卡片跑在哪個分頁。`claim_for_dispatch` 之後、PTY 開起來才知道
/// 分頁 id，所以分兩步。
pub async fn set_tab_id(pool: &SqlitePool, id: &str, tab_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET tab_id = ? WHERE id = ?")
        .bind(tab_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 測試專用：把 production 分成兩步的派工（先 `claim_for_dispatch`，
/// spawn 完再 `set_tab_id`）合成一步，讓不關心競態的測試好寫。
///
/// production **沒有**這樣的單一函式是刻意的：認領一定要發生在 spawn
/// 之前，兩者中間隔著最久 30 秒的等待。
#[cfg(test)]
pub(crate) async fn dispatch_for_test(pool: &SqlitePool, id: &str, tab_id: &str) {
    assert!(claim_for_dispatch(pool, id).await.unwrap(), "認領失敗：{id}");
    set_tab_id(pool, id, tab_id).await.unwrap();
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
/// 排程器用的查詢。跟看板的 `list_tasks` 一樣要排除封存的卡片——兩者是
/// 不同的查詢，各自都得擋。
pub async fn list_by_status(pool: &SqlitePool, status: &str) -> Result<Vec<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        "SELECT * FROM tasks WHERE status = ? AND archived_at IS NULL ORDER BY sort_order",
    )
        .bind(status)
        .fetch_all(pool)
        .await
}

/// 某個 status 目前有幾張卡片。供 `projects_list` 產生專案總覽的計數。
pub async fn count_by_status(pool: &SqlitePool, status: &str) -> Result<i64, sqlx::Error> {
    // 排除封存：專案總覽的計數要跟看板上看得到的張數一致。
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks WHERE status = ? AND archived_at IS NULL")
        .bind(status)
        .fetch_one(pool)
        .await
}

/// 全部卡片數，不分 status。搬遷時用來判斷舊資料庫是否值得搬。
pub async fn count_all(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM tasks").fetch_one(pool).await
}

/// 這個專案的卡片用過的工作目錄，去重後依字母排序。
/// 供新增工作時的目錄快捷選項——專案不綁資料夾，工作可散布在多個 repo，
/// 沒有這個的話使用者每次都得重新瀏覽選取。
pub async fn distinct_project_dirs(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT project_dir FROM tasks WHERE project_dir <> '' ORDER BY project_dir",
    )
    .fetch_all(pool)
    .await
}

/// 把 `transcript_path` 與附件的 `stored_path` 中的 `old_prefix` 換成
/// `new_prefix`。搬遷舊資料時用——那些欄位存的是絕對路徑，複製資料夾
/// 之後若不改寫，新的專案資料夾就不是自成一體的（複製到別台機器會
/// 掉附件）。只換開頭相符的，其他路徑不動。
pub async fn rewrite_stored_paths(
    pool: &SqlitePool,
    old_prefix: &str,
    new_prefix: &str,
) -> Result<(), sqlx::Error> {
    let like = format!("{old_prefix}%");
    sqlx::query(
        "UPDATE tasks SET transcript_path = ? || SUBSTR(transcript_path, ?)
         WHERE transcript_path LIKE ?",
    )
    .bind(new_prefix)
    .bind(old_prefix.len() as i64 + 1)
    .bind(&like)
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE task_attachments SET stored_path = ? || SUBSTR(stored_path, ?)
         WHERE stored_path LIKE ?",
    )
    .bind(new_prefix)
    .bind(old_prefix.len() as i64 + 1)
    .bind(&like)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn finished(pool: &SqlitePool, title: &str) -> String {
        let id = create_task(pool, title, "", "/r", true, false).await.unwrap();
        move_task(pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        dispatch_for_test(pool, &id, "tab").await;
        finish_task(pool, &id, "success", None, None).await.unwrap();
        id
    }

    #[tokio::test]
    async fn archiving_hides_a_card_from_the_board_but_keeps_the_row() {
        let pool = mem_pool().await;
        let id = finished(&pool, "收工").await;

        archive_task(&pool, &id).await.unwrap();

        assert!(list_tasks(&pool).await.unwrap().is_empty(), "封存的卡片不該還在看板上");
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.archived_at.is_some());
        assert_eq!(row.title, "收工", "資料必須原封不動地留著");
    }

    /// 只有已完成的卡片能封存。把計畫中或執行中的卡片藏起來，等於讓工作
    /// 憑空消失——而執行中的那張背後還有一個真的在跑的 Agent。
    #[tokio::test]
    async fn only_done_cards_can_be_archived() {
        let pool = mem_pool().await;
        let planning = create_task(&pool, "還在想", "", "/r", true, false).await.unwrap();
        assert!(archive_task(&pool, &planning).await.is_err());

        let running = create_task(&pool, "跑著", "", "/r", true, false).await.unwrap();
        move_task(&pool, &running, STATUS_QUEUED, 1.0).await.unwrap();
        dispatch_for_test(&pool, &running, "tab").await;
        assert!(archive_task(&pool, &running).await.is_err());

        assert_eq!(list_tasks(&pool).await.unwrap().len(), 2, "兩張都該還在看板上");
    }

    #[tokio::test]
    async fn unarchiving_puts_the_card_back_on_the_board() {
        let pool = mem_pool().await;
        let id = finished(&pool, "回來").await;
        archive_task(&pool, &id).await.unwrap();

        unarchive_task(&pool, &id).await.unwrap();

        assert_eq!(list_tasks(&pool).await.unwrap().len(), 1);
        assert!(get_task(&pool, &id).await.unwrap().unwrap().archived_at.is_none());
    }

    #[tokio::test]
    async fn archive_all_done_takes_the_whole_column_and_reports_how_many() {
        let pool = mem_pool().await;
        finished(&pool, "一").await;
        finished(&pool, "二").await;
        let planning = create_task(&pool, "留下", "", "/r", true, false).await.unwrap();

        let n = archive_all_done(&pool).await.unwrap();

        assert_eq!(n, 2);
        let left: Vec<String> =
            list_tasks(&pool).await.unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(left, vec!["留下"], "只有已完成的那一欄該被收走");
        assert!(get_task(&pool, &planning).await.unwrap().unwrap().archived_at.is_none());
    }

    /// 已經封存的不該被重複計數，也不該把封存時間刷成新的——那會打亂
    /// 封存清單的排序。
    #[tokio::test]
    async fn archive_all_done_skips_cards_that_are_already_archived() {
        let pool = mem_pool().await;
        let first = finished(&pool, "早就收了").await;
        archive_task(&pool, &first).await.unwrap();
        let stamp = get_task(&pool, &first).await.unwrap().unwrap().archived_at;
        finished(&pool, "剛完成").await;

        assert_eq!(archive_all_done(&pool).await.unwrap(), 1);
        assert_eq!(get_task(&pool, &first).await.unwrap().unwrap().archived_at, stamp);
    }

    #[tokio::test]
    async fn archived_cards_are_listed_newest_archived_first() {
        let pool = mem_pool().await;
        for (title, stamp) in [("早", 1_000_i64), ("晚", 3_000), ("中", 2_000)] {
            let id = finished(&pool, title).await;
            archive_task(&pool, &id).await.unwrap();
            sqlx::query("UPDATE tasks SET archived_at = ? WHERE id = ?")
                .bind(stamp)
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
        }

        let titles: Vec<String> =
            list_archived(&pool).await.unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, vec!["晚", "中", "早"]);
    }

    /// 排程器絕不能撿到封存的卡片。它查的是 `list_by_status`，跟看板用的
    /// 不是同一個查詢，所以要各自釘住。
    #[tokio::test]
    async fn list_by_status_ignores_archived_cards() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "封存的佇列卡", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        sqlx::query("UPDATE tasks SET archived_at = 1 WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(list_by_status(&pool, STATUS_QUEUED).await.unwrap().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已完成的卡片要新的排在前面。
    ///
    /// 原本的排序是 `ORDER BY status, sort_order`，而 `finish_task` 從來
    /// 不碰 `sort_order`——完成的卡片保留的是它還在佇列時的值。實機上那些
    /// 值常常一模一樣（各自曾是所屬狀態裡的第一張），於是「已完成」欄的
    /// 順序完全由 SQLite 的內部順序決定，既不是完成順序也不是反序。
    #[tokio::test]
    async fn done_cards_are_listed_newest_finished_first() {
        let pool = mem_pool().await;
        // 三張卡的 sort_order 刻意都設成 1.0，重現實機的狀況。
        let mut ids = Vec::new();
        for (name, finished) in [("早", 1_000_i64), ("晚", 3_000), ("中", 2_000)] {
            let id = create_task(&pool, name, "", "/r", true, false).await.unwrap();
            move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
            dispatch_for_test(&pool, &id, "tab").await;
            finish_task(&pool, &id, "success", None, None).await.unwrap();
            sqlx::query("UPDATE tasks SET finished_at = ?, sort_order = 1.0 WHERE id = ?")
                .bind(finished)
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
            ids.push(id);
        }

        let titles: Vec<String> = list_tasks(&pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.status == "done")
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, vec!["晚", "中", "早"], "已完成欄沒有依完成時間新到舊排列");
    }

    /// 還沒完成的卡片 `finished_at` 是 NULL，不可以因為新的排序規則就被
    /// 打亂——那幾欄的順序是使用者自己拖出來的。
    #[tokio::test]
    async fn unfinished_cards_still_follow_their_drag_order() {
        let pool = mem_pool().await;
        for (name, ord) in [("第三", 3.0_f64), ("第一", 1.0), ("第二", 2.0)] {
            let id = create_task(&pool, name, "", "/r", true, false).await.unwrap();
            move_task(&pool, &id, STATUS_QUEUED, ord).await.unwrap();
        }

        let titles: Vec<String> = list_tasks(&pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|t| t.status == STATUS_QUEUED)
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, vec!["第一", "第二", "第三"]);
    }
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
    async fn dispatching_and_finishing_set_the_right_columns() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        dispatch_for_test(&pool, &id, "tab-xyz").await;
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
        dispatch_for_test(&pool, &id, "tab-1").await;
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

#[cfg(test)]
mod project_query_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn count_by_status_counts_only_that_status() {
        let pool = mem_pool().await;
        let a = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        create_task(&pool, "b", "", "/r", true, false).await.unwrap();
        move_task(&pool, &a, STATUS_QUEUED, 1.0).await.unwrap();

        assert_eq!(count_by_status(&pool, STATUS_PLANNING).await.unwrap(), 1);
        assert_eq!(count_by_status(&pool, STATUS_QUEUED).await.unwrap(), 1);
        assert_eq!(count_by_status(&pool, STATUS_RUNNING).await.unwrap(), 0);
        assert_eq!(count_by_status(&pool, STATUS_DONE).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn distinct_project_dirs_dedupes_and_sorts() {
        let pool = mem_pool().await;
        create_task(&pool, "a", "", "/b/api", true, false).await.unwrap();
        create_task(&pool, "b", "", "/a/web", true, false).await.unwrap();
        create_task(&pool, "c", "", "/b/api", true, false).await.unwrap();

        let dirs = distinct_project_dirs(&pool).await.unwrap();
        assert_eq!(dirs, vec!["/a/web".to_string(), "/b/api".to_string()]);
    }

    #[tokio::test]
    async fn distinct_project_dirs_skips_empty_strings() {
        let pool = mem_pool().await;
        create_task(&pool, "a", "", "", true, false).await.unwrap();
        create_task(&pool, "b", "", "/real", true, false).await.unwrap();
        assert_eq!(distinct_project_dirs(&pool).await.unwrap(), vec!["/real".to_string()]);
    }

    #[tokio::test]
    async fn rewrite_stored_paths_repoints_attachments_and_transcripts() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        dispatch_for_test(&pool, &id, "tab").await;
        finish_task(&pool, &id, "success", None, Some("/old/home/tasks/x/transcript.txt"))
            .await
            .unwrap();
        add_attachment(&pool, &id, "f.png", "/old/home/tasks/x/attachments/f.png")
            .await
            .unwrap();

        rewrite_stored_paths(&pool, "/old/home", "/new/home").await.unwrap();

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.transcript_path.as_deref(), Some("/new/home/tasks/x/transcript.txt"));
        let atts = list_attachments(&pool, &id).await.unwrap();
        assert_eq!(atts[0].stored_path, "/new/home/tasks/x/attachments/f.png");
    }

    #[tokio::test]
    async fn rewrite_stored_paths_leaves_unrelated_paths_alone() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "a", "", "/r", true, false).await.unwrap();
        add_attachment(&pool, &id, "f.png", "/somewhere/else/f.png").await.unwrap();

        rewrite_stored_paths(&pool, "/old/home", "/new/home").await.unwrap();

        let atts = list_attachments(&pool, &id).await.unwrap();
        assert_eq!(atts[0].stored_path, "/somewhere/else/f.png");
    }
}

#[cfg(test)]
mod claim_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn queued_card(pool: &SqlitePool) -> String {
        let id = create_task(pool, "t", "", "/r", true, false).await.unwrap();
        move_task(pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        id
    }

    #[tokio::test]
    async fn claiming_a_queued_card_marks_it_running() {
        let pool = mem_pool().await;
        let id = queued_card(&pool).await;
        assert!(claim_for_dispatch(&pool, &id).await.unwrap());

        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, STATUS_RUNNING);
        assert!(row.dispatched_at.is_some());
    }

    /// 這是整個修正的重點：派工要等 claude 的 TUI 起來，最久 30 秒。
    /// 在那之前卡片若還留在 queued，第二次掃描就會再派一次同一張卡——
    /// 實機出現過同一張卡開出兩個 claude 行程。先原子地認領就不可能。
    #[tokio::test]
    async fn a_card_can_only_be_claimed_once() {
        let pool = mem_pool().await;
        let id = queued_card(&pool).await;
        assert!(claim_for_dispatch(&pool, &id).await.unwrap());
        assert!(
            !claim_for_dispatch(&pool, &id).await.unwrap(),
            "已經被認領的卡片不可以再被認領一次"
        );
    }

    #[tokio::test]
    async fn a_card_that_is_not_queued_cannot_be_claimed() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        // 還在 planning
        assert!(!claim_for_dispatch(&pool, &id).await.unwrap());
    }

    /// 認領時就清掉上一次執行的摘要：舊摘要描述的是上一次的執行。
    #[tokio::test]
    async fn claiming_clears_a_stale_summary() {
        let pool = mem_pool().await;
        let id = queued_card(&pool).await;
        set_summary(&pool, &id, "上一次執行的摘要").await.unwrap();
        assert!(claim_for_dispatch(&pool, &id).await.unwrap());
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().ai_summary, None);
    }

    #[tokio::test]
    async fn set_tab_id_records_which_tab_the_card_is_running_in() {
        let pool = mem_pool().await;
        let id = queued_card(&pool).await;
        claim_for_dispatch(&pool, &id).await.unwrap();
        set_tab_id(&pool, &id, "tab-9").await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().tab_id.as_deref(), Some("tab-9"));
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn a_new_task_has_no_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().ai_summary, None);
    }

    #[tokio::test]
    async fn set_summary_round_trips() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        set_summary(&pool, &id, "做了 A 和 B，結果成功").await.unwrap();
        assert_eq!(
            get_task(&pool, &id).await.unwrap().unwrap().ai_summary.as_deref(),
            Some("做了 A 和 B，結果成功")
        );
    }

    #[tokio::test]
    async fn re_dispatching_clears_a_stale_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        move_task(&pool, &id, STATUS_QUEUED, 1.0).await.unwrap();
        dispatch_for_test(&pool, &id, "tab-1").await;
        finish_task(&pool, &id, "success", None, None).await.unwrap();
        set_summary(&pool, &id, "第一次執行的摘要").await.unwrap();

        sqlx::query("UPDATE tasks SET status = 'queued' WHERE id = ?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        dispatch_for_test(&pool, &id, "tab-2").await;

        assert_eq!(
            get_task(&pool, &id).await.unwrap().unwrap().ai_summary,
            None,
            "重新派工後不可留著上一次執行的摘要"
        );
    }

    #[tokio::test]
    async fn a_cloned_task_does_not_inherit_the_summary() {
        let pool = mem_pool().await;
        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        set_summary(&pool, &id, "原卡片的摘要").await.unwrap();
        let clone_id = clone_task_fields(&pool, &id).await.unwrap();
        assert_eq!(get_task(&pool, &clone_id).await.unwrap().unwrap().ai_summary, None);
    }
}

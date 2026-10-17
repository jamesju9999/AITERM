//! 專案的 token 上限：專案（Agent 協作）用掉的 token 超過上限後，新的 Agent 卡
//! 先不派工，已經在跑的照樣跑完。上限存在專案自己的 tasks.db（`project_settings`）。
//!
//! 「用掉多少」跟畫面上的 Σ token 同一個算法（`usage::task_usage` 加總）。

use std::path::Path;

use sqlx::SqlitePool;

use crate::tasks::store::TaskRow;

const KEY: &str = "token_budget";

pub async fn get_budget(pool: &SqlitePool) -> Result<Option<u64>, sqlx::Error> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await?;
    Ok(v.and_then(|s| s.parse().ok()))
}

/// `None` 或 0 = 不設上限。
pub async fn set_budget(pool: &SqlitePool, budget: Option<u64>) -> Result<(), sqlx::Error> {
    match budget.filter(|b| *b > 0) {
        Some(b) => {
            sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(KEY)
                .bind(b.to_string())
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM project_settings WHERE key = ?").bind(KEY).execute(pool).await?;
        }
    }
    Ok(())
}

/// 這些卡片總共用了多少 token（含快取讀寫；跟畫面上的 Σ 一樣）。
pub fn total_tokens(project_path: &Path, projects_root: Option<&Path>, tasks: &[TaskRow]) -> u64 {
    tasks
        .iter()
        .filter(|t| t.dispatched_at.is_some())
        .filter_map(|t| {
            crate::tasks::usage::task_usage(project_path, projects_root, &t.id, t.session_id.as_deref(), t.session_path.as_deref())
        })
        .map(|u| u.input + u.cache_write + u.cache_read + u.output)
        .sum()
}

/// 專案有設上限而且已經用到（或超過）上限。讀檔放到 blocking 執行緒。
pub async fn over_budget(pool: &SqlitePool, project_path: &Path) -> Result<bool, sqlx::Error> {
    let Some(budget) = get_budget(pool).await? else { return Ok(false) };
    let tasks = crate::tasks::store::list_tasks(pool).await?;
    let path = project_path.to_path_buf();
    let used = tokio::task::spawn_blocking(move || {
        total_tokens(&path, crate::tasks::session_log::claude_projects_root().as_deref(), &tasks)
    })
    .await
    .unwrap_or(0);
    Ok(used >= budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store;
    use crate::tasks::usage::{save, TokenUsage};

    async fn mem_pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn the_budget_round_trips_and_zero_clears_it() {
        let pool = mem_pool().await;
        assert_eq!(get_budget(&pool).await.unwrap(), None);
        set_budget(&pool, Some(5_000_000)).await.unwrap();
        assert_eq!(get_budget(&pool).await.unwrap(), Some(5_000_000));
        set_budget(&pool, Some(2_000_000)).await.unwrap();
        assert_eq!(get_budget(&pool).await.unwrap(), Some(2_000_000));
        set_budget(&pool, Some(0)).await.unwrap();
        assert_eq!(get_budget(&pool).await.unwrap(), None);
    }

    #[tokio::test]
    async fn over_budget_counts_what_the_cards_used() {
        let pool = mem_pool().await;
        let project = tempfile::tempdir().unwrap();
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        sqlx::query("UPDATE tasks SET dispatched_at = 1 WHERE id = ?").bind(&id).execute(&pool).await.unwrap();
        save(project.path(), &id, &TokenUsage { input: 100, cache_write: 200, cache_read: 600, output: 100, calls: 3 });

        assert!(!over_budget(&pool, project.path()).await.unwrap(), "沒設上限");
        set_budget(&pool, Some(1001)).await.unwrap();
        assert!(!over_budget(&pool, project.path()).await.unwrap(), "用了 1000，還沒到");
        set_budget(&pool, Some(1000)).await.unwrap();
        assert!(over_budget(&pool, project.path()).await.unwrap(), "用到上限就算");
    }
}

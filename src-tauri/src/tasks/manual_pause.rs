//! 使用者手動暫停：不再派新卡給暫停的 Agent（已經在跑的卡照樣跑完，要中斷用「停止」）。
//!
//! 跟 `agents.paused_task_id` 是兩回事：那個是「有張卡失敗／被取消，要使用者決定重試或跳過」，
//! 這個是使用者自己按了暫停，沒有卡需要處理。存在專案自己的 tasks.db（`project_settings`），
//! 全部暫停是一個專案層級的旗標，不是替每位 Agent 各設一次——繼續時不會誤清掉個別暫停。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

const KEY: &str = "manual_pause";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualPause {
    /// 全部暫停（整個專案停手）。
    #[serde(default)]
    pub all: bool,
    /// 個別暫停的 Agent id。
    #[serde(default)]
    pub agents: Vec<String>,
}

pub async fn get(pool: &SqlitePool) -> Result<ManualPause, sqlx::Error> {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?")
        .bind(KEY)
        .fetch_optional(pool)
        .await?;
    Ok(v.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
}

async fn save(pool: &SqlitePool, state: &ManualPause) -> Result<(), sqlx::Error> {
    if *state == ManualPause::default() {
        sqlx::query("DELETE FROM project_settings WHERE key = ?").bind(KEY).execute(pool).await?;
        return Ok(());
    }
    sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(KEY)
        .bind(serde_json::to_string(state).unwrap_or_default())
        .execute(pool)
        .await?;
    Ok(())
}

/// `agent_id` 是 `None` = 全部暫停／繼續；否則只動那一位。
pub async fn set(pool: &SqlitePool, agent_id: Option<&str>, paused: bool) -> Result<(), sqlx::Error> {
    let mut state = get(pool).await?;
    match agent_id {
        None => state.all = paused,
        Some(id) => {
            state.agents.retain(|a| a != id);
            if paused {
                state.agents.push(id.to_string());
            }
        }
    }
    save(pool, &state).await
}

/// 這一輪排程要跳過的 Agent：全部暫停時是所有 Agent，否則是個別暫停的。
pub async fn blocked_agent_ids(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let state = get(pool).await?;
    if state.all {
        return Ok(crate::tasks::agents::list_agents(pool).await?.into_iter().map(|a| a.id).collect());
    }
    Ok(state.agents.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn coder(pool: &SqlitePool, name: &str) -> String {
        crate::tasks::agents::create_agent(
            pool,
            &crate::tasks::agents::NewAgent {
                kind: "coder",
                name,
                role: "r",
                prompt_prefix: "p",
                project_dir: "/repo",
                canvas_x: 0.0,
                canvas_y: 0.0,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn pausing_one_agent_blocks_only_that_agent_and_resume_clears_it() {
        let pool = mem_pool().await;
        let a = coder(&pool, "A").await;
        let b = coder(&pool, "B").await;
        assert!(blocked_agent_ids(&pool).await.unwrap().is_empty());

        set(&pool, Some(&a), true).await.unwrap();
        set(&pool, Some(&a), true).await.unwrap();
        assert_eq!(blocked_agent_ids(&pool).await.unwrap(), HashSet::from([a.clone()]), "重複暫停不會重複記");

        set(&pool, Some(&a), false).await.unwrap();
        assert!(blocked_agent_ids(&pool).await.unwrap().is_empty());
        let _ = b;
    }

    #[tokio::test]
    async fn pausing_all_blocks_everyone_and_keeps_individual_pauses_on_resume() {
        let pool = mem_pool().await;
        let a = coder(&pool, "A").await;
        let b = coder(&pool, "B").await;
        set(&pool, Some(&a), true).await.unwrap();
        set(&pool, None, true).await.unwrap();
        assert_eq!(blocked_agent_ids(&pool).await.unwrap(), HashSet::from([a.clone(), b.clone()]));

        // 全部繼續：只解除「全部」，先前個別暫停的 A 仍然暫停。
        set(&pool, None, false).await.unwrap();
        assert_eq!(blocked_agent_ids(&pool).await.unwrap(), HashSet::from([a]));
        assert!(!get(&pool).await.unwrap().all);
    }

    #[tokio::test]
    async fn nothing_paused_leaves_no_setting_behind() {
        let pool = mem_pool().await;
        let a = coder(&pool, "A").await;
        set(&pool, Some(&a), true).await.unwrap();
        set(&pool, Some(&a), false).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_settings WHERE key = 'manual_pause'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}

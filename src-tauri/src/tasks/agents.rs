//! Agent 協作的 Agent：一份常駐的「角色設定」，不是一個行程。
//! 指派給它的每張卡片仍是看板上獨立的一次派工，但同一個 Agent 的卡片
//! 依序執行、共用同一個 git worktree／分支。
//! 見 `docs/superpowers/specs/2026-10-15-agent-canvas-design.md` §1。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::tasks::store::{STATUS_PLANNING, STATUS_QUEUED, STATUS_RUNNING};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AgentRow {
    pub id: String,
    /// `coder` | `reviewer` | `orchestrator`。第一期只會建立 `coder`。
    pub kind: String,
    pub name: String,
    /// 角色預設的名稱（Senior Coder…），純顯示用；實際影響派工的是 `prompt_prefix`。
    pub role: String,
    pub prompt_prefix: String,
    /// 這個 Agent 負責的 git repo。建立後不可改——分支已經從它分出去了。
    pub project_dir: String,
    pub canvas_x: f64,
    pub canvas_y: f64,
    /// 第一次派工時才建立。
    pub worktree_path: Option<String>,
    pub branch: Option<String>,
    /// 非 `None` = 暫停中，指向造成暫停的那張卡。排程器跳過暫停中的 Agent。
    pub paused_task_id: Option<String>,
    pub created_at: String,
    /// Orchestrator：上一次為哪一組模組版本建立過合併卡（見 workflow::maybe_enqueue_merge）。
    pub last_merge_signature: Option<String>,
}

pub struct NewAgent<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub role: &'a str,
    pub prompt_prefix: &'a str,
    pub project_dir: &'a str,
    pub canvas_x: f64,
    pub canvas_y: f64,
}

/// `<專案資料夾>/agents/<agent_id>` — 這個 Agent 的 worktree 放在底下。
/// 跟 `task_dir` 同一個理由放在專案資料夾內：專案資料夾自成一體。
pub fn agent_dir(project_path: &Path, agent_id: &str) -> PathBuf {
    project_path.join("agents").join(agent_id)
}

/// Agent 分支名稱。用 id 而不是顯示名稱：名稱可以改、可以重複、可以含
/// git 不接受的字元。
pub fn branch_name(agent_id: &str) -> String {
    format!("aiterm-agent/{agent_id}")
}

/// 建立 Agent。Reviewer 與 Orchestrator 每個專案最多一個——連線是自動
/// 建立的（所有 Coder → 唯一的 Reviewer → 唯一的 Orchestrator）。
pub async fn create_agent(pool: &SqlitePool, a: &NewAgent<'_>) -> Result<String, sqlx::Error> {
    match a.kind {
        "coder" => {}
        "reviewer" | "orchestrator" => {
            let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents WHERE kind = ?")
                .bind(a.kind)
                .fetch_one(pool)
                .await?;
            if n > 0 {
                return Err(sqlx::Error::Protocol(format!("每個專案最多一個 {}", a.kind)));
            }
        }
        other => return Err(sqlx::Error::Protocol(format!("未知的 Agent 種類：{other}"))),
    }
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO agents (id, kind, name, role, prompt_prefix, project_dir, canvas_x, canvas_y)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(a.kind)
    .bind(a.name)
    .bind(a.role)
    .bind(a.prompt_prefix)
    .bind(a.project_dir)
    .bind(a.canvas_x)
    .bind(a.canvas_y)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list_agents(pool: &SqlitePool) -> Result<Vec<AgentRow>, sqlx::Error> {
    sqlx::query_as::<_, AgentRow>("SELECT * FROM agents ORDER BY created_at, id")
        .fetch_all(pool)
        .await
}

pub async fn get_agent(pool: &SqlitePool, id: &str) -> Result<Option<AgentRow>, sqlx::Error> {
    sqlx::query_as::<_, AgentRow>("SELECT * FROM agents WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_agent(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    role: &str,
    prompt_prefix: &str,
    canvas_x: f64,
    canvas_y: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE agents SET name = ?, role = ?, prompt_prefix = ?, canvas_x = ?, canvas_y = ? WHERE id = ?",
    )
    .bind(name)
    .bind(role)
    .bind(prompt_prefix)
    .bind(canvas_x)
    .bind(canvas_y)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_agent_worktree(
    pool: &SqlitePool,
    id: &str,
    path: &str,
    branch: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE agents SET worktree_path = ?, branch = ? WHERE id = ?")
        .bind(path)
        .bind(branch)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// `Some(task_id)` 暫停、`None` 解除暫停。
pub async fn set_paused(pool: &SqlitePool, id: &str, task_id: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE agents SET paused_task_id = ? WHERE id = ?")
        .bind(task_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 暫停中的 Agent id。排程器每一輪用它決定哪些 Agent 的佇列不能動。
pub async fn paused_agent_ids(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM agents WHERE paused_task_id IS NOT NULL")
        .fetch_all(pool)
        .await
}

async fn agent_has_running(pool: &SqlitePool, id: &str) -> Result<bool, sqlx::Error> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tasks WHERE agent_id = ? AND status = ? AND archived_at IS NULL",
    )
    .bind(id)
    .bind(STATUS_RUNNING)
    .fetch_one(pool)
    .await?;
    Ok(n > 0)
}

/// 刪除 Agent。有卡片正在跑時拒絕——背後有真的 claude 行程在它的
/// worktree 裡工作。名下待執行的卡片退回「計畫中」並清掉指派；已完成的
/// 卡片保留 `agent_id`（歷史紀錄）。worktree 與分支**不刪**，跟看板
/// 「不自動清理」的既有決定一致。
pub async fn delete_agent(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    if agent_has_running(pool, id).await? {
        return Err(sqlx::Error::Protocol("這個 Agent 還有工作在執行，不能刪除".into()));
    }
    let mut tx = pool.begin().await?;
    let base: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(MAX(sort_order), 0) AS REAL) FROM tasks WHERE status = ?",
    )
    .bind(STATUS_PLANNING)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE tasks SET agent_id = NULL, status = ?, sort_order = ? + sort_order
         WHERE agent_id = ? AND status = ?",
    )
    .bind(STATUS_PLANNING)
    .bind(base)
    .bind(id)
    .bind(STATUS_QUEUED)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM agents WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn max_order(pool: &SqlitePool, status: &str) -> Result<f64, sqlx::Error> {
    sqlx::query_scalar("SELECT CAST(COALESCE(MAX(sort_order), 0) AS REAL) FROM tasks WHERE status = ?")
        .bind(status)
        .fetch_one(pool)
        .await
}

/// 把卡片指派給 Agent（`Some`）或取消指派（`None`）。
///
/// 指派：卡片必須是計畫中或待執行（執行中的卡片背後有行程，改派會讓它
/// 跑在錯的 worktree 認知下）。`project_dir` 改成 Agent 的——看板上顯示
/// 的工作目錄才會跟實際一致。狀態轉成待執行並排到最後。
///
/// 取消指派：只有待執行的卡片可以，退回計畫中。
pub async fn assign_task(
    pool: &SqlitePool,
    task_id: &str,
    agent_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let status: Option<String> = sqlx::query_scalar("SELECT status FROM tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await?;
    let status = status.ok_or(sqlx::Error::RowNotFound)?;
    match agent_id {
        Some(aid) => {
            if status != STATUS_PLANNING && status != STATUS_QUEUED {
                return Err(sqlx::Error::Protocol("只有計畫中或待執行的卡片可以指派".into()));
            }
            let agent = get_agent(pool, aid).await?.ok_or(sqlx::Error::RowNotFound)?;
            if agent.kind != "coder" {
                return Err(sqlx::Error::Protocol("模組只能指派給 Coder Agent".into()));
            }
            let order = max_order(pool, STATUS_QUEUED).await? + 1.0;
            sqlx::query(
                "UPDATE tasks SET agent_id = ?, project_dir = ?, status = ?, sort_order = ? WHERE id = ?",
            )
            .bind(aid)
            .bind(&agent.project_dir)
            .bind(STATUS_QUEUED)
            .bind(order)
            .bind(task_id)
            .execute(pool)
            .await?;
        }
        None => {
            if status != STATUS_QUEUED {
                return Err(sqlx::Error::Protocol("只有待執行的卡片可以取消指派".into()));
            }
            let order = max_order(pool, STATUS_PLANNING).await? + 1.0;
            sqlx::query("UPDATE tasks SET agent_id = NULL, status = ?, sort_order = ? WHERE id = ?")
                .bind(STATUS_PLANNING)
                .bind(order)
                .bind(task_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// 只清掉指派、不動狀態。看板上把 Agent 卡片拖回「計畫中」時用——那時
/// `move_task` 已經處理好狀態與位置。
pub async fn clear_agent(pool: &SqlitePool, task_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET agent_id = NULL WHERE id = ?")
        .bind(task_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 這個 Agent 待執行卡片中最前面那張的 `sort_order`。重試時要插到它前面。
pub async fn first_queued_order(pool: &SqlitePool, agent_id: &str) -> Result<Option<f64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT CAST(MIN(sort_order) AS REAL) FROM tasks
         WHERE agent_id = ? AND status = ? AND archived_at IS NULL",
    )
    .bind(agent_id)
    .bind(STATUS_QUEUED)
    .fetch_one(pool)
    .await
}

/// App 重啟收尾：還停在執行中的 Agent 卡片，它的行程已經跟著上一個 App
/// 一起死了。`store::recover_orphaned_running` 會把卡片收成 cancelled，
/// 這裡要在**那之前**先把對應的 Agent 暫停——否則下一輪排程就會在半成品
/// worktree 上直接接著跑下一張。回傳暫停了幾個。
pub async fn pause_agents_of_orphaned_cards(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE agents SET paused_task_id = (
             SELECT t.id FROM tasks t
             WHERE t.agent_id = agents.id AND t.status = 'running'
             LIMIT 1
         )
         WHERE EXISTS (SELECT 1 FROM tasks t WHERE t.agent_id = agents.id AND t.status = 'running')",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    async fn coder(pool: &SqlitePool, name: &str) -> String {
        create_agent(
            pool,
            &NewAgent {
                kind: "coder",
                name,
                role: "Senior Coder",
                prompt_prefix: "你是資深工程師",
                project_dir: "/repo/htbktw",
                canvas_x: 10.0,
                canvas_y: 20.0,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_then_list_and_update_round_trips() {
        let pool = mem_pool().await;
        let id = coder(&pool, "Coder Agent1").await;
        let all = list_agents(&pool).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Coder Agent1");
        assert_eq!(all[0].project_dir, "/repo/htbktw");
        assert_eq!(all[0].paused_task_id, None);

        update_agent(&pool, &id, "改名", "Bug Fixer", "修 bug", 5.0, 6.0).await.unwrap();
        let a = get_agent(&pool, &id).await.unwrap().unwrap();
        assert_eq!((a.name.as_str(), a.role.as_str(), a.prompt_prefix.as_str()), ("改名", "Bug Fixer", "修 bug"));
        assert_eq!((a.canvas_x, a.canvas_y), (5.0, 6.0));
    }

    #[tokio::test]
    async fn assign_moves_a_planning_card_to_queued_and_takes_the_agent_project_dir() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        let other = store::create_task(&pool, "already queued", "", "/x", true, false).await.unwrap();
        store::move_task(&pool, &other, store::STATUS_QUEUED, 7.0).await.unwrap();
        let tid = store::create_task(&pool, "A 模組", "", "/somewhere/else", true, false).await.unwrap();

        assign_task(&pool, &tid, Some(&aid)).await.unwrap();

        let t = store::get_task(&pool, &tid).await.unwrap().unwrap();
        assert_eq!(t.agent_id.as_deref(), Some(aid.as_str()));
        assert_eq!(t.status, store::STATUS_QUEUED);
        assert_eq!(t.project_dir, "/repo/htbktw");
        assert!(t.sort_order > 7.0, "要排在既有待執行卡片之後，實際 {}", t.sort_order);
    }

    #[tokio::test]
    async fn assign_rejects_a_running_card() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        let tid = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &tid, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &tid, "tab").await;
        assert!(assign_task(&pool, &tid, Some(&aid)).await.is_err());
        assert!(assign_task(&pool, &tid, None).await.is_err());
    }

    #[tokio::test]
    async fn unassign_returns_a_queued_card_to_planning() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        let tid = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assign_task(&pool, &tid, Some(&aid)).await.unwrap();
        assign_task(&pool, &tid, None).await.unwrap();
        let t = store::get_task(&pool, &tid).await.unwrap().unwrap();
        assert_eq!(t.agent_id, None);
        assert_eq!(t.status, store::STATUS_PLANNING);
    }

    #[tokio::test]
    async fn delete_returns_queued_cards_keeps_done_ones_and_refuses_while_running() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        let queued = store::create_task(&pool, "queued", "", "/r", true, false).await.unwrap();
        let running = store::create_task(&pool, "running", "", "/r", true, false).await.unwrap();
        assign_task(&pool, &running, Some(&aid)).await.unwrap();
        assign_task(&pool, &queued, Some(&aid)).await.unwrap();
        store::dispatch_for_test(&pool, &running, "tab").await;

        assert!(delete_agent(&pool, &aid).await.is_err(), "有卡片在跑時必須拒絕");

        store::finish_task(&pool, &running, "success", None, None).await.unwrap();
        delete_agent(&pool, &aid).await.unwrap();

        assert!(get_agent(&pool, &aid).await.unwrap().is_none());
        let q = store::get_task(&pool, &queued).await.unwrap().unwrap();
        assert_eq!((q.status.as_str(), q.agent_id.as_deref()), (store::STATUS_PLANNING, None));
        let d = store::get_task(&pool, &running).await.unwrap().unwrap();
        assert_eq!(d.agent_id.as_deref(), Some(aid.as_str()), "完成的卡片保留歷史指派");
    }

    #[tokio::test]
    async fn paused_ids_track_set_paused() {
        let pool = mem_pool().await;
        let a = coder(&pool, "C1").await;
        let b = coder(&pool, "C2").await;
        set_paused(&pool, &b, Some("t9")).await.unwrap();
        assert_eq!(paused_agent_ids(&pool).await.unwrap(), vec![b.clone()]);
        set_paused(&pool, &b, None).await.unwrap();
        assert!(paused_agent_ids(&pool).await.unwrap().is_empty());
        let _ = a;
    }

    #[tokio::test]
    async fn restart_recovery_pauses_only_agents_with_a_running_card() {
        let pool = mem_pool().await;
        let busy = coder(&pool, "busy").await;
        let idle = coder(&pool, "idle").await;
        let t = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assign_task(&pool, &t, Some(&busy)).await.unwrap();
        store::dispatch_for_test(&pool, &t, "tab").await;

        assert_eq!(pause_agents_of_orphaned_cards(&pool).await.unwrap(), 1);
        assert_eq!(get_agent(&pool, &busy).await.unwrap().unwrap().paused_task_id.as_deref(), Some(t.as_str()));
        assert_eq!(get_agent(&pool, &idle).await.unwrap().unwrap().paused_task_id, None);
    }

    #[tokio::test]
    async fn only_one_reviewer_and_one_orchestrator_per_project() {
        let pool = mem_pool().await;
        let mk = |kind: &'static str| NewAgent {
            kind, name: kind, role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
        };
        create_agent(&pool, &mk("reviewer")).await.unwrap();
        assert!(create_agent(&pool, &mk("reviewer")).await.is_err());
        create_agent(&pool, &mk("orchestrator")).await.unwrap();
        assert!(create_agent(&pool, &mk("orchestrator")).await.is_err());
        create_agent(&pool, &mk("coder")).await.unwrap();
        create_agent(&pool, &mk("coder")).await.unwrap();
        assert!(create_agent(&pool, &mk("pilot")).await.is_err());
    }

    #[tokio::test]
    async fn modules_cannot_be_assigned_to_a_reviewer() {
        let pool = mem_pool().await;
        let rid = create_agent(&pool, &NewAgent {
            kind: "reviewer", name: "R", role: "r", prompt_prefix: "", project_dir: "/r", canvas_x: 0.0, canvas_y: 0.0,
        }).await.unwrap();
        let tid = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assert!(assign_task(&pool, &tid, Some(&rid)).await.is_err());
    }

    #[tokio::test]
    async fn clear_agent_only_drops_the_assignment() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        let tid = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        assign_task(&pool, &tid, Some(&aid)).await.unwrap();
        clear_agent(&pool, &tid).await.unwrap();
        let t = store::get_task(&pool, &tid).await.unwrap().unwrap();
        assert_eq!(t.agent_id, None);
        assert_eq!(t.status, store::STATUS_QUEUED, "狀態由 move_task 負責，這裡不能動");
    }

    #[tokio::test]
    async fn first_queued_order_is_the_agents_head() {
        let pool = mem_pool().await;
        let aid = coder(&pool, "C1").await;
        assert_eq!(first_queued_order(&pool, &aid).await.unwrap(), None);
        let t1 = store::create_task(&pool, "1", "", "/r", true, false).await.unwrap();
        let t2 = store::create_task(&pool, "2", "", "/r", true, false).await.unwrap();
        assign_task(&pool, &t1, Some(&aid)).await.unwrap();
        assign_task(&pool, &t2, Some(&aid)).await.unwrap();
        let o1 = store::get_task(&pool, &t1).await.unwrap().unwrap().sort_order;
        assert_eq!(first_queued_order(&pool, &aid).await.unwrap(), Some(o1));
    }
}

//! 團隊編制：一個專案可以存好幾組團隊，依階段切換（盤點 → 轉換 → 實測）。
//! 同一時間桌布上只有一組；卡片、活動記錄與專案設定屬於專案，換編制時都保留。
//! 見 `docs/superpowers/specs/2026-10-20-agent-canvas-team-formations-design.md`。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::tasks::agents::{self, NewAgent};

const ACTIVE_KEY: &str = "active_formation";
const PENDING_KEY: &str = "pending_formation";

fn default_cli() -> String {
    agents::CLI_CLAUDE.to_string()
}

/// 編制裡的一位 Agent：重建它需要的全部設定（不含程式庫，切換時用專案的）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormationAgent {
    pub kind: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub prompt_prefix: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_cli")]
    pub cli: String,
    #[serde(default)]
    pub produces_spec: bool,
    /// 桌布上的位置。
    pub dx: f64,
    pub dy: f64,
    #[serde(default)]
    pub db_skill: Option<String>,
    #[serde(default)]
    pub custom_skills: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormationSummary {
    pub id: String,
    pub name: String,
    pub agent_count: usize,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct FormationLogRow {
    pub at: i64,
    pub from_name: Option<String>,
    pub to_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormationsState {
    pub formations: Vec<FormationSummary>,
    pub active: Option<String>,
    pub pending: Option<String>,
    /// 還在工作、擋住切換的 Agent 名稱。
    pub busy: Vec<String>,
    pub log: Vec<FormationLogRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SwitchOutcome {
    Switched,
    Busy { names: Vec<String> },
}

/// 已經從桌布移除的 Agent：已完成卡片的歷史還指向它，畫面要顯示得出是誰。
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
pub struct RetiredAgent {
    pub id: String,
    pub name: String,
    pub kind: String,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS formations (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            agents TEXT NOT NULL,
            sort_order REAL NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS formation_log (at INTEGER NOT NULL, from_name TEXT, to_name TEXT NOT NULL)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS retired_agents (
            id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, retired_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn to_json(agents: &[FormationAgent]) -> String {
    serde_json::to_string(agents).unwrap_or_else(|_| "[]".into())
}

pub async fn create(pool: &SqlitePool, name: &str, agents: &[FormationAgent]) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let order: f64 = sqlx::query_scalar("SELECT CAST(COALESCE(MAX(sort_order), 0) + 1 AS REAL) FROM formations")
        .fetch_one(pool)
        .await?;
    sqlx::query("INSERT INTO formations (id, name, agents, sort_order, updated_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(name.trim())
        .bind(to_json(agents))
        .bind(order)
        .bind(now())
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<FormationSummary>, sqlx::Error> {
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, agents FROM formations ORDER BY sort_order, id").fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, raw)| FormationSummary {
            id,
            name,
            agent_count: serde_json::from_str::<Vec<FormationAgent>>(&raw).map(|v| v.len()).unwrap_or(0),
        })
        .collect())
}

pub async fn agents_of(pool: &SqlitePool, id: &str) -> Result<Option<Vec<FormationAgent>>, sqlx::Error> {
    let raw: Option<String> = sqlx::query_scalar("SELECT agents FROM formations WHERE id = ?").bind(id).fetch_optional(pool).await?;
    Ok(raw.map(|r| serde_json::from_str(&r).unwrap_or_default()))
}

async fn name_of(pool: &SqlitePool, id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM formations WHERE id = ?").bind(id).fetch_optional(pool).await
}

async fn save_agents(pool: &SqlitePool, id: &str, agents: &[FormationAgent]) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE formations SET agents = ?, updated_at = ? WHERE id = ?")
        .bind(to_json(agents))
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn rename(pool: &SqlitePool, id: &str, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE formations SET name = ? WHERE id = ?").bind(name.trim()).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), String> {
    if active(pool).await.map_err(|e| e.to_string())?.as_deref() == Some(id) {
        return Err("不能刪除目前的編制".into());
    }
    if pending(pool).await.map_err(|e| e.to_string())?.as_deref() == Some(id) {
        set_pending(pool, None).await.map_err(|e| e.to_string())?;
    }
    sqlx::query("DELETE FROM formations WHERE id = ?").bind(id).execute(pool).await.map_err(|e| e.to_string())?;
    Ok(())
}

async fn setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM project_settings WHERE key = ?").bind(key).fetch_optional(pool).await
}

async fn set_setting(pool: &SqlitePool, key: &str, value: Option<&str>) -> Result<(), sqlx::Error> {
    match value {
        Some(v) => {
            sqlx::query("INSERT INTO project_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
                .bind(key)
                .bind(v)
                .execute(pool)
                .await?;
        }
        None => {
            sqlx::query("DELETE FROM project_settings WHERE key = ?").bind(key).execute(pool).await?;
        }
    }
    Ok(())
}

pub async fn active(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    setting(pool, ACTIVE_KEY).await
}

pub async fn set_active(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    set_setting(pool, ACTIVE_KEY, Some(id)).await
}

pub async fn pending(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    setting(pool, PENDING_KEY).await
}

pub async fn set_pending(pool: &SqlitePool, id: Option<&str>) -> Result<(), sqlx::Error> {
    set_setting(pool, PENDING_KEY, id).await
}

/// 目前桌布上的團隊。
pub async fn snapshot(pool: &SqlitePool) -> Result<Vec<FormationAgent>, sqlx::Error> {
    Ok(agents::list_agents(pool)
        .await?
        .into_iter()
        .map(|a| FormationAgent {
            kind: a.kind,
            name: a.name,
            role: a.role,
            prompt_prefix: a.prompt_prefix,
            model: a.model.unwrap_or_default(),
            cli: a.cli,
            produces_spec: a.produces_spec,
            dx: a.canvas_x,
            dy: a.canvas_y,
            db_skill: a.db_skill,
            custom_skills: a.custom_skills,
        })
        .collect())
}

/// 還在工作的 Agent：有執行中的卡片，或排隊中的品管流程卡（審查、修正、合併、驗收、拆解）——
/// 一張模組還沒走完。暫停中的 Agent 不算：它的佇列不會自己動，等它會永遠等不到。
pub async fn busy_agents(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT a.name FROM agents a WHERE EXISTS (
            SELECT 1 FROM tasks t WHERE t.agent_id = a.id AND t.archived_at IS NULL AND (
                t.status = 'running'
                OR (t.status = 'queued' AND t.card_kind IN ('review', 'fix', 'merge', 'verify', 'plan') AND a.paused_task_id IS NULL)
            )
        ) ORDER BY a.created_at, a.id",
    )
    .fetch_all(pool)
    .await
}

pub async fn retired(pool: &SqlitePool) -> Result<Vec<RetiredAgent>, sqlx::Error> {
    sqlx::query_as::<_, RetiredAgent>("SELECT id, name, kind FROM retired_agents").fetch_all(pool).await
}

/// 目前的程式庫路徑：桌布上 Agent 的 → 專案記住的。不檢查是不是可用的 repo（指令層會檢查）。
pub async fn repo_path(pool: &SqlitePool) -> Result<Option<String>, sqlx::Error> {
    let dirs: Vec<String> = agents::list_agents(pool).await?.into_iter().map(|a| a.project_dir).collect();
    let saved = crate::tasks::repo_check::saved_project_repo(pool).await?;
    Ok(crate::tasks::repo_check::project_repo(&dirs, saved).map(|r| r.path))
}

async fn build(pool: &SqlitePool, a: &FormationAgent, repo: &str) -> Result<(), String> {
    let e = |e: sqlx::Error| e.to_string();
    let id = agents::create_agent(
        pool,
        &NewAgent {
            kind: &a.kind,
            name: &a.name,
            role: &a.role,
            prompt_prefix: &a.prompt_prefix,
            project_dir: repo,
            canvas_x: a.dx,
            canvas_y: a.dy,
        },
    )
    .await
    .map_err(e)?;
    if a.produces_spec && a.kind == "coder" {
        agents::set_produces_spec(pool, &id, true).await.map_err(e)?;
    }
    agents::set_model(pool, &id, Some(&a.model)).await.map_err(e)?;
    agents::set_cli(pool, &id, &a.cli).await.map_err(e)?;
    sqlx::query("UPDATE agents SET db_skill = ?, custom_skills = ? WHERE id = ?")
        .bind(&a.db_skill)
        .bind(&a.custom_skills)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(e)?;
    Ok(())
}

/// 切換到 `target`：先把目前桌布存回目前的編制，移除全部 Agent（排隊中的模組退回模組庫），
/// 再依目標建立。有人在工作就不動，回 `Busy`。
pub async fn switch_to(pool: &SqlitePool, target: &str, repo: Option<&str>) -> Result<SwitchOutcome, String> {
    let e = |e: sqlx::Error| e.to_string();
    let target_agents = agents_of(pool, target).await.map_err(e)?.ok_or("找不到這組編制")?;
    let from = active(pool).await.map_err(e)?;
    if from.as_deref() == Some(target) {
        set_pending(pool, None).await.map_err(e)?;
        return Ok(SwitchOutcome::Switched);
    }
    let busy = busy_agents(pool).await.map_err(e)?;
    if !busy.is_empty() {
        return Ok(SwitchOutcome::Busy { names: busy });
    }
    let repo = match (repo, target_agents.is_empty()) {
        (Some(r), _) => r,
        (None, true) => "",
        (None, false) => return Err("這組編制要先設定專案資料夾".into()),
    };
    let from_name = match &from {
        Some(f) => {
            save_agents(pool, f, &snapshot(pool).await.map_err(e)?).await.map_err(e)?;
            name_of(pool, f).await.map_err(e)?
        }
        None => None,
    };
    agents::delete_all_agents(pool).await?;
    for a in &target_agents {
        build(pool, a, repo).await?;
    }
    set_active(pool, target).await.map_err(e)?;
    set_pending(pool, None).await.map_err(e)?;
    let to_name = name_of(pool, target).await.map_err(e)?.unwrap_or_default();
    sqlx::query("INSERT INTO formation_log (at, from_name, to_name) VALUES (?, ?, ?)")
        .bind(now())
        .bind(from_name)
        .bind(to_name)
        .execute(pool)
        .await
        .map_err(e)?;
    Ok(SwitchOutcome::Switched)
}

/// 把目前桌布另存成新編制並設為目前（舊的那組先存下最新的桌布，之後的變動算在新的）。
pub async fn save_as(pool: &SqlitePool, name: &str) -> Result<String, sqlx::Error> {
    let snap = snapshot(pool).await?;
    if let Some(f) = active(pool).await? {
        save_agents(pool, &f, &snap).await?;
    }
    let id = create(pool, name, &snap).await?;
    set_active(pool, &id).await?;
    Ok(id)
}

/// 排程器每一輪呼叫：有等待中的切換而且沒人在工作了，就切換。回傳有沒有切換。
pub async fn try_complete_pending(pool: &SqlitePool) -> Result<bool, String> {
    let Some(target) = pending(pool).await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    if !busy_agents(pool).await.map_err(|e| e.to_string())?.is_empty() {
        return Ok(false);
    }
    let repo = repo_path(pool).await.map_err(|e| e.to_string())?;
    match switch_to(pool, &target, repo.as_deref()).await? {
        SwitchOutcome::Switched => Ok(true),
        SwitchOutcome::Busy { .. } => Ok(false),
    }
}

/// 舊專案：沒有任何編制但桌布上有 Agent → 用目前的團隊建立一組並設為目前。
pub async fn ensure_default(pool: &SqlitePool, name: &str) -> Result<(), sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM formations").fetch_one(pool).await?;
    if n > 0 || agents::list_agents(pool).await?.is_empty() {
        return Ok(());
    }
    let snap = snapshot(pool).await?;
    let id = create(pool, name, &snap).await?;
    set_active(pool, &id).await
}

pub async fn state(pool: &SqlitePool) -> Result<FormationsState, sqlx::Error> {
    Ok(FormationsState {
        formations: list(pool).await?,
        active: active(pool).await?,
        pending: pending(pool).await?,
        busy: busy_agents(pool).await?,
        log: sqlx::query_as::<_, FormationLogRow>("SELECT at, from_name, to_name FROM formation_log ORDER BY at DESC, rowid DESC LIMIT 20")
            .fetch_all(pool)
            .await?,
    })
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

    fn fa(kind: &str, name: &str) -> FormationAgent {
        FormationAgent {
            kind: kind.into(),
            name: name.into(),
            role: format!("{name}的角色"),
            prompt_prefix: format!("你是{name}"),
            model: String::new(),
            cli: "claude".into(),
            produces_spec: false,
            dx: 40.0,
            dy: 40.0,
            db_skill: None,
            custom_skills: None,
        }
    }

    const REPO: &str = "/repo/htbktw";

    /// 建一組編制並切過去，回傳 id。
    async fn active_with(pool: &SqlitePool, name: &str, agents: &[FormationAgent]) -> String {
        let id = create(pool, name, agents).await.unwrap();
        assert_eq!(switch_to(pool, &id, Some(REPO)).await.unwrap(), SwitchOutcome::Switched);
        id
    }

    #[tokio::test]
    async fn create_list_rename_delete() {
        let pool = mem_pool().await;
        let a = create(&pool, "盤點", &[fa("coder", "工程師1"), fa("reviewer", "審核者")]).await.unwrap();
        let b = create(&pool, "轉換", &[]).await.unwrap();
        let l = list(&pool).await.unwrap();
        assert_eq!(l.iter().map(|f| (f.name.as_str(), f.agent_count)).collect::<Vec<_>>(), vec![("盤點", 2), ("轉換", 0)]);
        rename(&pool, &b, " 實測 ").await.unwrap();
        assert_eq!(list(&pool).await.unwrap()[1].name, "實測");
        delete(&pool, &b).await.unwrap();
        assert_eq!(list(&pool).await.unwrap().iter().map(|f| f.id.clone()).collect::<Vec<_>>(), vec![a]);
    }

    #[tokio::test]
    async fn cannot_delete_active() {
        let pool = mem_pool().await;
        let a = create(&pool, "盤點", &[]).await.unwrap();
        set_active(&pool, &a).await.unwrap();
        assert!(delete(&pool, &a).await.is_err());
        assert_eq!(list(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn agents_json_round_trips() {
        let pool = mem_pool().await;
        let mut x = fa("coder", "工程師1");
        x.db_skill = Some(r#"{"connection_id":"dev","mode":"read"}"#.into());
        x.custom_skills = Some(r#"["/s/a"]"#.into());
        x.produces_spec = true;
        x.cli = "codex".into();
        let id = create(&pool, "盤點", std::slice::from_ref(&x)).await.unwrap();
        assert_eq!(agents_of(&pool, &id).await.unwrap().unwrap(), vec![x]);
    }

    #[tokio::test]
    async fn snapshot_captures_skills_and_positions() {
        let pool = mem_pool().await;
        let mut x = fa("coder", "工程師1");
        x.db_skill = Some(r#"{"connection_id":"dev"}"#.into());
        x.custom_skills = Some(r#"["/s/a"]"#.into());
        x.model = "opus".into();
        x.cli = "codex".into();
        x.produces_spec = true;
        x.dx = 123.0;
        x.dy = 456.0;
        active_with(&pool, "盤點", std::slice::from_ref(&x)).await;
        assert_eq!(snapshot(&pool).await.unwrap(), vec![x]);
    }

    #[tokio::test]
    async fn busy_counts_running_and_queued_pipeline_but_not_paused_or_queued_work() {
        let pool = mem_pool().await;
        active_with(&pool, "A", &[fa("coder", "工程師1"), fa("coder", "工程師2"), fa("coder", "工程師3")]).await;
        // 同一秒建立的 Agent 依 id 排序，順序不固定：照名稱找。
        let all = agents::list_agents(&pool).await.unwrap();
        let id_of = |n: &str| all.iter().find(|a| a.name == n).unwrap().id.clone();
        let ids = [id_of("工程師1"), id_of("工程師2")];

        // 排隊中的模組卡不算。
        let w = store::create_task(&pool, "w", "", REPO, true, false).await.unwrap();
        agents::assign_task(&pool, &w, Some(&ids[0])).await.unwrap();
        assert!(busy_agents(&pool).await.unwrap().is_empty());

        // 執行中算。
        store::dispatch_for_test(&pool, &w, "tab").await;
        assert_eq!(busy_agents(&pool).await.unwrap(), vec!["工程師1"]);

        // 排隊中的審查卡算；Agent 暫停時不算。
        let r = store::create_task(&pool, "r", "", REPO, true, false).await.unwrap();
        agents::assign_task(&pool, &r, Some(&ids[1])).await.unwrap();
        sqlx::query("UPDATE tasks SET card_kind = 'review' WHERE id = ?").bind(&r).execute(&pool).await.unwrap();
        let mut both = busy_agents(&pool).await.unwrap();
        both.sort();
        assert_eq!(both, vec!["工程師1", "工程師2"]);
        agents::set_paused(&pool, &ids[1], Some("x")).await.unwrap();
        assert_eq!(busy_agents(&pool).await.unwrap(), vec!["工程師1"]);
    }

    #[tokio::test]
    async fn switch_saves_current_then_builds_target() {
        let pool = mem_pool().await;
        let a = active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        // 切換前在桌布上又加了一位、裝了技能。
        let added = agents::create_agent(
            &pool,
            &NewAgent { kind: "coder", name: "工程師2", role: "r", prompt_prefix: "", project_dir: REPO, canvas_x: 9.0, canvas_y: 8.0 },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE agents SET custom_skills = '[\"/s/x\"]' WHERE id = ?").bind(&added).execute(&pool).await.unwrap();

        let b = create(&pool, "轉換", &[fa("reviewer", "審核者")]).await.unwrap();
        assert_eq!(switch_to(&pool, &b, Some(REPO)).await.unwrap(), SwitchOutcome::Switched);

        let saved = agents_of(&pool, &a).await.unwrap().unwrap();
        let mut names: Vec<&str> = saved.iter().map(|x| x.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["工程師1", "工程師2"]);
        let two = saved.iter().find(|x| x.name == "工程師2").unwrap();
        assert_eq!(two.custom_skills.as_deref(), Some("[\"/s/x\"]"));
        let now: Vec<(String, String)> = agents::list_agents(&pool).await.unwrap().into_iter().map(|x| (x.kind, x.project_dir)).collect();
        assert_eq!(now, vec![("reviewer".to_string(), REPO.to_string())]);
        assert_eq!(active(&pool).await.unwrap().as_deref(), Some(b.as_str()));
        let st = state(&pool).await.unwrap();
        assert_eq!((st.log[0].from_name.as_deref(), st.log[0].to_name.as_str()), (Some("盤點"), "轉換"));
    }

    #[tokio::test]
    async fn switch_back_restores_skills() {
        let pool = mem_pool().await;
        let mut x = fa("coder", "工程師1");
        x.db_skill = Some(r#"{"connection_id":"dev"}"#.into());
        x.model = "sonnet".into();
        let a = active_with(&pool, "盤點", std::slice::from_ref(&x)).await;
        let first_id = agents::list_agents(&pool).await.unwrap()[0].id.clone();
        let b = create(&pool, "轉換", &[]).await.unwrap();
        switch_to(&pool, &b, Some(REPO)).await.unwrap();
        assert!(agents::list_agents(&pool).await.unwrap().is_empty());
        switch_to(&pool, &a, Some(REPO)).await.unwrap();
        let back = agents::list_agents(&pool).await.unwrap();
        assert_eq!(back.len(), 1);
        assert_ne!(back[0].id, first_id);
        assert_eq!(snapshot(&pool).await.unwrap(), vec![x]);
    }

    #[tokio::test]
    async fn switch_refuses_when_busy() {
        let pool = mem_pool().await;
        active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        let aid = agents::list_agents(&pool).await.unwrap()[0].id.clone();
        let w = store::create_task(&pool, "w", "", REPO, true, false).await.unwrap();
        agents::assign_task(&pool, &w, Some(&aid)).await.unwrap();
        store::dispatch_for_test(&pool, &w, "tab").await;
        let b = create(&pool, "轉換", &[]).await.unwrap();
        assert_eq!(switch_to(&pool, &b, Some(REPO)).await.unwrap(), SwitchOutcome::Busy { names: vec!["工程師1".into()] });
        assert_eq!(agents::list_agents(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn switch_needs_repo_for_non_empty_target() {
        let pool = mem_pool().await;
        active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        let b = create(&pool, "轉換", &[fa("coder", "工程師2")]).await.unwrap();
        assert!(switch_to(&pool, &b, None).await.is_err());
        assert_eq!(agents::list_agents(&pool).await.unwrap()[0].name, "工程師1");
    }

    #[tokio::test]
    async fn queued_work_returns_to_library() {
        let pool = mem_pool().await;
        active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        let aid = agents::list_agents(&pool).await.unwrap()[0].id.clone();
        let w = store::create_task(&pool, "w", "", REPO, true, false).await.unwrap();
        agents::assign_task(&pool, &w, Some(&aid)).await.unwrap();
        let b = create(&pool, "轉換", &[]).await.unwrap();
        switch_to(&pool, &b, Some(REPO)).await.unwrap();
        let t = store::get_task(&pool, &w).await.unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.agent_id.as_deref()), (store::STATUS_PLANNING, None));
    }

    #[tokio::test]
    async fn try_complete_pending_waits_for_busy_then_switches() {
        let pool = mem_pool().await;
        active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        let aid = agents::list_agents(&pool).await.unwrap()[0].id.clone();
        let w = store::create_task(&pool, "w", "", REPO, true, false).await.unwrap();
        agents::assign_task(&pool, &w, Some(&aid)).await.unwrap();
        store::dispatch_for_test(&pool, &w, "tab").await;
        let b = create(&pool, "轉換", &[fa("coder", "工程師9")]).await.unwrap();

        assert!(!try_complete_pending(&pool).await.unwrap(), "沒有等待中的切換");
        set_pending(&pool, Some(&b)).await.unwrap();
        assert!(!try_complete_pending(&pool).await.unwrap(), "還有人在工作");
        assert_eq!(agents::list_agents(&pool).await.unwrap()[0].name, "工程師1");

        store::finish_task(&pool, &w, "success", None, None).await.unwrap();
        assert!(try_complete_pending(&pool).await.unwrap());
        let now = agents::list_agents(&pool).await.unwrap();
        assert_eq!((now[0].name.as_str(), now[0].project_dir.as_str()), ("工程師9", REPO));
        assert_eq!(pending(&pool).await.unwrap(), None);
    }

    #[tokio::test]
    async fn ensure_default_only_when_agents_and_no_formations() {
        let pool = mem_pool().await;
        ensure_default(&pool, "預設").await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty(), "沒有 Agent 不建");

        agents::create_agent(
            &pool,
            &NewAgent { kind: "coder", name: "工程師1", role: "r", prompt_prefix: "", project_dir: REPO, canvas_x: 0.0, canvas_y: 0.0 },
        )
        .await
        .unwrap();
        ensure_default(&pool, "預設").await.unwrap();
        ensure_default(&pool, "預設").await.unwrap();
        let l = list(&pool).await.unwrap();
        assert_eq!(l.iter().map(|f| (f.name.as_str(), f.agent_count)).collect::<Vec<_>>(), vec![("預設", 1)]);
        assert_eq!(active(&pool).await.unwrap(), Some(l[0].id.clone()));
    }

    #[tokio::test]
    async fn save_as_keeps_old_and_makes_new_active() {
        let pool = mem_pool().await;
        let a = active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        agents::create_agent(
            &pool,
            &NewAgent { kind: "reviewer", name: "審核者", role: "r", prompt_prefix: "", project_dir: REPO, canvas_x: 0.0, canvas_y: 0.0 },
        )
        .await
        .unwrap();
        let b = save_as(&pool, "盤點2").await.unwrap();
        assert_eq!(active(&pool).await.unwrap().as_deref(), Some(b.as_str()));
        assert_eq!(agents_of(&pool, &a).await.unwrap().unwrap().len(), 2, "舊的那組存下最新的桌布");
        assert_eq!(agents_of(&pool, &b).await.unwrap().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn deleted_agents_are_remembered() {
        let pool = mem_pool().await;
        active_with(&pool, "盤點", &[fa("coder", "工程師1")]).await;
        let aid = agents::list_agents(&pool).await.unwrap()[0].id.clone();
        agents::delete_agent(&pool, &aid).await.unwrap();
        assert_eq!(retired(&pool).await.unwrap(), vec![RetiredAgent { id: aid, name: "工程師1".into(), kind: "coder".into() }]);
    }
}

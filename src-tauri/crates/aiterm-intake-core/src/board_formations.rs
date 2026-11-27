//! 團隊編制（server 版）：一個共用專案可以存好幾組團隊，依階段切換。同一時間桌布上只有一組；
//! 卡片、活動記錄與專案設定屬於專案，換編制時都保留。規則同桌面版 `tasks::formations`，
//! 見 docs/superpowers/specs/2026-11-27-server-ask-human-formations-design.md。
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::board_store::{self as bs, NewAgent};

const ACTIVE_KEY: &str = "active_formation";
const PENDING_KEY: &str = "pending_formation";

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn default_cli() -> String {
    "claude".to_string()
}

/// 編制裡的一位 Agent：重建它需要的全部設定。
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
    /// 系統連線（server 系統 id 的 JSON 陣列）。
    #[serde(default)]
    pub systems: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormationSummary {
    pub id: String,
    pub name: String,
    pub agent_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct FormationLogRow {
    pub at: i64,
    pub from_name: Option<String>,
    pub to_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormationsState {
    pub formations: Vec<FormationSummary>,
    pub active: Option<String>,
    pub pending: Option<String>,
    /// 還在工作、擋住切換的 Agent 名稱。
    pub busy: Vec<String>,
    pub log: Vec<FormationLogRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SwitchOutcome {
    Switched,
    Busy { names: Vec<String> },
}

/// 已經從桌布移除的 Agent：已完成卡片的歷史還指向它，畫面要顯示得出是誰。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct RetiredAgent {
    pub id: String,
    pub name: String,
    pub kind: String,
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    for sql in [
        "CREATE TABLE IF NOT EXISTS board_formations (id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL, name TEXT NOT NULL, agents TEXT NOT NULL, sort_order REAL NOT NULL, created_by TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL)",
        "CREATE INDEX IF NOT EXISTS board_formations_project ON board_formations(project_id, sort_order)",
        "CREATE TABLE IF NOT EXISTS board_formation_log (project_id TEXT NOT NULL, at INTEGER NOT NULL, from_name TEXT, to_name TEXT NOT NULL, by TEXT NOT NULL DEFAULT '')",
        "CREATE TABLE IF NOT EXISTS board_formation_state (project_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (project_id, key))",
        "CREATE TABLE IF NOT EXISTS board_retired_agents (project_id TEXT NOT NULL, id TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, retired_at INTEGER NOT NULL, PRIMARY KEY (project_id, id))",
    ] {
        sqlx::query(sql).execute(pool).await?;
    }
    Ok(())
}

fn to_json(agents: &[FormationAgent]) -> String {
    serde_json::to_string(agents).unwrap_or_else(|_| "[]".into())
}

pub async fn create(pool: &SqlitePool, pid: &str, name: &str, agents: &[FormationAgent], by: &str) -> Result<Result<String, String>, sqlx::Error> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(Err("編制名稱不能是空的".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let order: f64 = sqlx::query_scalar("SELECT CAST(COALESCE(MAX(sort_order), 0) + 1 AS REAL) FROM board_formations WHERE project_id = ?").bind(pid).fetch_one(pool).await?;
    sqlx::query("INSERT INTO board_formations (id, project_id, name, agents, sort_order, created_by, updated_at) VALUES (?,?,?,?,?,?,?)")
        .bind(&id).bind(pid).bind(name).bind(to_json(agents)).bind(order).bind(by).bind(now()).execute(pool).await?;
    Ok(Ok(id))
}

pub async fn list(pool: &SqlitePool, pid: &str) -> Result<Vec<FormationSummary>, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT id, name, agents FROM board_formations WHERE project_id = ? ORDER BY sort_order, id").bind(pid).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|(id, name, raw)| FormationSummary { id, name, agent_count: serde_json::from_str::<Vec<FormationAgent>>(&raw).map(|v| v.len()).unwrap_or(0) }).collect())
}

async fn agents_of(pool: &SqlitePool, pid: &str, id: &str) -> Result<Option<Vec<FormationAgent>>, sqlx::Error> {
    let raw: Option<String> = sqlx::query_scalar("SELECT agents FROM board_formations WHERE id = ? AND project_id = ?").bind(id).bind(pid).fetch_optional(pool).await?;
    Ok(raw.map(|r| serde_json::from_str(&r).unwrap_or_default()))
}

async fn name_of(pool: &SqlitePool, pid: &str, id: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM board_formations WHERE id = ? AND project_id = ?").bind(id).bind(pid).fetch_optional(pool).await
}

async fn save_agents(pool: &SqlitePool, pid: &str, id: &str, agents: &[FormationAgent]) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE board_formations SET agents = ?, updated_at = ? WHERE id = ? AND project_id = ?").bind(to_json(agents)).bind(now()).bind(id).bind(pid).execute(pool).await?;
    Ok(())
}

pub async fn rename(pool: &SqlitePool, pid: &str, id: &str, name: &str) -> Result<Result<(), String>, sqlx::Error> {
    if name.trim().is_empty() {
        return Ok(Err("編制名稱不能是空的".into()));
    }
    let r = sqlx::query("UPDATE board_formations SET name = ? WHERE id = ? AND project_id = ?").bind(name.trim()).bind(id).bind(pid).execute(pool).await?;
    Ok(if r.rows_affected() == 0 { Err("找不到這組編制".into()) } else { Ok(()) })
}

pub async fn delete(pool: &SqlitePool, pid: &str, id: &str) -> Result<Result<(), String>, sqlx::Error> {
    if active(pool, pid).await?.as_deref() == Some(id) {
        return Ok(Err("不能刪除目前的編制".into()));
    }
    if pending(pool, pid).await?.as_deref() == Some(id) {
        set_state(pool, pid, PENDING_KEY, None).await?;
    }
    sqlx::query("DELETE FROM board_formations WHERE id = ? AND project_id = ?").bind(id).bind(pid).execute(pool).await?;
    Ok(Ok(()))
}

async fn state_value(pool: &SqlitePool, pid: &str, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM board_formation_state WHERE project_id = ? AND key = ?").bind(pid).bind(key).fetch_optional(pool).await
}

async fn set_state(pool: &SqlitePool, pid: &str, key: &str, value: Option<&str>) -> Result<(), sqlx::Error> {
    match value {
        Some(v) => {
            sqlx::query("INSERT INTO board_formation_state (project_id, key, value) VALUES (?,?,?) ON CONFLICT(project_id, key) DO UPDATE SET value = excluded.value").bind(pid).bind(key).bind(v).execute(pool).await?;
        }
        None => {
            sqlx::query("DELETE FROM board_formation_state WHERE project_id = ? AND key = ?").bind(pid).bind(key).execute(pool).await?;
        }
    }
    Ok(())
}

pub async fn active(pool: &SqlitePool, pid: &str) -> Result<Option<String>, sqlx::Error> {
    state_value(pool, pid, ACTIVE_KEY).await
}

pub async fn pending(pool: &SqlitePool, pid: &str) -> Result<Option<String>, sqlx::Error> {
    state_value(pool, pid, PENDING_KEY).await
}

/// 等目前的工作做完後再切換到 `id`（只記下來，由排程每一輪嘗試）。
pub async fn request_switch(pool: &SqlitePool, pid: &str, id: &str) -> Result<Result<(), String>, sqlx::Error> {
    if agents_of(pool, pid, id).await?.is_none() {
        return Ok(Err("找不到這組編制".into()));
    }
    set_state(pool, pid, PENDING_KEY, Some(id)).await?;
    Ok(Ok(()))
}

pub async fn cancel_switch(pool: &SqlitePool, pid: &str) -> Result<(), sqlx::Error> {
    set_state(pool, pid, PENDING_KEY, None).await
}

/// 目前桌布上的團隊。
pub async fn snapshot(pool: &SqlitePool, pid: &str) -> Result<Vec<FormationAgent>, sqlx::Error> {
    Ok(bs::list_agents(pool, pid).await?.into_iter().map(|a| FormationAgent {
        kind: a.kind, name: a.name, role: a.role, prompt_prefix: a.prompt_prefix, model: a.model.unwrap_or_default(), cli: a.cli,
        produces_spec: a.produces_spec, dx: a.canvas_x, dy: a.canvas_y, db_skill: a.db_skill, custom_skills: a.custom_skills, systems: a.systems,
    }).collect())
}

/// 還在工作的 Agent：有執行中的卡片，或排隊中的品管流程卡（審查、修正、合併、驗收、拆解）——一張模組還沒走完。
/// 暫停中的 Agent 不算：它的佇列不會自己動，等它會永遠等不到。
pub async fn busy_agents(pool: &SqlitePool, pid: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT a.name FROM board_agents a WHERE a.project_id = ? AND EXISTS (
            SELECT 1 FROM board_cards c WHERE c.agent_id = a.id AND c.archived_at IS NULL AND (
                c.status = 'running'
                OR (c.status = 'queued' AND c.card_kind IN ('review', 'fix', 'merge', 'verify', 'plan') AND a.paused_card_id IS NULL)
            )
        ) ORDER BY a.created_at, a.id",
    )
    .bind(pid)
    .fetch_all(pool)
    .await
}

pub async fn retired(pool: &SqlitePool, pid: &str) -> Result<Vec<RetiredAgent>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, kind FROM board_retired_agents WHERE project_id = ?").bind(pid).fetch_all(pool).await
}

async fn build(pool: &SqlitePool, pid: &str, a: &FormationAgent, by: &str) -> Result<(), String> {
    let e = |e: sqlx::Error| e.to_string();
    let created = bs::create_agent(
        pool,
        NewAgent { project_id: pid, kind: &a.kind, name: &a.name, role: &a.role, prompt_prefix: &a.prompt_prefix, canvas_x: a.dx, canvas_y: a.dy, produces_spec: a.produces_spec && a.kind == "coder", model: Some(&a.model), cli: &a.cli, created_by: by },
    )
    .await
    .map_err(e)??;
    sqlx::query("UPDATE board_agents SET db_skill = ?, custom_skills = ?, systems = ? WHERE id = ?").bind(&a.db_skill).bind(&a.custom_skills).bind(&a.systems).bind(&created.id).execute(pool).await.map_err(e)?;
    Ok(())
}

/// 移除桌布上的一位 Agent：排隊中的審查／合併卡直接刪掉、其餘排隊中的模組退回計畫中，並記下它的名稱（歷史顯示用）。
async fn retire_agent(pool: &SqlitePool, pid: &str, a: &bs::BoardAgent) -> Result<(), String> {
    let e = |e: sqlx::Error| e.to_string();
    sqlx::query("DELETE FROM board_cards WHERE agent_id = ? AND status = 'queued' AND card_kind IN ('review', 'merge')").bind(&a.id).execute(pool).await.map_err(e)?;
    sqlx::query("INSERT OR REPLACE INTO board_retired_agents (project_id, id, name, kind, retired_at) VALUES (?,?,?,?,?)").bind(pid).bind(&a.id).bind(&a.name).bind(&a.kind).bind(now()).execute(pool).await.map_err(e)?;
    match bs::delete_agent(pool, &a.id).await.map_err(e)? {
        Ok(_) => Ok(()),
        Err(why) => Err(why),
    }
}

/// 切換到 `target`：先把目前桌布存回目前的編制，移除全部 Agent（排隊中的模組退回計畫中），再依目標建立。有人在工作就不動，回 `Busy`。
pub async fn switch_to(pool: &SqlitePool, pid: &str, target: &str, by: &str) -> Result<Result<SwitchOutcome, String>, sqlx::Error> {
    let Some(target_agents) = agents_of(pool, pid, target).await? else { return Ok(Err("找不到這組編制".into())) };
    let from = active(pool, pid).await?;
    if from.as_deref() == Some(target) {
        set_state(pool, pid, PENDING_KEY, None).await?;
        return Ok(Ok(SwitchOutcome::Switched));
    }
    let busy = busy_agents(pool, pid).await?;
    if !busy.is_empty() {
        return Ok(Ok(SwitchOutcome::Busy { names: busy }));
    }
    // 還有執行中的卡（例如暫停中的 Agent 卡在 running）：一位都不動。
    for a in bs::list_agents(pool, pid).await? {
        if bs::agent_busy(pool, &a.id).await? {
            return Ok(Ok(SwitchOutcome::Busy { names: vec![a.name] }));
        }
    }
    let from_name = match &from {
        Some(f) => {
            save_agents(pool, pid, f, &snapshot(pool, pid).await?).await?;
            name_of(pool, pid, f).await?
        }
        None => None,
    };
    for a in bs::list_agents(pool, pid).await? {
        if let Err(why) = retire_agent(pool, pid, &a).await {
            return Ok(Err(why));
        }
    }
    for a in &target_agents {
        if let Err(why) = build(pool, pid, a, by).await {
            return Ok(Err(why));
        }
    }
    set_state(pool, pid, ACTIVE_KEY, Some(target)).await?;
    set_state(pool, pid, PENDING_KEY, None).await?;
    let to_name = name_of(pool, pid, target).await?.unwrap_or_default();
    sqlx::query("INSERT INTO board_formation_log (project_id, at, from_name, to_name, by) VALUES (?,?,?,?,?)").bind(pid).bind(now()).bind(from_name).bind(to_name).bind(by).execute(pool).await?;
    Ok(Ok(SwitchOutcome::Switched))
}

/// 把目前桌布另存成新編制並設為目前（舊的那組先存下最新的桌布，之後的變動算在新的）。
pub async fn save_as(pool: &SqlitePool, pid: &str, name: &str, by: &str) -> Result<Result<String, String>, sqlx::Error> {
    let snap = snapshot(pool, pid).await?;
    let id = match create(pool, pid, name, &snap, by).await? {
        Ok(id) => id,
        Err(why) => return Ok(Err(why)),
    };
    if let Some(f) = active(pool, pid).await? {
        save_agents(pool, pid, &f, &snap).await?;
    }
    set_state(pool, pid, ACTIVE_KEY, Some(&id)).await?;
    Ok(Ok(id))
}

/// 有等待中的切換而且沒人在工作了，就切換。回傳有沒有切換。
pub async fn try_complete_pending(pool: &SqlitePool, pid: &str) -> Result<bool, String> {
    let e = |e: sqlx::Error| e.to_string();
    let Some(target) = pending(pool, pid).await.map_err(e)? else { return Ok(false) };
    match switch_to(pool, pid, &target, "系統").await.map_err(e)?? {
        SwitchOutcome::Switched => Ok(true),
        SwitchOutcome::Busy { .. } => Ok(false),
    }
}

/// 有等待中切換的專案（排程每一輪掃一次）。
pub async fn projects_with_pending(pool: &SqlitePool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT project_id FROM board_formation_state WHERE key = ?").bind(PENDING_KEY).fetch_all(pool).await
}

/// 舊專案：沒有任何編制但桌布上有 Agent → 用目前的團隊建立一組並設為目前。
pub async fn ensure_default(pool: &SqlitePool, pid: &str, name: &str) -> Result<(), sqlx::Error> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_formations WHERE project_id = ?").bind(pid).fetch_one(pool).await?;
    if n > 0 || bs::list_agents(pool, pid).await?.is_empty() {
        return Ok(());
    }
    let snap = snapshot(pool, pid).await?;
    if let Ok(id) = create(pool, pid, if name.trim().is_empty() { "預設" } else { name }, &snap, "").await? {
        set_state(pool, pid, ACTIVE_KEY, Some(&id)).await?;
    }
    Ok(())
}

pub async fn state(pool: &SqlitePool, pid: &str) -> Result<FormationsState, sqlx::Error> {
    Ok(FormationsState {
        formations: list(pool, pid).await?,
        active: active(pool, pid).await?,
        pending: pending(pool, pid).await?,
        busy: busy_agents(pool, pid).await?,
        log: sqlx::query_as("SELECT at, from_name, to_name FROM board_formation_log WHERE project_id = ? ORDER BY at DESC, rowid DESC LIMIT 20").bind(pid).fetch_all(pool).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board_store::{NewCard, RUNNING};

    const P: &str = "P";

    async fn pool() -> SqlitePool {
        let p = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        bs::init_schema(&p).await.unwrap();
        p
    }

    fn fa(kind: &str, name: &str) -> FormationAgent {
        FormationAgent { kind: kind.into(), name: name.into(), role: format!("{name}的角色"), prompt_prefix: format!("你是{name}"), model: String::new(), cli: "claude".into(), produces_spec: false, dx: 40.0, dy: 40.0, db_skill: None, custom_skills: None, systems: None }
    }

    async fn active_with(p: &SqlitePool, name: &str, agents: &[FormationAgent]) -> String {
        let id = create(p, P, name, agents, "小華").await.unwrap().unwrap();
        assert_eq!(switch_to(p, P, &id, "小華").await.unwrap().unwrap(), SwitchOutcome::Switched);
        id
    }

    #[tokio::test]
    async fn create_list_rename_delete_and_cannot_delete_the_active_one() {
        let p = pool().await;
        let a = create(&p, P, "盤點", &[fa("coder", "工程師1"), fa("reviewer", "審核者")], "x").await.unwrap().unwrap();
        let b = create(&p, P, "轉換", &[], "x").await.unwrap().unwrap();
        assert_eq!(list(&p, P).await.unwrap().iter().map(|f| (f.name.as_str(), f.agent_count)).collect::<Vec<_>>(), vec![("盤點", 2), ("轉換", 0)]);
        assert!(create(&p, P, "  ", &[], "x").await.unwrap().is_err());
        rename(&p, P, &b, " 實測 ").await.unwrap().unwrap();
        assert_eq!(list(&p, P).await.unwrap()[1].name, "實測");
        assert!(rename(&p, "別的專案", &b, "x").await.unwrap().is_err(), "別的專案的編制不能動");
        switch_to(&p, P, &a, "x").await.unwrap().unwrap();
        assert!(delete(&p, P, &a).await.unwrap().is_err());
        delete(&p, P, &b).await.unwrap().unwrap();
        assert_eq!(list(&p, P).await.unwrap().len(), 1);
        assert!(list(&p, "別的專案").await.unwrap().is_empty(), "編制屬於專案");
    }

    #[tokio::test]
    async fn switching_rebuilds_the_team_keeps_cards_saves_the_old_desk_and_records_retired_agents() {
        let p = pool().await;
        let mut c = fa("coder", "工程師1");
        c.model = "sonnet".into();
        c.systems = Some("[\"s1\"]".into());
        c.db_skill = Some("{\"connections\":[],\"mode\":\"read\"}".into());
        let a = active_with(&p, "盤點", &[c.clone(), fa("reviewer", "審核者")]).await;
        let old: Vec<String> = bs::list_agents(&p, P).await.unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(old.len(), 2);
        // 同一秒建立的 Agent 排序不固定：用名稱找。
        let agent = bs::list_agents(&p, P).await.unwrap().into_iter().find(|x| x.name == "工程師1").unwrap();
        assert_eq!((agent.model.as_deref(), agent.systems.as_deref()), (Some("sonnet"), Some("[\"s1\"]")), "技能與設定都會重建");
        // 桌布上手動挪了位置：切走時存回這組。
        sqlx::query("UPDATE board_agents SET canvas_x = 500 WHERE id = ?").bind(&agent.id).execute(&p).await.unwrap();
        // 專案裡有一張計畫中的卡，切換後還在。
        let card = bs::create_card(&p, NewCard { project_id: P, title: "要做的事", body: "", parallel_ok: true, interactive: false, label: None, created_by: "x" }).await.unwrap();
        let b = create(&p, P, "轉換", &[fa("coder", "轉換工程師")], "x").await.unwrap().unwrap();
        assert_eq!(switch_to(&p, P, &b, "小華").await.unwrap().unwrap(), SwitchOutcome::Switched);
        let now_agents = bs::list_agents(&p, P).await.unwrap();
        assert_eq!(now_agents.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["轉換工程師"]);
        assert!(!old.contains(&now_agents[0].id), "是重建的新 Agent");
        assert!(bs::get_card(&p, &card.id).await.unwrap().is_some(), "卡片保留");
        assert_eq!(active(&p, P).await.unwrap().as_deref(), Some(b.as_str()));
        let mut r: Vec<String> = retired(&p, P).await.unwrap().into_iter().map(|x| x.name).collect();
        r.sort();
        assert_eq!(r, vec!["審核者", "工程師1"]);
        // 切回去：位置是切走前存下的。
        switch_to(&p, P, &a, "小華").await.unwrap().unwrap();
        let back = bs::list_agents(&p, P).await.unwrap();
        assert_eq!(back.iter().find(|x| x.name == "工程師1").unwrap().canvas_x, 500.0);
        let log = state(&p, P).await.unwrap().log;
        assert_eq!(log.iter().map(|l| (l.from_name.as_deref(), l.to_name.as_str())).collect::<Vec<_>>(), vec![(Some("轉換"), "盤點"), (Some("盤點"), "轉換"), (None, "盤點")]);
    }

    #[tokio::test]
    async fn a_running_card_blocks_the_switch_and_pending_completes_it_later() {
        let p = pool().await;
        let a = active_with(&p, "盤點", &[fa("coder", "工程師1")]).await;
        let b = create(&p, P, "轉換", &[fa("coder", "轉換工程師")], "x").await.unwrap().unwrap();
        let agent = bs::list_agents(&p, P).await.unwrap().remove(0);
        let card = bs::create_card(&p, NewCard { project_id: P, title: "工作", body: "", parallel_ok: true, interactive: false, label: None, created_by: "x" }).await.unwrap();
        sqlx::query("UPDATE board_cards SET agent_id = ?, status = ? WHERE id = ?").bind(&agent.id).bind(RUNNING).bind(&card.id).execute(&p).await.unwrap();
        assert_eq!(switch_to(&p, P, &b, "x").await.unwrap().unwrap(), SwitchOutcome::Busy { names: vec!["工程師1".into()] });
        assert_eq!(bs::list_agents(&p, P).await.unwrap().len(), 1, "被擋時一位都不動");
        assert_eq!(active(&p, P).await.unwrap().as_deref(), Some(a.as_str()));
        request_switch(&p, P, &b).await.unwrap().unwrap();
        assert_eq!(projects_with_pending(&p).await.unwrap(), vec![P.to_string()]);
        assert!(!try_complete_pending(&p, P).await.unwrap(), "還在工作就不換");
        // 卡做完（已完成的卡不擋）。
        sqlx::query("UPDATE board_cards SET status = 'done' WHERE id = ?").bind(&card.id).execute(&p).await.unwrap();
        assert!(try_complete_pending(&p, P).await.unwrap());
        assert_eq!(active(&p, P).await.unwrap().as_deref(), Some(b.as_str()));
        assert_eq!(pending(&p, P).await.unwrap(), None);
        assert!(projects_with_pending(&p).await.unwrap().is_empty());
        assert_eq!(bs::get_card(&p, &card.id).await.unwrap().unwrap().status, "done", "已完成的卡不被動到");
    }

    #[tokio::test]
    async fn a_queued_review_card_of_an_unpaused_agent_blocks_but_a_paused_one_does_not() {
        let p = pool().await;
        active_with(&p, "盤點", &[fa("reviewer", "審核者")]).await;
        let b = create(&p, P, "轉換", &[], "x").await.unwrap().unwrap();
        let agent = bs::list_agents(&p, P).await.unwrap().remove(0);
        let card = bs::create_card(&p, NewCard { project_id: P, title: "審查", body: "", parallel_ok: true, interactive: false, label: None, created_by: "x" }).await.unwrap();
        sqlx::query("UPDATE board_cards SET agent_id = ?, status = 'queued', card_kind = 'review' WHERE id = ?").bind(&agent.id).bind(&card.id).execute(&p).await.unwrap();
        assert!(matches!(switch_to(&p, P, &b, "x").await.unwrap().unwrap(), SwitchOutcome::Busy { .. }));
        sqlx::query("UPDATE board_agents SET paused_card_id = 'x' WHERE id = ?").bind(&agent.id).execute(&p).await.unwrap();
        assert_eq!(switch_to(&p, P, &b, "x").await.unwrap().unwrap(), SwitchOutcome::Switched);
        assert!(bs::get_card(&p, &card.id).await.unwrap().is_none(), "排隊中的審查卡直接刪掉");
    }

    #[tokio::test]
    async fn save_as_and_ensure_default_use_the_current_desk() {
        let p = pool().await;
        ensure_default(&p, P, "預設").await.unwrap();
        assert!(list(&p, P).await.unwrap().is_empty(), "桌布上沒有 Agent 就不建");
        bs::create_agent(&p, NewAgent { project_id: P, kind: "coder", name: "工程師1", role: "", prompt_prefix: "", canvas_x: 1.0, canvas_y: 2.0, produces_spec: false, model: None, cli: "claude", created_by: "x" }).await.unwrap().unwrap();
        ensure_default(&p, P, "預設").await.unwrap();
        ensure_default(&p, P, "預設").await.unwrap();
        let l = list(&p, P).await.unwrap();
        assert_eq!((l.len(), l[0].name.as_str(), l[0].agent_count), (1, "預設", 1));
        assert_eq!(active(&p, P).await.unwrap().as_deref(), Some(l[0].id.as_str()));
        let id = save_as(&p, P, "另存", "x").await.unwrap().unwrap();
        assert_eq!(active(&p, P).await.unwrap().as_deref(), Some(id.as_str()));
        assert_eq!(list(&p, P).await.unwrap().len(), 2);
        assert!(save_as(&p, P, "", "x").await.unwrap().is_err());
    }
}

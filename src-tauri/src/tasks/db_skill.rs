//! Agent 的「資料庫」技能：設定、權限判定與查詢記錄。
//! 見 `docs/superpowers/specs/2026-10-19-agent-db-skill-design.md`。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::tasks::sql_guard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbMode {
    /// 只能跑唯讀查詢。
    Read,
    /// 寫入要使用者每次核准。
    Approve,
    /// 寫入直接放行。
    Write,
}

/// 存在 `agents.db_skill` 的 JSON。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbSkill {
    pub connections: Vec<String>,
    pub mode: DbMode,
}

/// `agents.db_skill` 欄位的值；沒裝、壞掉或沒有任何連線都是 `None`。
pub fn parse(raw: Option<&str>) -> Option<DbSkill> {
    raw.and_then(|r| serde_json::from_str::<DbSkill>(r).ok()).filter(|s| !s.connections.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// 要使用者核准才執行。
    NeedApproval,
    Deny(String),
}

/// 這位 Agent 能不能在這個連線跑這句 SQL。
pub fn authorize(skill: &DbSkill, connection_id: &str, sql: &str) -> Decision {
    if !skill.connections.iter().any(|c| c == connection_id) {
        return Decision::Deny("這個連線沒有開放給你，先用 db_list_connections 看可以用哪些。".into());
    }
    if sql_guard::is_read_only(sql) {
        return Decision::Allow;
    }
    match skill.mode {
        DbMode::Read => Decision::Deny(
            "這個連線只開放唯讀查詢（SELECT／WITH／SHOW／DESCRIBE／EXPLAIN，一次一句）。需要寫入請在待確認事項說明。".into(),
        ),
        DbMode::Approve => Decision::NeedApproval,
        DbMode::Write => Decision::Allow,
    }
}

/// 卡片提示詞後面附的一段：告訴 Agent 有哪些連線可以用、權限是什麼。
/// `names`：連線 id → 名稱（找不到名稱就顯示 id）。
pub fn prompt_note(skill: &DbSkill, names: &[(String, String)]) -> String {
    let list: Vec<String> = skill
        .connections
        .iter()
        .map(|id| {
            let name = names.iter().find(|(i, _)| i == id).map_or(id.as_str(), |(_, n)| n.as_str());
            format!("{name}（id：{id}）")
        })
        .collect();
    let rule = match skill.mode {
        DbMode::Read => "只能執行唯讀查詢",
        DbMode::Approve => "唯讀查詢直接執行；寫入要等使用者核准，被拒絕就不要硬做",
        DbMode::Write => "可以讀寫，寫入前先確認會影響哪些資料",
    };
    format!(
        "\n\n資料庫：你可以用 db_list_connections、db_list_tables、db_table_schema、db_query 工具存取這些連線：{}。{rule}。\
         帳密由 AITerm 管理，不要在程式碼或檔案裡寫死連線資訊。",
        list.join("、")
    )
}

pub async fn set_agent_skill(pool: &SqlitePool, agent_id: &str, skill: Option<&DbSkill>) -> Result<(), sqlx::Error> {
    let raw = skill.filter(|s| !s.connections.is_empty()).map(|s| serde_json::to_string(s).unwrap_or_default());
    sqlx::query("UPDATE agents SET db_skill = ? WHERE id = ?").bind(raw).bind(agent_id).execute(pool).await?;
    Ok(())
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let _ = sqlx::query("ALTER TABLE agents ADD COLUMN db_skill TEXT").execute(pool).await;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS db_queries (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id         TEXT NOT NULL,
            agent_id        TEXT,
            connection_id   TEXT NOT NULL,
            connection_name TEXT NOT NULL,
            sql             TEXT NOT NULL,
            started_at      INTEGER NOT NULL,
            finished_at     INTEGER,
            rows            INTEGER,
            ms              INTEGER,
            error           TEXT,
            decision        TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_db_queries_started ON db_queries(started_at)").execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow, PartialEq)]
pub struct DbQueryRow {
    pub id: i64,
    pub task_id: String,
    pub agent_id: Option<String>,
    pub connection_id: String,
    pub connection_name: String,
    pub sql: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub rows: Option<i64>,
    pub ms: Option<i64>,
    pub error: Option<String>,
    /// `allowed`／`pending`（等核准）／`approved`／`denied`。
    pub decision: String,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[derive(Clone, Copy)]
pub struct NewQuery<'a> {
    pub task_id: &'a str,
    pub agent_id: Option<&'a str>,
    pub connection_id: &'a str,
    pub connection_name: &'a str,
    pub sql: &'a str,
    pub decision: &'a str,
}

/// 記一筆開始執行的查詢；回傳 id，結束時用 `finish_query` 補上結果。
pub async fn start_query(pool: &SqlitePool, q: &NewQuery<'_>) -> Result<i64, sqlx::Error> {
    let id = sqlx::query(
        "INSERT INTO db_queries (task_id, agent_id, connection_id, connection_name, sql, started_at, decision)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(q.task_id)
    .bind(q.agent_id)
    .bind(q.connection_id)
    .bind(q.connection_name)
    .bind(q.sql)
    .bind(now())
    .bind(q.decision)
    .execute(pool)
    .await?
    .last_insert_rowid();
    Ok(id)
}

pub async fn finish_query(
    pool: &SqlitePool,
    id: i64,
    rows: Option<i64>,
    ms: i64,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE db_queries SET finished_at = ?, rows = ?, ms = ?, error = ? WHERE id = ?")
        .bind(now())
        .bind(rows)
        .bind(ms)
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 等核准的查詢有了結果（`approved`／`denied`）。
pub async fn set_decision(pool: &SqlitePool, id: i64, decision: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE db_queries SET decision = ? WHERE id = ?").bind(decision).bind(id).execute(pool).await?;
    Ok(())
}

/// 被擋下或被拒絕的查詢：一次記完（沒有執行）。
pub async fn record_denied(pool: &SqlitePool, q: &NewQuery<'_>, reason: &str) -> Result<(), sqlx::Error> {
    let id = start_query(pool, &NewQuery { decision: "denied", ..*q }).await?;
    finish_query(pool, id, None, 0, Some(reason)).await
}

/// 最近的查詢（新的在前）。
pub async fn recent_queries(pool: &SqlitePool, limit: i64) -> Result<Vec<DbQueryRow>, sqlx::Error> {
    sqlx::query_as::<_, DbQueryRow>("SELECT * FROM db_queries ORDER BY started_at DESC, id DESC LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// App 重啟時還沒結束的查詢已經不會結束了：標成中斷，畫面才不會一直顯示「使用中」。
pub async fn close_dangling(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE db_queries SET finished_at = started_at, error = 'AITerm 重新啟動，查詢中斷' WHERE finished_at IS NULL")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(mode: DbMode) -> DbSkill {
        DbSkill { connections: vec!["dev".into()], mode }
    }

    #[test]
    fn parse_ignores_missing_broken_and_empty() {
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("not json")), None);
        assert_eq!(parse(Some(r#"{"connections":[],"mode":"read"}"#)), None);
        assert_eq!(parse(Some(r#"{"connections":["dev"],"mode":"approve"}"#)), Some(skill(DbMode::Approve)));
    }

    #[test]
    fn authorization_by_connection_and_mode() {
        let read = "SELECT * FROM t";
        let write = "DELETE FROM t";
        for mode in [DbMode::Read, DbMode::Approve, DbMode::Write] {
            assert_eq!(authorize(&skill(mode), "dev", read), Decision::Allow, "{mode:?} 唯讀一律放行");
            assert!(matches!(authorize(&skill(mode), "prod", read), Decision::Deny(_)), "{mode:?} 沒開放的連線");
        }
        assert!(matches!(authorize(&skill(DbMode::Read), "dev", write), Decision::Deny(m) if m.contains("唯讀")));
        assert_eq!(authorize(&skill(DbMode::Approve), "dev", write), Decision::NeedApproval);
        assert_eq!(authorize(&skill(DbMode::Write), "dev", write), Decision::Allow);
    }

    #[test]
    fn the_prompt_note_names_connections_and_the_rule() {
        let note = prompt_note(&DbSkill { connections: vec!["dev".into(), "x".into()], mode: DbMode::Read }, &[("dev".into(), "HTBKTW-DEV".into())]);
        assert!(note.contains("HTBKTW-DEV（id：dev）") && note.contains("x（id：x）"), "{note}");
        assert!(note.contains("唯讀") && note.contains("db_query"), "{note}");
        assert!(prompt_note(&skill(DbMode::Approve), &[]).contains("核准"));
    }

    #[tokio::test]
    async fn queries_are_logged_and_dangling_ones_closed() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        let q = NewQuery { task_id: "t1", agent_id: Some("a1"), connection_id: "dev", connection_name: "Dev", sql: "SELECT 1", decision: "allowed" };
        let id = start_query(&pool, &q).await.unwrap();
        let running = start_query(&pool, &NewQuery { sql: "SELECT 2", ..q }).await.unwrap();
        finish_query(&pool, id, Some(12), 300, None).await.unwrap();
        record_denied(&pool, &NewQuery { sql: "DROP TABLE t", ..q }, "唯讀").await.unwrap();

        let rows = recent_queries(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 3);
        let done = rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!((done.rows, done.ms, done.decision.as_str()), (Some(12), Some(300), "allowed"));
        assert!(rows.iter().find(|r| r.id == running).unwrap().finished_at.is_none(), "還在跑");
        let denied = rows.iter().find(|r| r.sql == "DROP TABLE t").unwrap();
        assert_eq!((denied.decision.as_str(), denied.error.as_deref()), ("denied", Some("唯讀")));

        close_dangling(&pool).await.unwrap();
        assert!(recent_queries(&pool, 10).await.unwrap().iter().all(|r| r.finished_at.is_some()));

        set_agent_skill(&pool, "nobody", Some(&skill(DbMode::Read))).await.unwrap();
    }
}

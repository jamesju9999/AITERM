//! Agent 的「資料庫」技能：設定、權限判定與查詢記錄。
//! 見 `docs/superpowers/specs/2026-10-19-agent-db-skill-design.md`。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

// 設定格式、權限判定與提示詞搬到共用的 aiterm-db（server 也用）；這裡 re-export，原本的路徑不變。
pub use aiterm_db::skill::{authorize, parse, prompt_note, DbMode, DbSkill, Decision};

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
    // 等核准的那句沒有執行過：記成被拒絕，不是「查詢失敗」。
    sqlx::query(
        "UPDATE db_queries SET finished_at = started_at, error = 'AITerm 重新啟動，查詢中斷',
             decision = CASE WHEN decision = 'pending' THEN 'denied' ELSE decision END
         WHERE finished_at IS NULL",
    )
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let waiting = start_query(&pool, &NewQuery { sql: "UPDATE t", decision: "pending", ..q }).await.unwrap();
        close_dangling(&pool).await.unwrap();
        let rows = recent_queries(&pool, 10).await.unwrap();
        assert!(rows.iter().all(|r| r.finished_at.is_some()));
        assert_eq!(rows.iter().find(|r| r.id == waiting).unwrap().decision, "denied", "沒執行過的等核准記成被拒絕");
        assert_eq!(rows.iter().find(|r| r.id == running).unwrap().decision, "allowed");

        set_agent_skill(&pool, "nobody", Some(&DbSkill { connections: vec!["dev".into()], mode: DbMode::Read })).await.unwrap();
    }
}

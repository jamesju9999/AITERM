//! 系統連線的審計記錄（`systems.db`）：每個會動作的工具呼叫與每次閘門判定一筆。

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Deny,
    Approved,
    Rejected,
    Timeout,
    /// 使用者接手期間的操作，沒有經過閘門判定。
    Manual,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Deny => "deny",
            Verdict::Approved => "approved",
            Verdict::Rejected => "rejected",
            Verdict::Timeout => "timeout",
            Verdict::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub system_id: String,
    pub tool: String,
    pub intent: String,
    pub method: Option<String>,
    pub url: Option<String>,
    pub verdict: Verdict,
    /// 判定來源：`rule`（模式／規則自動判定）或 `user`（使用者按的）。
    pub source: String,
    pub screenshot: Option<String>,
    pub result: Option<String>,
    pub ts_ms: i64,
}

impl Entry {
    pub fn new(
        system_id: &str,
        tool: &str,
        intent: &str,
        method: Option<&str>,
        url: Option<&str>,
        verdict: Verdict,
        source: &str,
    ) -> Self {
        Self {
            system_id: system_id.into(),
            tool: tool.into(),
            intent: intent.into(),
            method: method.map(Into::into),
            url: url.map(Into::into),
            verdict,
            source: source.into(),
            screenshot: None,
            result: None,
            ts_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Row {
    pub id: i64,
    pub system_id: String,
    pub tool: String,
    pub intent: String,
    pub method: Option<String>,
    pub url: Option<String>,
    pub verdict: String,
    pub source: String,
    pub screenshot: Option<String>,
    pub result: Option<String>,
    pub ts_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS system_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    system_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    intent TEXT NOT NULL,
    method TEXT,
    url TEXT,
    verdict TEXT NOT NULL,
    source TEXT NOT NULL,
    screenshot TEXT,
    result TEXT,
    ts_ms INTEGER NOT NULL
)";

async fn init(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(SCHEMA).execute(pool).await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_system_audit_sys_ts ON system_audit(system_id, ts_ms)")
        .execute(pool)
        .await?;
    Ok(())
}

/// 應用層的 `systems.db`（跟 knowledge_base.db 同一個資料夾）。
pub async fn open() -> Result<SqlitePool, sqlx::Error> {
    let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM");
    std::fs::create_dir_all(&dir).ok();
    let options = SqliteConnectOptions::new()
        .filename(dir.join("systems.db"))
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));
    let pool = SqlitePool::connect_with(options).await?;
    init(&pool).await?;
    super::recipes::init(&pool).await?;
    Ok(pool)
}

pub async fn record(pool: &SqlitePool, e: &Entry) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO system_audit (system_id, tool, intent, method, url, verdict, source, screenshot, result, ts_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&e.system_id)
    .bind(&e.tool)
    .bind(&e.intent)
    .bind(&e.method)
    .bind(&e.url)
    .bind(e.verdict.as_str())
    .bind(&e.source)
    .bind(&e.screenshot)
    .bind(&e.result)
    .bind(e.ts_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// 某系統最近的記錄，新的在前。
pub async fn list(pool: &SqlitePool, system_id: &str, limit: i64) -> Result<Vec<Row>, sqlx::Error> {
    sqlx::query_as::<_, Row>(
        "SELECT id, system_id, tool, intent, method, url, verdict, source, screenshot, result, ts_ms
         FROM system_audit WHERE system_id = ? ORDER BY ts_ms DESC, id DESC LIMIT ?",
    )
    .bind(system_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn memory() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        init(&pool).await.unwrap();
        pool
    }

    fn at(mut e: Entry, ts: i64) -> Entry {
        e.ts_ms = ts;
        e
    }

    #[tokio::test]
    async fn records_are_listed_newest_first_per_system() {
        let pool = memory().await;
        record(&pool, &at(Entry::new("a", "sys_click", "點儲存", Some("POST"), Some("https://x/y"), Verdict::Approved, "user"), 1)).await.unwrap();
        record(&pool, &at(Entry::new("b", "sys_click", "別的系統", None, None, Verdict::Allow, "rule"), 2)).await.unwrap();
        record(&pool, &at(Entry::new("a", "sys_navigate", "開首頁", Some("GET"), Some("https://x/"), Verdict::Allow, "rule"), 3)).await.unwrap();

        let rows = list(&pool, "a", 10).await.unwrap();
        assert_eq!(rows.len(), 2, "別的系統的不能混進來");
        assert_eq!(rows[0].tool, "sys_navigate", "最新的在前");
        assert_eq!(rows[1].verdict, "approved");
        assert_eq!(rows[1].source, "user");
    }

    #[tokio::test]
    async fn limit_caps_the_result() {
        let pool = memory().await;
        for i in 0..5 {
            record(&pool, &at(Entry::new("a", "t", "i", None, None, Verdict::Deny, "rule"), i)).await.unwrap();
        }
        let rows = list(&pool, "a", 3).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].ts_ms, 4);
    }
}

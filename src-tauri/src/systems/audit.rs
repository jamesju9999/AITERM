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
    super::jobs::init(&pool).await?;
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

/// 刪掉 `cutoff_ms` 之前的記錄，回傳刪了幾筆。
pub async fn prune(pool: &SqlitePool, cutoff_ms: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM system_audit WHERE ts_ms < ?").bind(cutoff_ms).execute(pool).await?.rows_affected())
}

/// 刪掉資料夾（含子資料夾）裡最後修改時間早於 `cutoff` 的檔案，回傳刪了幾個。資料夾本身不動。
pub fn prune_files(dir: &std::path::Path, cutoff: std::time::SystemTime) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let p = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() => n += prune_files(&p, cutoff),
            Ok(t) if t.is_file() => {
                if e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < cutoff) && std::fs::remove_file(&p).is_ok() {
                    n += 1;
                }
            }
            _ => {}
        }
    }
    n
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

/// 測試用：全新的記憶體資料庫，建好 systems.db 的所有表。
#[cfg(test)]
pub async fn open_in_memory() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    init(&pool).await.unwrap();
    super::recipes::init(&pool).await.unwrap();
    super::jobs::init(&pool).await.unwrap();
    pool
}

/// 某系統在 `since_ms` 之後被拒絕、被你拒絕、或等核准逾時的次數（定時任務用它判斷「寫入沒成功」）。
pub async fn count_blocked_since(pool: &SqlitePool, system_id: &str, since_ms: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_audit WHERE system_id = ? AND ts_ms >= ? AND verdict IN ('deny', 'rejected', 'timeout')",
    )
    .bind(system_id)
    .bind(since_ms)
    .fetch_one(pool)
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
    async fn pruning_removes_only_records_older_than_the_cutoff() {
        let pool = memory().await;
        for ts in [5, 99, 100, 101] {
            record(&pool, &at(Entry::new("a", "gate", "i", None, None, Verdict::Allow, "rule"), ts)).await.unwrap();
        }
        assert_eq!(prune(&pool, 100).await.unwrap(), 2);
        let left: Vec<i64> = list(&pool, "a", 10).await.unwrap().iter().map(|r| r.ts_ms).collect();
        assert_eq!(left, vec![101, 100], "剛好等於截止時間的保留");
    }

    #[test]
    fn old_files_are_pruned_recursively_but_folders_and_new_files_stay() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sys");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("old.yml"), "x").unwrap();
        std::fs::write(dir.path().join("also-old.log"), "x").unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        assert_eq!(prune_files(dir.path(), past), 0, "都比截止時間新，一個都不刪");
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        assert_eq!(prune_files(dir.path(), future), 2);
        assert!(sub.is_dir() && !sub.join("old.yml").exists());
    }

    #[tokio::test]
    async fn blocked_writes_since_a_time_are_counted_per_system() {
        let pool = memory().await;
        for (sys, v, ts) in [("a", Verdict::Deny, 10), ("a", Verdict::Rejected, 20), ("a", Verdict::Timeout, 30), ("a", Verdict::Approved, 40), ("a", Verdict::Allow, 50), ("a", Verdict::Deny, 5), ("b", Verdict::Deny, 25)] {
            record(&pool, &at(Entry::new(sys, "gate", "i", Some("POST"), Some("u"), v, "rule"), ts)).await.unwrap();
        }
        assert_eq!(count_blocked_since(&pool, "a", 10).await.unwrap(), 3, "deny、rejected、timeout 算；approved、allow 不算；更早的不算");
        assert_eq!(count_blocked_since(&pool, "a", 31).await.unwrap(), 0);
        assert_eq!(count_blocked_since(&pool, "b", 0).await.unwrap(), 1);
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

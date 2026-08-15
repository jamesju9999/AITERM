//! 用量紀錄的 SQLite 落地。
//!
//! 路徑與初始化方式比照 `crate::db::loop_sessions`：明確 `create_if_missing`，
//! 失敗才退回 in-memory —— 記帳失效不得影響 AI 主流程。

use crate::ai::TokenUsage;
use serde::Serialize;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{FromRow, SqlitePool};
use std::path::{Path, PathBuf};

/// 保留天數。更舊的紀錄在啟動時清除。
const RETENTION_DAYS: i64 = 90;

pub struct UsageStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct UsageSummaryRow {
    pub provider_id: String,
    pub model: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

/// 前端可選的統計區間。
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageRange {
    Today,
    Days7,
    Days30,
}

impl UsageRange {
    /// 這個區間的起始 Unix 秒。`Today` 以 UTC 日界計算。
    pub fn cutoff(&self, now: i64) -> i64 {
        match self {
            UsageRange::Today => now - now.rem_euclid(86_400),
            UsageRange::Days7 => now - 7 * 86_400,
            UsageRange::Days30 => now - 30 * 86_400,
        }
    }
}

impl UsageStore {
    /// 正式路徑：`{data_dir}/AITERM/usage.db`。
    pub async fn new() -> Self {
        let dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM");
        std::fs::create_dir_all(&dir).ok();
        Self::new_at(dir.join("usage.db")).await
    }

    /// 指定路徑（測試用）。
    pub async fn new_at(path: impl AsRef<Path>) -> Self {
        let options = SqliteConnectOptions::new().filename(path.as_ref()).create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .unwrap_or_else(|_| SqlitePool::connect_lazy("sqlite::memory:").unwrap());
        let store = Self { pool };
        store.init().await.ok();
        store
    }

    async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS usage_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_tokens INTEGER NOT NULL DEFAULT 0,
                completion_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                occurred_at INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_usage_time ON usage_records (occurred_at)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_usage_provider
             ON usage_records (provider_id, occurred_at)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 寫入一筆。**寫入失敗只記 log，不回傳錯誤** —— 記帳是旁路，
    /// 不得讓 AI 請求因為記帳失敗而失敗。
    pub async fn record(&self, provider_id: &str, model: &str, usage: TokenUsage, at: i64) {
        let r = sqlx::query(
            "INSERT INTO usage_records
             (provider_id, model, prompt_tokens, completion_tokens,
              cache_read_tokens, cache_write_tokens, occurred_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(provider_id)
        .bind(model)
        .bind(usage.prompt as i64)
        .bind(usage.completion as i64)
        .bind(usage.cache_read as i64)
        .bind(usage.cache_write as i64)
        .bind(at)
        .execute(&self.pool)
        .await;
        if let Err(e) = r {
            log::warn!("寫入用量紀錄失敗（不影響 AI 流程）: {e}");
        }
    }

    /// 依 provider + model 分組彙總 `occurred_at >= since` 的紀錄。
    pub async fn summary_since(&self, since: i64) -> Result<Vec<UsageSummaryRow>, sqlx::Error> {
        sqlx::query_as::<_, UsageSummaryRow>(
            "SELECT provider_id, model,
                    COUNT(*) AS requests,
                    COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                    COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                    COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
                    COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens
             FROM usage_records
             WHERE occurred_at >= ?
             GROUP BY provider_id, model
             ORDER BY prompt_tokens + completion_tokens DESC",
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
    }

    /// 刪除早於 `cutoff` 的紀錄。啟動時呼叫一次。
    pub async fn prune_before(&self, cutoff: i64) {
        let r = sqlx::query("DELETE FROM usage_records WHERE occurred_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await;
        if let Err(e) = r {
            log::warn!("清理舊用量紀錄失敗: {e}");
        }
    }

    /// 依保留天數清理。
    pub async fn prune_expired(&self, now: i64) {
        self.prune_before(now - RETENTION_DAYS * 86_400).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每個測試各自用一個暫存檔，彼此不互相污染。
    async fn temp_store() -> (UsageStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = UsageStore::new_at(dir.path().join("usage.db")).await;
        (store, dir)
    }

    #[tokio::test]
    async fn records_and_sums_by_provider_and_model() {
        let (store, _dir) = temp_store().await;
        let now = 1_786_760_000;
        store.record("anthropic-pro", "claude-sonnet-4-5", TokenUsage {
            prompt: 100, completion: 20, cache_read: 4000, cache_write: 300,
        }, now).await;
        store.record("anthropic-pro", "claude-sonnet-4-5", TokenUsage {
            prompt: 50, completion: 10, cache_read: 0, cache_write: 0,
        }, now).await;
        store.record("GPT5.6", "gpt-5.6-luna", TokenUsage {
            prompt: 7, completion: 3, cache_read: 0, cache_write: 0,
        }, now).await;

        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows.len(), 2, "兩個 provider/model 組合");

        let anthropic = rows.iter()
            .find(|r| r.provider_id == "anthropic-pro")
            .expect("anthropic row");
        assert_eq!(anthropic.requests, 2);
        assert_eq!(anthropic.prompt_tokens, 150);
        assert_eq!(anthropic.completion_tokens, 30);
        assert_eq!(anthropic.cache_read_tokens, 4000);
        assert_eq!(anthropic.cache_write_tokens, 300);
    }

    #[tokio::test]
    async fn summary_since_excludes_older_records() {
        let (store, _dir) = temp_store().await;
        let u = TokenUsage { prompt: 1, completion: 1, cache_read: 0, cache_write: 0 };
        store.record("p", "m", u, 1_000).await;
        store.record("p", "m", u, 9_000).await;

        let rows = store.summary_since(5_000).await.expect("summary");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].requests, 1, "只算 occurred_at >= 5000 的那筆");
    }

    #[tokio::test]
    async fn zero_usage_is_still_recorded() {
        // 「上游沒回 usage」由呼叫端決定不寫；真的全 0 的請求則照實記錄。
        let (store, _dir) = temp_store().await;
        store.record("p", "m", TokenUsage::default(), 1_000).await;
        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows[0].requests, 1);
    }

    #[tokio::test]
    async fn prune_removes_records_older_than_cutoff() {
        let (store, _dir) = temp_store().await;
        let u = TokenUsage { prompt: 1, completion: 1, cache_read: 0, cache_write: 0 };
        store.record("p", "m", u, 1_000).await;
        store.record("p", "m", u, 9_000).await;
        store.prune_before(5_000).await;
        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows[0].requests, 1);
    }

    #[test]
    fn range_cutoff_boundaries() {
        // 2026-08-15 02:26:20 UTC
        let now = 1_786_760_780_i64;
        // Today 落在當天 UTC 00:00:00
        assert_eq!(UsageRange::Today.cutoff(now), 1_786_752_000);
        assert_eq!(UsageRange::Days7.cutoff(now), now - 604_800);
        assert_eq!(UsageRange::Days30.cutoff(now), now - 2_592_000);
    }
}

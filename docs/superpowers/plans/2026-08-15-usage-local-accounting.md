# 本地用量累計 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AITerm 把每次 AI 請求的 token 用量落地到 SQLite，並在 Settings 顯示依 provider／model 分組的累計與成本估算。

**Architecture:** 在 `AiRouter::resolve_by_id` 回傳的 `Arc<dyn AiProvider>` 外包一層 `MeteredProvider` 裝飾器 —— 它自建中繼 channel、轉發每個 chunk 給原本的呼叫端，同時攔截最後一個帶 `usage` 的 chunk 寫進 `usage.db`。**一個接點覆蓋全部 6 個呼叫端檔案，沒有任何既有消費端需要改動。**

**Tech Stack:** Rust / sqlx (SQLite) / async_trait / Tauri 2 IPC / React 19 / Vitest

**規格來源：** `docs/superpowers/specs/2026-08-15-model-usage-display-design.md`

**前置條件：** `src-tauri/binaries/uv-aarch64-apple-darwin` 必須存在，否則連 `cargo test` 都會因 `externalBin` 驗證失敗（見 CLAUDE.md）。本機已確認存在。

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/usage/mod.rs`（新增） | 模組宣告與公開型別 re-export |
| `src-tauri/src/usage/store.rs`（新增） | `UsageStore`：SQLite 建表、寫入單筆、區間彙總查詢 |
| `src-tauri/src/usage/pricing.rs`（新增） | 單價表與成本估算，純函式無 I/O |
| `src-tauri/src/usage/metered.rs`（新增） | `MeteredProvider` 裝飾器 |
| `src-tauri/src/ai/mod.rs:144-148`（修改） | `TokenUsage` 補 `cache_read` / `cache_write` 欄位 |
| `src-tauri/src/ai/anthropic.rs`（修改） | 解析 `cache_creation_input_tokens` / `cache_read_input_tokens` |
| `src-tauri/src/ai/codex.rs`（修改） | 解析 `input_tokens_details.{cache_write_tokens,cached_tokens}` |
| `src-tauri/src/ai/sse.rs`（修改） | `usage_into` 補新欄位（OpenAI 系無 cache 資訊，填 0） |
| `src-tauri/src/ai/antigravity.rs`（修改） | 補新欄位填 0 |
| `src-tauri/src/ai/router.rs`（修改） | `resolve_by_id` / `resolve` 回傳前包上 `MeteredProvider` |
| `src-tauri/src/commands/usage.rs`（新增） | `usage_summary` IPC 指令 |
| `src-tauri/src/lib.rs`（修改） | 註冊 `UsageStore` state 與 `usage_summary` handler |
| `src/ipc/usage.ts`（新增） | 前端型別與 invoke 包裝 |
| `src/components/Settings/UsagePage.tsx`（新增） | 用量頁（本計畫只做本地累計區） |
| `src/components/Settings/UsagePage.css`（新增） | 樣式 |
| `src/components/Settings/SettingsView.tsx`（修改） | 掛上新分頁 |
| `src/lib/i18n.ts`（修改） | en / zh-TW 字串 |
| `src/hooks/useAgentMission.ts`（修改） | 累加本次 mission 的 token |
| `src/components/AgentStatusBar.tsx`（修改） | 顯示本次 mission token 數 |

---

## Task 1: `TokenUsage` 補上快取欄位

現有 `TokenUsage` 只有 `prompt` / `completion`。Anthropic 與 Codex 的回應本來就帶快取 token 數，目前被丟掉 —— 而快取命中率是這份統計最有價值的一欄（成本差一個數量級）。

**Files:**
- Modify: `src-tauri/src/ai/mod.rs:144-148`
- Modify: `src-tauri/src/ai/sse.rs:321-326`
- Modify: `src-tauri/src/ai/codex.rs:232-235`, `src-tauri/src/ai/antigravity.rs:231-234`
- Modify: `src-tauri/src/ai/anthropic.rs:492-495`, `src-tauri/src/ai/anthropic.rs:610-613`

- [ ] **Step 1: 擴充結構**

`src-tauri/src/ai/mod.rs`，把：

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
}
```

改成：

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    /// 從快取讀取的 token（Anthropic `cache_read_input_tokens`、
    /// Codex `input_tokens_details.cached_tokens`）。來源沒提供就是 0。
    pub cache_read: u32,
    /// 寫入快取的 token（Anthropic `cache_creation_input_tokens`、
    /// Codex `input_tokens_details.cache_write_tokens`）。
    pub cache_write: u32,
}
```

- [ ] **Step 2: 編譯，看它在哪裡壞掉**

Run: `cd src-tauri && cargo check 2>&1 | grep -E "^error" -A 5`
Expected: 5 處 `missing fields cache_read and cache_write` —— `sse.rs`、`codex.rs`、`antigravity.rs`、`anthropic.rs` 兩處。

- [ ] **Step 3: 補上不提供快取資訊的來源（填 0）**

`src-tauri/src/ai/sse.rs` 的 `usage_into`：

```rust
    pub fn usage_into(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|u| TokenUsage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
            ..Default::default()
        })
    }
```

`src-tauri/src/ai/antigravity.rs:231`：

```rust
                let usage = chunk.usage_metadata.map(|u| TokenUsage {
                    prompt: u.prompt_token_count,
                    completion: u.candidates_token_count,
                    ..Default::default()
                });
```

- [ ] **Step 4: 確認只剩 Anthropic 與 Codex 未補**

Run: `cd src-tauri && cargo check 2>&1 | grep -c "^error"`
Expected: `3`（anthropic 兩處 + codex 一處，留給 Task 2、3）

- [ ] **Step 5: 先不 commit**，Task 2、3 完成後一起提交（中間狀態編不過）

---

## Task 2: Anthropic 解析快取 token

**Files:**
- Modify: `src-tauri/src/ai/anthropic.rs:686-691`（`MessageDeltaUsage`）
- Modify: `src-tauri/src/ai/anthropic.rs:492-495`, `:610-613`
- Test: `src-tauri/src/ai/anthropic.rs`（既有 `mod tests`）

探勘實測的真實回應片段（`probe_anthropic_usage.txt`）：

```json
"usage": { "input_tokens": 22, "cache_creation_input_tokens": 0,
           "cache_read_input_tokens": 0, "output_tokens": 1 }
```

- [ ] **Step 1: 寫失敗的測試**

加到 `src-tauri/src/ai/anthropic.rs` 既有的 `mod tests` 裡：

```rust
    #[test]
    fn message_delta_usage_parses_cache_tokens() {
        // 欄位取自真實回應（探勘 dump），不是自編的。
        let raw = r#"{"input_tokens":22,"cache_creation_input_tokens":150,
                      "cache_read_input_tokens":4096,"output_tokens":7}"#;
        let u: MessageDeltaUsage = serde_json::from_str(raw).expect("parse");
        assert_eq!(u.input_tokens, 22);
        assert_eq!(u.output_tokens, 7);
        assert_eq!(u.cache_creation_input_tokens, 150);
        assert_eq!(u.cache_read_input_tokens, 4096);
    }

    #[test]
    fn message_delta_usage_defaults_cache_tokens_to_zero() {
        // 舊版／非快取請求不會帶這兩個欄位，必須降級成 0 而不是解析失敗。
        let raw = r#"{"input_tokens":10,"output_tokens":2}"#;
        let u: MessageDeltaUsage = serde_json::from_str(raw).expect("parse");
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 0);
    }
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib ai::anthropic::tests::message_delta_usage 2>&1 | tail -20`
Expected: 編譯失敗，`no field cache_creation_input_tokens on type MessageDeltaUsage`

- [ ] **Step 3: 擴充結構**

`src-tauri/src/ai/anthropic.rs:686`：

```rust
struct MessageDeltaUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib ai::anthropic::tests::message_delta_usage 2>&1 | tail -5`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: 把欄位接到 `TokenUsage`**

`anthropic.rs:492` 與 `:610` **兩處**都改成（兩處的變數名都是 `usage`）：

```rust
                    let token_usage = usage.map(|u| TokenUsage {
                        prompt: u.input_tokens,
                        completion: u.output_tokens,
                        cache_read: u.cache_read_input_tokens,
                        cache_write: u.cache_creation_input_tokens,
                    });
```

- [ ] **Step 6: 確認 anthropic 不再有編譯錯誤**

Run: `cd src-tauri && cargo check 2>&1 | grep "anthropic.rs" | head`
Expected: 無輸出

---

## Task 3: Codex 解析快取 token

**Files:**
- Modify: `src-tauri/src/ai/codex.rs:291-297`（`CodexUsage`）、`:232-235`
- Test: `src-tauri/src/ai/codex.rs`（既有 `mod tests`）

探勘實測的真實回應片段（`probe_codex_usage.txt`，`response.completed` 事件）：

```json
"usage": { "input_tokens": 17,
           "input_tokens_details": { "cache_write_tokens": 0, "cached_tokens": 0 },
           "output_tokens": 13, "output_tokens_details": { "reasoning_tokens": 0 },
           "total_tokens": 30 }
```

- [ ] **Step 1: 寫失敗的測試**

加到 `src-tauri/src/ai/codex.rs` 既有的 `mod tests` 裡：

```rust
    #[test]
    fn codex_usage_parses_cache_token_details() {
        let raw = r#"{"input_tokens":17,
                      "input_tokens_details":{"cache_write_tokens":320,"cached_tokens":8192},
                      "output_tokens":13}"#;
        let u: CodexUsage = serde_json::from_str(raw).expect("parse");
        assert_eq!(u.input_tokens, 17);
        assert_eq!(u.output_tokens, 13);
        assert_eq!(u.input_tokens_details.cached_tokens, 8192);
        assert_eq!(u.input_tokens_details.cache_write_tokens, 320);
    }

    #[test]
    fn codex_usage_without_details_defaults_to_zero() {
        let raw = r#"{"input_tokens":10,"output_tokens":2}"#;
        let u: CodexUsage = serde_json::from_str(raw).expect("parse");
        assert_eq!(u.input_tokens_details.cached_tokens, 0);
        assert_eq!(u.input_tokens_details.cache_write_tokens, 0);
    }
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib ai::codex::tests::codex_usage 2>&1 | tail -20`
Expected: 編譯失敗，`no field input_tokens_details`

- [ ] **Step 3: 擴充結構**

`src-tauri/src/ai/codex.rs:291`：

```rust
#[derive(Deserialize, Default)]
struct CodexInputTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
    #[serde(default)]
    cache_write_tokens: u32,
}

#[derive(Deserialize)]
struct CodexUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: CodexInputTokensDetails,
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib ai::codex::tests::codex_usage 2>&1 | tail -5`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: 把欄位接到 `TokenUsage`**

`codex.rs:232`：

```rust
                    let usage = response.and_then(|r| r.usage).map(|u| TokenUsage {
                        prompt: u.input_tokens,
                        completion: u.output_tokens,
                        cache_read: u.input_tokens_details.cached_tokens,
                        cache_write: u.input_tokens_details.cache_write_tokens,
                    });
```

- [ ] **Step 6: 全庫編譯 + 全測試**

Run: `cd src-tauri && cargo test --lib 2>&1 | tail -5`
Expected: 全部通過（含既有的 `sse_event_completed_parses_usage`）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/mod.rs src-tauri/src/ai/anthropic.rs src-tauri/src/ai/codex.rs \
        src-tauri/src/ai/sse.rs src-tauri/src/ai/antigravity.rs
git commit -m "feat(usage): TokenUsage 補上 cache_read / cache_write 欄位

Anthropic 與 Codex 的回應本來就帶快取 token 數，先前被丟棄。
快取命中率是用量統計最有價值的一欄（成本差一個數量級）。
不提供此資訊的來源（OpenAI 系、Antigravity）填 0。"
```

---

## Task 4: `UsageStore` —— SQLite 落地

**Files:**
- Create: `src-tauri/src/usage/mod.rs`
- Create: `src-tauri/src/usage/store.rs`
- Modify: `src-tauri/src/lib.rs`（加 `mod usage;`）

- [ ] **Step 1: 建立模組骨架**

`src-tauri/src/usage/mod.rs`：

```rust
//! 用量記帳與配額查詢。
//!
//! `store` 負責把每次 AI 請求的 token 落地到 `usage.db`；`metered` 是包在
//! `AiProvider` 外的裝飾器，是唯一的記帳接點；`pricing` 是純函式的成本估算。

pub mod metered;
pub mod pricing;
pub mod store;

pub use store::{UsageRange, UsageStore, UsageSummaryRow};
```

`src-tauri/src/lib.rs` 在既有的模組宣告區加一行 `pub mod usage;`（與 `pub mod ai;`、`pub mod db;` 同區塊，`lib.rs:1-9`）。

- [ ] **Step 2: 寫失敗的測試**

`src-tauri/src/usage/store.rs`（先只寫測試，讓檔案存在）：

```rust
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
}
```

- [ ] **Step 3: 確認 `tempfile` 已是 dev-dependency**

Run: `cd src-tauri && grep -n "tempfile" Cargo.toml`
Expected: 出現在 `[dev-dependencies]`（CLAUDE.md 記載測試已用 tempfile）。若沒有，加 `tempfile = "3"` 到 `[dev-dependencies]`。

- [ ] **Step 4: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::store 2>&1 | tail -20`
Expected: 編譯失敗，`cannot find type UsageStore`

- [ ] **Step 5: 實作**

在 `src-tauri/src/usage/store.rs` **測試模組之前**加入：

```rust
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
```

- [ ] **Step 6: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::store 2>&1 | tail -8`
Expected: `test result: ok. 4 passed`

- [ ] **Step 7: 補上區間換算的測試**

加到同一個 `mod tests`：

```rust
    #[test]
    fn range_cutoff_boundaries() {
        // 2026-08-15 02:26:20 UTC
        let now = 1_786_760_780_i64;
        // Today 落在當天 UTC 00:00:00
        assert_eq!(UsageRange::Today.cutoff(now), 1_786_752_000);
        assert_eq!(UsageRange::Days7.cutoff(now), now - 604_800);
        assert_eq!(UsageRange::Days30.cutoff(now), now - 2_592_000);
    }
```

- [ ] **Step 8: 跑測試**

Run: `cd src-tauri && cargo test --lib usage::store 2>&1 | tail -5`
Expected: `test result: ok. 5 passed`

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/usage/mod.rs src-tauri/src/usage/store.rs src-tauri/src/lib.rs
git commit -m "feat(usage): UsageStore —— 用量紀錄的 SQLite 落地

比照 db/loop_sessions 的初始化寫法（明確 create_if_missing，
失敗退回 in-memory）。record() 刻意不回傳錯誤：記帳是旁路，
不得讓 AI 請求因記帳失敗而失敗。保留 90 天。"
```

---

## Task 5: `pricing.rs` —— 成本估算

**Files:**
- Create: `src-tauri/src/usage/pricing.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/usage/pricing.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_costs_are_computed_per_million_tokens() {
        // 1M input + 1M output 的 claude-sonnet-4-5 = 3.0 + 15.0
        let cost = estimate_cost("claude-sonnet-4-5", 1_000_000, 1_000_000, 0, 0)
            .expect("已知模型應有單價");
        assert!((cost - 18.0).abs() < 1e-6, "得到 {cost}");
    }

    #[test]
    fn cache_read_is_cheaper_than_fresh_input() {
        let fresh = estimate_cost("claude-sonnet-4-5", 1_000_000, 0, 0, 0).unwrap();
        let cached = estimate_cost("claude-sonnet-4-5", 0, 0, 1_000_000, 0).unwrap();
        assert!(cached < fresh, "快取讀取必須比新輸入便宜: {cached} vs {fresh}");
    }

    #[test]
    fn matches_by_prefix_so_dated_variants_resolve() {
        // 上游常回帶日期的完整 id，單價表只列基底名稱。
        let a = estimate_cost("claude-sonnet-4-5-20250929", 1_000_000, 0, 0, 0);
        let b = estimate_cost("claude-sonnet-4-5", 1_000_000, 0, 0, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn unknown_model_returns_none_rather_than_guessing() {
        // 查不到單價就不顯示金額。猜一個數字比不顯示更糟。
        assert!(estimate_cost("some-local-gguf-model", 1_000_000, 1_000_000, 0, 0).is_none());
    }

    #[test]
    fn longest_prefix_wins() {
        // "gpt-5.6-luna" 與 "gpt-5.6" 同時存在時，必須選較長的那個。
        let luna = PRICES.iter().find(|(k, _)| *k == "gpt-5.6-luna");
        if luna.is_some() {
            let a = estimate_cost("gpt-5.6-luna", 1_000_000, 0, 0, 0).unwrap();
            let b = estimate_cost("gpt-5.6", 1_000_000, 0, 0, 0).unwrap();
            assert_ne!(a, b, "較長的前綴必須勝出，否則單價表形同虛設");
        }
    }
}
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::pricing 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find function estimate_cost`

- [ ] **Step 3: 實作**

在測試模組之前加入：

```rust
//! 模型單價表與成本估算。純函式，無 I/O。
//!
//! 單價會過期。**查不到單價一律回 None（不顯示金額），絕不猜測** ——
//! 顯示一個錯的金額比不顯示更糟。

/// 每百萬 token 的美元單價。
#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// 以 model id 的**前綴**比對。同時命中多個時取最長的。
/// 單價來源：各供應商公開定價頁，2026-08 校對。
pub const PRICES: &[(&str, ModelPrice)] = &[
    ("claude-opus-4", ModelPrice { input: 15.0, output: 75.0, cache_read: 1.50, cache_write: 18.75 }),
    ("claude-sonnet-4-5", ModelPrice { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-sonnet-4", ModelPrice { input: 3.0, output: 15.0, cache_read: 0.30, cache_write: 3.75 }),
    ("claude-haiku-4-5", ModelPrice { input: 1.0, output: 5.0, cache_read: 0.10, cache_write: 1.25 }),
    ("gpt-5.6", ModelPrice { input: 1.25, output: 10.0, cache_read: 0.125, cache_write: 0.0 }),
    ("gpt-4.1", ModelPrice { input: 2.0, output: 8.0, cache_read: 0.50, cache_write: 0.0 }),
    ("gemini-3.5-flash", ModelPrice { input: 0.30, output: 2.50, cache_read: 0.075, cache_write: 0.0 }),
    ("gemini-2.5-flash", ModelPrice { input: 0.30, output: 2.50, cache_read: 0.075, cache_write: 0.0 }),
];

/// 找出前綴命中且最長的單價。
pub fn price_for(model: &str) -> Option<ModelPrice> {
    PRICES
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
        .map(|(_, p)| *p)
}

/// 估算美元成本。查不到單價回 `None`。
pub fn estimate_cost(
    model: &str,
    prompt: i64,
    completion: i64,
    cache_read: i64,
    cache_write: i64,
) -> Option<f64> {
    let p = price_for(model)?;
    let m = 1_000_000.0;
    Some(
        prompt as f64 / m * p.input
            + completion as f64 / m * p.output
            + cache_read as f64 / m * p.cache_read
            + cache_write as f64 / m * p.cache_write,
    )
}
```

- [ ] **Step 4: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::pricing 2>&1 | tail -5`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/usage/pricing.rs
git commit -m "feat(usage): 模型單價表與成本估算

前綴比對，同時命中取最長者。查不到單價回 None 不顯示金額 ——
顯示一個猜出來的金額比不顯示更糟。"
```

---

## Task 6: `MeteredProvider` 裝飾器

這是整個計畫的核心。`GenerateChunk` 的消費端散在 6 個檔案（光 `commands/ai.rs` 就有 7 處 `rx.recv()` 迴圈），逐一改保證會漏；包一層則零既有消費端改動。

**Files:**
- Create: `src-tauri/src/usage/metered.rs`

- [ ] **Step 1: 寫失敗的測試**

`src-tauri/src/usage/metered.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    /// 假 provider：吐出固定的 chunk 序列，最後一個帶 usage。
    struct FakeProvider {
        chunks: Vec<GenerateChunk>,
    }

    #[async_trait]
    impl AiProvider for FakeProvider {
        fn id(&self) -> &str { "fake" }
        fn display_name(&self) -> &str { "Fake" }
        async fn generate(
            &self,
            _req: GenerateRequest,
            tx: mpsc::Sender<GenerateChunk>,
        ) -> Result<(), AiError> {
            for c in &self.chunks {
                tx.send(c.clone()).await.ok();
            }
            Ok(())
        }
        async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
    }

    /// 欄位名稱取自 `ai/mod.rs:129-135` 與 `:77-87`，不要憑印象改。
    fn req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: String::new(),
            messages: vec![],
            context: EnvSnapshot {
                os: "macos".into(),
                shell: "zsh".into(),
                cwd: PathBuf::from("/"),
                recent_output: None,
                dir_listing: None,
            },
            mode: QueryMode::Chat,
            max_tokens: None,
        }
    }

    async fn store() -> (Arc<UsageStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Arc::new(UsageStore::new_at(dir.path().join("u.db")).await), dir)
    }

    #[tokio::test]
    async fn forwards_every_chunk_unchanged() {
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![
                GenerateChunk { delta: "he".into(), done: false, usage: None },
                GenerateChunk { delta: "llo".into(), done: false, usage: None },
                GenerateChunk { delta: String::new(), done: true, usage: None },
            ],
        });
        let metered = MeteredProvider::new(inner, store, "p".into(), "m".into());

        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");

        let mut text = String::new();
        let mut count = 0;
        while let Some(c) = rx.recv().await {
            text.push_str(&c.delta);
            count += 1;
        }
        assert_eq!(text, "hello", "轉發不得改動內容");
        assert_eq!(count, 3, "每個 chunk 都要轉發，一個都不能吞");
    }

    #[tokio::test]
    async fn records_usage_from_final_chunk() {
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![GenerateChunk {
                delta: String::new(),
                done: true,
                usage: Some(TokenUsage {
                    prompt: 22, completion: 7, cache_read: 4096, cache_write: 150,
                }),
            }],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "anthropic-pro".into(),
                                           "claude-sonnet-4-5".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "anthropic-pro");
        assert_eq!(rows[0].model, "claude-sonnet-4-5");
        assert_eq!(rows[0].prompt_tokens, 22);
        assert_eq!(rows[0].cache_read_tokens, 4096);
    }

    #[tokio::test]
    async fn no_usage_means_no_record() {
        // 上游沒回 usage 就不寫，不要用估算值補 —— 那會讓統計失去可信度。
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![GenerateChunk { delta: "x".into(), done: true, usage: None }],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "p".into(), "m".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        assert!(store.summary_since(0).await.expect("summary").is_empty());
    }

    #[tokio::test]
    async fn usage_on_non_final_chunk_is_still_captured() {
        // Anthropic 的 usage 出現在 done:false 的 message_delta 上
        // （見 anthropic.rs:496），不是最後那個 chunk。
        let (store, _d) = store().await;
        let inner = Arc::new(FakeProvider {
            chunks: vec![
                GenerateChunk {
                    delta: String::new(),
                    done: false,
                    usage: Some(TokenUsage { prompt: 9, completion: 4, cache_read: 0, cache_write: 0 }),
                },
                GenerateChunk { delta: String::new(), done: true, usage: None },
            ],
        });
        let metered = MeteredProvider::new(inner, store.clone(), "p".into(), "m".into());
        let (tx, mut rx) = mpsc::channel(16);
        metered.generate(req(), tx).await.expect("generate");
        while rx.recv().await.is_some() {}

        let rows = store.summary_since(0).await.expect("summary");
        assert_eq!(rows.len(), 1, "done:false 上的 usage 也必須記到");
        assert_eq!(rows[0].prompt_tokens, 9);
    }
}
```

> **`usage_on_non_final_chunk_is_still_captured` 是這個 Task 最重要的測試。** Anthropic 把 usage 放在 `done: false` 的 chunk 上（`anthropic.rs:496`），只看最後一個 chunk 的實作會**完全漏掉 Anthropic 的用量**卻不報任何錯。

- [ ] **Step 2: 跑測試確認它失敗**

Run: `cd src-tauri && cargo test --lib usage::metered 2>&1 | tail -10`
Expected: 編譯失敗，`cannot find type MeteredProvider`

- [ ] **Step 3: 實作**

在測試模組之前加入：

```rust
//! 包在 `AiProvider` 外的記帳裝飾器。
//!
//! `GenerateChunk` 的消費端散在 6 個檔案（`commands/ai.rs` 一支就有 7 處
//! `rx.recv()` 迴圈），逐一加記帳保證會漏。改在 `AiRouter` 解析出 provider
//! 時包一層，一個接點覆蓋全部呼叫端。
//!
//! ```text
//! 呼叫端 ──tx──▶ MeteredProvider ──tx2──▶ 真正的 provider
//!                      └─ 攔截 usage ─▶ usage.db
//! ```

use crate::ai::{
    AiError, AiProvider, GenerateChunk, GenerateRequest, GenerateWithToolsResult, McpToolDefinition,
    TokenUsage,
};
use crate::usage::store::UsageStore;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct MeteredProvider {
    inner: Arc<dyn AiProvider>,
    store: Arc<UsageStore>,
    provider_id: String,
    model: String,
}

impl MeteredProvider {
    pub fn new(
        inner: Arc<dyn AiProvider>,
        store: Arc<UsageStore>,
        provider_id: String,
        model: String,
    ) -> Self {
        Self { inner, store, provider_id, model }
    }

    /// 建立中繼 channel，回傳「給內層用的 tx」與「轉發任務的 handle」。
    /// handle 完成時給出攔截到的最後一筆 usage。
    fn tap(
        &self,
        out: mpsc::Sender<GenerateChunk>,
    ) -> (mpsc::Sender<GenerateChunk>, tokio::task::JoinHandle<Option<TokenUsage>>) {
        let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(16);
        let handle = tokio::spawn(async move {
            let mut last: Option<TokenUsage> = None;
            while let Some(chunk) = rx2.recv().await {
                // 任何 chunk 上的 usage 都要收 —— Anthropic 放在 done:false 上。
                if chunk.usage.is_some() {
                    last = chunk.usage;
                }
                // 送不出去代表呼叫端已經走了，繼續收乾內層以免它卡在 send。
                let _ = out.send(chunk).await;
            }
            last
        });
        (tx2, handle)
    }

    /// 等轉發任務結束、拿到 usage 就寫入。**任何失敗都只記 log。**
    async fn finish(&self, handle: tokio::task::JoinHandle<Option<TokenUsage>>) {
        let usage = match handle.await {
            Ok(u) => u,
            Err(e) => {
                log::warn!("用量轉發任務失敗（不影響 AI 流程）: {e}");
                return;
            }
        };
        // 上游沒回 usage 就不寫。不要用估算值補。
        let Some(usage) = usage else { return };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.store.record(&self.provider_id, &self.model, usage, now).await;
    }
}

#[async_trait]
impl AiProvider for MeteredProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn display_name(&self) -> &str {
        self.inner.display_name()
    }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let (tx2, handle) = self.tap(tx);
        // tx2 在 inner.generate 返回時被丟棄，轉發任務隨之結束。
        let result = self.inner.generate(req, tx2).await;
        self.finish(handle).await;
        result
    }

    async fn health_check(&self) -> Result<(), AiError> {
        self.inner.health_check().await
    }

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let (tx2, handle) = self.tap(tx);
        let result = self.inner.generate_with_tools(req, tools, tx2).await;
        self.finish(handle).await;
        result
    }
}
```

- [ ] **Step 4: 對照 trait 定義補齊未覆寫的方法**

Run: `cd src-tauri && sed -n '214,260p' src/ai/mod.rs`

檢查 `AiProvider` 上是否還有其他**有預設實作**的方法。若有任何方法會改變行為（例如另一個串流入口），必須一併委派給 `self.inner`，否則裝飾器會靜默改變行為。把發現的每個方法都加上委派。

- [ ] **Step 5: 跑測試確認通過**

Run: `cd src-tauri && cargo test --lib usage::metered 2>&1 | tail -8`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/usage/metered.rs src-tauri/src/usage/mod.rs
git commit -m "feat(usage): MeteredProvider 記帳裝飾器

包在 AiProvider 外攔截 chunk 上的 usage，一個接點覆蓋全部 6 個
消費端檔案，零既有程式碼改動。

關鍵：usage 可能出現在 done:false 的 chunk 上（Anthropic 即是），
只看最後一個 chunk 會完全漏掉 Anthropic 的用量且不報錯。"
```

---

## Task 7: 接到 `AiRouter` 與 Tauri state

**Files:**
- Modify: `src-tauri/src/ai/router.rs:417-450`（`AiRouter` 結構與 `new`）、`:429-448`（`resolve`）、`:451+`（`resolve_by_id` 結尾）
- Modify: `src-tauri/src/lib.rs:131` 附近與 `:248-265` 的 `.manage()` 區塊

- [ ] **Step 1: `AiRouter` 持有 `UsageStore`**

`src-tauri/src/ai/router.rs`，把結構與建構子改成：

```rust
pub struct AiRouter {
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    usage: Arc<crate::usage::UsageStore>,
}

impl AiRouter {
    pub fn new(
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        usage: Arc<crate::usage::UsageStore>,
    ) -> Self {
        Self { config, secrets, usage }
    }
```

- [ ] **Step 2: `resolve_by_id` 結尾包上裝飾器**

`resolve_by_id` 目前的結尾是 `Ok(provider)`（`provider` 是 `Arc<dyn AiProvider>`）。改成：

```rust
        Ok(Arc::new(crate::usage::metered::MeteredProvider::new(
            provider,
            self.usage.clone(),
            provider_cfg.id.clone(),
            provider_cfg.model.clone(),
        )))
```

> 注意 `provider_cfg` 在函式開頭已被 `.clone()` 出來，但後續分支可能已把 `provider_cfg.model` move 走。若編譯報 use-after-move，在函式開頭先 `let (meter_id, meter_model) = (provider_cfg.id.clone(), provider_cfg.model.clone());`，結尾改用這兩個變數。

- [ ] **Step 3: `resolve` 的環境變數 fallback 路徑也要包**

`router.rs` 的 `resolve` 裡有一段在完全沒設定 provider 時用 `OPENAI_API_KEY` 建立 `OpenAiClient` 並直接回傳。那條路徑繞過 `resolve_by_id`，同樣要包：

```rust
                if !key.trim().is_empty() {
                    let client: Arc<dyn AiProvider> = Arc::new(OpenAiClient::new(key));
                    return Ok(Arc::new(crate::usage::metered::MeteredProvider::new(
                        client,
                        self.usage.clone(),
                        "env:OPENAI_API_KEY".into(),
                        "gpt-4o-mini".into(),
                    )));
                }
```

> provider_id 用 `env:OPENAI_API_KEY` 是刻意的：它在統計表裡要能跟真正設定過的 provider 區分開。

- [ ] **Step 4: `lib.rs` 建立並註冊 state**

在 `lib.rs` 建立 `loop_session_db` 的那一段附近（約 `:131`）加：

```rust
    let usage_store = Arc::new(tauri::async_runtime::block_on(async {
        let s = crate::usage::UsageStore::new().await;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        s.prune_expired(now).await;
        s
    }));
```

找到建立 `AiRouter` 的那一行，把 `usage_store.clone()` 當第三個參數傳進去。

在 `.manage()` 區塊加一行（緊接 `.manage(router)` 之後）：

```rust
        .manage(usage_store)
```

- [ ] **Step 5: 修掉其他 `AiRouter::new` 呼叫端**

Run: `cd src-tauri && grep -rn "AiRouter::new" src/ tests/`

每一處都要補第三個參數。測試裡的呼叫用臨時 store：

```rust
Arc::new(UsageStore::new_at(temp_dir.path().join("usage.db")).await)
```

- [ ] **Step 6: 全庫編譯與測試**

Run: `cd src-tauri && cargo test 2>&1 | tail -15`
Expected: 全部通過

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/router.rs src-tauri/src/lib.rs
git commit -m "feat(usage): AiRouter 解析 provider 時包上 MeteredProvider

resolve_by_id 與 resolve 的 OPENAI_API_KEY fallback 兩條路徑都要包，
後者繞過 resolve_by_id，漏掉會讓未設定 provider 的使用者完全沒有統計。
啟動時清理 90 天前的舊紀錄。"
```

---

## Task 8: `usage_summary` IPC 指令

**Files:**
- Create: `src-tauri/src/commands/usage.rs`
- Modify: `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`（`invoke_handler`）

- [ ] **Step 1: 實作指令**

`src-tauri/src/commands/usage.rs`：

```rust
//! 用量統計的 IPC 入口。

use crate::usage::pricing::estimate_cost;
use crate::usage::{UsageRange, UsageStore};
use serde::Serialize;
use tauri::State;

/// 一列統計，比 `UsageSummaryRow` 多了成本估算。
#[derive(Debug, Serialize)]
pub struct UsageSummaryEntry {
    pub provider_id: String,
    pub model: String,
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    /// 查不到單價時為 None —— 前端顯示「—」而不是 0。
    pub estimated_cost_usd: Option<f64>,
}

#[tauri::command]
pub async fn usage_summary(
    range: UsageRange,
    store: State<'_, std::sync::Arc<UsageStore>>,
) -> Result<Vec<UsageSummaryEntry>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let rows = store
        .summary_since(range.cutoff(now))
        .await
        .map_err(|e| format!("查詢用量統計失敗: {e}"))?;

    Ok(rows
        .into_iter()
        .map(|r| UsageSummaryEntry {
            estimated_cost_usd: estimate_cost(
                &r.model,
                r.prompt_tokens,
                r.completion_tokens,
                r.cache_read_tokens,
                r.cache_write_tokens,
            ),
            provider_id: r.provider_id,
            model: r.model,
            requests: r.requests,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            cache_read_tokens: r.cache_read_tokens,
            cache_write_tokens: r.cache_write_tokens,
        })
        .collect())
}
```

- [ ] **Step 2: 註冊**

`src-tauri/src/commands/mod.rs` 加 `pub mod usage;`。
`src-tauri/src/lib.rs` 的 `tauri::generate_handler![...]` 裡加 `commands::usage::usage_summary,`。

- [ ] **Step 3: 編譯**

Run: `cd src-tauri && cargo check 2>&1 | grep -E "^error" -A 5`
Expected: 無輸出

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/usage.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(usage): usage_summary IPC 指令

成本為 Option：查不到單價回 None，前端顯示「—」而非 0。"
```

---

## Task 9: 前端 IPC 包裝與用量頁

**Files:**
- Create: `src/ipc/usage.ts`
- Create: `src/components/Settings/UsagePage.tsx`, `UsagePage.css`, `UsagePage.test.tsx`
- Modify: `src/components/Settings/SettingsView.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: IPC 包裝**

`src/ipc/usage.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";

export type UsageRange = "today" | "days7" | "days30";

export interface UsageSummaryEntry {
  provider_id: string;
  model: string;
  requests: number;
  prompt_tokens: number;
  completion_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  /** null 代表查不到單價 —— 顯示「—」而不是 0。 */
  estimated_cost_usd: number | null;
}

export function usageSummary(range: UsageRange): Promise<UsageSummaryEntry[]> {
  return invoke<UsageSummaryEntry[]>("usage_summary", { range });
}

/** 快取命中率：讀自快取的 token 佔總輸入的比例。無輸入時回 null。 */
export function cacheHitRate(e: UsageSummaryEntry): number | null {
  const total = e.prompt_tokens + e.cache_read_tokens;
  if (total === 0) return null;
  return e.cache_read_tokens / total;
}
```

- [ ] **Step 2: 寫失敗的測試**

`src/components/Settings/UsagePage.test.tsx`：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { UsagePage } from "./UsagePage";

const mockSummary = vi.fn();
vi.mock("../../ipc/usage", async () => {
  const actual = await vi.importActual<typeof import("../../ipc/usage")>("../../ipc/usage");
  return { ...actual, usageSummary: (r: string) => mockSummary(r) };
});

describe("UsagePage", () => {
  beforeEach(() => {
    mockSummary.mockReset();
  });

  it("預設載入今天的統計", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("today"));
  });

  it("顯示每個 provider/model 的 token 與成本", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "anthropic-pro", model: "claude-sonnet-4-5",
      requests: 12, prompt_tokens: 5000, completion_tokens: 1200,
      cache_read_tokens: 40000, cache_write_tokens: 800,
      estimated_cost_usd: 0.0435,
    }]);
    render(<UsagePage />);
    expect(await screen.findByText("claude-sonnet-4-5")).toBeInTheDocument();
    expect(screen.getByText("anthropic-pro")).toBeInTheDocument();
    expect(screen.getByText("$0.0435")).toBeInTheDocument();
  });

  it("查不到單價時顯示破折號而不是 $0", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "local", model: "qwen3.6-27b",
      requests: 3, prompt_tokens: 100, completion_tokens: 50,
      cache_read_tokens: 0, cache_write_tokens: 0,
      estimated_cost_usd: null,
    }]);
    render(<UsagePage />);
    await screen.findByText("qwen3.6-27b");
    expect(screen.queryByText("$0")).not.toBeInTheDocument();
    expect(screen.getByTestId("cost-local-qwen3.6-27b")).toHaveTextContent("—");
  });

  it("顯示快取命中率", async () => {
    mockSummary.mockResolvedValue([{
      provider_id: "anthropic-pro", model: "claude-sonnet-4-5",
      requests: 1, prompt_tokens: 1000, completion_tokens: 10,
      cache_read_tokens: 9000, cache_write_tokens: 0,
      estimated_cost_usd: 0.01,
    }]);
    render(<UsagePage />);
    // 9000 / (1000 + 9000) = 90%
    expect(await screen.findByText("90%")).toBeInTheDocument();
  });

  it("切換區間會重新查詢", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("today"));
    await userEvent.click(screen.getByRole("button", { name: /7/ }));
    await waitFor(() => expect(mockSummary).toHaveBeenCalledWith("days7"));
  });

  it("沒有資料時顯示空狀態", async () => {
    mockSummary.mockResolvedValue([]);
    render(<UsagePage />);
    expect(await screen.findByTestId("usage-empty")).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: 跑測試確認它失敗**

Run: `npm run test -- UsagePage 2>&1 | tail -20`
Expected: FAIL，`Failed to resolve import "./UsagePage"`

- [ ] **Step 4: 實作元件**

`src/components/Settings/UsagePage.tsx`：

```tsx
import { useEffect, useState } from "react";
import { usageSummary, cacheHitRate, type UsageRange, type UsageSummaryEntry } from "../../ipc/usage";
import { useLocale } from "../../contexts/LocaleContext";
import "./UsagePage.css";

const RANGES: { key: UsageRange; labelKey: "usageToday" | "usage7Days" | "usage30Days" }[] = [
  { key: "today", labelKey: "usageToday" },
  { key: "days7", labelKey: "usage7Days" },
  { key: "days30", labelKey: "usage30Days" },
];

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function UsagePage() {
  const { t } = useLocale();
  const [range, setRange] = useState<UsageRange>("today");
  const [rows, setRows] = useState<UsageSummaryEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    setError(null);
    usageSummary(range)
      .then((r) => { if (!cancelled) setRows(r); })
      .catch((e) => { if (!cancelled) setError(String(e)); });
    return () => { cancelled = true; };
  }, [range]);

  return (
    <div className="usage-page">
      <div className="usage-range-tabs">
        {RANGES.map((r) => (
          <button
            key={r.key}
            className={r.key === range ? "active" : ""}
            onClick={() => setRange(r.key)}
          >
            {t(r.labelKey)}
          </button>
        ))}
      </div>

      {error && <div className="usage-error">{error}</div>}

      {rows !== null && rows.length === 0 && (
        <div className="usage-empty" data-testid="usage-empty">{t("usageEmpty")}</div>
      )}

      {rows !== null && rows.length > 0 && (
        <table className="usage-table">
          <thead>
            <tr>
              <th>{t("usageProvider")}</th>
              <th>{t("usageModel")}</th>
              <th>{t("usageRequests")}</th>
              <th>{t("usageInput")}</th>
              <th>{t("usageOutput")}</th>
              <th>{t("usageCacheHit")}</th>
              <th>{t("usageCost")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((e) => {
              const hit = cacheHitRate(e);
              return (
                <tr key={`${e.provider_id}-${e.model}`}>
                  <td>{e.provider_id}</td>
                  <td>{e.model}</td>
                  <td>{e.requests}</td>
                  <td>{formatTokens(e.prompt_tokens)}</td>
                  <td>{formatTokens(e.completion_tokens)}</td>
                  <td>{hit === null ? "—" : `${Math.round(hit * 100)}%`}</td>
                  <td data-testid={`cost-${e.provider_id}-${e.model}`}>
                    {e.estimated_cost_usd === null
                      ? "—"
                      : `$${e.estimated_cost_usd.toFixed(4)}`}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
```

- [ ] **Step 5: 樣式**

`src/components/Settings/UsagePage.css`：

```css
.usage-page { padding: 16px; }
.usage-range-tabs { display: flex; gap: 8px; margin-bottom: 16px; }
.usage-range-tabs button {
  padding: 4px 12px; border-radius: 6px; border: 1px solid var(--border, #3a3a3a);
  background: transparent; color: inherit; cursor: pointer;
}
.usage-range-tabs button.active { background: var(--accent, #3b82f6); color: #fff; }
.usage-table { width: 100%; border-collapse: collapse; font-size: 13px; }
.usage-table th, .usage-table td {
  text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--border, #2a2a2a);
}
.usage-table th { font-weight: 600; opacity: 0.7; }
.usage-empty { opacity: 0.6; padding: 24px; text-align: center; }
.usage-error { color: var(--danger, #ef4444); padding: 12px; }
```

- [ ] **Step 6: i18n 字串**

`src/lib/i18n.ts` 的 en 與 zh-TW 兩份都加：

```ts
  usageTitle: "Usage",              // zh-TW: "用量"
  usageToday: "Today",              // zh-TW: "今天"
  usage7Days: "7 days",             // zh-TW: "7 天"
  usage30Days: "30 days",           // zh-TW: "30 天"
  usageProvider: "Provider",        // zh-TW: "供應商"
  usageModel: "Model",              // zh-TW: "模型"
  usageRequests: "Requests",        // zh-TW: "次數"
  usageInput: "Input",              // zh-TW: "輸入"
  usageOutput: "Output",            // zh-TW: "輸出"
  usageCacheHit: "Cache hit",       // zh-TW: "快取命中"
  usageCost: "Est. cost",           // zh-TW: "估算成本"
  usageEmpty: "No usage recorded yet.",  // zh-TW: "尚無用量紀錄。"
```

- [ ] **Step 7: 跑測試確認通過**

Run: `npm run test -- UsagePage 2>&1 | tail -10`
Expected: `6 passed`

- [ ] **Step 8: 掛上 Settings 分頁**

`src/components/Settings/SettingsView.tsx`：照既有分頁（例如 `MailAccountsPage`）的寫法加一個 `usage` 分頁，import `UsagePage`，標題用 `t("usageTitle")`。

- [ ] **Step 9: 型別檢查與完整測試**

Run: `npx tsc -b && npm run test 2>&1 | tail -8`
Expected: 型別檢查無輸出、測試全過

> 型別檢查一定要用 `npx tsc -b`。`tsc --noEmit` 因為根 `tsconfig.json` 是 solution file（`"files": []`），永遠回 0 什麼都不檢查。

- [ ] **Step 10: Commit**

```bash
git add src/ipc/usage.ts src/components/Settings/UsagePage.tsx \
        src/components/Settings/UsagePage.css src/components/Settings/UsagePage.test.tsx \
        src/components/Settings/SettingsView.tsx src/lib/i18n.ts
git commit -m "feat(usage): Settings 用量頁（本地累計）

依 provider/model 分組顯示次數、input/output、快取命中率與估算成本。
查不到單價顯示「—」而非 \$0。"
```

---

## Task 10: Agent mission 即時用量

`useAgentMission.ts` 的多步迴圈最會爆量，一次 mission 常比一整天手動問答還多。

**前提（實際查證過的成本）：** `AiStreamEvent`（`src/ipc/ai.ts:115-120`）目前**不帶任何 token 資訊**，而 `useAgentMission.ts` 只有 `stepCount` / `history`。所以這個 Task 必須先把 usage 帶上事件 —— `commands/ai.rs` 有 **5 個 `app.emit("ai-stream", ...)` 點**（`:218`、`:369`、`:414`、`:422`，以及 `:588` 之後的迴圈），全部都要補欄位才編得過。

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`（`AiStreamEvent` 定義 + 5 個 emit 點）
- Modify: `src/ipc/ai.ts:115-120`
- Modify: `src/hooks/useAgentMission.ts`
- Modify: `src/components/AgentStatusBar.tsx`
- Test: `src/components/AgentStatusBar.test.tsx`（既有）

- [ ] **Step 1: 後端事件加上 token 欄位**

`src-tauri/src/commands/ai.rs` 的 `AiStreamEvent` 結構加：

```rust
    /// 本次請求的總 token（prompt + completion）。只在 `done` 的事件上有值。
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<u32>,
```

- [ ] **Step 2: 編譯，看它在哪些 emit 點壞掉**

Run: `cd src-tauri && cargo check 2>&1 | grep -E "^error" -A 3 | grep -c "missing field .tokens."`
Expected: 5

- [ ] **Step 3: 每個 emit 點補上欄位**

不帶 usage 的中途事件一律 `tokens: None`。**帶 usage 的那些 chunk** 改成：

```rust
            tokens: chunk.usage.map(|u| u.prompt + u.completion),
```

`:414` 那個點（`tool_markup` 的可見 delta）與 `:422`（該迴圈的 done）用同一份 `chunk`，前者填 `None`、後者填 `chunk.usage.map(...)`。

> **不要用 `chunk.done` 當判斷條件。** Anthropic 的 usage 出現在 `done: false` 的 chunk 上（`ai/anthropic.rs:496`）。條件一律是「這個 chunk 有沒有 usage」。

- [ ] **Step 4: 全庫編譯與測試**

Run: `cd src-tauri && cargo test 2>&1 | tail -6`
Expected: 全過

- [ ] **Step 5: 前端型別**

`src/ipc/ai.ts` 的 `AiStreamEvent` 加：

```ts
  /** 本次請求的總 token。只有 done 的事件會帶。 */
  tokens?: number;
```

- [ ] **Step 6: `useAgentMission` 累加**

`AgentMission` 介面加 `tokensUsed: number;`，`startMission` 的初始物件加 `tokensUsed: 0,`，並新增一個累加函式：

```ts
  const addTokens = useCallback((n: number) => {
    setAgentMission((prev) => {
      if (!prev || !prev.active) return prev;
      return { ...prev, tokensUsed: prev.tokensUsed + n };
    });
  }, []);
```

加進 return 物件：`addTokens,`。

在 `TerminalView.tsx` 監聽 `ai-stream` 的地方，收到帶 `tokens` 的事件就呼叫 `addTokens(e.tokens)`。

Run: `grep -n "ai-stream" src/components/TerminalView.tsx`

- [ ] **Step 7: 寫失敗的測試**

先看既有測試怎麼 render 這個元件：

Run: `grep -n "render(<AgentStatusBar" -A 6 src/components/AgentStatusBar.test.tsx | head -20`

沿用它的 props 寫法，加上：

```tsx
  it("顯示本次 mission 的累計 token", () => {
    render(<AgentStatusBar {...baseProps} missionTokens={12400} />);
    expect(screen.getByTestId("mission-tokens")).toHaveTextContent("12.4k");
  });

  it("token 為 0 時不顯示這一段", () => {
    render(<AgentStatusBar {...baseProps} missionTokens={0} />);
    expect(screen.queryByTestId("mission-tokens")).not.toBeInTheDocument();
  });
```

> `baseProps` 沿用該檔案既有的 props 物件；若沒有現成的，從既有測試的 render 呼叫抽一個出來。

- [ ] **Step 8: 跑測試確認它失敗**

Run: `npm run test -- AgentStatusBar 2>&1 | tail -15`
Expected: FAIL，找不到 `mission-tokens`

- [ ] **Step 9: 加上 prop 與顯示**

`AgentStatusBar.tsx` 的 props 介面加：

```tsx
  /** 本次 mission 累計 token；0 或未提供時不顯示。 */
  missionTokens?: number;
```

在狀態列既有內容之後加：

```tsx
      {missionTokens ? (
        <span className="agent-status-tokens" data-testid="mission-tokens">
          {missionTokens >= 1000
            ? `${(missionTokens / 1000).toFixed(1)}k`
            : String(missionTokens)}
        </span>
      ) : null}
```

在 `TerminalView.tsx` 渲染 `AgentStatusBar` 的地方傳入 `missionTokens={agentMission?.tokensUsed ?? 0}`。

- [ ] **Step 10: 跑測試**

Run: `npm run test -- AgentStatusBar 2>&1 | tail -8`
Expected: 全過

- [ ] **Step 11: 完整驗證**

Run: `npx tsc -b && npm run test 2>&1 | tail -5 && cd src-tauri && cargo test 2>&1 | tail -5`
Expected: 三者皆通過

- [ ] **Step 12: Commit**

```bash
git add src-tauri/src/commands/ai.rs src/ipc/ai.ts src/hooks/useAgentMission.ts \
        src/components/AgentStatusBar.tsx src/components/AgentStatusBar.test.tsx \
        src/components/TerminalView.tsx
git commit -m "feat(usage): agent mission 進行中顯示累計 token

ai-stream 事件補上 tokens 欄位（5 個 emit 點）。判斷條件是「chunk 有沒有
usage」而非 chunk.done —— Anthropic 的 usage 在 done:false 上。"
```

---

## 完成驗收

- [ ] `cd src-tauri && cargo test` 全過
- [ ] `npx tsc -b` 無輸出
- [ ] `npm run test` 全過
- [ ] `npm run lint` 無新增警告
- [ ] `npm run tauri:dev` 實際跑一次：送幾個 `/ai` 查詢後，Settings → 用量頁看得到紀錄，且 **Anthropic provider 的紀錄有 cache_read 數字**（驗證 `done:false` 上的 usage 真的被攔到 —— 這是最容易靜默失效的地方）

> 最後一項不能只看「測試綠」。測試用的是假 provider，真正的 Anthropic 串流時序可能不同。要用真實請求驗證，並確認統計表裡的數字非零。

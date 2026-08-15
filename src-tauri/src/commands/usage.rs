//! 用量/配額相關的 IPC 指令。

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::usage::pricing::estimate_cost;
use crate::usage::quota::{cache::QuotaCache, source_for, ProviderQuota};
use crate::usage::{UsageRange, UsageStore};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

/// 單一 provider 的配額查詢結果。
///
/// **三態必須能分辨**：查到了、沒有配額概念、查詢失敗。把後兩者壓成同一種，
/// UI 會對使用者的本地模型（Ollama、本地 vLLM）顯示「配額查詢失敗」——
/// 本地模型根本沒有配額這回事，那是錯的訊息。
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QuotaResult {
    Ok { quota: ProviderQuota },
    /// 這個 provider 沒有訂閱配額概念（Ollama、所有 API key 型）。
    NotApplicable { provider_id: String },
    Failed { provider_id: String, message: String },
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn fetch_one(
    provider_id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    cache: &QuotaCache,
    force: bool,
) -> QuotaResult {
    let source = match source_for(provider_id, config, secrets).await {
        Ok(Some(s)) => s,
        Ok(None) => return QuotaResult::NotApplicable { provider_id: provider_id.into() },
        Err(e) => {
            return QuotaResult::Failed {
                provider_id: provider_id.into(),
                message: e.to_string(),
            }
        }
    };
    match cache.get_or_fetch(provider_id, source.as_ref(), force, now_secs()).await {
        Ok(quota) => QuotaResult::Ok { quota },
        Err(e) => QuotaResult::Failed {
            provider_id: provider_id.into(),
            message: e.to_string(),
        },
    }
}

#[tauri::command]
pub async fn usage_quota(
    provider_id: String,
    force: bool,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    cache: State<'_, Arc<QuotaCache>>,
) -> Result<QuotaResult, String> {
    Ok(fetch_one(&provider_id, &config, &secrets, &cache, force).await)
}

#[tauri::command]
pub async fn usage_quota_all(
    force: bool,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    cache: State<'_, Arc<QuotaCache>>,
) -> Result<Vec<QuotaResult>, String> {
    let ids: Vec<String> = config.get().providers.iter().map(|p| p.id.clone()).collect();
    let mut out = Vec::with_capacity(ids.len());
    // 逐一查而非 join_all：一次打三個上游端點沒有實質好處，而且逐一查讓
    // 某一家卡住時不會拖累其他家的逾時疊加。
    for id in ids {
        out.push(fetch_one(&id, &config, &secrets, &cache, force).await);
    }
    Ok(out)
}

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
    store: State<'_, Arc<UsageStore>>,
) -> Result<Vec<UsageSummaryEntry>, String> {
    let now = now_secs();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_result_serializes_as_tagged_three_state() {
        // 前端靠 status 欄位分辨三態，壓成兩態會讓本地模型顯示成「查詢失敗」。
        let na = QuotaResult::NotApplicable { provider_id: "ollama-local".into() };
        let j = serde_json::to_value(&na).expect("serialize");
        assert_eq!(j["status"], "not_applicable");
        assert_eq!(j["provider_id"], "ollama-local");

        let f = QuotaResult::Failed {
            provider_id: "GPT5.6".into(),
            message: "AuthFailed".into(),
        };
        let j = serde_json::to_value(&f).expect("serialize");
        assert_eq!(j["status"], "failed");
    }

    #[test]
    fn unknown_model_cost_serializes_as_null_not_zero() {
        // 前端靠 null 顯示「—」。壓成 0 會讓使用者以為這個模型免費。
        let entry = UsageSummaryEntry {
            provider_id: "local".into(),
            model: "some-local-gguf".into(),
            requests: 3,
            prompt_tokens: 100,
            completion_tokens: 50,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            estimated_cost_usd: None,
        };
        let j = serde_json::to_value(&entry).expect("serialize");
        assert!(j["estimated_cost_usd"].is_null());
    }
}

//! 用量/配額相關的 IPC 指令。

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::usage::quota::{cache::QuotaCache, source_for, ProviderQuota};
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
}

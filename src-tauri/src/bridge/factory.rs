//! provider_id → 可用的上游實例。
//!
//! 憑證每個請求重新解析，不快取：Codex 的 access token 300 秒就過期
//! （`ai/router.rs:244`），Antigravity 每請求都要帶 project。M1 雖然還沒接
//! 這兩條路徑，但介面先照這個約束設計，M2/M3 才不用回頭改。

use std::sync::Arc;

use crate::ai::AiError;
use crate::config::types::{ProviderConfig, ProviderType};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

use super::upstream::anthropic::AnthropicUpstream;
use super::upstream::openai::client::OpenAiUpstream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    OpenAi,
    Anthropic,
}

/// M1 支援的上游種類；不支援的回 `None`。
pub fn kind_for(p: &ProviderConfig) -> Option<UpstreamKind> {
    match p.provider_type {
        ProviderType::Openai
        | ProviderType::OpenaiCompatible
        | ProviderType::Ollama
        | ProviderType::Openrouter
        | ProviderType::Deepseek
        | ProviderType::Kimi
        | ProviderType::Xai
        | ProviderType::GithubCopilot => Some(UpstreamKind::OpenAi),

        ProviderType::Anthropic | ProviderType::AnthropicCompatible => Some(UpstreamKind::Anthropic),

        // router.rs:463 的同一個判斷：oauth 走 Antigravity（M3），其餘是
        // OpenAI 相容端點。
        ProviderType::GoogleAi => match p.auth_method.as_deref() {
            Some("oauth") => None,
            _ => Some(UpstreamKind::OpenAi),
        },

        // M2 / M3。
        ProviderType::Codex => None,
    }
}

/// 回給 Claude Code 顯示的訊息，所以要寫成使用者看得懂的句子。
pub fn unsupported_message(p: &ProviderConfig) -> String {
    format!(
        "AITerm 橋接目前還不支援供應商「{}」。請到設定 → Claude Code 橋接改選其他供應商。",
        p.display_name
    )
}

pub enum Upstream {
    OpenAi(OpenAiUpstream),
    Anthropic(AnthropicUpstream),
}

/// 建立上游實例。每個請求呼叫一次。
pub async fn build(
    config: &Arc<ConfigStore>,
    secrets: &Arc<SecretStore>,
    provider_id: &str,
) -> Result<Upstream, AiError> {
    let cfg = config.get();
    // AiError::NotConfigured 是無欄位的 unit variant（見 ai/mod.rs），塞不下
    // 我們想附帶的供應商名稱，所以這裡改用 InvalidInput { reason } 承載訊息
    // ——語意上也貼切：問題出在請求引用了一個不存在/不支援的供應商設定，
    // 不是「完全沒設定」。status_for/error_kind 兩者都把它當 400 處理。
    let p = cfg.find_provider(provider_id).ok_or_else(|| AiError::InvalidInput {
        reason: format!("AITerm 橋接找不到供應商 id「{provider_id}」，它可能已被刪除。"),
    })?;

    let kind = kind_for(p).ok_or_else(|| AiError::InvalidInput {
        reason: unsupported_message(p),
    })?;

    match kind {
        UpstreamKind::OpenAi => {
            let base = p.base_url.clone().unwrap_or_default();
            let key = secrets.get(provider_id).ok().flatten().unwrap_or_default();
            Ok(Upstream::OpenAi(OpenAiUpstream::new(base, key)))
        }
        UpstreamKind::Anthropic => {
            let base = p
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            let is_oauth = p.auth_method.as_deref() == Some("oauth");
            let token = if is_oauth {
                crate::ai::router::get_valid_oauth_token(provider_id, secrets).await?
            } else {
                secrets.get(provider_id).ok().flatten().unwrap_or_default()
            };
            Ok(Upstream::Anthropic(AnthropicUpstream::new(base, token, is_oauth)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{ProviderConfig, ProviderType};

    fn provider(ty: ProviderType, auth: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            id: "p1".into(),
            display_name: "P".into(),
            provider_type: ty,
            base_url: Some("https://x.test".into()),
            oauth_client_id: None,
            model: "m".into(),
            supports_json_mode: false,
            auth_method: auth.map(str::to_string),
        }
    }

    #[test]
    fn openai_family_maps_to_openai_kind() {
        for ty in [
            ProviderType::Openai,
            ProviderType::OpenaiCompatible,
            ProviderType::Ollama,
            ProviderType::Openrouter,
            ProviderType::Deepseek,
            ProviderType::Kimi,
            ProviderType::Xai,
            ProviderType::GithubCopilot,
        ] {
            assert_eq!(kind_for(&provider(ty, None)), Some(UpstreamKind::OpenAi), "{ty:?}");
        }
    }

    #[test]
    fn anthropic_family_maps_to_anthropic_kind() {
        assert_eq!(
            kind_for(&provider(ProviderType::Anthropic, None)),
            Some(UpstreamKind::Anthropic)
        );
        assert_eq!(
            kind_for(&provider(ProviderType::AnthropicCompatible, None)),
            Some(UpstreamKind::Anthropic)
        );
    }

    #[test]
    fn google_ai_splits_on_auth_method() {
        // 沿用 router.rs:463 的判斷：oauth 走 Antigravity，其餘走 OpenAI 相容。
        assert_eq!(
            kind_for(&provider(ProviderType::GoogleAi, None)),
            Some(UpstreamKind::OpenAi)
        );
        assert_eq!(kind_for(&provider(ProviderType::GoogleAi, Some("oauth"))), None);
    }

    #[test]
    fn codex_is_not_supported_in_m1() {
        assert_eq!(kind_for(&provider(ProviderType::Codex, None)), None);
    }

    #[test]
    fn unsupported_message_names_the_provider() {
        let err = unsupported_message(&provider(ProviderType::Codex, None));
        assert!(err.contains("P"), "訊息要含顯示名稱：{err}");
    }
}

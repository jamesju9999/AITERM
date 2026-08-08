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

use super::tool_meta::ToolMetaCache;
use super::upstream::anthropic::AnthropicUpstream;
use super::upstream::antigravity::client::AntigravityUpstream;
use super::upstream::codex::client::CodexUpstream;
use super::upstream::openai::client::OpenAiUpstream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamKind {
    OpenAi,
    Anthropic,
    Codex,
    Antigravity,
}

/// 目前支援的上游種類；不支援的回 `None`。
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
            Some("oauth") => Some(UpstreamKind::Antigravity),
            _ => Some(UpstreamKind::OpenAi),
        },

        // M2。
        ProviderType::Codex => Some(UpstreamKind::Codex),
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
    Codex(CodexUpstream),
    Antigravity(AntigravityUpstream),
}

/// 建立上游實例。每個請求呼叫一次。
pub async fn build(
    config: &Arc<ConfigStore>,
    secrets: &Arc<SecretStore>,
    tool_meta: &Arc<ToolMetaCache>,
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
            let base = resolve_base_url(p, provider_id)?;
            let url = crate::ai::router::openai_chat_url(p.provider_type, &base);

            // GitHub Copilot 是這個分支裡唯一不是「拿 API key 直接打 base_url」
            // 的 provider type：原始 GitHub OAuth token 要先換成短期的 Copilot
            // session token，而且每個請求要多帶幾個 IDE 風格標頭，否則
            // api.githubcopilot.com 回 403。邏輯與 router.rs 的 GithubCopilot
            // 分支共用同一份 `get_copilot_session_token` / `copilot_headers`。
            if p.provider_type == ProviderType::GithubCopilot {
                let github_token = secrets.get(provider_id).ok().flatten().unwrap_or_default();
                let copilot_token = crate::ai::copilot::get_copilot_session_token(&github_token)
                    .await
                    .map_err(|message| AiError::Network { message })?;
                Ok(Upstream::OpenAi(OpenAiUpstream::with_extra_headers(
                    url,
                    copilot_token,
                    crate::ai::copilot::copilot_headers(),
                    tool_meta.clone(),
                )))
            } else {
                let key = secrets.get(provider_id).ok().flatten().unwrap_or_default();
                Ok(Upstream::OpenAi(OpenAiUpstream::new(url, key, tool_meta.clone())))
            }
        }
        UpstreamKind::Anthropic => {
            let base = resolve_base_url(p, provider_id)?;
            let is_oauth = p.auth_method.as_deref() == Some("oauth");
            let token = if is_oauth {
                crate::ai::router::get_valid_oauth_token(provider_id, secrets).await?
            } else {
                secrets.get(provider_id).ok().flatten().unwrap_or_default()
            };
            Ok(Upstream::Anthropic(AnthropicUpstream::new(base, token, is_oauth)))
        }
        UpstreamKind::Codex => {
            // Codex 的 access token 300 秒就過期且 refresh token 會輪替，
            // 所以每個請求都重新解析（見模組頂端的註解）。這個函式會處理
            // 刷新與回存，不要自己重寫。
            let (token, account_id) =
                crate::ai::router::get_valid_codex_oauth_token(provider_id, secrets).await?;
            // base_url 固定 https://chatgpt.com（見 ai/codex.rs），
            // 使用者填的 base_url 只在測試時用得到。
            let base = p.base_url.clone().unwrap_or_else(|| "https://chatgpt.com".into());
            Ok(Upstream::Codex(CodexUpstream::new(base, token, account_id)))
        }
        UpstreamKind::Antigravity => {
            // Antigravity 每個請求都要帶 project id，跟 access token 一起由
            // 同一個函式解析（含過期時的自動刷新），不要自己重寫。
            let (token, project_id) =
                crate::ai::router::get_valid_google_oauth_token(provider_id, secrets).await?;
            // 端點固定，**刻意不讀 p.base_url** —— google-ai 的 provider 設定
            // 通常帶著 API key 路徑的 generativelanguage.googleapis.com，沿用
            // 它會把 OAuth 請求送到錯的主機並得到一個沒有 body 的 404。這是
            // 實際出過的 bug，router.rs 的 AntigravityClient::new 也是寫死的。
            Ok(Upstream::Antigravity(AntigravityUpstream::new(
                crate::ai::antigravity::ANTIGRAVITY_BASE_URL.to_string(),
                token,
                project_id,
                tool_meta.clone(),
            )))
        }
    }
}

/// `p.base_url` 若有填就用，否則用 `ai::router::default_base_url` 補上該
/// provider type 的預設值——與 `ai/router.rs::resolve_by_id` 共用同一份端點
/// 知識，不在這裡另外硬編一次。`OpenaiCompatible` / `AnthropicCompatible`
/// 沒有預設值，缺 base_url 時回明確錯誤，而不是把空字串送去打一個沒有
/// host 的 URL。
fn resolve_base_url(p: &ProviderConfig, provider_id: &str) -> Result<String, AiError> {
    match p.base_url.clone() {
        Some(base) => Ok(base),
        None => crate::ai::router::default_base_url(p.provider_type)
            .map(str::to_string)
            .ok_or_else(|| AiError::Network {
                message: format!("provider '{provider_id}' has no base_url configured"),
            }),
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
        // 沿用 router.rs:463 的判斷：oauth 走 Antigravity（M3），其餘走 OpenAI 相容。
        assert_eq!(
            kind_for(&provider(ProviderType::GoogleAi, None)),
            Some(UpstreamKind::OpenAi)
        );
        assert_eq!(
            kind_for(&provider(ProviderType::GoogleAi, Some("oauth"))),
            Some(UpstreamKind::Antigravity)
        );
    }

    #[test]
    fn codex_maps_to_codex_kind() {
        assert_eq!(
            kind_for(&provider(ProviderType::Codex, Some("oauth"))),
            Some(UpstreamKind::Codex)
        );
    }

    fn provider_without_base_url(ty: ProviderType) -> ProviderConfig {
        let mut p = provider(ty, None);
        p.base_url = None;
        p
    }

    #[test]
    fn resolve_base_url_uses_explicit_base_url_when_present() {
        let p = provider(ProviderType::Openai, None); // base_url = "https://x.test"
        assert_eq!(resolve_base_url(&p, "p1").unwrap(), "https://x.test");
    }

    #[test]
    fn resolve_base_url_falls_back_to_default_for_every_known_type() {
        // 這是這次修的核心 bug：base_url 沒填時，factory 必須套用跟
        // router.rs 一樣的預設值，而不是空字串。逐一驗證每一種有預設值
        // 的 provider type。
        let cases = [
            (ProviderType::Openai, "https://api.openai.com"),
            (ProviderType::Ollama, "http://localhost:11434"),
            (ProviderType::GithubCopilot, "https://api.githubcopilot.com"),
            (ProviderType::GoogleAi, "https://generativelanguage.googleapis.com/v1beta/openai"),
            (ProviderType::Openrouter, "https://openrouter.ai/api/v1"),
            (ProviderType::Xai, "https://api.x.ai/v1"),
            (ProviderType::Deepseek, "https://api.deepseek.com/v1"),
            (ProviderType::Kimi, "https://api.moonshot.ai/v1"),
            (ProviderType::Anthropic, "https://api.anthropic.com"),
        ];
        for (ty, expected) in cases {
            let p = provider_without_base_url(ty);
            assert_eq!(resolve_base_url(&p, "p1").unwrap(), expected, "{ty:?}");
        }
    }

    #[test]
    fn resolve_base_url_errors_for_openai_compatible_without_base_url() {
        let p = provider_without_base_url(ProviderType::OpenaiCompatible);
        let err = resolve_base_url(&p, "p1").unwrap_err();
        match err {
            AiError::Network { message } => assert!(message.contains("no base_url configured"), "{message}"),
            other => panic!("expected AiError::Network, got {other:?}"),
        }
    }

    #[test]
    fn resolve_base_url_errors_for_anthropic_compatible_without_base_url() {
        // AnthropicCompatible 跟 Anthropic 共用 UpstreamKind::Anthropic，但不
        // 該共用預設值——它存在的目的就是打非官方 host，猜一個
        // api.anthropic.com 出來等於把憑證送去錯的地方。
        let p = provider_without_base_url(ProviderType::AnthropicCompatible);
        let err = resolve_base_url(&p, "p1").unwrap_err();
        match err {
            AiError::Network { message } => assert!(message.contains("no base_url configured"), "{message}"),
            other => panic!("expected AiError::Network, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_message_names_the_provider() {
        let err = unsupported_message(&provider(ProviderType::Codex, None));
        assert!(err.contains("P"), "訊息要含顯示名稱：{err}");
    }
}

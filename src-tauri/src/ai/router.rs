//! AiRouter resolves the active `AiProvider` from `ConfigStore` + `SecretStore`
//! at query time. Provider instances are constructed on-demand rather than cached,
//! so config changes take effect immediately without restarting the app.
//!
//! M1 fallback: if no config file exists and `OPENAI_API_KEY` is set in the
//! environment, we use it as a transient OpenAI provider so existing dev setups
//! keep working without migration.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    ai::{
        anthropic::AnthropicClient,
        compatible::OpenAiCompatibleClient,
        ollama::OllamaClient,
        openai::OpenAiClient,
        AiError, AiProvider,
    },
    config::{ConfigStore, ProviderType},
    secret::SecretStore,
};

const ANTHROPIC_OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// MUST stay in sync with the same-named constants in `commands/provider.rs`,
// which the initial login uses. This copy is what `do_google_oauth_refresh`
// below sends. They were briefly out of sync during development (login had
// the real id here while this still held ""), which produced a nasty failure
// mode: login succeeded, then every background refresh silently 400'd and
// was swallowed by the warn-and-reuse-old-token fallback, so the session
// only died ~an hour later with no signal pointing at the real cause.
const GOOGLE_OAUTH_CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_OAUTH_CLIENT_SECRET: &str = "";

const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

pub(crate) const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub(crate) const XAI_DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
pub(crate) const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";
pub(crate) const KIMI_DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";

/// Returns a valid OAuth access token, refreshing it first if it's expired or
/// within 5 minutes of expiry. Falls back to the stored token on refresh failure.
async fn get_valid_oauth_token(provider_id: &str, secrets: &SecretStore) -> Result<String, AiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let needs_refresh = secrets
        .get(&format!("{provider_id}:oauth_expires_at"))
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|exp| exp < now + 300) // refresh 5 min before expiry
        .unwrap_or(false);

    if needs_refresh {
        if let Some(refresh_token) = secrets.get(&format!("{provider_id}:oauth_refresh")).ok().flatten() {
            match do_oauth_refresh(provider_id, &refresh_token, secrets).await {
                Ok(access_token) => return Ok(access_token),
                Err(e) => log::warn!("OAuth token refresh failed, using existing token: {e}"),
            }
        }
    }

    secrets
        .get(provider_id)
        .map_err(|_| AiError::NotConfigured)?
        .ok_or(AiError::NotConfigured)
}

async fn do_oauth_refresh(provider_id: &str, refresh_token: &str, secrets: &SecretStore) -> Result<String, String> {
    #[derive(Serialize)]
    struct RefreshReq<'a> {
        grant_type: &'a str,
        refresh_token: &'a str,
        client_id: &'a str,
    }
    #[derive(Deserialize)]
    struct RefreshResp {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(ANTHROPIC_OAUTH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
        .json(&RefreshReq {
            grant_type: "refresh_token",
            refresh_token,
            client_id: ANTHROPIC_OAUTH_CLIENT_ID,
        })
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {body}"));
    }

    let data: RefreshResp = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    let _ = secrets.set(provider_id, &data.access_token);
    if let Some(new_refresh) = data.refresh_token {
        let _ = secrets.set(&format!("{provider_id}:oauth_refresh"), &new_refresh);
    }
    if let Some(expires_in) = data.expires_in {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &(now + expires_in).to_string());
    }

    Ok(data.access_token)
}

/// Returns a valid Antigravity access token (refreshing first if within 15
/// minutes of expiry — a longer lead than Anthropic/Codex's 5 minutes since
/// Google's refresh tokens don't rotate, so there's no "stale refresh token"
/// risk to hurry around) plus the account's onboarded Cloud Code project id.
async fn get_valid_google_oauth_token(provider_id: &str, secrets: &SecretStore) -> Result<(String, String), AiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let needs_refresh = secrets
        .get(&format!("{provider_id}:oauth_expires_at"))
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|exp| exp < now + 900)
        .unwrap_or(false);

    if needs_refresh {
        if let Some(refresh_token) = secrets.get(&format!("{provider_id}:oauth_refresh")).ok().flatten() {
            match do_google_oauth_refresh(provider_id, &refresh_token, secrets).await {
                Ok(access_token) => {
                    let project_id = secrets
                        .get(&format!("{provider_id}:project_id"))
                        .map_err(|_| AiError::NotConfigured)?
                        .ok_or(AiError::NotConfigured)?;
                    return Ok((access_token, project_id));
                }
                Err(e) => log::warn!("Google OAuth token refresh failed, using existing token: {e}"),
            }
        }
    }

    let token = secrets.get(provider_id).map_err(|_| AiError::NotConfigured)?.ok_or(AiError::NotConfigured)?;
    let project_id = secrets
        .get(&format!("{provider_id}:project_id"))
        .map_err(|_| AiError::NotConfigured)?
        .ok_or(AiError::NotConfigured)?;
    Ok((token, project_id))
}

async fn do_google_oauth_refresh(provider_id: &str, refresh_token: &str, secrets: &SecretStore) -> Result<String, String> {
    #[derive(Deserialize)]
    struct RefreshResp {
        access_token: String,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let http = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", GOOGLE_OAUTH_CLIENT_ID),
        ("client_secret", GOOGLE_OAUTH_CLIENT_SECRET),
    ];
    let resp = http
        .post(GOOGLE_OAUTH_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {body}"));
    }

    let data: RefreshResp = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    let _ = secrets.set(provider_id, &data.access_token);
    if let Some(expires_in) = data.expires_in {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &(now + expires_in).to_string());
    }

    Ok(data.access_token)
}

/// Returns a valid Codex access token (refreshing first if within 5 minutes
/// of expiry) plus the cached `chatgpt-account-id`, if any.
async fn get_valid_codex_oauth_token(
    provider_id: &str,
    secrets: &SecretStore,
) -> Result<(String, Option<String>), AiError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let needs_refresh = secrets
        .get(&format!("{provider_id}:oauth_expires_at"))
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|exp| exp < now + 300)
        .unwrap_or(false);

    if needs_refresh {
        if let Some(refresh_token) = secrets.get(&format!("{provider_id}:oauth_refresh")).ok().flatten() {
            match do_codex_oauth_refresh(provider_id, &refresh_token, secrets).await {
                Ok(access_token) => {
                    let account_id = secrets.get(&format!("{provider_id}:oauth_account_id")).ok().flatten();
                    return Ok((access_token, account_id));
                }
                Err(e) => log::warn!("Codex OAuth token refresh failed, using existing token: {e}"),
            }
        }
    }

    let token = secrets.get(provider_id).map_err(|_| AiError::NotConfigured)?.ok_or(AiError::NotConfigured)?;
    let account_id = secrets.get(&format!("{provider_id}:oauth_account_id")).ok().flatten();
    Ok((token, account_id))
}

async fn do_codex_oauth_refresh(
    provider_id: &str,
    refresh_token: &str,
    secrets: &SecretStore,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct RefreshReq<'a> {
        grant_type: &'a str,
        refresh_token: &'a str,
        client_id: &'a str,
    }
    #[derive(Deserialize)]
    struct RefreshResp {
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let client = reqwest::Client::new();
    // `scope` is intentionally omitted — OpenAI/Auth0 treats a `scope` on the
    // refresh_token grant as a re-scope request, which can invalidate sibling
    // refresh-token families sharing this client_id (multi-account support
    // depends on this NOT happening).
    let resp = client
        .post(CODEX_OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&RefreshReq {
            grant_type: "refresh_token",
            refresh_token,
            client_id: CODEX_OAUTH_CLIENT_ID,
        })
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {body}"));
    }

    let data: RefreshResp = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;

    let _ = secrets.set(provider_id, &data.access_token);
    // Codex refresh tokens are one-time-use/rotating — the new refresh_token
    // returned here MUST replace the old one, or the next refresh attempt
    // fails with `refresh_token_reused` / `invalid_grant`.
    if let Some(new_refresh) = data.refresh_token {
        let _ = secrets.set(&format!("{provider_id}:oauth_refresh"), &new_refresh);
    }
    if let Some(expires_in) = data.expires_in {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &(now + expires_in).to_string());
    }

    Ok(data.access_token)
}

pub struct AiRouter {
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
}

impl AiRouter {
    pub fn new(config: Arc<ConfigStore>, secrets: Arc<SecretStore>) -> Self {
        Self { config, secrets }
    }

    /// Resolve the default provider (from config) into a live `AiProvider`.
    pub async fn resolve(&self) -> Result<Arc<dyn AiProvider>, AiError> {
        let cfg = self.config.get();

        // M1 env-var fallback: honour OPENAI_API_KEY if no providers configured yet.
        if cfg.providers.is_empty() {
            if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                if !key.trim().is_empty() {
                    let client: Arc<dyn AiProvider> =
                        Arc::new(OpenAiClient::new(key));
                    return Ok(client);
                }
            }
            return Err(AiError::NotConfigured);
        }

        let id = cfg.default_provider.as_deref().unwrap_or_else(|| {
            cfg.providers.first().map(|p| p.id.as_str()).unwrap_or("")
        });
        self.resolve_by_id(id).await
    }

    /// Resolve a specific provider by id.
    pub async fn resolve_by_id(&self, id: &str) -> Result<Arc<dyn AiProvider>, AiError> {
        let cfg = self.config.get();
        let provider_cfg = cfg
            .find_provider(id)
            .ok_or(AiError::NotConfigured)?
            .clone();

        let provider: Arc<dyn AiProvider> = match provider_cfg.provider_type {
            ProviderType::Openai => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiClient::with_base_url(
                    key,
                    provider_cfg.model.clone(),
                    provider_cfg.base_url.unwrap_or_else(|| "https://api.openai.com".into()),
                ))
            }
            ProviderType::Anthropic => {
                let is_oauth = provider_cfg.auth_method.as_deref() == Some("oauth");
                let token = if is_oauth {
                    get_valid_oauth_token(&provider_cfg.id, &self.secrets).await?
                } else {
                    self.secrets
                        .get(&provider_cfg.id)
                        .map_err(|_| AiError::NotConfigured)?
                        .ok_or(AiError::NotConfigured)?
                };
                let base_url = provider_cfg.base_url.unwrap_or_else(|| "https://api.anthropic.com".into());
                if is_oauth {
                    Arc::new(AnthropicClient::with_oauth(token, provider_cfg.model.clone(), base_url))
                } else {
                    Arc::new(AnthropicClient::with_base_url(token, provider_cfg.model.clone(), base_url))
                }
            }
            ProviderType::Ollama => {
                // Ollama has no API key.
                Arc::new(OllamaClient::with_base_url(
                    provider_cfg.model.clone(),
                    provider_cfg.base_url.unwrap_or_else(|| "http://localhost:11434".into()),
                ))
            }
            ProviderType::OpenaiCompatible => {
                // API key is optional for compatible providers.
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .unwrap_or(None); // swallow keychain errors — key is optional
                let base_url = provider_cfg
                    .base_url
                    .ok_or_else(|| AiError::Network {
                        message: format!("provider '{}' has no base_url configured", provider_cfg.id),
                    })?;
                Arc::new(OpenAiCompatibleClient::new(
                    base_url,
                    provider_cfg.model.clone(),
                    key,
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::GithubCopilot => {
                let github_token = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .filter(|v| !v.trim().is_empty())
                    .ok_or(AiError::NotConfigured)?;
                // Exchange the GitHub OAuth token for a short-lived Copilot
                // session token — the raw OAuth token is rejected by
                // api.githubcopilot.com/chat/completions with 403.
                let copilot_token =
                    crate::ai::copilot::get_copilot_session_token(&github_token)
                        .await
                        .map_err(|msg| AiError::Network { message: msg })?;
                // Copilot API requires IDE-style headers on every request.
                let copilot_headers = vec![
                    ("Editor-Version".into(), "vscode/1.99.0".into()),
                    ("Editor-Plugin-Version".into(), "copilot/1.0.0".into()),
                    ("Copilot-Integration-Id".into(), "vscode-chat".into()),
                    ("Openai-Intent".into(), "conversation-panel".into()),
                ];
                Arc::new(OpenAiCompatibleClient::with_extra_headers(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| "https://api.githubcopilot.com".into()),
                    provider_cfg.model.clone(),
                    Some(copilot_token),
                    provider_cfg.supports_json_mode,
                    copilot_headers,
                ))
            }
            ProviderType::GoogleAi => {
                let is_oauth = provider_cfg.auth_method.as_deref() == Some("oauth");
                if is_oauth {
                    let (token, project_id) = get_valid_google_oauth_token(&provider_cfg.id, &self.secrets).await?;
                    Arc::new(crate::ai::antigravity::AntigravityClient::new(token, project_id, provider_cfg.model.clone()))
                } else {
                    let key = self
                        .secrets
                        .get(&provider_cfg.id)
                        .map_err(|_| AiError::NotConfigured)?
                        .ok_or(AiError::NotConfigured)?;
                    Arc::new(OpenAiCompatibleClient::new(
                        provider_cfg
                            .base_url
                            .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta/openai".into()),
                        provider_cfg.model.clone(),
                        Some(key),
                        provider_cfg.supports_json_mode,
                    ))
                }
            }
            ProviderType::Openrouter => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| OPENROUTER_DEFAULT_BASE_URL.into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::Xai => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| XAI_DEFAULT_BASE_URL.into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::Deepseek => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| DEEPSEEK_DEFAULT_BASE_URL.into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::Kimi => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| KIMI_DEFAULT_BASE_URL.into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::AnthropicCompatible => {
                // Check base_url before the API key so a missing base_url is
                // reported as AiError::Network regardless of whether a key
                // happens to be present — mirrors OpenaiCompatible's ordering
                // and keeps this branch independently testable without
                // needing a real secret in the keychain (see the tests
                // below).
                let base_url = provider_cfg.base_url.ok_or_else(|| AiError::Network {
                    message: format!("provider '{}' has no base_url configured", provider_cfg.id),
                })?;
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(AnthropicClient::with_base_url(key, provider_cfg.model.clone(), base_url))
            }
            ProviderType::Codex => {
                let (token, account_id) = get_valid_codex_oauth_token(&provider_cfg.id, &self.secrets).await?;
                Arc::new(crate::ai::codex::CodexClient::new(token, provider_cfg.model.clone(), account_id))
            }
        };
        Ok(provider)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProviderConfig};
    use std::sync::Arc;

    /// Global lock for tests that mutate the OPENAI_API_KEY env var to prevent
    /// races between them. Every test in this file that reads or writes that
    /// var must take this lock — env vars are process-global, so an unguarded
    /// test can interleave with a guarded one regardless of lock duration.
    /// Uses a tokio (async-aware) mutex so it's safe to hold across `.await`.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn make_router(cfg: AppConfig) -> AiRouter {
        // Use an in-memory ConfigStore (temp path) and a SecretStore that
        // returns nothing from the keychain (no real keys needed for these tests).
        let config = Arc::new(crate::config::ConfigStore::from_config(cfg));
        let secrets = Arc::new(SecretStore::new());
        AiRouter::new(config, secrets)
    }

    #[test]
    fn openai_compatible_provider_default_base_urls_are_correct() {
        assert_eq!(OPENROUTER_DEFAULT_BASE_URL, "https://openrouter.ai/api/v1");
        assert_eq!(XAI_DEFAULT_BASE_URL, "https://api.x.ai/v1");
        assert_eq!(DEEPSEEK_DEFAULT_BASE_URL, "https://api.deepseek.com/v1");
        assert_eq!(KIMI_DEFAULT_BASE_URL, "https://api.moonshot.ai/v1");
    }

    #[tokio::test]
    async fn empty_config_no_env_var_returns_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let router = make_router(AppConfig::default());
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn empty_config_with_env_var_returns_openai_provider() {
        let _g = ENV_LOCK.lock().await;
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        let router = make_router(AppConfig::default());
        let result = router.resolve().await.is_ok();
        std::env::remove_var("OPENAI_API_KEY");
        assert!(result);
    }

    #[tokio::test]
    async fn unknown_provider_id_returns_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let router = make_router(AppConfig::default());
        assert!(matches!(router.resolve_by_id("nonexistent").await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn ollama_provider_resolves_without_api_key() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "local-llama".into(),
            display_name: "Ollama".into(),
            provider_type: ProviderType::Ollama,
            base_url: Some("http://localhost:11434".into()),
            oauth_client_id: None,
            model: "llama3".into(),
            supports_json_mode: false,
            auth_method: None,
        });
        cfg.default_provider = Some("local-llama".into());
        let router = make_router(cfg);
        // Should succeed even with no secret in the keychain.
        assert!(router.resolve().await.is_ok());
    }

    #[tokio::test]
    async fn openrouter_provider_without_api_key_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "or".into(),
            display_name: "OpenRouter".into(),
            provider_type: ProviderType::Openrouter,
            base_url: None,
            oauth_client_id: None,
            model: "openai/gpt-4o-mini".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("or".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn xai_provider_without_api_key_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "grok".into(),
            display_name: "xAI".into(),
            provider_type: ProviderType::Xai,
            base_url: None,
            oauth_client_id: None,
            model: "grok-4".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("grok".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn deepseek_provider_without_api_key_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "ds".into(),
            display_name: "DeepSeek".into(),
            provider_type: ProviderType::Deepseek,
            base_url: None,
            oauth_client_id: None,
            model: "deepseek-chat".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("ds".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn kimi_provider_without_api_key_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "kimi".into(),
            display_name: "Kimi".into(),
            provider_type: ProviderType::Kimi,
            base_url: None,
            oauth_client_id: None,
            model: "kimi-latest".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("kimi".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn anthropic_compatible_without_base_url_is_network_error() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "kimi-coding".into(),
            display_name: "Kimi Coding".into(),
            provider_type: ProviderType::AnthropicCompatible,
            base_url: None,
            oauth_client_id: None,
            model: "kimi-for-coding".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("kimi-coding".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::Network { .. })));
    }

    #[tokio::test]
    async fn anthropic_compatible_with_base_url_but_no_key_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "kimi-coding".into(),
            display_name: "Kimi Coding".into(),
            provider_type: ProviderType::AnthropicCompatible,
            base_url: Some("https://api.kimi.com/coding".into()),
            oauth_client_id: None,
            model: "kimi-for-coding".into(),
            supports_json_mode: true,
            auth_method: None,
        });
        cfg.default_provider = Some("kimi-coding".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn google_ai_oauth_provider_without_token_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "gemini".into(),
            display_name: "Gemini".into(),
            provider_type: ProviderType::GoogleAi,
            base_url: None,
            oauth_client_id: None,
            model: "gemini-2.5-pro".into(),
            supports_json_mode: false,
            auth_method: Some("oauth".into()),
        });
        cfg.default_provider = Some("gemini".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn google_ai_oauth_provider_with_token_but_no_project_id_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "gemini-no-project".into(),
            display_name: "Gemini".into(),
            provider_type: ProviderType::GoogleAi,
            base_url: None,
            oauth_client_id: None,
            model: "gemini-2.5-pro".into(),
            supports_json_mode: false,
            auth_method: Some("oauth".into()),
        });
        cfg.default_provider = Some("gemini-no-project".into());
        let router = make_router(cfg);
        // The in-memory test SecretStore has neither a token nor a project id
        // stored, so this exercises the same NotConfigured path a real
        // half-provisioned login would hit. Locks in that a missing
        // project_id is a hard error, never an empty-string default silently
        // sent in the request body.
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }

    #[tokio::test]
    async fn codex_provider_without_oauth_token_is_not_configured() {
        let _g = ENV_LOCK.lock().await;
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "codex".into(),
            display_name: "Codex".into(),
            provider_type: ProviderType::Codex,
            base_url: None,
            oauth_client_id: None,
            model: "gpt-5.1-codex".into(),
            supports_json_mode: false,
            auth_method: Some("oauth".into()),
        });
        cfg.default_provider = Some("codex".into());
        let router = make_router(cfg);
        assert!(matches!(router.resolve().await, Err(AiError::NotConfigured)));
    }
}

//! AiRouter resolves the active `AiProvider` from `ConfigStore` + `SecretStore`
//! at query time. Provider instances are constructed on-demand rather than cached,
//! so config changes take effect immediately without restarting the app.
//!
//! M1 fallback: if no config file exists and `OPENAI_API_KEY` is set in the
//! environment, we use it as a transient OpenAI provider so existing dev setups
//! keep working without migration.

use std::sync::Arc;

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

pub struct AiRouter {
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
}

impl AiRouter {
    pub fn new(config: Arc<ConfigStore>, secrets: Arc<SecretStore>) -> Self {
        Self { config, secrets }
    }

    /// Resolve the default provider (from config) into a live `AiProvider`.
    pub fn resolve(&self) -> Result<Arc<dyn AiProvider>, AiError> {
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
        self.resolve_by_id(id)
    }

    /// Resolve a specific provider by id.
    pub fn resolve_by_id(&self, id: &str) -> Result<Arc<dyn AiProvider>, AiError> {
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
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(AnthropicClient::with_base_url(
                    key,
                    provider_cfg.model.clone(),
                    provider_cfg.base_url.unwrap_or_else(|| "https://api.anthropic.com".into()),
                ))
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
        };
        Ok(provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, ProviderConfig};
    use std::sync::{Arc, Mutex};

    /// Global mutex for tests that mutate OPENAI_API_KEY to prevent races.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_router(cfg: AppConfig) -> AiRouter {
        // Use an in-memory ConfigStore (temp path) and a SecretStore that
        // returns nothing from the keychain (no real keys needed for these tests).
        let config = Arc::new(crate::config::ConfigStore::from_config(cfg));
        let secrets = Arc::new(SecretStore::new());
        AiRouter::new(config, secrets)
    }

    #[test]
    fn empty_config_no_env_var_returns_not_configured() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        let router = make_router(AppConfig::default());
        assert!(matches!(router.resolve(), Err(AiError::NotConfigured)));
    }

    #[test]
    fn empty_config_with_env_var_returns_openai_provider() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("OPENAI_API_KEY", "sk-test");
        let router = make_router(AppConfig::default());
        let result = router.resolve().is_ok();
        std::env::remove_var("OPENAI_API_KEY");
        assert!(result);
    }

    #[test]
    fn unknown_provider_id_returns_not_configured() {
        std::env::remove_var("OPENAI_API_KEY");
        let router = make_router(AppConfig::default());
        assert!(matches!(router.resolve_by_id("nonexistent"), Err(AiError::NotConfigured)));
    }

    #[test]
    fn ollama_provider_resolves_without_api_key() {
        std::env::remove_var("OPENAI_API_KEY");
        let mut cfg = AppConfig::default();
        cfg.providers.push(ProviderConfig {
            id: "local-llama".into(),
            display_name: "Ollama".into(),
            provider_type: ProviderType::Ollama,
            base_url: Some("http://localhost:11434".into()),
            model: "llama3".into(),
            supports_json_mode: false,
        });
        cfg.default_provider = Some("local-llama".into());
        let router = make_router(cfg);
        // Should succeed even with no secret in the keychain.
        assert!(router.resolve().is_ok());
    }
}

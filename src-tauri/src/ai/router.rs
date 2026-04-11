//! In M1 the router is a thin holder for a single provider (or a
//! NotConfigured error captured at startup so the app can still boot).
//! M2 will expand this to pick a provider by id.

use std::sync::Arc;

use crate::ai::{openai::OpenAiClient, AiError, AiProvider};

pub struct AiRouter {
    provider: Result<Arc<dyn AiProvider>, AiError>,
}

impl AiRouter {
    /// Try to build the default provider from environment. If the env var
    /// is missing, capture the error and defer reporting until the user
    /// actually triggers `/ai`. Spec §7.3.
    pub fn from_env() -> Self {
        let result = match std::env::var("OPENAI_API_KEY") {
            Ok(key) if !key.trim().is_empty() => {
                let client: Arc<dyn AiProvider> = Arc::new(OpenAiClient::new(key));
                Ok(client)
            }
            _ => Err(AiError::NotConfigured),
        };
        Self { provider: result }
    }

    pub fn with_provider(p: Arc<dyn AiProvider>) -> Self {
        Self { provider: Ok(p) }
    }

    /// Get the provider, or surface the captured error.
    pub fn require_provider(&self) -> Result<Arc<dyn AiProvider>, AiError> {
        self.provider.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_var_produces_not_configured() {
        std::env::remove_var("OPENAI_API_KEY");
        let router = AiRouter::from_env();
        match router.require_provider() {
            Err(AiError::NotConfigured) => {}
            other => panic!("expected NotConfigured, got something else: {:?}", other.err()),
        }
    }

    #[test]
    fn empty_env_var_produces_not_configured() {
        std::env::set_var("OPENAI_API_KEY", "   ");
        let router = AiRouter::from_env();
        assert!(matches!(router.require_provider(), Err(AiError::NotConfigured)));
        std::env::remove_var("OPENAI_API_KEY");
    }

    #[test]
    fn present_env_var_produces_ok() {
        std::env::set_var("OPENAI_API_KEY", "fake-key");
        let router = AiRouter::from_env();
        assert!(router.require_provider().is_ok());
        std::env::remove_var("OPENAI_API_KEY");
    }
}

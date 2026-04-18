//! Tauri commands for provider management (list / add / update / remove / test).

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    ai::{AiError, router::AiRouter},
    config::{ConfigStore, ProviderConfig, ProviderType},
    secret::SecretStore,
};

/// A view of a provider suitable for the frontend — never includes secrets.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub model: String,
    pub supports_json_mode: bool,
    pub has_api_key: bool,
    pub is_default: bool,
}

/// Input payload for adding or updating a provider.
#[derive(Debug, Deserialize)]
pub struct ProviderInput {
    pub id: String,
    pub display_name: String,
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub model: String,
    pub supports_json_mode: bool,
    /// The API key to store in the keychain. `None` means "don't change".
    pub api_key: Option<String>,
}

#[tauri::command]
pub fn list_providers(
    config: State<Arc<ConfigStore>>,
    secrets: State<Arc<SecretStore>>,
) -> Vec<ProviderInfo> {
    let cfg = config.get();
    cfg.providers
        .iter()
        .map(|p| ProviderInfo {
            id: p.id.clone(),
            display_name: p.display_name.clone(),
            provider_type: p.provider_type,
            base_url: p.base_url.clone(),
            model: p.model.clone(),
            supports_json_mode: p.supports_json_mode,
            has_api_key: secrets.has(&p.id),
            is_default: cfg.default_provider.as_deref() == Some(&p.id),
        })
        .collect()
}

#[tauri::command]
pub fn add_provider(
    input: ProviderInput,
    config: State<Arc<ConfigStore>>,
    secrets: State<Arc<SecretStore>>,
) -> Result<(), String> {
    // Validate: id must be unique.
    {
        let cfg = config.get();
        if cfg.find_provider(&input.id).is_some() {
            return Err(format!("Provider id '{}' already exists", input.id));
        }
    }

    let provider_cfg = ProviderConfig {
        id: input.id.clone(),
        display_name: input.display_name,
        provider_type: input.provider_type,
        base_url: input.base_url,
        model: input.model,
        supports_json_mode: input.supports_json_mode,
    };

    config
        .update(|cfg| {
            cfg.upsert_provider(provider_cfg);
            // Set as default if it's the first provider.
            if cfg.providers.len() == 1 {
                cfg.default_provider = Some(input.id.clone());
            }
        })
        .map_err(|e| e.to_string())?;

    if let Some(key) = input.api_key {
        secrets.set(&input.id, &key).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn update_provider(
    input: ProviderInput,
    config: State<Arc<ConfigStore>>,
    secrets: State<Arc<SecretStore>>,
) -> Result<(), String> {
    let provider_cfg = ProviderConfig {
        id: input.id.clone(),
        display_name: input.display_name,
        provider_type: input.provider_type,
        base_url: input.base_url,
        model: input.model,
        supports_json_mode: input.supports_json_mode,
    };

    config
        .update(|cfg| { cfg.upsert_provider(provider_cfg); })
        .map_err(|e| e.to_string())?;

    if let Some(key) = input.api_key {
        secrets.set(&input.id, &key).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn remove_provider(
    id: String,
    config: State<Arc<ConfigStore>>,
    secrets: State<Arc<SecretStore>>,
) -> Result<(), String> {
    config
        .update(|cfg| { cfg.remove_provider(&id); })
        .map_err(|e| e.to_string())?;

    // Best-effort keychain cleanup.
    let _ = secrets.delete(&id);
    Ok(())
}

#[tauri::command]
pub fn set_default_provider(
    id: String,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config
        .update(|cfg| {
            // Only set if the provider actually exists.
            if cfg.find_provider(&id).is_some() {
                cfg.default_provider = Some(id);
            }
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, AiError> {
    let router = AiRouter::new(config.inner().clone(), secrets.inner().clone());
    let provider = router.resolve_by_id(&id)?;
    provider.health_check().await?;
    Ok("ok".into())
}

#[tauri::command]
pub async fn get_ollama_models(
    base_url: Option<String>,
    _config: State<'_, Arc<ConfigStore>>,
) -> Result<Vec<String>, String> {
    use crate::ai::ollama::OllamaClient;
    let url = base_url.unwrap_or_else(|| "http://localhost:11434".into());
    let client = OllamaClient::with_base_url("".into(), url);
    client.list_models().await.map_err(|e| e.to_string())
}

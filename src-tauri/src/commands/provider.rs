//! Tauri commands for provider management (list / add / update / remove / test).

use std::sync::Arc;
use std::process::Command;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    ai::{AiError, router::AiRouter},
    config::{ConfigStore, ProviderConfig, ProviderType},
    secret::SecretStore,
};

const DEFAULT_GITHUB_DEVICE_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// A view of a provider suitable for the frontend — never includes secrets.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub provider_type: ProviderType,
    pub base_url: Option<String>,
    pub oauth_client_id: Option<String>,
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
    pub oauth_client_id: Option<String>,
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
            oauth_client_id: p.oauth_client_id.clone(),
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
        oauth_client_id: input.oauth_client_id,
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
        oauth_client_id: input.oauth_client_id,
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
    let provider_cfg = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or(AiError::NotConfigured)?;

    // Copilot's chat/completions endpoint may reject probe-style requests
    // with intermittent 403 policy responses. For connectivity testing,
    // verify auth by listing models with the saved token instead.
    if provider_cfg.provider_type == ProviderType::GithubCopilot {
        let token = secrets
            .get(&id)
            .map_err(|e| AiError::Network { message: format!("keychain read failed: {e}") })?
            .filter(|v| !v.trim().is_empty())
            .ok_or(AiError::NotConfigured)?;
        list_github_copilot_models(provider_cfg.base_url, token.trim())
            .await
            .map_err(|e| AiError::Network { message: e })?;
        return Ok("ok".into());
    }

    let router = AiRouter::new(config.inner().clone(), secrets.inner().clone());
    let provider = router.resolve_by_id(&id).await?;
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

#[tauri::command]
pub async fn github_copilot_device_start(
    client_id: Option<String>,
) -> Result<GithubDeviceStartResponse, String> {
    let client_id = resolve_github_client_id(client_id);
    let client = reqwest::Client::new();

    let device = client
        .post("https://github.com/login/device/code")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id.as_str()),
            ("scope", "read:user"),
        ])
        .send()
        .await
        .map_err(|e| format!("request device code failed: {e}"))?;

    if !device.status().is_success() {
        let body = device.text().await.unwrap_or_default();
        return Err(format!("request device code failed: {body}"));
    }

    let device: GithubDeviceCodeResponse = device
        .json()
        .await
        .map_err(|e| format!("parse device code response failed: {e}"))?;

    open_browser(&device.verification_uri);

    Ok(GithubDeviceStartResponse {
        device_code: device.device_code,
        user_code: device.user_code,
        verification_uri: device.verification_uri,
        expires_in: device.expires_in,
        interval: device.interval.max(1),
    })
}

#[tauri::command]
pub async fn github_copilot_device_poll(
    client_id: Option<String>,
    device_code: String,
) -> Result<GithubDevicePollResponse, String> {
    let client_id = resolve_github_client_id(client_id);
    let device_code = device_code.trim().to_string();
    if device_code.is_empty() {
        return Err("device_code is required".into());
    }

    let client = reqwest::Client::new();
    let token_resp = client
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", client_id.as_str()),
            ("device_code", device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("poll access token failed: {e}"))?;

    if !token_resp.status().is_success() {
        let body = token_resp.text().await.unwrap_or_default();
        return Err(format!("poll access token failed: {body}"));
    }

    let payload: GithubAccessTokenResponse = token_resp
        .json()
        .await
        .map_err(|e| format!("parse access token response failed: {e}"))?;

    if let Some(token) = payload.access_token {
        if token.trim().is_empty() {
            return Err("received empty access token from GitHub".into());
        }
        return Ok(GithubDevicePollResponse::Authorized { access_token: token });
    }

    Ok(match payload.error.as_deref() {
        Some("authorization_pending") => GithubDevicePollResponse::AuthorizationPending,
        Some("slow_down") => GithubDevicePollResponse::SlowDown,
        Some("access_denied") => GithubDevicePollResponse::AccessDenied {
            message: payload
                .error_description
                .unwrap_or_else(|| "authorization denied by user".into()),
        },
        Some("expired_token") | Some("token_expired") => GithubDevicePollResponse::ExpiredToken {
            message: payload
                .error_description
                .unwrap_or_else(|| "device code expired".into()),
        },
        Some(other) => GithubDevicePollResponse::Error {
            message: format!(
                "{other}: {}",
                payload.error_description.unwrap_or_default()
            ),
        },
        None => GithubDevicePollResponse::Error {
            message: "device flow returned no token and no error".into(),
        },
    })
}

#[tauri::command]
pub async fn get_github_copilot_models(
    base_url: Option<String>,
    access_token: String,
) -> Result<Vec<String>, String> {
    let token = access_token.trim();
    if token.is_empty() {
        return Err("access token is required".into());
    }

    list_github_copilot_models(base_url, token).await
}

#[tauri::command]
pub async fn get_github_copilot_models_by_provider(
    id: String,
    base_url: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::GithubCopilot {
        return Err(format!("provider '{id}' is not github-copilot"));
    }
    let token = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved access token"))?;

    list_github_copilot_models(base_url, token.trim()).await
}

async fn list_github_copilot_models(
    base_url: Option<String>,
    access_token: &str,
) -> Result<Vec<String>, String> {
    let token = access_token.trim();
    if token.is_empty() {
        return Err("access token is required".into());
    }

    let base = base_url
        .unwrap_or_else(|| "https://api.githubcopilot.com".into())
        .trim_end_matches('/')
        .to_string();
    let url = format!("{base}/models");

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("list github copilot models failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("list github copilot models failed ({status}): {body}"));
    }

    let payload: OpenAiModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse github copilot models failed: {e}"))?;

    Ok(payload.data.into_iter().map(|m| m.id).collect())
}

/// Fetch available Google AI (Gemini) models using an API key supplied directly
/// (used while the user is typing the key before saving the provider).
#[tauri::command]
pub async fn get_google_ai_models(api_key: String) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("api_key is required".into());
    }
    list_google_ai_models(key).await
}

/// Fetch available Google AI models for a saved provider (reads key from keychain).
#[tauri::command]
pub async fn get_google_ai_models_by_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::GoogleAi {
        return Err(format!("provider '{id}' is not google-ai"));
    }
    let key = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved API key"))?;
    list_google_ai_models(key.trim()).await
}

async fn list_google_ai_models(api_key: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct GoogleModelsResponse {
        models: Vec<GoogleModelItem>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoogleModelItem {
        name: String,
        #[serde(default)]
        supported_generation_methods: Vec<String>,
    }

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models?key={api_key}"
    );
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("list google ai models failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("list google ai models failed ({status}): {body}"));
    }

    let payload: GoogleModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse google ai models response failed: {e}"))?;

    let mut models: Vec<String> = payload
        .models
        .into_iter()
        .filter(|m| m.supported_generation_methods.iter().any(|method| method == "generateContent"))
        .map(|m| m.name.trim_start_matches("models/").to_string())
        .collect();
    models.sort();
    Ok(models)
}

fn resolve_github_client_id(client_id: Option<String>) -> String {
    if let Some(v) = client_id {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(v) = std::env::var("GITHUB_DEVICE_CLIENT_ID") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    DEFAULT_GITHUB_DEVICE_CLIENT_ID.to_string()
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}


#[derive(Debug, Serialize)]
pub struct GithubDeviceStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u32,
    pub interval: u64,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GithubDevicePollResponse {
    Authorized { access_token: String },
    AuthorizationPending,
    SlowDown,
    AccessDenied { message: String },
    ExpiredToken { message: String },
    Error { message: String },
}

#[derive(Debug, Deserialize)]
struct GithubDeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u32,
    interval: u64,
}

#[derive(Debug, Deserialize)]
struct GithubAccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
}

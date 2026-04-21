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

/// Open the browser for the Google OAuth loopback flow (using bundled Gemini CLI
/// credentials), wait for the callback, exchange the code for tokens, and store
/// the JSON token blob in the keychain.
#[tauri::command]
pub async fn google_gemini_oauth_auth(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    use crate::ai::google_oauth::{
        GEMINI_CLI_CLIENT_ID, GEMINI_CLI_CLIENT_SECRET, GEMINI_OAUTH_SCOPES, GoogleOAuthToken,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Bind a local loopback server on a random available port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to start local server: {e}"))?;
    let port = listener.local_addr().unwrap().port();
    let redirect_uri = format!("http://localhost:{port}");

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
        url_encode(GEMINI_CLI_CLIENT_ID),
        url_encode(&redirect_uri),
        url_encode(GEMINI_OAUTH_SCOPES),
    );

    open_browser(&auth_url);

    // Wait for the browser redirect (up to 2 minutes).
    let code = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        async {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("accept error: {e}"))?;

            let mut buf = vec![0u8; 8192];
            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| format!("read error: {e}"))?;
            let request = String::from_utf8_lossy(&buf[..n]).to_string();

            let body = "<html><body style='font-family:sans-serif;text-align:center;padding:40px'>\
                <h2>&#10003; Google login successful!</h2>\
                <p>You can close this tab and return to AITerm.</p>\
                </body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes()).await;

            request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|path| path.split_once('?').map(|(_, q)| q.to_string()))
                .and_then(|query| {
                    query
                        .split('&')
                        .find(|p| p.starts_with("code="))
                        .map(|p| p[5..].to_string())
                })
                .ok_or_else(|| "no authorization code in OAuth callback".to_string())
        },
    )
    .await
    .map_err(|_| "OAuth login timed out (2 minutes). Please try again.".to_string())?
    .map_err(|e| format!("OAuth callback error: {e}"))?;

    // Exchange the authorization code for access + refresh tokens.
    let http = reqwest::Client::new();
    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GEMINI_CLI_CLIENT_ID),
            ("client_secret", GEMINI_CLI_CLIENT_SECRET),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
        .map_err(|e| format!("token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token exchange failed (HTTP {status}): {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse token response: {e}"))?;

    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| format!("no access_token in response: {json}"))?
        .to_string();

    let refresh_token = json["refresh_token"]
        .as_str()
        .ok_or_else(|| "Google did not return a refresh token; please re-authorize".to_string())?
        .to_string();

    let expires_in = json["expires_in"].as_i64().unwrap_or(3600);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let token = GoogleOAuthToken {
        access_token,
        refresh_token,
        expires_at: now + expires_in,
    };
    let token_json =
        serde_json::to_string(&token).map_err(|e| format!("failed to serialize token: {e}"))?;

    secrets
        .set(&provider_id, &token_json)
        .map_err(|e| format!("failed to store token: {e}"))?;

    Ok(token.access_token)
}

/// Percent-encode a string for use in a URL query parameter.
fn url_encode(s: &str) -> String {
    s.bytes()
        .flat_map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![b as char]
            }
            _ => format!("%{b:02X}").chars().collect::<Vec<_>>(),
        })
        .collect()
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

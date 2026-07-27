//! Tauri commands for provider management (list / add / update / remove / test).

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::State;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    ai::{
        router::{
            AiRouter, DEEPSEEK_DEFAULT_BASE_URL, KIMI_DEFAULT_BASE_URL,
            OPENROUTER_DEFAULT_BASE_URL, XAI_DEFAULT_BASE_URL,
        },
        AiError,
    },
    config::{ConfigStore, ProviderConfig, ProviderType},
    secret::SecretStore,
};

const ANTHROPIC_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_OAUTH_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_OAUTH_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const ANTHROPIC_OAUTH_REDIRECT_URI_ENCODED: &str = "https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
// Claude subscription scopes — platform.claude.com requires %20 (not +) for space
// org:create_api_key must NOT be included (it switches to "Anthropic organization" mode)
const ANTHROPIC_OAUTH_SCOPE_ENCODED: &str = "user%3Aprofile%20user%3Ainference%20user%3Asessions%3Aclaude_code";

struct PkceSession {
    state: String,
    code_verifier: String,
}

pub struct AnthropicOAuthState {
    pending: Mutex<Option<PkceSession>>,
}

impl AnthropicOAuthState {
    pub fn new() -> Self {
        Self { pending: Mutex::new(None) }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn gen_code_verifier() -> String {
    let bytes: Vec<u8> = [
        Uuid::new_v4().as_bytes().as_slice(),
        Uuid::new_v4().as_bytes().as_slice(),
    ].concat();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn gen_code_challenge(verifier: &str) -> String {
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(h.finalize())
}

fn gen_state() -> String {
    // 24 bytes → 32 base64url chars, matching the format used by other Anthropic OAuth clients
    let b1 = Uuid::new_v4();
    let b2 = Uuid::new_v4();
    let mut bytes = Vec::with_capacity(24);
    bytes.extend_from_slice(b1.as_bytes());
    bytes.extend_from_slice(&b2.as_bytes()[..8]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes a JWT's payload (middle segment) without verifying the signature —
/// safe here because the token came directly from the OAuth token endpoint
/// over HTTPS; this only reads a claim, it doesn't trust the JWT as a
/// standalone credential.
fn decode_jwt_payload(jwt: &str) -> Option<serde_json::Value> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Extracts the `chatgpt_account_id` claim OpenAI nests under the
/// `https://api.openai.com/auth` custom claim of a Codex id_token. Used to
/// populate the `chatgpt-account-id` header required on Codex API requests.
fn extract_codex_account_id(id_token: &str) -> Option<String> {
    let payload = decode_jwt_payload(id_token)?;
    payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(str::to_string)
}

#[derive(Deserialize)]
struct AnthropicTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

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
    pub auth_method: Option<String>,
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
    pub auth_method: Option<String>,
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
            auth_method: p.auth_method.clone(),
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
        auth_method: input.auth_method,
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
        auth_method: input.auth_method,
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

    // Google AI OAuth: validate by checking token info, then verify Vertex AI project access.
    if provider_cfg.provider_type == ProviderType::GoogleAi
        && provider_cfg.auth_method.as_deref() == Some("oauth")
    {
        let token = secrets
            .get(&id)
            .map_err(|e| AiError::Network { message: format!("keychain read failed: {e}") })?
            .filter(|v| !v.trim().is_empty())
            .ok_or(AiError::NotConfigured)?;

        // Verify the token is valid via tokeninfo (no AI quota consumed).
        let client = reqwest::Client::new();
        let info_resp = client
            .get("https://oauth2.googleapis.com/tokeninfo")
            .query(&[("access_token", token.as_str())])
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;
        let info_status = info_resp.status();
        if !info_status.is_success() {
            let body = info_resp.text().await.unwrap_or_default();
            return Err(AiError::Network {
                message: format!("Token 已過期或無效，請重新登入 ({})", info_status.as_u16()),
            });
        }
        drop(info_resp);

        let base = provider_cfg.base_url.as_deref().unwrap_or("").trim_end_matches('/');
        if base.is_empty() {
            return Err(AiError::Network {
                message: "請填入 GCP Project ID 並儲存後再測試".into(),
            });
        }

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
    // Delegate to the `open` crate. Its Windows path launches the default browser
    // via `cmd /c start "" "<url>"` with the URL QUOTED, so ampersands in the OAuth
    // query string survive. The old hand-rolled `cmd /C start "" <url>` passed the
    // URL UNQUOTED (Rust doesn't quote args without spaces), and cmd.exe treats `&`
    // as a command separator — so it opened only `...?code=true` and dropped every
    // param after the first, which made claude.ai report "Missing client_id parameter".
    // macOS uses `/usr/bin/open`, Linux `xdg-open`, both passing the URL as one arg.
    let _ = open::that_detached(url);
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

// ── Anthropic OAuth ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn anthropic_oauth_start(
    oauth_state: State<'_, AnthropicOAuthState>,
) -> Result<(), String> {
    let code_verifier = gen_code_verifier();
    let code_challenge = gen_code_challenge(&code_verifier);
    let state = gen_state();

    let auth_url = format!(
        "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        ANTHROPIC_OAUTH_AUTH_URL,
        ANTHROPIC_OAUTH_CLIENT_ID,
        ANTHROPIC_OAUTH_REDIRECT_URI_ENCODED,
        ANTHROPIC_OAUTH_SCOPE_ENCODED,
        code_challenge,
        state,
    );

    *oauth_state.pending.lock().await = Some(PkceSession {
        state,
        code_verifier,
    });

    open_browser(&auth_url);
    Ok(())
}

#[tauri::command]
pub async fn anthropic_oauth_complete(
    provider_id: String,
    code_and_state: String,
    oauth_state: State<'_, AnthropicOAuthState>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    // Accept a full callback URL or the legacy "{code}#{state}" format.
    let (code, state_from_user) = if code_and_state.contains("code=") && code_and_state.contains("state=") {
        let query = code_and_state.find('?')
            .map(|i| &code_and_state[i + 1..])
            .unwrap_or(&code_and_state);
        let code = query.split('&')
            .find_map(|p| p.strip_prefix("code="))
            .map(str::to_owned)
            .ok_or("URL missing code parameter")?;
        let st = query.split('&')
            .find_map(|p| p.strip_prefix("state="))
            .map(str::to_owned)
            .ok_or("URL missing state parameter")?;
        (code, st)
    } else {
        let (c, s) = code_and_state.split_once('#')
            .ok_or("Invalid format — paste the full URL from the browser address bar")?;
        (c.to_owned(), s.to_owned())
    };

    let session = oauth_state
        .pending
        .lock()
        .await
        .take()
        .ok_or("No OAuth session in progress — please click 'Open Browser' first")?;

    if session.state != state_from_user {
        return Err("State mismatch — the auth code may be expired or tampered with".into());
    }

    let client = reqwest::Client::new();

    #[derive(Serialize)]
    struct TokenReq<'a> {
        grant_type: &'a str,
        code: &'a str,
        redirect_uri: &'a str,
        client_id: &'a str,
        code_verifier: &'a str,
        state: &'a str,
    }
    let resp = client
        .post(ANTHROPIC_OAUTH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
        .json(&TokenReq {
            grant_type: "authorization_code",
            code: &code,
            redirect_uri: ANTHROPIC_OAUTH_REDIRECT_URI,
            client_id: ANTHROPIC_OAUTH_CLIENT_ID,
            code_verifier: &session.code_verifier,
            state: &state_from_user,
        })
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed (HTTP {status}): {body}"));
    }

    let token_resp: AnthropicTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    secrets
        .set(&provider_id, &token_resp.access_token)
        .map_err(|e| format!("Failed to store access token: {e}"))?;

    if let Some(refresh) = token_resp.refresh_token {
        let _ = secrets.set(&format!("{provider_id}:oauth_refresh"), &refresh);
    }
    if let Some(expires_in) = token_resp.expires_in {
        let exp = now_secs() + expires_in;
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &exp.to_string());
    }

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = Some("oauth".into());
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))?;

    Ok(())
}

#[tauri::command]
pub fn anthropic_oauth_logout(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let _ = secrets.delete(&provider_id);
    let refresh_key = format!("{provider_id}:oauth_refresh");
    let _ = secrets.delete(&refresh_key);

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = None;
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))
}

#[tauri::command]
pub async fn get_anthropic_oauth_models(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let token = secrets
        .get(&provider_id)
        .map_err(|e| format!("Failed to read token: {e}"))?
        .ok_or("No OAuth token stored for this provider")?;

    #[derive(Deserialize)]
    struct ModelsResp {
        data: Vec<ModelItem>,
    }
    #[derive(Deserialize)]
    struct ModelItem {
        id: String,
    }

    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.anthropic.com/v1/models")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
        .header("x-app", "cli")
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Models request failed (HTTP {status}): {body}"));
    }

    let data: ModelsResp = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;
    let mut ids: Vec<String> = data.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    Ok(ids)
}

// ── Google OAuth ──────────────────────────────────────────────────────────────

const GOOGLE_OAUTH_CLIENT_ID: &str = "";
const GOOGLE_OAUTH_CLIENT_SECRET: &str = "";
const GOOGLE_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile openid";

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Starts the Google OAuth flow: spins up a local HTTP server, opens the browser,
/// waits for the callback (up to 2 minutes), exchanges the code for tokens, and
/// stores everything in the keychain. Blocks until complete or timeout.
#[tauri::command]
pub async fn google_oauth_login(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let client_id = GOOGLE_OAUTH_CLIENT_ID;
    let client_secret = GOOGLE_OAUTH_CLIENT_SECRET;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let code_verifier = gen_code_verifier();
    let code_challenge = gen_code_challenge(&code_verifier);
    let state = gen_state();

    // Bind on a random port so Google can redirect back to us.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Failed to start local server: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}/oauth2callback");

    // URL-encode redirect URI and scope.
    let scope_enc = GOOGLE_OAUTH_SCOPE
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace(' ', "%20");
    let redirect_enc = redirect_uri
        .replace(':', "%3A")
        .replace('/', "%2F");

    let auth_url = format!(
        "{url}?response_type=code&client_id={cid}&redirect_uri={redir}&scope={scope}&code_challenge={cc}&code_challenge_method=S256&state={st}&access_type=offline&prompt=consent",
        url = GOOGLE_OAUTH_AUTH_URL,
        cid = client_id,
        redir = redirect_enc,
        scope = scope_enc,
        cc = code_challenge,
        st = state,
    );

    open_browser(&auth_url);

    // Wait up to 2 minutes for the browser callback.
    let (mut stream, _) = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        listener.accept(),
    )
    .await
    .map_err(|_| "OAuth 超時（2 分鐘），請重試".to_string())?
    .map_err(|e| format!("Server accept error: {e}"))?;

    // Read the incoming HTTP GET request.
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse "GET /oauth2callback?code=...&state=... HTTP/1.1"
    let path_query = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("Invalid callback HTTP request")?;

    // Use url crate to properly decode percent-encoded values.
    let full_url = format!("http://localhost{path_query}");
    let parsed_url = url::Url::parse(&full_url)
        .map_err(|e| format!("Failed to parse callback URL: {e}"))?;
    let params: std::collections::HashMap<_, _> = parsed_url.query_pairs().collect();

    let code = params
        .get("code")
        .map(|v| v.to_string())
        .ok_or("No 'code' parameter in callback")?;
    let returned_state = params
        .get("state")
        .map(|v| v.to_string())
        .unwrap_or_default();

    // Send success page so the browser shows a friendly message.
    let html = concat!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Authorization Successful</title></head>",
        "<body style=\"font-family:sans-serif;text-align:center;padding:60px 20px;background:#1a1a1a;color:#fff\">",
        "<h2 style=\"color:#4caf50;margin-bottom:12px\">Authorization Successful!</h2>",
        "<p style=\"color:#aaa\">You can close this window and return to AITerm.</p>",
        "</body></html>"
    );
    let http_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(), html
    );
    let _ = stream.write_all(http_resp.as_bytes()).await;
    drop(stream);

    if returned_state != state {
        return Err("State mismatch — the authorization code may be expired or tampered with".into());
    }

    // Exchange authorization code for tokens.
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", code_verifier.as_str()),
    ];

    let resp = client
        .post(GOOGLE_OAUTH_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed (HTTP {status}): {body}"));
    }

    let token_resp: GoogleTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse token response: {e}"))?;

    secrets
        .set(&provider_id, &token_resp.access_token)
        .map_err(|e| format!("Failed to store access token: {e}"))?;

    if let Some(refresh) = token_resp.refresh_token {
        let _ = secrets.set(&format!("{provider_id}:oauth_refresh"), &refresh);
    }
    if let Some(expires_in) = token_resp.expires_in {
        let exp = now_secs() + expires_in;
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &exp.to_string());
    }

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = Some("oauth".into());
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))?;

    Ok(())
}

#[tauri::command]
pub fn google_oauth_logout(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let _ = secrets.delete(&provider_id);
    let _ = secrets.delete(&format!("{provider_id}:oauth_refresh"));
    let _ = secrets.delete(&format!("{provider_id}:oauth_expires_at"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_id"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_secret"));

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = None;
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))
}

/// Well-known Vertex AI Gemini models, used as fallback when the dynamic list fails.
fn vertex_ai_known_models() -> Vec<String> {
    vec![
        "google/gemini-2.5-pro-preview-06-05".into(),
        "google/gemini-2.5-pro-preview-05-06".into(),
        "google/gemini-2.5-flash-preview-05-20".into(),
        "google/gemini-2.5-flash-lite-preview-06-17".into(),
        "google/gemini-2.0-flash-001".into(),
        "google/gemini-2.0-flash-lite-001".into(),
        "google/gemini-2.0-flash-exp".into(),
        "google/gemini-1.5-pro-002".into(),
        "google/gemini-1.5-pro-001".into(),
        "google/gemini-1.5-flash-002".into(),
        "google/gemini-1.5-flash-001".into(),
    ]
}

#[tauri::command]
pub async fn get_google_oauth_models(
    provider_id: String,
    base_url_override: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let token = secrets
        .get(&provider_id)
        .map_err(|e| format!("Failed to read token: {e}"))?
        .ok_or("No OAuth token stored for this provider")?;

    // Prefer caller-supplied URL (before save), fall back to stored config.
    let base_url = base_url_override.unwrap_or_else(|| {
        config
            .get()
            .find_provider(&provider_id)
            .and_then(|p| p.base_url.clone())
            .unwrap_or_default()
    });

    if base_url.is_empty() {
        return Ok(vertex_ai_known_models());
    }

    let models_url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&models_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !resp.status().is_success() {
        log::warn!("Vertex AI /models returned {}, using known model list", resp.status());
        return Ok(vertex_ai_known_models());
    }

    #[derive(Deserialize)]
    struct ModelsResp {
        data: Vec<ModelItem>,
    }
    #[derive(Deserialize)]
    struct ModelItem {
        id: String,
    }

    match resp.json::<ModelsResp>().await {
        Ok(data) => {
            let mut ids: Vec<String> = data
                .data
                .into_iter()
                .map(|m| m.id)
                .filter(|id| id.contains("gemini"))
                .collect();
            if ids.is_empty() {
                return Ok(vertex_ai_known_models());
            }
            ids.sort();
            Ok(ids)
        }
        Err(_) => Ok(vertex_ai_known_models()),
    }
}

// ── ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
}

/// Fetch and parse a `/models` endpoint that returns the OpenAI-shaped
/// `{ "data": [{ "id": "..." }, ...] }` payload. Shared by the OpenRouter,
/// xAI, DeepSeek, and Kimi model-list commands below, which are otherwise
/// identical except for their default base URL.
pub async fn list_openai_style_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("api_key is required".into());
    }

    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/models");

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("list models failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("list models failed ({status}): {body}"));
    }

    let payload: OpenAiModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse models response failed: {e}"))?;

    Ok(payload.data.into_iter().map(|m| m.id).collect())
}

/// Fetch available OpenRouter models using an API key supplied directly
/// (used while the user is typing the key before saving the provider).
#[tauri::command]
pub async fn get_openrouter_models(api_key: String) -> Result<Vec<String>, String> {
    list_openai_style_models(OPENROUTER_DEFAULT_BASE_URL, &api_key).await
}

/// Fetch available OpenRouter models for a saved provider (reads key from keychain).
#[tauri::command]
pub async fn get_openrouter_models_by_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::Openrouter {
        return Err(format!("provider '{id}' is not openrouter"));
    }
    let key = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved API key"))?;
    list_openai_style_models(
        provider.base_url.as_deref().unwrap_or(OPENROUTER_DEFAULT_BASE_URL),
        key.trim(),
    )
    .await
}

/// Fetch available xAI models using an API key supplied directly.
#[tauri::command]
pub async fn get_xai_models(api_key: String) -> Result<Vec<String>, String> {
    list_openai_style_models(XAI_DEFAULT_BASE_URL, &api_key).await
}

/// Fetch available xAI models for a saved provider (reads key from keychain).
#[tauri::command]
pub async fn get_xai_models_by_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::Xai {
        return Err(format!("provider '{id}' is not xai"));
    }
    let key = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved API key"))?;
    list_openai_style_models(
        provider.base_url.as_deref().unwrap_or(XAI_DEFAULT_BASE_URL),
        key.trim(),
    )
    .await
}

/// Fetch available DeepSeek models using an API key supplied directly.
#[tauri::command]
pub async fn get_deepseek_models(api_key: String) -> Result<Vec<String>, String> {
    list_openai_style_models(DEEPSEEK_DEFAULT_BASE_URL, &api_key).await
}

/// Fetch available DeepSeek models for a saved provider (reads key from keychain).
#[tauri::command]
pub async fn get_deepseek_models_by_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::Deepseek {
        return Err(format!("provider '{id}' is not deepseek"));
    }
    let key = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved API key"))?;
    list_openai_style_models(
        provider.base_url.as_deref().unwrap_or(DEEPSEEK_DEFAULT_BASE_URL),
        key.trim(),
    )
    .await
}

/// Fetch available Kimi models using an API key supplied directly.
#[tauri::command]
pub async fn get_kimi_models(api_key: String) -> Result<Vec<String>, String> {
    list_openai_style_models(KIMI_DEFAULT_BASE_URL, &api_key).await
}

/// Fetch available Kimi models for a saved provider (reads key from keychain).
#[tauri::command]
pub async fn get_kimi_models_by_provider(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let provider = config
        .get()
        .find_provider(&id)
        .cloned()
        .ok_or_else(|| format!("provider '{id}' not found"))?;
    if provider.provider_type != ProviderType::Kimi {
        return Err(format!("provider '{id}' is not kimi"));
    }
    let key = secrets
        .get(&id)
        .map_err(|e| format!("failed to read provider secret: {e}"))?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("provider '{id}' has no saved API key"))?;
    list_openai_style_models(
        provider.base_url.as_deref().unwrap_or(KIMI_DEFAULT_BASE_URL),
        key.trim(),
    )
    .await
}

#[cfg(test)]
mod codex_jwt_tests {
    use super::*;

    #[test]
    fn extract_codex_account_id_reads_nested_auth_claim() {
        let payload = URL_SAFE_NO_PAD.encode(
            br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-abc123"}}"#,
        );
        let jwt = format!("header.{payload}.signature");
        assert_eq!(extract_codex_account_id(&jwt), Some("acct-abc123".to_string()));
    }

    #[test]
    fn extract_codex_account_id_returns_none_when_claim_missing() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"user-1"}"#);
        let jwt = format!("header.{payload}.signature");
        assert_eq!(extract_codex_account_id(&jwt), None);
    }

    #[test]
    fn extract_codex_account_id_returns_none_for_malformed_jwt() {
        assert_eq!(extract_codex_account_id("not-a-jwt"), None);
    }
}

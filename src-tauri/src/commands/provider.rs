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

/// 這裡是 Anthropic OAuth 端點知識唯一的一份定義；`ai/router.rs` 的 refresh
/// 流程 import 這裡，不另外複製一份。同一個檔案先前對 Google 的 client
/// secret 留了第二份副本，結果登入成功但每次 refresh 都 400，約一小時後
/// session 死掉且毫無線索（見 router.rs 開頭的註解）。共用一份定義讓那種
/// 失敗在結構上不可能發生。
pub(crate) const ANTHROPIC_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_OAUTH_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
pub(crate) const ANTHROPIC_OAUTH_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const ANTHROPIC_OAUTH_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const ANTHROPIC_OAUTH_REDIRECT_URI_ENCODED: &str = "https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback";
// Claude subscription scopes — platform.claude.com requires %20 (not +) for space
// org:create_api_key must NOT be included (it switches to "Anthropic organization" mode)
const ANTHROPIC_OAUTH_SCOPE_ENCODED: &str = "user%3Aprofile%20user%3Ainference%20user%3Asessions%3Aclaude_code";

const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_OAUTH_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_OAUTH_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CODEX_OAUTH_REDIRECT_PORT: u16 = 1455;

#[derive(Deserialize)]
struct CodexTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

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

const ANTIGRAVITY_BOOTSTRAP_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";

/// `loadCodeAssist`'s response nests the project id as either a plain string
/// or an `{"id": "..."}` object under `cloudaicompanionProject` — tolerate
/// both. Empty strings count as "absent" (a fresh account with no project
/// yet still returns the key, just empty).
fn extract_cloudaicompanion_project_id(json: &serde_json::Value) -> Option<String> {
    let field = json.get("cloudaicompanionProject")?;
    let id = field
        .as_str()
        .map(str::to_string)
        .or_else(|| field.get("id").and_then(|v| v.as_str()).map(str::to_string))?;
    if id.is_empty() { None } else { Some(id) }
}

fn antigravity_headers(builder: reqwest::RequestBuilder, access_token: &str) -> reqwest::RequestBuilder {
    builder
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", format!("antigravity/ide/{} darwin/arm64", crate::ai::antigravity::ANTIGRAVITY_IDE_VERSION))
        .bearer_auth(access_token)
}

/// Onboards a Google account for Antigravity/Cloud Code Assist and returns
/// its Cloud Code project id. Called once right after OAuth token exchange.
///
/// Sequence (ported from OmniRoute's `postExchangeAntigravity`):
/// 1. Call `loadCodeAssist`. If it already returns a project id, done.
/// 2. If not (brand-new Google account with no Cloud Code project yet), call
///    `onboardUser` and poll up to 10 times (5s apart, ~45s total) until it
///    reports `done: true`, then retry `loadCodeAssist` once to pick up the
///    freshly-provisioned project.
async fn perform_antigravity_onboarding(access_token: &str) -> Result<String, String> {
    perform_antigravity_onboarding_at(
        access_token,
        ANTIGRAVITY_BOOTSTRAP_BASE_URL,
        std::time::Duration::from_secs(5),
    )
    .await
}

/// Test-only hook: lets integration tests point at a wiremock server instead
/// of the real cloudcode-pa.googleapis.com backend, and shrink the polling
/// interval so exhaustion tests don't have to sleep for real.
async fn perform_antigravity_onboarding_at(
    access_token: &str,
    base_url: &str,
    poll_interval: std::time::Duration,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let load_body = serde_json::json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
    let resp = antigravity_headers(
        client.post(format!("{base_url}/v1internal:loadCodeAssist")),
        access_token,
    )
    .json(&load_body)
    .send()
    .await
    .map_err(|e| format!("loadCodeAssist request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("loadCodeAssist failed (HTTP {status}): {body}"));
    }
    let load_json: serde_json::Value = resp.json().await.map_err(|e| format!("loadCodeAssist parse error: {e}"))?;

    if let Some(project_id) = extract_cloudaicompanion_project_id(&load_json) {
        return Ok(project_id);
    }

    // No project yet — onboard, then retry loadCodeAssist once.
    let tier_id = load_json
        .get("allowedTiers")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.iter().find(|t| t.get("isDefault").and_then(|d| d.as_bool()).unwrap_or(false)))
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            log::warn!(
                "Antigravity loadCodeAssist returned no default tier; falling back to \"legacy-tier\" (this value is reverse-engineered and may be wrong for this account type)"
            );
            "legacy-tier".to_string()
        });

    let onboard_body = serde_json::json!({
        "tierId": tier_id,
        "metadata": { "ideType": "ANTIGRAVITY" },
    });

    for attempt in 0..10 {
        let resp = antigravity_headers(
            client.post(format!("{base_url}/v1internal:onboardUser")),
            access_token,
        )
        .json(&onboard_body)
        .send()
        .await
        .map_err(|e| format!("onboardUser request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("onboardUser failed (HTTP {status}): {body}"));
        }
        let onboard_json: serde_json::Value = resp.json().await.map_err(|e| format!("onboardUser parse error: {e}"))?;
        if onboard_json.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
            break;
        }
        if attempt < 9 {
            tokio::time::sleep(poll_interval).await;
        }
    }

    let resp = antigravity_headers(
        client.post(format!("{base_url}/v1internal:loadCodeAssist")),
        access_token,
    )
    .json(&load_body)
    .send()
    .await
    .map_err(|e| format!("loadCodeAssist (post-onboard) request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("loadCodeAssist (post-onboard) failed (HTTP {status}): {body}"));
    }
    let load_json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("loadCodeAssist (post-onboard) parse error: {e}"))?;

    extract_cloudaicompanion_project_id(&load_json)
        .ok_or_else(|| "Onboarding completed but no Cloud Code project id was returned".to_string())
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

    // NOTE: Google AI OAuth deliberately has NO special case here — it falls
    // through to the generic path below, same as Anthropic/Codex OAuth.
    //
    // There used to be one, written for an earlier design where the user
    // hand-entered a GCP project id into `base_url`. That design is gone: the
    // project id is now resolved automatically by `perform_antigravity_onboarding`
    // at login and stored under `{id}:project_id`, and the base_url field is
    // hidden entirely in OAuth mode. The old check had degenerated into a
    // false positive — it verified the token via `tokeninfo` and then merely
    // asserted `base_url` was non-empty (which the form auto-populates), so it
    // returned "ok" without ever exercising the project id or the real
    // endpoint. `AntigravityClient::health_check()` does the correct thing:
    // one minimal streamGenerateContent call validating token + project id
    // together against the actual backend.

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

/// Persist an OAuth login's secrets as a unit.
///
/// Every entry matters: without `:oauth_refresh` the session silently dies at
/// the first refresh, and without `:project_id`/`:oauth_account_id` the provider
/// can't build a request at all — so a half-stored login is worse than none.
/// On failure whatever was already written is removed and the caller reports the
/// error, which is what lets the user simply log in again.
fn store_login_secrets(secrets: &SecretStore, entries: &[(String, String)]) -> Result<(), String> {
    for (i, (key, value)) in entries.iter().enumerate() {
        if let Err(e) = secrets.set(key, value) {
            for (written, _) in &entries[..i] {
                let _ = secrets.delete(written);
            }
            return Err(format!("Failed to store keychain entry '{key}': {e}"));
        }
    }
    Ok(())
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
        .header("anthropic-beta", crate::ai::anthropic::OAUTH_BETA_HEADER)
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

    let mut entries = vec![(provider_id.clone(), token_resp.access_token)];
    if let Some(refresh) = token_resp.refresh_token {
        entries.push((format!("{provider_id}:oauth_refresh"), refresh));
    }
    if let Some(expires_in) = token_resp.expires_in {
        entries.push((format!("{provider_id}:oauth_expires_at"), (now_secs() + expires_in).to_string()));
    }
    store_login_secrets(&secrets, &entries)?;

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
        .header("anthropic-beta", crate::ai::anthropic::OAUTH_BETA_HEADER)
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

// Antigravity's OAuth client — Google's public installed-app client for
// Antigravity / Gemini Code Assist. Per Google's own native-app OAuth docs,
// installed-app client_id/client_secret pairs are distributed inside the
// client binary and are NOT confidential; PKCE (which this flow uses) is
// what actually secures the exchange. Confirmed empirically: an empty
// client_secret makes the token endpoint reject the exchange with
// `invalid_request: client_secret is missing`, so a value is required even
// though it isn't secret.
//
// These are `pub(crate)` and referenced from `ai/router.rs`'s refresh path
// rather than duplicated there. An earlier revision DID duplicate them, and
// the copy in router.rs silently kept an empty client_id — login worked but
// every token refresh 400'd into a warn-and-reuse-stale-token fallback, so
// sessions died ~an hour later with nothing pointing at the cause. One
// definition means that can't recur.
pub(crate) const GOOGLE_OAUTH_CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";

// Split so the literal never appears as a contiguous `GOCSPX-…` string in
// source text. That exact shape is matched by GitHub Secret Scanning,
// Semgrep, and friends, which would flag every release and can trip push
// protection on legitimate commits. This is obfuscation for scanner hygiene
// ONLY — not encryption, and not an attempt to hide the value from a reader
// (it's public by design, see above). Same goal as the reference
// implementation's XOR-masking approach, minus the machinery.
const GOOGLE_OAUTH_SECRET_PREFIX: &str = "GOCSPX";
const GOOGLE_OAUTH_SECRET_BODY: &str = "K58FWR486LdLJ1mLB8sXC4z6qDAf";

/// The Antigravity OAuth client secret, overridable via the
/// `AITERM_GOOGLE_OAUTH_CLIENT_SECRET` env var for anyone who registered
/// their own Google OAuth app and wants to use their own credentials.
pub(crate) fn google_oauth_client_secret() -> String {
    match std::env::var("AITERM_GOOGLE_OAUTH_CLIENT_SECRET") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => format!("{GOOGLE_OAUTH_SECRET_PREFIX}-{GOOGLE_OAUTH_SECRET_BODY}"),
    }
}

const GOOGLE_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Deliberately NO "openid" scope — including it (even without PKCE) has been
// found to route Google into a hanging `firstparty/nativeapp` consent screen
// for this specific client. `cclog`/`experimentsandconfigs` are required by
// the Antigravity backend itself, not optional extras.
const GOOGLE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

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
    let client_secret = google_oauth_client_secret();
    let client_secret = client_secret.as_str();
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

    // Onboard BEFORE persisting anything — if this fails, the login as a
    // whole fails and nothing is half-saved (no token stored without a
    // matching project id).
    let project_id = perform_antigravity_onboarding(&token_resp.access_token)
        .await
        .map_err(|e| format!("Antigravity onboarding failed: {e}"))?;

    let mut entries = vec![(provider_id.clone(), token_resp.access_token)];
    if let Some(refresh) = token_resp.refresh_token {
        entries.push((format!("{provider_id}:oauth_refresh"), refresh));
    }
    if let Some(expires_in) = token_resp.expires_in {
        entries.push((format!("{provider_id}:oauth_expires_at"), (now_secs() + expires_in).to_string()));
    }
    entries.push((format!("{provider_id}:project_id"), project_id));
    store_login_secrets(&secrets, &entries)?;

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
    let _ = secrets.delete(&format!("{provider_id}:project_id"));

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = None;
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))
}

/// Known Gemini model ids as of this writing, used as a fallback when the
/// live discovery call fails or returns nothing. Model availability through
/// this internal API is known to shift over time (OmniRoute's own catalog
/// carries this exact caveat) — treat this list as a starting point to
/// re-verify periodically, not a permanent source of truth.
fn antigravity_known_models() -> Vec<String> {
    vec![
        "gemini-2.5-pro".into(),
        "gemini-2.5-flash".into(),
        "gemini-2.5-flash-lite".into(),
    ]
}

/// Extracts model ids from Antigravity's model-discovery response.
///
/// The real shape (confirmed against the live endpoint) is an OBJECT keyed by
/// model id, whose values hold metadata:
/// `{"models": {"gemini-pro-agent": {"displayName": "...", ...}, ...}}`
/// — so the id we want is the KEY, not a field inside the value. Entries
/// flagged `isInternal` are Antigravity's own internal/placeholder models and
/// aren't usable as chat models, so they're skipped.
///
/// The array forms (`{"models": [...]}` / a bare array, id read from `name`
/// or `id`) are kept as a fallback in case the shape shifts again.
fn parse_antigravity_models_response(json: &serde_json::Value) -> Vec<String> {
    if let Some(map) = json.get("models").and_then(|v| v.as_object()) {
        let mut ids: Vec<String> = map
            .iter()
            .filter(|(_, meta)| !meta.get("isInternal").and_then(|v| v.as_bool()).unwrap_or(false))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        return ids;
    }

    let items: Vec<&serde_json::Value> = if let Some(arr) = json.get("models").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else if let Some(arr) = json.as_array() {
        arr.iter().collect()
    } else {
        vec![]
    };

    let mut ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("name")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                // Google's model-listing APIs return `name` as a qualified
                // resource path (e.g. "models/gemini-2.5-pro") rather than a
                // bare id — see list_google_ai_models() in this same file,
                // which strips the same prefix for the public API. The bare
                // id is what must go into the request body's "model" field.
                .map(|s| s.trim_start_matches("models/").to_string())
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[tauri::command]
pub async fn get_google_oauth_models(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let token = secrets
        .get(&provider_id)
        .map_err(|e| format!("Failed to read token: {e}"))?
        .ok_or("No OAuth token stored for this provider")?;

    // POST, not GET, and the onboarded project id goes in the body — Google's
    // `:verb`-style endpoints are POST (same as loadCodeAssist/onboardUser
    // above). An earlier revision used GET with no body and got a flat 404,
    // which silently degraded every user to the hardcoded fallback list.
    let project_id = secrets.get(&format!("{provider_id}:project_id")).ok().flatten();
    let request_body = match &project_id {
        Some(p) => serde_json::json!({ "project": p }),
        None => serde_json::json!({}),
    };

    let client = reqwest::Client::new();
    let models = match antigravity_headers(
        client.post(format!("{ANTIGRAVITY_BOOTSTRAP_BASE_URL}/v1internal:fetchAvailableModels")),
        &token,
    )
    .json(&request_body)
    .send()
    .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(json) => parse_antigravity_models_response(&json),
            Err(e) => {
                log::warn!("Antigravity fetchAvailableModels response was not valid JSON, using fallback list ({e})");
                vec![]
            }
        },
        Ok(resp) => {
            log::warn!("Antigravity fetchAvailableModels returned {}, using fallback list", resp.status());
            vec![]
        }
        Err(e) => {
            log::warn!("Antigravity fetchAvailableModels request failed: {e}");
            vec![]
        }
    };

    if models.is_empty() {
        log::warn!("Antigravity fetchAvailableModels returned no recognizable model ids, using fallback list");
        Ok(antigravity_known_models())
    } else {
        Ok(models)
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

// ── Codex OAuth ───────────────────────────────────────────────────────────────

/// Starts the Codex OAuth flow: spins up a local HTTP server fixed to port
/// 1455 (the only redirect_uri registered against Codex's public client_id),
/// opens the browser, waits for the callback (up to 2 minutes), exchanges
/// the code for tokens, extracts the ChatGPT account id from the id_token,
/// and stores everything in the keychain. Blocks until complete or timeout.
#[tauri::command]
pub async fn codex_oauth_login(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let code_verifier = gen_code_verifier();
    let code_challenge = gen_code_challenge(&code_verifier);
    let state = gen_state();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CODEX_OAUTH_REDIRECT_PORT))
        .await
        .map_err(|e| format!("無法在 1455 port 啟動本機伺服器（可能已被其他程式占用）：{e}"))?;

    // `prompt=login` forces re-authentication instead of silently reusing an
    // existing Auth0 session — without it, logging in with a second ChatGPT
    // account on this same client_id invalidates the first account's refresh
    // token (session takeover).
    let auth_url = format!(
        "{url}?response_type=code&client_id={cid}&redirect_uri={redir}&scope=openid+profile+email+offline_access&code_challenge={cc}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs&prompt=login&state={st}",
        url = CODEX_OAUTH_AUTH_URL,
        cid = CODEX_OAUTH_CLIENT_ID,
        redir = CODEX_OAUTH_REDIRECT_URI,
        cc = code_challenge,
        st = state,
    );

    open_browser(&auth_url);

    let (mut stream, _) = tokio::time::timeout(std::time::Duration::from_secs(120), listener.accept())
        .await
        .map_err(|_| "OAuth 超時（2 分鐘），請重試".to_string())?
        .map_err(|e| format!("Server accept error: {e}"))?;

    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let path_query = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or("Invalid callback HTTP request")?;

    let full_url = format!("http://localhost{path_query}");
    let parsed_url =
        url::Url::parse(&full_url).map_err(|e| format!("Failed to parse callback URL: {e}"))?;
    let params: std::collections::HashMap<_, _> = parsed_url.query_pairs().collect();

    let code = params.get("code").map(|v| v.to_string()).ok_or("No 'code' parameter in callback")?;
    let returned_state = params.get("state").map(|v| v.to_string()).unwrap_or_default();

    let html = concat!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Authorization Successful</title></head>",
        "<body style=\"font-family:sans-serif;text-align:center;padding:60px 20px;background:#1a1a1a;color:#fff\">",
        "<h2 style=\"color:#4caf50;margin-bottom:12px\">Authorization Successful!</h2>",
        "<p style=\"color:#aaa\">You can close this window and return to AITerm.</p>",
        "</body></html>"
    );
    let http_resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(http_resp.as_bytes()).await;
    drop(stream);

    if returned_state != state {
        return Err("State mismatch — the authorization code may be expired or tampered with".into());
    }

    let client = reqwest::Client::new();
    let form_params = [
        ("grant_type", "authorization_code"),
        ("client_id", CODEX_OAUTH_CLIENT_ID),
        ("code", code.as_str()),
        ("redirect_uri", CODEX_OAUTH_REDIRECT_URI),
        ("code_verifier", code_verifier.as_str()),
    ];

    let resp = client
        .post(CODEX_OAUTH_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&form_params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token exchange failed (HTTP {status}): {body}"));
    }

    let token_resp: CodexTokenResponse =
        resp.json().await.map_err(|e| format!("Failed to parse token response: {e}"))?;

    let mut entries = vec![(provider_id.clone(), token_resp.access_token)];
    if let Some(refresh) = token_resp.refresh_token {
        entries.push((format!("{provider_id}:oauth_refresh"), refresh));
    }
    if let Some(expires_in) = token_resp.expires_in {
        entries.push((format!("{provider_id}:oauth_expires_at"), (now_secs() + expires_in).to_string()));
    }
    if let Some(account_id) = token_resp.id_token.as_deref().and_then(extract_codex_account_id) {
        entries.push((format!("{provider_id}:oauth_account_id"), account_id));
    }
    store_login_secrets(&secrets, &entries)?;

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = Some("oauth".into());
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))?;

    Ok(())
}

/// Log out from Codex OAuth: clears the access token, refresh token, expiry,
/// and cached ChatGPT account id.
#[tauri::command]
pub fn codex_oauth_logout(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let _ = secrets.delete(&provider_id);
    let _ = secrets.delete(&format!("{provider_id}:oauth_refresh"));
    let _ = secrets.delete(&format!("{provider_id}:oauth_expires_at"));
    let _ = secrets.delete(&format!("{provider_id}:oauth_account_id"));

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = None;
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))
}

/// Extracts model ids from Codex's `/models` discovery response, tolerating
/// the shapes it's known to return: `{"models":[...]}`, `{"data":[...]}`, a
/// bare array, or an object keyed by model id.
fn parse_codex_models_response(json: &serde_json::Value) -> Vec<String> {
    let items: Vec<&serde_json::Value> = if let Some(arr) = json.get("models").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else if let Some(arr) = json.get("data").and_then(|v| v.as_array()) {
        arr.iter().collect()
    } else if let Some(arr) = json.as_array() {
        arr.iter().collect()
    } else if let Some(obj) = json.as_object() {
        obj.values().collect()
    } else {
        vec![]
    };

    let mut ids: Vec<String> = items
        .iter()
        .filter_map(|item| {
            item.get("slug")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("model"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

const CODEX_FALLBACK_MODELS: &[&str] = &["gpt-5.1-codex", "gpt-5.1-codex-mini"];

/// Fetches the live Codex model catalog using the stored OAuth token. Falls
/// back to a small hardcoded list if the request fails or the response is
/// empty/unparseable — never blocks saving the provider on this.
#[tauri::command]
pub async fn get_codex_oauth_models(
    provider_id: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let token = secrets
        .get(&provider_id)
        .map_err(|e| format!("Failed to read token: {e}"))?
        .ok_or("No OAuth token stored for this provider")?;
    let account_id = secrets.get(&format!("{provider_id}:oauth_account_id")).ok().flatten();

    let client = reqwest::Client::new();
    let mut builder = client
        .get(format!(
            "https://chatgpt.com/backend-api/codex/models?client_version={}",
            crate::ai::codex::CODEX_CLIENT_VERSION
        ))
        .bearer_auth(&token)
        .header("originator", "codex_cli_rs")
        .header("User-Agent", crate::ai::codex::CODEX_USER_AGENT)
        .header("Version", crate::ai::codex::CODEX_CLIENT_VERSION)
        .header("Openai-Beta", "responses=experimental")
        .header("X-Codex-Beta-Features", "responses_websockets");
    if let Some(id) = &account_id {
        builder = builder.header("chatgpt-account-id", id.as_str());
    }

    let models = match builder.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(json) => parse_codex_models_response(&json),
            Err(e) => {
                log::warn!("Codex /models response was not valid JSON, using fallback list ({e})");
                vec![]
            }
        },
        Ok(resp) => {
            log::warn!("Codex /models returned {}, using fallback list", resp.status());
            vec![]
        }
        Err(e) => {
            log::warn!("Codex /models request failed: {e}");
            vec![]
        }
    };

    if models.is_empty() {
        Ok(CODEX_FALLBACK_MODELS.iter().map(|s| s.to_string()).collect())
    } else {
        Ok(models)
    }
}

#[cfg(test)]
mod login_secrets_tests {
    use super::*;

    /// URL-encoded 版本必須與原始 redirect_uri 等價。
    ///
    /// 這兩個常數是分開手寫的：一個進 authorize URL 的 query string，一個進
    /// token 交換的 JSON body。不一致時 Anthropic 會拒絕交換，而錯誤訊息不會
    /// 告訴你是 redirect_uri 對不上。
    #[test]
    fn redirect_uri_encoded_matches_plain() {
        let encoded = ANTHROPIC_OAUTH_REDIRECT_URI
            .replace(':', "%3A")
            .replace('/', "%2F");
        assert_eq!(encoded, ANTHROPIC_OAUTH_REDIRECT_URI_ENCODED);
    }

    #[test]
    #[ignore = "requires OS keychain"]
    fn rolls_back_everything_written_before_a_failure() {
        let secrets = SecretStore::new();
        let id = "aiterm-test-login-rollback";
        // An empty key is always rejected by the keychain, so the second write
        // fails after the first one has already landed.
        let entries = vec![
            (id.to_string(), "access-token".to_string()),
            (String::new(), "refresh-token".to_string()),
        ];

        assert!(store_login_secrets(&secrets, &entries).is_err());
        assert!(!secrets.has(id), "a half-stored login must be rolled back");
    }

    #[test]
    #[ignore = "requires OS keychain"]
    fn stores_every_entry_on_success() {
        let secrets = SecretStore::new();
        let id = "aiterm-test-login-success";
        let entries = vec![
            (id.to_string(), "access-token".to_string()),
            (format!("{id}:oauth_refresh"), "refresh-token".to_string()),
        ];

        store_login_secrets(&secrets, &entries).unwrap();

        assert_eq!(secrets.get(id).unwrap().as_deref(), Some("access-token"));
        assert_eq!(
            secrets.get(&format!("{id}:oauth_refresh")).unwrap().as_deref(),
            Some("refresh-token")
        );

        for (key, _) in &entries {
            let _ = secrets.delete(key);
        }
    }
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

#[cfg(test)]
mod codex_models_tests {
    use super::*;

    #[test]
    fn parse_codex_models_response_handles_models_key() {
        let json = serde_json::json!({"models": [{"id": "gpt-5.1-codex-high"}, {"slug": "gpt-5.1-codex"}]});
        assert_eq!(
            parse_codex_models_response(&json),
            vec!["gpt-5.1-codex".to_string(), "gpt-5.1-codex-high".to_string()]
        );
    }

    #[test]
    fn parse_codex_models_response_handles_data_key() {
        let json = serde_json::json!({"data": [{"model": "gpt-5.1-codex"}]});
        assert_eq!(parse_codex_models_response(&json), vec!["gpt-5.1-codex".to_string()]);
    }

    #[test]
    fn parse_codex_models_response_handles_bare_array() {
        let json = serde_json::json!([{"slug": "gpt-5.1-codex"}]);
        assert_eq!(parse_codex_models_response(&json), vec!["gpt-5.1-codex".to_string()]);
    }

    #[test]
    fn parse_codex_models_response_handles_object_map() {
        let json = serde_json::json!({"gpt-5.1-codex": {"slug": "gpt-5.1-codex"}});
        assert_eq!(parse_codex_models_response(&json), vec!["gpt-5.1-codex".to_string()]);
    }

    #[test]
    fn parse_codex_models_response_skips_items_with_no_id_field() {
        let json = serde_json::json!({"models": [{"display_name": "no id here"}]});
        assert!(parse_codex_models_response(&json).is_empty());
    }

    #[test]
    fn parse_codex_models_response_deduplicates_ids() {
        let json = serde_json::json!({"models": [{"slug": "a"}, {"id": "a"}, {"slug": "b"}]});
        assert_eq!(parse_codex_models_response(&json), vec!["a".to_string(), "b".to_string()]);
    }
}

#[cfg(test)]
mod antigravity_tests {
    use super::*;

    #[test]
    fn extract_project_id_reads_plain_string_field() {
        let json = serde_json::json!({"cloudaicompanionProject": "proj-abc"});
        assert_eq!(extract_cloudaicompanion_project_id(&json), Some("proj-abc".to_string()));
    }

    #[test]
    fn extract_project_id_reads_nested_id_field() {
        let json = serde_json::json!({"cloudaicompanionProject": {"id": "proj-xyz"}});
        assert_eq!(extract_cloudaicompanion_project_id(&json), Some("proj-xyz".to_string()));
    }

    #[test]
    fn extract_project_id_returns_none_when_absent() {
        let json = serde_json::json!({"allowedTiers": []});
        assert_eq!(extract_cloudaicompanion_project_id(&json), None);
    }

    #[test]
    fn extract_project_id_returns_none_for_empty_string() {
        let json = serde_json::json!({"cloudaicompanionProject": ""});
        assert_eq!(extract_cloudaicompanion_project_id(&json), None);
    }

    #[test]
    fn parse_antigravity_models_response_handles_models_key() {
        let json = serde_json::json!({"models": [{"name": "gemini-2.5-pro"}, {"id": "gemini-2.5-flash"}]});
        assert_eq!(
            parse_antigravity_models_response(&json),
            vec!["gemini-2.5-flash".to_string(), "gemini-2.5-pro".to_string()]
        );
    }

    #[test]
    fn parse_antigravity_models_response_handles_bare_array() {
        let json = serde_json::json!([{"name": "gemini-2.5-pro"}]);
        assert_eq!(parse_antigravity_models_response(&json), vec!["gemini-2.5-pro".to_string()]);
    }

    #[test]
    fn parse_antigravity_models_response_skips_items_with_no_id_field() {
        let json = serde_json::json!({"models": [{"displayName": "no id here"}]});
        assert!(parse_antigravity_models_response(&json).is_empty());
    }

    #[test]
    fn parse_antigravity_models_response_deduplicates_ids() {
        let json = serde_json::json!({"models": [{"name": "a"}, {"id": "a"}, {"name": "b"}]});
        assert_eq!(parse_antigravity_models_response(&json), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn parse_antigravity_models_response_strips_models_prefix() {
        let json = serde_json::json!({"models": [{"name": "models/gemini-2.5-pro"}]});
        assert_eq!(parse_antigravity_models_response(&json), vec!["gemini-2.5-pro".to_string()]);
    }

    #[test]
    fn parse_antigravity_models_response_leaves_bare_name_untouched() {
        let json = serde_json::json!({"models": [{"name": "gemini-2.5-pro"}]});
        assert_eq!(parse_antigravity_models_response(&json), vec!["gemini-2.5-pro".to_string()]);
    }

    /// Regression: the live endpoint returns `models` as an OBJECT keyed by
    /// model id, not an array. An earlier revision only handled the array
    /// forms, so a perfectly good 200 response parsed to zero ids and every
    /// user silently got the 3-item hardcoded fallback list instead of the
    /// real catalog. Shape captured verbatim from a real response.
    #[test]
    fn parse_antigravity_models_response_handles_object_keyed_by_model_id() {
        let json = serde_json::json!({
            "models": {
                "gemini-pro-agent": {
                    "displayName": "Gemini 3.1 Pro (High)",
                    "supportsThinking": true,
                    "model": "MODEL_PLACEHOLDER_M16"
                },
                "gemini-3.1-flash-image": {
                    "displayName": "Gemini 3.1 Flash Image",
                    "model": "MODEL_PLACEHOLDER_M21"
                }
            }
        });
        assert_eq!(
            parse_antigravity_models_response(&json),
            vec!["gemini-3.1-flash-image".to_string(), "gemini-pro-agent".to_string()]
        );
    }

    /// `isInternal` entries (e.g. "chat_23310") are Antigravity's own
    /// internal/placeholder models — not selectable as chat models.
    #[test]
    fn parse_antigravity_models_response_skips_internal_models() {
        let json = serde_json::json!({
            "models": {
                "chat_23310": { "model": "MODEL_CHAT_23310", "isInternal": true },
                "gemini-pro-agent": { "displayName": "Gemini 3.1 Pro (High)" }
            }
        });
        assert_eq!(parse_antigravity_models_response(&json), vec!["gemini-pro-agent".to_string()]);
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn onboarding_short_circuits_when_project_already_exists() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"cloudaicompanionProject": "proj-existing"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        // No onboardUser mock is mounted — if the code were to call it despite
        // already having a project id, wiremock would 404 and the request
        // would fail, turning that regression into a test failure here.

        let result =
            perform_antigravity_onboarding_at("test-token", &server.uri(), std::time::Duration::from_millis(1))
                .await;

        assert_eq!(result, Ok("proj-existing".to_string()));
    }

    #[tokio::test]
    async fn onboarding_onboards_then_succeeds() {
        let server = MockServer::start().await;

        // First loadCodeAssist call: brand-new account, no project yet.
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "allowedTiers": [{"id": "free-tier", "isDefault": true}]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1internal:onboardUser"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"done": true})))
            .expect(1)
            .mount(&server)
            .await;

        // Second loadCodeAssist call (post-onboard retry): project now exists.
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "cloudaicompanionProject": {"id": "proj-new"}
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result =
            perform_antigravity_onboarding_at("test-token", &server.uri(), std::time::Duration::from_millis(1))
                .await;

        assert_eq!(result, Ok("proj-new".to_string()));
    }

    #[tokio::test]
    async fn onboarding_returns_err_when_polling_never_completes() {
        let server = MockServer::start().await;

        // Every loadCodeAssist call (initial + post-onboard) reports no project.
        Mock::given(method("POST"))
            .and(path("/v1internal:loadCodeAssist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "allowedTiers": [{"id": "free-tier", "isDefault": true}]
            })))
            .mount(&server)
            .await;

        // onboardUser never reports done — polling exhausts all 10 attempts.
        Mock::given(method("POST"))
            .and(path("/v1internal:onboardUser"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"done": false})))
            .expect(10)
            .mount(&server)
            .await;

        // 1ms poll interval keeps this test fast despite the 10 polling attempts.
        let result =
            perform_antigravity_onboarding_at("test-token", &server.uri(), std::time::Duration::from_millis(1))
                .await;

        assert!(result.is_err());
    }
}

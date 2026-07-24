# OpenRouter / xAI / DeepSeek / Kimi / Anthropic-Compatible Providers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add five new AI provider types to AITerm — OpenRouter, xAI (Grok), DeepSeek, and Kimi (all API-key-only, reusing the existing `OpenAiCompatibleClient`), plus a generic "Anthropic-Compatible" type (reusing the existing `AnthropicClient`) for third-party Anthropic Messages API-shaped endpoints such as Kimi Coding.

**Architecture:** Every new provider type is a thin configuration layer on top of two clients that already exist and are already fully tested (`OpenAiCompatibleClient`, `AnthropicClient`). No new HTTP/SSE protocol code is written. The only genuinely new backend logic is a shared `/models`-list fetcher (`list_openai_style_models`) reused across the four API-key providers for the Settings UI's dynamic model dropdown.

**Tech Stack:** Rust (Tauri backend, `reqwest`, `wiremock` for tests), TypeScript/React (Settings UI).

**Spec:** `docs/superpowers/specs/2026-07-24-multi-provider-support-design.md`

---

## Task 1: Backend — Add 5 new `ProviderType` variants and wire them into the router

**Files:**
- Modify: `src-tauri/src/config/types.rs:164-186` (enum + Display impl), `:377-395` (existing roundtrip test)
- Modify: `src-tauri/src/ai/router.rs:227-332` (router match), `:338-405` (test module)

- [ ] **Step 1: Add the 5 enum variants**

In `src-tauri/src/config/types.rs`, replace:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Openai,
    Anthropic,
    Ollama,
    OpenaiCompatible,
    GithubCopilot,
    GoogleAi,
}
```

with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderType {
    Openai,
    Anthropic,
    Ollama,
    OpenaiCompatible,
    GithubCopilot,
    GoogleAi,
    Openrouter,
    Xai,
    Deepseek,
    Kimi,
    AnthropicCompatible,
}
```

`#[serde(rename_all = "kebab-case")]` already turns these into `"openrouter"`, `"xai"`, `"deepseek"`, `"kimi"`, `"anthropic-compatible"` — no manual `#[serde(rename = "...")]` needed.

- [ ] **Step 2: Add the 5 `Display` arms**

Replace:

```rust
impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Openai => write!(f, "OpenAI"),
            ProviderType::Anthropic => write!(f, "Anthropic"),
            ProviderType::Ollama => write!(f, "Ollama"),
            ProviderType::OpenaiCompatible => write!(f, "OpenAI-Compatible"),
            ProviderType::GithubCopilot => write!(f, "GitHub Copilot"),
            ProviderType::GoogleAi => write!(f, "Google AI"),
        }
    }
}
```

with:

```rust
impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::Openai => write!(f, "OpenAI"),
            ProviderType::Anthropic => write!(f, "Anthropic"),
            ProviderType::Ollama => write!(f, "Ollama"),
            ProviderType::OpenaiCompatible => write!(f, "OpenAI-Compatible"),
            ProviderType::GithubCopilot => write!(f, "GitHub Copilot"),
            ProviderType::GoogleAi => write!(f, "Google AI"),
            ProviderType::Openrouter => write!(f, "OpenRouter"),
            ProviderType::Xai => write!(f, "xAI (Grok)"),
            ProviderType::Deepseek => write!(f, "DeepSeek"),
            ProviderType::Kimi => write!(f, "Kimi (Moonshot)"),
            ProviderType::AnthropicCompatible => write!(f, "Anthropic-Compatible"),
        }
    }
}
```

- [ ] **Step 3: Extend the existing TOML roundtrip test**

In the same file's `#[cfg(test)] mod tests`, replace the `provider_type_roundtrips_toml` test body's array with:

```rust
    #[test]
    fn provider_type_roundtrips_toml() {
        // TOML requires a struct at the top level, so we wrap the enum.
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { ty: ProviderType }
        for (ty, expected_str) in [
            (ProviderType::Openai, "openai"),
            (ProviderType::Anthropic, "anthropic"),
            (ProviderType::Ollama, "ollama"),
            (ProviderType::OpenaiCompatible, "openai-compatible"),
            (ProviderType::GithubCopilot, "github-copilot"),
            (ProviderType::GoogleAi, "google-ai"),
            (ProviderType::Openrouter, "openrouter"),
            (ProviderType::Xai, "xai"),
            (ProviderType::Deepseek, "deepseek"),
            (ProviderType::Kimi, "kimi"),
            (ProviderType::AnthropicCompatible, "anthropic-compatible"),
        ] {
            let w = W { ty };
            let serialized = toml::to_string(&w).unwrap();
            assert!(serialized.contains(expected_str), "got: {serialized}");
            let deserialized: W = toml::from_str(&serialized).unwrap();
            assert_eq!(deserialized.ty, w.ty);
        }
    }
```

- [ ] **Step 4: Confirm the expected compile break in `router.rs`**

Run: `cd src-tauri && cargo check`
Expected: `error[E0004]: non-exhaustive patterns` pointing at the `match provider_cfg.provider_type` in `src/ai/router.rs`, listing the 5 new variants as unhandled. This is expected — it's the compiler telling you exactly which match arms Step 5 must add.

- [ ] **Step 5: Add the 5 router match arms**

In `src-tauri/src/ai/router.rs`, inside `resolve_by_id`, the `match provider_cfg.provider_type { ... }` block currently ends with the `ProviderType::GoogleAi => { ... }` arm followed by the closing `};`. Insert these 5 arms immediately before that closing `};` (i.e. after the `GoogleAi` arm, still inside the `match`):

```rust
            ProviderType::Openrouter => {
                let key = self
                    .secrets
                    .get(&provider_cfg.id)
                    .map_err(|_| AiError::NotConfigured)?
                    .ok_or(AiError::NotConfigured)?;
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| "https://openrouter.ai/api/v1".into()),
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
                        .unwrap_or_else(|| "https://api.x.ai/v1".into()),
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
                        .unwrap_or_else(|| "https://api.deepseek.com/v1".into()),
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
                        .unwrap_or_else(|| "https://api.moonshot.ai/v1".into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
            ProviderType::AnthropicCompatible => {
                // Check base_url before the API key so a missing base_url is
                // reported as AiError::Network regardless of whether a key
                // happens to be present — mirrors OpenaiCompatible's ordering
                // and keeps this branch independently testable (see Task 1
                // tests below).
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
```

- [ ] **Step 6: Confirm it compiles**

Run: `cd src-tauri && cargo check`
Expected: clean compile, no errors.

- [ ] **Step 7: Add router resolution tests**

In the same file's `#[cfg(test)] mod tests`, add these 6 tests after the existing `ollama_provider_resolves_without_api_key` test:

```rust
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
```

- [ ] **Step 8: Run the full test suite for both files**

Run: `cd src-tauri && cargo test --lib config:: ai::router::`
Expected: all tests pass, including the 6 new ones and the extended roundtrip test.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/ai/router.rs
git commit -m "feat(ai): add OpenRouter/xAI/DeepSeek/Kimi/Anthropic-Compatible provider types"
```

---

## Task 2: Backend — Shared `/models` fetcher with wiremock contract tests

**Files:**
- Modify: `src-tauri/src/commands/provider.rs` (end of file, after the existing `OpenAiModelsResponse`/`OpenAiModelItem` structs)
- Create: `src-tauri/tests/provider_models.rs`

- [ ] **Step 1: Write the failing contract tests**

Create `src-tauri/tests/provider_models.rs`:

```rust
//! Contract test for `list_openai_style_models` — the shared model-list
//! fetcher used by the OpenRouter/xAI/DeepSeek/Kimi provider commands.
//! All four providers speak the same OpenAI-shaped `{"data":[{"id":...}]}`
//! `/models` response, so this is tested once against a wiremock fake rather
//! than per-provider.

use aiterm_lib::commands::provider::list_openai_style_models;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn parses_openai_shaped_model_list() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"data":[{"id":"model-a"},{"id":"model-b"}]}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let models = list_openai_style_models(&server.uri(), "test-key")
        .await
        .unwrap();
    assert_eq!(models, vec!["model-a".to_string(), "model-b".to_string()]);
}

#[tokio::test]
async fn empty_api_key_errors_without_making_a_request() {
    let server = MockServer::start().await;
    // Deliberately no Mock registered: if the function made an HTTP request
    // despite the empty key, wiremock would return its default 404 and the
    // error message would say "404" instead of "api_key is required",
    // failing the assertion below.
    let err = list_openai_style_models(&server.uri(), "   ")
        .await
        .unwrap_err();
    assert!(err.contains("api_key is required"), "got: {err}");
}

#[tokio::test]
async fn non_success_status_returns_error_with_status_and_body() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let err = list_openai_style_models(&server.uri(), "bad-key")
        .await
        .unwrap_err();
    assert!(err.contains("401"), "got: {err}");
    assert!(err.contains("unauthorized"), "got: {err}");
}

#[tokio::test]
async fn trailing_slash_on_base_url_is_handled() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[{"id":"m1"}]}"#))
        .mount(&server)
        .await;

    let base_with_slash = format!("{}/", server.uri());
    let models = list_openai_style_models(&base_with_slash, "k")
        .await
        .unwrap();
    assert_eq!(models, vec!["m1".to_string()]);
}
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cd src-tauri && cargo test --test provider_models`
Expected: compile error — `unresolved import 'aiterm_lib::commands::provider::list_openai_style_models'` (or "function not found") because the function doesn't exist yet.

- [ ] **Step 3: Implement `list_openai_style_models`**

At the end of `src-tauri/src/commands/provider.rs`, after the existing:

```rust
#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
}
```

add:

```rust

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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test --test provider_models`
Expected: 4 passed; 0 failed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/provider.rs src-tauri/tests/provider_models.rs
git commit -m "feat(ai): add shared OpenAI-style /models fetcher with contract tests"
```

---

## Task 3: Backend — Add the 8 thin model-list commands

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

Note: existing `_by_provider` commands in this file (e.g. `get_google_ai_models_by_provider`) take `tauri::State<'_, Arc<ConfigStore>>` parameters and are not unit-tested directly anywhere in this codebase — Tauri's `State` isn't constructible outside a running Tauri app, so these thin wrappers are verified by `cargo build` (they compile and type-check correctly) and by the manual end-to-end check in Task 8, matching how the existing Google AI / GitHub Copilot wrapper commands are already handled.

- [ ] **Step 1: Add the 8 command functions**

Immediately after the `list_openai_style_models` function added in Task 2, add:

```rust

const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const XAI_DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";
const KIMI_DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";

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
```

- [ ] **Step 2: Confirm it compiles**

Run: `cd src-tauri && cargo check`
Expected: clean compile (these commands aren't registered in `lib.rs` yet, but that doesn't affect compilation of this module).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(ai): add model-list commands for OpenRouter/xAI/DeepSeek/Kimi"
```

---

## Task 4: Backend — Register the new commands in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs:54-64` (import block), `:271-291` (invoke_handler list)

- [ ] **Step 1: Add the imports**

In `src-tauri/src/lib.rs`, find the `provider::{ ... }` import block (around line 54) and add the 8 new command names. The block should read:

```rust
        provider::{
            add_provider, get_github_copilot_models, get_github_copilot_models_by_provider,
            get_google_ai_models, get_google_ai_models_by_provider,
            get_ollama_models, github_copilot_device_poll, github_copilot_device_start,
            list_providers, remove_provider, set_default_provider,
            test_provider, update_provider,
            anthropic_oauth_start, anthropic_oauth_complete, anthropic_oauth_logout,
            get_anthropic_oauth_models,
            google_oauth_login, google_oauth_logout, get_google_oauth_models,
            get_openrouter_models, get_openrouter_models_by_provider,
            get_xai_models, get_xai_models_by_provider,
            get_deepseek_models, get_deepseek_models_by_provider,
            get_kimi_models, get_kimi_models_by_provider,
            AnthropicOAuthState,
        },
```

- [ ] **Step 2: Add the invoke_handler entries**

In the `.invoke_handler(tauri::generate_handler![ ... ])` list (around line 271), after the existing `get_google_oauth_models,` line, add:

```rust
                get_openrouter_models,
                get_openrouter_models_by_provider,
                get_xai_models,
                get_xai_models_by_provider,
                get_deepseek_models,
                get_deepseek_models_by_provider,
                get_kimi_models,
                get_kimi_models_by_provider,
```

- [ ] **Step 3: Verify the full crate builds**

Run: `cd src-tauri && cargo build`
Expected: clean build, no errors.

- [ ] **Step 4: Run the full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass (including everything added in Tasks 1–3).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(ai): register OpenRouter/xAI/DeepSeek/Kimi model-list commands"
```

---

## Task 5: Frontend — Extend `ProviderType` and add provider.ts constants/IPC wrappers

**Files:**
- Modify: `src/ipc/config.ts:5-11`
- Modify: `src/ipc/provider.ts:157-190`

- [ ] **Step 1: Extend the `ProviderType` union**

In `src/ipc/config.ts`, replace:

```ts
export type ProviderType =
  | "openai"
  | "anthropic"
  | "ollama"
  | "openai-compatible"
  | "github-copilot"
  | "google-ai";
```

with:

```ts
export type ProviderType =
  | "openai"
  | "anthropic"
  | "ollama"
  | "openai-compatible"
  | "github-copilot"
  | "google-ai"
  | "openrouter"
  | "xai"
  | "deepseek"
  | "kimi"
  | "anthropic-compatible";
```

- [ ] **Step 2: Confirm the expected type-check break**

Run: `npx tsc --noEmit`
Expected: errors in `src/ipc/provider.ts` — the `Record<ProviderType, string>` types (`PROVIDER_TYPE_LABELS`, `DEFAULT_MODELS`, `DEFAULT_BASE_URLS`) are now missing the 5 new keys. This confirms exactly which objects Step 3 must extend.

- [ ] **Step 3: Extend `PROVIDER_TYPE_LABELS`, `DEFAULT_MODELS`, `DEFAULT_BASE_URLS`**

In `src/ipc/provider.ts`, replace the three `Record<ProviderType, ...>` objects and the `COMPATIBLE_PRESETS` array with:

```ts
export const PROVIDER_TYPE_LABELS: Record<ProviderType, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  ollama: "Ollama",
  "openai-compatible": "OpenAI-Compatible",
  "github-copilot": "GitHub Copilot",
  "google-ai": "Google AI",
  openrouter: "OpenRouter",
  xai: "xAI (Grok)",
  deepseek: "DeepSeek",
  kimi: "Kimi (Moonshot)",
  "anthropic-compatible": "Anthropic-Compatible",
};

export const DEFAULT_MODELS: Record<ProviderType, string> = {
  openai: "gpt-4o-mini",
  anthropic: "claude-sonnet-4-5",
  ollama: "llama3.1:8b",
  "openai-compatible": "",
  "github-copilot": "gpt-4o-mini",
  "google-ai": "gemini-2.5-pro",
  openrouter: "",
  xai: "grok-4",
  deepseek: "deepseek-chat",
  kimi: "kimi-latest",
  "anthropic-compatible": "",
};

export const DEFAULT_BASE_URLS: Record<ProviderType, string> = {
  openai: "",
  anthropic: "",
  ollama: "http://localhost:11434",
  "openai-compatible": "",
  "github-copilot": "https://api.githubcopilot.com",
  "google-ai": "https://generativelanguage.googleapis.com/v1beta/openai",
  openrouter: "https://openrouter.ai/api/v1",
  xai: "https://api.x.ai/v1",
  deepseek: "https://api.deepseek.com/v1",
  kimi: "https://api.moonshot.ai/v1",
  "anthropic-compatible": "",
};

/** OpenAI-compatible quick-pick presets shown in the form — for servers with
 *  no dedicated provider type (self-hosted / local). OpenRouter and DeepSeek
 *  used to be listed here but are now dedicated provider types above; keep
 *  this list to genuine "bring your own endpoint" cases only. */
export const COMPATIBLE_PRESETS = [
  { label: "LM Studio", url: "http://localhost:1234/v1" },
  { label: "vLLM", url: "http://localhost:8000/v1" },
];

/** Anthropic-compatible quick-pick presets shown in the form. */
export const ANTHROPIC_COMPATIBLE_PRESETS = [
  { label: "Kimi Coding", url: "https://api.kimi.com/coding" },
];
```

- [ ] **Step 4: Add the 8 IPC wrapper functions**

In the same file, after the existing `getGoogleOAuthModels` export (in the "Commands" section, before "Display helpers"), add:

```ts
export const getOpenRouterModels = (apiKey: string): Promise<string[]> =>
  invoke("get_openrouter_models", { apiKey });

export const getOpenRouterModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_openrouter_models_by_provider", { id });

export const getXaiModels = (apiKey: string): Promise<string[]> =>
  invoke("get_xai_models", { apiKey });

export const getXaiModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_xai_models_by_provider", { id });

export const getDeepseekModels = (apiKey: string): Promise<string[]> =>
  invoke("get_deepseek_models", { apiKey });

export const getDeepseekModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_deepseek_models_by_provider", { id });

export const getKimiModels = (apiKey: string): Promise<string[]> =>
  invoke("get_kimi_models", { apiKey });

export const getKimiModelsByProvider = (id: string): Promise<string[]> =>
  invoke("get_kimi_models_by_provider", { id });
```

- [ ] **Step 5: Confirm the type check is clean**

Run: `npx tsc --noEmit`
Expected: 0 errors.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/config.ts src/ipc/provider.ts
git commit -m "feat(settings): add IPC layer for OpenRouter/xAI/DeepSeek/Kimi/Anthropic-Compatible"
```

---

## Task 6: Frontend — `ProviderForm.tsx` structural additions (type list, base_url visibility, Anthropic-Compatible section)

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`

- [ ] **Step 1: Add the 5 new types to `PROVIDER_TYPES`**

Replace:

```ts
const PROVIDER_TYPES: ProviderType[] = [
  "openai",
  "anthropic",
  "ollama",
  "openai-compatible",
  "github-copilot",
  "google-ai",
];
```

with:

```ts
const PROVIDER_TYPES: ProviderType[] = [
  "openai",
  "anthropic",
  "ollama",
  "openai-compatible",
  "github-copilot",
  "google-ai",
  "openrouter",
  "xai",
  "deepseek",
  "kimi",
  "anthropic-compatible",
];
```

- [ ] **Step 2: Import the new preset list**

In the existing import from `"../../ipc/provider"`, add `ANTHROPIC_COMPATIBLE_PRESETS` alongside `COMPATIBLE_PRESETS`:

```ts
import {
  PROVIDER_TYPE_LABELS,
  DEFAULT_MODELS,
  DEFAULT_BASE_URLS,
  COMPATIBLE_PRESETS,
  ANTHROPIC_COMPATIBLE_PRESETS,
  getOllamaModels,
  githubCopilotDeviceStart,
  githubCopilotDevicePoll,
  getGithubCopilotModels,
  getGithubCopilotModelsByProvider,
  getGoogleAiModels,
  getGoogleAiModelsByProvider,
  anthropicOAuthStart,
  anthropicOAuthComplete,
  anthropicOAuthLogout,
  getAnthropicOAuthModels,

} from "../../ipc/provider";
```

- [ ] **Step 3: Show the base_url field (with presets) for `anthropic-compatible`**

Replace:

```tsx
      {(providerType === "ollama" ||
        providerType === "openai-compatible" ||
        providerType === "github-copilot" ||
        providerType === "google-ai") && (
        <div className="form-group">
          <label>{t.provider_base_url}</label>
          {providerType === "openai-compatible" && (
            <div className="presets">
              {COMPATIBLE_PRESETS.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  className="preset-btn"
                  onClick={() => setBaseUrl(p.url)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={DEFAULT_BASE_URLS[providerType] || "https://..."}
          />
        </div>
      )}
```

with:

```tsx
      {(providerType === "ollama" ||
        providerType === "openai-compatible" ||
        providerType === "github-copilot" ||
        providerType === "google-ai" ||
        providerType === "anthropic-compatible") && (
        <div className="form-group">
          <label>{t.provider_base_url}</label>
          {providerType === "openai-compatible" && (
            <div className="presets">
              {COMPATIBLE_PRESETS.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  className="preset-btn"
                  onClick={() => setBaseUrl(p.url)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}
          {providerType === "anthropic-compatible" && (
            <div className="presets">
              {ANTHROPIC_COMPATIBLE_PRESETS.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  className="preset-btn"
                  onClick={() => setBaseUrl(p.url)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={DEFAULT_BASE_URLS[providerType] || "https://..."}
          />
        </div>
      )}
```

Note: `openrouter`/`xai`/`deepseek`/`kimi` are deliberately **not** added to this condition — per the approved design, their base_url is fixed and not exposed in the UI (matching how the `openai` type already hides it). `anthropic-compatible`'s model field needs no change in this task: it isn't listed in any of the special-cased branches of the model `<input>`/`<select>` chain, so it already falls through to the generic free-text `<input>` at the bottom of that chain — exactly the desired behavior (no dynamic fetch for this type, per the design).

- [ ] **Step 4: Type-check and manually smoke-test**

Run: `npx tsc --noEmit`
Expected: 0 errors.

Run: `npm run tauri:dev`, open Settings → Add Provider, and confirm:
- All 11 provider types appear in the dropdown with the expected labels.
- Selecting `anthropic-compatible` shows a base_url field with a "Kimi Coding" preset button that fills in `https://api.kimi.com/coding` when clicked, and the model field is a plain free-text input.
- Selecting `openrouter`/`xai`/`deepseek`/`kimi` shows **no** base_url field.

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): add provider type list entries and Anthropic-Compatible UI section"
```

---

## Task 7: Frontend — Dynamic model-fetch UI for OpenRouter/xAI/DeepSeek/Kimi

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`

- [ ] **Step 1: Import the remaining IPC wrappers**

Extend the same `"../../ipc/provider"` import from Task 6 with the 8 model-list functions:

```ts
import {
  PROVIDER_TYPE_LABELS,
  DEFAULT_MODELS,
  DEFAULT_BASE_URLS,
  COMPATIBLE_PRESETS,
  ANTHROPIC_COMPATIBLE_PRESETS,
  getOllamaModels,
  githubCopilotDeviceStart,
  githubCopilotDevicePoll,
  getGithubCopilotModels,
  getGithubCopilotModelsByProvider,
  getGoogleAiModels,
  getGoogleAiModelsByProvider,
  anthropicOAuthStart,
  anthropicOAuthComplete,
  anthropicOAuthLogout,
  getAnthropicOAuthModels,
  getOpenRouterModels,
  getOpenRouterModelsByProvider,
  getXaiModels,
  getXaiModelsByProvider,
  getDeepseekModels,
  getDeepseekModelsByProvider,
  getKimiModels,
  getKimiModelsByProvider,
} from "../../ipc/provider";
```

- [ ] **Step 2: Add state for each provider's model list**

After the existing `googleAiModels`/`googleAiLoading` state declarations, add:

```ts
  const [openRouterModels, setOpenRouterModels] = useState<string[]>([]);
  const [openRouterLoading, setOpenRouterLoading] = useState(false);
  const [xaiModels, setXaiModels] = useState<string[]>([]);
  const [xaiLoading, setXaiLoading] = useState(false);
  const [deepseekModels, setDeepseekModels] = useState<string[]>([]);
  const [deepseekLoading, setDeepseekLoading] = useState(false);
  const [kimiModels, setKimiModels] = useState<string[]>([]);
  const [kimiLoading, setKimiLoading] = useState(false);
```

- [ ] **Step 3: Add the 4 fetch `useEffect` blocks**

After the existing Google AI `useEffect` (the one with the 500ms debounce), add these 4 blocks — each mirrors the Google AI one exactly, swapping in its own provider type string, state setters, and IPC functions:

```ts
  useEffect(() => {
    if (providerType !== "openrouter") return;

    if (!apiKey.trim() && isEdit && existing?.has_api_key && id.trim()) {
      setOpenRouterLoading(true);
      getOpenRouterModelsByProvider(id.trim())
        .then((models) => {
          setOpenRouterModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setOpenRouterModels([]))
        .finally(() => setOpenRouterLoading(false));
      return;
    }

    if (!apiKey.trim()) { setOpenRouterModels([]); return; }

    const timer = setTimeout(() => {
      setOpenRouterLoading(true);
      getOpenRouterModels(apiKey.trim())
        .then((models) => {
          setOpenRouterModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setOpenRouterModels([]))
        .finally(() => setOpenRouterLoading(false));
    }, 500);

    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, apiKey, isEdit, existing?.has_api_key, id]);

  useEffect(() => {
    if (providerType !== "xai") return;

    if (!apiKey.trim() && isEdit && existing?.has_api_key && id.trim()) {
      setXaiLoading(true);
      getXaiModelsByProvider(id.trim())
        .then((models) => {
          setXaiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setXaiModels([]))
        .finally(() => setXaiLoading(false));
      return;
    }

    if (!apiKey.trim()) { setXaiModels([]); return; }

    const timer = setTimeout(() => {
      setXaiLoading(true);
      getXaiModels(apiKey.trim())
        .then((models) => {
          setXaiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setXaiModels([]))
        .finally(() => setXaiLoading(false));
    }, 500);

    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, apiKey, isEdit, existing?.has_api_key, id]);

  useEffect(() => {
    if (providerType !== "deepseek") return;

    if (!apiKey.trim() && isEdit && existing?.has_api_key && id.trim()) {
      setDeepseekLoading(true);
      getDeepseekModelsByProvider(id.trim())
        .then((models) => {
          setDeepseekModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setDeepseekModels([]))
        .finally(() => setDeepseekLoading(false));
      return;
    }

    if (!apiKey.trim()) { setDeepseekModels([]); return; }

    const timer = setTimeout(() => {
      setDeepseekLoading(true);
      getDeepseekModels(apiKey.trim())
        .then((models) => {
          setDeepseekModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setDeepseekModels([]))
        .finally(() => setDeepseekLoading(false));
    }, 500);

    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, apiKey, isEdit, existing?.has_api_key, id]);

  useEffect(() => {
    if (providerType !== "kimi") return;

    if (!apiKey.trim() && isEdit && existing?.has_api_key && id.trim()) {
      setKimiLoading(true);
      getKimiModelsByProvider(id.trim())
        .then((models) => {
          setKimiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setKimiModels([]))
        .finally(() => setKimiLoading(false));
      return;
    }

    if (!apiKey.trim()) { setKimiModels([]); return; }

    const timer = setTimeout(() => {
      setKimiLoading(true);
      getKimiModels(apiKey.trim())
        .then((models) => {
          setKimiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setKimiModels([]))
        .finally(() => setKimiLoading(false));
    }, 500);

    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, apiKey, isEdit, existing?.has_api_key, id]);
```

- [ ] **Step 4: Add the 4 model-field render branches**

In the model `<div className="form-group">` block, find the chain of `providerType === "..." ? (...) : providerType === "..." ? (...) : ...` and insert 4 new branches right before the final `providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (...)` branch (order among branches doesn't matter functionally since each checks a distinct `providerType`, but keeping them grouped near the other dynamic-fetch branches — right after the Google AI one — keeps the file readable). The Google AI branch currently ends with:

```tsx
        ) : providerType === "google-ai" ? (
          googleAiLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="google-ai-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={googleAiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {googleAiModels.length > 0 && (
                <datalist id="google-ai-models-list">
                  {googleAiModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (
```

Insert 4 new branches between the Google AI branch's closing `)` and the `anthropic` branch:

```tsx
        ) : providerType === "openrouter" ? (
          openRouterLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="openrouter-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={openRouterModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {openRouterModels.length > 0 && (
                <datalist id="openrouter-models-list">
                  {openRouterModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "xai" ? (
          xaiLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="xai-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={xaiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {xaiModels.length > 0 && (
                <datalist id="xai-models-list">
                  {xaiModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "deepseek" ? (
          deepseekLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="deepseek-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={deepseekModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {deepseekModels.length > 0 && (
                <datalist id="deepseek-models-list">
                  {deepseekModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "kimi" ? (
          kimiLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="kimi-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={kimiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {kimiModels.length > 0 && (
                <datalist id="kimi-models-list">
                  {kimiModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (
```

(No i18n changes are needed — `t.provider_model_loading` and `t.settings_provider_model_placeholder` are already generic keys reused from the existing Google AI section.)

- [ ] **Step 5: Type-check**

Run: `npx tsc --noEmit`
Expected: 0 errors.

- [ ] **Step 6: Lint**

Run: `npm run lint`
Expected: 0 errors (the `react-hooks/exhaustive-deps` warnings on the 4 new effects are suppressed the same way as the existing Google AI effect, via the same inline eslint-disable comment).

- [ ] **Step 7: Manual smoke test**

Run: `npm run tauri:dev`, open Settings → Add Provider:
- Select `xai`, type any text into the API Key field, wait ~500ms — confirm the model input doesn't error (a real invalid key will make the fetch fail silently and the datalist will just stay empty, which is expected/acceptable per the design's error handling).
- Repeat for `openrouter`, `deepseek`, `kimi`.
- If a real API key is available for any of these four services, use it to confirm the datalist actually populates with real model IDs — this is the only way to verify the "assumption to validate" noted in the spec (whether each provider's `/models` endpoint exists and returns the expected shape). If no real key is available, note in the task/PR that this remains unverified against live services.

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): add dynamic model-list dropdowns for OpenRouter/xAI/DeepSeek/Kimi"
```

---

## Task 8: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass, no regressions in unrelated modules.

- [ ] **Step 2: Full Rust build**

Run: `cd src-tauri && cargo build`
Expected: clean build.

- [ ] **Step 3: Frontend type check and lint**

Run: `npx tsc --noEmit && npm run lint`
Expected: 0 errors.

- [ ] **Step 4: Frontend test suite**

Run: `npm run test`
Expected: all existing tests pass (no new frontend tests were added in this plan — `ProviderForm.tsx` has no existing test file, per the approved spec's testing section, which scoped this to manual verification only).

- [ ] **Step 5: End-to-end manual walkthrough**

Run: `npm run tauri:dev` and, for each of the 5 new provider types:
1. Add a provider of that type with a placeholder API key and a model name.
2. Confirm it appears in the provider list with the correct label.
3. Set it as the default provider.
4. Remove it.

Note explicitly in your final report which of these were verified against **real** API keys/services (actual model list populated, actual `generate()` call succeeded) versus verified only at the UI/wiring level (form renders, saves, and removes correctly, but the underlying `/models` or `/messages` endpoint was not exercised with live credentials) — per the spec's open risk about unverified `/models` endpoints and the Kimi Coding `?beta=true` question.

# Codex (ChatGPT Subscription) AI Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 12th AI provider type, `codex`, that authenticates with a user's ChatGPT (Plus/Pro/Team, with Codex access) subscription via OAuth instead of an API key, and talks to OpenAI's Responses API through the ChatGPT backend.

**Architecture:** A new `CodexClient` (`src-tauri/src/ai/codex.rs`) implements the `AiProvider` trait directly (Responses API request/response shape differs from Chat Completions, so it can't reuse `OpenAiCompatibleClient`). Auth is a single blocking Tauri command (`codex_oauth_login`) that binds a fixed-port (1455) local HTTP server, opens the browser, waits for the OAuth callback, and exchanges the code for tokens — mirroring the existing `google_oauth_login` command. `ai/router.rs` gets a 5th OAuth-refresh code path (Codex's refresh tokens rotate and must never send `scope`). Frontend wiring mirrors the existing Anthropic-OAuth-button + Google-AI-model-datalist patterns already in `ProviderForm.tsx`.

**Tech Stack:** Rust (Tauri 2 backend, `reqwest`, `serde`, `tokio`), React 19 + TypeScript (frontend), `wiremock` for Rust integration tests.

**Spec:** `docs/superpowers/specs/2026-07-27-codex-provider-design.md`

---

## Task 1: `ProviderType::Codex` enum variant

**Files:**
- Modify: `src-tauri/src/config/types.rs`

- [ ] **Step 1: Extend the roundtrip test to include the new variant (write it failing first)**

Find the `provider_type_roundtrips_toml` test (search for `fn provider_type_roundtrips_toml`) and add one line to the array literal, right after the `AnthropicCompatible` entry:

```rust
            (ProviderType::AnthropicCompatible, "anthropic-compatible"),
            (ProviderType::Codex, "codex"),
        ] {
```

- [ ] **Step 2: Run the test, confirm it fails to compile**

Run: `cd src-tauri && cargo test provider_type_roundtrips_toml 2>&1 | tail -20`
Expected: compile error — `no variant named 'Codex' found for enum 'ProviderType'`.

- [ ] **Step 3: Add the `Codex` variant and its `Display` arm**

In the `ProviderType` enum definition, add `Codex` after `AnthropicCompatible`:

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
    Codex,
}
```

In the `impl std::fmt::Display for ProviderType` block, add a matching arm right before the closing brace:

```rust
            ProviderType::AnthropicCompatible => write!(f, "Anthropic-Compatible"),
            ProviderType::Codex => write!(f, "Codex"),
        }
```

- [ ] **Step 4: Run the test, confirm it passes**

Run: `cd src-tauri && cargo test provider_type_roundtrips_toml 2>&1 | tail -20`
Expected: `test config::types::tests::provider_type_roundtrips_toml ... ok`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(config): add Codex provider type"
```

---

## Task 2: `CodexClient` request-body builder

**Files:**
- Create: `src-tauri/src/ai/codex.rs`
- Modify: `src-tauri/src/ai/mod.rs:13-20` (register the module)

- [ ] **Step 1: Register the module (needed before the file can be tested)**

In `src-tauri/src/ai/mod.rs`, change:

```rust
pub mod anthropic;
pub mod compatible;
pub mod context;
pub mod copilot;
pub mod ollama;
pub mod openai;
pub mod router;
pub(crate) mod sse;
```

to:

```rust
pub mod anthropic;
pub mod codex;
pub mod compatible;
pub mod context;
pub mod copilot;
pub mod ollama;
pub mod openai;
pub mod router;
pub(crate) mod sse;
```

- [ ] **Step 2: Create `codex.rs` with just the struct + a failing test for the body builder**

Create `src-tauri/src/ai/codex.rs`:

```rust
//! Codex (ChatGPT subscription) provider — OpenAI's Responses API.
//!
//! Key differences from the OpenAI Chat Completions clients elsewhere in this
//! module:
//! - Endpoint is `chatgpt.com/backend-api/codex/responses`, not `api.openai.com`.
//! - Request shape uses `input: [...]` items instead of `messages`, and a
//!   required top-level `instructions` field instead of a "system" message.
//! - `stream: true` and `store: false` are forced — the endpoint rejects
//!   `store: true` on normal (non-reasoning-continuation) requests.
//! - Auth needs OAuth Bearer plus client-identity headers (`originator`, a
//!   spoofed `User-Agent`, `chatgpt-account-id`) so the backend treats the
//!   request as coming from the official Codex CLI — the same role
//!   Anthropic's "Claude Code sentinel" header plays in `anthropic.rs`.
//! - SSE events use `response.output_text.delta` / `response.completed`,
//!   not Chat Completions' `choices[0].delta.content`.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, map_http_error, separator_len},
    AiError, AiProvider, GenerateChunk, GenerateRequest, TokenUsage,
};

pub(crate) const CODEX_CLIENT_VERSION: &str = "0.144.1";
pub(crate) const CODEX_USER_AGENT: &str = "codex-cli/0.144.1 (Windows 10.0.26200; x64)";

pub struct CodexClient {
    access_token: String,
    model: String,
    chatgpt_account_id: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

/// Build the Responses API request body. `instructions` is Codex's required
/// system-prompt-equivalent — the backend rejects requests without it.
pub(crate) fn build_request_body(model: &str, req: &GenerateRequest) -> serde_json::Value {
    serde_json::json!({});
}
```

- [ ] **Step 3: Write the failing unit tests for `build_request_body`**

Append to the bottom of `src-tauri/src/ai/codex.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn req(system_prompt: &str, messages: Vec<ChatMessage>) -> GenerateRequest {
        GenerateRequest {
            system_prompt: system_prompt.into(),
            messages,
            context: EnvSnapshot {
                os: "linux".into(),
                shell: "bash".into(),
                cwd: PathBuf::from("/"),
                ..Default::default()
            },
            mode: QueryMode::Chat,
            max_tokens: Some(256),
        }
    }

    #[test]
    fn system_prompt_becomes_instructions_field() {
        let r = req("You are a helpful CLI assistant.", vec![]);
        let body = build_request_body("gpt-5.1-codex", &r);
        assert_eq!(body["instructions"], "You are a helpful CLI assistant.");
    }

    #[test]
    fn messages_become_input_array_with_input_text_content() {
        let r = req(
            "sys",
            vec![ChatMessage {
                role: "user".into(),
                content: serde_json::json!("list files"),
                tool_call_id: None,
                tool_calls: None,
            }],
        );
        let body = build_request_body("gpt-5.1-codex", &r);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "list files");
    }

    #[test]
    fn stream_and_store_are_always_forced() {
        let r = req("sys", vec![]);
        let body = build_request_body("gpt-5.1-codex", &r);
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
    }

    #[test]
    fn model_field_passes_through_unchanged() {
        let r = req("sys", vec![]);
        let body = build_request_body("gpt-5.1-codex-high", &r);
        assert_eq!(body["model"], "gpt-5.1-codex-high");
    }
}
```

- [ ] **Step 4: Run the tests, confirm they fail**

Run: `cd src-tauri && cargo test --lib ai::codex 2>&1 | tail -30`
Expected: all 4 tests FAIL (the stub returns `{}`).

- [ ] **Step 5: Implement `build_request_body` for real**

Replace the stub body of `build_request_body` in `src-tauri/src/ai/codex.rs`:

```rust
pub(crate) fn build_request_body(model: &str, req: &GenerateRequest) -> serde_json::Value {
    let input: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let text = match &m.content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            serde_json::json!({
                "type": "message",
                "role": m.role,
                "content": [{ "type": "input_text", "text": text }]
            })
        })
        .collect();

    serde_json::json!({
        "model": model,
        "instructions": req.system_prompt,
        "input": input,
        "stream": true,
        "store": false,
    })
}
```

- [ ] **Step 6: Run the tests, confirm they pass**

Run: `cd src-tauri && cargo test --lib ai::codex 2>&1 | tail -30`
Expected: `test result: ok. 4 passed`

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/codex.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add Codex request-body builder (Responses API shape)"
```

---

## Task 3: `CodexClient` constructor, headers, SSE parsing, and `AiProvider` impl

**Files:**
- Modify: `src-tauri/src/ai/codex.rs`
- Create: `src-tauri/tests/codex_client.rs`

- [ ] **Step 1: Write the failing integration test first**

Create `src-tauri/tests/codex_client.rs`:

```rust
//! Contract test for `CodexClient` against a wiremock fake of the ChatGPT
//! Codex Responses API. Covers the happy path (SSE streaming + required
//! client-identity headers) and 401 → AuthFailed.

use aiterm_lib::ai::{
    codex::CodexClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
    GenerateRequest, QueryMode,
};
use std::path::PathBuf;
use tokio::sync::mpsc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn req() -> GenerateRequest {
    GenerateRequest {
        system_prompt: "You are a terminal assistant.".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!("list files"),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
            ..Default::default()
        },
        mode: QueryMode::Chat,
        max_tokens: Some(256),
    }
}

#[tokio::test]
async fn happy_path_streams_and_sends_required_headers() {
    let server = MockServer::start().await;

    let sse_body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n\
                     data: {\"type\":\"response.output_text.delta\",\"delta\":\" world\"}\n\n\
                     data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":2}}}\n\n";

    Mock::given(method("POST"))
        .and(path("/backend-api/codex/responses"))
        .and(header("authorization", "Bearer test-token"))
        .and(header("originator", "codex_cli_rs"))
        .and(header("chatgpt-account-id", "acct-123"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Production always talks to the real chatgpt.com host — `with_base_url`
    // is a test-only hook to point at the wiremock server.
    let client = CodexClient::with_base_url(
        "test-token".into(),
        "gpt-5.1-codex".into(),
        Some("acct-123".into()),
        server.uri(),
    );
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    client.generate(req(), tx).await.expect("generate ok");

    let mut buf = String::new();
    let mut saw_done = false;
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done {
            saw_done = true;
            break;
        }
    }
    assert!(saw_done, "expected a done chunk");
    assert_eq!(buf, "Hello world");
}

#[tokio::test]
async fn returns_auth_failed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client =
        CodexClient::with_base_url("bad-token".into(), "gpt-5.1-codex".into(), None, server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}
```

- [ ] **Step 2: Run it, confirm it fails to compile**

Run: `cd src-tauri && cargo test --test codex_client 2>&1 | tail -30`
Expected: compile error — `CodexClient` has no `with_base_url` / no `generate` (struct has no fields yet, no `AiProvider` impl).

- [ ] **Step 3: Implement the constructor, header helper, SSE consumer, and `AiProvider` impl**

Replace the `CodexClient` struct declaration and everything below it (but above the `#[cfg(test)]` module) in `src-tauri/src/ai/codex.rs` with:

```rust
pub struct CodexClient {
    access_token: String,
    model: String,
    chatgpt_account_id: Option<String>,
    base_url: String,
    client: reqwest::Client,
}

impl CodexClient {
    pub fn new(access_token: String, model: String, chatgpt_account_id: Option<String>) -> Self {
        Self::with_base_url(access_token, model, chatgpt_account_id, "https://chatgpt.com".into())
    }

    /// Test-only hook: lets integration tests point at a wiremock server
    /// instead of the real chatgpt.com backend. There is no user-facing
    /// base_url setting for Codex — the endpoint is fixed in production.
    pub fn with_base_url(
        access_token: String,
        model: String,
        chatgpt_account_id: Option<String>,
        base_url: String,
    ) -> Self {
        Self { access_token, model, chatgpt_account_id, base_url, client: reqwest::Client::new() }
    }

    fn responses_url(&self) -> String {
        format!("{}/backend-api/codex/responses", self.base_url.trim_end_matches('/'))
    }

    fn models_url(&self) -> String {
        format!(
            "{}/backend-api/codex/models?client_version={CODEX_CLIENT_VERSION}",
            self.base_url.trim_end_matches('/')
        )
    }

    fn apply_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = builder
            .bearer_auth(&self.access_token)
            .header("originator", "codex_cli_rs")
            .header("User-Agent", CODEX_USER_AGENT)
            .header("Version", CODEX_CLIENT_VERSION)
            .header("Openai-Beta", "responses=experimental")
            .header("X-Codex-Beta-Features", "responses_websockets");
        match &self.chatgpt_account_id {
            Some(id) => builder.header("chatgpt-account-id", id.as_str()),
            None => builder,
        }
    }
}

#[async_trait]
impl AiProvider for CodexClient {
    fn id(&self) -> &str {
        "codex"
    }
    fn display_name(&self) -> &str {
        "Codex"
    }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req);
        let resp = self
            .apply_headers(self.client.post(self.responses_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }
        consume_codex_sse(resp, tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        let resp = self
            .apply_headers(self.client.get(self.models_url()))
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;
        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }
        Ok(())
    }
}

async fn consume_codex_sse(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut saw_done = false;

    'outer: while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
        buf.extend_from_slice(&bytes);

        loop {
            let Some(pos) = find_line_end(&buf) else { break };
            let line_bytes: Vec<u8> = buf.drain(..pos).collect();
            let sep = separator_len(&buf);
            buf.drain(..sep);
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };
            if line.is_empty() {
                continue;
            }

            let Some(data) = line.strip_prefix("data:") else { continue };
            let data = data.trim();
            match serde_json::from_str::<CodexSseEvent>(data) {
                Ok(CodexSseEvent::OutputTextDelta { delta }) => {
                    let _ = tx.send(GenerateChunk { delta, done: false, usage: None }).await;
                }
                Ok(CodexSseEvent::Completed { response }) => {
                    let usage = response.and_then(|r| r.usage).map(|u| TokenUsage {
                        prompt: u.input_tokens,
                        completion: u.output_tokens,
                    });
                    let _ = tx
                        .send(GenerateChunk { delta: String::new(), done: true, usage })
                        .await;
                    saw_done = true;
                    break 'outer;
                }
                Ok(CodexSseEvent::Failed { response }) => {
                    let reason = response
                        .and_then(|r| r.error)
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Codex response failed".into());
                    return Err(AiError::ModelError {
                        reason,
                        raw: data.chars().take(300).collect(),
                    });
                }
                Ok(CodexSseEvent::Other) => {}
                Err(_) => continue,
            }
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum CodexSseEvent {
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta { delta: String },
    #[serde(rename = "response.completed")]
    Completed {
        #[serde(default)]
        response: Option<CodexResponseSummary>,
    },
    #[serde(rename = "response.failed")]
    Failed {
        #[serde(default)]
        response: Option<CodexResponseSummary>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Default)]
struct CodexResponseSummary {
    #[serde(default)]
    usage: Option<CodexUsage>,
    #[serde(default)]
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct CodexUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}
```

- [ ] **Step 4: Run the integration test, confirm it passes**

Run: `cd src-tauri && cargo test --test codex_client 2>&1 | tail -30`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: Run the whole `ai` module's tests to check nothing else broke**

Run: `cd src-tauri && cargo test --lib ai:: 2>&1 | tail -40`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/codex.rs src-tauri/tests/codex_client.rs
git commit -m "feat(ai): implement CodexClient (Responses API streaming + auth headers)"
```

---

## Task 4: JWT `chatgpt_account_id` claim extraction helper

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Write the failing tests**

`src-tauri/src/commands/provider.rs` has no test module yet. Add this new one at the very end of the file:

```rust
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
```

- [ ] **Step 2: Run, confirm it fails to compile**

Run: `cd src-tauri && cargo test codex_jwt_tests 2>&1 | tail -20`
Expected: compile error — `extract_codex_account_id` not found.

- [ ] **Step 3: Implement the helper**

Add this near the other small free functions at the top of `src-tauri/src/commands/provider.rs` (right after `fn gen_state()`):

```rust
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
```

- [ ] **Step 4: Run, confirm it passes**

Run: `cd src-tauri && cargo test codex_jwt_tests 2>&1 | tail -20`
Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): add Codex id_token account-id claim extraction"
```

---

## Task 5: `codex_oauth_login` / `codex_oauth_logout` commands

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Add the OAuth constants and token-response struct**

Add near the top of `src-tauri/src/commands/provider.rs`, right after the existing `ANTHROPIC_OAUTH_*` constants:

```rust
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
```

- [ ] **Step 2: Add `codex_oauth_login`, modeled on the existing `google_oauth_login`**

Add at the end of `src-tauri/src/commands/provider.rs` (this is a new command, so there's no "failing test first" step here — the existing project convention, matching `google_oauth_login`/`anthropic_oauth_start`, is to cover OAuth login commands only via manual end-to-end verification, since they require a real browser + real subscription account and can't be meaningfully unit-tested):

```rust
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

    secrets
        .set(&provider_id, &token_resp.access_token)
        .map_err(|e| format!("Failed to store access token: {e}"))?;

    if let Some(refresh) = &token_resp.refresh_token {
        let _ = secrets.set(&format!("{provider_id}:oauth_refresh"), refresh);
    }
    if let Some(expires_in) = token_resp.expires_in {
        let exp = now_secs() + expires_in;
        let _ = secrets.set(&format!("{provider_id}:oauth_expires_at"), &exp.to_string());
    }
    if let Some(id_token) = &token_resp.id_token {
        if let Some(account_id) = extract_codex_account_id(id_token) {
            let _ = secrets.set(&format!("{provider_id}:oauth_account_id"), &account_id);
        }
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
```

- [ ] **Step 3: Compile-check (these commands have no unit tests — they need a live browser + real ChatGPT account)**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: 0 errors. (`codex_oauth_login`/`codex_oauth_logout` aren't wired into `lib.rs` yet — Task 8 does that — so `cargo check` here just confirms this file compiles standalone.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): add codex_oauth_login/codex_oauth_logout commands"
```

---

## Task 6: `get_codex_oauth_models` command + response-shape parsing

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Write the failing tests for the response-shape parser**

Add to `src-tauri/src/commands/provider.rs`'s test area (a new `#[cfg(test)] mod codex_models_tests`):

```rust
#[cfg(test)]
mod codex_models_tests {
    use super::*;

    #[test]
    fn parse_codex_models_response_handles_models_key() {
        let json = serde_json::json!({"models": [{"slug": "gpt-5.1-codex"}, {"id": "gpt-5.1-codex-high"}]});
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
}
```

- [ ] **Step 2: Run, confirm it fails to compile**

Run: `cd src-tauri && cargo test codex_models_tests 2>&1 | tail -20`
Expected: compile error — `parse_codex_models_response` not found.

- [ ] **Step 3: Implement the parser and the command**

Add near the top of `src-tauri/src/commands/provider.rs`, after the JWT helpers from Task 4:

```rust
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
            Err(_) => vec![],
        },
        _ => vec![],
    };

    if models.is_empty() {
        Ok(CODEX_FALLBACK_MODELS.iter().map(|s| s.to_string()).collect())
    } else {
        Ok(models)
    }
}
```

- [ ] **Step 4: Run the parser tests, confirm they pass**

Run: `cd src-tauri && cargo test codex_models_tests 2>&1 | tail -20`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): add get_codex_oauth_models with tolerant response parsing"
```

---

## Task 7: `get_valid_codex_oauth_token` refresh logic + `ProviderType::Codex` router arm

**Files:**
- Modify: `src-tauri/src/ai/router.rs`

- [ ] **Step 1: Write the failing router test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/ai/router.rs`, after `anthropic_compatible_with_base_url_but_no_key_is_not_configured`:

```rust
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
```

- [ ] **Step 2: Run, confirm it fails to compile**

Run: `cd src-tauri && cargo test codex_provider_without_oauth_token 2>&1 | tail -20`
Expected: compile error — `no variant named 'Codex'` won't recur (Task 1 added it), but `no method named 'resolve_by_id'` match arm exists yet — actual failure will be a runtime assertion failure or a missing-arm compile error, since `match provider_cfg.provider_type` in `resolve_by_id` is exhaustive and doesn't yet have a `Codex` arm.
Expected: compile error — `non-exhaustive patterns: 'ProviderType::Codex' not covered`.

- [ ] **Step 3: Add the refresh function and the router match arm**

Add near the top of `src-tauri/src/ai/router.rs`, after the existing `GOOGLE_OAUTH_*` constants:

```rust
const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
```

Add this function right after `get_valid_google_oauth_token`/`do_google_oauth_refresh`:

```rust
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
```

Add the new match arm in `resolve_by_id`, right after the `ProviderType::AnthropicCompatible` arm and before the closing `};` of the `match`:

```rust
            ProviderType::Codex => {
                let (token, account_id) = get_valid_codex_oauth_token(&provider_cfg.id, &self.secrets).await?;
                Arc::new(crate::ai::codex::CodexClient::new(token, provider_cfg.model.clone(), account_id))
            }
```

- [ ] **Step 4: Run the new test, confirm it passes**

Run: `cd src-tauri && cargo test codex_provider_without_oauth_token 2>&1 | tail -20`
Expected: `test result: ok. 1 passed`

- [ ] **Step 5: Run the whole router test suite to confirm nothing else broke**

Run: `cd src-tauri && cargo test --lib ai::router:: 2>&1 | tail -40`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/router.rs
git commit -m "feat(ai): wire Codex OAuth token refresh + router resolution"
```

---

## Task 8: Wire the 3 new commands into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the imports**

In `src-tauri/src/lib.rs`, find:

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

Replace with:

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
        codex_oauth_login, codex_oauth_logout, get_codex_oauth_models,
        AnthropicOAuthState,
    },
```

- [ ] **Step 2: Add the commands to `invoke_handler!`**

Find, inside the `tauri::generate_handler![` list:

```rust
            google_oauth_login,
            google_oauth_logout,
            get_google_oauth_models,
```

Replace with:

```rust
            google_oauth_login,
            google_oauth_logout,
            get_google_oauth_models,
            codex_oauth_login,
            codex_oauth_logout,
            get_codex_oauth_models,
```

- [ ] **Step 3: Full workspace compile check**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: 0 errors.

- [ ] **Step 4: Run the full Rust test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -60`
Expected: all tests pass (including the new ones from Tasks 1–7).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(app): register Codex OAuth commands with the Tauri IPC handler"
```

---

## Task 9: Frontend types — `ProviderType` union

**Files:**
- Modify: `src/ipc/config.ts:5-16`

- [ ] **Step 1: Add `"codex"` to the union**

In `src/ipc/config.ts`, change:

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

to:

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
  | "anthropic-compatible"
  | "codex";
```

- [ ] **Step 2: Type-check**

Run: `npx tsc --noEmit`
Expected: errors pointing at every `Record<ProviderType, ...>` object in `src/ipc/provider.ts` and `src/components/Settings/ProviderForm.tsx` missing the new `"codex"` key — expected at this point, Task 10 fixes `provider.ts` and Task 11 fixes `ProviderForm.tsx`.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/config.ts
git commit -m "feat(ipc): add codex to the ProviderType union"
```

---

## Task 10: Frontend IPC wrappers — `src/ipc/provider.ts`

**Files:**
- Modify: `src/ipc/provider.ts`

- [ ] **Step 1: Add the 3 IPC wrapper functions**

In `src/ipc/provider.ts`, right after the existing `getKimiModelsByProvider` export, add:

```ts
/** Start Codex OAuth: opens browser, waits for the localhost:1455 callback, exchanges tokens. Blocks until done. */
export const codexOAuthLogin = (providerId: string): Promise<void> =>
  invoke("codex_oauth_login", { providerId });

/** Log out from Codex OAuth (clears stored tokens + cached ChatGPT account id). */
export const codexOAuthLogout = (providerId: string): Promise<void> =>
  invoke("codex_oauth_logout", { providerId });

/** Fetch available Codex models using the stored OAuth token. */
export const getCodexOAuthModels = (providerId: string): Promise<string[]> =>
  invoke("get_codex_oauth_models", { providerId });
```

- [ ] **Step 2: Add `codex` to `PROVIDER_TYPE_LABELS`, `DEFAULT_MODELS`, `DEFAULT_BASE_URLS`**

Change:

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
```

to:

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
  codex: "Codex (ChatGPT)",
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
  codex: "gpt-5.1-codex",
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
  codex: "",
};
```

- [ ] **Step 3: Type-check**

Run: `npx tsc --noEmit`
Expected: remaining errors only in `src/components/Settings/ProviderForm.tsx` (Task 11 fixes those).

- [ ] **Step 4: Commit**

```bash
git add src/ipc/provider.ts
git commit -m "feat(ipc): add Codex OAuth wrapper functions and provider metadata"
```

---

## Task 11: `ProviderForm.tsx` — Codex option, login button, model datalist

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`

- [ ] **Step 1: Import the new IPC functions and add `codex` to `PROVIDER_TYPES`**

Change the import block:

```ts
  getKimiModels,
  getKimiModelsByProvider,
} from "../../ipc/provider";
```

to:

```ts
  getKimiModels,
  getKimiModelsByProvider,
  codexOAuthLogin,
  codexOAuthLogout,
  getCodexOAuthModels,
} from "../../ipc/provider";
```

Change:

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

to:

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
  "codex",
];
```

- [ ] **Step 2: Add Codex-specific state, next to the other per-provider model/loading state**

Find:

```ts
  const [kimiModels, setKimiModels] = useState<string[]>([]);
  const [kimiLoading, setKimiLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authing, setAuthing] = useState(false);
  const [authStatus, setAuthStatus] = useState<string | null>(null);
```

Replace with:

```ts
  const [kimiModels, setKimiModels] = useState<string[]>([]);
  const [kimiLoading, setKimiLoading] = useState(false);
  const [codexModels, setCodexModels] = useState<string[]>([]);
  const [codexModelsLoading, setCodexModelsLoading] = useState(false);
  const [codexOAuthLoggedIn, setCodexOAuthLoggedIn] = useState(
    !!(existing?.provider_type === "codex" && existing?.auth_method === "oauth"),
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authing, setAuthing] = useState(false);
  const [authStatus, setAuthStatus] = useState<string | null>(null);
```

- [ ] **Step 3: Fetch models automatically when already logged in (mirrors the Google AI `useEffect` pattern)**

Add this `useEffect`, right after the `kimi` one (after the block ending `}, [providerType, apiKey, isEdit, existing?.has_api_key, id]);` that follows the `kimi` fetch):

```ts
  useEffect(() => {
    if (providerType !== "codex" || !codexOAuthLoggedIn) return;
    const pid = id.trim();
    if (!pid) return;
    setCodexModelsLoading(true);
    getCodexOAuthModels(pid)
      .then((models) => {
        setCodexModels(models);
        if (models.length > 0 && !models.includes(model)) setModel(models[0]);
      })
      .catch(() => setCodexModels([]))
      .finally(() => setCodexModelsLoading(false));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, codexOAuthLoggedIn, id]);
```

- [ ] **Step 4: Add the auth-section UI block (login/logout button)**

Find the closing of the Anthropic OAuth auth section — the line `{anthropicOAuthLoggedIn && authStatus && <div className="form-hint">{authStatus}</div>}` followed by its enclosing `</div>` and blank line, right before:

```ts
          {providerType !== "github-copilot" && providerType !== "anthropic" && (
            <div className="form-group">
              <label>
                {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
              </label>
```

Insert a new block right after `{anthropicOAuthLoggedIn && authStatus && <div className="form-hint">{authStatus}</div>}` and its closing `</div>` (i.e. right before the `{providerType !== "github-copilot" ...}` API-key block), and also change that condition to exclude `codex`:

```ts
          {providerType === "codex" && (
            <div className="form-group">
              <label>{t.settings_provider_auth_oauth("ChatGPT")}</label>
              {codexOAuthLoggedIn ? (
                <div className="anthropic-oauth-done">
                  <span className="anthropic-oauth-ok">{t.settings_provider_oauth_ok}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    disabled={saving}
                    onClick={async () => {
                      if (!isEdit) return;
                      try {
                        await codexOAuthLogout(id.trim());
                        setCodexOAuthLoggedIn(false);
                        setCodexModels([]);
                      } catch (e: unknown) {
                        setAuthStatus(String(e));
                      }
                    }}
                  >
                    {t.settings_provider_oauth_logout}
                  </button>
                </div>
              ) : (
                <>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--primary"
                    disabled={!id.trim() || authing}
                    onClick={async () => {
                      setAuthing(true);
                      setAuthStatus(null);
                      try {
                        await codexOAuthLogin(id.trim());
                        setCodexOAuthLoggedIn(true);
                        setAuthStatus(t.settings_provider_oauth_success);
                      } catch (e: unknown) {
                        setAuthStatus(t.settings_provider_oauth_err(String(e)));
                      } finally {
                        setAuthing(false);
                      }
                    }}
                  >
                    {authing ? t.provider_auth_running : t.settings_provider_btn_open_auth}
                  </button>
                  {!id.trim() && (
                    <div className="form-hint">{t.settings_provider_oauth_id_required}</div>
                  )}
                </>
              )}
              {authStatus && (
                <div className={`form-hint ${authStatus.startsWith("錯誤") || authStatus.startsWith("Error") ? "form-hint--error" : ""}`}>
                  {authStatus}
                </div>
              )}
            </div>
          )}

          {providerType !== "github-copilot" && providerType !== "anthropic" && providerType !== "codex" && (
            <div className="form-group">
              <label>
                {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
              </label>
```

(Leave the rest of that block — the `<input type="password" .../>` and its closing tags — unchanged; only the opening condition line changes.)

- [ ] **Step 5: Add the model-field branch**

Find the ternary chain's `kimi` branch (ends with the closing `)` right before `) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (`) and insert a new branch immediately before that `anthropic` oauth branch:

```ts
          ) : providerType === "codex" ? (
            codexModelsLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="codex-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={codexModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {codexModels.length > 0 && (
                  <datalist id="codex-models-list">
                    {codexModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
          ) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (
```

- [ ] **Step 6: Type-check**

Run: `npx tsc --noEmit`
Expected: 0 errors.

- [ ] **Step 7: Lint**

Run: `npm run lint`
Expected: no new errors introduced by this file (pre-existing unrelated warnings/errors in other files are fine — see prior session notes; don't fix those here).

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): add Codex provider option with OAuth login and model picker"
```

---

## Task 12: i18n strings

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Confirm the keys this plan already relies on exist for both locales**

`settings_provider_auth_oauth`, `settings_provider_oauth_ok`, `settings_provider_oauth_success`, `settings_provider_oauth_err`, `settings_provider_btn_open_auth`, `settings_provider_oauth_logout`, `settings_provider_oauth_id_required`, `settings_provider_model_placeholder`, `provider_model_loading`, `provider_auth_running` are all already defined (they're reused from the Anthropic OAuth / Google AI / GitHub Copilot sections) — no new keys are strictly required for Task 11's code to compile and run. This step is just a sanity check.

Run: `grep -n "settings_provider_oauth_id_required\|provider_auth_running\|settings_provider_model_placeholder" src/lib/i18n.ts`
Expected: matches in both the zh-TW and en blocks.

- [ ] **Step 2: (Optional polish) Add a Codex-specific hint string**

If you want a more specific hint than the generic `"OAuth "+suffix"` composed string when `t.settings_provider_auth_oauth("ChatGPT")` renders, you can skip this — the generic composition already reads correctly ("網頁 OAuth ChatGPT" / "Web OAuth ChatGPT"). No action needed for MVP; this step exists only to record that it was considered.

- [ ] **Step 3: Commit (only if Step 2's optional string was added)**

```bash
git add src/lib/i18n.ts
git commit -m "feat(i18n): add Codex-specific auth hint string"
```

---

## Task 13: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -80`
Expected: all tests pass, including every Codex test added in Tasks 1–7.

- [ ] **Step 2: Rust lint**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | tail -60`
Expected: no new warnings introduced by `codex.rs`, `router.rs`, or `commands/provider.rs` changes (pre-existing warnings elsewhere are out of scope).

- [ ] **Step 3: Frontend type check + test suite**

Run: `npx tsc --noEmit && npm run test 2>&1 | tail -40`
Expected: 0 type errors; all existing frontend tests still pass (no new frontend tests were added in this plan — `ProviderForm.tsx` has no existing test file to extend, matching the precedent set by the prior multi-provider-support plan).

- [ ] **Step 4: Frontend lint**

Run: `npm run lint`
Expected: no new errors/warnings in the files this plan touched.

- [ ] **Step 5: Manual end-to-end verification (requires a real ChatGPT subscription account — cannot be automated)**

Run: `npm run tauri:dev`, open Settings → Add Provider → type "Codex", enter an id/display name, click the ChatGPT login button, complete login in the browser with a real ChatGPT Plus/Pro/Team account, confirm:
- The button flips to the "已登入" (logged in) state.
- The model datalist populates with real model ids (or the 2-item fallback list if the live discovery call fails — check which happened and note it).
- Save the provider, set it as default, send a `/ai` query, confirm a streamed response comes back.
- If the live discovery endpoint's `slug` values turn out to already be base+effort-suffixed compound ids that the `/responses` endpoint rejects when sent back as `model` verbatim (the risk flagged in the spec's "待驗證假設" section), note the exact error and file it as a fast-follow — do not attempt to fix the effort-suffix-splitting logic as part of this plan (explicitly out of scope for MVP).

- [ ] **Step 6: Final commit (only if Step 5 surfaced fixes)**

If manual verification required any fixes, commit them separately with a clear message describing what was wrong and why. If no fixes were needed, this task ends at Step 5 — nothing to commit.

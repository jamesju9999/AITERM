# Gemini (Antigravity OAuth) Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete AITerm's existing (currently dormant) `ProviderType::GoogleAi` OAuth scaffold so it authenticates against Google's internal Antigravity/Cloud Code Assist API and lets a user chat with Gemini using their Google account instead of an API key.

**Architecture:** A new `AntigravityClient` (`src-tauri/src/ai/antigravity.rs`) implements the `AiProvider` trait directly against Gemini's native `contents`/`parts` request format (not Chat Completions, not Responses API — a third distinct shape). `commands/provider.rs`'s existing `google_oauth_login` gets its dead client_id/secret/scope constants replaced with real Antigravity values, plus a new post-token-exchange "onboarding" step (`loadCodeAssist`/`onboardUser`) that resolves a `project_id` required on every chat request. `ai/router.rs`'s existing `ProviderType::GoogleAi` OAuth branch gets rewired from `OpenAiCompatibleClient` (wrong — that hits the public API, which rejects Antigravity-scoped tokens) to the new `AntigravityClient`. Frontend adds an API-Key/OAuth tab pair to the `google-ai` provider type in Settings, mirroring the existing Anthropic tabs.

**Tech Stack:** Rust (Tauri 2 backend, `reqwest`, `serde`, `tokio`, `uuid`), React 19 + TypeScript (frontend), `wiremock` for Rust integration tests.

**Spec:** `docs/superpowers/specs/2026-07-27-gemini-antigravity-provider-design.md`

**Scope adjustment vs. the spec** (flagged to the user, not silent): the spec called for a dynamic Antigravity-IDE-version fetch with a hardcoded fallback. The real response shape of that endpoint is unverified (research found only the URL, not its JSON structure) — writing a parser against a guessed shape would ship dead/wrong code. This plan implements **only the hardcoded fallback version** for MVP; dynamic fetching is a documented fast-follow once someone can inspect a real response (see the plan's final "待驗證假設" carryover in Task 13).

---

## Task 1: `AntigravityClient` request-body builder

**Files:**
- Create: `src-tauri/src/ai/antigravity.rs`
- Modify: `src-tauri/src/ai/mod.rs`

- [ ] **Step 1: Register the module**

In `src-tauri/src/ai/mod.rs`, change:

```rust
pub mod anthropic;
pub mod codex;
pub mod compatible;
```

to:

```rust
pub mod anthropic;
pub mod antigravity;
pub mod codex;
pub mod compatible;
```

(alphabetically, right before `codex`)

- [ ] **Step 2: Create `antigravity.rs` with the struct + a failing test for the body builder**

Create `src-tauri/src/ai/antigravity.rs`:

```rust
//! Gemini provider via Google's internal Antigravity / Cloud Code Assist API
//! (`cloudcode-pa.googleapis.com`) — NOT the public Generative Language API,
//! and NOT Vertex AI's documented REST API. This is what a user's Google
//! account OAuth (scoped for Antigravity/Gemini Code Assist) actually talks
//! to; it requires a "project" id obtained via a separate onboarding step
//! (see `commands/provider.rs`) on every request.
//!
//! Key differences from every other client in this module:
//! - Endpoint is `cloudcode-pa.googleapis.com/v1internal:streamGenerateContent`.
//! - Request/response shape is Gemini's own native format (`contents: [{role,
//!   parts:[{text}]}]`, not Chat Completions `messages` and not the Responses
//!   API's `input` items.
//! - Every request must carry the account's onboarded Cloud Code project id
//!   in a top-level `project` field.
//! - The `contents` array only supports alternating "user"/"model" turns —
//!   there is no "system"/"developer" role slot. A caller that injects a
//!   `{role:"system",...}` message directly into history (e.g. AiPanel's
//!   Agent Mode loop, `src/components/AiPanel/index.tsx`'s `runAgentLoop`)
//!   would hit the same class of bug already root-caused and fixed for
//!   Codex (see `codex.rs`'s `build_request_body` — that endpoint accepts a
//!   remapped "developer" role; this one has no equivalent slot at all), so
//!   any system-role message here is folded into `systemInstruction` instead
//!   of being emitted as a `contents` turn.
//! - The client must present as the real Antigravity IDE client (User-Agent
//!   fixed to `darwin/arm64` regardless of host OS) — this is the closest
//!   analogue to Anthropic's "Claude Code sentinel" or Codex's
//!   `originator: codex_cli_rs` header for this provider.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, map_http_error, separator_len},
    AiError, AiProvider, GenerateChunk, GenerateRequest, TokenUsage,
};

/// Hardcoded fallback client version. Dynamic fetching from Antigravity's
/// auto-updater feed is a documented fast-follow (see plan Task 13) — the
/// real response shape of that endpoint hasn't been verified yet.
pub(crate) const ANTIGRAVITY_IDE_VERSION: &str = "2.1.1";

pub struct AntigravityClient {
    access_token: String,
    project_id: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

/// Build the Antigravity `streamGenerateContent` request envelope.
pub(crate) fn build_request_body(model: &str, project_id: &str, req: &GenerateRequest) -> serde_json::Value {
    serde_json::json!({});
}
```

- [ ] **Step 3: Write the failing unit tests**

Append to the bottom of `src-tauri/src/ai/antigravity.rs`:

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

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage { role: role.into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
    }

    #[test]
    fn project_id_is_set_at_top_level() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        assert_eq!(body["project"], "proj-123");
    }

    #[test]
    fn envelope_has_fixed_metadata_fields() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["requestType"], "agent");
        assert_eq!(body["model"], "gemini-2.5-pro");
        assert!(body["requestId"].as_str().unwrap().starts_with("agent/"));
    }

    #[test]
    fn user_and_assistant_messages_map_to_user_and_model_roles() {
        let r = req("sys", vec![msg("user", "hi"), msg("assistant", "hello")]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let contents = body["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "hi");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "hello");
    }

    /// Regression-by-construction: mirrors the Codex "system messages are not
    /// allowed" bug (AiPanel's Agent Mode injects {role:"system",...} directly
    /// into history). Gemini's `contents` has no role slot for it at all, so
    /// it must be folded into systemInstruction, never emitted as a turn.
    #[test]
    fn system_role_message_is_folded_into_system_instruction_not_contents() {
        let r = req("base system prompt", vec![
            msg("system", "agent orchestration prompt"),
            msg("user", "go"),
        ]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let contents = body["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1, "the system-role message must not appear as a contents turn");
        assert_eq!(contents[0]["role"], "user");
        for c in contents {
            assert_ne!(c["role"], "system");
        }
        let system_instruction = body["request"]["systemInstruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(system_instruction.contains("base system prompt"));
        assert!(system_instruction.contains("agent orchestration prompt"));
    }

    #[test]
    fn generation_config_forces_expected_values() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let gc = &body["request"]["generationConfig"];
        assert_eq!(gc["topK"], 40);
        assert_eq!(gc["topP"], 1.0);
        assert_eq!(gc["maxOutputTokens"], 16384);
    }
}
```

- [ ] **Step 4: Run the tests, confirm they fail**

Run: `cd src-tauri && cargo test --lib ai::antigravity 2>&1 | tail -30`
Expected: all 5 tests FAIL (the stub returns `{}`).

- [ ] **Step 5: Implement `build_request_body` for real**

Replace the stub body of `build_request_body` in `src-tauri/src/ai/antigravity.rs`:

```rust
pub(crate) fn build_request_body(model: &str, project_id: &str, req: &GenerateRequest) -> serde_json::Value {
    // Any {role:"system",...} message injected directly into history (see
    // this file's module doc) is folded into systemInstruction rather than
    // emitted as a contents turn — Gemini's contents array only supports
    // alternating "user"/"model" roles.
    let mut system_parts: Vec<String> = vec![req.system_prompt.clone()];

    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter_map(|m| {
            let text = match &m.content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if m.role == "system" {
                system_parts.push(text);
                return None;
            }
            let role = if m.role == "assistant" { "model" } else { "user" };
            Some(serde_json::json!({ "role": role, "parts": [{ "text": text }] }))
        })
        .collect();

    let system_instruction_text = system_parts
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let request_id = format!("agent/{now_ms}/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);

    serde_json::json!({
        "project": project_id,
        "requestId": request_id,
        "userAgent": "antigravity",
        "requestType": "agent",
        "model": model,
        "request": {
            "contents": contents,
            "systemInstruction": { "parts": [{ "text": system_instruction_text }] },
            "generationConfig": { "topK": 40, "topP": 1.0, "maxOutputTokens": 16384 },
        },
    })
}
```

- [ ] **Step 6: Run the tests, confirm they pass**

Run: `cd src-tauri && cargo test --lib ai::antigravity 2>&1 | tail -30`
Expected: `test result: ok. 5 passed`

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/antigravity.rs src-tauri/src/ai/mod.rs
git commit -m "feat(ai): add Antigravity/Gemini request-body builder"
```

---

## Task 2: `AntigravityClient` constructor, headers, SSE parsing, and `AiProvider` impl

**Files:**
- Modify: `src-tauri/src/ai/antigravity.rs`
- Create: `src-tauri/tests/antigravity_client.rs`

- [ ] **Step 1: Write the failing integration test first**

Create `src-tauri/tests/antigravity_client.rs`:

```rust
//! Contract test for `AntigravityClient` against a wiremock fake of the
//! Antigravity/Cloud Code Assist streamGenerateContent endpoint. Covers the
//! happy path (SSE streaming + required headers) and 401 → AuthFailed.

use aiterm_lib::ai::{
    antigravity::AntigravityClient, AiError, AiProvider, ChatMessage, EnvSnapshot, GenerateChunk,
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

    let sse_body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}\n\n\
                     data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":2}}\n\n";

    Mock::given(method("POST"))
        .and(path("/v1internal:streamGenerateContent"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .expect(1)
        .mount(&server)
        .await;

    // Production always talks to the real cloudcode-pa.googleapis.com host —
    // `with_base_url` is a test-only hook to point at the wiremock server.
    let client = AntigravityClient::with_base_url(
        "test-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
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

    let client = AntigravityClient::with_base_url(
        "bad-token".into(),
        "proj-123".into(),
        "gemini-2.5-pro".into(),
        server.uri(),
    );
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(16);
    let err = client.generate(req(), tx).await.unwrap_err();
    assert!(matches!(err, AiError::AuthFailed), "expected AuthFailed, got {err:?}");
}
```

- [ ] **Step 2: Run it, confirm it fails to compile**

Run: `cd src-tauri && cargo test --test antigravity_client 2>&1 | tail -30`
Expected: compile error — `AntigravityClient` has no `with_base_url` / no `generate`.

- [ ] **Step 3: Implement the constructor, headers, SSE consumer, and `AiProvider` impl**

Replace the `AntigravityClient` struct declaration and everything below it (but above the `#[cfg(test)]` module) in `src-tauri/src/ai/antigravity.rs` with:

```rust
pub struct AntigravityClient {
    access_token: String,
    project_id: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl AntigravityClient {
    pub fn new(access_token: String, project_id: String, model: String) -> Self {
        Self::with_base_url(access_token, project_id, model, "https://cloudcode-pa.googleapis.com".into())
    }

    /// Test-only hook: lets integration tests point at a wiremock server
    /// instead of the real cloudcode-pa.googleapis.com backend. There is no
    /// user-facing base_url setting for this provider — the endpoint is
    /// fixed in production.
    pub fn with_base_url(access_token: String, project_id: String, model: String, base_url: String) -> Self {
        Self { access_token, project_id, model, base_url, client: reqwest::Client::new() }
    }

    fn generate_content_url(&self) -> String {
        format!("{}/v1internal:streamGenerateContent?alt=sse", self.base_url.trim_end_matches('/'))
    }

    /// Fixed User-Agent presenting as the real Antigravity IDE client.
    /// Deliberately reports darwin/arm64 regardless of host OS — the
    /// upstream backend is known to treat the Mac build identity more
    /// permissively (see this file's module doc for the OmniRoute-sourced
    /// rationale this was ported from).
    fn user_agent(&self) -> String {
        format!("antigravity/ide/{ANTIGRAVITY_IDE_VERSION} darwin/arm64")
    }

    fn apply_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .header("User-Agent", self.user_agent())
            .bearer_auth(&self.access_token)
    }
}

#[async_trait]
impl AiProvider for AntigravityClient {
    fn id(&self) -> &str {
        "google-ai"
    }
    fn display_name(&self) -> &str {
        "Gemini (Google Account)"
    }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &self.project_id, &req);
        let resp = self
            .apply_headers(self.client.post(self.generate_content_url()))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }
        consume_gemini_sse(resp, tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // A minimal real generate call is the only reliable way to validate an
        // Antigravity token + project id pair — there is no cheap read-only
        // endpoint reused here (model discovery lives in commands/provider.rs
        // and needs its own token, not this client's).
        let (tx, mut rx) = mpsc::channel::<GenerateChunk>(4);
        let probe = GenerateRequest {
            system_prompt: String::new(),
            messages: vec![crate::ai::ChatMessage {
                role: "user".into(),
                content: serde_json::json!("ping"),
                tool_call_id: None,
                tool_calls: None,
            }],
            context: crate::ai::EnvSnapshot::default(),
            mode: crate::ai::QueryMode::Chat,
            max_tokens: Some(1),
        };
        self.generate(probe, tx).await?;
        while rx.recv().await.is_some() {}
        Ok(())
    }
}

async fn consume_gemini_sse(
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
            let Ok(chunk) = serde_json::from_str::<GeminiStreamChunk>(data) else { continue };

            let candidate = chunk.candidates.into_iter().next();
            let text = candidate
                .as_ref()
                .and_then(|c| c.content.as_ref())
                .map(|c| c.parts.iter().filter_map(|p| p.text.clone()).collect::<String>())
                .unwrap_or_default();
            let finished = candidate.as_ref().and_then(|c| c.finish_reason.as_ref()).is_some();

            if !text.is_empty() {
                let _ = tx.send(GenerateChunk { delta: text, done: false, usage: None }).await;
            }
            if finished {
                let usage = chunk.usage_metadata.map(|u| TokenUsage {
                    prompt: u.prompt_token_count,
                    completion: u.candidates_token_count,
                });
                let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage }).await;
                saw_done = true;
                break 'outer;
            }
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

#[derive(Deserialize)]
struct GeminiStreamChunk {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Deserialize, Default)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct GeminiUsageMetadata {
    #[serde(default, rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_token_count: u32,
}
```

IMPORTANT: leave `build_request_body` (from Task 1) and the existing `#[cfg(test)] mod tests { ... }` block completely untouched — only add the struct/impls/SSE code above them.

Also add `use uuid::Uuid;`-free reference: `build_request_body` already references `uuid::Uuid` with a full path (`uuid::Uuid::new_v4()`), so no new `use` statement is needed — `uuid` is already a workspace dependency (used elsewhere in `commands/provider.rs`).

- [ ] **Step 4: Run the integration test, confirm it passes**

Run: `cd src-tauri && cargo test --test antigravity_client 2>&1 | tail -30`
Expected: `test result: ok. 2 passed`

- [ ] **Step 5: Run the whole `ai` module's tests to check nothing else broke**

Run: `cd src-tauri && cargo test --lib ai:: 2>&1 | tail -40`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/antigravity.rs src-tauri/tests/antigravity_client.rs
git commit -m "feat(ai): implement AntigravityClient (Gemini native streaming + auth headers)"
```

---

## Task 3: Antigravity onboarding (`loadCodeAssist`/`onboardUser`) helper

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Write the failing tests for response parsing**

Add a new `#[cfg(test)] mod antigravity_onboarding_tests` at the end of `src-tauri/src/commands/provider.rs`:

```rust
#[cfg(test)]
mod antigravity_onboarding_tests {
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
}
```

- [ ] **Step 2: Run, confirm it fails to compile**

Run: `cd src-tauri && cargo test antigravity_onboarding_tests 2>&1 | tail -20`
Expected: compile error — `extract_cloudaicompanion_project_id` not found.

- [ ] **Step 3: Implement the parser + the onboarding orchestration function**

Add near the top of `src-tauri/src/commands/provider.rs`, after the JWT helpers (`decode_jwt_payload`/`extract_codex_account_id`):

```rust
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
///    `onboardUser` and poll up to 10 times (5s apart, ~50s total) until it
///    reports `done: true`, then retry `loadCodeAssist` once to pick up the
///    freshly-provisioned project.
async fn perform_antigravity_onboarding(access_token: &str) -> Result<String, String> {
    let client = reqwest::Client::new();

    let load_body = serde_json::json!({ "metadata": { "ideType": "ANTIGRAVITY" } });
    let resp = antigravity_headers(
        client.post(format!("{ANTIGRAVITY_BOOTSTRAP_BASE_URL}/v1internal:loadCodeAssist")),
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
        .unwrap_or("legacy-tier")
        .to_string();

    let onboard_body = serde_json::json!({
        "tierId": tier_id,
        "metadata": { "ideType": "ANTIGRAVITY" },
    });

    for _ in 0..10 {
        let resp = antigravity_headers(
            client.post(format!("{ANTIGRAVITY_BOOTSTRAP_BASE_URL}/v1internal:onboardUser")),
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
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    let resp = antigravity_headers(
        client.post(format!("{ANTIGRAVITY_BOOTSTRAP_BASE_URL}/v1internal:loadCodeAssist")),
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
```

- [ ] **Step 4: Run, confirm the parser tests pass**

Run: `cd src-tauri && cargo test antigravity_onboarding_tests 2>&1 | tail -20`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Full compile check**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: 0 errors. (`perform_antigravity_onboarding` isn't called yet — that's Task 4 — so expect an "unused function" warning, which is fine and self-resolving.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): add Antigravity onboarding (loadCodeAssist/onboardUser)"
```

---

## Task 4: Point `google_oauth_login`/`logout` at real Antigravity credentials + wire in onboarding

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Replace the dead OAuth constants**

Find:

```rust
const GOOGLE_OAUTH_CLIENT_ID: &str = "";
const GOOGLE_OAUTH_CLIENT_SECRET: &str = "";
const GOOGLE_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile openid";
```

Replace with:

```rust
// Antigravity's OAuth client. This is Google's public installed-app client
// id for Antigravity/Gemini Code Assist — confirmed byte-for-byte against
// two independent client entry points (IDE login and the standalone `agy`
// CLI login) that both use it. The client_secret for Google's installed-app
// OAuth clients is not a confidential secret (Google's own docs describe
// this client type as public); TODO before shipping: obtain the real secret
// value via your own Antigravity/agy client traffic or documentation rather
// than trusting a third-party's obfuscated constant — this repo intentionally
// does not decode one from anywhere. Left empty here as a placeholder for
// that follow-up; Google's token endpoint may or may not reject an empty
// client_secret for this client type (verify at implementation time — see
// design spec's "待驗證假設" #1).
const GOOGLE_OAUTH_CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_OAUTH_CLIENT_SECRET: &str = "";
const GOOGLE_OAUTH_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_OAUTH_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Deliberately NO "openid" scope — including it (even without PKCE) has been
// found to route Google into a hanging `firstparty/nativeapp` consent screen
// for this specific client. `cclog`/`experimentsandconfigs` are required by
// the Antigravity backend itself, not optional extras.
const GOOGLE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";
```

- [ ] **Step 2: Wire onboarding into `google_oauth_login`**

Find, inside `google_oauth_login`:

```rust
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
```

Replace with:

```rust
    // Onboard BEFORE persisting anything — if this fails, the login as a
    // whole fails and nothing is half-saved (no token stored without a
    // matching project id).
    let project_id = perform_antigravity_onboarding(&token_resp.access_token)
        .await
        .map_err(|e| format!("Antigravity onboarding failed: {e}"))?;

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
    let _ = secrets.set(&format!("{provider_id}:project_id"), &project_id);

    config
        .update(|cfg| {
            if let Some(p) = cfg.providers.iter_mut().find(|p| p.id == provider_id) {
                p.auth_method = Some("oauth".into());
            }
        })
        .map_err(|e| format!("Failed to update provider config: {e}"))?;
```

- [ ] **Step 3: Clear `project_id` on logout**

Find, inside `google_oauth_logout`:

```rust
    let _ = secrets.delete(&provider_id);
    let _ = secrets.delete(&format!("{provider_id}:oauth_refresh"));
    let _ = secrets.delete(&format!("{provider_id}:oauth_expires_at"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_id"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_secret"));
```

Replace with:

```rust
    let _ = secrets.delete(&provider_id);
    let _ = secrets.delete(&format!("{provider_id}:oauth_refresh"));
    let _ = secrets.delete(&format!("{provider_id}:oauth_expires_at"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_id"));
    let _ = secrets.delete(&format!("{provider_id}:google_oauth_client_secret"));
    let _ = secrets.delete(&format!("{provider_id}:project_id"));
```

(The `google_oauth_client_id`/`google_oauth_client_secret` deletions are pre-existing dead entries from before this task — leave them; they're harmless no-ops now that login never writes those specific keys, and removing them isn't this task's job.)

- [ ] **Step 4: Full compile check**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: 0 errors.

- [ ] **Step 5: Run the full test suite to confirm nothing else broke**

Run: `cd src-tauri && cargo test 2>&1 | tail -40`
Expected: all pass (these two functions have no unit tests of their own — matching the existing project convention that OAuth login/logout commands are covered by manual E2E verification only, since they need a real browser + real account).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): point google_oauth_login at real Antigravity credentials, add onboarding"
```

---

## Task 5: Replace `get_google_oauth_models` with Antigravity model discovery

**Files:**
- Modify: `src-tauri/src/commands/provider.rs`

- [ ] **Step 1: Write the failing tests for the tolerant models-response parser**

Add to the `antigravity_onboarding_tests` module (from Task 3) — rename that module to `antigravity_tests` to reflect it now covers more than onboarding:

Find:
```rust
#[cfg(test)]
mod antigravity_onboarding_tests {
    use super::*;
```
Replace with:
```rust
#[cfg(test)]
mod antigravity_tests {
    use super::*;
```

Then add these 4 tests inside that same module, after the existing `extract_project_id_*` tests:

```rust
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
```

- [ ] **Step 2: Run, confirm it fails to compile**

Run: `cd src-tauri && cargo test antigravity_tests 2>&1 | tail -20`
Expected: compile error — `parse_antigravity_models_response` not found.

- [ ] **Step 3: Replace the Vertex-AI-oriented model list with the Antigravity version**

The current `get_google_oauth_models` (and its `vertex_ai_known_models` helper) talks to a Vertex-AI-shaped `{base_url}/models` endpoint with an OpenAI-style `{"data":[{"id":...}]}` response — that's the wrong endpoint and wrong shape for Antigravity tokens. Find the whole existing block:

```rust
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
```

Replace the whole block with:

```rust
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

/// Extracts model ids from Antigravity's model-discovery response, tolerating
/// the shapes it might return: `{"models":[...]}` or a bare array, with each
/// item's id read from `name` or `id`.
fn parse_antigravity_models_response(json: &serde_json::Value) -> Vec<String> {
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
                .map(str::to_string)
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

    let client = reqwest::Client::new();
    let models = match antigravity_headers(
        client.get(format!("{ANTIGRAVITY_BOOTSTRAP_BASE_URL}/v1internal:fetchAvailableModels")),
        &token,
    )
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
        Ok(antigravity_known_models())
    } else {
        Ok(models)
    }
}
```

**Note the signature change**: `base_url_override`/`config` parameters are dropped — Antigravity has no user-configurable base_url (fixed endpoint, matching Codex's design). Task 6 updates the frontend IPC wrapper and its one call site to match.

- [ ] **Step 4: Run the parser tests, confirm they pass**

Run: `cd src-tauri && cargo test antigravity_tests 2>&1 | tail -20`
Expected: `test result: ok. 8 passed` (4 from Task 3 + 4 new).

- [ ] **Step 5: Full compile check**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: compile errors pointing at `lib.rs`'s `get_google_oauth_models` import/registration (still expects the old 2-arg signature) and `src/ipc/provider.ts`'s `getGoogleOAuthModels` wrapper — both expected, fixed in Tasks 6 and 8. Confirm no *other* unrelated errors.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/provider.rs
git commit -m "feat(provider): replace Vertex-AI-shaped model discovery with Antigravity fetchAvailableModels"
```

---

## Task 6: `router.rs` — project id in token resolution, simplified refresh, `AntigravityClient` wiring

**Files:**
- Modify: `src-tauri/src/ai/router.rs`

- [ ] **Step 1: Write the failing router test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/ai/router.rs`, after the existing `codex_provider_without_oauth_token_is_not_configured` test:

```rust
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
```

- [ ] **Step 2: Run, confirm it fails**

Run: `cd src-tauri && cargo test google_ai_oauth_provider_without_token 2>&1 | tail -20`
Expected: FAILS. The current `GoogleAi` arm resolves successfully even with no token stored, because `get_valid_google_oauth_token` today only errors if the DIRECT `secrets.get(provider_id)` read comes back empty — which it does here (no token stored) — so this should actually already return `NotConfigured`... **but** double-check by running it: if it unexpectedly passes already, that's fine (the existing behavior for "no token" was already correct) — this task's real changes are Steps 3-4 below, not this test's pass/fail state per se. Note the actual result you observe in your report.

- [ ] **Step 3: Update `get_valid_google_oauth_token` to also resolve `project_id`, and simplify the refresh function**

Find:

```rust
async fn get_valid_google_oauth_token(provider_id: &str, secrets: &SecretStore) -> Result<String, AiError> {
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
            match do_google_oauth_refresh(provider_id, &refresh_token, secrets).await {
                Ok(access_token) => return Ok(access_token),
                Err(e) => log::warn!("Google OAuth token refresh failed, using existing token: {e}"),
            }
        }
    }

    secrets
        .get(provider_id)
        .map_err(|_| AiError::NotConfigured)?
        .ok_or(AiError::NotConfigured)
}
```

Replace with:

```rust
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
```

(`do_google_oauth_refresh` itself needs no changes — it already only sets `access_token`/`oauth_expires_at` and never touches `oauth_refresh`, which is exactly correct for Google's non-rotating refresh tokens.)

- [ ] **Step 4: Wire the `GoogleAi` router arm to `AntigravityClient` for the OAuth case**

Find:

```rust
            ProviderType::GoogleAi => {
                let is_oauth = provider_cfg.auth_method.as_deref() == Some("oauth");
                let key = if is_oauth {
                    get_valid_google_oauth_token(&provider_cfg.id, &self.secrets).await?
                } else {
                    self.secrets
                        .get(&provider_cfg.id)
                        .map_err(|_| AiError::NotConfigured)?
                        .ok_or(AiError::NotConfigured)?
                };
                Arc::new(OpenAiCompatibleClient::new(
                    provider_cfg
                        .base_url
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta/openai".into()),
                    provider_cfg.model.clone(),
                    Some(key),
                    provider_cfg.supports_json_mode,
                ))
            }
```

Replace with:

```rust
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
```

- [ ] **Step 5: Run the new test, confirm it passes**

Run: `cd src-tauri && cargo test google_ai_oauth_provider_without_token 2>&1 | tail -20`
Expected: `test result: ok. 1 passed`

- [ ] **Step 6: Run the whole router test suite to confirm nothing else broke**

Run: `cd src-tauri && cargo test --lib ai::router:: 2>&1 | tail -40`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/ai/router.rs
git commit -m "feat(ai): resolve Antigravity project id + wire AntigravityClient into GoogleAi OAuth path"
```

---

## Task 7: Frontend IPC wrapper signature update

**Files:**
- Modify: `src/ipc/provider.ts`

- [ ] **Step 1: Update `getGoogleOAuthModels` to match the new no-base_url backend signature**

Find:

```ts
/** Fetch available Gemini models using the stored Google OAuth token.
 *  Pass baseUrlOverride to use a URL before it's been saved to config. */
export const getGoogleOAuthModels = (
  providerId: string,
  baseUrlOverride?: string,
): Promise<string[]> =>
  invoke("get_google_oauth_models", {
    providerId,
    baseUrlOverride: baseUrlOverride ?? null,
  });
```

Replace with:

```ts
/** Fetch available Gemini models using the stored Antigravity OAuth token. */
export const getGoogleOAuthModels = (providerId: string): Promise<string[]> =>
  invoke("get_google_oauth_models", { providerId });
```

- [ ] **Step 2: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors (no existing frontend code calls this function yet with the old 2-arg form — it's currently unused dead code, matching the state confirmed at the start of this whole feature).

- [ ] **Step 3: Commit**

```bash
git add src/ipc/provider.ts
git commit -m "feat(ipc): drop base_url_override from getGoogleOAuthModels (fixed Antigravity endpoint)"
```

---

## Task 8: Wire `lib.rs` for the updated `get_google_oauth_models` signature

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Confirm nothing needs to change**

`google_oauth_login`, `google_oauth_logout`, and `get_google_oauth_models` are already imported and registered in `tauri::generate_handler!` (they were added when this dormant scaffold was first built). Only the FUNCTION SIGNATURE of `get_google_oauth_models` changed (Task 5), not its name — `tauri::generate_handler!` matches by identifier, so no `lib.rs` edit is needed for that alone.

Run: `grep -n "get_google_oauth_models\|google_oauth_login\|google_oauth_logout" src-tauri/src/lib.rs`
Expected: all three already present in both the import block and the `generate_handler!` list.

- [ ] **Step 2: Full workspace compile check**

Run: `cd src-tauri && cargo check 2>&1 | tail -40`
Expected: 0 errors.

- [ ] **Step 3: Run the full Rust test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -80`
Expected: all tests pass, including everything from Tasks 1–6.

- [ ] **Step 4: No commit needed for this task**

If Step 1's grep confirms all three commands are already wired (expected), there is nothing to change or commit here — this task exists purely to verify the assumption rather than blindly re-wire something already correct. If the grep comes back empty for any of the three (i.e. the assumption was wrong), add them to `lib.rs` following the exact pattern used for `codex_oauth_login`/`codex_oauth_logout`/`get_codex_oauth_models` in the same file, then commit as `feat(app): register Google OAuth commands with the Tauri IPC handler`.

---

## Task 9: `ProviderForm.tsx` — API Key / OAuth tabs for `google-ai`

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`

- [ ] **Step 1: Import the new IPC functions (already exist, just not imported here) and add state**

Find:

```ts
  getKimiModels,
  getKimiModelsByProvider,
  codexOAuthLogin,
  codexOAuthLogout,
  getCodexOAuthModels,
} from "../../ipc/provider";
```

Replace with:

```ts
  getKimiModels,
  getKimiModelsByProvider,
  codexOAuthLogin,
  codexOAuthLogout,
  getCodexOAuthModels,
  googleOAuthLogin,
  googleOAuthLogout,
  getGoogleOAuthModels,
} from "../../ipc/provider";
```

- [ ] **Step 2: Add Google auth-method + OAuth state, next to the Codex state block**

Find:

```ts
  const [codexModels, setCodexModels] = useState<string[]>([]);
  const [codexModelsLoading, setCodexModelsLoading] = useState(false);
  const [codexOAuthLoggedIn, setCodexOAuthLoggedIn] = useState(
    !!(existing?.provider_type === "codex" && existing?.auth_method === "oauth"),
  );
  const [saving, setSaving] = useState(false);
```

Replace with:

```ts
  const [codexModels, setCodexModels] = useState<string[]>([]);
  const [codexModelsLoading, setCodexModelsLoading] = useState(false);
  const [codexOAuthLoggedIn, setCodexOAuthLoggedIn] = useState(
    !!(existing?.provider_type === "codex" && existing?.auth_method === "oauth"),
  );
  const [googleAuthMethod, setGoogleAuthMethod] = useState<"api_key" | "oauth">(
    existing?.provider_type === "google-ai" && existing?.auth_method === "oauth" ? "oauth" : "api_key",
  );
  const [googleOAuthLoggedIn, setGoogleOAuthLoggedIn] = useState(
    !!(existing?.provider_type === "google-ai" && existing?.auth_method === "oauth"),
  );
  const [googleOAuthModels, setGoogleOAuthModels] = useState<string[]>([]);
  const [googleOAuthModelsLoading, setGoogleOAuthModelsLoading] = useState(false);
  const [onboardingWait, setOnboardingWait] = useState(false);
  const [saving, setSaving] = useState(false);
```

(`onboardingWait` drives a distinct, longer-form status message during the up-to-~50s onboarding wait, separate from the generic `authing` spinner used everywhere else.)

- [ ] **Step 3: Auto-fetch Gemini OAuth models when already logged in**

Find the Codex model-fetch `useEffect` (added in the earlier Codex plan):

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

Add a new effect immediately after it:

```ts
  useEffect(() => {
    if (providerType !== "google-ai" || googleAuthMethod !== "oauth" || !googleOAuthLoggedIn) return;
    const pid = id.trim();
    if (!pid) return;
    setGoogleOAuthModelsLoading(true);
    getGoogleOAuthModels(pid)
      .then((models) => {
        setGoogleOAuthModels(models);
        if (models.length > 0 && !models.includes(model)) setModel(models[0]);
      })
      .catch(() => setGoogleOAuthModels([]))
      .finally(() => setGoogleOAuthModelsLoading(false));
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, googleAuthMethod, googleOAuthLoggedIn, id]);
```

- [ ] **Step 4: `handleSave` — preserve `auth_method:"oauth"` for `google-ai` too**

Find:

```ts
        auth_method:
          providerType === "anthropic" ? anthropicAuthMethod
          : providerType === "codex" && codexOAuthLoggedIn ? "oauth"
          : null,
```

Replace with:

```ts
        auth_method:
          providerType === "anthropic" ? anthropicAuthMethod
          : providerType === "codex" && codexOAuthLoggedIn ? "oauth"
          : providerType === "google-ai" && googleAuthMethod === "oauth" && googleOAuthLoggedIn ? "oauth"
          : null,
```

- [ ] **Step 5: Add the auth-method tabs + OAuth login/logout UI**

Find (the existing `google-ai` model-fetch effect's guard already establishes `providerType === "google-ai"` is a valid branch point elsewhere in the file, but the AUTH SECTION currently has no `google-ai`-specific block at all — it's not excluded from anything, it's simply never mentioned there, so it silently falls into the generic password-field branch today).

Find the exact boundary where the Codex auth block ends and the generic API-key field begins:

```ts
          {providerType !== "github-copilot" && providerType !== "anthropic" && providerType !== "codex" && (
            <div className="form-group">
              <label>
                {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
              </label>
```

Replace with (inserting the new `google-ai` tabs + both sub-panels right before the generic field, and extending that field's exclusion condition):

```ts
          {providerType === "google-ai" && (
            <div className="form-group">
              <label>{t.settings_provider_auth_type}</label>
              <div className="anthropic-auth-tabs">
                <button
                  type="button"
                  className={`auth-tab ${googleAuthMethod === "api_key" ? "active" : ""}`}
                  onClick={() => { setGoogleAuthMethod("api_key"); setAuthStatus(null); }}
                >
                  {t.settings_provider_auth_api_key}
                </button>
                <button
                  type="button"
                  className={`auth-tab ${googleAuthMethod === "oauth" ? "active" : ""}`}
                  onClick={() => { setGoogleAuthMethod("oauth"); setApiKey(""); setAuthStatus(null); }}
                >
                  {t.settings_provider_auth_oauth("Google")}
                </button>
              </div>
            </div>
          )}

          {providerType === "google-ai" && googleAuthMethod === "api_key" && (
            <div className="form-group">
              <label>{t.provider_api_key}</label>
              <input
                type="password"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={isEdit ? t.provider_api_key_placeholder_edit : t.provider_api_key_placeholder_new}
                autoComplete="off"
              />
            </div>
          )}

          {providerType === "google-ai" && googleAuthMethod === "oauth" && (
            <div className="form-group">
              <label>{t.settings_provider_auth_oauth("")}</label>
              {googleOAuthLoggedIn ? (
                <div className="anthropic-oauth-done">
                  <span className="anthropic-oauth-ok">{t.settings_provider_oauth_ok}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    disabled={saving}
                    onClick={async () => {
                      if (!isEdit) return;
                      try {
                        await googleOAuthLogout(id.trim());
                        setGoogleOAuthLoggedIn(false);
                        setGoogleOAuthModels([]);
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
                      setOnboardingWait(false);
                      setAuthStatus(null);
                      // Onboarding a brand-new Google account can take up to
                      // ~50s (loadCodeAssist -> onboardUser polling); switch
                      // to a longer-wait message if login is still running
                      // after 5s so the user doesn't think it's stuck.
                      const onboardingHintTimer = setTimeout(() => setOnboardingWait(true), 5000);
                      try {
                        await googleOAuthLogin(id.trim());
                        setGoogleOAuthLoggedIn(true);
                        setAuthStatus(t.settings_provider_oauth_success);
                      } catch (e: unknown) {
                        setAuthStatus(t.settings_provider_oauth_err(String(e)));
                      } finally {
                        clearTimeout(onboardingHintTimer);
                        setOnboardingWait(false);
                        setAuthing(false);
                      }
                    }}
                  >
                    {authing
                      ? (onboardingWait ? t.settings_provider_oauth_onboarding_wait : t.provider_auth_running)
                      : t.settings_provider_btn_open_auth}
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

          {providerType !== "github-copilot" && providerType !== "anthropic" && providerType !== "codex" && providerType !== "google-ai" && (
            <div className="form-group">
              <label>
                {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
              </label>
```

(Leave the rest of that generic field block — the `<input type="password">` and its closing tags — unchanged; only the opening condition line changes.)

- [ ] **Step 6: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 1 error — `t.settings_provider_oauth_onboarding_wait` doesn't exist yet (Task 10 adds it). Confirm no *other* unrelated errors.

- [ ] **Step 7: Commit**

```bash
git add src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): add API-Key/OAuth tabs for google-ai provider"
```

---

## Task 10: `ProviderForm.tsx` — base_url exclusion + model field branch for Gemini OAuth

**Files:**
- Modify: `src/components/Settings/ProviderForm.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add the missing i18n key (zh-TW + en)**

In `src/lib/i18n.ts`, find the zh-TW block's `settings_provider_oauth_id_required` key (search for it) and add a new key immediately after it:

```ts
    settings_provider_oauth_id_required: "請先填寫 ID 再登入",
    settings_provider_oauth_onboarding_wait: "登入中…（首次使用可能需要最多 50 秒準備 Google Cloud 專案）",
```

Then find the corresponding English block's `settings_provider_oauth_id_required` key and add the English version immediately after it:

```ts
    settings_provider_oauth_id_required: "Please fill in the ID before logging in",
    settings_provider_oauth_onboarding_wait: "Signing in… (first-time setup may take up to 50s to prepare your Google Cloud project)",
```

(If the exact surrounding text of `settings_provider_oauth_id_required` differs slightly from what's shown, just add the new key in the equivalent position in both blocks — right after that key — rather than forcing an exact match.)

- [ ] **Step 2: Exclude `google-ai` OAuth mode from the base_url field**

Find:

```ts
        {(providerType === "ollama" ||
          providerType === "openai-compatible" ||
          providerType === "github-copilot" ||
          providerType === "google-ai" ||
          providerType === "anthropic-compatible") && (
```

Replace with:

```ts
        {(providerType === "ollama" ||
          providerType === "openai-compatible" ||
          providerType === "github-copilot" ||
          (providerType === "google-ai" && googleAuthMethod !== "oauth") ||
          providerType === "anthropic-compatible") && (
```

- [ ] **Step 3: Split the model field branch — API-key mode keeps the existing datalist, OAuth mode gets its own**

Find:

```ts
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
          ) : providerType === "openrouter" ? (
```

Replace with:

```ts
          ) : providerType === "google-ai" && googleAuthMethod === "oauth" ? (
            googleOAuthModelsLoading ? (
              <input type="text" value={t.provider_model_loading} disabled />
            ) : (
              <>
                <input
                  type="text"
                  list="google-oauth-models-list"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={googleOAuthModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
                />
                {googleOAuthModels.length > 0 && (
                  <datalist id="google-oauth-models-list">
                    {googleOAuthModels.map((m) => (
                      <option key={m} value={m} />
                    ))}
                  </datalist>
                )}
              </>
            )
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
          ) : providerType === "openrouter" ? (
```

(The pre-existing `providerType === "google-ai"` branch — the API-key/datalist path — is kept completely unchanged below the new OAuth branch; ternary order means the OAuth-specific check must come first since it's a more specific condition on the same `providerType` value.)

- [ ] **Step 4: Type-check**

Run: `npx tsc --noEmit -p tsconfig.app.json`
Expected: 0 errors.

- [ ] **Step 5: Lint**

Run: `npm run lint`
Expected: no new error/warning categories introduced by this file (compare against the pre-existing baseline — this file already has the same `react-hooks/set-state-in-effect` pattern on every sibling provider's model-fetch effect, including the ones just added).

- [ ] **Step 6: Commit**

```bash
git add src/lib/i18n.ts src/components/Settings/ProviderForm.tsx
git commit -m "feat(settings): add Gemini OAuth model picker and onboarding-wait messaging"
```

---

## Task 11: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -80`
Expected: all tests pass, including every Antigravity/Gemini test added in Tasks 1–6.

- [ ] **Step 2: Rust lint**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | tail -60`
Expected: no new warning categories introduced by `antigravity.rs`, `router.rs`, or `commands/provider.rs` changes.

- [ ] **Step 3: Frontend type check + test suite**

Run: `npx tsc --noEmit -p tsconfig.app.json && npx vitest run 2>&1 | tail -40`
Expected: 0 type errors; all existing frontend tests still pass (no new frontend tests were added — `ProviderForm.tsx` has no existing test file to extend, matching the precedent from the Codex plan).

- [ ] **Step 4: Frontend lint**

Run: `npm run lint`
Expected: no new errors/warnings in the files this plan touched.

- [ ] **Step 5: Manual end-to-end verification (requires a real Google account with Gemini Code Assist / Antigravity access — cannot be automated)**

Run `npm run tauri:dev`, open Settings → add/edit a `google-ai` provider → switch to the "Google" OAuth tab → click login → complete login in the browser with a real Google account, confirm:
- Token exchange succeeds (verify the `待驗證假設 #1` open item from the design spec: does Google's token endpoint accept an empty `client_secret` for this client, or does it need a real value? If it fails with an auth error mentioning the client secret, that confirms a real secret is needed — obtain it before this feature can ship).
- Onboarding completes (watch for the "登入中…(首次使用可能需要最多 50 秒...)" message if it's a fresh account) and a project id gets stored.
- The model datalist populates with real model ids (or the 3-item fallback if live discovery fails — note which happened).
- Save the provider, set as default, send an `/ai` query and an Ask AI chat message, confirm both produce real streamed Gemini responses.
- Test the exact same "Agent Mode" flow that broke Codex (`role:"system"` injection) against Gemini specifically, to confirm the `systemInstruction`-folding fix actually prevents a repeat of that bug class here.

- [ ] **Step 6: Final commit (only if Step 5 surfaced fixes)**

If manual verification required any fixes (especially the client_secret question — a real, non-empty value may need to replace the empty placeholder from Task 4), commit them separately with a clear message describing what was wrong and why. If everything works with no fixes needed, this task ends at Step 5 — nothing to commit.

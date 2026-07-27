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
        // A minimal real request is the only reliable way to validate an
        // Antigravity token + project id pair — there is no cheap read-only
        // endpoint reused here (model discovery lives in commands/provider.rs
        // and needs its own token, not this client's). Unlike `generate`,
        // this deliberately does NOT drain the SSE stream: dropping `resp`
        // after checking the status closes the connection, which is enough
        // to confirm the token+project id pair was accepted without paying
        // for (or waiting on) a full generation.
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
        let body = build_request_body(&self.model, &self.project_id, &probe);
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

            if chunk.candidates.is_empty() {
                if let Some(reason) = chunk.prompt_feedback.as_ref().and_then(|f| f.block_reason.clone()) {
                    return Err(AiError::ModelError {
                        reason,
                        raw: data.chars().take(300).collect(),
                    });
                }
            }

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
    #[serde(default, rename = "promptFeedback")]
    prompt_feedback: Option<GeminiPromptFeedback>,
}

#[derive(Deserialize)]
struct GeminiPromptFeedback {
    #[serde(default, rename = "blockReason")]
    block_reason: Option<String>,
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

/// Build the Antigravity `streamGenerateContent` request envelope.
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

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

/// Build the Responses API request body. `instructions` is Codex's required
/// system-prompt-equivalent — the backend rejects requests without it.
pub(crate) fn build_request_body(model: &str, req: &GenerateRequest) -> serde_json::Value {
    let input: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let text = match &m.content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            // The Responses API rejects `role: "system"` input items outright
            // ("System messages are not allowed") — unlike Chat Completions,
            // which accepts a system message natively. Callers that inject a
            // system-role message directly into history (e.g. the Agent Mode
            // loop in AiPanel/index.tsx, which sends `{role:"system",...}` as
            // its own orchestration prompt, separate from `system_prompt`
            // below) would otherwise make every Codex request fail with a
            // silent 400. Remap to "developer", the Responses API's
            // equivalent role for system-level instructions mid-conversation.
            let role = if m.role == "system" { "developer" } else { m.role.as_str() };
            serde_json::json!({
                "type": "message",
                "role": role,
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

    /// Regression test: the Agent Mode loop (AiPanel/index.tsx) injects its own
    /// orchestration prompt as `{role:"system",...}` directly into the message
    /// history (separate from GenerateRequest.system_prompt). Codex's Responses
    /// API rejects any input item with role "system" outright ("System messages
    /// are not allowed"), which silently broke every Codex request made from
    /// Agent Mode. Must be remapped to "developer".
    #[test]
    fn system_role_message_is_remapped_to_developer() {
        let r = req(
            "sys",
            vec![
                ChatMessage { role: "system".into(), content: serde_json::json!("orchestration prompt"), tool_call_id: None, tool_calls: None },
                ChatMessage { role: "user".into(), content: serde_json::json!("go"), tool_call_id: None, tool_calls: None },
            ],
        );
        let body = build_request_body("gpt-5.1-codex", &r);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"][0]["text"], "orchestration prompt");
        assert_eq!(input[1]["role"], "user", "other roles must pass through unchanged");
    }

    #[test]
    fn sse_event_output_text_delta_parses() {
        let raw = r#"{"type":"response.output_text.delta","delta":"Hello"}"#;
        let event: CodexSseEvent = serde_json::from_str(raw).unwrap();
        match event {
            CodexSseEvent::OutputTextDelta { delta } => assert_eq!(delta, "Hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sse_event_completed_parses_usage() {
        let raw = r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":2}}}"#;
        let event: CodexSseEvent = serde_json::from_str(raw).unwrap();
        match event {
            CodexSseEvent::Completed { response } => {
                let usage = response.expect("response present").usage.expect("usage present");
                assert_eq!(usage.input_tokens, 10);
                assert_eq!(usage.output_tokens, 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sse_event_unknown_type_parses_as_other() {
        let raw = r#"{"type":"response.created"}"#;
        let event: CodexSseEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(event, CodexSseEvent::Other));
    }

    #[test]
    fn sse_event_failed_parses() {
        let raw = r#"{"type":"response.failed","response":{"error":{"message":"something broke"}}}"#;
        let event: CodexSseEvent = serde_json::from_str(raw).unwrap();
        match event {
            CodexSseEvent::Failed { response } => {
                let error = response.expect("response present").error.expect("error present");
                assert_eq!(error["message"], "something broke");
            }
            _ => panic!("wrong variant"),
        }
    }
}

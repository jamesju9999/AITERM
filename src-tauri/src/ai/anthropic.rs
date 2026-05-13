//! Anthropic Claude provider — `https://api.anthropic.com/v1/messages` (SSE).
//!
//! API reference: https://docs.anthropic.com/en/api/messages
//!
//! Key differences from OpenAI format:
//! - `system` is a top-level field, NOT inside the messages array.
//! - Auth uses `x-api-key` header + `anthropic-version`.
//! - SSE events: `content_block_delta` carries text; `message_stop` signals done.
//! - Status 529 means "overloaded" (treat as Network error).

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, map_http_error, separator_len},
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, TokenUsage,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_base_url(api_key, model, "https://api.anthropic.com".into())
    }

    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self { api_key, model, base_url, client: reqwest::Client::new() }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl AiProvider for AnthropicClient {
    fn id(&self) -> &str { "anthropic" }
    fn display_name(&self) -> &str { "Anthropic" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req, true);
        let resp = self
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            // 529 = Anthropic overloaded
            if status.as_u16() == 529 {
                return Err(AiError::Network { message: "Anthropic API is overloaded".into() });
            }
            return Err(map_http_error(status, resp).await);
        }
        consume_anthropic_sse(resp, tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // Minimal 1-token non-streaming request.
        let hc_req = health_check_request();
        let body = build_request_body(&self.model, &hc_req, false);
        let resp = self
            .client
            .post(self.messages_url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else if status.as_u16() == 529 {
            Err(AiError::Network { message: "Anthropic API is overloaded".into() })
        } else {
            Err(map_http_error(status, resp).await)
        }
    }
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    system: &'a str,
    messages: Vec<AnthropicMessage<'a>>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

fn build_request_body<'a>(
    model: &'a str,
    req: &'a GenerateRequest,
    stream: bool,
) -> AnthropicRequest<'a> {
    let messages = req
        .messages
        .iter()
        .map(|m| AnthropicMessage { role: m.role.as_str(), content: m.content.as_str().unwrap_or("") })
        .collect();
    AnthropicRequest {
        model,
        system: &req.system_prompt,
        messages,
        max_tokens: req.max_tokens.unwrap_or(1024),
        stream,
    }
}

fn health_check_request() -> GenerateRequest {
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;
    GenerateRequest {
        system_prompt: "ping".into(),
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi") }],
        context: EnvSnapshot {
            os: std::env::consts::OS.into(),
            shell: "sh".into(),
            cwd: PathBuf::from("."),
            ..Default::default()
        },
        mode: QueryMode::SingleCommand,
        max_tokens: Some(1),
    }
}

// ── SSE consumer ─────────────────────────────────────────────────────────────

async fn consume_anthropic_sse(
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
            if line.is_empty() { continue; }

            // SSE lines: "event: ..." or "data: ..."
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                match serde_json::from_str::<AnthropicSseEvent>(data) {
                    Ok(event) => match event {
                        AnthropicSseEvent::ContentBlockDelta { delta } => {
                            if let Some(text) = delta.text {
                                let _ = tx
                                    .send(GenerateChunk { delta: text, done: false, usage: None })
                                    .await;
                            }
                        }
                        AnthropicSseEvent::MessageDelta { usage } => {
                            let token_usage = usage.map(|u| TokenUsage {
                                prompt: u.input_tokens,
                                completion: u.output_tokens,
                            });
                            let _ = tx
                                .send(GenerateChunk { delta: String::new(), done: false, usage: token_usage })
                                .await;
                        }
                        AnthropicSseEvent::MessageStop => {
                            let _ = tx
                                .send(GenerateChunk { delta: String::new(), done: true, usage: None })
                                .await;
                            saw_done = true;
                            break 'outer;
                        }
                        AnthropicSseEvent::Other => {}
                    },
                    Err(_) => continue,
                }
            }
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

// ── SSE event types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicSseEvent {
    ContentBlockDelta {
        delta: ContentDelta,
    },
    MessageDelta {
        #[serde(default)]
        usage: Option<MessageDeltaUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ContentDelta {
    #[serde(rename = "type")]
    _ty: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct MessageDeltaUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "You are a terminal assistant.".into(),
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files") }],
            context: EnvSnapshot { os: "windows".into(), shell: "pwsh".into(), cwd: PathBuf::from("C:\\"), ..Default::default() },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(256),
        }
    }

    #[test]
    fn request_body_puts_system_at_top_level() {
        let req = sample_req();
        let body = build_request_body("claude-sonnet-4-5", &req, true);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["system"], "You are a terminal assistant.");
        assert!(json["messages"][0]["role"] == "user");
        // system must NOT appear inside messages array
        for msg in json["messages"].as_array().unwrap() {
            assert_ne!(msg["role"], "system", "system must not be in messages array");
        }
        assert_eq!(json["stream"], true);
        assert_eq!(json["model"], "claude-sonnet-4-5");
    }

    #[test]
    fn sse_event_content_block_delta_parses() {
        let raw = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        match event {
            AnthropicSseEvent::ContentBlockDelta { delta } => {
                assert_eq!(delta.text.unwrap(), "hello");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sse_event_message_stop_parses() {
        let raw = r#"{"type":"message_stop"}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(event, AnthropicSseEvent::MessageStop));
    }

    #[test]
    fn sse_event_unknown_type_parses_as_other() {
        let raw = r#"{"type":"ping"}"#;
        let event: AnthropicSseEvent = serde_json::from_str(raw).unwrap();
        assert!(matches!(event, AnthropicSseEvent::Other));
    }
}

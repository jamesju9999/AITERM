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
    AiError, AiProvider, AiToolCall, ChatMessage, GenerateChunk, GenerateRequest,
    GenerateWithToolsResult, McpToolDefinition, TokenUsage,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicClient {
    token: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
    is_oauth: bool,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String) -> Self {
        Self::with_base_url(api_key, model, "https://api.anthropic.com".into())
    }

    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self { token: api_key, model, base_url, client: reqwest::Client::new(), is_oauth: false }
    }

    pub fn with_oauth(access_token: String, model: String, base_url: String) -> Self {
        Self { token: access_token, model, base_url, client: reqwest::Client::new(), is_oauth: true }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }

    fn auth_request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth {
            builder
                .header("Authorization", format!("Bearer {}", self.token))
                .header("anthropic-beta", "oauth-2025-04-20")
        } else {
            builder.header("x-api-key", &self.token)
        }
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
            .auth_request(self.client.post(self.messages_url()))
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

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let tool_defs: serde_json::Value = serde_json::Value::Array(
            tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })).collect()
        );

        let messages: Vec<serde_json::Value> = build_anthropic_messages(&req.messages);

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": req.system_prompt,
            "messages": messages,
            "tools": tool_defs
        });

        let resp = self
            .auth_request(self.client.post(self.messages_url()))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status == 401 { return Err(AiError::AuthFailed); }
        if status.as_u16() == 529 {
            return Err(AiError::Network { message: "Anthropic API is overloaded".into() });
        }
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }

        let json: serde_json::Value = resp.json().await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let stop_reason = json["stop_reason"].as_str().unwrap_or("");

        if stop_reason == "tool_use" {
            let content_blocks = json["content"].as_array()
                .ok_or_else(|| AiError::ModelError {
                    reason: "missing content".into(),
                    raw: json.to_string(),
                })?;

            let calls: Vec<AiToolCall> = content_blocks.iter()
                .filter(|b| b["type"].as_str() == Some("tool_use"))
                .map(|b| AiToolCall {
                    id: b["id"].as_str().unwrap_or("").to_string(),
                    tool_name: b["name"].as_str().unwrap_or("").to_string(),
                    args: b["input"].clone(),
                    thought_signature: None,
                })
                .collect();

            return Ok(GenerateWithToolsResult::ToolCalls {
                calls,
                raw: Some(serde_json::Value::Array(content_blocks.clone())),
            });
        }

        let content = json["content"][0]["text"].as_str().unwrap_or("").to_string();
        let _ = tx.send(GenerateChunk { delta: content.clone(), done: true, usage: None }).await;
        Ok(GenerateWithToolsResult::Text(content))
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // Minimal 1-token non-streaming request.
        let hc_req = health_check_request();
        let body = build_request_body(&self.model, &hc_req, false);
        let resp = self
            .auth_request(self.client.post(self.messages_url()))
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
struct AnthropicRequest {
    model: String,
    system: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

/// Convert internal ChatMessage history into Anthropic's Messages API format.
/// Anthropic has no "tool" role: tool calls live inside an assistant message's
/// `content` array as `tool_use` blocks, and tool results are wrapped in a
/// `user` message's `content` array as `tool_result` blocks. Consecutive
/// `role: "tool"` ChatMessages (parallel tool calls) are coalesced into one
/// user turn, since Anthropic requires strictly alternating roles.
fn build_anthropic_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut result: Vec<serde_json::Value> = Vec::with_capacity(messages.len());
    let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();

    for m in messages {
        if m.role == "tool" {
            let tool_use_id = m.tool_call_id.clone().unwrap_or_else(|| {
                log::warn!("tool-role ChatMessage missing tool_call_id; sending empty tool_use_id to Anthropic");
                String::new()
            });
            pending_tool_results.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": m.content.clone(),
            }));
            continue;
        }

        flush_tool_results(&mut result, &mut pending_tool_results);

        if m.role == "assistant" {
            if let Some(tool_calls) = &m.tool_calls {
                result.push(serde_json::json!({
                    "role": "assistant",
                    "content": to_anthropic_content_blocks(tool_calls),
                }));
                continue;
            }
        }

        result.push(serde_json::json!({ "role": m.role, "content": m.content }));
    }

    flush_tool_results(&mut result, &mut pending_tool_results);
    result
}

fn flush_tool_results(result: &mut Vec<serde_json::Value>, pending: &mut Vec<serde_json::Value>) {
    if !pending.is_empty() {
        result.push(serde_json::json!({
            "role": "user",
            "content": std::mem::take(pending),
        }));
    }
}

/// Convert a ChatMessage's `tool_calls` value into Anthropic content blocks.
/// Handles two possible shapes: Anthropic-native (already `tool_use` blocks,
/// e.g. echoed back verbatim from a prior `raw`) and OpenAI-shaped (the
/// frontend's fallback reconstruction, `function.arguments` as a JSON string).
/// Detection: OpenAI-shaped elements always have a `"function"` key; Anthropic
/// content blocks (whether `text` or `tool_use`) never do — so checking only
/// the first element would misdetect a `[text, tool_use]` raw echo.
fn to_anthropic_content_blocks(tool_calls: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(arr) = tool_calls.as_array() else {
        return vec![];
    };

    let is_openai_shaped = arr.iter().any(|el| el.get("function").is_some());
    if !is_openai_shaped {
        return arr.clone();
    }

    arr.iter()
        // Assumes a homogeneous array (all-native or all-OpenAI-shaped, never mixed) —
        // a genuinely mixed array would silently drop the native blocks here.
        .filter(|el| el.get("function").is_some())
        .map(|el| {
            let arguments = el["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(serde_json::json!({}));
            serde_json::json!({
                "type": "tool_use",
                "id": el["id"],
                "name": el["function"]["name"],
                "input": arguments,
            })
        })
        .collect()
}

fn build_request_body(
    model: &str,
    req: &GenerateRequest,
    stream: bool,
) -> AnthropicRequest {
    let messages = req
        .messages
        .iter()
        .map(|m| AnthropicMessage { role: m.role.clone(), content: m.content.clone() })
        .collect();
    AnthropicRequest {
        model: model.to_owned(),
        system: req.system_prompt.clone(),
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
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi"), tool_call_id: None, tool_calls: None }],
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
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("list files"), tool_call_id: None, tool_calls: None }],
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

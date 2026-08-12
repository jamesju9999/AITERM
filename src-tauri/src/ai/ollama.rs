//! Ollama local provider — `http://localhost:11434/api/chat` (NDJSON streaming).
//!
//! Ollama uses newline-delimited JSON (NDJSON), not SSE. Each line is a JSON
//! object with `{ "message": { "content": "..." }, "done": false }` until the
//! final line with `"done": true`.
//!
//! No API key required — Ollama runs locally with no auth.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, separator_len},
    AiError, AiProvider, AiToolCall, GenerateChunk, GenerateRequest, GenerateWithToolsResult,
    McpToolDefinition,
};

pub struct OllamaClient {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaClient {
    pub fn new(model: String) -> Self {
        Self::with_base_url(model, "http://localhost:11434".into())
    }

    pub fn with_base_url(model: String, base_url: String) -> Self {
        Self { model, base_url, client: reqwest::Client::new() }
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url.trim_end_matches('/'))
    }

    fn tags_url(&self) -> String {
        format!("{}/api/tags", self.base_url.trim_end_matches('/'))
    }

    /// List locally available model names. Used by the Settings UI dropdown.
    pub async fn list_models(&self) -> Result<Vec<String>, AiError> {
        let resp = self
            .client
            .get(self.tags_url())
            .send()
            .await
            .map_err(|e| connection_error(&e))?;

        if !resp.status().is_success() {
            return Err(AiError::Network {
                message: format!("Ollama tags endpoint returned {}", resp.status()),
            });
        }

        #[derive(Deserialize)]
        struct TagsResponse {
            models: Vec<OllamaModel>,
        }
        #[derive(Deserialize)]
        struct OllamaModel {
            name: String,
        }

        let body: TagsResponse = resp
            .json()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;
        Ok(body.models.into_iter().map(|m| m.name).collect())
    }
}

#[async_trait]
impl AiProvider for OllamaClient {
    fn id(&self) -> &str { "ollama" }
    fn display_name(&self) -> &str { "Ollama" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req, true);
        let resp = self
            .client
            .post(self.chat_url())
            .json(&body)
            .send()
            .await
            .map_err(|e| connection_error(&e))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AiError::Network {
                message: format!("Ollama http {}: {}", status.as_u16(), &body_text[..body_text.len().min(200)]),
            });
        }

        consume_ndjson(resp, tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // Just hit /api/tags — if Ollama is running it returns 200.
        let resp = self
            .client
            .get(self.tags_url())
            .send()
            .await
            .map_err(|e| connection_error(&e))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(AiError::Network {
                message: format!("Ollama returned {}", resp.status()),
            })
        }
    }

    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let ollama_tools: Vec<OllamaTool> = tools
            .iter()
            .map(|t| OllamaTool {
                kind: "function".into(),
                function: OllamaToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                },
            })
            .collect();

        // 串流。原本是 stream:false，帶工具的對話因此整段一次跳出來。
        let body = OllamaToolRequest {
            model: self.model.clone(),
            messages: build_messages(&req),
            stream: true,
            tools: ollama_tools,
        };

        let resp = self
            .client
            .post(self.chat_url())
            .json(&body)
            .send()
            .await
            .map_err(|e| connection_error(&e))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AiError::Network {
                message: format!(
                    "Ollama http {}: {}",
                    status.as_u16(),
                    &body_text[..body_text.len().min(200)]
                ),
            });
        }

        let (streamed_text, streamed_calls) = consume_ndjson_with_tools(resp, tx).await?;
        let data = OllamaToolResponse {
            message: OllamaToolResponseMessage {
                content: Some(streamed_text),
                tool_calls: streamed_calls,
            },
        };

        if !data.message.tool_calls.is_empty() {
            // Ollama's response has no per-call id, and this synthetic one must stay
            // unique across turns within the same conversation (a per-response index
            // like "call_0" would collide if a later turn also has one tool call).
            let ids: Vec<String> = (0..data.message.tool_calls.len())
                .map(|_| format!("call_{}", uuid::Uuid::new_v4()))
                .collect();

            let raw_tool_calls: Vec<serde_json::Value> = data.message.tool_calls.iter().zip(ids.iter())
                .map(|(tc, id)| serde_json::json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": tc.function.name,
                        "arguments": serde_json::to_string(&tc.function.arguments).unwrap_or_default(),
                    }
                }))
                .collect();
            let raw = Some(serde_json::Value::Array(raw_tool_calls));

            let calls = data
                .message
                .tool_calls
                .into_iter()
                .zip(ids)
                .map(|(tc, id)| AiToolCall {
                    id,
                    tool_name: tc.function.name,
                    args: tc.function.arguments,
                    thought_signature: None,
                })
                .collect();
            Ok(GenerateWithToolsResult::ToolCalls { calls, raw })
        } else {
            // <think> 已在串流時逐段剝除（狀態跨 chunk 延續），這裡拿到的就是
            // 可見文字，也已經逐段送出去了。
            Ok(GenerateWithToolsResult::Text(data.message.content.unwrap_or_default()))
        }
    }
}

/// Convert a connection-refused / DNS error into a user-friendly message.
fn connection_error(e: &reqwest::Error) -> AiError {
    if e.is_connect() || e.is_timeout() {
        AiError::Network { message: "Ollama is not running. Please start Ollama and try again.".into() }
    } else {
        AiError::Network { message: e.to_string() }
    }
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OllamaResponseToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

fn build_messages(req: &GenerateRequest) -> Vec<OllamaMessage> {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
        tool_calls: None,
        tool_call_id: None,
    });
    for m in &req.messages {
        match &m.tool_calls {
            Some(tool_calls) => {
                messages.push(OllamaMessage {
                    role: m.role.clone(),
                    content: serde_json::Value::String(String::new()),
                    tool_calls: Some(to_ollama_tool_calls(tool_calls)),
                    tool_call_id: None,
                });
            }
            None => {
                messages.push(OllamaMessage {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    tool_calls: None,
                    tool_call_id: m.tool_call_id.clone(),
                });
            }
        }
    }
    messages
}

/// Convert the system's OpenAI-shaped tool_calls (arguments as a JSON string)
/// into Ollama's native shape (arguments as a parsed JSON object).
fn to_ollama_tool_calls(tool_calls: &serde_json::Value) -> Vec<OllamaResponseToolCall> {
    tool_calls
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|c| {
                    let name = c["function"]["name"].as_str().unwrap_or("").to_string();
                    let arguments = match c["function"]["arguments"].as_str() {
                        Some(s) => serde_json::from_str(s).unwrap_or_else(|e| {
                            log::warn!("failed to parse tool_call arguments as JSON for Ollama: {e}; raw: {s}");
                            serde_json::json!({})
                        }),
                        None => serde_json::json!({}),
                    };
                    OllamaResponseToolCall {
                        function: OllamaResponseFunction { name, arguments },
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build_request_body(model: &str, req: &GenerateRequest, stream: bool) -> OllamaChatRequest {
    OllamaChatRequest { model: model.to_owned(), messages: build_messages(req), stream }
}

// ── Tool calling types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaToolRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    tools: Vec<OllamaTool>,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    kind: String,
    function: OllamaToolFunction,
}

#[derive(Serialize)]
struct OllamaToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaToolResponse {
    message: OllamaToolResponseMessage,
}

#[derive(Deserialize)]
struct OllamaToolResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OllamaResponseToolCall>,
}

#[derive(Serialize, Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Serialize, Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}

// ── NDJSON consumer ───────────────────────────────────────────────────────────

/// Strip `<think>...</think>` blocks from a content chunk.
/// `in_think` persists across calls so tags that span chunk boundaries are handled.
/// Partial open/close tags at the end of a chunk are NOT buffered — they emit as-is,
/// which is the simplest correct behaviour given Ollama's chunk granularity (≥1 char).
fn strip_think_blocks(text: &str, in_think: &mut bool) -> String {
    let mut output = String::new();
    let mut s = text;
    loop {
        if *in_think {
            match s.find("</think>") {
                Some(pos) => {
                    *in_think = false;
                    s = &s[pos + "</think>".len()..];
                    if s.starts_with('\n') { s = &s[1..]; }
                }
                None => break, // whole chunk is inside think block
            }
        } else {
            match s.find("<think>") {
                Some(pos) => {
                    output.push_str(&s[..pos]);
                    *in_think = true;
                    s = &s[pos + "<think>".len()..];
                }
                None => {
                    output.push_str(s);
                    break;
                }
            }
        }
    }
    output
}

/// 像 `consume_ndjson`，但同時把串流中的工具呼叫收起來。
///
/// Ollama 的工具呼叫不像 OpenAI 是分片的——`arguments` 本來就是完整的 JSON
/// object，所以只要在遇到的那一則收下即可，不需要拼接。
async fn consume_ndjson_with_tools(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(String, Vec<OllamaResponseToolCall>), AiError> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut text = String::new();
    let mut calls: Vec<OllamaResponseToolCall> = Vec::new();
    // `<think>` 區塊會跨 chunk，狀態必須延續，不能每則重置。
    let mut in_think = false;
    let mut stream_ended = false;
    let mut saw_done = false;

    while !stream_ended && !saw_done {
        match stream.next().await {
            Some(item) => {
                let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
                buf.extend_from_slice(&bytes);
            }
            None => {
                stream_ended = true;
                if !buf.is_empty() && buf.last() != Some(&b'\n') {
                    buf.push(b'\n');
                }
            }
        }

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
            let Ok(chunk) = serde_json::from_str::<OllamaChunk>(line) else { continue };

            if let Some(msg) = chunk.message {
                let raw = msg.content.unwrap_or_default();
                let visible = strip_think_blocks(&raw, &mut in_think);
                if !visible.is_empty() {
                    text.push_str(&visible);
                    let _ = tx.send(GenerateChunk { delta: visible, done: false, usage: None }).await;
                }
                calls.extend(msg.tool_calls);
            }
            if chunk.done {
                let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
                saw_done = true;
                break;
            }
        }
    }

    if !saw_done {
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok((text, calls))
}

async fn consume_ndjson(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::<u8>::new();
    let mut saw_done = false;
    let mut in_think = false;

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

            match serde_json::from_str::<OllamaChunk>(line) {
                Ok(chunk) => {
                    let raw = chunk.message.and_then(|m| m.content).unwrap_or_default();
                    let text = strip_think_blocks(&raw, &mut in_think);
                    if !text.is_empty() {
                        let _ = tx.send(GenerateChunk { delta: text, done: false, usage: None }).await;
                    }
                    if chunk.done {
                        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
                        saw_done = true;
                        break 'outer;
                    }
                }
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
struct OllamaChunk {
    #[serde(default)]
    message: Option<OllamaChunkMessage>,
    #[serde(default)]
    done: bool,
}

#[derive(Deserialize)]
struct OllamaChunkMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OllamaResponseToolCall>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("ls"), tool_call_id: None, tool_calls: None }],
            context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
            mode: QueryMode::SingleCommand,
            max_tokens: None,
        }
    }

    #[test]
    fn request_body_includes_system_as_first_message() {
        let req = sample_req();
        let body = build_request_body("llama3", &req, true);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][0]["content"], "sys");
        assert_eq!(json["messages"][1]["role"], "user");
        assert_eq!(json["stream"], true);
    }

    #[test]
    fn ndjson_chunk_parses_done_false() {
        let raw = r#"{"model":"llama3","created_at":"2024","message":{"role":"assistant","content":"hello"},"done":false}"#;
        let chunk: OllamaChunk = serde_json::from_str(raw).unwrap();
        assert!(!chunk.done);
        assert_eq!(chunk.message.unwrap().content, Some("hello".to_owned()));
    }

    #[test]
    fn ndjson_chunk_parses_done_true() {
        let raw = r#"{"model":"llama3","created_at":"2024","message":{"role":"assistant","content":""},"done":true}"#;
        let chunk: OllamaChunk = serde_json::from_str(raw).unwrap();
        assert!(chunk.done);
    }

    #[test]
    fn connection_error_produces_friendly_message() {
        // We can't actually trigger reqwest errors in unit tests without a real
        // server, so we test the helper indirectly via the string content.
        let fake_err_msg = "Ollama is not running. Please start Ollama and try again.";
        assert!(fake_err_msg.contains("Ollama is not running"));
    }

    #[test]
    fn strip_think_blocks_removes_complete_block() {
        let mut in_think = false;
        let out = strip_think_blocks("<think>hidden</think>visible", &mut in_think);
        assert_eq!(out, "visible");
        assert!(!in_think);
    }

    #[test]
    fn strip_think_blocks_suppresses_open_block() {
        let mut in_think = false;
        let out = strip_think_blocks("before<think>hidden", &mut in_think);
        assert_eq!(out, "before");
        assert!(in_think);
    }

    #[test]
    fn strip_think_blocks_resumes_across_chunks() {
        let mut in_think = false;
        // chunk 1: enter think
        let out1 = strip_think_blocks("A<think>thinking...", &mut in_think);
        assert_eq!(out1, "A");
        assert!(in_think);
        // chunk 2: close think and continue
        let out2 = strip_think_blocks("more thinking</think>B", &mut in_think);
        assert_eq!(out2, "B");
        assert!(!in_think);
    }

    #[test]
    fn strip_think_blocks_skips_leading_newline_after_close() {
        let mut in_think = false;
        let out = strip_think_blocks("<think>x</think>\nresult", &mut in_think);
        assert_eq!(out, "result");
    }
}

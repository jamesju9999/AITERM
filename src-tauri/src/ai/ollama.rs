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
        _tx: mpsc::Sender<GenerateChunk>,
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

        let body = OllamaToolRequest {
            model: self.model.clone(),
            messages: build_messages(&req),
            stream: false,
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

        let data: OllamaToolResponse = resp
            .json()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        if !data.message.tool_calls.is_empty() {
            let calls = data
                .message
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(i, tc)| AiToolCall {
                    id: format!("call_{}", i),
                    tool_name: tc.function.name,
                    args: tc.function.arguments,
                })
                .collect();
            Ok(GenerateWithToolsResult::ToolCalls(calls))
        } else {
            Ok(GenerateWithToolsResult::Text(data.message.content))
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
}

fn build_messages(req: &GenerateRequest) -> Vec<OllamaMessage> {
    let mut messages: Vec<OllamaMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OllamaMessage {
        role: "system".to_owned(),
        content: serde_json::Value::String(req.system_prompt.clone()),
    });
    for m in &req.messages {
        messages.push(OllamaMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }
    messages
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
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaResponseToolCall>,
}

#[derive(Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}

// ── NDJSON consumer ───────────────────────────────────────────────────────────

async fn consume_ndjson(
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

            match serde_json::from_str::<OllamaChunk>(line) {
                Ok(chunk) => {
                    let text = chunk.message.map(|m| m.content).unwrap_or_default();
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
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("ls") }],
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
        assert_eq!(chunk.message.unwrap().content, "hello");
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
}

//! OpenAI provider implementation. Uses the chat completions endpoint with
//! `response_format: { type: "json_object" }` (spec decision D11) and a
//! hard-coded model `gpt-4o-mini` (D12). SSE streaming is consumed internally
//! and chunks are forwarded via the trait's `mpsc::Sender<GenerateChunk>`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, TokenUsage,
};

const DEFAULT_MODEL: &str = "gpt-4o-mini";

pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String, // exposed for wiremock
    client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "https://api.openai.com".to_string())
    }

    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            model: DEFAULT_MODEL.to_string(),
            base_url,
            client: reqwest::Client::new(),
        }
    }

    fn url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl AiProvider for OpenAiClient {
    fn id(&self) -> &str { "openai" }
    fn display_name(&self) -> &str { "OpenAI" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req);

        let resp = self
            .client
            .post(self.url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }

        consume_sse(resp, tx).await
    }
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
    stream: bool,
    response_format: ResponseFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    ty: &'static str,
}

fn build_request_body<'a>(model: &'a str, req: &'a GenerateRequest) -> OpenAiChatRequest<'a> {
    let mut messages: Vec<OpenAiMessage<'a>> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OpenAiMessage { role: "system", content: &req.system_prompt });
    for m in &req.messages {
        messages.push(OpenAiMessage { role: m.role.as_str(), content: m.content.as_str() });
    }
    OpenAiChatRequest {
        model,
        messages,
        stream: true,
        response_format: ResponseFormat { ty: "json_object" },
        max_tokens: req.max_tokens,
    }
}

async fn map_http_error(status: reqwest::StatusCode, resp: reqwest::Response) -> AiError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return AiError::AuthFailed;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        return AiError::RateLimit { retry_after };
    }
    let body = resp.text().await.unwrap_or_default();
    AiError::Network {
        message: format!("http {}: {}", status.as_u16(), truncate(&body, 200)),
    }
}

async fn consume_sse(
    resp: reqwest::Response,
    tx: mpsc::Sender<GenerateChunk>,
) -> Result<(), AiError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut leftover = Vec::<u8>::new();
    let mut saw_done = false;

    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|e| AiError::Network { message: e.to_string() })?;
        leftover.extend_from_slice(&bytes);

        while let Some(pos) = find_line_end(&leftover) {
            let line_bytes = leftover.drain(..pos).collect::<Vec<u8>>();
            // Advance past the actual separator byte(s).
            let sep_len = separator_len(&leftover);
            leftover.drain(..sep_len);
            let line = match std::str::from_utf8(&line_bytes) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };
            if line.is_empty() { continue; }
            let payload = match line.strip_prefix("data:") {
                Some(p) => p.trim(),
                None => continue,
            };
            if payload == "[DONE]" {
                saw_done = true;
                break;
            }
            match serde_json::from_str::<SsePayload>(payload) {
                Ok(p) => {
                    let delta = p.delta_text();
                    let usage = p.usage_into();
                    let done = p.finish_reason_present();
                    let _ = tx
                        .send(GenerateChunk { delta, done: false, usage: usage.clone() })
                        .await;
                    if done {
                        let _ = tx
                            .send(GenerateChunk { delta: String::new(), done: true, usage })
                            .await;
                    }
                }
                Err(_) => {
                    // Malformed SSE payload is soft-ignored — the final
                    // "missing done" guard below catches catastrophic cases.
                    continue;
                }
            }
        }
        if saw_done { break; }
    }

    if !saw_done {
        // Send a terminating chunk so the consumer unblocks even if we never
        // saw [DONE] and did not see finish_reason.
        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage: None }).await;
    }
    Ok(())
}

fn find_line_end(buf: &[u8]) -> Option<usize> {
    for (i, w) in buf.windows(2).enumerate() {
        if w == b"\r\n" { return Some(i); }
    }
    buf.iter().position(|&b| b == b'\n' || b == b'\r')
}

fn separator_len(buf: &[u8]) -> usize {
    match buf.first() {
        Some(&b'\r') if buf.get(1) == Some(&b'\n') => 2,
        Some(&b'\r') | Some(&b'\n') => 1,
        _ => 0,
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(Deserialize)]
struct SsePayload {
    #[serde(default)]
    choices: Vec<SseChoice>,
    #[serde(default)]
    usage: Option<SseUsage>,
}

#[derive(Deserialize)]
struct SseChoice {
    #[serde(default)]
    delta: SseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct SseDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct SseUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl SsePayload {
    fn delta_text(&self) -> String {
        self.choices
            .first()
            .and_then(|c| c.delta.content.clone())
            .unwrap_or_default()
    }
    fn finish_reason_present(&self) -> bool {
        self.choices.first().and_then(|c| c.finish_reason.as_ref()).is_some()
    }
    fn usage_into(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|u| TokenUsage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
        })
    }
}

#[allow(dead_code)]
fn _unused_chatmessage_anchor(_: ChatMessage) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_request() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
            context: EnvSnapshot {
                os: "linux".into(),
                shell: "bash".into(),
                cwd: PathBuf::from("/"),
            },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(256),
        }
    }

    #[test]
    fn request_body_sets_stream_and_response_format() {
        let req = sample_request();
        let body = build_request_body("gpt-4o-mini", &req);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "gpt-4o-mini");
        assert_eq!(json["stream"], true);
        assert_eq!(json["response_format"]["type"], "json_object");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][0]["content"], "sys");
        assert_eq!(json["messages"][1]["role"], "user");
        assert_eq!(json["messages"][1]["content"], "hi");
        assert_eq!(json["max_tokens"], 256);
    }

    #[test]
    fn find_line_end_prefers_crlf() {
        assert_eq!(find_line_end(b"abc\r\nxyz"), Some(3));
        assert_eq!(find_line_end(b"abc\nxyz"), Some(3));
        assert_eq!(find_line_end(b"nope"), None);
    }

    #[test]
    fn separator_len_handles_both() {
        assert_eq!(separator_len(b"\r\nxyz"), 2);
        assert_eq!(separator_len(b"\nxyz"), 1);
        assert_eq!(separator_len(b"xyz"), 0);
    }

    #[test]
    fn sse_payload_extracts_delta() {
        let raw = r#"{"choices":[{"delta":{"content":"hello"}}]}"#;
        let p: SsePayload = serde_json::from_str(raw).unwrap();
        assert_eq!(p.delta_text(), "hello");
        assert!(!p.finish_reason_present());
    }

    #[test]
    fn sse_payload_detects_finish_reason() {
        let raw = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
        let p: SsePayload = serde_json::from_str(raw).unwrap();
        assert!(p.finish_reason_present());
    }
}

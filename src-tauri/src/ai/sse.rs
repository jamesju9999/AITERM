//! Shared SSE (Server-Sent Events) streaming utilities.
//!
//! Used by `OpenAiClient` and `OpenAiCompatibleClient`.

use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{AiError, GenerateChunk, TokenUsage};

// ── Public entry point ───────────────────────────────────────────────────────

/// Consume an OpenAI-format SSE response stream and forward `GenerateChunk`s.
pub async fn consume_openai_sse(
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

            let payload = match line.strip_prefix("data:") {
                Some(p) => p.trim(),
                None => continue,
            };
            if payload == "[DONE]" {
                saw_done = true;
                break 'outer;
            }
            match serde_json::from_str::<OpenAiSsePayload>(payload) {
                Ok(p) => {
                    let delta = p.delta_text();
                    let usage = p.usage_into();
                    let finish = p.finish_reason_present();
                    let _ = tx.send(GenerateChunk { delta, done: false, usage: usage.clone() }).await;
                    if finish {
                        let _ = tx.send(GenerateChunk { delta: String::new(), done: true, usage }).await;
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

// ── OpenAI SSE payload types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct OpenAiSsePayload {
    #[serde(default)]
    choices: Vec<OpenAiSseChoice>,
    #[serde(default)]
    usage: Option<OpenAiSseUsage>,
}

#[derive(Deserialize)]
struct OpenAiSseChoice {
    #[serde(default)]
    delta: OpenAiSseDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiSseDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct OpenAiSseUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

impl OpenAiSsePayload {
    pub fn delta_text(&self) -> String {
        self.choices.first().and_then(|c| c.delta.content.clone()).unwrap_or_default()
    }
    pub fn finish_reason_present(&self) -> bool {
        self.choices.first().and_then(|c| c.finish_reason.as_ref()).is_some()
    }
    pub fn usage_into(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|u| TokenUsage {
            prompt: u.prompt_tokens,
            completion: u.completion_tokens,
        })
    }
}

// ── Byte helpers ──────────────────────────────────────────────────────────────

pub fn find_line_end(buf: &[u8]) -> Option<usize> {
    for (i, w) in buf.windows(2).enumerate() {
        if w == b"\r\n" { return Some(i); }
    }
    buf.iter().position(|&b| b == b'\n' || b == b'\r')
}

pub fn separator_len(buf: &[u8]) -> usize {
    match buf.first() {
        Some(&b'\r') if buf.get(1) == Some(&b'\n') => 2,
        Some(&b'\r') | Some(&b'\n') => 1,
        _ => 0,
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Map a non-2xx HTTP response to an `AiError`.
pub async fn map_http_error(status: reqwest::StatusCode, resp: reqwest::Response) -> AiError {
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
    AiError::Network { message: format!("http {}: {}", status.as_u16(), truncate(&body, 200)) }
}

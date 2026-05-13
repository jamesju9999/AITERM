//! OpenAI provider — `https://api.openai.com/v1/chat/completions` (SSE).
//!
//! Uses `response_format: { type: "json_object" }` for single-command queries
//! and SSE streaming. The model is configurable (default: `gpt-4o-mini`).

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::ai::{
    sse::{consume_openai_sse, map_http_error},
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
};

pub struct OpenAiClient {
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) base_url: String,
    pub(crate) client: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, "gpt-4o-mini".into(), "https://api.openai.com".into())
    }

    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Self {
        Self { api_key, model, base_url, client: reqwest::Client::new() }
    }

    fn completions_url(&self) -> String {
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
        let json_mode = req.mode == crate::ai::QueryMode::SingleCommand;
        let body = build_request_body(&self.model, &req, json_mode);
        let resp = self
            .client
            .post(self.completions_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }
        consume_openai_sse(resp, tx).await
    }

    async fn health_check(&self) -> Result<(), AiError> {
        // Send a minimal 1-token request just to verify the key and endpoint.
        use crate::ai::{ChatMessage, EnvSnapshot, QueryMode};
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let (_tx, _rx) = mpsc::channel::<GenerateChunk>(1);
        let req = GenerateRequest {
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
        };
        let body = build_request_body(&self.model, &req, false);
        let resp = self
            .client
            .post(self.completions_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(map_http_error(status, resp).await)
        }
    }
}

// ── Request building ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
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

fn build_request_body<'a>(
    model: &'a str,
    req: &'a GenerateRequest,
    json_mode: bool,
) -> OpenAiChatRequest<'a> {
    let mut messages: Vec<OpenAiMessage<'a>> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(OpenAiMessage { role: "system", content: &req.system_prompt });
    for m in &req.messages {
        messages.push(OpenAiMessage { role: m.role.as_str(), content: m.content.as_str().unwrap_or("") });
    }
    OpenAiChatRequest {
        model,
        messages,
        stream: true,
        response_format: if json_mode { Some(ResponseFormat { ty: "json_object" }) } else { None },
        max_tokens: req.max_tokens,
    }
}

#[allow(dead_code)]
fn _unused_chatmessage_anchor(_: ChatMessage) {}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_request() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi") }],
            context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(256),
        }
    }

    #[test]
    fn request_body_sets_stream_and_response_format() {
        let req = sample_request();
        let body = build_request_body("gpt-4o-mini", &req, true);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "gpt-4o-mini");
        assert_eq!(json["stream"], true);
        assert_eq!(json["response_format"]["type"], "json_object");
        assert_eq!(json["messages"][0]["role"], "system");
        assert_eq!(json["messages"][1]["role"], "user");
        assert_eq!(json["max_tokens"], 256);
    }

    #[test]
    fn request_body_no_json_mode_omits_response_format() {
        let req = sample_request();
        let body = build_request_body("gpt-4o-mini", &req, false);
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("response_format").map_or(true, |v| v.is_null()));
    }
}

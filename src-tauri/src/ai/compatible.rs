//! OpenAI-compatible provider — arbitrary base URL + optional API key.
//!
//! Covers: LM Studio, vLLM, OpenRouter, DeepSeek, self-hosted services.
//! Uses the same OpenAI SSE format as `OpenAiClient` but with a configurable
//! base URL and optional authentication.

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::ai::{
    sse::{consume_openai_sse, map_http_error},
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
};

pub struct OpenAiCompatibleClient {
    /// Optional — some local servers (LM Studio, vLLM) don't require auth.
    api_key: Option<String>,
    model: String,
    base_url: String,
    /// Whether to send `response_format: { type: "json_object" }`.
    /// Most OpenAI-compatible servers support this, but some don't.
    supports_json_mode: bool,
    client: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub fn new(
        base_url: String,
        model: String,
        api_key: Option<String>,
        supports_json_mode: bool,
    ) -> Self {
        Self { api_key, model, base_url, supports_json_mode, client: reqwest::Client::new() }
    }

    fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl AiProvider for OpenAiCompatibleClient {
    fn id(&self) -> &str { "compatible" }
    fn display_name(&self) -> &str { "OpenAI-Compatible" }

    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        let body = build_request_body(&self.model, &req, self.supports_json_mode);
        let mut builder = self.client.post(self.completions_url()).json(&body);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }

        let resp = builder
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();

        // ── Auto-retry without response_format on 400 ────────────────────────
        // Some models (e.g. Gemma in LM Studio) reject `response_format: json_object`
        // with HTTP 400. Retry transparently without it.
        if status == reqwest::StatusCode::BAD_REQUEST && self.supports_json_mode {
            let body_text = resp.text().await.unwrap_or_default();
            if body_text.contains("response_format") {
                log::warn!("Provider rejected response_format, retrying without it: {body_text}");
                let body_no_json = build_request_body(&self.model, &req, false);
                let mut retry = self.client.post(self.completions_url()).json(&body_no_json);
                if let Some(key) = &self.api_key {
                    retry = retry.bearer_auth(key);
                }
                let retry_resp = retry.send().await
                    .map_err(|e| AiError::Network { message: e.to_string() })?;
                let retry_status = retry_resp.status();
                if !retry_status.is_success() {
                    return Err(map_http_error(retry_status, retry_resp).await);
                }
                return consume_openai_sse(retry_resp, tx).await;
            }
            return Err(AiError::Network { message: format!("http {status}: {body_text}") });
        }

        if !status.is_success() {
            return Err(map_http_error(status, resp).await);
        }
        consume_openai_sse(resp, tx).await
    }


    async fn health_check(&self) -> Result<(), AiError> {
        use crate::ai::{EnvSnapshot, QueryMode};
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let (_tx, _rx) = mpsc::channel::<GenerateChunk>(1);
        let req = GenerateRequest {
            system_prompt: "ping".into(),
            messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
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
        let mut builder = self.client.post(self.completions_url()).json(&body);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        let resp = builder
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

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct CompatibleChatRequest<'a> {
    model: &'a str,
    messages: Vec<CompatibleMessage<'a>>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct CompatibleMessage<'a> {
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
) -> CompatibleChatRequest<'a> {
    let mut messages: Vec<CompatibleMessage<'a>> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(CompatibleMessage { role: "system", content: &req.system_prompt });
    for m in &req.messages {
        messages.push(CompatibleMessage { role: m.role.as_str(), content: m.content.as_str() });
    }
    CompatibleChatRequest {
        model,
        messages,
        stream: true,
        response_format: if json_mode { Some(ResponseFormat { ty: "json_object" }) } else { None },
        max_tokens: req.max_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{EnvSnapshot, QueryMode};
    use std::path::PathBuf;

    fn sample_req() -> GenerateRequest {
        GenerateRequest {
            system_prompt: "sys".into(),
            messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
            context: EnvSnapshot { os: "linux".into(), shell: "bash".into(), cwd: PathBuf::from("/"), ..Default::default() },
            mode: QueryMode::SingleCommand,
            max_tokens: Some(128),
        }
    }

    #[test]
    fn with_json_mode_includes_response_format() {
        let req = sample_req();
        let body = build_request_body("qwen2", &req, true);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["response_format"]["type"], "json_object");
    }

    #[test]
    fn without_json_mode_omits_response_format() {
        let req = sample_req();
        let body = build_request_body("qwen2", &req, false);
        let json = serde_json::to_value(&body).unwrap();
        // response_format should be absent (serialized as null or missing)
        assert!(json.get("response_format").map_or(true, |v| v.is_null()));
    }

    #[test]
    fn no_api_key_omits_auth_header() {
        // Just verifies the client can be constructed without a key.
        let client = OpenAiCompatibleClient::new(
            "http://localhost:1234/v1".into(),
            "local-model".into(),
            None,
            true,
        );
        assert!(client.api_key.is_none());
    }
}

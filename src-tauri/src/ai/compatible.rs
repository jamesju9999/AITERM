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
    GenerateWithToolsResult, McpToolDefinition,
};

pub struct OpenAiCompatibleClient {
    /// Optional — some local servers (LM Studio, vLLM) don't require auth.
    api_key: Option<String>,
    model: String,
    base_url: String,
    /// Whether to send `response_format: { type: "json_object" }`.
    /// Most OpenAI-compatible servers support this, but some don't.
    supports_json_mode: bool,
    /// Extra headers added to every request (e.g. Copilot IDE headers).
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
}

impl OpenAiCompatibleClient {
    pub fn new(
        base_url: String,
        model: String,
        api_key: Option<String>,
        supports_json_mode: bool,
    ) -> Self {
        Self { api_key, model, base_url, supports_json_mode, extra_headers: vec![], client: reqwest::Client::new() }
    }

    /// Create a client with additional static headers on every request.
    pub fn with_extra_headers(
        base_url: String,
        model: String,
        api_key: Option<String>,
        supports_json_mode: bool,
        extra_headers: Vec<(String, String)>,
    ) -> Self {
        Self { api_key, model, base_url, supports_json_mode, extra_headers, client: reqwest::Client::new() }
    }

    fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn apply_headers(&self, mut builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        for (k, v) in &self.extra_headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        builder
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
        let json_mode = self.supports_json_mode && req.mode == crate::ai::QueryMode::SingleCommand;
        let body = build_request_body(&self.model, &req, json_mode);
        let builder = self.apply_headers(self.client.post(self.completions_url()).json(&body));

        let resp = builder
            .send()
            .await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();

        // ── Auto-retry without response_format on 400 ────────────────────────
        // Some models (e.g. Gemma in LM Studio) reject `response_format: json_object`
        // with HTTP 400. Retry transparently without it.
        if status == reqwest::StatusCode::BAD_REQUEST && json_mode {
            let body_text = resp.text().await.unwrap_or_default();
            if body_text.contains("response_format") {
                log::warn!("Provider rejected response_format, retrying without it: {body_text}");
                let body_no_json = build_request_body(&self.model, &req, false);
                let retry = self.apply_headers(self.client.post(self.completions_url()).json(&body_no_json));
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


    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let tool_defs: serde_json::Value = serde_json::Value::Array(
            tools.iter().map(|t| serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema
                }
            })).collect()
        );

        let mut messages: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
        // Only prepend system_prompt when non-empty; agent_chat callers inject system msg via messages[0]
        if !req.system_prompt.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": req.system_prompt}));
        }
        for m in &req.messages {
            let mut msg = serde_json::json!({"role": m.role, "content": m.content});
            if let Some(id) = &m.tool_call_id {
                msg["tool_call_id"] = serde_json::Value::String(id.clone());
            }
            if let Some(tool_calls) = &m.tool_calls {
                msg["tool_calls"] = tool_calls.clone();
                msg["content"] = serde_json::Value::Null;
            }
            messages.push(msg);
        }

        // 串流。這條路徑原本是 stream:false，所有帶工具的對話（終端機 MCP、
        // 程式庫協助的研究階段）都因此變成「整段一次跳出來」。
        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "tools": tool_defs,
            "tool_choice": "auto",
            "stream": true
        });

        let builder = self.apply_headers(self.client.post(self.completions_url()).json(&body));
        let resp = builder.send().await
            .map_err(|e| AiError::Network { message: e.to_string() })?;

        let status = resp.status();
        if status == 401 { return Err(AiError::AuthFailed); }
        if status == 429 { return Err(AiError::RateLimit { retry_after: None, body: None }); }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(AiError::Network { message: format!("HTTP {status}: {body_text}") });
        }

        let streamed = crate::ai::sse::consume_openai_sse_with_tools(resp, tx).await?;

        if !streamed.calls.is_empty() {
            return Ok(GenerateWithToolsResult::ToolCalls {
                calls: streamed.calls,
                raw: streamed.raw,
            });
        }
        Ok(GenerateWithToolsResult::Text(streamed.text))
    }

    async fn health_check(&self) -> Result<(), AiError> {
        use crate::ai::{EnvSnapshot, QueryMode};
        use std::path::PathBuf;
        use tokio::sync::mpsc;

        let (_tx, _rx) = mpsc::channel::<GenerateChunk>(1);
        let req = GenerateRequest {
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
        };
        let body = build_request_body(&self.model, &req, false);
        let builder = self.apply_headers(self.client.post(self.completions_url()).json(&body));
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
struct CompatibleChatRequest {
    model: String,
    messages: Vec<CompatibleMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct CompatibleMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    ty: &'static str,
}

fn build_request_body(
    model: &str,
    req: &GenerateRequest,
    json_mode: bool,
) -> CompatibleChatRequest {
    let mut messages: Vec<CompatibleMessage> = Vec::with_capacity(req.messages.len() + 1);
    messages.push(CompatibleMessage { role: "system".to_owned(), content: serde_json::Value::String(req.system_prompt.clone()) });
    for m in &req.messages {
        messages.push(CompatibleMessage { role: m.role.clone(), content: m.content.clone() });
    }
    CompatibleChatRequest {
        model: model.to_owned(),
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
            messages: vec![ChatMessage { role: "user".into(), content: serde_json::json!("hi"), tool_call_id: None, tool_calls: None }],
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

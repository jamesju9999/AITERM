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
            serde_json::json!({
                "type": "message",
                "role": m.role,
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
}

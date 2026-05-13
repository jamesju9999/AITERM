//! AI provider abstractions and implementations.
//!
//! Module layout:
//! - `mod.rs` (this file): trait + shared types + errors
//! - `openai.rs`: `OpenAiClient`
//! - `anthropic.rs`: `AnthropicClient`
//! - `ollama.rs`: `OllamaClient`
//! - `compatible.rs`: `OpenAiCompatibleClient`
//! - `sse.rs`: shared SSE streaming utilities
//! - `router.rs`: `AiRouter`
//! - `context.rs`: environment snapshot helper

pub mod anthropic;
pub mod compatible;
pub mod context;
pub mod copilot;
pub mod ollama;
pub mod openai;
pub mod router;
pub(crate) mod sse;

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiError {
    #[error("OPENAI_API_KEY environment variable is not set")]
    NotConfigured,

    #[error("network error: {message}")]
    Network { message: String },

    #[error("authentication failed (check your API key)")]
    AuthFailed,

    #[error("rate limit exceeded")]
    RateLimit { retry_after: Option<String> },

    #[error("AI returned invalid response: {reason}")]
    ModelError { reason: String, raw: String },

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },
}

/// Environment snapshot sent to the AI as context.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
pub struct EnvSnapshot {
    pub os: String,      // e.g. "windows", "macos", "linux"
    pub shell: String,   // e.g. "pwsh", "powershell", "cmd", "bash", "zsh"
    pub cwd: PathBuf,
    /// Recent terminal output (ANSI-stripped, last ~50 lines). None if unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_output: Option<String>,
    /// Top-level directory listing of cwd. None if unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir_listing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    SingleCommand,
    /// Multi-turn chat used by the M4 AI panel (`ai_chat` command).
    Chat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,    // "user" | "assistant" | "system"
    /// Either a plain string (legacy) or an OpenAI-format content array.
    pub content: serde_json::Value,
}

#[derive(Debug)]
pub struct GenerateRequest {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub context: EnvSnapshot,
    pub mode: QueryMode,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct GenerateChunk {
    pub delta: String,
    pub done: bool,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
}

/// The structured payload the AI is required to return for `/ai` queries.
/// All fields have defaults so that models that omit optional fields don't
/// cause a hard parse error — we apply a conservative fallback instead.
#[derive(Debug, Clone, Deserialize)]
pub struct AiSingleCommand {
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub command: String,
    /// Defaults to NeedsConfirm when missing — conservative fallback.
    #[serde(default = "default_risk_level")]
    pub risk_level: RiskLevel,
}

fn default_risk_level() -> RiskLevel { RiskLevel::NeedsConfirm }

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Safe,
    NeedsConfirm,
    Dangerous,
    Blocked,
}

use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;

    /// Produce a response, streaming chunks through `tx`. Implementations must
    /// push a final chunk with `done: true` before returning `Ok(())`.
    async fn generate(
        &self,
        req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError>;

    /// Validate connectivity and credentials without generating a full response.
    /// Used by the Settings UI "Test Connection" button.
    async fn health_check(&self) -> Result<(), AiError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_error_serializes_kind_tag() {
        let err = AiError::NotConfigured;
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, r#"{"kind":"not_configured"}"#);
    }

    #[test]
    fn ai_error_network_carries_message() {
        let err = AiError::Network { message: "connection refused".into() };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "network");
        assert_eq!(json["message"], "connection refused");
    }

    #[test]
    fn ai_error_rate_limit_optional_retry_after() {
        let none = AiError::RateLimit { retry_after: None };
        let some = AiError::RateLimit { retry_after: Some("20".into()) };
        let j_none = serde_json::to_value(&none).unwrap();
        let j_some = serde_json::to_value(&some).unwrap();
        assert_eq!(j_none["kind"], "rate_limit");
        assert!(j_none["retry_after"].is_null());
        assert_eq!(j_some["retry_after"], "20");
    }

    #[test]
    fn ai_error_model_error_carries_reason_and_raw() {
        let err = AiError::ModelError {
            reason: "missing field `command`".into(),
            raw: "{\"explanation\":\"...\"}".into(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "model_error");
        assert_eq!(json["reason"], "missing field `command`");
        assert_eq!(json["raw"], "{\"explanation\":\"...\"}");
    }

    #[test]
    fn ai_error_invalid_input_carries_reason() {
        let err = AiError::InvalidInput { reason: "empty messages".into() };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "invalid_input");
        assert_eq!(json["reason"], "empty messages");
    }

    #[test]
    fn env_snapshot_serializes_expected_fields() {
        let snap = EnvSnapshot {
            os: "windows".into(),
            shell: "pwsh".into(),
            cwd: PathBuf::from("C:\\Users\\test"),
            ..Default::default()
        };
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["os"], "windows");
        assert_eq!(json["shell"], "pwsh");
        assert!(json["cwd"].as_str().unwrap().contains("test"));
    }

    #[test]
    fn ai_single_command_parses_valid_json() {
        let json = r#"{
            "explanation": "列出檔案",
            "command": "Get-ChildItem",
            "risk_level": "safe"
        }"#;
        let parsed: AiSingleCommand = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.explanation, "列出檔案");
        assert_eq!(parsed.command, "Get-ChildItem");
        assert_eq!(parsed.risk_level, RiskLevel::Safe);
    }

    #[test]
    fn ai_single_command_all_risk_levels() {
        for (raw, expected) in [
            ("safe", RiskLevel::Safe),
            ("needs_confirm", RiskLevel::NeedsConfirm),
            ("dangerous", RiskLevel::Dangerous),
            ("blocked", RiskLevel::Blocked),
        ] {
            let json = format!(r#"{{"explanation":"x","command":"y","risk_level":"{raw}"}}"#);
            let parsed: AiSingleCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed.risk_level, expected, "for raw={raw}");
        }
    }

    #[test]
    fn ai_single_command_missing_command_fails() {
        let json = r#"{"explanation":"x","risk_level":"safe"}"#;
        let err = serde_json::from_str::<AiSingleCommand>(json).unwrap_err();
        assert!(err.to_string().contains("command"), "unexpected err: {err}");
    }

    #[test]
    fn ai_single_command_invalid_risk_level_fails() {
        let json = r#"{"explanation":"x","command":"y","risk_level":"bogus"}"#;
        let err = serde_json::from_str::<AiSingleCommand>(json).unwrap_err();
        assert!(err.to_string().contains("risk_level") || err.to_string().contains("bogus"));
    }

    #[test]
    fn ai_single_command_markdown_fence_fails() {
        // Strict parser: markdown fences are not valid JSON. Test ensures we DO
        // NOT silently strip them — spec §6.3 says violations become ModelError.
        let json = "```json\n{\"explanation\":\"x\",\"command\":\"y\",\"risk_level\":\"safe\"}\n```";
        assert!(serde_json::from_str::<AiSingleCommand>(json).is_err());
    }

    #[test]
    fn chat_message_content_accepts_array() {
        let msg = ChatMessage {
            role: "user".into(),
            content: serde_json::json!([
                {"type": "text", "text": "hello"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
            ]),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json["content"].is_array());
        assert_eq!(json["content"][0]["type"], "text");
    }
}

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
    RateLimit { retry_after: Option<String>, body: Option<String> },

    #[error("AI returned invalid response: {reason}")]
    ModelError { reason: String, raw: String },

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

    #[error("Provider does not support tool calling")]
    ToolCallingUnsupported,
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

/// UI locale, mirrors the frontend's `Locale` type (`"en" | "zh-TW"`).
/// Threaded explicitly through AI commands so prompt builders know what
/// language to instruct the model to respond in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[serde(rename = "zh-TW")]
    ZhTw,
}

/// Human-readable language name for the "respond in X" prompt rule.
pub fn language_name(locale: Locale) -> &'static str {
    match locale {
        Locale::En => "English",
        Locale::ZhTw => "Traditional Chinese (繁體中文)",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,    // "user" | "assistant" | "system" | "tool"
    /// Plain string, content array, or null (when tool_calls is present).
    pub content: serde_json::Value,
    /// Required when role == "tool" — matches the tool_calls[].id from the assistant turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Present on assistant messages that invoke tools (role == "assistant").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
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

/// Definition of an MCP tool sent to the AI provider.
#[derive(Debug, Clone)]
pub struct McpToolDefinition {
    /// Encoded tool name: "server_id_sanitized__tool_name"
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A tool call requested by the AI.
#[derive(Debug, Clone, Serialize)]
pub struct AiToolCall {
    /// Provider's opaque tool call ID (needed when sending tool results back).
    pub id: String,
    /// Encoded tool name (contains server_id + tool_name).
    pub tool_name: String,
    pub args: serde_json::Value,
    /// Gemini thinking-mode opaque blob — must be echoed verbatim in conversation history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

/// Result of `generate_with_tools`.
pub enum GenerateWithToolsResult {
    /// AI returned tool calls. `raw` is the verbatim tool_calls JSON from the provider
    /// (needed to echo thought_signature back to Gemini thinking-mode models).
    ToolCalls { calls: Vec<AiToolCall>, raw: Option<serde_json::Value> },
    /// AI returned text (full response text).
    Text(String),
    /// This provider does not support tool calling.
    Unsupported,
}

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

    /// Generate with tool definitions. Providers that support tool calling
    /// override this. Default impl returns `Unsupported`.
    async fn generate_with_tools(
        &self,
        req: GenerateRequest,
        tools: Vec<McpToolDefinition>,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<GenerateWithToolsResult, AiError> {
        let _ = (req, tools, tx);
        Ok(GenerateWithToolsResult::Unsupported)
    }
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
        let none = AiError::RateLimit { retry_after: None, body: None };
        let some = AiError::RateLimit { retry_after: Some("20".into()), body: Some("too many".into()) };
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
            tool_call_id: None,
            tool_calls: None,
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert!(json["content"].is_array());
        assert_eq!(json["content"][0]["type"], "text");
        let roundtrip: ChatMessage = serde_json::from_value(json.clone()).unwrap();
        assert!(roundtrip.content.is_array());
    }

    #[test]
    fn locale_deserializes_from_frontend_strings() {
        let en: Locale = serde_json::from_str("\"en\"").unwrap();
        let zh: Locale = serde_json::from_str("\"zh-TW\"").unwrap();
        assert_eq!(en, Locale::En);
        assert_eq!(zh, Locale::ZhTw);
    }

    #[test]
    fn language_name_maps_locale_to_readable_name() {
        assert_eq!(language_name(Locale::En), "English");
        assert_eq!(language_name(Locale::ZhTw), "Traditional Chinese (繁體中文)");
    }
}

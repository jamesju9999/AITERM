//! Gemini provider via Google's internal Antigravity / Cloud Code Assist API
//! (`cloudcode-pa.googleapis.com`) — NOT the public Generative Language API,
//! and NOT Vertex AI's documented REST API. This is what a user's Google
//! account OAuth (scoped for Antigravity/Gemini Code Assist) actually talks
//! to; it requires a "project" id obtained via a separate onboarding step
//! (see `commands/provider.rs`) on every request.
//!
//! Key differences from every other client in this module:
//! - Endpoint is `cloudcode-pa.googleapis.com/v1internal:streamGenerateContent`.
//! - Request/response shape is Gemini's own native format (`contents: [{role,
//!   parts:[{text}]}]`, not Chat Completions `messages` and not the Responses
//!   API's `input` items.
//! - Every request must carry the account's onboarded Cloud Code project id
//!   in a top-level `project` field.
//! - The `contents` array only supports alternating "user"/"model" turns —
//!   there is no "system"/"developer" role slot. A caller that injects a
//!   `{role:"system",...}` message directly into history (e.g. AiPanel's
//!   Agent Mode loop, `src/components/AiPanel/index.tsx`'s `runAgentLoop`)
//!   would hit the same class of bug already root-caused and fixed for
//!   Codex (see `codex.rs`'s `build_request_body` — that endpoint accepts a
//!   remapped "developer" role; this one has no equivalent slot at all), so
//!   any system-role message here is folded into `systemInstruction` instead
//!   of being emitted as a `contents` turn.
//! - The client must present as the real Antigravity IDE client (User-Agent
//!   fixed to `darwin/arm64` regardless of host OS) — this is the closest
//!   analogue to Anthropic's "Claude Code sentinel" or Codex's
//!   `originator: codex_cli_rs` header for this provider.

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{
    sse::{find_line_end, map_http_error, separator_len},
    AiError, AiProvider, GenerateChunk, GenerateRequest, TokenUsage,
};

/// Hardcoded fallback client version. Dynamic fetching from Antigravity's
/// auto-updater feed is a documented fast-follow (see plan Task 13) — the
/// real response shape of that endpoint hasn't been verified yet.
pub(crate) const ANTIGRAVITY_IDE_VERSION: &str = "2.1.1";

pub struct AntigravityClient {
    access_token: String,
    project_id: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

/// Build the Antigravity `streamGenerateContent` request envelope.
pub(crate) fn build_request_body(model: &str, project_id: &str, req: &GenerateRequest) -> serde_json::Value {
    // Any {role:"system",...} message injected directly into history (see
    // this file's module doc) is folded into systemInstruction rather than
    // emitted as a contents turn — Gemini's contents array only supports
    // alternating "user"/"model" roles.
    let mut system_parts: Vec<String> = vec![req.system_prompt.clone()];

    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .filter_map(|m| {
            let text = match &m.content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if m.role == "system" {
                system_parts.push(text);
                return None;
            }
            let role = if m.role == "assistant" { "model" } else { "user" };
            Some(serde_json::json!({ "role": role, "parts": [{ "text": text }] }))
        })
        .collect();

    let system_instruction_text = system_parts
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let request_id = format!("agent/{now_ms}/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);

    serde_json::json!({
        "project": project_id,
        "requestId": request_id,
        "userAgent": "antigravity",
        "requestType": "agent",
        "model": model,
        "request": {
            "contents": contents,
            "systemInstruction": { "parts": [{ "text": system_instruction_text }] },
            "generationConfig": { "topK": 40, "topP": 1.0, "maxOutputTokens": 16384 },
        },
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

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage { role: role.into(), content: serde_json::json!(text), tool_call_id: None, tool_calls: None }
    }

    #[test]
    fn project_id_is_set_at_top_level() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        assert_eq!(body["project"], "proj-123");
    }

    #[test]
    fn envelope_has_fixed_metadata_fields() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["requestType"], "agent");
        assert_eq!(body["model"], "gemini-2.5-pro");
        assert!(body["requestId"].as_str().unwrap().starts_with("agent/"));
    }

    #[test]
    fn user_and_assistant_messages_map_to_user_and_model_roles() {
        let r = req("sys", vec![msg("user", "hi"), msg("assistant", "hello")]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let contents = body["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "hi");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "hello");
    }

    /// Regression-by-construction: mirrors the Codex "system messages are not
    /// allowed" bug (AiPanel's Agent Mode injects {role:"system",...} directly
    /// into history). Gemini's `contents` has no role slot for it at all, so
    /// it must be folded into systemInstruction, never emitted as a turn.
    #[test]
    fn system_role_message_is_folded_into_system_instruction_not_contents() {
        let r = req("base system prompt", vec![
            msg("system", "agent orchestration prompt"),
            msg("user", "go"),
        ]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let contents = body["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1, "the system-role message must not appear as a contents turn");
        assert_eq!(contents[0]["role"], "user");
        for c in contents {
            assert_ne!(c["role"], "system");
        }
        let system_instruction = body["request"]["systemInstruction"]["parts"][0]["text"].as_str().unwrap();
        assert!(system_instruction.contains("base system prompt"));
        assert!(system_instruction.contains("agent orchestration prompt"));
    }

    #[test]
    fn generation_config_forces_expected_values() {
        let r = req("sys", vec![]);
        let body = build_request_body("gemini-2.5-pro", "proj-123", &r);
        let gc = &body["request"]["generationConfig"];
        assert_eq!(gc["topK"], 40);
        assert_eq!(gc["topP"], 1.0);
        assert_eq!(gc["maxOutputTokens"], 16384);
    }
}

// src-tauri/src/mail/classify.rs
use std::sync::Arc;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, QueryMode};
use crate::commands::ai::extract_json_from_response;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MailClassification {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub is_important: bool,
    #[serde(default)]
    pub is_promotional: bool,
}

/// System prompt for the mail triage classifier. Pure function of no
/// arguments — every call produces byte-identical output, which is what
/// `prompt_is_deterministic_and_forbids_dual_classification` checks.
pub fn build_mail_classify_prompt() -> String {
    r#"You are an email triage assistant. Given an email's sender, subject, and body,
output ONLY a JSON object, no prose, no markdown fences, no extra keys.

Schema:
{
  "summary": "one or two sentence summary of the email, in Traditional Chinese (繁體中文)",
  "is_important": true or false,
  "is_promotional": true or false
}

Rules:
1. is_important is true only if the email requires the user's timely attention or action.
2. is_promotional is true if this is a marketing/advertising/newsletter email.
3. An email is never both is_important and is_promotional — promotional email is never important.
4. Automated notifications that need no action (e.g. "your package shipped") are not important."#.to_string()
}

/// Classify one email: summarize it and flag importance/promotional status
/// in a single AI call. Truncates the body to ~4000 chars before sending —
/// enough for triage, without spending tokens on a full attachment-length body.
pub async fn classify_message(
    provider: Arc<dyn AiProvider>,
    sender: &str,
    subject: &str,
    body_text: &str,
) -> Result<MailClassification, AiError> {
    let truncated_body: String = body_text.chars().take(4000).collect();
    let user_content = format!("From: {sender}\nSubject: {subject}\n\n{truncated_body}");

    let req = GenerateRequest {
        system_prompt: build_mail_classify_prompt(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::Value::String(user_content),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: Default::default(),
        mode: QueryMode::SingleCommand,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    let cleaned = extract_json_from_response(&buf);
    serde_json::from_str(&cleaned).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(300).collect(),
    })
}

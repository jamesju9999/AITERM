//! Activity Reporter: filters sensitive data and batches reports to Management Server.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Patterns that indicate a value should be redacted.
/// Applied to command strings before reporting.
static REDACT_PATTERNS: &[&str] = &[
    "password=",
    "--password",
    "-p ",
    "PGPASSWORD=",
    "Bearer ",
    "api_key=",
    "api-key=",
    "token=",
    "secret=",
    "Authorization:",
];

/// Redact sensitive values from a command string.
/// Each matching pattern causes the rest of the token to be replaced with [REDACTED].
pub fn redact_command(cmd: &str) -> String {
    let mut result = cmd.to_string();
    for pattern in REDACT_PATTERNS {
        if let Some(idx) = result.to_lowercase().find(&pattern.to_lowercase()) {
            let after = idx + pattern.len();
            // Find end of the token (whitespace or end of string).
            let end = result[after..]
                .find(|c: char| c.is_whitespace() || c == '\'' || c == '"')
                .map(|i| after + i)
                .unwrap_or(result.len());
            result.replace_range(after..end, "[REDACTED]");
        }
    }
    result
}

#[derive(Debug, Serialize)]
pub struct CommandEntry {
    pub command: String,
    pub exit_code: Option<i32>,
    pub executed_at: DateTime<Utc>,
    pub task_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiConversationEntry {
    pub user_prompt: String,
    pub ai_response: String,
    pub provider_id: String,
    pub tokens_used: Option<i32>,
    pub task_id: Option<String>,
}

pub struct ActivityReporter {
    pub commands: Vec<CommandEntry>,
    pub conversations: Vec<AiConversationEntry>,
}

impl ActivityReporter {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            conversations: Vec::new(),
        }
    }

    pub fn record_command(
        &mut self,
        raw_command: &str,
        exit_code: Option<i32>,
        task_id: Option<String>,
    ) {
        self.commands.push(CommandEntry {
            command: redact_command(raw_command),
            exit_code,
            executed_at: Utc::now(),
            task_id,
        });
    }

    pub fn record_ai_conversation(
        &mut self,
        user_prompt: &str,
        ai_response: &str,
        provider_id: &str,
        tokens_used: Option<i32>,
        task_id: Option<String>,
    ) {
        self.conversations.push(AiConversationEntry {
            user_prompt: redact_command(user_prompt),
            ai_response: ai_response.to_string(),
            provider_id: provider_id.to_string(),
            tokens_used,
            task_id,
        });
    }

    /// Drain and return batches for sending to Management Server.
    pub fn drain_commands(&mut self) -> Vec<CommandEntry> {
        std::mem::take(&mut self.commands)
    }

    pub fn drain_conversations(&mut self) -> Vec<AiConversationEntry> {
        std::mem::take(&mut self.conversations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_password_flag() {
        let cmd = "mysql -u root --password=secret123 mydb";
        let redacted = redact_command(cmd);
        assert!(redacted.contains("[REDACTED]"), "got: {}", redacted);
        assert!(!redacted.contains("secret123"), "got: {}", redacted);
    }

    #[test]
    fn redacts_bearer_token() {
        let cmd = "curl -H 'Authorization: Bearer sk-abc123' https://api.example.com";
        let redacted = redact_command(cmd);
        assert!(redacted.contains("[REDACTED]"), "got: {}", redacted);
        assert!(!redacted.contains("sk-abc123"), "got: {}", redacted);
    }

    #[test]
    fn preserves_normal_commands() {
        let cmd = "git status";
        assert_eq!(redact_command(cmd), cmd);
    }
}

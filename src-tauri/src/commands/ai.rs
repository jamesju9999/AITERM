//! `/ai` query command. Wires the frontend invoke to the ai_router and the
//! PTY manager for context, and returns a fully-parsed `AiCommandReady`.

use serde::Serialize;
use tauri::State;
use tokio::sync::mpsc;

use crate::ai::{
    context, router::AiRouter, AiError, AiSingleCommand, ChatMessage, GenerateChunk,
    GenerateRequest, QueryMode,
};
use crate::pty::PtyManager;

#[derive(Debug, Clone, Serialize)]
pub struct AiCommandReady {
    pub command: String,
    pub explanation: String,
}

/// Build the M1 single-command system prompt. Pure function for testability.
pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot) -> String {
    format!(
r#"You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request into ONE
executable shell command for their current environment.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths)

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {{
     "explanation": "一句話說明這個命令做什麼 (use Traditional Chinese)",
     "command":     "a single shell command, no prompt prefix, no line breaks",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }}
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command and explain the limitation in `explanation`.
5. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous"."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display()
    )
}

#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.require_provider()?;

    let prompt = build_single_command_prompt(&snapshot);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages: vec![ChatMessage { role: "user".into(), content: query }],
        context: snapshot,
        mode: QueryMode::SingleCommand,
        max_tokens: Some(512),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    // Propagate any provider error after draining the channel.
    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    let parsed: AiSingleCommand = serde_json::from_str(&buf).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(200).collect(),
    })?;

    // M1 ignores risk_level (spec §6.4). It was parsed to fail-fast on
    // malformed responses, and the value is discarded here.
    let _ = parsed.risk_level;

    Ok(AiCommandReady {
        command: parsed.command,
        explanation: parsed.explanation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::EnvSnapshot;
    use std::path::PathBuf;

    #[test]
    fn prompt_contains_environment_fields() {
        let snap = EnvSnapshot {
            os: "windows".into(),
            shell: "pwsh".into(),
            cwd: PathBuf::from("C:\\Users\\a"),
        };
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("OS: windows"));
        assert!(prompt.contains("Shell: pwsh"));
        assert!(prompt.contains("C:\\Users\\a"));
        assert!(prompt.contains("JSON object"));
        assert!(prompt.contains("risk_level"));
    }
}

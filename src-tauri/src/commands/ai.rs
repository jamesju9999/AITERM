//! `/ai` query command. Wires the frontend invoke to the ai_router and the
//! PTY manager for context, and returns a fully-parsed `AiCommandReady`.
//!
//! Streaming: while the provider generates tokens, each chunk is emitted as a
//! `ai-stream` Tauri event so the frontend can show a live indicator. The
//! final structured result is still returned as the invoke response.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use crate::ai::{
    context, router::AiRouter, AiError, AiSingleCommand, ChatMessage, GenerateChunk,
    GenerateRequest, QueryMode, RiskLevel,
};
use crate::guard::CommandGuard;
use crate::pty::PtyManager;

#[derive(Debug, Clone, Serialize)]
pub struct AiCommandReady {
    pub command: String,
    pub explanation: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiStreamKind {
    Query, // /ai single-command flow
    Chat,  // AI Panel multi-turn flow
}

/// Payload emitted as a Tauri event for each streaming chunk.
#[derive(Debug, Clone, Serialize)]
pub struct AiStreamEvent {
    pub session_id: String,
    pub kind: AiStreamKind,
    pub delta: String,
    pub done: bool,
}

/// Extract the JSON object from raw AI output.
///
/// Some models (Qwen, DeepSeek, etc.) may:
/// - Wrap output in markdown fences: ```json { ... } ```
/// - Emit a thinking block first: <think>...</think>{ ... }
/// - Prefix with prose before the JSON object
///
/// This function strips all of these and returns a string starting with `{`.
fn extract_json_from_response(raw: &str) -> String {
    // 1. Strip <think>...</think> blocks (DeepSeek, Qwen thinking mode)
    let without_think = {
        let mut s = raw.to_string();
        while let (Some(start), Some(end)) = (s.find("<think>"), s.find("</think>")) {
            if start < end {
                s.drain(start..end + "</think>".len());
            } else {
                break;
            }
        }
        s
    };

    // 2. Strip markdown code fences: ```json ... ``` or ``` ... ```
    let without_fences = {
        let trimmed = without_think.trim();
        if trimmed.starts_with("```") {
            // Find the first newline after the opening fence
            if let Some(newline_pos) = trimmed.find('\n') {
                let inner = &trimmed[newline_pos + 1..];
                // Strip the closing fence
                if let Some(close_pos) = inner.rfind("```") {
                    inner[..close_pos].trim().to_string()
                } else {
                    inner.trim().to_string()
                }
            } else {
                trimmed.to_string()
            }
        } else {
            trimmed.to_string()
        }
    };

    // 3. Find the first `{` — skip any preamble text before the JSON object
    if let Some(json_start) = without_fences.find('{') {
        // Find the matching last `}` to get the full JSON object
        let json_candidate = &without_fences[json_start..];
        if let Some(json_end) = json_candidate.rfind('}') {
            return json_candidate[..=json_end].to_string();
        }
        return json_candidate.to_string();
    }

    // Fallback: return as-is (will fail JSON parsing with a meaningful error)
    without_fences
}

/// Build the system prompt. Includes OS/shell/cwd and, if available, recent
/// terminal output and a directory listing for richer context.
pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 { &o[o.len() - 2000..] } else { o };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    format!(
r#"You are an AI command generator for a cross-platform terminal application.
Your only job is to translate the user's natural-language request (or execution goal) into ONE
executable shell command for their current environment.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}            (may be slightly stale; prefer relative paths or
                         shell variables over hardcoded absolute paths){recent_section}{dir_section}

Rules:
1. Output ONLY a JSON object, no prose, no markdown fences, no extra keys.
2. Schema:
   {{
     "explanation": "一句話說明這個命令做什麼，或是總結已完成的結果 (use Traditional Chinese)",
     "command":     "a single shell command, no prompt prefix, no line breaks. SET TO 'DONE' IF GOAL IS FULLY MET.",
     "risk_level":  one of "safe", "needs_confirm", "dangerous"
   }}
3. The command must be syntactically valid for {shell}. Do not mix shells.
4. If the request cannot be satisfied with one command, pick the most useful
   single command to progress further.
5. If the user provides an execution history and it shows their ultimate goal is achieved, you MUST set "command" to "DONE".
6. Never produce destructive operations against system roots. If the user
   explicitly asks for one, set risk_level="dangerous"."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
    )
}

/// Build the system prompt for Chat mode. Unlike `build_single_command_prompt`,
/// this does NOT instruct JSON output — instead it explains the `<cmd>` tag
/// protocol and invites free-form Traditional Chinese prose.
pub fn build_chat_prompt(snapshot: &crate::ai::EnvSnapshot) -> String {
    let recent_section = snapshot.recent_output.as_deref().map(|o| {
        let trimmed = if o.len() > 2000 {
            let start = o.len() - 2000;
            let start = (start..=o.len())
                .find(|&i| o.is_char_boundary(i))
                .unwrap_or(o.len());
            &o[start..]
        } else {
            o
        };
        format!("\nRecent terminal output (last ~50 lines):\n```\n{trimmed}\n```")
    }).unwrap_or_default();

    let dir_section = snapshot.dir_listing.as_deref().map(|d| {
        format!("\nDirectory listing ({}):\n```\n{d}\n```", snapshot.cwd.display())
    }).unwrap_or_default();

    format!(
r#"You are an AI terminal assistant. The user is in an interactive terminal
session and you can see their OS, shell, cwd, and recent output.

Environment:
  OS: {os}
  Shell: {shell}
  Cwd: {cwd}{recent_section}{dir_section}

Rules:
1. Respond in Traditional Chinese (繁體中文).
2. When you want to suggest a runnable shell command, wrap it in
   <cmd>...</cmd> tags. The user can click the tag to execute it.
3. You may include multiple <cmd> tags in one reply if needed.
4. Each <cmd> must contain a command valid for {shell}. Prefer single-line
   commands; multi-line commands will ask the user for confirmation before
   executing.
5. Free-form explanation outside <cmd> tags is encouraged.
6. Never produce destructive operations against system roots unless the
   user explicitly asks; if you do, mark it clearly in prose."#,
        os = snapshot.os,
        shell = snapshot.shell,
        cwd = snapshot.cwd.display(),
    )
}

#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve()?;

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
        // Emit streaming event so the frontend can show live progress.
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Query,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    // Propagate any provider error after draining the channel.
    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    // ── Guard: empty response ───────────────────────────────────────────────────
    // Some model servers return 200 OK with an empty body when they don't
    // support the requested endpoint (e.g. POST /chat/completions on Ollama).
    if buf.trim().is_empty() {
        return Err(AiError::ModelError {
            reason: "模型回傳空回應（HTTP 200 但 body 為空）。\
                     請確認 Provider 的 base_url 和 model 設定正確，\
                     以及目標模型是否支援 /chat/completions 端點。".into(),
            raw: String::new(),
        });
    }

    // ── Clean up AI output before parsing ──────────────────────────────────────
    // Some models wrap JSON in markdown fences (```json ... ```) or
    // include thinking output (<think>...</think>). Strip those first.
    let cleaned = extract_json_from_response(&buf);

    let parsed: AiSingleCommand = serde_json::from_str(&cleaned).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(300).collect(),
    })?;

    // M3: Verify AI's generated command with CommandGuard
    let (guard_level, guard_reason) = CommandGuard::classify(&parsed.command);
    
    // Always take the HIGHER risk level (the more conservative one)
    let final_risk_level = std::cmp::max(parsed.risk_level, guard_level);
    
    // Append the guard reason if it bumped the risk level
    let final_explanation = if guard_level > parsed.risk_level && guard_reason.is_some() {
        format!("{} (系統安全攔截: {})", parsed.explanation, guard_reason.unwrap())
    } else {
        parsed.explanation
    };

    Ok(AiCommandReady {
        command: parsed.command,
        explanation: final_explanation,
        risk_level: final_risk_level,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AiChatReply {
    pub content: String,
}

#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
    // Reject empty history or histories whose last message isn't from the user.
    // This is a cheap sanity check — the real contract is enforced at the UI.
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    if messages.last().map(|m| m.role.as_str()) != Some("user") {
        return Err(AiError::InvalidInput { reason: "last message must be from user".into() });
    }

    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve()?;

    let prompt = build_chat_prompt(&snapshot);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply { content: buf })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::EnvSnapshot;
    use std::path::PathBuf;

    fn make_snap(os: &str, shell: &str, cwd: &str) -> EnvSnapshot {
        EnvSnapshot {
            os: os.into(),
            shell: shell.into(),
            cwd: PathBuf::from(cwd),
            ..Default::default()
        }
    }

    #[test]
    fn prompt_contains_environment_fields() {
        let snap = make_snap("windows", "pwsh", "C:\\Users\\a");
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("OS: windows"));
        assert!(prompt.contains("Shell: pwsh"));
        assert!(prompt.contains("C:\\Users\\a"));
        assert!(prompt.contains("JSON object"));
        assert!(prompt.contains("risk_level"));
    }

    #[test]
    fn prompt_includes_recent_output_when_present() {
        let snap = EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/tmp"),
            recent_output: Some("$ ls\nfoo  bar".into()),
            dir_listing: None,
        };
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("Recent terminal output"));
        assert!(prompt.contains("foo  bar"));
    }

    #[test]
    fn prompt_includes_dir_listing_when_present() {
        let snap = EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/home/u"),
            recent_output: None,
            dir_listing: Some("docs/\nsrc/\nCargo.toml".into()),
        };
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("Directory listing"));
        assert!(prompt.contains("Cargo.toml"));
    }

    #[test]
    fn prompt_omits_context_sections_when_none() {
        let snap = make_snap("macos", "zsh", "/home");
        let prompt = build_single_command_prompt(&snap);
        assert!(!prompt.contains("Recent terminal output"));
        assert!(!prompt.contains("Directory listing"));
    }

    #[test]
    fn chat_prompt_contains_environment_fields() {
        let snap = make_snap("windows", "pwsh", "C:\\Users\\a");
        let prompt = build_chat_prompt(&snap);
        assert!(prompt.contains("OS: windows"));
        assert!(prompt.contains("Shell: pwsh"));
        assert!(prompt.contains("C:\\Users\\a"));
    }

    #[test]
    fn chat_prompt_includes_recent_output_when_present() {
        let snap = EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/tmp"),
            recent_output: Some("$ ls\nfoo  bar".into()),
            dir_listing: None,
        };
        let prompt = build_chat_prompt(&snap);
        assert!(prompt.contains("Recent terminal output"));
        assert!(prompt.contains("foo  bar"));
    }

    #[test]
    fn chat_prompt_instructs_cmd_tag_format() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap);
        assert!(prompt.contains("<cmd>"), "prompt must mention <cmd> tag");
        assert!(prompt.contains("</cmd>"), "prompt must mention closing tag");
    }

    #[test]
    fn chat_prompt_omits_json_schema_rules() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap);
        // Chat mode must NOT contain the single-command JSON schema instruction.
        assert!(
            !prompt.contains("Output ONLY a JSON object"),
            "chat prompt must not inherit the JSON schema rule"
        );
        assert!(
            !prompt.contains("risk_level"),
            "chat prompt must not mention risk_level (that's single-command only)"
        );
    }

    #[test]
    fn chat_prompt_truncates_long_recent_output_without_utf8_panic() {
        // Build a string > 2000 bytes where the slice boundary lands inside
        // a multi-byte CJK codepoint. "中" is 3 bytes in UTF-8.
        let long = "中".repeat(800); // ~2400 bytes
        let snap = EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/tmp"),
            recent_output: Some(long),
            dir_listing: None,
        };
        // Must not panic.
        let prompt = build_chat_prompt(&snap);
        assert!(prompt.contains("Recent terminal output"));
        assert!(prompt.contains("中"));
    }

    #[test]
    fn prompt_truncates_long_recent_output() {
        let long_output = "z".repeat(3000); // use 'z' — not present in the prompt template
        let snap = EnvSnapshot {
            os: "linux".into(),
            shell: "bash".into(),
            cwd: PathBuf::from("/"),
            recent_output: Some(long_output.clone()),
            dir_listing: None,
        };
        let prompt = build_single_command_prompt(&snap);
        assert!(prompt.contains("Recent terminal output"));
        // The full 3000-char string must NOT be present — truncation happened.
        assert!(!prompt.contains(&long_output), "full 3000-char output should have been truncated");
        // The tail (last 2000 chars) IS present — we kept the most recent output.
        assert!(prompt.contains(&long_output[1000..]), "the tail 2000 chars must be in the prompt");
    }
}

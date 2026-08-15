//! `/ai` query command. Wires the frontend invoke to the ai_router and the
//! PTY manager for context, and returns a fully-parsed `AiCommandReady`.
//!
//! Streaming: while the provider generates tokens, each chunk is emitted as a
//! `ai-stream` Tauri event so the frontend can show a live indicator. The
//! final structured result is still returned as the invoke response.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;

use crate::ai::{
    context, router::AiRouter, AiError, AiSingleCommand, ChatMessage, GenerateChunk,
    GenerateRequest, Locale, McpToolDefinition, QueryMode, RiskLevel,
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
    /// 本次請求的總 token（prompt + completion）。只在 `done` 的事件上有值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
}

/// Extract the JSON object from raw AI output.
///
/// Some models (Qwen, DeepSeek, etc.) may:
/// - Wrap output in markdown fences: ```json { ... } ```
/// - Emit a thinking block first: <think>...</think>{ ... }
/// - Prefix with prose before the JSON object
///
/// This function strips all of these and returns a string starting with `{`.
pub(crate) fn extract_json_from_response(raw: &str) -> String {
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
pub fn build_single_command_prompt(snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String {
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
     "explanation": "one-sentence description of what this command does, or a summary of the completed result (use {language})",
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
        language = crate::ai::language_name(locale),
    )
}

/// Build the system prompt for Chat mode. Unlike `build_single_command_prompt`,
/// this does NOT instruct JSON output — instead it explains the `<cmd>` tag
/// protocol and invites free-form prose in the caller's locale.
pub fn build_chat_prompt(snapshot: &crate::ai::EnvSnapshot, locale: Locale) -> String {
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
1. Respond in {language}.
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
        language = crate::ai::language_name(locale),
    )
}

#[tauri::command]
pub async fn ai_query(
    query: String,
    session_id: String,
    locale: Locale,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiCommandReady, AiError> {
    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve().await?;
    let prompt = build_single_command_prompt(&snapshot, locale);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages: vec![ChatMessage { role: "user".into(), content: serde_json::Value::String(query), tool_call_id: None, tool_calls: None }],
        context: snapshot,
        mode: QueryMode::SingleCommand,
        max_tokens: None,
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
            tokens: chunk.usage.map(|u| u.prompt + u.completion),
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

    // "DONE" is the agent-loop sentinel meaning "task complete" — it's not
    // a shell command, so skip CommandGuard to avoid polluting the final
    // explanation with an irrelevant "(系統安全攔截: 無法辨識該指令…)" tag.
    if parsed.command.trim().eq_ignore_ascii_case("DONE") {
        return Ok(AiCommandReady {
            command: parsed.command,
            explanation: parsed.explanation,
            risk_level: parsed.risk_level,
        });
    }

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
    pub content: Option<String>,
    pub tool_calls: Vec<crate::ai::AiToolCall>,
    pub tool_calling_unsupported: bool,
    /// 降級的原因。`None` 代表沒有降級。
    ///
    /// 光靠 `tool_calling_unsupported` 這個布林，畫面只能講「此供應商無法使用
    /// 原生工具呼叫」——對 Claude 訂閱那種計費歸屬問題來說那句話是錯的，會讓
    /// 使用者以為 Claude 不會工具呼叫。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_fallback_reason: Option<crate::ai::ToolFallbackReason>,
    /// Raw tool_calls JSON from the provider response, verbatim.
    /// Gemini thinking-mode models require this to be echoed back in conversation history.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_tool_calls: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn ai_chat(
    messages: Vec<ChatMessage>,
    session_id: String,
    provider_id: Option<String>,
    use_mcp: bool,
    locale: Locale,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
    mcp_manager: State<'_, std::sync::Arc<tokio::sync::Mutex<crate::mcp::McpManager>>>,
    config: State<'_, std::sync::Arc<crate::config::ConfigStore>>,
) -> Result<AiChatReply, AiError> {
    // Reject empty history or histories whose last message is in an unexpected role.
    // When use_mcp=true the agent loop may send a history ending with a "tool"
    // result message (multi-turn tool calling), so we allow that too.
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    let last_role = messages.last().map(|m| m.role.as_str());
    let last_role_ok = last_role == Some("user")
        || (use_mcp && last_role == Some("tool"));
    if !last_role_ok {
        return Err(AiError::InvalidInput { reason: "last message must be from user".into() });
    }

    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    let prompt = build_chat_prompt(&snapshot, locale);
    let req = GenerateRequest {
        system_prompt: prompt,
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    // ── MCP tool calling path ─────────────────────────────────────────────────
    let cfg = config.get();
    if use_mcp && cfg.mcp_enabled {
        let tools: Vec<crate::ai::McpToolDefinition> = {
            let manager = mcp_manager.lock().await;
            manager.list_tool_infos().into_iter().map(|t| crate::ai::McpToolDefinition {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            }).collect()
        };

        if !tools.is_empty() {
            let tools_for_fallback = tools.clone();
            let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
            let provider_clone = provider.clone();
            let req_clone = req.clone();
            let join = tokio::spawn(async move {
                provider_clone.generate_with_tools(req_clone, tools, tx).await
            });

            while let Some(chunk) = rx.recv().await {
                let _ = app.emit("ai-stream", AiStreamEvent {
                    session_id: session_id.clone(),
                    kind: AiStreamKind::Chat,
                    delta: chunk.delta.clone(),
                    done: chunk.done,
                    tokens: chunk.usage.map(|u| u.prompt + u.completion),
                });
                if chunk.done { break; }
            }

            let joined = join.await;
            // 為什麼降級要能傳到畫面上——「模型做不到」跟「這張憑證的計費歸屬」
            // 對使用者是兩件完全不同的事，該講的話也完全不同。
            let fallback_reason = match &joined {
                Ok(Err(AiError::ToolCallingUnsupported { reason })) => *reason,
                _ => crate::ai::ToolFallbackReason::Unsupported,
            };
            return match joined {
                Ok(Ok(crate::ai::GenerateWithToolsResult::ToolCalls { calls, raw })) =>
                    Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: raw }),
                Ok(Ok(crate::ai::GenerateWithToolsResult::Text(content))) =>
                    Ok(AiChatReply { content: Some(content), tool_calls: vec![], tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None }),
                Ok(Ok(crate::ai::GenerateWithToolsResult::Unsupported)) |
                Ok(Err(AiError::ToolCallingUnsupported { .. })) => {
                    // System prompt fallback: inject tool descriptions and re-call generate()
                    let tool_injection = build_tool_prompt_injection(&tools_for_fallback);
                    let mut fallback_req = req.clone();
                    fallback_req.system_prompt =
                        format!("{}\n\n{}", fallback_req.system_prompt, tool_injection);

                    let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(16);
                    let provider2 = provider.clone();
                    let join2 = tokio::spawn(async move {
                        provider2.generate(fallback_req, tx2).await
                    });

                    // 這條 fallback 是叫模型把工具呼叫寫成 `<tool_call>{…}` 文字。
                    // 那是指令、不是講給使用者聽的話——只送標記之前的部分，否則
                    // 整條指令會印在對話裡（程式庫協助那邊已經被這個咬過一次）。
                    let mut buf2 = String::new();
                    let mut emitted = 0usize;
                    while let Some(chunk) = rx2.recv().await {
                        buf2.push_str(&chunk.delta);
                        let visible = crate::ai::tool_markup::next_visible_delta(&buf2, emitted);
                        if let Some((delta, next)) = visible {
                            emitted = next;
                            let _ = app.emit("ai-stream", AiStreamEvent {
                                session_id: session_id.clone(),
                                kind: AiStreamKind::Chat,
                                delta,
                                done: false,
                                tokens: None,
                            });
                        }
                        if chunk.done {
                            let _ = app.emit("ai-stream", AiStreamEvent {
                                session_id: session_id.clone(),
                                kind: AiStreamKind::Chat,
                                delta: String::new(),
                                done: true,
                                tokens: chunk.usage.map(|u| u.prompt + u.completion),
                            });
                            break;
                        }
                    }
                    let _ = join2.await;

                    // tool_calling_unsupported=true：讓前端知道這一輪是降級跑的，
                    // 不要靜默發生。
                    if let Some(calls) = parse_tool_calls_from_text(&buf2) {
                        Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: true, tool_fallback_reason: Some(fallback_reason), raw_tool_calls: None })
                    } else {
                        Ok(AiChatReply { content: Some(buf2), tool_calls: vec![], tool_calling_unsupported: true, tool_fallback_reason: Some(fallback_reason), raw_tool_calls: None })
                    }
                }
                Ok(Err(e)) => Err(e),
                Err(e) => Err(AiError::Network { message: e.to_string() }),
            };
        }
    }
    // ── End MCP path — fall through to normal streaming path ─────────────────

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
            tokens: chunk.usage.map(|u| u.prompt + u.completion),
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply { content: Some(buf), tool_calls: vec![], tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
}

/// Build the tool injection suffix for providers that don't support native tool calling.
pub(crate) fn build_tool_prompt_injection(tools: &[crate::ai::McpToolDefinition]) -> String {
    let tools_json: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.input_schema,
        }))
        .collect();
    format!(
        "You have access to the following tools. To call a tool, output ONLY a single JSON block \
using this exact format and nothing else before or after it:\n\
<tool_call>{{\"name\":\"<tool_name>\",\"arguments\":{{...}}}}</tool_call>\n\n\
Available tools:\n{}\n\n\
After receiving tool results, continue the conversation naturally in the user's language.",
        serde_json::to_string_pretty(&tools_json).unwrap_or_default()
    )
}

/// Parse `<tool_call>...</tool_call>` blocks from model text output.
/// Returns `None` if no valid tool calls found.
pub(crate) fn parse_tool_calls_from_text(text: &str) -> Option<Vec<crate::ai::AiToolCall>> {
    let mut calls = Vec::new();
    let mut pos = 0;
    let open = "<tool_call>";
    let close = "</tool_call>";
    while let Some(start_offset) = text[pos..].find(open) {
        let content_start = pos + start_offset + open.len();
        // Determine end of this block:
        //   1. Prefer explicit </tool_call> closing tag
        //   2. Fall back to the next <tool_call> opening tag (multiple calls, no closing tags)
        //   3. Fall back to end of string
        let (json_str, next_pos) = if let Some(end_offset) = text[content_start..].find(close) {
            (&text[content_start..content_start + end_offset], content_start + end_offset + close.len())
        } else if let Some(next_open) = text[content_start..].find(open) {
            (&text[content_start..content_start + next_open], content_start + next_open)
        } else {
            (&text[content_start..], text.len())
        };
        let json_str = json_str.trim();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            let name = val["name"].as_str().unwrap_or("").to_string();
            // arguments is optional — default to empty object if missing
            let args = if val["arguments"].is_null() || !val["arguments"].is_object() {
                serde_json::Value::Object(Default::default())
            } else {
                val["arguments"].clone()
            };
            if !name.is_empty() {
                calls.push(crate::ai::AiToolCall {
                    id: format!("call_sp_{}", calls.len()),
                    tool_name: name,
                    args,
                    thought_signature: None,
                });
            }
        }
        pos = next_pos;
        if pos >= text.len() { break; }
    }
    if calls.is_empty() { None } else { Some(calls) }
}

/// Tool definition passed from the frontend for agent_chat calls.
/// Mirrors McpToolDefinition but is Deserializable since it comes over IPC.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Like ai_chat but with an explicit required provider_id and frontend-supplied
/// tool definitions instead of MCP tools. Used by the Loop Studio.
#[tauri::command]
pub async fn agent_chat(
    provider_id: String,
    messages: Vec<ChatMessage>,
    tools: Vec<AgentToolDefinition>,
    session_id: String,
    app: AppHandle,
    pty_manager: State<'_, PtyManager>,
    router: State<'_, AiRouter>,
) -> Result<AiChatReply, AiError> {
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }

    let snapshot = context::snapshot(&pty_manager, &session_id);
    let provider = router.resolve_by_id(&provider_id).await?;

    let req = GenerateRequest {
        system_prompt: String::new(), // caller injects via system message
        messages,
        context: snapshot,
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    if !tools.is_empty() {
        let mcp_tools: Vec<McpToolDefinition> = tools.into_iter().map(|t| McpToolDefinition {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        }).collect();
        let tools_for_fallback = mcp_tools.clone();

        let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
        let provider_clone = provider.clone();
        let req_clone = req.clone();
        let join = tokio::spawn(async move {
            provider_clone.generate_with_tools(req_clone, mcp_tools, tx).await
        });

        while let Some(chunk) = rx.recv().await {
            let _ = app.emit("ai-stream", AiStreamEvent {
                session_id: session_id.clone(),
                kind: AiStreamKind::Chat,
                delta: chunk.delta.clone(),
                done: chunk.done,
                tokens: chunk.usage.map(|u| u.prompt + u.completion),
            });
            if chunk.done { break; }
        }

        return match join.await {
            Ok(Ok(crate::ai::GenerateWithToolsResult::ToolCalls { calls, raw })) =>
                Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: raw }),
            Ok(Ok(crate::ai::GenerateWithToolsResult::Text(content))) => {
                // Model sometimes outputs tool calls in <tool_call> text format even when
                // native function calling is available (Gemini occasionally does this).
                if let Some(calls) = parse_tool_calls_from_text(&content) {
                    Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
                } else {
                    Ok(AiChatReply { content: Some(content), tool_calls: vec![], tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
                }
            },
            Ok(Ok(crate::ai::GenerateWithToolsResult::Unsupported)) |
            Ok(Err(AiError::ToolCallingUnsupported { .. })) => {
                // Fallback: inject tool descriptions into system prompt
                let tool_injection = build_tool_prompt_injection(&tools_for_fallback);
                let mut fallback_req = req.clone();
                fallback_req.system_prompt =
                    format!("{}\n\n{}", fallback_req.system_prompt, tool_injection);

                let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(16);
                let provider2 = provider.clone();
                let join2 = tokio::spawn(async move { provider2.generate(fallback_req, tx2).await });

                let mut buf2 = String::new();
                while let Some(chunk) = rx2.recv().await {
                    let _ = app.emit("ai-stream", AiStreamEvent {
                        session_id: session_id.clone(),
                        kind: AiStreamKind::Chat,
                        delta: chunk.delta.clone(),
                        done: chunk.done,
                        tokens: chunk.usage.map(|u| u.prompt + u.completion),
                    });
                    buf2.push_str(&chunk.delta);
                    if chunk.done { break; }
                }
                let _ = join2.await;

                if let Some(calls) = parse_tool_calls_from_text(&buf2) {
                    Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
                } else {
                    Ok(AiChatReply { content: Some(buf2), tool_calls: vec![], tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
                }
            }
            Ok(Err(e)) => Err(e),
            Err(e) => Err(AiError::Network { message: e.to_string() }),
        };
    }

    // No tools — plain generation
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
            tokens: chunk.usage.map(|u| u.prompt + u.completion),
        });
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    Ok(AiChatReply { content: Some(buf), tool_calls: vec![], tool_calling_unsupported: false, tool_fallback_reason: None, raw_tool_calls: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiToolCall, EnvSnapshot};
    use std::path::PathBuf;

    #[test]
    fn parse_tool_calls_finds_single_call() {
        let text = r#"Some text before <tool_call>{"name":"brave__search","arguments":{"query":"WWDC 2026"}}</tool_call> after"#;
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "brave__search");
        assert_eq!(calls[0].args["query"], "WWDC 2026");
    }

    #[test]
    fn parse_tool_calls_returns_none_when_absent() {
        let text = "Just a plain answer, no tool calls here.";
        assert!(parse_tool_calls_from_text(text).is_none());
    }

    #[test]
    fn parse_tool_calls_finds_multiple_calls() {
        let text = r#"<tool_call>{"name":"tool_a","arguments":{"x":1}}</tool_call> and <tool_call>{"name":"tool_b","arguments":{}}</tool_call>"#;
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "tool_a");
        assert_eq!(calls[1].tool_name, "tool_b");
    }

    #[test]
    fn parse_tool_calls_skips_invalid_json_blocks() {
        let text = r#"<tool_call>NOT_JSON</tool_call> <tool_call>{"name":"valid_tool","arguments":{}}</tool_call>"#;
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "valid_tool");
    }

    #[test]
    fn parse_tool_calls_handles_no_closing_tag() {
        // Qwen sometimes omits the closing </tool_call> tag
        let text = "<tool_call>\n{\"name\":\"brave__search\",\"arguments\":{\"query\":\"WWDC 2026\"}}";
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "brave__search");
    }

    #[test]
    fn parse_tool_calls_trims_whitespace_around_json() {
        // Model outputs newline after <tool_call> before the JSON
        let text = "<tool_call>\n  {\"name\":\"my_tool\",\"arguments\":{}}\n</tool_call>";
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "my_tool");
    }

    #[test]
    fn parse_tool_calls_handles_multiple_no_closing_tags() {
        // Model outputs multiple <tool_call> lines without closing tags
        let text = "<tool_call>{\"name\":\"tool_a\"}\n<tool_call>{\"name\":\"tool_b\"}\n<tool_call>{\"name\":\"tool_c\"}";
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].tool_name, "tool_a");
        assert_eq!(calls[1].tool_name, "tool_b");
        assert_eq!(calls[2].tool_name, "tool_c");
    }

    #[test]
    fn parse_tool_calls_handles_missing_arguments() {
        // Model omits the arguments field entirely
        let text = "<tool_call>{\"name\":\"list_files\"}</tool_call>";
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "list_files");
        assert!(calls[0].args.is_object());
    }

    #[test]
    fn ai_chat_reply_serializes_tool_calls() {
        let reply = AiChatReply {
            content: None,
            tool_calls: vec![AiToolCall {
                id: "call_abc".into(),
                tool_name: "fs__read_file".into(),
                args: serde_json::json!({"path": "/tmp/test.txt"}),
                thought_signature: None,
            }],
            tool_calling_unsupported: false,
            tool_fallback_reason: None,
            raw_tool_calls: None,
        };
        let j = serde_json::to_value(&reply).unwrap();
        assert!(j["content"].is_null());
        assert_eq!(j["tool_calls"][0]["tool_name"], "fs__read_file");
    }

    #[test]
    fn ai_chat_reply_serializes_text_content() {
        let reply = AiChatReply {
            content: Some("hello world".into()),
            tool_calls: vec![],
            tool_calling_unsupported: false,
            tool_fallback_reason: None,
            raw_tool_calls: None,
        };
        let j = serde_json::to_value(&reply).unwrap();
        assert_eq!(j["content"], "hello world");
        assert!(j["tool_calls"].as_array().unwrap().is_empty());
    }

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
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
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
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
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
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
        assert!(prompt.contains("Directory listing"));
        assert!(prompt.contains("Cargo.toml"));
    }

    #[test]
    fn prompt_omits_context_sections_when_none() {
        let snap = make_snap("macos", "zsh", "/home");
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
        assert!(!prompt.contains("Recent terminal output"));
        assert!(!prompt.contains("Directory listing"));
    }

    #[test]
    fn chat_prompt_contains_environment_fields() {
        let snap = make_snap("windows", "pwsh", "C:\\Users\\a");
        let prompt = build_chat_prompt(&snap, Locale::ZhTw);
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
        let prompt = build_chat_prompt(&snap, Locale::ZhTw);
        assert!(prompt.contains("Recent terminal output"));
        assert!(prompt.contains("foo  bar"));
    }

    #[test]
    fn chat_prompt_instructs_cmd_tag_format() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap, Locale::ZhTw);
        assert!(prompt.contains("<cmd>"), "prompt must mention <cmd> tag");
        assert!(prompt.contains("</cmd>"), "prompt must mention closing tag");
    }

    #[test]
    fn chat_prompt_omits_json_schema_rules() {
        let snap = make_snap("linux", "bash", "/");
        let prompt = build_chat_prompt(&snap, Locale::ZhTw);
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
        let prompt = build_chat_prompt(&snap, Locale::ZhTw);
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
        let prompt = build_single_command_prompt(&snap, Locale::ZhTw);
        assert!(prompt.contains("Recent terminal output"));
        // The full 3000-char string must NOT be present — truncation happened.
        assert!(!prompt.contains(&long_output), "full 3000-char output should have been truncated");
        // The tail (last 2000 chars) IS present — we kept the most recent output.
        assert!(prompt.contains(&long_output[1000..]), "the tail 2000 chars must be in the prompt");
    }
}

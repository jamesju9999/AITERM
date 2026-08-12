pub mod tools;
pub mod tree;

use std::path::PathBuf;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::ai::{
    tool_markup::visible_prefix_len,
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
    GenerateWithToolsResult, Locale, McpToolDefinition, QueryMode,
};

const MAX_TOOL_ROUNDS: usize = 20;
/// Conservative limit: ~4 chars/token for Latin/code, 1-2 chars/token for CJK.
/// We aim for ≤50 000 tokens of tool results to leave room for model output.
const TOKEN_ESTIMATE_LIMIT: usize = 50_000;
/// Trigger context compression when accumulated tool results exceed this threshold.
const CHECKPOINT_THRESHOLD: usize = 30_000;
/// Maximum number of mid-session compressions allowed before forcing a final answer.
const MAX_CHECKPOINTS: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeAssistantEvent {
    ToolCall {
        session_id: String,
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolProgress {
        session_id: String,
        call_id: String,
        message: String,
    },
    ToolResult {
        session_id: String,
        call_id: String,
        content: String,
        truncated: bool,
    },
    TextDelta {
        session_id: String,
        delta: String,
    },
    Checkpoint {
        session_id: String,
        number: usize,
    },
    Done {
        session_id: String,
    },
    Error {
        session_id: String,
        message: String,
    },
    FallbackMode {
        session_id: String,
    },
    TokenCount {
        session_id: String,
        count: usize,
        limit: usize,
    },
}

fn tool_definitions() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition {
            name: "get_file_tree".into(),
            description: "Get a multi-level directory tree rooted at the given path. Much more efficient than multiple list_directory calls for understanding project structure. Use this first to orient yourself.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":  { "type": "string",  "description": "Path relative to project root, e.g. '/' or 'src/'" },
                    "depth": { "type": "integer", "description": "Depth to traverse (1–5). Default 3.", "default": 3 }
                },
                "required": ["path"]
            }),
        },
        McpToolDefinition {
            name: "find_files".into(),
            description: "Find files by name pattern (case-insensitive substring). Returns paths relative to project root. Use when you know part of a file name but not its location.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name_pattern":   { "type": "string", "description": "Substring to match against file names, e.g. 'JMS', 'Config', 'Service'" },
                    "file_extension": { "type": "string", "description": "Optional extension filter, e.g. '.java', '.xml', '.properties'" }
                },
                "required": ["name_pattern"]
            }),
        },
        McpToolDefinition {
            name: "list_directory".into(),
            description: "List files and subdirectories at the given path (one level). Use get_file_tree for multi-level views.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to project root, e.g. '/', 'src/', 'src/components'" }
                },
                "required": ["path"]
            }),
        },
        McpToolDefinition {
            name: "read_file_lines".into(),
            description: "Read a specific line range from a file (1-indexed, inclusive). PREFER this over read_file after search_in_files finds a match — read only the relevant section, not the whole file. Lines are returned with line numbers prefixed.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":       { "type": "string",  "description": "File path relative to project root" },
                    "start_line": { "type": "integer", "description": "First line to read (1-indexed). Use the line number from search_in_files minus ~10 for context." },
                    "end_line":   { "type": "integer", "description": "Last line to read (1-indexed, inclusive). Max 200 lines per call." }
                },
                "required": ["path", "start_line", "end_line"]
            }),
        },
        McpToolDefinition {
            name: "read_file".into(),
            description: "Read the full contents of a file. Use only when you need the entire file (e.g. to see all imports or the full class structure). Prefer read_file_lines when you already know the relevant line range from search results.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path relative to project root, e.g. 'src/main.rs'" }
                },
                "required": ["path"]
            }),
        },
        McpToolDefinition {
            name: "search_in_files".into(),
            description: "Search for a text pattern in project files (case-insensitive). ALWAYS set `path` to the most specific relevant subdirectory — never search the whole project when you already know where to look.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query":        { "type": "string", "description": "Text to search for (single precise term gives better results than broad terms)" },
                    "path":         { "type": "string", "description": "Subdirectory to search within (relative to project root, e.g. 'src/main/java/'). Omit only if you have no idea where the code lives." },
                    "file_pattern": { "type": "string", "description": "Optional file extension filter, e.g. '.java', '.xml', '.properties'" }
                },
                "required": ["query"]
            }),
        },
    ]
}

fn build_system_prompt(project_root: &str, locale: Locale) -> String {
    let language = crate::ai::language_name(locale);
    format!(
r#"You are a code assistant helping the user understand and work with a software project.

## Reply Language — MANDATORY
Write your ENTIRE reply in {language}: every sentence of explanation, every heading, every summary, and every text label inside Mermaid diagrams. The project's code, file names, and tool output stay in their original language, but that must NOT change the language you write in. Do not default to English. This rule overrides everything else in this prompt.

Project root: {project_root}

## Tools

- get_file_tree(path, depth?): multi-level directory tree. depth 1–5, default 3.
- find_files(name_pattern, file_extension?): find files by name substring (case-insensitive). Fast — use before search_in_files.
- list_directory(path): list one directory level.
- read_file_lines(path, start_line, end_line): read a specific line range (1-indexed, max 200 lines). **Use this instead of read_file whenever search_in_files gives you line numbers.**
- read_file(path): read the full file (capped at 100 KB). Use only when you need the whole file.
- search_in_files(query, path?, file_pattern?): grep within a directory subtree. Returns file:line matches. `path` narrows the scope — always set it.

## Search Strategy (follow in this order)

1. Call get_file_tree("/", 3) once to understand the top-level structure.
2. Identify the most specific subdirectory where relevant code likely lives (e.g. src/main/java/ for Java, src/ for JS/TS).
3. Use find_files to locate files by name — faster than grep.
4. Use search_in_files (scoped to a subdirectory) to find the relevant line numbers.
5. Use read_file_lines to read only the section around those line numbers (e.g. match at line 42 → read lines 30–100). This keeps context small.
6. Use read_file only when you genuinely need the entire file.
7. Answer immediately once you have enough verified content.

## Hard Limits

- At most 3 search_in_files calls per question. If you haven't found what you need in 3 searches, state what you found and what is missing.
- Always set `path` in search_in_files unless you have genuinely no idea where to look.
- Pick one precise search term per call — a focused term gives better results than a broad one.
- NEVER mention a file path unless you confirmed it exists via a tool call in this session.
- If you cannot find something after targeted searching, say so honestly.

## Accuracy — Non-Negotiable

### CODE BLOCKS ARE EVIDENCE, NOT ILLUSTRATION
Show a code block ONLY when ALL of the following are true:
  1. You called read_file or read_file_lines on that EXACT file path in this session.
  2. The code in the block is COPIED VERBATIM from the tool result — not reconstructed, not inferred.

**ABSOLUTELY FORBIDDEN:**
  - Showing a code block after ONLY using search_in_files (search returns 300-char snippets, not files).
  - Constructing code that "should exist" based on patterns, conventions, or training knowledge.
  - Writing "關鍵程式碼：" or "實際程式碼：" followed by code you did not read verbatim.
  - Combining fragments from multiple search snippets into one synthesised code block.
  - Adding explanatory comments to inferred code to make it look authentic.
  - Citing a file path you did not open with read_file/read_file_lines this session.

**What to write instead when you haven't read the file:**
  "I found a reference to X in search results but have not opened the file to confirm the implementation."

### Factual claims require direct evidence
- Class names, method names, field names, annotations → read the file, quote verbatim.
- Configuration values, queue names, URLs, constants → read the file, quote the exact value.
- Control flow, call chains → read EVERY file in the chain, not just the entry point.
- search_in_files snippets are LEADS ONLY. A truncated 300-char line proves the text exists somewhere — it does NOT prove file structure, class layout, or surrounding logic.

- Reminder: the entire reply, including all diagram labels, must be in {language}.
- **Mermaid diagrams**: node IDs must be plain ASCII identifiers (e.g. `A`, `LoadConf`). Wrap every node label and edge label in double quotes, e.g. `A["使用者點擊「連線」按鈕"]` and `-->|"host, port, user"|`. NEVER put the characters `|`, `<`, `>`, or `"` inside a label — they collide with Mermaid syntax and break the diagram. Write `Bearer key` not `Bearer <key>`, and use a comma or space instead of `|`. Do not use `<br/>` inside labels — use a space instead. Prefer the theme's default node colors; if you must set a `fill`, keep it dark/muted so light text stays readable, and do not rely on custom colors to convey meaning."#
    )
}

fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

async fn dispatch_tool(
    root: &PathBuf,
    name: &str,
    args: &serde_json::Value,
    app: &AppHandle,
    session_id: &str,
    call_id: &str,
) -> (String, bool) {
    match name {
        "get_file_tree" => {
            let path = args["path"].as_str().unwrap_or("/").to_owned();
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
            let root_clone = root.clone();
            match tokio::task::spawn_blocking(move || tools::get_file_tree(&root_clone, &path, depth)).await {
                Ok(Ok(r)) => (r.content, r.truncated),
                Ok(Err(e)) => (format!("Error: {e}"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "find_files" => {
            let pattern = args["name_pattern"].as_str().unwrap_or("").to_owned();
            let ext = args.get("file_extension").and_then(|v| v.as_str()).map(|s| s.to_owned());
            let root_clone = root.clone();
            match tokio::task::spawn_blocking(move || tools::find_files(&root_clone, &pattern, ext.as_deref())).await {
                Ok(Ok(r)) => (r.content, r.truncated),
                Ok(Err(e)) => (format!("Error: {e}"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "list_directory" => {
            let path = args["path"].as_str().unwrap_or("/").to_owned();
            let root_clone = root.clone();
            match tokio::task::spawn_blocking(move || tools::list_directory(&root_clone, &path)).await {
                Ok(Ok(r)) => (r.content, r.truncated),
                Ok(Err(e)) => (format!("Error: {e}"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "read_file_lines" => {
            let path = args["path"].as_str().unwrap_or("").to_owned();
            let start = args["start_line"].as_u64().unwrap_or(1) as usize;
            let end = args["end_line"].as_u64().unwrap_or(start as u64 + 49) as usize;
            let root_clone = root.clone();
            match tokio::task::spawn_blocking(move || tools::read_file_lines(&root_clone, &path, start, end)).await {
                Ok(Ok(r)) => (r.content, r.truncated),
                Ok(Err(e)) => (format!("Error: {e}"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "read_file" => {
            let path = args["path"].as_str().unwrap_or("").to_owned();
            let root_clone = root.clone();
            match tokio::task::spawn_blocking(move || tools::read_file(&root_clone, &path)).await {
                Ok(Ok(r)) => (r.content, r.truncated),
                Ok(Err(e)) => (format!("Error: {e}"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "search_in_files" => {
            let query = args["query"].as_str().unwrap_or("").to_owned();
            let pattern = args.get("file_pattern").and_then(|v| v.as_str()).map(|s| s.to_owned());
            let search_path = args.get("path").and_then(|v| v.as_str()).map(|s| s.to_owned());
            let root_clone = root.clone();
            let app_clone = app.clone();
            let session_id_owned = session_id.to_owned();
            let call_id_owned = call_id.to_owned();

            let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<String>(128);

            let join = tokio::task::spawn_blocking(move || {
                tools::search_in_files_with_progress(
                    &root_clone,
                    &query,
                    pattern.as_deref(),
                    search_path.as_deref(),
                    &|dir: &str| { let _ = progress_tx.try_send(dir.to_owned()); },
                )
            });
            tokio::pin!(join);

            loop {
                tokio::select! {
                    result = &mut join => {
                        return match result {
                            Ok(Ok(r)) => (r.content, r.truncated),
                            Ok(Err(e)) => (format!("Error: {e}"), false),
                            Err(e) => (format!("Error: {e}"), false),
                        };
                    }
                    msg = progress_rx.recv() => {
                        if let Some(msg) = msg {
                            let _ = app_clone.emit("code-assistant-event", CodeAssistantEvent::ToolProgress {
                                session_id: session_id_owned.clone(),
                                call_id: call_id_owned.clone(),
                                message: msg,
                            });
                        }
                    }
                }
            }
        }
        _ => (format!("Unknown tool: {name}"), false),
    }
}

/// Recursively sort object keys so two JSON-equivalent values always format
/// to the same string, regardless of the order the model emitted them in.
/// `serde_json`'s `preserve_order` feature (needed elsewhere for settings
/// files) makes `Value`'s `Display` reflect insertion order rather than a
/// canonical one, so without this two calls with the same arguments in a
/// different order would no longer dedupe.
fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: std::collections::BTreeMap<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), canonicalize_json(v)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json).collect())
        }
        other => other.clone(),
    }
}

/// Build a deduplication key for a tool call.
fn tool_call_key(tool_name: &str, args: &serde_json::Value) -> String {
    format!("{}:{}", tool_name, canonicalize_json(args))
}

/// Parse tool calls from XML text that local models emit instead of proper JSON tool-calls.
///
/// Handles two formats:
///   1. JSON inside tag:  <tool_call>{"name":"fn","arguments":{...}}</tool_call>
///   2. Attribute style:  <function=fn> <parameter=key> val </parameter> </function>
fn parse_xml_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    let mut results = Vec::new();

    // Format 1: <tool_call>JSON</tool_call>
    let mut search = text;
    while let Some(start) = search.find("<tool_call>") {
        let after = &search[start + "<tool_call>".len()..];
        let inner = if let Some(end) = after.find("</tool_call>") {
            after[..end].trim()
        } else {
            after.trim()
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(inner) {
            if let Some(name) = v["name"].as_str() {
                let args = v.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                results.push((name.to_owned(), args));
            }
        }
        // Advance past this tag
        let skip = start + "<tool_call>".len();
        search = &search[skip..];
    }

    // Format 2: <function=NAME> <parameter=KEY> VALUE </parameter> … </function>
    // Only attempt if Format 1 found nothing.
    if results.is_empty() {
        let mut s = text;
        while let Some(fn_pos) = s.find("<function=") {
            let after_fn = &s[fn_pos + "<function=".len()..];
            let name_end = after_fn.find('>').unwrap_or(after_fn.len());
            let fn_name = after_fn[..name_end].trim().to_owned();
            let rest = &after_fn[name_end..];

            let mut args = serde_json::Map::new();
            let mut param_search = rest;
            while let Some(p) = param_search.find("<parameter=") {
                let after_p = &param_search[p + "<parameter=".len()..];
                let key_end = after_p.find('>').unwrap_or(after_p.len());
                let key = after_p[..key_end].trim().to_owned();
                let after_key = &after_p[key_end + 1..];
                let val = if let Some(c) = after_key.find("</parameter>") {
                    after_key[..c].trim()
                } else {
                    after_key.trim()
                };
                let json_val = if let Ok(n) = val.parse::<i64>() {
                    serde_json::Value::Number(n.into())
                } else {
                    serde_json::Value::String(val.to_owned())
                };
                args.insert(key, json_val);
                param_search = &after_key[after_key.find("</parameter>").map(|x| x + "</parameter>".len()).unwrap_or(after_key.len())..];
            }

            if !fn_name.is_empty() {
                results.push((fn_name, serde_json::Value::Object(args)));
            }
            s = &after_fn[name_end..];
        }
    }

    results
}

pub async fn run_chat(
    project_root: String,
    messages: Vec<ChatMessage>,
    provider: std::sync::Arc<dyn AiProvider>,
    session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    let root_path = PathBuf::from(&project_root);
    let tool_defs = tool_definitions();
    let system_prompt = build_system_prompt(&project_root, locale);

    let mut conversation = messages;
    let mut token_estimate = estimate_tokens(&system_prompt);
    let mut rounds = 0usize;
    let mut checkpoints = 0usize;
    // Deduplication: skip tool calls whose (name, args) key was already executed.
    let mut seen_calls: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let _ = app.emit("code-assistant-event", CodeAssistantEvent::TokenCount {
            session_id: session_id.clone(),
            count: token_estimate,
            limit: TOKEN_ESTIMATE_LIMIT,
        });

        // ── Checkpoint compression ────────────────────────────────────────────
        // When tool results fill most of the context budget, summarise what has
        // been found so far, discard the raw tool history, and continue with a
        // compact checkpoint message. This keeps the context window available
        // for further investigation. Allowed at most MAX_CHECKPOINTS times.
        if token_estimate >= CHECKPOINT_THRESHOLD && checkpoints < MAX_CHECKPOINTS {
            checkpoints += 1;
            let _ = app.emit("code-assistant-event", CodeAssistantEvent::Checkpoint {
                session_id: session_id.clone(),
                number: checkpoints,
            });
            let summary = generate_checkpoint_summary(
                &conversation, provider.clone(), locale
            ).await;
            conversation = compress_conversation(conversation, &summary, checkpoints);
            token_estimate = estimate_tokens(&summary);
            continue;
        }

        let force_answer = rounds >= MAX_TOOL_ROUNDS
            || (token_estimate >= TOKEN_ESTIMATE_LIMIT && checkpoints >= MAX_CHECKPOINTS);

        let language = crate::ai::language_name(locale);
        let effective_prompt = if force_answer {
            format!(
                "{system_prompt}\n\n\
                 STOP ALL TOOL CALLS NOW. Research limit reached after {rounds} rounds.\n\
                 Write your FINAL ANSWER in {language}. Rules:\n\
                 1. Natural language ONLY — absolutely NO JSON, NO arrays, NO file path lists.\n\
                 2. Only state facts you directly read in files during this session.\n\
                 3. If you did not find something, explicitly say so.\n\
                 4. Summarise your findings in clear prose."
            )
        } else {
            system_prompt.clone()
        };

        let req = GenerateRequest {
            system_prompt: effective_prompt,
            messages: conversation.clone(),
            context: Default::default(),
            mode: QueryMode::Chat,
            max_tokens: None,
        };

        let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
        let provider_clone = provider.clone();
        let tools_for_call = if force_answer { vec![] } else { tool_defs.clone() };
        let force_answer_clone = force_answer;

        let join = tokio::spawn(async move {
            if force_answer_clone {
                provider_clone.generate(req, tx).await
                    .map(|_| GenerateWithToolsResult::Text(String::new()))
            } else {
                provider_clone.generate_with_tools(req, tools_for_call, tx).await
            }
        });

        // Drain streaming chunks (final text arrives here).
        //
        // 只送「標記之前」那段：模型把工具呼叫寫成文字時，那條指令不能出現在
        // 答案裡。累積起來邊送邊切，串流因此保留（能串的 provider 照樣逐字出）。
        let mut seen = String::new();
        let mut emitted = 0usize;
        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                seen.push_str(&chunk.delta);
                let safe = visible_prefix_len(&seen);
                if safe > emitted {
                    let delta = seen[emitted..safe].to_string();
                    emitted = safe;
                    let _ = app.emit("code-assistant-event", CodeAssistantEvent::TextDelta {
                        session_id: session_id.clone(),
                        delta,
                    });
                }
            }
            if chunk.done { break; }
        }

        match join.await {
            Err(e) => {
                let msg = e.to_string();
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::Error {
                    session_id: session_id.clone(),
                    message: msg.clone(),
                });
                return Err(AiError::Network { message: msg });
            }
            Ok(Err(AiError::ToolCallingUnsupported)) |
            Ok(Ok(GenerateWithToolsResult::Unsupported)) => {
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::FallbackMode {
                    session_id: session_id.clone(),
                });
                return run_fallback(root_path, conversation, provider, session_id, locale, app).await;
            }
            Ok(Err(e)) => {
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::Error {
                    session_id: session_id.clone(),
                    message: e.to_string(),
                });
                return Err(e);
            }
            Ok(Ok(GenerateWithToolsResult::Text(text))) => {
                // Local models sometimes emit XML-style tool calls as text instead of JSON
                // (common after checkpoint compression resets context). Try to parse and
                // execute them so the loop can continue rather than stopping prematurely.
                let xml_calls = parse_xml_tool_calls(&text);
                if !xml_calls.is_empty() {
                    for (tool_name, args) in xml_calls {
                        let key = tool_call_key(&tool_name, &args);
                        if seen_calls.contains(&key) { continue; }
                        seen_calls.insert(key);

                        let call_id = format!("xml_{}", uuid::Uuid::new_v4());
                        let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            tool: tool_name.clone(),
                            args: args.clone(),
                        });
                        let (result_content, truncated) =
                            dispatch_tool(&root_path, &tool_name, &args, &app, &session_id, &call_id).await;
                        token_estimate += estimate_tokens(&result_content);
                        let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolResult {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            content: result_content.clone(),
                            truncated,
                        });
                        conversation.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::Value::String(result_content),
                            tool_call_id: Some(call_id),
                            tool_calls: None,
                        });
                    }
                    rounds += 1;
                } else {
                    // Genuine text answer — already streamed via TextDelta events above
                    let _ = app.emit("code-assistant-event", CodeAssistantEvent::Done {
                        session_id: session_id.clone(),
                    });
                    return Ok(());
                }
            }
            Ok(Ok(GenerateWithToolsResult::ToolCalls { calls, raw })) => {
                // Append assistant tool-call message to conversation
                conversation.push(ChatMessage {
                    role: "assistant".into(),
                    content: serde_json::Value::Null,
                    tool_call_id: None,
                    tool_calls: raw.or_else(|| serde_json::to_value(&calls).ok()),
                });

                for call in &calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&call.args.to_string()).unwrap_or_default();

                    // Skip calls already executed in this session (deduplication)
                    let key = tool_call_key(&call.tool_name, &args);
                    if seen_calls.contains(&key) {
                        let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.tool_name.clone(),
                            args: args.clone(),
                        });
                        let skip_msg = format!("(skipped: same call already executed earlier in this session)");
                        let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolResult {
                            session_id: session_id.clone(),
                            call_id: call.id.clone(),
                            content: skip_msg.clone(),
                            truncated: false,
                        });
                        conversation.push(ChatMessage {
                            role: "tool".into(),
                            content: serde_json::Value::String(skip_msg),
                            tool_call_id: Some(call.id.clone()),
                            tool_calls: None,
                        });
                        continue;
                    }
                    seen_calls.insert(key);

                    let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolCall {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        tool: call.tool_name.clone(),
                        args: args.clone(),
                    });

                    let (result_content, truncated) =
                        dispatch_tool(&root_path, &call.tool_name, &args, &app, &session_id, &call.id).await;

                    token_estimate += estimate_tokens(&result_content);

                    let _ = app.emit("code-assistant-event", CodeAssistantEvent::ToolResult {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        content: result_content.clone(),
                        truncated,
                    });

                    conversation.push(ChatMessage {
                        role: "tool".into(),
                        content: serde_json::Value::String(result_content),
                        tool_call_id: Some(call.id.clone()),
                        tool_calls: None,
                    });
                }

                rounds += 1;
            }
        }
    }
}

async fn run_fallback(
    root_path: PathBuf,
    messages: Vec<ChatMessage>,
    provider: std::sync::Arc<dyn AiProvider>,
    session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    // Phase 1: get directory listing, ask AI which files to read
    let dir_listing = tools::list_directory(&root_path, "/")
        .map(|r| r.content)
        .unwrap_or_else(|e| format!("(could not list: {e})"));

    let language = crate::ai::language_name(locale);
    let phase1_prompt = format!(
r#"Given the project file tree below, output ONLY a JSON array (max 10 items) of file paths
you need to read to answer the user's question. No explanation. Example: ["src/main.rs", "Cargo.toml"]

Project structure:
{dir_listing}"#
    );

    let phase1_req = GenerateRequest {
        system_prompt: phase1_prompt,
        messages: messages.clone(),
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: Some(512),
    };

    let (tx1, mut rx1) = mpsc::channel::<GenerateChunk>(16);
    let p1 = provider.clone();
    let j1 = tokio::spawn(async move { p1.generate(phase1_req, tx1).await });
    let mut buf1 = String::new();
    while let Some(chunk) = rx1.recv().await {
        buf1.push_str(&chunk.delta);
        if chunk.done { break; }
    }
    let _ = j1.await;

    // Parse JSON array from phase 1 response
    let file_paths: Vec<String> = {
        let try_parse = |s: &str| serde_json::from_str::<Vec<String>>(s).ok();
        try_parse(&buf1)
            .or_else(|| {
                let start = buf1.find('[')?;
                let end = buf1.rfind(']')?;
                try_parse(&buf1[start..=end])
            })
            .unwrap_or_default()
    };

    // Phase 2: read files and answer
    let mut file_context = String::new();
    for path in file_paths.iter().take(10) {
        if let Ok(r) = tools::read_file(&root_path, path) {
            file_context.push_str(&format!("\n\n### {path}\n```\n{}\n```", r.content));
        }
    }

    let phase2_prompt = format!(
        "You are a code assistant. Answer the user's question based on the project files below.\nRespond in {language}.\n{file_context}"
    );

    let phase2_req = GenerateRequest {
        system_prompt: phase2_prompt,
        messages,
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(32);
    let p2 = provider.clone();
    let j2 = tokio::spawn(async move { p2.generate(phase2_req, tx2).await });
    while let Some(chunk) = rx2.recv().await {
        if !chunk.delta.is_empty() {
            let _ = app.emit("code-assistant-event", CodeAssistantEvent::TextDelta {
                session_id: session_id.clone(),
                delta: chunk.delta.clone(),
            });
        }
        if chunk.done { break; }
    }
    let _ = j2.await;

    let _ = app.emit("code-assistant-event", CodeAssistantEvent::Done {
        session_id: session_id.clone(),
    });
    Ok(())
}

/// Ask the model to distil all confirmed findings from the current conversation
/// into a compact summary. Called when tool results approach the context limit.
async fn generate_checkpoint_summary(
    conversation: &[ChatMessage],
    provider: std::sync::Arc<dyn AiProvider>,
    locale: Locale,
) -> String {
    let language = crate::ai::language_name(locale);
    let system = format!(
        "You are creating a research checkpoint. The conversation below shows a code \
         investigation with tool calls and results. Write a concise structured summary \
         IN {language} of ONLY what has been CONFIRMED — facts you directly observed:\n\
         - Directories and files found (exact paths)\n\
         - Class names, method names, annotations (quote exactly as seen)\n\
         - Configuration values, queue names, URLs (quote exactly)\n\
         - What was searched but NOT found\n\
         - What still needs to be investigated\n\n\
         Max 500 words. Facts only. Do NOT speculate. Do NOT make tool calls."
    );

    let req = GenerateRequest {
        system_prompt: system,
        messages: conversation.to_vec(),
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: Some(700),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let p = provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }
    let _ = join.await;
    buf
}

/// Replace the full tool-call history with the checkpoint summary.
/// Keeps the original user question so the model retains the goal.
fn compress_conversation(
    conversation: Vec<ChatMessage>,
    summary: &str,
    checkpoint_n: usize,
) -> Vec<ChatMessage> {
    let mut result: Vec<ChatMessage> = conversation
        .into_iter()
        .filter(|m| m.role == "user")
        .collect();

    // `summary` is raw streamed model output and normally ends with a newline
    // (and is empty outright when the summarising call failed). Anthropic
    // validates the last assistant turn as a prefill and rejects it with
    // "final assistant content cannot end with trailing whitespace", so trim.
    let summary = summary.trim();
    let checkpoint = if summary.is_empty() {
        format!("[Checkpoint #{checkpoint_n} — no findings could be summarised]")
    } else {
        format!("[Checkpoint #{checkpoint_n} — confirmed findings so far]\n{summary}")
    };

    result.push(ChatMessage {
        role: "assistant".into(),
        content: serde_json::Value::String(checkpoint),
        tool_call_id: None,
        tool_calls: None,
    });
    // Newer Claude models reject a trailing assistant turn outright (prefill is
    // unsupported), so close the compressed history with a user turn.
    result.push(ChatMessage {
        role: "user".into(),
        content: serde_json::Value::String(
            "Continue from the checkpoint above: keep investigating with the tools, \
             or write the final answer if you already have enough."
                .into(),
        ),
        tool_call_id: None,
        tool_calls: None,
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // 模型偶爾會把工具呼叫寫成一般文字。那段是給 parse_xml_tool_calls 吃的
    // 指令，不是講給使用者聽的話——但 TextDelta 是在解析之前就送出去的，於是
    // `<tool_call> <function=read_file> …` 整條印在答案裡（實測回報）。
    
    
    // 串流時標記是一個 delta 一個 delta 拼出來的，中間會經過 "<"、"<fun"…
    // 不擋的話畫面會先閃出半截標記再被後續內容蓋掉。
    
    
    
    #[test]
    fn parse_xml_json_format() {
        let text = r#"<tool_call>{"name":"get_file_tree","arguments":{"path":"src","depth":3}}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "get_file_tree");
        assert_eq!(calls[0].1["path"], "src");
        assert_eq!(calls[0].1["depth"], 3);
    }

    #[test]
    fn parse_xml_attribute_format() {
        let text = "<function=read_file> <parameter=path> src/Foo.java </parameter> </function>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "read_file");
        assert_eq!(calls[0].1["path"], "src/Foo.java");
    }

    #[test]
    fn parse_xml_no_match_returns_empty() {
        let calls = parse_xml_tool_calls("Here is my answer: the answer is 42.");
        assert!(calls.is_empty());
    }

    #[test]
    fn dedup_key_is_stable() {
        let args = serde_json::json!({"path": "src", "depth": 3});
        let k1 = tool_call_key("get_file_tree", &args);
        let k2 = tool_call_key("get_file_tree", &args);
        assert_eq!(k1, k2);
    }

    #[test]
    fn dedup_key_ignores_argument_order() {
        // Same arguments, different key order — models reorder keys often enough
        // that this is a realistic way to accidentally repeat a call.
        let a = serde_json::json!({"path": "src", "depth": 3});
        let b = serde_json::json!({"depth": 3, "path": "src"});
        assert_eq!(tool_call_key("get_file_tree", &a), tool_call_key("get_file_tree", &b));
    }

    fn msg(role: &str, content: serde_json::Value) -> ChatMessage {
        ChatMessage { role: role.into(), content, tool_call_id: None, tool_calls: None }
    }

    #[test]
    fn compressed_conversation_is_anthropic_safe() {
        let conversation = vec![
            msg("user", serde_json::json!("這個專案怎麼啟動？")),
            msg("assistant", serde_json::Value::Null),
            msg("tool", serde_json::json!("file tree ...")),
        ];

        // Streamed model output normally ends with a newline.
        let out = compress_conversation(conversation, "已確認：入口在 src/main.rs。\n\n", 1);

        assert!(out.iter().all(|m| m.role != "tool"));
        assert_eq!(out.last().unwrap().role, "user", "must not end on an assistant turn");

        let checkpoint = out.iter().find(|m| m.role == "assistant").expect("checkpoint turn");
        let text = checkpoint.content.as_str().expect("checkpoint is plain text");
        assert!(text.contains("Checkpoint #1"));
        assert_eq!(text, text.trim_end(), "assistant content must not end with whitespace");
    }

    #[test]
    fn empty_checkpoint_summary_does_not_produce_blank_assistant_turn() {
        let out = compress_conversation(vec![msg("user", serde_json::json!("問題"))], "  \n ", 2);

        let checkpoint = out.iter().find(|m| m.role == "assistant").expect("checkpoint turn");
        let text = checkpoint.content.as_str().expect("checkpoint is plain text");
        assert!(!text.is_empty());
        assert_eq!(text, text.trim_end());
    }
}

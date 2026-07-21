pub mod tools;
pub mod tree;

use std::path::PathBuf;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::ai::{
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

Every factual claim about code must be directly traceable to file content you read with read_file in this session:
- Class names, method names, field names, annotations: read the file, then state what you saw verbatim.
- Configuration values, bean definitions, queue names, URLs: read the file, quote the exact value.
- Control flow, call chains, business logic: read every file in the chain, not just the entry point.
- Search snippets (search_in_files results) are leads, NOT evidence. A 300-character truncated line is not enough to make a factual claim — read the full file first.
- If you have not read the file, say "I found a reference in search results but have not verified the full implementation." Never paraphrase or infer from grep output alone.

- Respond in {language}.
- **Mermaid diagrams**: ONLY ASCII characters allowed everywhere (node IDs, node labels, edge labels, subgraph IDs). NO Chinese/CJK, NO `<br/>`, NO `()` inside `|edge labels|`, NO `/` in labels. Write all explanatory text in {language} OUTSIDE the diagram block as a legend or bullet list. If a diagram cannot be drawn with pure ASCII, describe it in text instead."#
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

    loop {
        // ── Checkpoint compression ────────────────────────────────────────────
        // When tool results fill most of the context budget, summarise what has
        // been found so far, discard the raw tool history, and continue with a
        // compact checkpoint message. This keeps the context window available
        // for further investigation. Allowed at most MAX_CHECKPOINTS times.
        if token_estimate >= CHECKPOINT_THRESHOLD && checkpoints < MAX_CHECKPOINTS {
            checkpoints += 1;
            let _ = app.emit("code-assistant-event", CodeAssistantEvent::TextDelta {
                session_id: session_id.clone(),
                delta: format!(
                    "\n\n> [Checkpoint #{checkpoints}：正在壓縮已蒐集的資料，繼續探索...]\n\n"
                ),
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

        // Drain streaming chunks (final text arrives here)
        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::TextDelta {
                    session_id: session_id.clone(),
                    delta: chunk.delta.clone(),
                });
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
            Ok(Ok(GenerateWithToolsResult::Text(_))) => {
                // Text was already streamed via TextDelta events above
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::Done {
                    session_id: session_id.clone(),
                });
                return Ok(());
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

    result.push(ChatMessage {
        role: "assistant".into(),
        content: serde_json::Value::String(format!(
            "[Checkpoint #{checkpoint_n} — confirmed findings so far]\n{summary}"
        )),
        tool_call_id: None,
        tool_calls: None,
    });
    result
}

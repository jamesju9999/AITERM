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
/// ~4 chars per token; leave buffer below 262k local model limit
const TOKEN_ESTIMATE_LIMIT: usize = 200_000;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodeAssistantEvent {
    ToolCall {
        session_id: String,
        call_id: String,
        tool: String,
        args: serde_json::Value,
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
            name: "list_directory".into(),
            description: "List files and subdirectories at the given path relative to the project root. Directories are shown with trailing '/'. Use '/' for the project root.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path relative to project root, e.g. '/', 'src/', 'src/components'" }
                },
                "required": ["path"]
            }),
        },
        McpToolDefinition {
            name: "read_file".into(),
            description: "Read the contents of a file. Path is relative to project root. Files over 100 KB are truncated.".into(),
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
            description: "Search for a text pattern across all project files (case-insensitive). Returns file:line matches, max 50 results.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query":        { "type": "string", "description": "Text to search for" },
                    "file_pattern": { "type": "string", "description": "Optional file extension, e.g. '.rs', '.ts'" }
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

You have three tools:
- list_directory(path): list files/subdirectories (use "/" for root)
- read_file(path): read a file's content
- search_in_files(query, file_pattern?): grep across the project

Instructions:
1. Start by exploring the project structure if you need it.
2. Only read files that are directly relevant to the user's question.
3. Be concise and direct in your answers.
4. Always reference specific file paths when discussing code.
5. Respond in {language}.
6. When drawing Mermaid diagrams, use only ASCII characters in node IDs, edge labels, and subgraph IDs. Put translated labels in a legend or description outside the diagram block to avoid parse errors."#
    )
}

fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

fn dispatch_tool(root: &PathBuf, name: &str, args: &serde_json::Value) -> (String, bool) {
    match name {
        "list_directory" => {
            let path = args["path"].as_str().unwrap_or("/");
            match tools::list_directory(root, path) {
                Ok(r) => (r.content, r.truncated),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "read_file" => {
            let path = args["path"].as_str().unwrap_or("");
            match tools::read_file(root, path) {
                Ok(r) => (r.content, r.truncated),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "search_in_files" => {
            let query = args["query"].as_str().unwrap_or("");
            let pattern = args.get("file_pattern").and_then(|v| v.as_str());
            match tools::search_in_files(root, query, pattern) {
                Ok(r) => (r.content, r.truncated),
                Err(e) => (format!("Error: {e}"), false),
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

    loop {
        let force_answer = rounds >= MAX_TOOL_ROUNDS || token_estimate > TOKEN_ESTIMATE_LIMIT;

        let effective_prompt = if force_answer {
            format!(
                "{system_prompt}\n\nNote: You have reached the tool call limit. Answer now based on what you have already read."
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
                        dispatch_tool(&root_path, &call.tool_name, &args);

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

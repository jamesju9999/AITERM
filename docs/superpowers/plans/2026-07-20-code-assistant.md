# Code Assistant Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增「程式庫協助」Tab，讓使用者指定專案目錄後透過 AI 的 tool-use 迭代式讀檔來回答關於程式庫的問題。

**Architecture:** 後端 Rust 的 `code_assistant` 模組內建完整 tool-use 迴圈（list_directory / read_file / search_in_files），透過 `code-assistant-event` Tauri 事件即時推送進度給前端。前端以新的 `useCodeAssistant` hook 管理狀態，`CodeAssistantView` 元件呈現對話與工具呼叫卡片。不支援 tool-use 的 provider 自動 fallback 兩段式流程。

**Tech Stack:** Rust (Tauri 2), React 19, TypeScript, Tauri `invoke` + `event::emit`

---

## 檔案清單

### 新增
| 路徑 | 用途 |
|------|------|
| `src-tauri/src/code_assistant/mod.rs` | tool-use 迴圈 + fallback 邏輯 + 事件定義 |
| `src-tauri/src/code_assistant/tools.rs` | list_directory / read_file / search_in_files 實作 |
| `src-tauri/src/code_assistant/tree.rs` | 路徑過濾（排除 node_modules、.git 等） |
| `src-tauri/src/commands/code_assistant.rs` | Tauri command: `code_assistant_chat` |
| `src/ipc/codeAssistant.ts` | 前端 IPC wrapper + 事件類型 |
| `src/hooks/useCodeAssistant.ts` | 狀態管理 hook |
| `src/components/CodeAssistantView/ToolCallCard.tsx` | 工具呼叫進度卡片元件 |
| `src/components/CodeAssistantView/index.tsx` | 主元件 |
| `src/components/CodeAssistantView/styles.css` | 樣式 |

### 修改
| 路徑 | 變更 |
|------|------|
| `src-tauri/src/lib.rs` | 宣告 `code_assistant` 模組 + 加入 invoke_handler |
| `src-tauri/src/commands/mod.rs` | 加入 `pub mod code_assistant;` |
| `src/components/TabBar/index.tsx` | `TabType` 加入 `"code-assistant"` |
| `src/components/NewTabPicker/index.tsx` | 加入 Code Assistant 選項 |
| `src/components/TerminalApp.tsx` | 加入 CodeAssistantView 渲染分支 |
| `src/components/Icons.tsx` | 新增 `CodeIcon` |
| `src/lib/i18n.ts` | 加入翻譯字串 |

---

## Task 1：路徑過濾模組（tree.rs）

**Files:**
- Create: `src-tauri/src/code_assistant/tree.rs`

- [ ] **Step 1: 建立 tree.rs**

```rust
use std::path::Path;

const EXCLUDED_DIRS: &[&str] = &[
    "node_modules", ".git", "target", "dist", "build",
    "__pycache__", ".next", ".nuxt", "vendor", ".svn", ".hg",
    "coverage", ".cache", ".parcel-cache",
];

const EXCLUDED_EXTENSIONS: &[&str] = &[
    "lock", "bin", "exe", "dll", "so", "dylib",
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "svg",
    "mp4", "mp3", "wav", "mov", "avi",
    "zip", "tar", "gz", "rar", "7z",
    "pdf", "doc", "docx", "xls", "xlsx",
    "woff", "woff2", "ttf", "eot",
    "pyc", "class", "o",
];

/// Returns true if `path` should be hidden from the AI (not listed, not readable).
pub fn is_excluded(path: &Path, project_root: &Path) -> bool {
    let relative = match path.strip_prefix(project_root) {
        Ok(r) => r,
        Err(_) => path,
    };

    // Exclude any path component that is a known build/dep directory
    for component in relative.components() {
        let name = component.as_os_str().to_string_lossy();
        // Hidden directories (starts with '.'), except the root itself
        if name.starts_with('.') && !relative.as_os_str().is_empty() {
            if path.is_dir() || path.join(&*name).is_dir() {
                return true;
            }
        }
        if EXCLUDED_DIRS.contains(&name.as_ref()) {
            return true;
        }
    }

    // Exclude files with excluded extensions
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        if EXCLUDED_EXTENSIONS.contains(&ext_str.as_ref()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn excludes_node_modules() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/node_modules/react/index.js");
        assert!(is_excluded(&path, &root));
    }

    #[test]
    fn excludes_git() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/.git/config");
        assert!(is_excluded(&path, &root));
    }

    #[test]
    fn excludes_png() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/icon.png");
        assert!(!is_excluded(&path.parent().unwrap(), &root)); // src/ is fine
        assert!(is_excluded(&path, &root)); // icon.png excluded
    }

    #[test]
    fn allows_rust_source() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/main.rs");
        assert!(!is_excluded(&path, &root));
    }

    #[test]
    fn allows_typescript_source() {
        let root = PathBuf::from("/project");
        let path = PathBuf::from("/project/src/App.tsx");
        assert!(!is_excluded(&path, &root));
    }
}
```

- [ ] **Step 2: 執行測試確認通過**

```bash
cd src-tauri && cargo test code_assistant::tree 2>&1 | tail -20
```

Expected: `test result: ok. 5 passed`

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/code_assistant/tree.rs
git commit -m "feat(code-assistant): add path filter module (tree.rs)"
```

---

## Task 2：工具實作（tools.rs）

**Files:**
- Create: `src-tauri/src/code_assistant/tools.rs`

- [ ] **Step 1: 建立 tools.rs**

```rust
use std::path::{Path, PathBuf};
use std::fs;
use crate::code_assistant::tree;

pub const MAX_LIST_ENTRIES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 100 * 1024; // 100 KB
pub const MAX_SEARCH_MATCHES: usize = 50;

pub struct ToolResult {
    pub content: String,
    pub truncated: bool,
}

/// Resolve and validate a relative path against the project root.
/// Returns Err if the resolved path escapes the project root.
fn resolve_safe(project_root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    let norm = rel_path.trim_start_matches('/');
    let target = if norm.is_empty() || norm == "." {
        project_root.to_path_buf()
    } else {
        project_root.join(norm)
    };

    // Use canonicalize so ".." escapes are caught
    let canonical_root = project_root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve project root: {e}"))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|e| format!("Path not found: {e}"))?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err("Path is outside project root".into());
    }
    Ok(canonical_target)
}

pub fn list_directory(project_root: &Path, rel_path: &str) -> Result<ToolResult, String> {
    let target = resolve_safe(project_root, rel_path)?;

    if !target.is_dir() {
        return Err(format!("{rel_path} is not a directory"));
    }

    let entries = fs::read_dir(&target)
        .map_err(|e| format!("Cannot list directory: {e}"))?;

    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| !tree::is_excluded(&e.path(), project_root))
        .take(MAX_LIST_ENTRIES + 1)
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let suffix = match e.file_type() {
                Ok(t) if t.is_dir() => "/",
                Ok(t) if t.is_symlink() => "@",
                _ => "",
            };
            format!("{name}{suffix}")
        })
        .collect();

    let truncated = names.len() > MAX_LIST_ENTRIES;
    if truncated {
        names.truncate(MAX_LIST_ENTRIES);
    }
    names.sort();

    Ok(ToolResult {
        content: names.join("\n"),
        truncated,
    })
}

pub fn read_file(project_root: &Path, rel_path: &str) -> Result<ToolResult, String> {
    let target = resolve_safe(project_root, rel_path)?;

    if tree::is_excluded(&target, project_root) {
        return Err("This file type is not readable".into());
    }

    if target.is_dir() {
        return Err(format!("{rel_path} is a directory, not a file"));
    }

    let metadata = fs::metadata(&target)
        .map_err(|e| format!("Cannot stat file: {e}"))?;

    if metadata.len() > MAX_FILE_BYTES {
        // Read up to MAX_FILE_BYTES bytes as UTF-8
        let raw = fs::read(&target)
            .map_err(|e| format!("Cannot read file: {e}"))?;
        let truncated_bytes = &raw[..MAX_FILE_BYTES as usize];
        let content = String::from_utf8_lossy(truncated_bytes).into_owned();
        return Ok(ToolResult {
            content: format!("{content}\n\n[TRUNCATED: file exceeds 100 KB limit]"),
            truncated: true,
        });
    }

    let content = fs::read_to_string(&target)
        .map_err(|e| format!("Cannot read file (may be binary): {e}"))?;

    Ok(ToolResult { content, truncated: false })
}

pub fn search_in_files(
    project_root: &Path,
    query: &str,
    file_pattern: Option<&str>,
) -> Result<ToolResult, String> {
    let mut matches: Vec<String> = Vec::new();
    search_recursive(project_root, project_root, query, file_pattern, &mut matches);

    let truncated = matches.len() > MAX_SEARCH_MATCHES;
    if truncated {
        matches.truncate(MAX_SEARCH_MATCHES);
    }

    if matches.is_empty() {
        return Ok(ToolResult { content: "No matches found.".into(), truncated: false });
    }

    Ok(ToolResult {
        content: matches.join("\n"),
        truncated,
    })
}

fn search_recursive(
    root: &Path,
    dir: &Path,
    query: &str,
    pattern: Option<&str>,
    matches: &mut Vec<String>,
) {
    if matches.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };

    for entry in entries.filter_map(|e| e.ok()) {
        if matches.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        let path = entry.path();
        if tree::is_excluded(&path, root) {
            continue;
        }
        if path.is_dir() {
            search_recursive(root, &path, query, pattern, matches);
        } else {
            // Apply file extension filter
            if let Some(pat) = pattern {
                let ext = format!(
                    ".{}",
                    path.extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                );
                let needle = pat.trim_start_matches('*').to_lowercase();
                if !ext.ends_with(&needle) {
                    continue;
                }
            }

            let Ok(content) = fs::read_to_string(&path) else { continue };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy();
            let query_lower = query.to_lowercase();

            for (line_num, line) in content.lines().enumerate() {
                if matches.len() >= MAX_SEARCH_MATCHES {
                    break;
                }
                if line.to_lowercase().contains(&query_lower) {
                    matches.push(format!("{rel}:{}: {line}", line_num + 1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_project() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        let mut f = fs::File::create(dir.path().join("src/main.rs")).unwrap();
        writeln!(f, "fn main() {{ println!(\"hello\"); }}").unwrap();
        let mut g = fs::File::create(dir.path().join("README.md")).unwrap();
        writeln!(g, "# My Project\nhello world").unwrap();
        dir
    }

    #[test]
    fn list_directory_lists_files() {
        let project = make_project();
        let result = list_directory(project.path(), "/").unwrap();
        assert!(result.content.contains("src/"));
        assert!(result.content.contains("README.md"));
    }

    #[test]
    fn list_directory_rejects_escape() {
        let project = make_project();
        let result = list_directory(project.path(), "../../etc");
        assert!(result.is_err());
    }

    #[test]
    fn read_file_returns_content() {
        let project = make_project();
        let result = read_file(project.path(), "README.md").unwrap();
        assert!(result.content.contains("hello world"));
        assert!(!result.truncated);
    }

    #[test]
    fn read_file_rejects_escape() {
        let project = make_project();
        let result = read_file(project.path(), "../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn search_finds_match() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", None).unwrap();
        assert!(result.content.contains("main.rs") || result.content.contains("README.md"));
    }

    #[test]
    fn search_with_pattern_filters() {
        let project = make_project();
        let result = search_in_files(project.path(), "hello", Some(".md")).unwrap();
        assert!(result.content.contains("README.md"));
        assert!(!result.content.contains("main.rs"));
    }
}
```

- [ ] **Step 2: Cargo.toml に tempfile を追加（テスト用）**

確認 `src-tauri/Cargo.toml` 已有 `tempfile`：

```bash
grep "tempfile" /Users/jamesju/Documents/GitHub/AITERM/src-tauri/Cargo.toml
```

若無輸出，在 `[dev-dependencies]` 加入：

```toml
tempfile = "3"
```

- [ ] **Step 3: 執行測試**

```bash
cd src-tauri && cargo test code_assistant::tools 2>&1 | tail -20
```

Expected: `test result: ok. 6 passed`

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/code_assistant/tools.rs src-tauri/Cargo.toml
git commit -m "feat(code-assistant): add file tools (list_directory, read_file, search_in_files)"
```

---

## Task 3：Tool-use 迴圈與事件（mod.rs）

**Files:**
- Create: `src-tauri/src/code_assistant/mod.rs`

- [ ] **Step 1: 建立 mod.rs**

```rust
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
5. Respond in {language}."#
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

        let join = tokio::spawn(async move {
            if force_answer {
                provider_clone.generate(req, tx).await.map(|_| GenerateWithToolsResult::Text(String::new()))
            } else {
                provider_clone.generate_with_tools(req, tools_for_call, tx).await
            }
        });

        // Drain streaming chunks (text arrives here for the final answer)
        let mut text_buf = String::new();
        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                let _ = app.emit("code-assistant-event", CodeAssistantEvent::TextDelta {
                    session_id: session_id.clone(),
                    delta: chunk.delta.clone(),
                });
                text_buf.push_str(&chunk.delta);
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
                // Final text was already streamed above
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
```

> **Note:** `stream_generate` 是輔助函式，可在 `run_fallback` 中使用。`run_chat` 中直接用 spawn 是為了讓 `generate_with_tools` 與 streaming drain 並行。

- [ ] **Step 2: 建立 mod.rs 後確認編譯**

```bash
cd src-tauri && cargo check 2>&1 | grep "error\[" | head -20
```

Expected: 0 errors（此階段模組尚未掛入 lib.rs，可能有 warning 說 module unused，屬正常）

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/code_assistant/mod.rs
git commit -m "feat(code-assistant): add tool-use loop and fallback (mod.rs)"
```

---

## Task 4：Tauri Command + 模組註冊

**Files:**
- Create: `src-tauri/src/commands/code_assistant.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 建立 commands/code_assistant.rs**

```rust
use tauri::{AppHandle, State};
use crate::ai::{router::AiRouter, ChatMessage, Locale, AiError};

#[tauri::command]
pub async fn code_assistant_chat(
    project_root: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
    router: State<'_, AiRouter>,
) -> Result<(), AiError> {
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    if project_root.is_empty() {
        return Err(AiError::InvalidInput { reason: "project_root is empty".into() });
    }

    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    crate::code_assistant::run_chat(project_root, messages, provider, session_id, locale, app).await
}
```

- [ ] **Step 2: 在 commands/mod.rs 加入 module**

在 `src-tauri/src/commands/mod.rs` 末尾加入：

```rust
pub mod code_assistant;
```

- [ ] **Step 3: 在 lib.rs 宣告 code_assistant 模組**

在 `src-tauri/src/lib.rs` 最上方的 `pub mod` 區塊（`pub mod ai;` 之後）加入：

```rust
pub mod code_assistant;
```

- [ ] **Step 4: 在 lib.rs 的 use 區塊加入 import**

在現有 `commands::ai::{agent_chat, ai_chat, ai_query}` 那行旁邊加入：

```rust
commands::code_assistant::code_assistant_chat,
```

- [ ] **Step 5: 在 invoke_handler 加入 command**

在 `lib.rs` 的 `invoke_handler` 區塊，`agent_chat` 之後加入：

```rust
code_assistant_chat,
```

- [ ] **Step 6: 確認編譯**

```bash
cd src-tauri && cargo check 2>&1 | grep "error\[" | head -20
```

Expected: 0 errors

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands/code_assistant.rs \
        src-tauri/src/commands/mod.rs \
        src-tauri/src/lib.rs
git commit -m "feat(code-assistant): register Tauri command code_assistant_chat"
```

---

## Task 5：前端 IPC Wrapper + Hook

**Files:**
- Create: `src/ipc/codeAssistant.ts`
- Create: `src/hooks/useCodeAssistant.ts`

- [ ] **Step 1: 建立 src/ipc/codeAssistant.ts**

```typescript
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";

export type CodeAssistantEvent =
  | { kind: "tool_call";    session_id: string; call_id: string; tool: string; args: Record<string, unknown> }
  | { kind: "tool_result";  session_id: string; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta";   session_id: string; delta: string }
  | { kind: "done";         session_id: string }
  | { kind: "error";        session_id: string; message: string }
  | { kind: "fallback_mode"; session_id: string };

export const CODE_ASSISTANT_EVENT = "code-assistant-event";

export function invokeCodeAssistantChat(
  projectRoot: string,
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string | null,
  locale: string = "zh-TW",
): Promise<void> {
  return invoke<void>("code_assistant_chat", {
    projectRoot,
    messages,
    sessionId,
    providerId: providerId ?? null,
    locale,
  });
}
```

- [ ] **Step 2: 建立 src/hooks/useCodeAssistant.ts**

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { ChatMessage } from "../ipc/ai";
import {
  CODE_ASSISTANT_EVENT,
  invokeCodeAssistantChat,
  type CodeAssistantEvent,
} from "../ipc/codeAssistant";
import { useLocale } from "../contexts/LocaleContext";

export interface ToolCallState {
  callId: string;
  tool: string;
  args: Record<string, unknown>;
  result?: { content: string; truncated: boolean };
}

export interface CodeMessage {
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCallState[];
  streaming?: boolean;
}

export interface UseCodeAssistantResult {
  messages: CodeMessage[];
  isStreaming: boolean;
  error: string | null;
  isFallbackMode: boolean;
  send: (userText: string, projectRoot: string, providerId?: string) => Promise<void>;
  clear: () => void;
}

export function useCodeAssistant(): UseCodeAssistantResult {
  const [messages, setMessages] = useState<CodeMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFallbackMode, setIsFallbackMode] = useState(false);
  const mountedRef = useRef(true);
  const { locale } = useLocale();

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const send = useCallback(async (
    userText: string,
    projectRoot: string,
    providerId?: string,
  ) => {
    if (!userText.trim() || isStreaming) return;
    setError(null);

    // Build chat history for backend (only role+content, no toolCalls UI state)
    const chatMessages: ChatMessage[] = [
      ...messages.map((m) => ({ role: m.role, content: m.content })),
      { role: "user" as const, content: userText },
    ];

    setMessages((prev) => [
      ...prev,
      { role: "user", content: userText },
      { role: "assistant", content: "", toolCalls: [], streaming: true },
    ]);
    setIsStreaming(true);

    const sessionId = crypto.randomUUID();

    const unlisten = await listen<CodeAssistantEvent>(CODE_ASSISTANT_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.session_id !== sessionId) return; // ignore other sessions

      if (p.kind === "tool_call") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = [
            ...(last.toolCalls ?? []),
            { callId: p.call_id, tool: p.tool, args: p.args },
          ];
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "tool_result") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = (last.toolCalls ?? []).map((tc) =>
            tc.callId === p.call_id
              ? { ...tc, result: { content: p.content, truncated: p.truncated } }
              : tc,
          );
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "text_delta") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.content = (last.content ?? "") + p.delta;
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "fallback_mode") {
        setIsFallbackMode(true);
      } else if (p.kind === "done") {
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        setIsStreaming(false);
        unlisten();
      } else if (p.kind === "error") {
        setError(p.message);
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        unlisten();
      }
    });

    try {
      await invokeCodeAssistantChat(projectRoot, chatMessages, sessionId, providerId, locale);
    } catch (e) {
      if (mountedRef.current) {
        setError(String(e));
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        unlisten();
      }
    }
  }, [messages, isStreaming, locale]);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
  }, []);

  return { messages, isStreaming, error, isFallbackMode, send, clear };
}
```

- [ ] **Step 3: TypeScript 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -30
```

Expected: 0 errors

- [ ] **Step 4: Commit**

```bash
git add src/ipc/codeAssistant.ts src/hooks/useCodeAssistant.ts
git commit -m "feat(code-assistant): add IPC wrapper and useCodeAssistant hook"
```

---

## Task 6：ToolCallCard 元件

**Files:**
- Create: `src/components/CodeAssistantView/ToolCallCard.tsx`
- Create: `src/components/CodeAssistantView/styles.css`

- [ ] **Step 1: 建立 ToolCallCard.tsx**

```tsx
import { useState } from "react";
import type { ToolCallState } from "../../hooks/useCodeAssistant";

interface ToolCallCardProps {
  toolCall: ToolCallState;
}

function formatArgs(args: Record<string, unknown>): string {
  const entries = Object.entries(args);
  if (entries.length === 0) return "";
  if (entries.length === 1) return String(entries[0][1]);
  return entries.map(([k, v]) => `${k}=${String(v)}`).join(", ");
}

export function ToolCallCard({ toolCall }: ToolCallCardProps) {
  const [expanded, setExpanded] = useState(false);
  const isDone = toolCall.result !== undefined;
  const isError = isDone && toolCall.result!.content.startsWith("Error:");

  return (
    <div className={`ca-tool-card ${isDone ? "ca-tool-card--done" : "ca-tool-card--loading"} ${isError ? "ca-tool-card--error" : ""}`}>
      <button
        className="ca-tool-card__header"
        onClick={() => isDone && setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        <span className="ca-tool-card__status">
          {!isDone && <span className="ca-tool-card__spinner" />}
          {isDone && !isError && "✓"}
          {isError && "✗"}
        </span>
        <span className="ca-tool-card__name">{toolCall.tool}</span>
        <span className="ca-tool-card__args">{formatArgs(toolCall.args)}</span>
        {isDone && (
          <span className="ca-tool-card__toggle">{expanded ? "▲" : "▼"}</span>
        )}
      </button>
      {expanded && isDone && (
        <div className="ca-tool-card__content">
          {toolCall.result!.truncated && (
            <div className="ca-tool-card__truncated">⚠ 內容已截斷</div>
          )}
          <pre className="ca-tool-card__pre">{toolCall.result!.content}</pre>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 建立 styles.css**

```css
/* ── CodeAssistantView ─────────────────────────────────────────── */
.ca-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: var(--bg-primary, #1a1a1a);
  color: var(--text-primary, #e0e0e0);
  font-family: inherit;
}

/* Header bar */
.ca-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border, #333);
  background: var(--bg-secondary, #242424);
  flex-shrink: 0;
}
.ca-header__path {
  flex: 1;
  font-size: 12px;
  color: var(--text-secondary, #aaa);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ca-header__btn {
  font-size: 11px;
  padding: 3px 8px;
  background: transparent;
  border: 1px solid var(--border, #444);
  border-radius: 4px;
  color: var(--text-secondary, #aaa);
  cursor: pointer;
}
.ca-header__btn:hover { background: var(--bg-hover, #333); }

/* Fallback banner */
.ca-fallback-banner {
  padding: 6px 12px;
  background: #3a2e00;
  color: #f5c518;
  font-size: 11px;
  border-bottom: 1px solid #5a4800;
  flex-shrink: 0;
}

/* Directory change confirmation bar */
.ca-dir-confirm {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  background: var(--bg-secondary, #242424);
  border-bottom: 1px solid var(--border, #333);
  font-size: 12px;
  flex-shrink: 0;
}
.ca-dir-confirm__path {
  flex: 1;
  color: var(--text-secondary, #aaa);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* Empty state (no project selected) */
.ca-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 16px;
  color: var(--text-secondary, #aaa);
}
.ca-empty__icon { font-size: 48px; opacity: 0.4; }
.ca-empty__title { font-size: 16px; font-weight: 500; }
.ca-empty__desc { font-size: 13px; opacity: 0.7; }
.ca-empty__btn {
  padding: 8px 20px;
  background: var(--accent, #4d8eff);
  color: #fff;
  border: none;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
}
.ca-empty__btn:hover { opacity: 0.85; }

/* Messages area */
.ca-messages {
  flex: 1;
  overflow-y: auto;
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* Input bar */
.ca-input-bar {
  display: flex;
  gap: 8px;
  padding: 10px 12px;
  border-top: 1px solid var(--border, #333);
  background: var(--bg-secondary, #242424);
  flex-shrink: 0;
}
.ca-input-bar__textarea {
  flex: 1;
  background: var(--bg-primary, #1a1a1a);
  border: 1px solid var(--border, #444);
  border-radius: 6px;
  color: var(--text-primary, #e0e0e0);
  font-size: 13px;
  padding: 6px 8px;
  resize: none;
  min-height: 36px;
  max-height: 120px;
  font-family: inherit;
}
.ca-input-bar__textarea:focus { outline: none; border-color: var(--accent, #4d8eff); }
.ca-input-bar__send {
  padding: 6px 14px;
  background: var(--accent, #4d8eff);
  color: #fff;
  border: none;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
  align-self: flex-end;
}
.ca-input-bar__send:disabled { opacity: 0.4; cursor: default; }
.ca-input-bar__clear {
  padding: 6px 10px;
  background: transparent;
  border: 1px solid var(--border, #444);
  border-radius: 6px;
  color: var(--text-secondary, #aaa);
  font-size: 13px;
  cursor: pointer;
  align-self: flex-end;
}

/* ── ToolCallCard ───────────────────────────────────────────────── */
.ca-tool-card {
  border: 1px solid var(--border, #333);
  border-radius: 6px;
  overflow: hidden;
  font-size: 12px;
  margin: 4px 0;
}
.ca-tool-card--loading { border-color: #555; }
.ca-tool-card--done    { border-color: #2a4a2a; }
.ca-tool-card--error   { border-color: #4a2a2a; }

.ca-tool-card__header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  background: var(--bg-secondary, #242424);
  border: none;
  width: 100%;
  text-align: left;
  color: var(--text-secondary, #aaa);
  cursor: default;
}
.ca-tool-card--done .ca-tool-card__header { cursor: pointer; }
.ca-tool-card__header:hover { background: var(--bg-hover, #2e2e2e); }

.ca-tool-card__status { width: 14px; text-align: center; flex-shrink: 0; }
.ca-tool-card__name   { font-weight: 500; color: #7ec8e3; }
.ca-tool-card__args   { flex: 1; color: #aaa; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.ca-tool-card__toggle { margin-left: auto; flex-shrink: 0; }

.ca-tool-card__spinner {
  display: inline-block;
  width: 10px; height: 10px;
  border: 2px solid #555;
  border-top-color: #aaa;
  border-radius: 50%;
  animation: ca-spin 0.6s linear infinite;
}
@keyframes ca-spin { to { transform: rotate(360deg); } }

.ca-tool-card__content {
  border-top: 1px solid var(--border, #333);
  max-height: 300px;
  overflow: auto;
}
.ca-tool-card__truncated {
  padding: 4px 8px;
  background: #3a2e00;
  color: #f5c518;
  font-size: 11px;
}
.ca-tool-card__pre {
  margin: 0;
  padding: 8px;
  font-size: 11px;
  font-family: monospace;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text-secondary, #ccc);
}

@media (prefers-color-scheme: light) {
  .ca-view        { --bg-primary: #f5f5f5; --bg-secondary: #efefef; --border: #ddd; --text-primary: #1a1a1a; --text-secondary: #555; --bg-hover: #e5e5e5; }
  .ca-tool-card--done { border-color: #c3e6c3; }
  .ca-tool-card__name { color: #0066aa; }
}
```

- [ ] **Step 3: Commit**

```bash
git add src/components/CodeAssistantView/ToolCallCard.tsx \
        src/components/CodeAssistantView/styles.css
git commit -m "feat(code-assistant): add ToolCallCard component and styles"
```

---

## Task 7：CodeAssistantView 主元件

**Files:**
- Create: `src/components/CodeAssistantView/index.tsx`

- [ ] **Step 1: 建立 index.tsx**

```tsx
import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { pickFolder } from "../../ipc/vcs";
import { useCodeAssistant } from "../../hooks/useCodeAssistant";
import { ToolCallCard } from "./ToolCallCard";
import { MarkdownText } from "../../lib/markdown";
import { StreamingIndicator } from "../StreamingIndicator";
import { useLocale } from "../../contexts/LocaleContext";
import "./styles.css";

const STORAGE_KEY = "aiterm-code-assistant-root";

function loadSavedRoot(): string {
  try { return localStorage.getItem(STORAGE_KEY) ?? ""; } catch { return ""; }
}
function saveRoot(path: string) {
  try { localStorage.setItem(STORAGE_KEY, path); } catch { /* ignore */ }
}

interface Props {
  isActive: boolean;
  providerId?: string;
}

export function CodeAssistantView({ isActive, providerId }: Props) {
  const { t } = useLocale();
  const [projectRoot, setProjectRoot] = useState(loadSavedRoot);
  const [pendingRoot, setPendingRoot] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const { messages, isStreaming, error, isFallbackMode, send, clear } = useCodeAssistant();

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    if (isActive) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages, isActive]);

  const handlePickFolder = useCallback(async () => {
    const folder = await pickFolder();
    if (!folder) return;

    if (messages.length > 0) {
      // Ask user: continue or new chat
      setPendingRoot(folder);
    } else {
      setProjectRoot(folder);
      saveRoot(folder);
    }
  }, [messages.length]);

  const handleConfirmDir = useCallback((newChat: boolean) => {
    if (!pendingRoot) return;
    setProjectRoot(pendingRoot);
    saveRoot(pendingRoot);
    setPendingRoot(null);
    if (newChat) clear();
  }, [pendingRoot, clear]);

  const handleSend = useCallback(() => {
    if (!input.trim() || isStreaming || !projectRoot) return;
    const text = input;
    setInput("");
    void send(text, projectRoot, providerId);
  }, [input, isStreaming, projectRoot, providerId, send]);

  const handleKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }, [handleSend]);

  if (!projectRoot) {
    return (
      <div className="ca-view">
        <div className="ca-empty">
          <div className="ca-empty__icon">📂</div>
          <div className="ca-empty__title">選擇專案目錄</div>
          <div className="ca-empty__desc">選定目錄後即可對程式庫提問</div>
          <button className="ca-empty__btn" onClick={handlePickFolder}>
            選擇目錄
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="ca-view">
      {/* Header */}
      <div className="ca-header">
        <span className="ca-header__path" title={projectRoot}>📁 {projectRoot}</span>
        <button className="ca-header__btn" onClick={handlePickFolder}>更換目錄</button>
        <button className="ca-header__btn" onClick={clear}>清除</button>
      </div>

      {/* Directory change confirmation bar */}
      {pendingRoot && (
        <div className="ca-dir-confirm">
          <span className="ca-dir-confirm__path">已選擇 {pendingRoot}</span>
          <button className="ca-header__btn" onClick={() => handleConfirmDir(false)}>
            繼續對話
          </button>
          <button className="ca-header__btn" onClick={() => handleConfirmDir(true)}>
            開新對話
          </button>
          <button className="ca-header__btn" onClick={() => setPendingRoot(null)}>
            取消
          </button>
        </div>
      )}

      {/* Fallback mode banner */}
      {isFallbackMode && (
        <div className="ca-fallback-banner">
          ⚠ 此 provider 不支援工具調用，已切換為兩段式模式
        </div>
      )}

      {/* Messages */}
      <div className="ca-messages">
        {messages.map((msg, i) => (
          <div key={i} className={`ca-msg ca-msg--${msg.role}`}>
            {/* Tool call cards (assistant only) */}
            {msg.role === "assistant" && (msg.toolCalls ?? []).map((tc) => (
              <ToolCallCard key={tc.callId} toolCall={tc} />
            ))}
            {/* Message text */}
            {msg.content && (
              <div className="ca-msg__bubble">
                {msg.role === "assistant" ? (
                  <MarkdownText text={msg.content} />
                ) : (
                  msg.content
                )}
                {msg.streaming && <StreamingIndicator />}
              </div>
            )}
          </div>
        ))}
        {error && <div className="ca-error">{error}</div>}
        <div ref={messagesEndRef} />
      </div>

      {/* Input bar */}
      <div className="ca-input-bar">
        <textarea
          ref={textareaRef}
          className="ca-input-bar__textarea"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="問關於這個專案的任何問題... (Enter 送出，Shift+Enter 換行)"
          rows={1}
          disabled={isStreaming}
        />
        <button
          className="ca-input-bar__send"
          onClick={handleSend}
          disabled={isStreaming || !input.trim()}
        >
          {isStreaming ? "..." : "送出"}
        </button>
        <button className="ca-input-bar__clear" onClick={clear}>清除</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: 在 styles.css 補上 .ca-msg 樣式**（在 Task 6 建立的 styles.css 末尾加入）

```css
/* Message bubbles */
.ca-msg { display: flex; flex-direction: column; gap: 4px; }
.ca-msg--user { align-items: flex-end; }
.ca-msg--assistant { align-items: flex-start; }

.ca-msg__bubble {
  max-width: 85%;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
  line-height: 1.5;
}
.ca-msg--user .ca-msg__bubble {
  background: var(--accent, #4d8eff);
  color: #fff;
  border-bottom-right-radius: 2px;
}
.ca-msg--assistant .ca-msg__bubble {
  background: var(--bg-secondary, #242424);
  border: 1px solid var(--border, #333);
  border-bottom-left-radius: 2px;
}

.ca-error {
  color: #f87171;
  font-size: 12px;
  padding: 6px 8px;
  background: #2a1a1a;
  border-radius: 4px;
  border: 1px solid #4a2a2a;
}
```

- [ ] **Step 3: TypeScript 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -30
```

Expected: 0 errors

- [ ] **Step 4: Commit**

```bash
git add src/components/CodeAssistantView/index.tsx \
        src/components/CodeAssistantView/styles.css
git commit -m "feat(code-assistant): add CodeAssistantView main component"
```

---

## Task 8：Tab 整合（TabType、NewTabPicker、TerminalApp、i18n、Icon）

**Files:**
- Modify: `src/components/Icons.tsx`
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/NewTabPicker/index.tsx`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: 在 Icons.tsx 加入 CodeIcon**

在 `src/components/Icons.tsx` 末尾（`SmartphoneIcon` 之後）加入：

```tsx
export function CodeIcon({ size = 16, ...props }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" {...props}>
      <polyline points="5 4 1 8 5 12" />
      <polyline points="11 4 15 8 11 12" />
    </svg>
  );
}
```

- [ ] **Step 2: 在 i18n.ts 加入翻譯字串（中文區塊，loop_studio_tab 之後）**

```typescript
code_assistant_tab: "程式庫協助",
new_code_assistant_desc: "以 AI 對任意專案目錄提問",
```

在英文區塊對應位置加入：

```typescript
code_assistant_tab: "Code Assistant",
new_code_assistant_desc: "Ask AI about any project directory",
```

- [ ] **Step 3: 在 TabBar/index.tsx 的 TabType 加入 "code-assistant"**

找到第 19 行：

```typescript
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio";
```

改為：

```typescript
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant";
```

- [ ] **Step 4: 在 NewTabPicker/index.tsx 加入 import 和 items 項目**

在 import 區塊加入 `CodeIcon`：

```typescript
import {
  // 現有 imports...
  CodeIcon,
} from "../Icons";
```

在 `items` 陣列末尾（`loop-studio` 之後）加入：

```typescript
{ type: "code-assistant", icon: <CodeIcon size={18} />, label: t.code_assistant_tab, desc: t.new_code_assistant_desc },
```

- [ ] **Step 5: 在 TerminalApp.tsx 加入 import 和渲染分支**

在 import 區塊加入：

```typescript
import { CodeAssistantView } from "./CodeAssistantView";
```

在 `handlePickerSelect` 的 type union 加入 `| "code-assistant"`：

```typescript
const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant") => {
```

在 title 判斷區塊加入（`loop-studio` 之後）：

```typescript
if (type === "code-assistant") title = t.code_assistant_tab;
```

在渲染區塊（`loop-studio` 的 `LoopStudioView` 之後，在 `else` 之前）加入：

```typescript
) : tab.type === "code-assistant" ? (
  <CodeAssistantView isActive={isActive} />
```

- [ ] **Step 6: TypeScript 型別檢查**

```bash
npx tsc --noEmit 2>&1 | head -30
```

Expected: 0 errors

- [ ] **Step 7: Rust 最終編譯確認**

```bash
cd src-tauri && cargo check 2>&1 | grep "error\[" | head -10
```

Expected: 0 errors

- [ ] **Step 8: Frontend 測試**

```bash
npm run test 2>&1 | tail -20
```

Expected: 已有測試全數通過，無新失敗

- [ ] **Step 9: Commit**

```bash
git add src/components/Icons.tsx \
        src/lib/i18n.ts \
        src/components/TabBar/index.tsx \
        src/components/NewTabPicker/index.tsx \
        src/components/TerminalApp.tsx
git commit -m "feat(code-assistant): integrate Code Assistant tab into UI"
```

---

## 完成後驗證

```bash
# 啟動 dev server 手動測試
npm run tauri:dev
```

測試清單：
- [ ] 點 + 新增 Tab，可看到「程式庫協助」選項
- [ ] 進入後顯示「選擇目錄」空狀態
- [ ] 選擇目錄後顯示路徑列
- [ ] 輸入問題後可看到 ToolCallCard 出現（工具呼叫進度）
- [ ] AI 最終文字以 streaming 方式顯示
- [ ] 點擊 ToolCallCard 可展開看回傳內容
- [ ] 「更換目錄」後出現「繼續對話 / 開新對話」確認列
- [ ] 「清除」清空對話，保留目錄
- [ ] 多輪問答正常保留上下文

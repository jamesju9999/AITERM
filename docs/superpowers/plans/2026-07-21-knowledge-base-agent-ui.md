# 知識庫（Knowledge Base）Agent 問答與前端 UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Plan A 完成的儲存/擷取管線之上，建立 Agent 問答迴圈（`search_documents`/`read_document` 工具 + tool-calling loop）與完整前端 UI（筆記本清單、同步進度、對話介面),讓使用者可以實際在「知識庫」分頁對指定資料夾提問並得到附出處引用的回答。

**Architecture:** 後端刻意**複製並改編** `code_assistant/mod.rs` 的 agent loop 結構（checkpoint compression、XML tool call 解析、dedup、streaming、fallback），而不是抽出共用抽象——因為那份程式碼已經過多輪除錯驗證,貿然重構抽象層風險大於重複約 300 行程式碼的成本（YAGNI：目前只有兩個消費者，不需要為假設的第三個消費者预先抽象)。前端同樣重用 Code Assistant 已驗證的元件（`ToolCallCard`、`MarkdownText`）與 hook 模式（`useCodeAssistant.ts` 的事件監聽邏輯)。

**Tech Stack:** Rust（`async_trait`、既有 `ai::router::AiRouter`）、React 19 + TypeScript（沿用既有 `TabBar`/`NewTabPicker` 分頁機制）。不新增任何依賴。

**前置條件：** Plan A（`docs/superpowers/plans/2026-07-21-knowledge-base-backend-core.md`）必須已完成並通過測試——本 plan 直接使用 Plan A 產出的 `db::knowledge_base`、`knowledge_base::embedding`、`kb_create_notebook`/`kb_list_notebooks`/`kb_delete_notebook`/`kb_sync_notebook` commands。

---

## 參考檔案（實作前请先讀過)

- `src-tauri/src/code_assistant/mod.rs` — agent loop 完整範本（`run_chat`、`run_fallback`、checkpoint compression、XML 解析、dedup）
- `src-tauri/src/commands/code_assistant.rs` — Tauri command 包裝範本
- `src-tauri/src/ai/mod.rs` — `AiProvider` trait、`ChatMessage`、`GenerateWithToolsResult`、`McpToolDefinition`
- `src-tauri/src/ai/router.rs` — `AiRouter::resolve()` / `resolve_by_id()`
- `src/hooks/useCodeAssistant.ts` — chat hook 範本（事件監聽、streaming、fallback）
- `src/components/CodeAssistantView/index.tsx`、`ToolCallCard.tsx` — 前端 UI 範本（直接重用 `ToolCallCard`)
- `src/components/TabBar/index.tsx`、`src/components/NewTabPicker/index.tsx`、`src/components/TerminalApp.tsx` — 分頁註冊三處
- `src/components/Icons.tsx` — icon 元件範本

---

## Task 1:`search_documents` / `read_document` 工具

**Files:**
- Create: `src-tauri/src/knowledge_base/tools.rs`
- Modify: `src-tauri/src/knowledge_base/mod.rs`
- Modify: `src-tauri/src/db/knowledge_base.rs`（新增 `get_document_by_path`）
- Test: `src-tauri/tests/knowledge_base_tools.rs`

- [ ] **Step 1: 在 `db/knowledge_base.rs` 加入依路徑查單一文件的函式**

```rust
// 加在 src-tauri/src/db/knowledge_base.rs 檔案末尾
pub async fn get_document_by_path(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
) -> Result<Option<DocumentRow>, sqlx::Error> {
    sqlx::query_as::<_, DocumentRow>(
        "SELECT id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message
         FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await
}
```

- [ ] **Step 2: 建立空殼模組**

```rust
// src-tauri/src/knowledge_base/tools.rs
// (空檔案,下一步驟才寫實作)
```

```rust
// src-tauri/src/knowledge_base/mod.rs — 加入
pub mod tools;
```

- [ ] **Step 3: 寫失敗測試**

```rust
// src-tauri/tests/knowledge_base_tools.rs
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;

use aiterm_lib::db::knowledge_base::{create_notebook, upsert_document, replace_chunks};
use aiterm_lib::knowledge_base::embedding::Embedder;
use aiterm_lib::knowledge_base::tools::{dispatch_tool, tool_definitions};

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE notebooks (
            id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, folder_path TEXT NOT NULL,
            embed_provider_id TEXT, embed_model TEXT, embed_dim INTEGER,
            last_synced_at INTEGER, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY NOT NULL, notebook_id TEXT NOT NULL, rel_path TEXT NOT NULL,
            mtime INTEGER NOT NULL, content_hash TEXT NOT NULL, markdown_cache TEXT,
            status TEXT NOT NULL DEFAULT 'ok', error_message TEXT,
            UNIQUE(notebook_id, rel_path)
        )"
    ).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE chunks (
            id TEXT PRIMARY KEY NOT NULL, document_id TEXT NOT NULL, chunk_index INTEGER NOT NULL,
            text TEXT NOT NULL, location_hint TEXT, embedding BLOB NOT NULL
        )"
    ).execute(&pool).await.unwrap();
    pool
}

struct FakeEmbedder;
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        // 回傳與輸入文字內容無關的固定向量，測試只關心 dispatch 邏輯正確串接
        Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
    }
}

#[test]
fn tool_definitions_include_search_and_read() {
    let defs = tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"search_documents"));
    assert!(names.contains(&"read_document"));
}

#[tokio::test]
async fn search_documents_returns_formatted_hits_with_citation_info() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None).await.unwrap();
    let doc_id = upsert_document(
        &pool, &notebook.id, "report.pdf", 0, "hash1", Some("# Report\n\ncontent"), "ok", None,
    ).await.unwrap();
    replace_chunks(&pool, &doc_id, &[
        ("重要段落內容".into(), Some("第一章".into()), vec![1.0, 0.0, 0.0]),
    ]).await.unwrap();

    let (content, truncated) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "search_documents", &serde_json::json!({ "query": "重要內容是什麼" }),
    ).await;

    assert!(!truncated);
    assert!(content.contains("report.pdf"), "result should cite the source file: {content}");
    assert!(content.contains("第一章"), "result should include the location hint: {content}");
    assert!(content.contains("重要段落內容"));
}

#[tokio::test]
async fn search_documents_with_empty_query_returns_error() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None).await.unwrap();
    let (content, _) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "search_documents", &serde_json::json!({ "query": "" }),
    ).await;
    assert!(content.starts_with("Error:"));
}

#[tokio::test]
async fn read_document_returns_full_markdown_content() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None).await.unwrap();
    upsert_document(
        &pool, &notebook.id, "notes.md", 0, "hash1", Some("# Notes\n\nfull content here"), "ok", None,
    ).await.unwrap();

    let (content, truncated) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "read_document", &serde_json::json!({ "path": "notes.md" }),
    ).await;

    assert!(!truncated);
    assert_eq!(content, "# Notes\n\nfull content here");
}

#[tokio::test]
async fn read_document_missing_path_returns_error() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None).await.unwrap();
    let (content, _) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "read_document", &serde_json::json!({ "path": "does-not-exist.md" }),
    ).await;
    assert!(content.starts_with("Error:"));
}
```

- [ ] **Step 4: 執行測試,確認因函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --test knowledge_base_tools -- --nocapture`
Expected: 編譯錯誤（找不到 `dispatch_tool`、`tool_definitions`）

- [ ] **Step 5: 實作工具定義與 dispatch**

```rust
// src-tauri/src/knowledge_base/tools.rs
use sqlx::SqlitePool;
use crate::db::knowledge_base;
use crate::knowledge_base::embedding::Embedder;
use crate::ai::McpToolDefinition;

const MAX_READ_DOCUMENT_BYTES: usize = 100 * 1024;

pub fn tool_definitions() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition {
            name: "search_documents".into(),
            description: "Semantic search over the notebook's indexed documents. Returns the most relevant text chunks, each tagged with its source file path, location hint, and similarity score. This is your primary tool — call it first for any question.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural-language description of what you're looking for — not just keywords." },
                    "top_k": { "type": "integer", "description": "Number of results to return (default 8, max 20)." }
                },
                "required": ["query"]
            }),
        },
        McpToolDefinition {
            name: "read_document".into(),
            description: "Read a document's full converted content by its exact path (as shown in search_documents results). Use when a single chunk doesn't give enough context.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Document rel_path exactly as returned by search_documents." }
                },
                "required": ["path"]
            }),
        },
    ]
}

/// 找不到剛好落在 max_bytes 的 UTF-8 字元邊界時往前找最近的合法邊界，
/// 避免在多位元組字元（中文）中間切斷造成 panic。
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub async fn dispatch_tool(
    pool: &SqlitePool,
    notebook_id: &str,
    embedder: &dyn Embedder,
    name: &str,
    args: &serde_json::Value,
) -> (String, bool) {
    match name {
        "search_documents" => {
            let query = args["query"].as_str().unwrap_or("").trim().to_owned();
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(8).clamp(1, 20) as usize;

            if query.is_empty() {
                return ("Error: query is empty".into(), false);
            }

            let mut vectors = match embedder.embed(&[query]).await {
                Ok(v) => v,
                Err(e) => return (format!("Error: {e}"), false),
            };
            let query_embedding = match vectors.pop() {
                Some(v) => v,
                None => return ("Error: embedding provider returned no vector".into(), false),
            };

            match knowledge_base::search_similar_chunks(pool, notebook_id, &query_embedding, top_k).await {
                Ok(hits) if hits.is_empty() => ("No matching content found.".into(), false),
                Ok(hits) => {
                    let formatted = hits.iter().enumerate().map(|(i, h)| {
                        let loc = h.location_hint.as_deref().unwrap_or("(no section title)");
                        format!(
                            "[{}] {} — {} (score {:.2})\n{}",
                            i + 1, h.rel_path, loc, h.score, h.text
                        )
                    }).collect::<Vec<_>>().join("\n\n---\n\n");
                    (formatted, false)
                }
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        "read_document" => {
            let path = args["path"].as_str().unwrap_or("").to_owned();
            match knowledge_base::get_document_by_path(pool, notebook_id, &path).await {
                Ok(Some(doc)) if doc.status == "ok" => {
                    let content = doc.markdown_cache.unwrap_or_default();
                    let truncated = content.len() > MAX_READ_DOCUMENT_BYTES;
                    let content = if truncated {
                        format!(
                            "{}\n\n[TRUNCATED: document exceeds size limit]",
                            safe_truncate(&content, MAX_READ_DOCUMENT_BYTES)
                        )
                    } else {
                        content
                    };
                    (content, truncated)
                }
                Ok(Some(doc)) => (
                    format!("Error: document has status '{}': {}", doc.status, doc.error_message.unwrap_or_default()),
                    false,
                ),
                Ok(None) => (format!("Error: no document found at path '{path}' in this notebook"), false),
                Err(e) => (format!("Error: {e}"), false),
            }
        }
        _ => (format!("Unknown tool: {name}"), false),
    }
}
```

- [ ] **Step 6: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --test knowledge_base_tools -- --nocapture`
Expected: PASS（5 passed）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/knowledge_base/tools.rs src-tauri/src/knowledge_base/mod.rs src-tauri/src/db/knowledge_base.rs src-tauri/tests/knowledge_base_tools.rs
git commit -m "feat(knowledge-base): add search_documents and read_document agent tools"
```

---

## Task 2: Chat Agent Loop（改編自 `code_assistant::run_chat`）

**Files:**
- Create: `src-tauri/src/knowledge_base/chat.rs`
- Modify: `src-tauri/src/knowledge_base/mod.rs`

此 task 刻意大量比照 `src-tauri/src/code_assistant/mod.rs` 的既有邏輯（checkpoint compression、XML tool call 解析、dedup、streaming），只替換工具集合、system prompt 與資料來源（`project_root: PathBuf` → `pool: SqlitePool` + `notebook_id`,並多帶一個 `embedder: Arc<dyn Embedder>` 給 `search_documents` 用）。單元測試只涵蓋純字串處理的輔助函式（與 code_assistant 對應測試相同斷言，因為邏輯逐字複製）。

- [ ] **Step 1: 建立空殼模組**

```rust
// src-tauri/src/knowledge_base/chat.rs
// (空檔案,下一步驟才寫實作)
```

```rust
// src-tauri/src/knowledge_base/mod.rs — 加入
pub mod chat;
```

- [ ] **Step 2: 寫失敗測試（純字串處理函式，邏輯與 code_assistant 對應測試相同）**

```rust
// 加在 src-tauri/src/knowledge_base/chat.rs 檔案末尾（下一步驟會先寫主體邏輯）
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_xml_json_format() {
        let text = r#"<tool_call>{"name":"search_documents","arguments":{"query":"pricing","top_k":5}}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "search_documents");
        assert_eq!(calls[0].1["query"], "pricing");
        assert_eq!(calls[0].1["top_k"], 5);
    }

    #[test]
    fn parse_xml_attribute_format() {
        let text = "<function=read_document> <parameter=path> notes.md </parameter> </function>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "read_document");
        assert_eq!(calls[0].1["path"], "notes.md");
    }

    #[test]
    fn parse_xml_no_match_returns_empty() {
        let calls = parse_xml_tool_calls("Here is my answer: the answer is 42.");
        assert!(calls.is_empty());
    }

    #[test]
    fn dedup_key_is_stable() {
        let args = serde_json::json!({"query": "pricing", "top_k": 5});
        let k1 = tool_call_key("search_documents", &args);
        let k2 = tool_call_key("search_documents", &args);
        assert_eq!(k1, k2);
    }
}
```

- [ ] **Step 3: 執行測試,確認因函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --lib knowledge_base::chat -- --nocapture`
Expected: 編譯錯誤（找不到 `parse_xml_tool_calls`、`tool_call_key`）

- [ ] **Step 4: 實作 chat agent loop（加在測試模組之前）**

```rust
// src-tauri/src/knowledge_base/chat.rs — 加在檔案最前面（#[cfg(test)] mod tests 之前）
use std::sync::Arc;
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest,
    GenerateWithToolsResult, Locale, QueryMode,
};
use crate::db::knowledge_base::NotebookRow;
use crate::knowledge_base::embedding::Embedder;
use crate::knowledge_base::tools::{dispatch_tool, tool_definitions};

const MAX_TOOL_ROUNDS: usize = 20;
const TOKEN_ESTIMATE_LIMIT: usize = 50_000;
const CHECKPOINT_THRESHOLD: usize = 30_000;
const MAX_CHECKPOINTS: usize = 2;

const KB_CHAT_EVENT: &str = "kb-chat-event";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KbChatEvent {
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
    TokenCount {
        session_id: String,
        count: usize,
        limit: usize,
    },
}

fn build_system_prompt(notebook_name: &str, locale: Locale) -> String {
    let language = crate::ai::language_name(locale);
    format!(
r#"You are a research assistant answering questions strictly from the documents in the notebook "{notebook_name}".

## Tools

- search_documents(query, top_k?): semantic search over indexed document chunks. Returns the most relevant chunks with source file path, location hint, and a similarity score. This is your primary tool — call it first for any question.
- read_document(path): read a document's full converted content by its exact path (as shown in search_documents results). Use when a single chunk doesn't give enough context.

## Search Strategy

1. Call search_documents with a natural-language description of what you need — not just keywords.
2. If the returned chunks don't fully answer the question, call read_document on the most promising source for full context.
3. If the first search doesn't find what you need, try search_documents again with different phrasing before giving up.
4. Answer once you have enough verified content.

## Accuracy — Non-Negotiable

- EVERY factual claim must come from a chunk returned by search_documents or read_document in THIS session — never from general knowledge or inference.
- ALWAYS cite your source after each claim using the exact rel_path and location_hint returned by the tools, e.g. (report.pdf, 第一章).
- NEVER cite a file you have not actually retrieved content from via search_documents or read_document this session.
- If the documents don't contain an answer, say so explicitly — do not guess or fill gaps with outside knowledge.
- Do not fabricate document names, section titles, or quotes.

- Respond in {language}.
- **Mermaid diagrams**: node IDs must be plain ASCII identifiers. Wrap every node label and edge label in double quotes. Do not use `<br/>` inside labels — use a space instead."#
    )
}

fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
}

/// Build a deduplication key for a tool call.
fn tool_call_key(tool_name: &str, args: &serde_json::Value) -> String {
    format!("{}:{}", tool_name, args)
}

/// Parse tool calls from XML text that local models emit instead of proper JSON tool-calls.
/// Identical logic to `code_assistant::parse_xml_tool_calls` — duplicated rather than shared
/// to keep the two agent loops independently evolvable (see Plan B Architecture note).
fn parse_xml_tool_calls(text: &str) -> Vec<(String, serde_json::Value)> {
    let mut results = Vec::new();

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
        let skip = start + "<tool_call>".len();
        search = &search[skip..];
    }

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
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    let tool_defs = tool_definitions();
    let system_prompt = build_system_prompt(&notebook.name, locale);

    let mut conversation = messages;
    let mut token_estimate = estimate_tokens(&system_prompt);
    let mut rounds = 0usize;
    let mut checkpoints = 0usize;
    let mut seen_calls: std::collections::HashSet<String> = std::collections::HashSet::new();

    loop {
        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TokenCount {
            session_id: session_id.clone(),
            count: token_estimate,
            limit: TOKEN_ESTIMATE_LIMIT,
        });

        if token_estimate >= CHECKPOINT_THRESHOLD && checkpoints < MAX_CHECKPOINTS {
            checkpoints += 1;
            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                session_id: session_id.clone(),
                delta: format!(
                    "\n\n> [Checkpoint #{checkpoints}：正在壓縮已蒐集的資料，繼續探索...]\n\n"
                ),
            });
            let summary = generate_checkpoint_summary(&conversation, chat_provider.clone(), locale).await;
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
                 1. Natural language ONLY — absolutely NO JSON, NO arrays.\n\
                 2. Only state facts you directly retrieved via search_documents/read_document.\n\
                 3. If you did not find something, explicitly say so.\n\
                 4. Summarise your findings in clear prose, with citations."
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
        let provider_clone = chat_provider.clone();
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

        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                    session_id: session_id.clone(),
                    delta: chunk.delta.clone(),
                });
            }
            if chunk.done { break; }
        }

        match join.await {
            Err(e) => {
                let msg = e.to_string();
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Error {
                    session_id: session_id.clone(),
                    message: msg.clone(),
                });
                return Err(AiError::Network { message: msg });
            }
            Ok(Err(AiError::ToolCallingUnsupported)) |
            Ok(Ok(GenerateWithToolsResult::Unsupported)) => {
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::FallbackMode {
                    session_id: session_id.clone(),
                });
                return run_fallback(pool, notebook, conversation, chat_provider, embedder, session_id, locale, app).await;
            }
            Ok(Err(e)) => {
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Error {
                    session_id: session_id.clone(),
                    message: e.to_string(),
                });
                return Err(e);
            }
            Ok(Ok(GenerateWithToolsResult::Text(text))) => {
                let xml_calls = parse_xml_tool_calls(&text);
                if !xml_calls.is_empty() {
                    for (tool_name, args) in xml_calls {
                        let key = tool_call_key(&tool_name, &args);
                        if seen_calls.contains(&key) { continue; }
                        seen_calls.insert(key);

                        let call_id = format!("xml_{}", uuid::Uuid::new_v4());
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            tool: tool_name.clone(),
                            args: args.clone(),
                        });
                        let (result_content, truncated) =
                            dispatch_tool(&pool, &notebook.id, embedder.as_ref(), &tool_name, &args).await;
                        token_estimate += estimate_tokens(&result_content);
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
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
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
                        session_id: session_id.clone(),
                    });
                    return Ok(());
                }
            }
            Ok(Ok(GenerateWithToolsResult::ToolCalls { calls, raw })) => {
                conversation.push(ChatMessage {
                    role: "assistant".into(),
                    content: serde_json::Value::Null,
                    tool_call_id: None,
                    tool_calls: raw.or_else(|| serde_json::to_value(&calls).ok()),
                });

                for call in &calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&call.args.to_string()).unwrap_or_default();

                    let key = tool_call_key(&call.tool_name, &args);
                    if seen_calls.contains(&key) {
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                            session_id: session_id.clone(),
                            call_id: call.id.clone(),
                            tool: call.tool_name.clone(),
                            args: args.clone(),
                        });
                        let skip_msg = "(skipped: same call already executed earlier in this session)".to_string();
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
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

                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolCall {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        tool: call.tool_name.clone(),
                        args: args.clone(),
                    });

                    let (result_content, truncated) =
                        dispatch_tool(&pool, &notebook.id, embedder.as_ref(), &call.tool_name, &args).await;

                    token_estimate += estimate_tokens(&result_content);

                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
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

/// 不支援 tool-use 的 provider 走簡化版：直接用使用者原始問題做一次 search_documents，
/// 把結果組進 context 後一次性回答（無多跳能力）。比 code_assistant 的兩階段選檔更簡單，
/// 因為語意搜尋本身就不需要先列目錄。
async fn run_fallback(
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
    let last_user_text = messages.iter().rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_string();

    let (search_result, _truncated) = dispatch_tool(
        &pool, &notebook.id, embedder.as_ref(),
        "search_documents", &serde_json::json!({ "query": last_user_text, "top_k": 8 }),
    ).await;

    let language = crate::ai::language_name(locale);
    let phase_prompt = format!(
        "You are a research assistant. Answer the user's question using ONLY the document \
         excerpts below. Always cite the source file and location for each claim. If the \
         excerpts don't answer the question, say so explicitly. Respond in {language}.\n\n\
         ## Document excerpts\n{search_result}"
    );

    let req = GenerateRequest {
        system_prompt: phase_prompt,
        messages,
        context: Default::default(),
        mode: QueryMode::Chat,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let p = chat_provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    while let Some(chunk) = rx.recv().await {
        if !chunk.delta.is_empty() {
            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                session_id: session_id.clone(),
                delta: chunk.delta.clone(),
            });
        }
        if chunk.done { break; }
    }
    let _ = join.await;

    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
        session_id: session_id.clone(),
    });
    Ok(())
}

/// 與 code_assistant 對應函式邏輯相同，措辭改為「文件研究」而非「程式碼調查」。
async fn generate_checkpoint_summary(
    conversation: &[ChatMessage],
    provider: Arc<dyn AiProvider>,
    locale: Locale,
) -> String {
    let language = crate::ai::language_name(locale);
    let system = format!(
        "You are creating a research checkpoint. The conversation below shows a document \
         research session with tool calls and results. Write a concise structured summary \
         IN {language} of ONLY what has been CONFIRMED — facts directly retrieved via tools:\n\
         - Documents consulted (exact file paths and sections)\n\
         - Specific facts, quotes, or values found (with their source citation)\n\
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
```

- [ ] **Step 5: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --lib knowledge_base::chat -- --nocapture`
Expected: PASS（4 passed）

- [ ] **Step 6: 編譯整個 crate 確認沒有其他錯誤**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/knowledge_base/chat.rs src-tauri/src/knowledge_base/mod.rs
git commit -m "feat(knowledge-base): add chat agent loop adapted from code_assistant"
```

---

## Task 3: `kb_chat` Tauri Command 與 App 註冊

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在 `commands/knowledge_base.rs` 加入 `kb_chat` command**

```rust
// 加在 src-tauri/src/commands/knowledge_base.rs 檔案末尾
// 注意：Embedder / EmbedderConfig / HttpEmbedder / Arc 在 Plan A Task 8 已經 import 過，
// 這裡不要重複 import，否則會出現 E0252 (the name is defined multiple times) 編譯錯誤。
use crate::ai::{router::AiRouter, ChatMessage, Locale};

#[tauri::command]
pub async fn kb_chat(
    notebook_id: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
    router: tauri::State<'_, AiRouter>,
) -> Result<(), crate::ai::AiError> {
    use crate::ai::AiError;

    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }

    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|e| AiError::Network { message: e.to_string() })?;

    let embed_provider_id = notebook.embed_provider_id.clone()
        .ok_or_else(|| AiError::InvalidInput { reason: "此筆記本尚未設定 embedding provider".into() })?;
    let embed_model = notebook.embed_model.clone()
        .ok_or_else(|| AiError::InvalidInput { reason: "此筆記本尚未設定 embedding model".into() })?;

    let mut embedder_cfg = resolve_embedder_config(&config, &secrets, &embed_provider_id)
        .map_err(|reason| AiError::InvalidInput { reason })?;
    embedder_cfg.model = embed_model;
    let embedder: Arc<dyn Embedder> = Arc::new(HttpEmbedder::new(embedder_cfg));

    let chat_provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    crate::knowledge_base::chat::run_chat(
        db.pool.clone(), notebook, messages, chat_provider, embedder, session_id, locale, app,
    ).await
}
```

- [ ] **Step 2: 在 `lib.rs` 的 `invoke_handler!` 清單中加入新 command**

在 `kb_sync_notebook,` 那一行之後加入：

```rust
            kb_chat,
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功。若出現 `AiRouter`/`Locale`/`ChatMessage` import 衝突，比對 `src-tauri/src/commands/code_assistant.rs` 的 `use` 區塊調整。

- [ ] **Step 4: 執行完整測試套件**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS（Plan A 全部測試 + Task 1、2 新增的測試)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/lib.rs
git commit -m "feat(knowledge-base): wire kb_chat command into the app"
```

---

## Task 4: `ipc/knowledgeBase.ts`

**Files:**
- Create: `src/ipc/knowledgeBase.ts`
- Test: `src/ipc/knowledgeBase.test.ts`

- [ ] **Step 1: 寫失敗測試（驗證 invoke 呼叫參數正確對應 Rust command 的 camelCase 參數名）**

```typescript
// src/ipc/knowledgeBase.test.ts
import { describe, it, expect, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  kbCreateNotebook, kbListNotebooks, kbDeleteNotebook, kbSyncNotebook, invokeKbChat,
} from "./knowledgeBase";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("knowledgeBase ipc", () => {
  it("kbCreateNotebook invokes kb_create_notebook with camelCase args", async () => {
    vi.mocked(invoke).mockResolvedValue({ id: "nb-1" });
    await kbCreateNotebook("My Notes", "/tmp/docs", "ollama-local", "nomic-embed-text");
    expect(invoke).toHaveBeenCalledWith("kb_create_notebook", {
      name: "My Notes",
      folderPath: "/tmp/docs",
      embedProviderId: "ollama-local",
      embedModel: "nomic-embed-text",
    });
  });

  it("kbListNotebooks invokes kb_list_notebooks with no args", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await kbListNotebooks();
    expect(invoke).toHaveBeenCalledWith("kb_list_notebooks");
  });

  it("kbDeleteNotebook invokes kb_delete_notebook with id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await kbDeleteNotebook("nb-1");
    expect(invoke).toHaveBeenCalledWith("kb_delete_notebook", { id: "nb-1" });
  });

  it("kbSyncNotebook invokes kb_sync_notebook with notebookId", async () => {
    vi.mocked(invoke).mockResolvedValue({ indexed: 0, failed: 0, deleted: 0 });
    await kbSyncNotebook("nb-1");
    expect(invoke).toHaveBeenCalledWith("kb_sync_notebook", { notebookId: "nb-1" });
  });

  it("invokeKbChat invokes kb_chat with full arg set", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await invokeKbChat("nb-1", [{ role: "user", content: "hi" }], "sess-1", "openai-1", "en");
    expect(invoke).toHaveBeenCalledWith("kb_chat", {
      notebookId: "nb-1",
      messages: [{ role: "user", content: "hi" }],
      sessionId: "sess-1",
      providerId: "openai-1",
      locale: "en",
    });
  });
});
```

- [ ] **Step 2: 執行測試,確認因模組不存在而失敗**

Run: `npx vitest run src/ipc/knowledgeBase.test.ts`
Expected: 失敗（找不到 `./knowledgeBase`）

- [ ] **Step 3: 實作 IPC wrapper**

```typescript
// src/ipc/knowledgeBase.ts
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage } from "./ai";

export interface Notebook {
  id: string;
  name: string;
  folder_path: string;
  embed_provider_id: string | null;
  embed_model: string | null;
  embed_dim: number | null;
  last_synced_at: number | null;
  created_at: string;
}

export interface SyncSummary {
  indexed: number;
  failed: number;
  deleted: number;
}

export type KbSyncEvent =
  | { kind: "progress"; notebook_id: string; processed: number; total: number; current_file: string }
  | { kind: "done"; notebook_id: string; indexed: number; failed: number; deleted: number };

export type KbChatEvent =
  | { kind: "tool_call"; session_id: string; call_id: string; tool: string; args: Record<string, unknown> }
  | { kind: "tool_result"; session_id: string; call_id: string; content: string; truncated: boolean }
  | { kind: "text_delta"; session_id: string; delta: string }
  | { kind: "done"; session_id: string }
  | { kind: "error"; session_id: string; message: string }
  | { kind: "fallback_mode"; session_id: string }
  | { kind: "token_count"; session_id: string; count: number; limit: number };

export const KB_SYNC_EVENT = "kb-sync-event";
export const KB_CHAT_EVENT = "kb-chat-event";

export function kbCreateNotebook(
  name: string,
  folderPath: string,
  embedProviderId?: string | null,
  embedModel?: string | null,
): Promise<Notebook> {
  return invoke<Notebook>("kb_create_notebook", {
    name,
    folderPath,
    embedProviderId: embedProviderId ?? null,
    embedModel: embedModel ?? null,
  });
}

export function kbListNotebooks(): Promise<Notebook[]> {
  return invoke<Notebook[]>("kb_list_notebooks");
}

export function kbDeleteNotebook(id: string): Promise<void> {
  return invoke<void>("kb_delete_notebook", { id });
}

export function kbSyncNotebook(notebookId: string): Promise<SyncSummary> {
  return invoke<SyncSummary>("kb_sync_notebook", { notebookId });
}

export function invokeKbChat(
  notebookId: string,
  messages: ChatMessage[],
  sessionId: string,
  providerId?: string | null,
  locale: string = "zh-TW",
): Promise<void> {
  return invoke<void>("kb_chat", {
    notebookId,
    messages,
    sessionId,
    providerId: providerId ?? null,
    locale,
  });
}
```

- [ ] **Step 4: 執行測試,確認通過**

Run: `npx vitest run src/ipc/knowledgeBase.test.ts`
Expected: PASS（5 passed）

- [ ] **Step 5: Commit**

```bash
git add src/ipc/knowledgeBase.ts src/ipc/knowledgeBase.test.ts
git commit -m "feat(knowledge-base): add frontend IPC wrapper for notebook and chat commands"
```

---

## Task 5: `useNotebooks` 與 `useKnowledgeBaseChat` Hooks

**Files:**
- Create: `src/hooks/useNotebooks.ts`
- Create: `src/hooks/useNotebooks.test.ts`
- Create: `src/hooks/useKnowledgeBaseChat.ts`（比照 `useCodeAssistant.ts`，不另外寫測試，與該檔案現有慣例一致）

- [ ] **Step 1: 寫 `useNotebooks` 失敗測試**

```typescript
// src/hooks/useNotebooks.test.ts
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

import { useNotebooks } from "./useNotebooks";

const NB1 = {
  id: "nb-1", name: "Docs", folder_path: "/tmp/docs",
  embed_provider_id: "ollama-local", embed_model: "nomic-embed-text",
  embed_dim: null, last_synced_at: null, created_at: "2026-01-01",
};

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(vi.fn());
});

describe("useNotebooks", () => {
  it("loads notebooks on mount", async () => {
    invokeMock.mockResolvedValueOnce([NB1]);
    const { result } = renderHook(() => useNotebooks());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.notebooks).toEqual([NB1]);
    expect(invokeMock).toHaveBeenCalledWith("kb_list_notebooks");
  });

  it("create() calls kb_create_notebook then refreshes the list", async () => {
    invokeMock.mockResolvedValueOnce([]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.loading).toBe(false));

    invokeMock.mockResolvedValueOnce(NB1); // kb_create_notebook
    invokeMock.mockResolvedValueOnce([NB1]); // refresh after create

    await act(async () => {
      await result.current.create("Docs", "/tmp/docs", "ollama-local", "nomic-embed-text");
    });

    expect(result.current.notebooks).toEqual([NB1]);
  });

  it("remove() calls kb_delete_notebook then refreshes the list", async () => {
    invokeMock.mockResolvedValueOnce([NB1]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.notebooks).toEqual([NB1]));

    invokeMock.mockResolvedValueOnce(undefined); // kb_delete_notebook
    invokeMock.mockResolvedValueOnce([]); // refresh after delete

    await act(async () => {
      await result.current.remove("nb-1");
    });

    expect(result.current.notebooks).toEqual([]);
  });

  it("sync() tracks progress events scoped to the syncing notebook", async () => {
    invokeMock.mockResolvedValueOnce([NB1]); // initial load
    const { result } = renderHook(() => useNotebooks());
    await waitFor(() => expect(result.current.notebooks).toEqual([NB1]));

    let capturedCallback: ((e: { payload: unknown }) => void) | null = null;
    listenMock.mockImplementationOnce((_event: string, cb: typeof capturedCallback) => {
      capturedCallback = cb;
      return Promise.resolve(vi.fn());
    });

    invokeMock.mockImplementationOnce(async () => {
      // Fire a progress event partway through the sync, before it resolves.
      act(() => {
        capturedCallback?.({
          payload: { kind: "progress", notebook_id: "nb-1", processed: 3, total: 10, current_file: "a.pdf" },
        });
      });
      return { indexed: 10, failed: 0, deleted: 0 };
    });
    invokeMock.mockResolvedValueOnce([NB1]); // refresh after sync

    const syncPromise = act(async () => {
      await result.current.sync("nb-1");
    });

    await waitFor(() => expect(result.current.syncProgress?.processed).toBe(3));
    await syncPromise;

    expect(result.current.syncingId).toBeNull();
  });
});
```

- [ ] **Step 2: 執行測試,確認因模組不存在而失敗**

Run: `npx vitest run src/hooks/useNotebooks.test.ts`
Expected: 失敗（找不到 `./useNotebooks`）

- [ ] **Step 3: 實作 `useNotebooks`**

```typescript
// src/hooks/useNotebooks.ts
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  kbCreateNotebook, kbListNotebooks, kbDeleteNotebook, kbSyncNotebook,
  KB_SYNC_EVENT, type Notebook, type KbSyncEvent, type SyncSummary,
} from "../ipc/knowledgeBase";

export interface SyncProgressState {
  processed: number;
  total: number;
  currentFile: string;
}

export interface UseNotebooksResult {
  notebooks: Notebook[];
  loading: boolean;
  error: string | null;
  syncingId: string | null;
  syncProgress: SyncProgressState | null;
  refresh: () => Promise<void>;
  create: (name: string, folderPath: string, embedProviderId?: string, embedModel?: string) => Promise<Notebook>;
  remove: (id: string) => Promise<void>;
  sync: (id: string) => Promise<SyncSummary>;
}

export function useNotebooks(): UseNotebooksResult {
  const [notebooks, setNotebooks] = useState<Notebook[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgressState | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await kbListNotebooks();
      if (mountedRef.current) setNotebooks(list);
    } catch (e) {
      if (mountedRef.current) setError(String(e));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const create = useCallback(async (
    name: string, folderPath: string, embedProviderId?: string, embedModel?: string,
  ) => {
    const nb = await kbCreateNotebook(name, folderPath, embedProviderId, embedModel);
    await refresh();
    return nb;
  }, [refresh]);

  const remove = useCallback(async (id: string) => {
    await kbDeleteNotebook(id);
    await refresh();
  }, [refresh]);

  const sync = useCallback(async (id: string) => {
    setSyncingId(id);
    setSyncProgress({ processed: 0, total: 0, currentFile: "" });

    const unlisten = await listen<KbSyncEvent>(KB_SYNC_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.notebook_id !== id) return;
      if (p.kind === "progress") {
        setSyncProgress({ processed: p.processed, total: p.total, currentFile: p.current_file });
      }
    });

    try {
      const summary = await kbSyncNotebook(id);
      await refresh();
      return summary;
    } finally {
      unlisten();
      if (mountedRef.current) {
        setSyncingId(null);
        setSyncProgress(null);
      }
    }
  }, [refresh]);

  return { notebooks, loading, error, syncingId, syncProgress, refresh, create, remove, sync };
}
```

- [ ] **Step 4: 執行測試,確認通過**

Run: `npx vitest run src/hooks/useNotebooks.test.ts`
Expected: PASS（4 passed）

- [ ] **Step 5: 實作 `useKnowledgeBaseChat`（比照 `useCodeAssistant.ts` 逐一對應改寫，無獨立測試）**

```typescript
// src/hooks/useKnowledgeBaseChat.ts
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { formatAiError, type AiError, type ChatMessage } from "../ipc/ai";
import { KB_CHAT_EVENT, invokeKbChat, type KbChatEvent } from "../ipc/knowledgeBase";
import { useLocale } from "../contexts/LocaleContext";
import type { ToolCallState } from "./useCodeAssistant";

export interface KbMessage {
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCallState[];
  streaming?: boolean;
}

export interface UseKnowledgeBaseChatResult {
  messages: KbMessage[];
  isStreaming: boolean;
  error: string | null;
  isFallbackMode: boolean;
  tokenCount: number;
  tokenLimit: number;
  send: (userText: string, providerId?: string) => Promise<void>;
  clear: () => void;
}

export function useKnowledgeBaseChat(notebookId: string | null): UseKnowledgeBaseChatResult {
  const [messages, setMessages] = useState<KbMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFallbackMode, setIsFallbackMode] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [tokenLimit, setTokenLimit] = useState(50000);
  const mountedRef = useRef(true);
  const { locale } = useLocale();

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // 切換筆記本時重置對話狀態，避免把上一個筆記本的對話帶到新的筆記本。
  useEffect(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
  }, [notebookId]);

  const send = useCallback(async (userText: string, providerId?: string) => {
    if (!userText.trim() || isStreaming || !notebookId) return;
    setError(null);

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

    const unlisten = await listen<KbChatEvent>(KB_CHAT_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.session_id !== sessionId) return;

      if (p.kind === "tool_call") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = [...(last.toolCalls ?? []), { callId: p.call_id, tool: p.tool, args: p.args }];
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "tool_result") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = (last.toolCalls ?? []).map((tc) =>
            tc.callId === p.call_id ? { ...tc, result: { content: p.content, truncated: p.truncated } } : tc,
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
      } else if (p.kind === "token_count") {
        setTokenCount(p.count);
        setTokenLimit(p.limit);
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
      await invokeKbChat(notebookId, chatMessages, sessionId, providerId, locale);
    } catch (e) {
      if (mountedRef.current) {
        const isAiError = e != null && typeof e === "object" && "kind" in (e as object);
        setError(isAiError ? formatAiError(e as AiError) : String(e));
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        unlisten();
      }
    }
  }, [messages, isStreaming, locale, notebookId]);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
  }, []);

  return { messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit, send, clear };
}
```

- [ ] **Step 6: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useNotebooks.ts src/hooks/useNotebooks.test.ts src/hooks/useKnowledgeBaseChat.ts
git commit -m "feat(knowledge-base): add useNotebooks and useKnowledgeBaseChat hooks"
```

---

## Task 6: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`

此 task 特意排在元件實作（Task 7、8）**之前**：`zhTW` 物件是 `Translations` 型別的唯一來源（`i18n.ts` 底部用 `keyof typeof translations["zh-TW"]` 推導型別),元件裡引用還不存在的 `t.kb_*` 會在型別檢查時直接報錯。先把字串加好，後面的元件才能一路型別檢查通過。

- [ ] **Step 1: 在 `zhTW` 物件中加入 Knowledge Base 相關字串**

在 `src/lib/i18n.ts` 的 `zhTW` 物件裡，`ca_files_searched_label` 那一行之後加入（沿用既有的 `ca_cancel`/`ca_export`/`ca_clear`/`ca_fallback_banner`/`ca_tool_truncated` 做通用文案，不重複定義)：

```typescript
    // Knowledge Base tab
    knowledge_base_tab: "知識庫",
    new_knowledge_base_desc: "以自然語言對一整個資料夾的文件提問",
    kb_empty_title: "尚未建立筆記本",
    kb_empty_desc: "建立一個筆記本，指定資料夾後即可對其中的文件提問",
    kb_create_notebook: "新增筆記本",
    kb_create_dialog_title: "新增筆記本",
    kb_create_name_label: "名稱",
    kb_create_name_placeholder: "例如：專案文件",
    kb_create_folder_label: "資料夾",
    kb_create_provider_label: "Embedding Provider",
    kb_create_model_label: "Embedding Model",
    kb_create_model_placeholder: "例如：nomic-embed-text",
    kb_create_submit: "建立",
    kb_create_no_provider: "沒有可用的 Ollama / OpenAI / OpenAI 相容 provider，請先到設定新增",
    kb_no_notebooks: "尚無筆記本",
    kb_sync_button: "同步",
    kb_syncing: "同步中...",
    kb_sync_progress: (processed: number, total: number) => `同步中 ${processed}/${total}`,
    kb_never_synced: "尚未同步",
    kb_last_synced: (time: string) => `上次同步：${time}`,
    kb_delete_notebook_confirm: (name: string) => `確定要刪除筆記本「${name}」嗎？這會刪除所有索引資料。`,
    kb_hint_title: "文件問答",
    kb_hint_desc: (name: string) => `詢問關於「${name}」筆記本中文件的任何問題`,
    kb_hint_examples: ["這些文件的主要內容是什麼？", "找出關於價格的資訊", "整理成重點摘要"],
    kb_input_placeholder: (shortcut: string) => `問關於這個筆記本文件的任何問題... (${shortcut} 送出)`,
    kb_select_notebook_hint: "從左側選擇或建立一個筆記本開始",
    kb_sync_summary: (indexed: number, failed: number, deleted: number) => `已索引 ${indexed} 份，失敗 ${failed} 份，刪除 ${deleted} 份`,
```

- [ ] **Step 2: 在 `enRaw` 物件中加入對應英文字串**

在 `src/lib/i18n.ts` 的 `enRaw` 物件裡，`ca_files_searched_label` 對應的那一行之後加入：

```typescript
    // Knowledge Base tab
    knowledge_base_tab: "Knowledge Base",
    new_knowledge_base_desc: "Ask questions about a whole folder of documents in natural language",
    kb_empty_title: "No Notebooks Yet",
    kb_empty_desc: "Create a notebook and point it at a folder to start asking questions",
    kb_create_notebook: "New Notebook",
    kb_create_dialog_title: "New Notebook",
    kb_create_name_label: "Name",
    kb_create_name_placeholder: "e.g. Project Docs",
    kb_create_folder_label: "Folder",
    kb_create_provider_label: "Embedding Provider",
    kb_create_model_label: "Embedding Model",
    kb_create_model_placeholder: "e.g. nomic-embed-text",
    kb_create_submit: "Create",
    kb_create_no_provider: "No Ollama / OpenAI / OpenAI-compatible provider available — add one in Settings first",
    kb_no_notebooks: "No notebooks yet",
    kb_sync_button: "Sync",
    kb_syncing: "Syncing...",
    kb_sync_progress: (processed: number, total: number) => `Syncing ${processed}/${total}`,
    kb_never_synced: "Never synced",
    kb_last_synced: (time: string) => `Last synced: ${time}`,
    kb_delete_notebook_confirm: (name: string) => `Delete notebook "${name}"? This removes all indexed data.`,
    kb_hint_title: "Document Q&A",
    kb_hint_desc: (name: string) => `Ask anything about the documents in "${name}"`,
    kb_hint_examples: ["What are these documents mainly about?", "Find information about pricing", "Summarize the key points"],
    kb_input_placeholder: (shortcut: string) => `Ask anything about this notebook's documents... (${shortcut} to send)`,
    kb_select_notebook_hint: "Select or create a notebook on the left to get started",
    kb_sync_summary: (indexed: number, failed: number, deleted: number) => `Indexed ${indexed}, failed ${failed}, deleted ${deleted}`,
```

- [ ] **Step 3: 型別檢查確認新增鍵值正確產生 `Translations` 型別**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(knowledge-base): add i18n strings for Knowledge Base tab"
```

---

## Task 7: `NotebookSidebar` / `NotebookCreateDialog` / `SyncProgress` 元件

**Files:**
- Create: `src/components/KnowledgeBaseView/NotebookSidebar.tsx`
- Create: `src/components/KnowledgeBaseView/NotebookCreateDialog.tsx`
- Create: `src/components/KnowledgeBaseView/SyncProgress.tsx`

這三個是純展示元件（props in, JSX out),比照 `CodeAssistantView/ToolCallCard.tsx` 的慣例不另外寫測試檔——該元件同樣沒有測試檔，靠 Task 8 結尾的手動瀏覽器驗證把關。CSS 類別在 Task 8 的 `styles.css` 中定義，這裡的元件不直接 import 樣式檔（比照 `ToolCallCard.tsx` 只在 `index.tsx` import 一次的慣例）。

- [ ] **Step 1: 建立 `SyncProgress.tsx`**

```tsx
// src/components/KnowledgeBaseView/SyncProgress.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { SyncProgressState } from "../../hooks/useNotebooks";

interface Props {
  progress: SyncProgressState;
}

export function SyncProgress({ progress }: Props) {
  const { t } = useLocale();
  const pct = progress.total > 0 ? Math.round((progress.processed / progress.total) * 100) : 0;

  return (
    <div className="kb-sync-progress">
      <div className="kb-sync-progress__label">
        {t.kb_sync_progress(progress.processed, progress.total)}
      </div>
      <div className="kb-sync-progress__bar">
        <div className="kb-sync-progress__fill" style={{ width: `${pct}%` }} />
      </div>
      {progress.currentFile && (
        <div className="kb-sync-progress__file" title={progress.currentFile}>
          {progress.currentFile}
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 建立 `NotebookCreateDialog.tsx`**

```tsx
// src/components/KnowledgeBaseView/NotebookCreateDialog.tsx
import { useEffect, useState } from "react";
import { pickFolder } from "../../ipc/vcs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  onCreate: (name: string, folderPath: string, embedProviderId?: string, embedModel?: string) => Promise<void>;
  onClose: () => void;
}

// Anthropic 沒有 embedding API，只有這三種 provider 類型可用於 embedding。
const EMBEDDING_CAPABLE_TYPES = new Set(["ollama", "openai", "openai-compatible"]);

export function NotebookCreateDialog({ onCreate, onClose }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [folderPath, setFolderPath] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [model, setModel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listProviders().then((list) => {
      const embeddable = list.filter((p) => EMBEDDING_CAPABLE_TYPES.has(p.provider_type));
      setProviders(embeddable);
      if (embeddable.length > 0) setProviderId(embeddable[0].id);
    }).catch(() => {});
  }, []);

  const handlePickFolder = async () => {
    const folder = await pickFolder();
    if (folder) setFolderPath(folder);
  };

  const handleSubmit = async () => {
    if (!name.trim() || !folderPath || !providerId || !model.trim() || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate(name.trim(), folderPath, providerId, model.trim());
      onClose();
    } catch (e) {
      setError(String(e));
      setSubmitting(false);
    }
  };

  return (
    <div className="kb-dialog-overlay" onClick={onClose}>
      <div className="kb-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="kb-dialog__title">{t.kb_create_dialog_title}</h3>

        <label className="kb-dialog__field">
          <span>{t.kb_create_name_label}</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t.kb_create_name_placeholder}
          />
        </label>

        <label className="kb-dialog__field">
          <span>{t.kb_create_folder_label}</span>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={handlePickFolder}>
            {folderPath || t.ca_pick_folder}
          </button>
        </label>

        {providers.length === 0 ? (
          <div className="kb-dialog__warning">{t.kb_create_no_provider}</div>
        ) : (
          <>
            <label className="kb-dialog__field">
              <span>{t.kb_create_provider_label}</span>
              <select value={providerId} onChange={(e) => setProviderId(e.target.value)}>
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>{p.display_name}</option>
                ))}
              </select>
            </label>

            <label className="kb-dialog__field">
              <span>{t.kb_create_model_label}</span>
              <input
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={t.kb_create_model_placeholder}
              />
            </label>
          </>
        )}

        {error && <div className="kb-dialog__error">{error}</div>}

        <div className="kb-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose}>
            {t.ca_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            onClick={handleSubmit}
            disabled={!name.trim() || !folderPath || !providerId || !model.trim() || submitting}
          >
            {t.kb_create_submit}
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 建立 `NotebookSidebar.tsx`**

```tsx
// src/components/KnowledgeBaseView/NotebookSidebar.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { Notebook } from "../../ipc/knowledgeBase";
import type { SyncProgressState } from "../../hooks/useNotebooks";
import { SyncProgress } from "./SyncProgress";

interface Props {
  notebooks: Notebook[];
  activeId: string | null;
  syncingId: string | null;
  syncProgress: SyncProgressState | null;
  onSelect: (id: string) => void;
  onSync: (id: string) => void;
  onDelete: (id: string) => void;
  onAddClick: () => void;
}

export function NotebookSidebar({
  notebooks, activeId, syncingId, syncProgress, onSelect, onSync, onDelete, onAddClick,
}: Props) {
  const { t } = useLocale();

  const formatSyncedAt = (ts: number | null): string =>
    ts === null ? t.kb_never_synced : t.kb_last_synced(new Date(ts * 1000).toLocaleString());

  return (
    <div className="kb-sidebar">
      <div className="kb-sidebar__header">
        <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onAddClick}>
          + {t.kb_create_notebook}
        </button>
      </div>

      <div className="kb-sidebar__list">
        {notebooks.length === 0 && (
          <div className="kb-sidebar__empty">{t.kb_no_notebooks}</div>
        )}
        {notebooks.map((nb) => {
          const isActive = nb.id === activeId;
          const isSyncing = nb.id === syncingId;
          return (
            <div key={nb.id} className={`kb-sidebar__item ${isActive ? "kb-sidebar__item--active" : ""}`}>
              <button className="kb-sidebar__item-main" onClick={() => onSelect(nb.id)}>
                <div className="kb-sidebar__item-name">{nb.name}</div>
                <div className="kb-sidebar__item-path" title={nb.folder_path}>{nb.folder_path}</div>
                <div className="kb-sidebar__item-synced">
                  {isSyncing ? t.kb_syncing : formatSyncedAt(nb.last_synced_at)}
                </div>
              </button>

              {isSyncing && syncProgress && <SyncProgress progress={syncProgress} />}

              <div className="kb-sidebar__item-actions">
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={() => onSync(nb.id)}
                  disabled={isSyncing}
                >
                  {t.kb_sync_button}
                </button>
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={() => {
                    if (window.confirm(t.kb_delete_notebook_confirm(nb.name))) onDelete(nb.id);
                  }}
                >
                  ✕
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤（`styles.css` 的 class 名稱此時還沒定義任何規則，但 TypeScript 不檢查 CSS class 是否存在，所以不影響型別檢查通過；視覺樣式會在 Task 8 補上）

- [ ] **Step 5: Commit**

```bash
git add src/components/KnowledgeBaseView/NotebookSidebar.tsx src/components/KnowledgeBaseView/NotebookCreateDialog.tsx src/components/KnowledgeBaseView/SyncProgress.tsx
git commit -m "feat(knowledge-base): add notebook sidebar, create dialog, and sync progress components"
```

---

## Task 8: `KnowledgeBaseView` 主元件與 Tab 整合

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs`（新增 `kb_open_document`）
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/ipc/knowledgeBase.ts`（新增 `kbOpenDocument`）
- Modify: `src/components/Icons.tsx`（新增 `LibraryIcon`）
- Create: `src/components/KnowledgeBaseView/index.tsx`
- Create: `src/components/KnowledgeBaseView/styles.css`
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/NewTabPicker/index.tsx`
- Modify: `src/components/TerminalApp.tsx`

引用來源 chip 需要「點擊開啟該檔案」（規格第 7 節）。既有的 `open_url` command 註解明確寫著「僅供內部硬編碼 HTTPS URL 呼叫，未經審查處理使用者輸入路徑」,不能直接挪用。這裡新增一個專用、有邊界檢查的 `kb_open_document` command（比照 `code_assistant/tools.rs` 的 `resolve_safe` 手法：canonicalize 後檢查是否仍在筆記本資料夾內),避免把未經驗證的路徑交給 `open::that`。

- [ ] **Step 1: 新增 `kb_open_document` command**

```rust
// 加在 src-tauri/src/commands/knowledge_base.rs 檔案末尾
/// 開啟筆記本資料夾內的某份文件（OS 預設應用程式）。
/// rel_path 來自工具呼叫結果（AI 影響的內容），開啟前一定要做邊界檢查，
/// 避免解析到筆記本資料夾以外的路徑。
#[tauri::command]
pub async fn kb_open_document(
    notebook_id: String,
    rel_path: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<(), String> {
    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|e| e.to_string())?;

    let root = std::path::Path::new(&notebook.folder_path);
    let canonical_root = root.canonicalize()
        .map_err(|e| format!("Cannot resolve notebook folder: {e}"))?;
    let target = root.join(rel_path.trim_start_matches('/'));
    let canonical_target = target.canonicalize()
        .map_err(|e| format!("File not found: {e}"))?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err("Path is outside the notebook folder".into());
    }

    open::that(canonical_target).map_err(|e| e.to_string())
}
```

- [ ] **Step 2: 在 `lib.rs` 的 `invoke_handler!` 清單中加入新 command**

在 `kb_chat,` 那一行之後加入：

```rust
            kb_open_document,
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功

- [ ] **Step 4: 在 `ipc/knowledgeBase.ts` 加入 `kbOpenDocument`**

```typescript
// 加在 src/ipc/knowledgeBase.ts 檔案末尾
export function kbOpenDocument(notebookId: string, relPath: string): Promise<void> {
  return invoke<void>("kb_open_document", { notebookId, relPath });
}
```

- [ ] **Step 5: 在 `Icons.tsx` 加入 `LibraryIcon`**

```tsx
// 加在 src/components/Icons.tsx 檔案末尾（緊接 CodeIcon 之後）
// 29. Library / Knowledge Base Icon
export function LibraryIcon({ size = 16, ...props }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      {...props}
    >
      <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
      <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
    </svg>
  );
}
```

- [ ] **Step 6: 建立 `KnowledgeBaseView/index.tsx`**

```tsx
// src/components/KnowledgeBaseView/index.tsx
import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { kbOpenDocument } from "../../ipc/knowledgeBase";
import { useNotebooks } from "../../hooks/useNotebooks";
import { useKnowledgeBaseChat } from "../../hooks/useKnowledgeBaseChat";
import { useLocale } from "../../contexts/LocaleContext";
import { ToolCallCard } from "../CodeAssistantView/ToolCallCard";
import { MarkdownText } from "../../lib/markdown";
import { ModelPickerButton } from "../ModelPickerButton";
import { NotebookSidebar } from "./NotebookSidebar";
import { NotebookCreateDialog } from "./NotebookCreateDialog";
// 重用 Code Assistant 的聊天氣泡/工具卡片樣式（ca-msg、ca-hint-*、ca-toolbar 等）。
// 這裡明確 import，不依賴「CodeAssistantView 剛好也被載入過」這種隱性順序。
import "../CodeAssistantView/styles.css";
import "./styles.css";

const STORAGE_KEY = "aiterm-knowledge-base-active-notebook";

function loadSavedNotebookId(): string | null {
  try { return localStorage.getItem(STORAGE_KEY); } catch { return null; }
}
function saveNotebookId(id: string | null) {
  try {
    if (id) localStorage.setItem(STORAGE_KEY, id);
    else localStorage.removeItem(STORAGE_KEY);
  } catch { /* ignore */ }
}

// search_documents 的結果格式："[1] report.pdf — 第一章 (score 0.85)\n<內容>"
const SOURCE_LINE_RE = /^\[\d+\]\s+(.+?)\s+—\s+(.+?)\s+\(score/;

interface SourceRef {
  path: string;
  location: string;
}

function extractSources(content: string): SourceRef[] {
  const out: SourceRef[] = [];
  for (const line of content.split("\n")) {
    const m = SOURCE_LINE_RE.exec(line);
    if (m) out.push({ path: m[1], location: m[2] });
  }
  return out;
}

function dedupeSources(sources: SourceRef[]): SourceRef[] {
  const seen = new Set<string>();
  const out: SourceRef[] = [];
  for (const s of sources) {
    if (seen.has(s.path)) continue;
    seen.add(s.path);
    out.push(s);
  }
  return out;
}

interface Props {
  isActive: boolean;
}

export function KnowledgeBaseView({ isActive }: Props) {
  const { t } = useLocale();
  const { notebooks, loading, syncingId, syncProgress, create, remove, sync } = useNotebooks();
  const [activeNotebookId, setActiveNotebookId] = useState<string | null>(loadSavedNotebookId);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const submitShortcutRef = useRef<SubmitShortcut>("enter");
  submitShortcutRef.current = submitShortcut;
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const activeNotebook = notebooks.find((nb) => nb.id === activeNotebookId) ?? null;
  const { messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit, send, clear } =
    useKnowledgeBaseChat(activeNotebookId);

  // 首次載入完成後，若儲存的筆記本 id 已不存在（例如被刪除），改選第一個。
  useEffect(() => {
    if (loading) return;
    if (activeNotebookId && notebooks.some((nb) => nb.id === activeNotebookId)) return;
    setActiveNotebookId(notebooks[0]?.id ?? null);
  }, [loading, notebooks, activeNotebookId]);

  useEffect(() => { saveNotebookId(activeNotebookId); }, [activeNotebookId]);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      if (list.length > 0 && !selectedProviderId) {
        const def = list.find((p) => p.is_default) ?? list[0];
        setSelectedProviderId(def.id);
      }
    }).catch(() => {});
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (isActive) {
      getConfig().then((cfg) => setSubmitShortcut(cfg.submit_shortcut ?? "enter")).catch(() => {});
    }
  }, [isActive]);

  useEffect(() => {
    if (isActive) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages, isActive]);

  const handleSend = useCallback(() => {
    if (!input.trim() || isStreaming || !activeNotebookId) return;
    const text = input;
    setInput("");
    void send(text, selectedProviderId || undefined);
  }, [input, isStreaming, activeNotebookId, selectedProviderId, send]);

  const shortcutLabel = submitShortcut === "shift-enter" ? "Shift+Enter" : submitShortcut === "ctrl-enter" ? "Ctrl+Enter" : "Enter";

  const handleKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter") return;
    const sc = submitShortcutRef.current;
    const ok = (sc === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
               (sc === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
               (sc === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
    if (ok) { e.preventDefault(); handleSend(); }
  }, [handleSend]);

  const handleCreateNotebook = useCallback(async (
    name: string, folderPath: string, embedProviderId?: string, embedModel?: string,
  ) => {
    const nb = await create(name, folderPath, embedProviderId, embedModel);
    setActiveNotebookId(nb.id);
  }, [create]);

  const handleDeleteNotebook = useCallback(async (id: string) => {
    await remove(id);
    if (activeNotebookId === id) setActiveNotebookId(null);
  }, [remove, activeNotebookId]);

  const handleOpenSource = useCallback((path: string) => {
    if (!activeNotebookId) return;
    void kbOpenDocument(activeNotebookId, path);
  }, [activeNotebookId]);

  return (
    <div className="kb-view">
      <NotebookSidebar
        notebooks={notebooks}
        activeId={activeNotebookId}
        syncingId={syncingId}
        syncProgress={syncProgress}
        onSelect={setActiveNotebookId}
        onSync={sync}
        onDelete={handleDeleteNotebook}
        onAddClick={() => setShowCreateDialog(true)}
      />

      <div className="kb-main">
        {!activeNotebook ? (
          <div className="ca-empty">
            <div className="ca-empty__icon">📚</div>
            <div className="ca-empty__title">{t.kb_empty_title}</div>
            <div className="ca-empty__desc">{t.kb_empty_desc}</div>
            <button className="aiterm-btn aiterm-btn--primary" onClick={() => setShowCreateDialog(true)}>
              {t.kb_create_notebook}
            </button>
          </div>
        ) : (
          <>
            <div className="ca-messages">
              {messages.length === 0 && (
                <div className="ca-hint-center">
                  <div className="ca-hint-title">{t.kb_hint_title}</div>
                  <div className="ca-hint-desc">{t.kb_hint_desc(activeNotebook.name)}</div>
                  <div className="ca-hint-examples">
                    {t.kb_hint_examples.map((ex) => (
                      <button key={ex} className="ca-hint-chip" onClick={() => setInput(ex)}>
                        {ex}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {messages.map((msg, i) => {
                const isDone = msg.role === "assistant" && !msg.streaming;
                const sources = isDone
                  ? dedupeSources([
                      ...(msg.toolCalls ?? [])
                        .filter((tc) => tc.tool === "search_documents" && tc.result && !tc.result.content.startsWith("Error:"))
                        .flatMap((tc) => extractSources(tc.result!.content)),
                      ...(msg.toolCalls ?? [])
                        .filter((tc) => tc.tool === "read_document" && tc.result && !tc.result.content.startsWith("Error:"))
                        .map((tc) => ({ path: String(tc.args.path ?? ""), location: "" })),
                    ])
                  : [];

                return (
                  <div key={i} className={`ca-msg ca-msg--${msg.role}`}>
                    {msg.role === "assistant" && (msg.toolCalls ?? []).map((tc) => (
                      <ToolCallCard key={tc.callId} toolCall={tc} />
                    ))}
                    {(msg.content || msg.streaming) && (
                      <div className="ca-msg__bubble">
                        {msg.role === "assistant" ? <MarkdownText text={msg.content} /> : msg.content}
                        {msg.streaming && <span className="ca-streaming-cursor" />}
                      </div>
                    )}
                    {sources.length > 0 && (
                      <div className="kb-sources">
                        {sources.map((s) => (
                          <button
                            key={s.path}
                            className="kb-sources__chip"
                            title={s.location ? `${s.path} — ${s.location}` : s.path}
                            onClick={() => handleOpenSource(s.path)}
                          >
                            {s.path.split("/").pop()}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
              {error && <div className="ca-error">{error}</div>}
              <div ref={messagesEndRef} />
            </div>

            {isFallbackMode && <div className="ca-fallback-banner">{t.ca_fallback_banner}</div>}

            <div className="ca-toolbar">
              <ModelPickerButton
                providers={providers}
                selectedId={selectedProviderId}
                onChange={setSelectedProviderId}
              />
              {isStreaming && tokenCount > 0 && (
                <span className="ca-token-count" title={`估算 token 用量（上限 ${tokenLimit.toLocaleString()}）`}>
                  {tokenCount.toLocaleString()} / {tokenLimit.toLocaleString()}
                </span>
              )}
              {messages.length > 0 && (
                <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={clear}>
                  {t.ca_clear}
                </button>
              )}
            </div>

            <div className="ca-input-row">
              <div className="aiterm-input-pill-container" style={{
                display: "flex", alignItems: "center",
                background: "rgba(255, 255, 255, 0.03)",
                border: "1px solid rgba(255, 255, 255, 0.08)",
                borderRadius: 20, padding: "4px 8px", flex: 1, gap: 6,
              }}>
                <textarea
                  ref={textareaRef}
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={handleKeyDown}
                  placeholder={t.kb_input_placeholder(shortcutLabel)}
                  rows={1}
                  disabled={isStreaming}
                  style={{
                    flex: 1, background: "transparent", border: "none",
                    color: "var(--text-primary)", padding: "4px 6px", fontSize: 13,
                    resize: "none", outline: "none", fontFamily: "inherit",
                    height: 24, lineHeight: "24px", overflowY: "hidden",
                  }}
                />
                <button
                  onClick={handleSend}
                  disabled={isStreaming || !input.trim()}
                  className="aiterm-btn aiterm-btn--primary aiterm-btn--icon"
                  title="送出 (Enter)"
                >
                  ▲
                </button>
              </div>
            </div>
          </>
        )}
      </div>

      {showCreateDialog && (
        <NotebookCreateDialog
          onCreate={handleCreateNotebook}
          onClose={() => setShowCreateDialog(false)}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 7: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/lib.rs src/ipc/knowledgeBase.ts src/components/Icons.tsx src/components/KnowledgeBaseView/index.tsx
git commit -m "feat(knowledge-base): add KnowledgeBaseView main component"
```

---

- [ ] **Step 9: 建立 `KnowledgeBaseView/styles.css`（只定義 `ca-*` 沒有涵蓋的新類別）**

```css
/* src/components/KnowledgeBaseView/styles.css */

.kb-view {
  display: flex;
  flex-direction: row;
  height: 100%;
  background: var(--bg-primary, #0a0b14);
  color: var(--text-primary, #e0e0e0);
}

/* ── Sidebar ──────────────────────────────────────────────────────── */
.kb-sidebar {
  width: 240px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid #1e1e1e;
  background: #0c0c0c;
  overflow-y: auto;
}
.kb-sidebar__header {
  padding: 10px;
  border-bottom: 1px solid #1e1e1e;
}
.kb-sidebar__list {
  flex: 1;
  overflow-y: auto;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.kb-sidebar__empty {
  padding: 20px 10px;
  text-align: center;
  color: #555;
  font-size: 12px;
}
.kb-sidebar__item {
  border: 1px solid transparent;
  border-radius: 6px;
  padding: 6px;
}
.kb-sidebar__item--active {
  border-color: rgba(168, 85, 247, 0.35);
  background: rgba(168, 85, 247, 0.08);
}
.kb-sidebar__item-main {
  display: block;
  width: 100%;
  text-align: left;
  background: transparent;
  border: none;
  cursor: pointer;
  padding: 2px 4px;
  color: inherit;
}
.kb-sidebar__item-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary, #e0e0e0);
}
.kb-sidebar__item-path {
  font-size: 10px;
  color: #666;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.kb-sidebar__item-synced {
  font-size: 10px;
  color: #555;
  margin-top: 2px;
}
.kb-sidebar__item-actions {
  display: flex;
  gap: 4px;
  margin-top: 4px;
  padding: 0 4px;
}

/* ── Main pane ────────────────────────────────────────────────────── */
.kb-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

/* ── Sync progress ────────────────────────────────────────────────── */
.kb-sync-progress {
  padding: 2px 4px;
  margin-top: 2px;
}
.kb-sync-progress__label {
  font-size: 10px;
  color: #888;
  margin-bottom: 2px;
}
.kb-sync-progress__bar {
  height: 3px;
  background: #222;
  border-radius: 2px;
  overflow: hidden;
}
.kb-sync-progress__fill {
  height: 100%;
  background: var(--accent, #a855f7);
  transition: width 0.2s ease;
}
.kb-sync-progress__file {
  font-size: 9px;
  color: #555;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  margin-top: 2px;
}

/* ── Create notebook dialog ──────────────────────────────────────── */
.kb-dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.6);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 2000;
}
.kb-dialog {
  width: 380px;
  max-width: 90vw;
  background: #111;
  border: 1px solid #2a2a2a;
  border-radius: 10px;
  padding: 18px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.kb-dialog__title {
  font-size: 15px;
  font-weight: 600;
  margin: 0 0 4px;
}
.kb-dialog__field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
  color: #888;
}
.kb-dialog__field input,
.kb-dialog__field select {
  background: #0c0c0c;
  border: 1px solid #2a2a2a;
  color: var(--text-primary, #e0e0e0);
  border-radius: 4px;
  padding: 6px 8px;
  font-size: 13px;
  outline: none;
}
.kb-dialog__field input:focus,
.kb-dialog__field select:focus {
  border-color: var(--accent, #a855f7);
}
.kb-dialog__warning {
  font-size: 11px;
  color: #f5c518;
  background: rgba(245, 197, 24, 0.08);
  border: 1px solid rgba(245, 197, 24, 0.2);
  border-radius: 4px;
  padding: 6px 8px;
}
.kb-dialog__error {
  font-size: 11px;
  color: #f87171;
}
.kb-dialog__actions {
  display: flex;
  justify-content: flex-end;
  gap: 6px;
  margin-top: 6px;
}

/* ── Source citation chips ───────────────────────────────────────── */
.kb-sources {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 5px;
  padding: 0 2px;
}
.kb-sources__chip {
  font-size: 10px;
  font-family: monospace;
  color: #3d5a3e;
  background: rgba(74, 222, 128, 0.05);
  border: 1px solid rgba(74, 222, 128, 0.12);
  border-radius: 3px;
  padding: 1px 6px;
  cursor: pointer;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.kb-sources__chip:hover {
  color: #4ade80;
  border-color: rgba(74, 222, 128, 0.3);
}
```

- [ ] **Step 10: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤

- [ ] **Step 11: Commit**

```bash
git add src/components/KnowledgeBaseView/styles.css
git commit -m "feat(knowledge-base): add KnowledgeBaseView styles"
```

---

### Tab 整合（三處註冊點）

- [ ] **Step 12: 在 `TabBar/index.tsx` 加入 `"knowledge-base"` 分頁類型與 icon**

```typescript
// src/components/TabBar/index.tsx
// 1) import 區塊加入 LibraryIcon
import {
  TerminalIcon,
  DatabaseIcon,
  PaintbrushIcon,
  LinkIcon,
  LeafIcon,
  FileTextIcon,
  BookOpenIcon,
  RefreshIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  CodeIcon,
  LibraryIcon
} from "../Icons";

// 2) TabType union 加入 "knowledge-base"
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base";

// 3) getTabIcon 的 switch 加入一個 case（緊接 "code-assistant" 之後）
    case "knowledge-base": return <LibraryIcon size={18} />;
```

- [ ] **Step 13: 在 `NewTabPicker/index.tsx` 加入選單項目**

```typescript
// src/components/NewTabPicker/index.tsx
// 1) import 區塊加入 LibraryIcon
import {
  TerminalIcon,
  DatabaseIcon,
  PaintbrushIcon,
  LinkIcon,
  BranchIcon,
  FileTextIcon,
  BookOpenIcon,
  RefreshIcon,
  CodeIcon,
  LibraryIcon,
} from "../Icons";

// 2) items 陣列加入一行（緊接 "code-assistant" 之後）
    { type: "knowledge-base", icon: <LibraryIcon size={18} />,     label: t.knowledge_base_tab, desc: t.new_knowledge_base_desc },
```

- [ ] **Step 14: 在 `TerminalApp.tsx` 加入 import、選單處理、與渲染分支**

```typescript
// src/components/TerminalApp.tsx
// 1) import 區塊加入（緊接 CodeAssistantView 之後）
import { KnowledgeBaseView } from "./KnowledgeBaseView";

// 2) handlePickerSelect 的型別聯集與 if 鏈加入 "knowledge-base"
  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base") => {
    const newId = crypto.randomUUID();
    let title = t.terminal_tab;
    if (type === "database") title = t.database_tab;
    if (type === "design") title = t.design_tab;
    if (type === "cross-db") title = t.cross_db_tab;
    if (type === "vcs") title = t.vcs_tab;
    if (type === "doc-converter") title = t.doc_converter_tab;
    if (type === "api-docs") title = t.api_docs_tab;
    if (type === "loop-studio") title = t.loop_studio_tab;
    if (type === "code-assistant") title = t.code_assistant_tab;
    if (type === "knowledge-base") title = t.knowledge_base_tab;
    setTabs((prev) => [...prev, { id: newId, title, type }]);
    setActiveId(newId);
    setPickerOpen(false);
  }, [t.terminal_tab, t.database_tab, t.design_tab, t.cross_db_tab, t.vcs_tab, t.doc_converter_tab, t.api_docs_tab, t.loop_studio_tab, t.code_assistant_tab, t.knowledge_base_tab]);

// 3) render 區塊的 tab.type 條件鏈加入一個分支（緊接 "code-assistant" 之後）
              ) : tab.type === "code-assistant" ? (
                <CodeAssistantView isActive={isActive} />
              ) : tab.type === "knowledge-base" ? (
                <KnowledgeBaseView isActive={isActive} />
              ) : (
```

- [ ] **Step 15: 型別檢查與 lint**

Run: `npx tsc --noEmit && npm run lint`
Expected: 無錯誤

- [ ] **Step 16: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/NewTabPicker/index.tsx src/components/TerminalApp.tsx
git commit -m "feat(knowledge-base): register knowledge-base tab type in TabBar/NewTabPicker/TerminalApp"
```

---

## 完成後的驗收方式

- [ ] **後端**：`cd src-tauri && cargo test` 全數通過
- [ ] **前端型別/單元測試**：`npx tsc --noEmit && npx vitest run`
- [ ] **手動瀏覽器驗證**（CLAUDE.md 要求 UI 變更需實際操作驗證,不能只憑型別檢查判定完成）：
  1. `npm run tauri:dev` 啟動開發伺服器
  2. 開一個新分頁，確認選單裡看得到「知識庫」項目且圖示正確
  3. 建立一個筆記本：選一個實際存在、內含幾份文件（PDF/txt/docx 皆可）的資料夾，選擇一個已設定的 Ollama 或 OpenAI provider 作為 embedding provider
  4. 點擊「同步」，確認進度列正確顯示已處理/總數，同步完成後「上次同步」時間有更新
  5. 提問一個文件裡確實有答案的問題，確認：
     - 出現 `search_documents` 的 ToolCallCard
     - 回答內容附有出處引用（檔名 + 章節)
     - 回答下方出現來源 chip，點擊後對應檔案能用 OS 預設程式開啟
  6. 提問一個文件裡沒有答案的問題，確認 AI 誠實說明沒找到，而不是編造
  7. 切換到另一個（或新建的）筆記本，確認對話歷史正確重置，不會把前一個筆記本的回答帶過來
  8. 刪除一個筆記本，確認清單更新且該筆記本的資料被清除

## 不在此 Plan 範圍內

- 自動監控資料夾變化並即時重新索引（維持設計規格的手動同步決定）
- 多筆記本交叉搜尋
- 匯出對話紀錄（可仿照 Code Assistant 未來加入)
- PDF 精確頁碼引用的保證（`location_hint` 為 best-effort，取決於 MarkItDown 輸出結構)


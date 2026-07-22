# KB 對話歷史記錄 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓知識庫（Knowledge Base）問答對話持久化保存，同一筆記本下可有多組獨立對話記錄，使用者可切換、刪除、匯出成 md 檔案，記錄清單顯示在可調寬度的右側邊欄。

**Architecture:** 新增 `kb_chat_sessions`/`kb_chat_messages` 兩張表到既有共用的 `knowledge_base.db`；後端在串流完成當下直接寫入（比照 `design_chat` 既有模式）；前端新增右側可調寬度側邊欄（比照 `DesignView` 的拖曳調整寬度邏輯），對話記錄的建立採延遲建立（使用者送出第一則訊息時才真正在後端建立 session row）。

**Tech Stack:** Rust（sqlx SQLite）、Tauri command、React + TypeScript、Vitest。

**Spec:** `docs/superpowers/specs/2026-07-22-kb-chat-history-design.md`

**Note on delete confirmation:** spec 文件描述「刪除沿用筆記本刪除的無確認彈窗風格」，但實際檢查 `NotebookSidebar.tsx:62-64` 發現筆記本刪除其實是有 `window.confirm()` 的。本計畫以實際程式碼為準：對話記錄刪除也使用 `window.confirm()`。

---

## 檔案結構總覽

**後端新增：**
- `src-tauri/src/db/kb_chat_sessions.rs` — `kb_chat_sessions`/`kb_chat_messages` 的 CRUD 函式與型別

**後端修改：**
- `src-tauri/src/db/mod.rs` — 註冊新模組
- `src-tauri/src/db/knowledge_base.rs` — schema 新增兩張表；`delete_notebook` 新增級聯清理
- `src-tauri/src/knowledge_base/chat.rs` — `run_chat`/`run_fallback` 新增 `chat_session_id` 參數並在完成時持久化
- `src-tauri/src/commands/knowledge_base.rs` — `kb_chat` 新增參數；新增 4 個指令
- `src-tauri/src/lib.rs` — 註冊新指令

**前端新增：**
- `src/components/KnowledgeBaseView/ChatHistorySidebar.tsx` — 右側對話記錄側邊欄（含匯出邏輯）

**前端修改：**
- `src/ipc/knowledgeBase.ts` — 新增型別與 4 個 invoke 包裝函式；`invokeKbChat` 新增參數
- `src/ipc/knowledgeBase.test.ts` — 更新既有 `invokeKbChat` 測試；新增 4 個新函式的測試
- `src/hooks/useKnowledgeBaseChat.ts` — 新增 session 管理（延遲建立、清單、切換、刪除）
- `src/components/KnowledgeBaseView/index.tsx` — 掛載側邊欄、拖曳調整寬度邏輯
- `src/components/KnowledgeBaseView/styles.css` — 側邊欄與拖曳把手樣式
- `src/lib/i18n.ts` — 新增中英文字串

---

### Task 1: 後端 schema — 新增對話記錄兩張表 + 修復 notebook 級聯刪除

**Files:**
- Modify: `src-tauri/src/db/knowledge_base.rs`

- [ ] **Step 1: 在 `init()` 新增兩張表與索引**

在 `src-tauri/src/db/knowledge_base.rs` 的 `init()` 函式內，緊接在 `idx_chunks_document` 索引建立之後（第 99-102 行之後、`Ok(())` 之前），插入：

```rust
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kb_chat_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                notebook_id TEXT NOT NULL,
                title TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kb_chat_messages (
                id TEXT PRIMARY KEY NOT NULL,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls_json TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kb_chat_sessions_notebook ON kb_chat_sessions(notebook_id)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kb_chat_messages_session ON kb_chat_messages(session_id)")
            .execute(&self.pool).await?;
```

（沒有宣告 `FOREIGN KEY` — 比照同一個檔案內 `documents`/`chunks` 表的既有風格，級聯刪除完全由應用層程式碼負責，不依賴 SQLite FK constraint。）

- [ ] **Step 2: 修復 `delete_notebook`，補上對話記錄的級聯刪除**

現有的 `delete_notebook`（第 140-147 行）只清理 `chunks`/`documents`，刪除筆記本後 `kb_chat_sessions`/`kb_chat_messages` 會變成孤兒資料。改為：

```rust
pub async fn delete_notebook(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE notebook_id = ?)")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM documents WHERE notebook_id = ?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kb_chat_messages WHERE session_id IN (SELECT id FROM kb_chat_sessions WHERE notebook_id = ?)")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kb_chat_sessions WHERE notebook_id = ?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM notebooks WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await
}
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo check --lib`
Expected: 編譯成功，無錯誤（可忽略既有的無關 warning）。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/db/knowledge_base.rs
git commit -m "feat(kb): add chat session/message schema, fix notebook cascade delete"
```

---

### Task 2: 後端 CRUD — `db/kb_chat_sessions.rs`

**Files:**
- Create: `src-tauri/src/db/kb_chat_sessions.rs`
- Modify: `src-tauri/src/db/mod.rs`

- [ ] **Step 1: 建立 CRUD 檔案**

Create `src-tauri/src/db/kb_chat_sessions.rs`:

```rust
use sqlx::{SqlitePool, FromRow};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ChatSessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ChatMessageRow {
    pub role: String,
    pub content: String,
    pub tool_calls_json: Option<String>,
    pub created_at: String,
}

pub async fn create_chat_session(
    pool: &SqlitePool,
    notebook_id: &str,
    title: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO kb_chat_sessions (id, notebook_id, title) VALUES (?, ?, ?)")
        .bind(&id).bind(notebook_id).bind(title)
        .execute(pool).await?;
    Ok(id)
}

pub async fn list_chat_sessions(
    pool: &SqlitePool,
    notebook_id: &str,
) -> Result<Vec<ChatSessionSummary>, sqlx::Error> {
    sqlx::query_as::<_, ChatSessionSummary>(
        "SELECT id, title, updated_at FROM kb_chat_sessions WHERE notebook_id = ? ORDER BY updated_at DESC"
    ).bind(notebook_id).fetch_all(pool).await
}

pub async fn load_chat_session_messages(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<ChatMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, ChatMessageRow>(
        "SELECT role, content, tool_calls_json, created_at FROM kb_chat_messages \
         WHERE session_id = ? ORDER BY created_at ASC, rowid ASC"
    ).bind(session_id).fetch_all(pool).await
}

pub async fn delete_chat_session(pool: &SqlitePool, session_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM kb_chat_messages WHERE session_id = ?")
        .bind(session_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kb_chat_sessions WHERE id = ?")
        .bind(session_id).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn create_chat_message(
    pool: &SqlitePool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_calls_json: Option<&str>,
) -> Result<(), sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO kb_chat_messages (id, session_id, role, content, tool_calls_json) \
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&id).bind(session_id).bind(role).bind(content).bind(tool_calls_json)
    .execute(pool).await?;
    sqlx::query("UPDATE kb_chat_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(session_id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;

    async fn setup() -> SqlitePool {
        let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(":memory:"))
            .await.unwrap();
        sqlx::query(
            "CREATE TABLE kb_chat_sessions (
                id TEXT PRIMARY KEY NOT NULL, notebook_id TEXT NOT NULL, title TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE kb_chat_messages (
                id TEXT PRIMARY KEY NOT NULL, session_id TEXT NOT NULL, role TEXT NOT NULL,
                content TEXT NOT NULL, tool_calls_json TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_list_load_roundtrip() {
        let pool = setup().await;
        let id = create_chat_session(&pool, "nb-1", "第一個問題").await.unwrap();

        create_chat_message(&pool, &id, "user", "這份文件在講什麼？", None).await.unwrap();
        create_chat_message(
            &pool, &id, "assistant", "這份文件在講 X。",
            Some(r#"[{"tool":"search_documents","args":{"query":"主題"},"result":"..."}]"#),
        ).await.unwrap();

        let sessions = list_chat_sessions(&pool, "nb-1").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "第一個問題");

        let messages = load_chat_session_messages(&pool, &id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].tool_calls_json.is_some());
    }

    #[tokio::test]
    async fn list_scoped_by_notebook() {
        let pool = setup().await;
        create_chat_session(&pool, "nb-1", "A").await.unwrap();
        create_chat_session(&pool, "nb-2", "B").await.unwrap();

        let sessions = list_chat_sessions(&pool, "nb-1").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "A");
    }

    #[tokio::test]
    async fn delete_removes_session_and_messages() {
        let pool = setup().await;
        let id = create_chat_session(&pool, "nb-1", "temp").await.unwrap();
        create_chat_message(&pool, &id, "user", "hi", None).await.unwrap();

        delete_chat_session(&pool, &id).await.unwrap();

        assert_eq!(list_chat_sessions(&pool, "nb-1").await.unwrap().len(), 0);
        assert_eq!(load_chat_session_messages(&pool, &id).await.unwrap().len(), 0);
    }
}
```

- [ ] **Step 2: 執行測試（應失敗，因為尚未註冊模組）**

Run: `cd src-tauri && cargo test db::kb_chat_sessions`
Expected: 編譯錯誤，`db` 底下找不到 `kb_chat_sessions` 模組（尚未在 `mod.rs` 註冊）。

- [ ] **Step 3: 在 `db/mod.rs` 註冊模組**

在 `src-tauri/src/db/mod.rs`，於 `pub mod knowledge_base;` 之後新增一行：

```rust
pub mod kb_chat_sessions;
```

- [ ] **Step 4: 執行測試（應通過）**

Run: `cd src-tauri && cargo test db::kb_chat_sessions`
Expected: 3 個測試全部 PASS（`create_list_load_roundtrip`、`list_scoped_by_notebook`、`delete_removes_session_and_messages`）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/kb_chat_sessions.rs src-tauri/src/db/mod.rs
git commit -m "feat(kb): add chat session/message CRUD with tests"
```

---

### Task 3: 後端聊天迴圈 — 完成時持久化對話

**Files:**
- Modify: `src-tauri/src/knowledge_base/chat.rs`

- [ ] **Step 1: 新增 `PersistedToolCall` 型別與 `save_chat_turn` 輔助函式**

在 `src-tauri/src/knowledge_base/chat.rs`，於 `KbChatEvent` enum 定義之後（第 60 行之後）插入：

```rust
#[derive(Debug, Clone, Serialize)]
struct PersistedToolCall {
    tool: String,
    args: serde_json::Value,
    result: String,
}

async fn save_chat_turn(
    pool: &SqlitePool,
    chat_session_id: &str,
    user_text: &str,
    assistant_text: &str,
    tool_calls: &[PersistedToolCall],
) {
    let _ = crate::db::kb_chat_sessions::create_chat_message(
        pool, chat_session_id, "user", user_text, None,
    ).await;

    let tool_calls_json = if tool_calls.is_empty() {
        None
    } else {
        serde_json::to_string(tool_calls).ok()
    };

    let _ = crate::db::kb_chat_sessions::create_chat_message(
        pool, chat_session_id, "assistant", assistant_text, tool_calls_json.as_deref(),
    ).await;
}
```

- [ ] **Step 2: `run_chat` 簽章新增 `chat_session_id` 參數**

修改 `run_chat` 簽章（第 164-173 行），在 `session_id: String,` 之後新增一行：

```rust
pub async fn run_chat(
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    chat_session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
```

- [ ] **Step 3: 函式頂層新增累積變數**

在 `let mut conversation = messages;`（第 177 行）之後，`let mut token_estimate = ...` 之前，插入：

```rust
    let last_user_text = conversation.iter().rev()
        .find(|m| m.role == "user")
        .and_then(|m| m.content.as_str())
        .unwrap_or("")
        .to_string();
    let mut persisted_tool_calls: Vec<PersistedToolCall> = Vec::new();
```

- [ ] **Step 4: 迴圈內累積本輪文字**

在 `loop {` 的第一行（第 183 行之後）插入：

```rust
        let mut current_round_text = String::new();
```

修改串流累積迴圈（第 242-250 行），在 `if !chunk.delta.is_empty() {` 區塊內、`app.emit` 之前，新增一行 push：

```rust
        while let Some(chunk) = rx.recv().await {
            if !chunk.delta.is_empty() {
                current_round_text.push_str(&chunk.delta);
                let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                    session_id: session_id.clone(),
                    delta: chunk.delta.clone(),
                });
            }
            if chunk.done { break; }
        }
```

- [ ] **Step 5: 兩個 ToolResult 發送點記錄工具呼叫**

在 XML 工具呼叫分支（第 290-298 行），`KbChatEvent::ToolResult` 發送之後新增：

```rust
                        let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                            session_id: session_id.clone(),
                            call_id: call_id.clone(),
                            content: result_content.clone(),
                            truncated,
                        });
                        persisted_tool_calls.push(PersistedToolCall {
                            tool: tool_name.clone(),
                            args: args.clone(),
                            result: result_content.clone(),
                        });
```

在原生 tool-calling 分支（第 358-368 行），同樣在 `KbChatEvent::ToolResult` 發送之後新增：

```rust
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::ToolResult {
                        session_id: session_id.clone(),
                        call_id: call.id.clone(),
                        content: result_content.clone(),
                        truncated,
                    });
                    persisted_tool_calls.push(PersistedToolCall {
                        tool: call.tool_name.clone(),
                        args: args.clone(),
                        result: result_content.clone(),
                    });
```

（跳過重複呼叫的分支——第 327-348 行「skipped: same call already executed」——不記錄，維持雜訊最小化。）

- [ ] **Step 6: Done 分支寫入持久化，並更新 fallback 呼叫**

修改第 307-312 行區塊：

```rust
                } else {
                    save_chat_turn(&pool, &chat_session_id, &last_user_text, &current_round_text, &persisted_tool_calls).await;
                    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
                        session_id: session_id.clone(),
                    });
                    return Ok(());
                }
```

修改第 266 行 fallback 呼叫，補上 `chat_session_id.clone()`：

```rust
                return run_fallback(pool, notebook, conversation, chat_provider, embedder, session_id, chat_session_id.clone(), locale, app).await;
```

- [ ] **Step 7: `run_fallback` 新增參數並在完成時持久化**

修改 `run_fallback` 簽章（第 387-396 行），新增 `chat_session_id` 參數：

```rust
async fn run_fallback(
    pool: SqlitePool,
    notebook: NotebookRow,
    messages: Vec<ChatMessage>,
    chat_provider: Arc<dyn AiProvider>,
    embedder: Arc<dyn Embedder>,
    session_id: String,
    chat_session_id: String,
    locale: Locale,
    app: AppHandle,
) -> Result<(), AiError> {
```

修改串流迴圈（第 424-436 行）累積回答文字，並在 Done 之前寫入持久化：

```rust
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(32);
    let p = chat_provider.clone();
    let join = tokio::spawn(async move { p.generate(req, tx).await });
    let mut answer_buf = String::new();
    while let Some(chunk) = rx.recv().await {
        if !chunk.delta.is_empty() {
            answer_buf.push_str(&chunk.delta);
            let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::TextDelta {
                session_id: session_id.clone(),
                delta: chunk.delta.clone(),
            });
        }
        if chunk.done { break; }
    }
    let _ = join.await;

    save_chat_turn(&pool, &chat_session_id, &last_user_text, &answer_buf, &[]).await;

    let _ = app.emit(KB_CHAT_EVENT, KbChatEvent::Done {
        session_id: session_id.clone(),
    });
    Ok(())
}
```

（fallback 模式本來就不發送 `ToolCall`/`ToolResult` 事件，維持現況：`tool_calls_json` 傳空陣列，不新增來源引用能力。）

- [ ] **Step 8: 編譯確認**

Run: `cd src-tauri && cargo check --lib`
Expected: 編譯錯誤——`commands/knowledge_base.rs` 呼叫 `run_chat` 時少了 `chat_session_id` 參數。這是預期的，下一個 Task 會修。

- [ ] **Step 9: Commit（先不 push，等 Task 4 修完呼叫端再一起確認整體編譯）**

```bash
git add src-tauri/src/knowledge_base/chat.rs
git commit -m "feat(kb): persist chat turns to DB at stream completion"
```

---

### Task 4: 後端指令 — 新增 4 個 Tauri command，修改 `kb_chat`

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: import 新模組**

在 `src-tauri/src/commands/knowledge_base.rs` 頂部，`use crate::db::knowledge_base as kb_db;`（第 39 行）之後新增：

```rust
use crate::db::kb_chat_sessions::{self, ChatSessionSummary, ChatMessageRow};
```

- [ ] **Step 2: `kb_chat` 新增 `chat_session_id` 參數**

修改第 165-205 行的 `kb_chat`：

```rust
#[tauri::command]
pub async fn kb_chat(
    notebook_id: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    chat_session_id: String,
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
        .await.map_err(|_| AiError::InvalidInput { reason: format!("找不到筆記本: {notebook_id}") })?;

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
        db.pool.clone(), notebook, messages, chat_provider, embedder, session_id, chat_session_id, locale, app,
    ).await
}
```

- [ ] **Step 3: 新增 4 個指令**

在 `kb_chat` 函式結束之後（`kb_open_document` 之前），插入：

```rust
#[tauri::command]
pub async fn kb_create_chat_session(
    notebook_id: String,
    title: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<String, String> {
    kb_chat_sessions::create_chat_session(&db.pool, &notebook_id, &title)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_list_chat_sessions(
    notebook_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<Vec<ChatSessionSummary>, String> {
    kb_chat_sessions::list_chat_sessions(&db.pool, &notebook_id)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_load_chat_session(
    session_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<Vec<ChatMessageRow>, String> {
    kb_chat_sessions::load_chat_session_messages(&db.pool, &session_id)
        .await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_delete_chat_session(
    session_id: String,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
) -> Result<(), String> {
    kb_chat_sessions::delete_chat_session(&db.pool, &session_id)
        .await.map_err(|e| e.to_string())
}
```

- [ ] **Step 4: 在 `lib.rs` 註冊新指令**

修改 `src-tauri/src/lib.rs` 第 27 行的 import：

```rust
    knowledge_base::{
        kb_create_notebook, kb_list_notebooks, kb_delete_notebook, kb_sync_notebook, kb_chat, kb_open_document,
        kb_create_chat_session, kb_list_chat_sessions, kb_load_chat_session, kb_delete_chat_session,
    },
```

修改第 248-254 行的 `generate_handler!` 清單：

```rust
            // Knowledge Base
            kb_create_notebook,
            kb_list_notebooks,
            kb_delete_notebook,
            kb_sync_notebook,
            kb_chat,
            kb_open_document,
            kb_create_chat_session,
            kb_list_chat_sessions,
            kb_load_chat_session,
            kb_delete_chat_session,
```

- [ ] **Step 5: 編譯確認**

Run: `cd src-tauri && cargo check --lib`
Expected: 編譯成功，無錯誤。

Run: `cd src-tauri && cargo test`
Expected: 所有既有測試 + Task 2 新增的 3 個測試全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/lib.rs
git commit -m "feat(kb): add chat session Tauri commands, wire chat_session_id through kb_chat"
```

---

### Task 5: 前端 IPC — 型別與 invoke 包裝

**Files:**
- Modify: `src/ipc/knowledgeBase.ts`
- Modify: `src/ipc/knowledgeBase.test.ts`

- [ ] **Step 1: 新增型別與函式**

在 `src/ipc/knowledgeBase.ts`，於 `SyncSummary` interface（第 15-19 行）之後新增：

```typescript
export interface ChatSessionSummary {
  id: string;
  title: string;
  updated_at: string;
}

export interface ChatMessageRow {
  role: string;
  content: string;
  tool_calls_json: string | null;
  created_at: string;
}
```

修改 `invokeKbChat`（第 64-78 行），新增 `chatSessionId` 參數：

```typescript
export function invokeKbChat(
  notebookId: string,
  messages: ChatMessage[],
  sessionId: string,
  chatSessionId: string,
  providerId?: string | null,
  locale: string = "zh-TW",
): Promise<void> {
  return invoke<void>("kb_chat", {
    notebookId,
    messages,
    sessionId,
    chatSessionId,
    providerId: providerId ?? null,
    locale,
  });
}
```

在檔案末尾（`kbOpenDocument` 之後）新增：

```typescript
export function kbCreateChatSession(notebookId: string, title: string): Promise<string> {
  return invoke<string>("kb_create_chat_session", { notebookId, title });
}

export function kbListChatSessions(notebookId: string): Promise<ChatSessionSummary[]> {
  return invoke<ChatSessionSummary[]>("kb_list_chat_sessions", { notebookId });
}

export function kbLoadChatSession(sessionId: string): Promise<ChatMessageRow[]> {
  return invoke<ChatMessageRow[]>("kb_load_chat_session", { sessionId });
}

export function kbDeleteChatSession(sessionId: string): Promise<void> {
  return invoke<void>("kb_delete_chat_session", { sessionId });
}
```

- [ ] **Step 2: 更新既有測試 + 新增測試**

修改 `src/ipc/knowledgeBase.test.ts` 第 3-5 行的 import：

```typescript
import {
  kbCreateNotebook, kbListNotebooks, kbDeleteNotebook, kbSyncNotebook, invokeKbChat,
  kbCreateChatSession, kbListChatSessions, kbLoadChatSession, kbDeleteChatSession,
} from "./knowledgeBase";
```

修改第 39-49 行既有的 `invokeKbChat` 測試（原本的呼叫少了新參數，會斷言失敗）：

```typescript
  it("invokeKbChat invokes kb_chat with full arg set", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await invokeKbChat("nb-1", [{ role: "user", content: "hi" }], "sess-1", "chat-sess-1", "openai-1", "en");
    expect(invoke).toHaveBeenCalledWith("kb_chat", {
      notebookId: "nb-1",
      messages: [{ role: "user", content: "hi" }],
      sessionId: "sess-1",
      chatSessionId: "chat-sess-1",
      providerId: "openai-1",
      locale: "en",
    });
  });
```

在檔案末尾（`});` 收尾 `describe` 之前）新增：

```typescript
  it("kbCreateChatSession invokes kb_create_chat_session with camelCase args", async () => {
    vi.mocked(invoke).mockResolvedValue("chat-sess-1");
    await kbCreateChatSession("nb-1", "第一個問題");
    expect(invoke).toHaveBeenCalledWith("kb_create_chat_session", {
      notebookId: "nb-1",
      title: "第一個問題",
    });
  });

  it("kbListChatSessions invokes kb_list_chat_sessions with notebookId", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await kbListChatSessions("nb-1");
    expect(invoke).toHaveBeenCalledWith("kb_list_chat_sessions", { notebookId: "nb-1" });
  });

  it("kbLoadChatSession invokes kb_load_chat_session with sessionId", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await kbLoadChatSession("chat-sess-1");
    expect(invoke).toHaveBeenCalledWith("kb_load_chat_session", { sessionId: "chat-sess-1" });
  });

  it("kbDeleteChatSession invokes kb_delete_chat_session with sessionId", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await kbDeleteChatSession("chat-sess-1");
    expect(invoke).toHaveBeenCalledWith("kb_delete_chat_session", { sessionId: "chat-sess-1" });
  });
```

- [ ] **Step 3: 執行測試**

Run: `npx vitest run src/ipc/knowledgeBase.test.ts`
Expected: 9 個測試全部 PASS。

- [ ] **Step 4: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤（此時 `useKnowledgeBaseChat.ts`/`index.tsx` 尚未更新呼叫端，若有型別錯誤屬預期，下個 Task 會修）。

- [ ] **Step 5: Commit**

```bash
git add src/ipc/knowledgeBase.ts src/ipc/knowledgeBase.test.ts
git commit -m "feat(kb): add chat session IPC bindings and tests"
```

---

### Task 6: 前端 Hook — session 管理

**Files:**
- Modify: `src/hooks/useKnowledgeBaseChat.ts`

- [ ] **Step 1: 更新 import 與型別**

修改 `src/hooks/useKnowledgeBaseChat.ts` 第 1-13 行：

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { formatAiError, type AiError, type ChatMessage } from "../ipc/ai";
import {
  KB_CHAT_EVENT, invokeKbChat, type KbChatEvent,
  kbCreateChatSession, kbListChatSessions, kbLoadChatSession, kbDeleteChatSession,
  type ChatSessionSummary, type ChatMessageRow,
} from "../ipc/knowledgeBase";
import { useLocale } from "../contexts/LocaleContext";
import type { ToolCallState } from "./useCodeAssistant";

export interface KbMessage {
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCallState[];
  checkpoints?: number[];
  streaming?: boolean;
}

const TITLE_MAX_LEN = 30;

function truncateTitle(text: string): string {
  const trimmed = text.trim();
  return trimmed.length > TITLE_MAX_LEN ? `${trimmed.slice(0, TITLE_MAX_LEN)}…` : trimmed;
}

interface PersistedToolCall {
  tool: string;
  args: Record<string, unknown>;
  result: string;
}

export function reconstructKbMessages(rows: ChatMessageRow[]): KbMessage[] {
  return rows.map((r, i) => {
    let toolCalls: ToolCallState[] | undefined;
    if (r.tool_calls_json) {
      try {
        const parsed = JSON.parse(r.tool_calls_json) as PersistedToolCall[];
        toolCalls = parsed.map((tc, j) => ({
          callId: `restored-${i}-${j}`,
          tool: tc.tool,
          args: tc.args,
          result: { content: tc.result, truncated: false },
        }));
      } catch {
        toolCalls = undefined;
      }
    }
    return {
      role: r.role === "user" ? "user" : "assistant",
      content: r.content,
      toolCalls,
    };
  });
}
```

（`KbMessage` interface 移到這裡且內容與現有版本相同，只是連同新增的 `truncateTitle`/`reconstructKbMessages` 一起放在檔案頂部；原本第 8-13 行的 `KbMessage` 定義整段被上面這段取代。）

- [ ] **Step 2: 更新 `UseKnowledgeBaseChatResult` 與 hook 內部 state**

修改第 15-24 行的 result interface：

```typescript
export interface UseKnowledgeBaseChatResult {
  messages: KbMessage[];
  isStreaming: boolean;
  error: string | null;
  isFallbackMode: boolean;
  tokenCount: number;
  tokenLimit: number;
  sessions: ChatSessionSummary[];
  activeChatSessionId: string | null;
  send: (userText: string, providerId?: string) => Promise<void>;
  clear: () => void;
  loadSession: (sessionId: string) => Promise<void>;
  deleteSession: (sessionId: string) => Promise<void>;
}
```

修改第 26-47 行的 hook 開頭（state 宣告與筆記本切換 effect）：

```typescript
export function useKnowledgeBaseChat(notebookId: string | null): UseKnowledgeBaseChatResult {
  const [messages, setMessages] = useState<KbMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFallbackMode, setIsFallbackMode] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [tokenLimit, setTokenLimit] = useState(50000);
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [activeChatSessionId, setActiveChatSessionId] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const { locale } = useLocale();

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refreshSessions = useCallback(async (nbId: string) => {
    try {
      const list = await kbListChatSessions(nbId);
      if (mountedRef.current) setSessions(list);
    } catch { /* ignore */ }
  }, []);

  // 切換筆記本時重置對話狀態，避免把上一個筆記本的對話帶到新的筆記本；
  // 重新載入該筆記本自己的對話記錄清單，但不自動接續舊對話。
  useEffect(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
    setActiveChatSessionId(null);
    setSessions([]);
    if (notebookId) void refreshSessions(notebookId);
  }, [notebookId, refreshSessions]);
```

- [ ] **Step 3: `send()` 新增延遲建立 session 邏輯**

修改 `send`（原第 49-138 行），在函式最前面（`setError(null);` 之後、組出 `chatMessages` 之前）新增延遲建立區塊，並更新 `invokeKbChat` 呼叫與依賴陣列：

```typescript
  const send = useCallback(async (userText: string, providerId?: string) => {
    if (!userText.trim() || isStreaming || !notebookId) return;
    setError(null);

    let chatSessionId = activeChatSessionId;
    if (!chatSessionId) {
      try {
        chatSessionId = await kbCreateChatSession(notebookId, truncateTitle(userText));
      } catch (e) {
        setError(String(e));
        return;
      }
      setActiveChatSessionId(chatSessionId);
      void refreshSessions(notebookId);
    }

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
      } else if (p.kind === "checkpoint") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.checkpoints = [...(last.checkpoints ?? []), p.number];
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
        void refreshSessions(notebookId);
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
      await invokeKbChat(notebookId, chatMessages, sessionId, chatSessionId, providerId, locale);
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
  }, [messages, isStreaming, locale, notebookId, activeChatSessionId, refreshSessions]);
```

- [ ] **Step 4: 更新 `clear`，新增 `loadSession`/`deleteSession`**

修改第 140-148 行的 `clear` 與 `return`：

```typescript
  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
    setActiveChatSessionId(null);
  }, []);

  const loadSession = useCallback(async (sessionId: string) => {
    if (isStreaming) return;
    const rows = await kbLoadChatSession(sessionId);
    setMessages(reconstructKbMessages(rows));
    setActiveChatSessionId(sessionId);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
  }, [isStreaming]);

  const deleteSession = useCallback(async (sessionId: string) => {
    await kbDeleteChatSession(sessionId);
    if (activeChatSessionId === sessionId) {
      setMessages([]);
      setActiveChatSessionId(null);
    }
    if (notebookId) void refreshSessions(notebookId);
  }, [activeChatSessionId, notebookId, refreshSessions]);

  return {
    messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit,
    sessions, activeChatSessionId, send, clear, loadSession, deleteSession,
  };
}
```

- [ ] **Step 5: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤。

- [ ] **Step 6: Commit**

```bash
git add src/hooks/useKnowledgeBaseChat.ts
git commit -m "feat(kb): add lazy session creation, load, and delete to chat hook"
```

---

### Task 7: 前端元件 — `ChatHistorySidebar`（含匯出）

**Files:**
- Create: `src/components/KnowledgeBaseView/ChatHistorySidebar.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 新增 i18n 字串**

在 `src/lib/i18n.ts`，於 `kb_checkpoint_notice`（第 204 行附近，zh-TW 區塊）之後新增：

```typescript
    kb_chat_history_title: "對話記錄",
    kb_new_conversation: "+ 新對話",
    kb_no_conversations: "尚無對話記錄",
    kb_delete_conversation_confirm: (title: string) => `確定要刪除對話「${title}」嗎？`,
    kb_export_conversation: "匯出成 md",
```

在對應的 en 區塊（`kb_checkpoint_notice` 的 en 版本之後）新增：

```typescript
    kb_chat_history_title: "Chat History",
    kb_new_conversation: "+ New Conversation",
    kb_no_conversations: "No conversations yet",
    kb_delete_conversation_confirm: (title: string) => `Delete conversation "${title}"?`,
    kb_export_conversation: "Export as .md",
```

- [ ] **Step 2: 建立元件**

Create `src/components/KnowledgeBaseView/ChatHistorySidebar.tsx`:

```tsx
import { save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "../../ipc/fs";
import { kbLoadChatSession, type ChatSessionSummary } from "../../ipc/knowledgeBase";
import { reconstructKbMessages, type KbMessage } from "../../hooks/useKnowledgeBaseChat";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  width: number;
  notebookName: string;
  sessions: ChatSessionSummary[];
  activeSessionId: string | null;
  onNew: () => void;
  onSelect: (sessionId: string) => void;
  onDelete: (sessionId: string) => void;
}

function formatSqliteTimestamp(ts: string): string {
  const iso = `${ts.replace(" ", "T")}Z`;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? ts : d.toLocaleString();
}

function buildExportMarkdown(notebookName: string, title: string, messages: KbMessage[]): string {
  const lines: string[] = [`# ${title}\n`, `**筆記本：** \`${notebookName}\`\n`];
  for (const msg of messages) {
    if (msg.role === "user") {
      lines.push(`\n---\n\n**問：** ${msg.content}\n`);
    } else {
      if ((msg.toolCalls ?? []).length > 0) {
        lines.push("\n**工具調用：**\n");
        for (const tc of msg.toolCalls!) {
          lines.push(`- \`${tc.tool}\`（${JSON.stringify(tc.args)}）`);
        }
        lines.push("");
      }
      if (msg.content) {
        lines.push(`\n**答：**\n\n${msg.content}\n`);
      }
    }
  }
  return lines.join("\n");
}

export function ChatHistorySidebar({
  width, notebookName, sessions, activeSessionId, onNew, onSelect, onDelete,
}: Props) {
  const { t } = useLocale();

  const handleExport = async (session: ChatSessionSummary) => {
    const rows = await kbLoadChatSession(session.id);
    const messages = reconstructKbMessages(rows);
    const path = await save({
      defaultPath: `${session.title}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    await writeTextFile(path, buildExportMarkdown(notebookName, session.title, messages));
  };

  return (
    <div className="kb-chat-history" style={{ width }}>
      <div className="kb-chat-history__header">
        <span className="kb-chat-history__title">{t.kb_chat_history_title}</span>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onNew}>
          {t.kb_new_conversation}
        </button>
      </div>
      <div className="kb-chat-history__list">
        {sessions.length === 0 && (
          <div className="kb-chat-history__empty">{t.kb_no_conversations}</div>
        )}
        {sessions.map((s) => (
          <div
            key={s.id}
            className={`kb-chat-history__item ${s.id === activeSessionId ? "kb-chat-history__item--active" : ""}`}
          >
            <button className="kb-chat-history__item-main" onClick={() => onSelect(s.id)}>
              <div className="kb-chat-history__item-title" title={s.title}>{s.title}</div>
              <div className="kb-chat-history__item-time">{formatSqliteTimestamp(s.updated_at)}</div>
            </button>
            <div className="kb-chat-history__item-actions">
              <button
                className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                title={t.kb_export_conversation}
                onClick={(e) => { e.stopPropagation(); void handleExport(s); }}
              >
                ↓
              </button>
              <button
                className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                onClick={(e) => {
                  e.stopPropagation();
                  if (window.confirm(t.kb_delete_conversation_confirm(s.title))) onDelete(s.id);
                }}
              >
                ✕
              </button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 3: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤。

- [ ] **Step 4: Commit**

```bash
git add src/components/KnowledgeBaseView/ChatHistorySidebar.tsx src/lib/i18n.ts
git commit -m "feat(kb): add chat history sidebar component with markdown export"
```

---

### Task 8: 前端整合 — 掛載側邊欄與拖曳調整寬度

**Files:**
- Modify: `src/components/KnowledgeBaseView/index.tsx`
- Modify: `src/components/KnowledgeBaseView/styles.css`

- [ ] **Step 1: 新增樣式**

在 `src/components/KnowledgeBaseView/styles.css`，於 `.kb-main` 區塊（第 81-86 行）之後新增：

```css
/* ── Chat history sidebar (right) ────────────────────────────────── */
.kb-chat-history-resizer {
  width: 6px;
  cursor: col-resize;
  background-color: transparent;
  transition: background-color 0.2s;
  flex-shrink: 0;
}
.kb-chat-history-resizer:hover {
  background-color: rgba(168, 85, 247, 0.4);
}
.kb-chat-history {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-left: 1px solid #1e1e1e;
  background: #0c0c0c;
  overflow: hidden;
}
.kb-chat-history__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px;
  border-bottom: 1px solid #1e1e1e;
}
.kb-chat-history__title {
  font-size: 12px;
  font-weight: 600;
  color: #999;
}
.kb-chat-history__list {
  flex: 1;
  overflow-y: auto;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.kb-chat-history__empty {
  padding: 20px 10px;
  text-align: center;
  color: #555;
  font-size: 12px;
}
.kb-chat-history__item {
  display: flex;
  align-items: center;
  border: 1px solid transparent;
  border-radius: 6px;
  padding: 4px 6px;
  gap: 4px;
}
.kb-chat-history__item--active {
  border-color: rgba(168, 85, 247, 0.35);
  background: rgba(168, 85, 247, 0.08);
}
.kb-chat-history__item-main {
  flex: 1;
  min-width: 0;
  text-align: left;
  background: transparent;
  border: none;
  cursor: pointer;
  padding: 2px 4px;
  color: inherit;
}
.kb-chat-history__item-title {
  font-size: 12px;
  color: var(--text-primary, #e0e0e0);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.kb-chat-history__item-time {
  font-size: 10px;
  color: #555;
  margin-top: 2px;
}
.kb-chat-history__item-actions {
  display: flex;
  gap: 2px;
  flex-shrink: 0;
}
```

- [ ] **Step 2: `index.tsx` — 新增寬度狀態與拖曳邏輯**

在 `src/components/KnowledgeBaseView/index.tsx` 第 1-17 行的 import 區塊，新增：

```typescript
import { ChatHistorySidebar } from "./ChatHistorySidebar";
```

（放在 `NotebookCreateDialog` import 之後即可。）

在 `STORAGE_KEY`/`loadSavedNotebookId`/`saveNotebookId`（第 18-28 行）之後新增：

```typescript
const HISTORY_WIDTH_KEY = "aiterm-knowledge-base-history-width";

function loadSavedHistoryWidth(): number {
  try {
    const raw = localStorage.getItem(HISTORY_WIDTH_KEY);
    const n = raw ? parseInt(raw, 10) : NaN;
    return Number.isFinite(n) ? n : 280;
  } catch { return 280; }
}
function saveHistoryWidth(width: number) {
  try { localStorage.setItem(HISTORY_WIDTH_KEY, String(width)); } catch { /* ignore */ }
}
```

在 `KnowledgeBaseView` 函式內，第 84-85 行（`useKnowledgeBaseChat` 呼叫）替換為，並新增寬度拖曳相關 state/ref/effect：

```typescript
  const activeNotebook = notebooks.find((nb) => nb.id === activeNotebookId) ?? null;
  const {
    messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit,
    sessions, activeChatSessionId, send, clear, loadSession, deleteSession,
  } = useKnowledgeBaseChat(activeNotebookId);

  const containerRef = useRef<HTMLDivElement>(null);
  const [historyWidth, setHistoryWidth] = useState(loadSavedHistoryWidth);
  const [isResizingHistory, setIsResizingHistory] = useState(false);

  useEffect(() => {
    if (!isResizingHistory) {
      document.body.style.userSelect = "";
      return;
    }
    document.body.style.userSelect = "none";
    const onMouseMove = (e: MouseEvent) => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const newWidth = rect.right - e.clientX;
      setHistoryWidth(Math.max(220, Math.min(newWidth, 480)));
    };
    const onMouseUp = () => setIsResizingHistory(false);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      document.body.style.userSelect = "";
    };
  }, [isResizingHistory]);

  useEffect(() => { saveHistoryWidth(historyWidth); }, [historyWidth]);
```

- [ ] **Step 3: 掛載側邊欄與 resizer**

修改 return 區塊的最外層 `<div className="kb-view">`（第 157-158 行），加上 `ref`：

```tsx
    <div className="kb-view" ref={containerRef}>
```

修改 `.kb-main` 結束之後（第 312 行，`</div>` 之後、`{showCreateDialog && ...}` 之前），新增：

```tsx
      {activeNotebook && (
        <>
          <div
            className="kb-chat-history-resizer"
            onMouseDown={(e) => { e.preventDefault(); setIsResizingHistory(true); }}
          />
          <ChatHistorySidebar
            width={historyWidth}
            notebookName={activeNotebook.name}
            sessions={sessions}
            activeSessionId={activeChatSessionId}
            onNew={clear}
            onSelect={loadSession}
            onDelete={deleteSession}
          />
        </>
      )}
```

- [ ] **Step 4: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤。

- [ ] **Step 5: Commit**

```bash
git add src/components/KnowledgeBaseView/index.tsx src/components/KnowledgeBaseView/styles.css
git commit -m "feat(kb): mount resizable chat history sidebar in KnowledgeBaseView"
```

---

### Task 9: 整體驗證

**Files:** 無新增/修改，純驗證步驟。

- [ ] **Step 1: 全量後端測試**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS，無新增失敗。

- [ ] **Step 2: 全量前端測試**

Run: `npx vitest run`
Expected: 全部 PASS，無新增失敗。

- [ ] **Step 3: 型別檢查**

Run: `npx tsc --noEmit`
Expected: 無錯誤。

- [ ] **Step 4: Lint**

Run: `npm run lint`
Expected: 無新增錯誤。

- [ ] **Step 5: 手動測試（dev server）**

Run: `npm run tauri:dev`（若已在背景執行則略過，等待自動重新編譯完成）

手動驗證流程：
1. 開啟知識庫分頁，選擇一個已同步的筆記本，確認右側出現「對話記錄」側邊欄（空清單提示文字）。
2. 送出一則問題，確認：(a) 右側清單立即出現一筆新記錄，標題為問題前段文字；(b) 該筆記錄有反白 active 樣式。
3. 點側邊欄「+ 新對話」，確認畫面清空；再送出另一則問題，確認清單出現第二筆記錄，且第一筆記錄仍在。
4. 點擊第一筆記錄，確認畫面正確還原之前的問答內容與來源引用 chip。
5. 拖曳側邊欄與主畫面之間的分隔線，確認寬度可調整；重新整理頁面後寬度維持上次調整的值。
6. 點任一記錄的「↓」匯出按鈕，確認跳出存檔對話框，存檔後開啟該 md 檔案內容正確。
7. 點任一記錄的「✕」，確認跳出確認對話框；確認後該記錄從清單移除；若刪除的是目前開啟中的對話，畫面清空。
8. 切換到另一個筆記本，確認側邊欄清單只顯示該筆記本自己的對話記錄。
9. 重新啟動 app（或重新整理），確認對話記錄仍然存在（persist 到 SQLite 生效）。

- [ ] **Step 6: 回報結果**

若手動測試全數通過，回報使用者「KB 對話歷史記錄功能已完成，通過全部自動化測試與手動驗證」；若發現問題，記錄具體重現步驟後進入除錯（`superpowers:systematic-debugging`）。

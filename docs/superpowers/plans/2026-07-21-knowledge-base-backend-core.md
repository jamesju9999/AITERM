# 知識庫（Knowledge Base）後端核心 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立知識庫功能的後端儲存層與文件擷取管線——本地 SQLite 儲存、Markdown 切片、多 provider embedding、資料夾同步（掃描/轉檔/切片/向量化/寫入）與 cosine similarity 搜尋。完成後可用 Rust 測試獨立驗證整條管線，尚不包含 Agent 問答迴圈與前端 UI（見 Plan B）。

**Architecture:** 比照既有的 `db/loop_sessions.rs`（本地 SQLite,透過 `sqlx::SqlitePool`）與 `code_assistant/`（獨立子模組 + Tauri command 薄層）兩個既有模式。新增 `db/knowledge_base.rs`（schema + CRUD + 向量搜尋）與 `knowledge_base/`（chunk 切片、embedding provider 抽象、擷取管線),透過 `async_trait` 定義 `DocumentConverter` 與 `Embedder` trait 讓管線邏輯可用 fake 實作測試,不需要真的呼叫 Python/MarkItDown 或外部 embedding API。文件轉換重用既有的 `commands::markitdown::markitdown_convert`。

**Tech Stack:** Rust、`sqlx`（SQLite,已是既有依賴）、`async-trait`（已是既有依賴）、`sha2`（已是既有依賴,用於 content hash)、`wiremock` + `tempfile`（測試,已是既有 dev-dependency）。不新增任何 Cargo.toml 依賴。

---

## 參考檔案（實作前请先讀過)

- `src-tauri/src/db/loop_sessions.rs` — SQLite pool 建立、schema、CRUD 函式的範本
- `src-tauri/src/commands/loop_session.rs` — Tauri command 包裝範本
- `src-tauri/src/commands/markitdown.rs` — 文件轉 Markdown 的既有實作（本 plan 直接重用 `markitdown_convert`)
- `src-tauri/src/config/types.rs` — `ProviderConfig` / `ProviderType`
- `src-tauri/src/ai/mod.rs` — `async_trait` 用法範例（`AiProvider` trait)
- `src-tauri/tests/db_design_integration.rs` — SQLite 整合測試範本（in-memory pool 手動建表)
- `src-tauri/tests/ollama_client.rs` — `wiremock` 測試範本

---

## Task 1: SQLite Schema 與連線設定

**Files:**
- Create: `src-tauri/src/db/knowledge_base.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Test: `src-tauri/tests/db_knowledge_base_integration.rs`

- [ ] **Step 1: 寫失敗測試（驗證三張表可以建立並寫入/讀出一筆 notebook）**

```rust
// src-tauri/tests/db_knowledge_base_integration.rs
use sqlx::sqlite::SqlitePoolOptions;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory DB");

    sqlx::query(
        "CREATE TABLE notebooks (
            id TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            folder_path TEXT NOT NULL,
            embed_provider_id TEXT,
            embed_model TEXT,
            embed_dim INTEGER,
            last_synced_at INTEGER,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("create notebooks table");

    sqlx::query(
        "CREATE TABLE documents (
            id TEXT PRIMARY KEY NOT NULL,
            notebook_id TEXT NOT NULL,
            rel_path TEXT NOT NULL,
            mtime INTEGER NOT NULL,
            content_hash TEXT NOT NULL,
            markdown_cache TEXT,
            status TEXT NOT NULL DEFAULT 'ok',
            error_message TEXT,
            UNIQUE(notebook_id, rel_path)
        )"
    ).execute(&pool).await.expect("create documents table");

    sqlx::query(
        "CREATE TABLE chunks (
            id TEXT PRIMARY KEY NOT NULL,
            document_id TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            text TEXT NOT NULL,
            location_hint TEXT,
            embedding BLOB NOT NULL
        )"
    ).execute(&pool).await.expect("create chunks table");

    pool
}

#[tokio::test]
async fn schema_allows_notebook_insert_and_select() {
    let pool = setup_pool().await;

    sqlx::query("INSERT INTO notebooks (id, name, folder_path) VALUES (?, ?, ?)")
        .bind("nb-1").bind("My Notebook").bind("/tmp/docs")
        .execute(&pool).await.expect("insert notebook");

    let row: (String, String, String) = sqlx::query_as(
        "SELECT id, name, folder_path FROM notebooks WHERE id = ?"
    ).bind("nb-1").fetch_one(&pool).await.expect("select notebook");

    assert_eq!(row.0, "nb-1");
    assert_eq!(row.1, "My Notebook");
    assert_eq!(row.2, "/tmp/docs");
}
```

- [ ] **Step 2: 執行測試,確認通過（此步驟只驗證手動 schema 本身沒有語法錯誤,尚未涉及 `db/knowledge_base.rs`)**

Run: `cd src-tauri && cargo test --test db_knowledge_base_integration -- --nocapture`
Expected: PASS（1 passed）

- [ ] **Step 3: 建立 `KnowledgeBaseDb` 與對應的 row struct,schema 需與測試中手動建的表完全一致**

```rust
// src-tauri/src/db/knowledge_base.rs
use sqlx::{SqlitePool, FromRow};
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::fs;

pub struct KnowledgeBaseDb {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct NotebookRow {
    pub id: String,
    pub name: String,
    pub folder_path: String,
    pub embed_provider_id: Option<String>,
    pub embed_model: Option<String>,
    pub embed_dim: Option<i64>,
    pub last_synced_at: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct DocumentRow {
    pub id: String,
    pub notebook_id: String,
    pub rel_path: String,
    pub mtime: i64,
    pub content_hash: String,
    pub markdown_cache: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
}

impl KnowledgeBaseDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("knowledge_base.db");
        let url = format!("sqlite:{}", db_path.to_string_lossy());
        let pool = SqlitePool::connect(&url).await.unwrap_or_else(|_| {
            SqlitePool::connect_lazy("sqlite::memory:").unwrap()
        });
        let db = Self { pool };
        db.init().await.ok();
        db
    }

    async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS notebooks (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                folder_path TEXT NOT NULL,
                embed_provider_id TEXT,
                embed_model TEXT,
                embed_dim INTEGER,
                last_synced_at INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY NOT NULL,
                notebook_id TEXT NOT NULL,
                rel_path TEXT NOT NULL,
                mtime INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                markdown_cache TEXT,
                status TEXT NOT NULL DEFAULT 'ok',
                error_message TEXT,
                UNIQUE(notebook_id, rel_path)
            )"
        ).execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY NOT NULL,
                document_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                text TEXT NOT NULL,
                location_hint TEXT,
                embedding BLOB NOT NULL
            )"
        ).execute(&self.pool).await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_documents_notebook ON documents(notebook_id)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_chunks_document ON chunks(document_id)")
            .execute(&self.pool).await?;

        Ok(())
    }
}
```

- [ ] **Step 4: 在 `db/mod.rs` 註冊新模組**

```rust
// src-tauri/src/db/mod.rs — 在既有 pub mod 清單中加入這一行
pub mod knowledge_base;
```

- [ ] **Step 5: 執行 `cargo build` 確認編譯通過**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功,無錯誤

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/knowledge_base.rs src-tauri/src/db/mod.rs src-tauri/tests/db_knowledge_base_integration.rs
git commit -m "feat(knowledge-base): add SQLite schema for notebooks/documents/chunks"
```

---

## Task 2: Notebook CRUD 函式

**Files:**
- Modify: `src-tauri/src/db/knowledge_base.rs`
- Modify: `src-tauri/tests/db_knowledge_base_integration.rs`

- [ ] **Step 1: 寫失敗測試（create → get → list → update sync status → delete 完整流程）**

在 `src-tauri/tests/db_knowledge_base_integration.rs` 中加入（保留既有的 `setup_pool` 與第一個測試）：

```rust
use aiterm_lib::db::knowledge_base::{
    create_notebook, get_notebook, list_notebooks, delete_notebook, mark_synced,
};

#[tokio::test]
async fn notebook_crud_roundtrip() {
    let pool = setup_pool().await;

    let created = create_notebook(&pool, "My Docs", "/tmp/docs", Some("ollama-local"), Some("nomic-embed-text"))
        .await.expect("create notebook");
    assert_eq!(created.name, "My Docs");
    assert_eq!(created.folder_path, "/tmp/docs");
    assert_eq!(created.embed_provider_id.as_deref(), Some("ollama-local"));
    assert!(created.last_synced_at.is_none());

    let fetched = get_notebook(&pool, &created.id).await.expect("get notebook");
    assert_eq!(fetched.id, created.id);

    let list = list_notebooks(&pool).await.expect("list notebooks");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, created.id);

    mark_synced(&pool, &created.id, 1_700_000_000).await.expect("mark synced");
    let after_sync = get_notebook(&pool, &created.id).await.expect("get after sync");
    assert_eq!(after_sync.last_synced_at, Some(1_700_000_000));

    delete_notebook(&pool, &created.id).await.expect("delete notebook");
    let list_after_delete = list_notebooks(&pool).await.expect("list after delete");
    assert!(list_after_delete.is_empty());
}
```

- [ ] **Step 2: 執行測試,確認因函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --test db_knowledge_base_integration -- --nocapture`
Expected: 編譯錯誤 `cannot find function 'create_notebook' in module`

- [ ] **Step 3: 在 `db/knowledge_base.rs` 加入 CRUD 函式**

```rust
// 加在 src-tauri/src/db/knowledge_base.rs 檔案末尾

pub async fn create_notebook(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    embed_provider_id: Option<&str>,
    embed_model: Option<&str>,
) -> Result<NotebookRow, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notebooks (id, name, folder_path, embed_provider_id, embed_model)
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&id).bind(name).bind(folder_path).bind(embed_provider_id).bind(embed_model)
    .execute(pool).await?;

    get_notebook(pool, &id).await
}

pub async fn get_notebook(pool: &SqlitePool, id: &str) -> Result<NotebookRow, sqlx::Error> {
    sqlx::query_as::<_, NotebookRow>(
        "SELECT id, name, folder_path, embed_provider_id, embed_model, embed_dim, last_synced_at, created_at
         FROM notebooks WHERE id = ?"
    ).bind(id).fetch_one(pool).await
}

pub async fn list_notebooks(pool: &SqlitePool) -> Result<Vec<NotebookRow>, sqlx::Error> {
    sqlx::query_as::<_, NotebookRow>(
        "SELECT id, name, folder_path, embed_provider_id, embed_model, embed_dim, last_synced_at, created_at
         FROM notebooks ORDER BY created_at DESC"
    ).fetch_all(pool).await
}

pub async fn delete_notebook(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE notebook_id = ?)"
    ).bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM documents WHERE notebook_id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM notebooks WHERE id = ?").bind(id).execute(pool).await?;
    Ok(())
}

pub async fn update_embed_settings(
    pool: &SqlitePool,
    id: &str,
    embed_provider_id: &str,
    embed_model: &str,
    embed_dim: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE notebooks SET embed_provider_id = ?, embed_model = ?, embed_dim = ? WHERE id = ?"
    ).bind(embed_provider_id).bind(embed_model).bind(embed_dim).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn mark_synced(pool: &SqlitePool, id: &str, ts: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE notebooks SET last_synced_at = ? WHERE id = ?")
        .bind(ts).bind(id).execute(pool).await?;
    Ok(())
}
```

- [ ] **Step 4: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --test db_knowledge_base_integration -- --nocapture`
Expected: PASS（2 passed）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/knowledge_base.rs src-tauri/tests/db_knowledge_base_integration.rs
git commit -m "feat(knowledge-base): add notebook CRUD functions"
```

---

## Task 3: 筆記本管理 Tauri Commands

**Files:**
- Create: `src-tauri/src/commands/knowledge_base.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 建立 command 檔案**

```rust
// src-tauri/src/commands/knowledge_base.rs
use tauri::State;
use crate::db::knowledge_base::{
    KnowledgeBaseDb, NotebookRow,
    create_notebook, list_notebooks, delete_notebook,
};

#[tauri::command]
pub async fn kb_create_notebook(
    name: String,
    folder_path: String,
    embed_provider_id: Option<String>,
    embed_model: Option<String>,
    db: State<'_, KnowledgeBaseDb>,
) -> Result<NotebookRow, String> {
    create_notebook(
        &db.pool, &name, &folder_path,
        embed_provider_id.as_deref(), embed_model.as_deref(),
    ).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_list_notebooks(db: State<'_, KnowledgeBaseDb>) -> Result<Vec<NotebookRow>, String> {
    list_notebooks(&db.pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_delete_notebook(id: String, db: State<'_, KnowledgeBaseDb>) -> Result<(), String> {
    delete_notebook(&db.pool, &id).await.map_err(|e| e.to_string())
}
```

- [ ] **Step 2: 在 `commands/mod.rs` 註冊模組**

```rust
// src-tauri/src/commands/mod.rs — 在既有 pub mod 清單中加入
pub mod knowledge_base;
```

- [ ] **Step 3: 在 `lib.rs` 建立 DB 實例、`.manage()` 並加入 `invoke_handler`**

在 `lib.rs` 建立 `loop_session_db` 的那兩處附近（`let loop_session_db = ...` 之後、`.manage(loop_session_db)` 之後）分別加入：

```rust
// 緊接在 let loop_session_db = ... 之後
let kb_db = tauri::async_runtime::block_on(async { db::knowledge_base::KnowledgeBaseDb::new().await });
```

```rust
// 緊接在 .manage(loop_session_db) 之後
.manage(kb_db)
```

在 `invoke_handler!` 巨集清單中,`code_assistant_chat,` 那一行之後加入：

```rust
            kb_create_notebook,
            kb_list_notebooks,
            kb_delete_notebook,
```

- [ ] **Step 4: 編譯確認**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(knowledge-base): add notebook management Tauri commands"
```

---

## Task 4: Markdown 切片邏輯

**Files:**
- Create: `src-tauri/src/knowledge_base/mod.rs`
- Create: `src-tauri/src/knowledge_base/chunk.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 建立模組檔案（先建空殼讓測試能編譯）**

```rust
// src-tauri/src/knowledge_base/mod.rs
pub mod chunk;
```

```rust
// src-tauri/src/knowledge_base/chunk.rs
// (空檔案,下一步驟才寫實作)
```

在 `lib.rs` 的 `pub mod` 清單中（`pub mod code_assistant;` 那一行附近）加入：

```rust
pub mod knowledge_base;
```

- [ ] **Step 2: 寫失敗測試**

```rust
// src-tauri/src/knowledge_base/chunk.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_markdown_becomes_single_chunk() {
        let md = "# Title\n\nSome short content.";
        let chunks = chunk_markdown(md);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].location_hint.as_deref(), Some("Title"));
        assert!(chunks[0].text.contains("Some short content."));
    }

    #[test]
    fn empty_markdown_returns_no_chunks() {
        let chunks = chunk_markdown("   \n  ");
        assert!(chunks.is_empty());
    }

    #[test]
    fn long_markdown_splits_with_overlap() {
        // 產生遠超過 TARGET_CHUNK_CHARS 的內容
        let paragraph = "這是一段測試內容，用來確保切片邏輯能正確處理長文件。";
        let mut md = String::from("# 第一章\n\n");
        for _ in 0..200 {
            md.push_str(paragraph);
            md.push('\n');
        }

        let chunks = chunk_markdown(&md);
        assert!(chunks.len() > 1, "long content should split into multiple chunks");

        // 每個 chunk 都應該標記正確的最近標題
        for c in &chunks {
            assert_eq!(c.location_hint.as_deref(), Some("第一章"));
        }

        // 檢查有 overlap：後一個 chunk 的開頭應該與前一個 chunk 的結尾有重疊字元
        let first_tail: String = chunks[0].text.chars().rev().take(50).collect();
        let first_tail_reversed: String = first_tail.chars().rev().collect();
        assert!(
            chunks[1].text.contains(&first_tail_reversed[..20]),
            "expected overlap between consecutive chunks"
        );
    }

    #[test]
    fn location_hint_tracks_nearest_heading() {
        let md = "# A\n\ncontent under A\n\n## B\n\ncontent under B";
        let chunks = chunk_markdown(md);
        // 內容量小，只會產生一個 chunk，但驗證切片函式至少能正確解析最後一個標題
        assert_eq!(chunks.last().unwrap().location_hint.as_deref(), Some("B"));
    }
}
```

- [ ] **Step 3: 執行測試,確認因 `chunk_markdown` 未定義而失敗**

Run: `cd src-tauri && cargo test --lib knowledge_base::chunk -- --nocapture`
Expected: 編譯錯誤 `cannot find function 'chunk_markdown'`

- [ ] **Step 4: 實作切片邏輯（加在測試模組之前）**

```rust
// src-tauri/src/knowledge_base/chunk.rs — 加在檔案最前面（#[cfg(test)] mod tests 之前）

const TARGET_CHUNK_CHARS: usize = 3200;
const OVERLAP_CHARS: usize = 600;

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub location_hint: Option<String>,
}

/// 將轉換後的 markdown 切成帶重疊的片段，優先沿標題邊界累積，
/// 超過 TARGET_CHUNK_CHARS 就切一刀，並保留 OVERLAP_CHARS 字元的重疊
/// 給下一個 chunk 以維持上下文連續性。
pub fn chunk_markdown(markdown: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current_heading: Option<String> = None;
    let mut buffer = String::new();
    let mut buffer_heading: Option<String> = None;

    for line in markdown.lines() {
        if let Some(heading) = parse_heading(line) {
            current_heading = Some(heading);
        }
        if buffer.is_empty() {
            buffer_heading = current_heading.clone();
        }
        buffer.push_str(line);
        buffer.push('\n');

        if buffer.chars().count() >= TARGET_CHUNK_CHARS {
            chunks.push(Chunk {
                text: buffer.trim_end().to_string(),
                location_hint: buffer_heading.clone(),
            });
            buffer = tail_chars(&buffer, OVERLAP_CHARS);
            buffer_heading = current_heading.clone();
        }
    }

    if !buffer.trim().is_empty() {
        chunks.push(Chunk {
            text: buffer.trim_end().to_string(),
            location_hint: buffer_heading,
        });
    }

    chunks
}

fn parse_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let text = trimmed.trim_start_matches('#').trim();
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    None
}

fn tail_chars(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}
```

- [ ] **Step 5: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --lib knowledge_base::chunk -- --nocapture`
Expected: PASS（4 passed）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/knowledge_base/mod.rs src-tauri/src/knowledge_base/chunk.rs src-tauri/src/lib.rs
git commit -m "feat(knowledge-base): add heading-aware markdown chunking with overlap"
```

---

## Task 5: Embedding Provider 抽象層

**Files:**
- Create: `src-tauri/src/knowledge_base/embedding.rs`
- Modify: `src-tauri/src/knowledge_base/mod.rs`
- Test: `src-tauri/tests/knowledge_base_embedding.rs`

- [ ] **Step 1: 建立空殼模組**

```rust
// src-tauri/src/knowledge_base/embedding.rs
// (空檔案,下一步驟才寫實作)
```

```rust
// src-tauri/src/knowledge_base/mod.rs — 加入
pub mod embedding;
```

- [ ] **Step 2: 寫失敗測試（Ollama 與 OpenAI 相容兩種 provider,用 wiremock 假伺服器）**

```rust
// src-tauri/tests/knowledge_base_embedding.rs
use aiterm_lib::config::types::ProviderType;
use aiterm_lib::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder};
use wiremock::matchers::{method, path, header};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn ollama_embed_returns_vectors_in_order() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "nomic-embed-text".into(),
    });

    let result = embedder.embed(&["hello".into(), "world".into()]).await.expect("embed ok");
    assert_eq!(result, vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]]);
}

#[tokio::test]
async fn openai_compatible_embed_sorts_by_index_and_sends_bearer_token() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"embedding": [0.9, 0.9], "index": 1},
                {"embedding": [0.1, 0.1], "index": 0}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Openai,
        base_url: server.uri(),
        api_key: Some("test-key".into()),
        model: "text-embedding-3-small".into(),
    });

    let result = embedder.embed(&["a".into(), "b".into()]).await.expect("embed ok");
    // index 0 → 第一筆, index 1 → 第二筆（不管伺服器回傳順序）
    assert_eq!(result, vec![vec![0.1, 0.1], vec![0.9, 0.9]]);
}

#[tokio::test]
async fn http_error_becomes_readable_error_message() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/embed"))
        .respond_with(ResponseTemplate::new(500).set_body_string("model not found"))
        .mount(&server)
        .await;

    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "does-not-exist".into(),
    });

    let err = embedder.embed(&["x".into()]).await.unwrap_err();
    assert!(err.contains("500"), "error should mention status code: {err}");
}

#[tokio::test]
async fn unsupported_provider_type_returns_error_without_http_call() {
    let embedder = HttpEmbedder::new(EmbedderConfig {
        provider_type: ProviderType::Anthropic,
        base_url: "http://localhost:9".into(), // 不會真的被呼叫
        api_key: None,
        model: "claude".into(),
    });

    let err = embedder.embed(&["x".into()]).await.unwrap_err();
    assert!(err.contains("Anthropic"), "error should name the unsupported provider: {err}");
}
```

- [ ] **Step 3: 執行測試,確認因型別/函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --test knowledge_base_embedding -- --nocapture`
Expected: 編譯錯誤（找不到 `Embedder`、`EmbedderConfig`、`HttpEmbedder`）

- [ ] **Step 4: 實作 embedding provider 抽象層**

```rust
// src-tauri/src/knowledge_base/embedding.rs
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::config::types::ProviderType;

#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    pub provider_type: ProviderType,
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}

pub struct HttpEmbedder {
    pub config: EmbedderConfig,
    client: reqwest::Client,
}

impl HttpEmbedder {
    pub fn new(config: EmbedderConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        match self.config.provider_type {
            ProviderType::Ollama => embed_ollama(&self.client, &self.config, texts).await,
            ProviderType::Openai | ProviderType::OpenaiCompatible => {
                embed_openai_compatible(&self.client, &self.config, texts).await
            }
            other => Err(format!("{other} 不支援 embedding")),
        }
    }
}

#[derive(Serialize)]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

async fn embed_ollama(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let url = format!("{}/api/embed", cfg.base_url.trim_end_matches('/'));
    let resp = client.post(&url)
        .json(&OllamaEmbedRequest { model: &cfg.model, input: texts })
        .send().await.map_err(|e| format!("Ollama embed request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Ollama embed HTTP {status}: {body}"));
    }

    let parsed: OllamaEmbedResponse = resp.json().await
        .map_err(|e| format!("Ollama embed parse error: {e}"))?;
    Ok(parsed.embeddings)
}

#[derive(Serialize)]
struct OpenAiEmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OpenAiEmbedResponse {
    data: Vec<OpenAiEmbedItem>,
}

#[derive(Deserialize)]
struct OpenAiEmbedItem {
    embedding: Vec<f32>,
    index: usize,
}

async fn embed_openai_compatible(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let url = format!("{}/embeddings", cfg.base_url.trim_end_matches('/'));
    let mut req = client.post(&url).json(&OpenAiEmbedRequest { model: &cfg.model, input: texts });
    if let Some(key) = &cfg.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await.map_err(|e| format!("Embedding request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Embedding HTTP {status}: {body}"));
    }

    let parsed: OpenAiEmbedResponse = resp.json().await
        .map_err(|e| format!("Embedding parse error: {e}"))?;
    let mut items = parsed.data;
    items.sort_by_key(|i| i.index);
    Ok(items.into_iter().map(|i| i.embedding).collect())
}
```

- [ ] **Step 5: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --test knowledge_base_embedding -- --nocapture`
Expected: PASS（4 passed）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/knowledge_base/embedding.rs src-tauri/src/knowledge_base/mod.rs src-tauri/tests/knowledge_base_embedding.rs
git commit -m "feat(knowledge-base): add Ollama and OpenAI-compatible embedding providers"
```

---

## Task 6: Document/Chunk CRUD 與 Cosine Similarity 搜尋

**Files:**
- Modify: `src-tauri/src/db/knowledge_base.rs`
- Modify: `src-tauri/tests/db_knowledge_base_integration.rs`

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/tests/db_knowledge_base_integration.rs` 加入：

```rust
use aiterm_lib::db::knowledge_base::{
    upsert_document, list_documents, delete_document_by_path, replace_chunks,
    search_similar_chunks, cosine_similarity, create_notebook,
};

#[test]
fn cosine_similarity_known_values() {
    assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

#[tokio::test]
async fn document_and_chunk_lifecycle() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None).await.expect("create notebook");

    let doc_id = upsert_document(
        &pool, &notebook.id, "report.pdf", 1000, "hash1",
        Some("# Report\n\ncontent"), "ok", None,
    ).await.expect("upsert document");

    let docs = list_documents(&pool, &notebook.id).await.expect("list documents");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].rel_path, "report.pdf");

    // 相同路徑再次 upsert 應該更新而不是新增一筆
    let doc_id_again = upsert_document(
        &pool, &notebook.id, "report.pdf", 2000, "hash2",
        Some("# Report v2"), "ok", None,
    ).await.expect("re-upsert document");
    assert_eq!(doc_id, doc_id_again);
    let docs_after = list_documents(&pool, &notebook.id).await.expect("list after re-upsert");
    assert_eq!(docs_after.len(), 1);
    assert_eq!(docs_after[0].content_hash, "hash2");

    replace_chunks(&pool, &doc_id, &[
        ("chunk one about apples".into(), Some("Report".into()), vec![1.0, 0.0, 0.0]),
        ("chunk two about oranges".into(), Some("Report".into()), vec![0.0, 1.0, 0.0]),
    ]).await.expect("replace chunks");

    let hits = search_similar_chunks(&pool, &notebook.id, &[1.0, 0.0, 0.0], 10)
        .await.expect("search chunks");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].text, "chunk one about apples");
    assert!(hits[0].score > hits[1].score);

    delete_document_by_path(&pool, &notebook.id, "report.pdf").await.expect("delete document");
    let docs_final = list_documents(&pool, &notebook.id).await.expect("list after delete");
    assert!(docs_final.is_empty());
    let hits_final = search_similar_chunks(&pool, &notebook.id, &[1.0, 0.0, 0.0], 10)
        .await.expect("search after delete");
    assert!(hits_final.is_empty(), "deleting a document must cascade-delete its chunks");
}
```

- [ ] **Step 2: 執行測試,確認因函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --test db_knowledge_base_integration -- --nocapture`
Expected: 編譯錯誤（找不到 `upsert_document` 等函式）

- [ ] **Step 3: 實作 document/chunk CRUD 與搜尋函式**

```rust
// 加在 src-tauri/src/db/knowledge_base.rs 檔案末尾

fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for f in v {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    buf
}

fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

pub async fn upsert_document(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    mtime: i64,
    content_hash: &str,
    markdown_cache: Option<&str>,
    status: &str,
    error_message: Option<&str>,
) -> Result<String, sqlx::Error> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await?;

    let id = match existing {
        Some((id,)) => {
            sqlx::query(
                "UPDATE documents SET mtime = ?, content_hash = ?, markdown_cache = ?, status = ?, error_message = ?
                 WHERE id = ?"
            )
            .bind(mtime).bind(content_hash).bind(markdown_cache).bind(status).bind(error_message)
            .bind(&id)
            .execute(pool).await?;
            id
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO documents (id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(notebook_id).bind(rel_path).bind(mtime).bind(content_hash)
            .bind(markdown_cache).bind(status).bind(error_message)
            .execute(pool).await?;
            id
        }
    };
    Ok(id)
}

pub async fn list_documents(pool: &SqlitePool, notebook_id: &str) -> Result<Vec<DocumentRow>, sqlx::Error> {
    sqlx::query_as::<_, DocumentRow>(
        "SELECT id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message
         FROM documents WHERE notebook_id = ?"
    ).bind(notebook_id).fetch_all(pool).await
}

pub async fn delete_document_by_path(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
) -> Result<(), sqlx::Error> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await?;

    if let Some((doc_id,)) = existing {
        sqlx::query("DELETE FROM chunks WHERE document_id = ?").bind(&doc_id).execute(pool).await?;
        sqlx::query("DELETE FROM documents WHERE id = ?").bind(&doc_id).execute(pool).await?;
    }
    Ok(())
}

pub async fn replace_chunks(
    pool: &SqlitePool,
    document_id: &str,
    chunks: &[(String, Option<String>, Vec<f32>)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chunks WHERE document_id = ?").bind(document_id).execute(&mut *tx).await?;

    for (idx, (text, location_hint, embedding)) in chunks.iter().enumerate() {
        let id = uuid::Uuid::new_v4().to_string();
        let blob = encode_embedding(embedding);
        sqlx::query(
            "INSERT INTO chunks (id, document_id, chunk_index, text, location_hint, embedding)
             VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&id).bind(document_id).bind(idx as i64).bind(text).bind(location_hint).bind(&blob)
        .execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub document_id: String,
    pub rel_path: String,
    pub text: String,
    pub location_hint: Option<String>,
    pub score: f32,
}

pub async fn search_similar_chunks(
    pool: &SqlitePool,
    notebook_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    #[derive(FromRow)]
    struct Row {
        document_id: String,
        rel_path: String,
        text: String,
        location_hint: Option<String>,
        embedding: Vec<u8>,
    }

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.document_id, d.rel_path, c.text, c.location_hint, c.embedding
         FROM chunks c JOIN documents d ON c.document_id = d.id
         WHERE d.notebook_id = ?"
    ).bind(notebook_id).fetch_all(pool).await?;

    let mut hits: Vec<SearchHit> = rows.into_iter().map(|r| {
        let embedding = decode_embedding(&r.embedding);
        let score = cosine_similarity(query_embedding, &embedding);
        SearchHit {
            document_id: r.document_id,
            rel_path: r.rel_path,
            text: r.text,
            location_hint: r.location_hint,
            score,
        }
    }).collect();

    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(top_k);
    Ok(hits)
}
```

- [ ] **Step 4: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --test db_knowledge_base_integration -- --nocapture`
Expected: PASS（4 passed）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/knowledge_base.rs src-tauri/tests/db_knowledge_base_integration.rs
git commit -m "feat(knowledge-base): add document/chunk CRUD and cosine similarity search"
```

---

## Task 7: 擷取管線（掃描 + 轉換 + 切片 + Embedding + 寫入）

**Files:**
- Create: `src-tauri/src/knowledge_base/ingest.rs`
- Modify: `src-tauri/src/knowledge_base/mod.rs`
- Test: `src-tauri/tests/knowledge_base_ingest.rs`

- [ ] **Step 1: 建立空殼模組**

```rust
// src-tauri/src/knowledge_base/ingest.rs
// (空檔案,下一步驟才寫實作)
```

```rust
// src-tauri/src/knowledge_base/mod.rs — 加入
pub mod ingest;
```

- [ ] **Step 2: 寫失敗測試（用 tempfile 建立假資料夾 + fake converter/embedder，驗證新增/不變/刪除三種情境）**

```rust
// src-tauri/tests/knowledge_base_ingest.rs
use std::path::Path;
use std::fs;
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use tempfile::tempdir;

use aiterm_lib::db::knowledge_base::{create_notebook, list_documents, search_similar_chunks};
use aiterm_lib::knowledge_base::embedding::Embedder;
use aiterm_lib::knowledge_base::ingest::{sync_notebook, DocumentConverter, SyncProgress};

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

struct FakeConverter;
#[async_trait]
impl DocumentConverter for FakeConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        let name = path.file_name().unwrap().to_string_lossy();
        Ok(format!("# {name}\n\nconverted content for {name}"))
    }
}

struct FakeEmbedder;
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|_| vec![0.1, 0.2, 0.3]).collect())
    }
}

#[tokio::test]
async fn sync_indexes_new_files_and_reports_progress() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("b.pdf"), "fake pdf bytes").unwrap();
    fs::write(dir.path().join("ignored.exe"), "not supported").unwrap();

    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None)
        .await.unwrap();

    let mut progress_events: Vec<SyncProgress> = Vec::new();
    let summary = sync_notebook(
        &pool, &notebook, &FakeConverter, &FakeEmbedder,
        |p| progress_events.push(p),
    ).await.unwrap();

    assert_eq!(summary.indexed, 2, "only a.txt and b.pdf are supported extensions");
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.deleted, 0);
    assert!(!progress_events.is_empty());

    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert_eq!(docs.len(), 2);

    let hits = search_similar_chunks(&pool, &notebook.id, &[0.1, 0.2, 0.3], 10).await.unwrap();
    assert_eq!(hits.len(), 2);
}

#[tokio::test]
async fn sync_skips_unchanged_files_on_second_run() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None)
        .await.unwrap();

    sync_notebook(&pool, &notebook, &FakeConverter, &FakeEmbedder, |_| {}).await.unwrap();
    let second_summary = sync_notebook(&pool, &notebook, &FakeConverter, &FakeEmbedder, |_| {})
        .await.unwrap();

    assert_eq!(second_summary.indexed, 0, "unchanged file must not be re-processed");
    assert_eq!(second_summary.deleted, 0);
}

#[tokio::test]
async fn sync_removes_documents_for_deleted_files() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("a.txt");
    fs::write(&file_path, "hello").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None)
        .await.unwrap();

    sync_notebook(&pool, &notebook, &FakeConverter, &FakeEmbedder, |_| {}).await.unwrap();
    fs::remove_file(&file_path).unwrap();
    let summary = sync_notebook(&pool, &notebook, &FakeConverter, &FakeEmbedder, |_| {})
        .await.unwrap();

    assert_eq!(summary.deleted, 1);
    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert!(docs.is_empty());
}
```

- [ ] **Step 3: 執行測試,確認因型別/函式不存在而編譯失敗**

Run: `cd src-tauri && cargo test --test knowledge_base_ingest -- --nocapture`
Expected: 編譯錯誤（找不到 `sync_notebook`、`DocumentConverter`、`SyncProgress`)

- [ ] **Step 4: 實作擷取管線**

```rust
// src-tauri/src/knowledge_base/ingest.rs
use std::path::{Path, PathBuf};
use std::fs;
use std::collections::{HashMap, HashSet};
use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use futures_util::stream::{self, StreamExt};

use crate::db::knowledge_base::{self, DocumentRow, NotebookRow};
use crate::knowledge_base::chunk::chunk_markdown;
use crate::knowledge_base::embedding::Embedder;

/// 單次同步最大併發轉換/embedding 數（見設計規格第 4 節安全限制）。
const MAX_CONCURRENT: usize = 3;

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
    "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
    "txt", "md", "rst", "xml", "json",
];

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub mtime: i64,
}

/// 遞迴掃描資料夾，略過隱藏檔案/目錄，只保留支援格式的副檔名。
pub fn scan_folder(root: &Path) -> Vec<ScannedFile> {
    let mut out = Vec::new();
    scan_dir(root, root, &mut out);
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    out
}

fn scan_dir(dir: &Path, root: &Path, out: &mut Vec<ScannedFile>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            scan_dir(&path, root, out);
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !SUPPORTED_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let mtime = fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let rel_path = path.strip_prefix(root).unwrap_or(&path)
            .to_string_lossy().replace('\\', "/");
        out.push(ScannedFile { rel_path, abs_path: path, mtime });
    }
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(h.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

/// 將檔案轉成 markdown 的抽象——正式環境由 MarkItDownConverter（Task 8）實作，
/// 測試用 fake 實作避免依賴 Python/MarkItDown。
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    async fn convert(&self, path: &Path) -> Result<String, String>;
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    pub processed: usize,
    pub total: usize,
    pub current_file: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncSummary {
    pub indexed: usize,
    pub failed: usize,
    pub deleted: usize,
}

/// 對筆記本做一次增量同步：比對資料夾現況與既有 documents 紀錄，
/// 只轉換/切片/embedding 新增或內容變更的檔案（最多 MAX_CONCURRENT 個並行），
/// 刪除已消失的檔案紀錄。
pub async fn sync_notebook(
    pool: &SqlitePool,
    notebook: &NotebookRow,
    converter: &dyn DocumentConverter,
    embedder: &dyn Embedder,
    mut on_progress: impl FnMut(SyncProgress),
) -> Result<SyncSummary, String> {
    let root = Path::new(&notebook.folder_path);
    let scanned = scan_folder(root);
    let existing = knowledge_base::list_documents(pool, &notebook.id)
        .await.map_err(|e| e.to_string())?;

    let scanned_paths: HashSet<&str> = scanned.iter().map(|f| f.rel_path.as_str()).collect();
    let mut summary = SyncSummary::default();

    for doc in existing.iter().filter(|d| !scanned_paths.contains(d.rel_path.as_str())) {
        knowledge_base::delete_document_by_path(pool, &notebook.id, &doc.rel_path)
            .await.map_err(|e| e.to_string())?;
        summary.deleted += 1;
    }

    let existing_by_path: HashMap<&str, &DocumentRow> =
        existing.iter().map(|d| (d.rel_path.as_str(), d)).collect();

    // 先算 hash 判斷哪些檔案真的需要處理（新增或內容變更）。
    // 讀不到內容的檔案直接記成 error，不進入併發處理階段。
    let mut to_process: Vec<(&ScannedFile, String)> = Vec::new();
    for file in &scanned {
        let hash = match hash_file(&file.abs_path) {
            Ok(h) => h,
            Err(e) => {
                knowledge_base::upsert_document(
                    pool, &notebook.id, &file.rel_path, file.mtime, "", None, "error", Some(&e),
                ).await.map_err(|e| e.to_string())?;
                summary.failed += 1;
                continue;
            }
        };
        let unchanged = existing_by_path.get(file.rel_path.as_str())
            .map(|d| d.content_hash == hash && d.status == "ok")
            .unwrap_or(false);
        if !unchanged {
            to_process.push((file, hash));
        }
    }

    let total = to_process.len();
    let mut processed = 0usize;

    let mut stream = stream::iter(to_process.into_iter().map(|(file, hash)| {
        process_one_file(pool, &notebook.id, &file.rel_path, &file.abs_path, file.mtime, hash, converter, embedder)
    })).buffer_unordered(MAX_CONCURRENT);

    while let Some(outcome) = stream.next().await {
        processed += 1;
        match outcome {
            Ok(rel_path) => {
                summary.indexed += 1;
                on_progress(SyncProgress { processed, total, current_file: rel_path });
            }
            Err((rel_path, _err)) => {
                summary.failed += 1;
                on_progress(SyncProgress { processed, total, current_file: rel_path });
            }
        }
    }

    Ok(summary)
}

/// 轉換單一檔案並寫入結果（成功或失敗都會 upsert 對應的 document row）。
/// 回傳 `Ok(rel_path)` 或 `Err((rel_path, error_message))`，方便併發收集結果時
/// 仍能標示是哪個檔案。
async fn process_one_file(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    abs_path: &Path,
    mtime: i64,
    hash: String,
    converter: &dyn DocumentConverter,
    embedder: &dyn Embedder,
) -> Result<String, (String, String)> {
    match process_one_file_inner(pool, notebook_id, rel_path, abs_path, mtime, &hash, converter, embedder).await {
        Ok(()) => Ok(rel_path.to_string()),
        Err(e) => {
            let _ = knowledge_base::upsert_document(
                pool, notebook_id, rel_path, mtime, &hash, None, "error", Some(&e),
            ).await;
            Err((rel_path.to_string(), e))
        }
    }
}

async fn process_one_file_inner(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    abs_path: &Path,
    mtime: i64,
    hash: &str,
    converter: &dyn DocumentConverter,
    embedder: &dyn Embedder,
) -> Result<(), String> {
    let markdown = converter.convert(abs_path).await?;
    let chunks = chunk_markdown(&markdown);

    let doc_id = knowledge_base::upsert_document(
        pool, notebook_id, rel_path, mtime, hash, Some(&markdown), "ok", None,
    ).await.map_err(|e| e.to_string())?;

    if chunks.is_empty() {
        knowledge_base::replace_chunks(pool, &doc_id, &[]).await.map_err(|e| e.to_string())?;
        return Ok(());
    }

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = embedder.embed(&texts).await?;
    if embeddings.len() != chunks.len() {
        return Err(format!(
            "Embedding count mismatch: {} chunks vs {} embeddings",
            chunks.len(), embeddings.len()
        ));
    }

    let rows: Vec<(String, Option<String>, Vec<f32>)> = chunks.into_iter()
        .zip(embeddings)
        .map(|(c, e)| (c.text, c.location_hint, e))
        .collect();
    knowledge_base::replace_chunks(pool, &doc_id, &rows).await.map_err(|e| e.to_string())?;

    Ok(())
}
```

- [ ] **Step 5: 執行測試,確認通過**

Run: `cd src-tauri && cargo test --test knowledge_base_ingest -- --nocapture`
Expected: PASS（3 passed）

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/knowledge_base/ingest.rs src-tauri/src/knowledge_base/mod.rs src-tauri/tests/knowledge_base_ingest.rs
git commit -m "feat(knowledge-base): add incremental folder sync pipeline"
```

---

## Task 8: `kb_sync_notebook` Command 與 Streaming 進度事件

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: 在 `commands/knowledge_base.rs` 加入真正的 `DocumentConverter`（包裝既有 `markitdown_convert`）與 embedding provider 設定解析**

```rust
// 加在 src-tauri/src/commands/knowledge_base.rs 檔案末尾（先擴充 use 區塊，見下一步）

use std::path::Path;
use std::sync::Arc;
use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Manager};
use serde::Serialize;

use crate::config::{ConfigStore, ProviderType};
use crate::secret::SecretStore;
use crate::db::knowledge_base as kb_db;
use crate::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder};
use crate::knowledge_base::ingest::{sync_notebook, DocumentConverter, SyncProgress, SyncSummary};

struct MarkItDownConverter {
    app: AppHandle,
    vision_provider_id: Option<String>,
}

#[async_trait]
impl DocumentConverter for MarkItDownConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        let config = self.app.state::<Arc<ConfigStore>>();
        let secrets = self.app.state::<Arc<SecretStore>>();
        crate::commands::markitdown::markitdown_convert(
            self.app.clone(),
            path.to_string_lossy().to_string(),
            self.vision_provider_id.clone(),
            config,
            secrets,
        ).await
    }
}

fn resolve_embedder_config(
    config: &ConfigStore,
    secrets: &SecretStore,
    provider_id: &str,
) -> Result<EmbedderConfig, String> {
    let cfg = config.get_provider(provider_id)
        .ok_or_else(|| format!("找不到 provider: {provider_id}"))?;
    let api_key = secrets.get(provider_id).ok().flatten();

    let base_url = match cfg.provider_type {
        ProviderType::Ollama => cfg.base_url.unwrap_or_else(|| "http://localhost:11434".to_string()),
        ProviderType::Openai => cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        ProviderType::OpenaiCompatible => cfg.base_url
            .ok_or_else(|| "OpenAI 相容 provider 缺少 base_url".to_string())?,
        other => return Err(format!("{other} 不支援 embedding")),
    };

    Ok(EmbedderConfig {
        provider_type: cfg.provider_type,
        base_url,
        api_key,
        model: cfg.model,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum KbSyncEvent {
    Progress {
        notebook_id: String,
        processed: usize,
        total: usize,
        current_file: String,
    },
    Done {
        notebook_id: String,
        indexed: usize,
        failed: usize,
        deleted: usize,
    },
}

#[tauri::command]
pub async fn kb_sync_notebook(
    notebook_id: String,
    app: AppHandle,
    db: tauri::State<'_, kb_db::KnowledgeBaseDb>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<SyncSummary, String> {
    let notebook = kb_db::get_notebook(&db.pool, &notebook_id)
        .await.map_err(|e| e.to_string())?;

    let provider_id = notebook.embed_provider_id.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding provider".to_string())?;
    let model = notebook.embed_model.clone()
        .ok_or_else(|| "此筆記本尚未設定 embedding model".to_string())?;

    let mut embedder_cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    embedder_cfg.model = model;
    let embedder = HttpEmbedder::new(embedder_cfg);

    let converter = MarkItDownConverter {
        app: app.clone(),
        vision_provider_id: Some(provider_id),
    };

    let app_for_progress = app.clone();
    let nb_id_for_progress = notebook_id.clone();

    let summary = sync_notebook(
        &db.pool,
        &notebook,
        &converter,
        &embedder,
        move |progress: SyncProgress| {
            let _ = app_for_progress.emit("kb-sync-event", KbSyncEvent::Progress {
                notebook_id: nb_id_for_progress.clone(),
                processed: progress.processed,
                total: progress.total,
                current_file: progress.current_file,
            });
        },
    ).await?;

    let now = chrono::Utc::now().timestamp();
    kb_db::mark_synced(&db.pool, &notebook_id, now).await.map_err(|e| e.to_string())?;

    let _ = app.emit("kb-sync-event", KbSyncEvent::Done {
        notebook_id: notebook_id.clone(),
        indexed: summary.indexed,
        failed: summary.failed,
        deleted: summary.deleted,
    });

    Ok(summary)
}
```

- [ ] **Step 2: 在 `lib.rs` 的 `invoke_handler!` 清單中加入新 command**

在 `kb_delete_notebook,` 那一行之後加入：

```rust
            kb_sync_notebook,
```

- [ ] **Step 3: 編譯確認**

Run: `cd src-tauri && cargo build --lib`
Expected: 編譯成功。若出現 `ProviderType`/`ConfigStore` 的 import 路徑錯誤，比對 `src-tauri/src/commands/markitdown.rs` 開頭的 `use` 區塊修正（`use crate::config::{ConfigStore, ProviderType};`、`use crate::secret::SecretStore;`）。

- [ ] **Step 4: 執行完整測試套件,確認 Plan A 全部通過**

Run: `cd src-tauri && cargo test`
Expected: 全部 PASS，包含前面 7 個 Task 累積的測試（schema、CRUD、chunk、embedding、ingest 全部）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/lib.rs
git commit -m "feat(knowledge-base): wire kb_sync_notebook command with streaming progress events"
```

---

## 完成後的驗收方式

Plan A 完成後，後端已具備完整、可獨立驗證的擷取管線，但**還沒有任何 UI 或聊天功能**（那是 Plan B 的範圍）。驗收方式：

1. `cd src-tauri && cargo test` 全數通過
2. 可以手動透過 Tauri 的開發者主控台呼叫 `invoke('kb_create_notebook', {...})` → `invoke('kb_sync_notebook', {...})`，並在 `~/Library/Application Support/AITERM/knowledge_base.db`（macOS 路徑，其他平台對應 `dirs::data_dir()`）用 `sqlite3` CLI 檢查 `documents`/`chunks` 表確實有資料寫入

## 不在此 Plan 範圍內（見 Plan B）

- `search_documents` / `read_document` agent tool
- Chat agent loop（重用 `code_assistant` 架構）
- `kb_chat` command
- 前端所有 UI（`ipc/knowledgeBase.ts`、`useKnowledgeBase` hook、`KnowledgeBaseView` 元件、Tab 整合、i18n）

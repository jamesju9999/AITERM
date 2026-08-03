# 知識庫 Embedding Model 選擇器 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增筆記本時列出該 provider 的模型供選擇，並在寫入資料庫前用一次真實的 embed 呼叫驗證所選模型，同時把一直空著的 `notebooks.embed_dim` 補上。

**Architecture:** 後端在 `knowledge_base/embedding.rs` 新增與既有 `HttpEmbedder::embed` 共用同一個 provider 類型 match 的 `list_models`。建立筆記本的驗證邏輯抽成獨立的 `knowledge_base/notebook.rs`，接受 `&dyn Embedder` 以便測試，tauri 指令只是薄包裝。前端沿用 `ProviderForm.tsx` 已在 5 處使用的 `<input list>` + `<datalist>` 模式。

**Tech Stack:** Rust / Tauri 2 / sqlx (SQLite) / wiremock、React 19 / TypeScript / Vitest / React Testing Library

**Spec:** `docs/superpowers/specs/2026-08-03-kb-embedding-model-picker-design.md`

---

## File Structure

**後端**

| 檔案 | 職責 | 動作 |
|---|---|---|
| `src-tauri/src/knowledge_base/embedding.rs` | embedding 的 HTTP 存取。新增 `list_models` 與 `list_openai_compatible`，與既有 `embed` 共用 provider 類型判斷 | 修改 |
| `src-tauri/src/knowledge_base/notebook.rs` | 「探測後建立」這一個動作。獨立成檔是為了讓它能接受 `&dyn Embedder` 而不需要 tauri State | **新建** |
| `src-tauri/src/knowledge_base/mod.rs` | 模組宣告 | 修改（加一行） |
| `src-tauri/src/db/knowledge_base.rs` | `create_notebook` 增加 `embed_dim` 參數 | 修改 |
| `src-tauri/src/commands/knowledge_base.rs` | 新增 `kb_list_embedding_models`；`kb_create_notebook` 改走驗證路徑 | 修改 |
| `src-tauri/src/lib.rs` | 註冊新指令 | 修改（兩處） |

**前端**

| 檔案 | 職責 | 動作 |
|---|---|---|
| `src/ipc/knowledgeBase.ts` | 新增 `kbListEmbeddingModels` | 修改 |
| `src/components/KnowledgeBaseView/NotebookCreateDialog.tsx` | provider 選項標示、模型 datalist、排序 | 修改 |
| `src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx` | 此元件目前無測試 | **新建** |

**測試**

| 檔案 | 涵蓋 |
|---|---|
| `src-tauri/tests/knowledge_base_embedding.rs` | `list_models` 分派與解析 |
| `src-tauri/tests/db_knowledge_base_integration.rs` | `embed_dim` 寫入 |
| `src-tauri/tests/knowledge_base_notebook.rs`（**新建**） | 探測失敗不寫入 DB 的不變式 |

---

## Task 1: 模型列舉

**Files:**
- Modify: `src-tauri/src/knowledge_base/embedding.rs`
- Test: `src-tauri/tests/knowledge_base_embedding.rs`

- [ ] **Step 1: 寫失敗測試**

附加到 `src-tauri/tests/knowledge_base_embedding.rs` 檔尾。檔案開頭的 `use` 需要補上 `list_models`：

```rust
// 檔頭既有的 use 改成：
use aiterm_lib::knowledge_base::embedding::{Embedder, EmbedderConfig, HttpEmbedder, list_models};
```

```rust
#[tokio::test]
async fn openai_compatible_list_models_returns_ids() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                {"id": "nomic-embed-text"},
                {"id": "Qwen3.6-35B-A3B-4bit"}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::OpenaiCompatible,
        base_url: server.uri(),
        api_key: Some("test-key".into()),
        model: "unused".into(),
    };

    let models = list_models(&cfg).await.expect("list ok");
    assert_eq!(models, vec!["nomic-embed-text", "Qwen3.6-35B-A3B-4bit"]);
}

#[tokio::test]
async fn list_models_errors_when_endpoint_missing() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Openai,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("404 should error");
    assert!(err.contains("404"), "error should mention the status: {err}");
}

#[tokio::test]
async fn ollama_list_models_reads_tags_endpoint() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [{"name": "nomic-embed-text"}, {"name": "llama3.1:8b"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let cfg = EmbedderConfig {
        provider_type: ProviderType::Ollama,
        base_url: server.uri(),
        api_key: None,
        model: "unused".into(),
    };

    let models = list_models(&cfg).await.expect("list ok");
    assert_eq!(models, vec!["nomic-embed-text", "llama3.1:8b"]);
}

#[tokio::test]
async fn list_models_rejects_provider_without_embedding_support() {
    let cfg = EmbedderConfig {
        provider_type: ProviderType::Anthropic,
        base_url: "http://unused".into(),
        api_key: None,
        model: "unused".into(),
    };

    let err = list_models(&cfg).await.expect_err("anthropic has no embedding API");
    assert!(err.contains("不支援 embedding"), "unexpected message: {err}");
}
```

- [ ] **Step 2: 執行測試確認會紅**

```bash
cd src-tauri && cargo test --test knowledge_base_embedding
```

Expected: 編譯失敗，`cannot find function 'list_models' in module`。

- [ ] **Step 3: 實作**

在 `src-tauri/src/knowledge_base/embedding.rs` 的 `HttpEmbedder` impl 之後加入。注意這個 match 必須與 `HttpEmbedder::embed`（同檔第 47-53 行）的判斷保持一致：

```rust
/// 列出某個 embedding provider 可用的模型。
///
/// 這個 match 必須與 `HttpEmbedder::embed` 的判斷一致——兩者對「哪些
/// provider 類型能做 embedding」的認定若漂移，UI 會列出實際上用不了的模型。
pub async fn list_models(cfg: &EmbedderConfig) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("建立 HTTP client 失敗: {e}"))?;

    match cfg.provider_type {
        ProviderType::Ollama => list_ollama(&client, cfg).await,
        ProviderType::Openai | ProviderType::OpenaiCompatible => {
            list_openai_compatible(&client, cfg).await
        }
        other => Err(format!("{other} 不支援 embedding")),
    }
}

async fn list_ollama(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", cfg.base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await
        .map_err(|e| format!("Ollama tags request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Ollama tags HTTP {}", resp.status()));
    }

    #[derive(Deserialize)]
    struct TagsResponse { models: Vec<TagItem> }
    #[derive(Deserialize)]
    struct TagItem { name: String }

    let parsed: TagsResponse = resp.json().await
        .map_err(|e| format!("Ollama tags parse error: {e}"))?;
    Ok(parsed.models.into_iter().map(|m| m.name).collect())
}

async fn list_openai_compatible(
    client: &reqwest::Client,
    cfg: &EmbedderConfig,
) -> Result<Vec<String>, String> {
    let url = format!("{}/models", cfg.base_url.trim_end_matches('/'));
    let mut req = client.get(&url);
    if let Some(key) = &cfg.api_key {
        req = req.bearer_auth(key);
    }

    let resp = req.send().await
        .map_err(|e| format!("Models request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Models HTTP {}", resp.status()));
    }

    #[derive(Deserialize)]
    struct ModelsResponse { data: Vec<ModelItem> }
    #[derive(Deserialize)]
    struct ModelItem { id: String }

    let parsed: ModelsResponse = resp.json().await
        .map_err(|e| format!("Models parse error: {e}"))?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}
```

`list_ollama` 沒有複用 `OllamaClient::list_models()`，因為那支回傳 `AiError` 而這裡需要 `String`，且它綁在 `OllamaClient` 的建構上。直接讀 `/api/tags` 讓這個檔案的四個函式維持一致的錯誤型別。

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test knowledge_base_embedding
```

Expected: 全部 PASS（含既有的 embed 測試）。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/knowledge_base/embedding.rs src-tauri/tests/knowledge_base_embedding.rs
git commit -m "feat(kb): list the models an embedding provider offers"
```

---

## Task 2: `create_notebook` 記錄向量維度

**Files:**
- Modify: `src-tauri/src/db/knowledge_base.rs:134-150`
- Modify: `src-tauri/src/commands/knowledge_base.rs:15-18`（呼叫端，暫時傳 0 讓編譯通過，Task 4 才接上真值）
- Test: `src-tauri/tests/db_knowledge_base_integration.rs`

- [ ] **Step 1: 寫失敗測試**

附加到 `src-tauri/tests/db_knowledge_base_integration.rs` 檔尾：

```rust
#[tokio::test]
async fn create_notebook_records_embedding_dimension() {
    let pool = setup_pool().await;

    let created = create_notebook(
        &pool, "Docs", "/tmp/docs",
        Some("ollama-local"), Some("nomic-embed-text"), 768,
    ).await.expect("create notebook");

    assert_eq!(created.embed_dim, Some(768));

    let fetched = get_notebook(&pool, &created.id).await.expect("get notebook");
    assert_eq!(fetched.embed_dim, Some(768), "dimension must survive a round trip");
}
```

同檔既有的 `notebook_crud_roundtrip`（第 92 行）呼叫了 `create_notebook`，簽名改變後會編譯失敗。把它的呼叫改成：

```rust
    let created = create_notebook(&pool, "My Docs", "/tmp/docs", Some("ollama-local"), Some("nomic-embed-text"), 768)
```

- [ ] **Step 2: 執行測試確認會紅**

```bash
cd src-tauri && cargo test --test db_knowledge_base_integration
```

Expected: 編譯失敗，`this function takes 5 arguments but 6 arguments were supplied`。

- [ ] **Step 3: 實作**

`src-tauri/src/db/knowledge_base.rs` 第 134-147 行改成：

```rust
pub async fn create_notebook(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    embed_provider_id: Option<&str>,
    embed_model: Option<&str>,
    embed_dim: i64,
) -> Result<NotebookRow, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notebooks (id, name, folder_path, embed_provider_id, embed_model, embed_dim)
         VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id).bind(name).bind(folder_path).bind(embed_provider_id).bind(embed_model).bind(embed_dim)
```

（第 147 行以後的 `.execute(pool)` 等維持原樣。）

`src-tauri/src/commands/knowledge_base.rs` 第 15-18 行暫時補一個 0 讓編譯通過，Task 4 會換成探測得到的真值：

```rust
    create_notebook(
        &db.pool, &name, &folder_path,
        embed_provider_id.as_deref(), embed_model.as_deref(), 0,
    ).await.map_err(|e| e.to_string())
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test db_knowledge_base_integration
```

Expected: 全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/db/knowledge_base.rs src-tauri/src/commands/knowledge_base.rs src-tauri/tests/db_knowledge_base_integration.rs
git commit -m "feat(kb): store the embedding dimension when a notebook is created"
```

---

## Task 3: 探測後才建立

**Files:**
- Create: `src-tauri/src/knowledge_base/notebook.rs`
- Modify: `src-tauri/src/knowledge_base/mod.rs`
- Test: `src-tauri/tests/knowledge_base_notebook.rs`（新建）

- [ ] **Step 1: 寫失敗測試**

建立 `src-tauri/tests/knowledge_base_notebook.rs`：

```rust
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use aiterm_lib::knowledge_base::embedding::Embedder;
use aiterm_lib::knowledge_base::notebook::create_notebook_verified;
use aiterm_lib::db::knowledge_base::list_notebooks;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");

    sqlx::query(
        "CREATE TABLE notebooks (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            folder_path TEXT NOT NULL,
            embed_provider_id TEXT,
            embed_model TEXT,
            embed_dim INTEGER,
            last_synced_at INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"
    ).execute(&pool).await.expect("create notebooks table");

    pool
}

struct OkEmbedder { dim: usize }

#[async_trait]
impl Embedder for OkEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|_| vec![0.0_f32; self.dim]).collect())
    }
}

struct FailingEmbedder;

#[async_trait]
impl Embedder for FailingEmbedder {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Err("model does not support embedding".into())
    }
}

struct EmptyVectorEmbedder;

#[async_trait]
impl Embedder for EmptyVectorEmbedder {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(vec![vec![]])
    }
}

#[tokio::test]
async fn successful_probe_records_the_returned_dimension() {
    let pool = setup_pool().await;
    let embedder = OkEmbedder { dim: 1024 };

    let nb = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "ollama-local", "nomic-embed-text", &embedder,
    ).await.expect("create ok");

    assert_eq!(nb.embed_dim, Some(1024));
}

#[tokio::test]
async fn failed_probe_writes_no_row() {
    let pool = setup_pool().await;
    let embedder = FailingEmbedder;

    let err = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "lmstudio", "Qwen3.6-35B-A3B-4bit", &embedder,
    ).await.expect_err("probe failure must propagate");

    assert!(err.contains("embedding"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty(), "a notebook must not exist when its model was never verified");
}

#[tokio::test]
async fn empty_vector_counts_as_failure_and_writes_no_row() {
    let pool = setup_pool().await;
    let embedder = EmptyVectorEmbedder;

    create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "lmstudio", "weird-model", &embedder,
    ).await.expect_err("a zero-length vector is not a usable embedding");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty());
}
```

- [ ] **Step 2: 執行測試確認會紅**

```bash
cd src-tauri && cargo test --test knowledge_base_notebook
```

Expected: 編譯失敗，`unresolved import 'aiterm_lib::knowledge_base::notebook'`。

- [ ] **Step 3: 實作**

建立 `src-tauri/src/knowledge_base/notebook.rs`：

```rust
//! 建立筆記本前先驗證 embedding 模型真的能用。
//!
//! 分成獨立函式而非寫在 tauri 指令裡，是為了讓它接受 `&dyn Embedder`：
//! 指令層需要 tauri 的 State 才能組出 embedder，那在測試裡很難建構，
//! 而「探測失敗就不寫入」是這個功能最需要被測到的不變式。

use sqlx::SqlitePool;

use super::embedding::Embedder;
use crate::db::knowledge_base::{create_notebook, NotebookRow};

/// 探測用的文字。固定 ASCII 短字串，成本一個 token。
const PROBE_TEXT: &str = "test";

/// 用一次真實的 embed 呼叫驗證模型，成功才寫入筆記本。
///
/// 回傳的向量長度會被記成 `embed_dim`。這是這個欄位唯一的寫入點。
pub async fn create_notebook_verified(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    embed_provider_id: &str,
    embed_model: &str,
    embedder: &dyn Embedder,
) -> Result<NotebookRow, String> {
    let vectors = embedder
        .embed(&[PROBE_TEXT.to_string()])
        .await
        .map_err(|e| format!("此模型無法用於 embedding: {e}"))?;

    let dim = vectors.first().map(|v| v.len()).unwrap_or(0);
    if dim == 0 {
        return Err("此模型未回傳任何向量，無法用於 embedding".into());
    }

    create_notebook(
        pool, name, folder_path,
        Some(embed_provider_id), Some(embed_model), dim as i64,
    )
    .await
    .map_err(|e| e.to_string())
}
```

`src-tauri/src/knowledge_base/mod.rs` 加入模組宣告（維持字母序）：

```rust
pub mod chat;
pub mod chunk;
pub mod embedding;
pub mod ingest;
pub mod notebook;
pub mod tools;
```

- [ ] **Step 4: 執行測試確認通過**

```bash
cd src-tauri && cargo test --test knowledge_base_notebook
```

Expected: 3 個測試全部 PASS。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/knowledge_base/notebook.rs src-tauri/src/knowledge_base/mod.rs src-tauri/tests/knowledge_base_notebook.rs
git commit -m "feat(kb): verify the embedding model before writing the notebook"
```

---

## Task 4: 接上 tauri 指令

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs:7-19`
- Modify: `src-tauri/src/lib.rs:31`（use）與 `:271` 附近（invoke_handler）

此任務沒有新測試——它是把 Task 1 與 Task 3 已測過的邏輯接到 IPC 邊界。驗證方式是編譯與既有測試不退步。

- [ ] **Step 1: 改寫 `kb_create_notebook`**

`src-tauri/src/commands/knowledge_base.rs` 第 7-19 行整段換成：

```rust
#[tauri::command]
pub async fn kb_create_notebook(
    name: String,
    folder_path: String,
    embed_provider_id: Option<String>,
    embed_model: Option<String>,
    db: State<'_, KnowledgeBaseDb>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<NotebookRow, String> {
    let provider_id = embed_provider_id
        .ok_or_else(|| "請選擇 embedding provider".to_string())?;
    let model = embed_model
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| "請填寫 embedding model".to_string())?;

    let mut cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    cfg.model = model.clone();
    let embedder = HttpEmbedder::new(cfg);

    crate::knowledge_base::notebook::create_notebook_verified(
        &db.pool, &name, &folder_path, &provider_id, &model, &embedder,
    ).await
}
```

第 4 行的 `use` 移除已不再直接使用的 `create_notebook`：

```rust
use crate::db::knowledge_base::{
    KnowledgeBaseDb, NotebookRow,
    list_notebooks, delete_notebook,
};
```

`resolve_embedder_config`（同檔第 64 行）與 `HttpEmbedder`（第 41 行的 use）都已存在，不需新增 import。但 `ConfigStore` / `SecretStore` / `Arc` 的 use 在第 32-38 行、位於 `kb_create_notebook` 之後——Rust 的 `use` 不受宣告順序影響，可直接使用。

- [ ] **Step 2: 新增 `kb_list_embedding_models`**

加在 `resolve_embedder_config`（第 87 行結束）之後：

```rust
#[tauri::command]
pub async fn kb_list_embedding_models(
    provider_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<String>, String> {
    let cfg = resolve_embedder_config(&config, &secrets, &provider_id)?;
    crate::knowledge_base::embedding::list_models(&cfg).await
}
```

- [ ] **Step 3: 註冊指令**

`src-tauri/src/lib.rs` 第 31 行的 use 清單加入 `kb_list_embedding_models`：

```rust
        kb_create_notebook, kb_list_notebooks, kb_delete_notebook, kb_sync_notebook, kb_chat, kb_open_document,
        kb_list_embedding_models,
```

第 271 行 `kb_create_notebook,` 之後加一行：

```rust
            kb_list_embedding_models,
```

- [ ] **Step 4: 編譯並跑全部 Rust 測試**

```bash
cd src-tauri && cargo test
```

Expected: 編譯通過，全部測試 PASS。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs src-tauri/src/lib.rs
git commit -m "feat(kb): expose model listing and route creation through the probe"
```

---

## Task 5: 前端 IPC 與 provider 標示

**Files:**
- Modify: `src/ipc/knowledgeBase.ts`
- Modify: `src/components/KnowledgeBaseView/NotebookCreateDialog.tsx`
- Test: `src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx`（新建）

- [ ] **Step 1: 寫失敗測試**

建立 `src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx`：

```tsx
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { NotebookCreateDialog } from "./NotebookCreateDialog";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/vcs", () => ({ pickFolder: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../../ipc/knowledgeBase", () => ({ kbListEmbeddingModels: vi.fn() }));

import { listProviders } from "../../ipc/provider";
import { kbListEmbeddingModels } from "../../ipc/knowledgeBase";

const PROVIDERS = [
  {
    id: "lmstudio", display_name: "Qwen3.6-35B-A3B-4bit",
    provider_type: "openai-compatible", base_url: "http://localhost:1234/v1",
    oauth_client_id: null, model: "Qwen3.6-35B-A3B-4bit",
    supports_json_mode: false, has_api_key: false, is_default: false, auth_method: null,
  },
  {
    id: "ollama-local", display_name: "本機 Ollama",
    provider_type: "ollama", base_url: null,
    oauth_client_id: null, model: "llama3.1:8b",
    supports_json_mode: false, has_api_key: false, is_default: false, auth_method: null,
  },
];

function renderDialog() {
  return render(
    <LocaleProvider>
      <NotebookCreateDialog onCreate={vi.fn()} onClose={vi.fn()} />
    </LocaleProvider>
  );
}

describe("NotebookCreateDialog provider labelling", () => {
  beforeEach(() => {
    vi.mocked(listProviders).mockResolvedValue(PROVIDERS as never);
    vi.mocked(kbListEmbeddingModels).mockResolvedValue([]);
  });

  it("shows the provider type and endpoint so the name is not read as a model", async () => {
    renderDialog();
    const option = await screen.findByRole("option", { name: /Qwen3\.6-35B-A3B-4bit/ });
    expect(option.textContent).toContain("OpenAI-Compatible");
    expect(option.textContent).toContain("localhost:1234");
  });

  it("falls back to the default endpoint when base_url is null", async () => {
    renderDialog();
    const option = await screen.findByRole("option", { name: /本機 Ollama/ });
    expect(option.textContent).toContain("localhost:11434");
  });
});
```

- [ ] **Step 2: 執行測試確認會紅**

```bash
npx vitest run src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
```

Expected: FAIL，`kbListEmbeddingModels` 不存在於 `../../ipc/knowledgeBase`，且選項文字不含類型與 endpoint。

- [ ] **Step 3: 新增 IPC**

`src/ipc/knowledgeBase.ts`，加在 `kbDeleteNotebook` 之後：

```ts
export function kbListEmbeddingModels(providerId: string): Promise<string[]> {
  return invoke<string[]>("kb_list_embedding_models", { providerId });
}
```

- [ ] **Step 4: 實作 provider 標示**

`src/components/KnowledgeBaseView/NotebookCreateDialog.tsx`，在 `EMBEDDING_CAPABLE_TYPES`（第 12 行）之後加入：

```tsx
// base_url 為 null 時 resolve_embedder_config 會套用這些預設值，
// 選單必須顯示同樣的位址，否則畫面上會出現空白的 endpoint。
const DEFAULT_ENDPOINTS: Record<string, string> = {
  ollama: "http://localhost:11434",
  openai: "https://api.openai.com/v1",
};

const TYPE_LABELS: Record<string, string> = {
  ollama: "Ollama",
  openai: "OpenAI",
  "openai-compatible": "OpenAI-Compatible",
};

function providerOptionLabel(p: ProviderInfo): string {
  const endpoint = p.base_url ?? DEFAULT_ENDPOINTS[p.provider_type] ?? "";
  const host = endpoint.replace(/^https?:\/\//, "").replace(/\/+$/, "");
  const type = TYPE_LABELS[p.provider_type] ?? p.provider_type;
  return host ? `${p.display_name} — ${type} · ${host}` : `${p.display_name} — ${type}`;
}
```

第 78-80 行的 `<option>` 改成：

```tsx
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>{providerOptionLabel(p)}</option>
                ))}
```

- [ ] **Step 5: 執行測試確認通過**

```bash
npx vitest run src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
```

Expected: 2 個測試 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/knowledgeBase.ts src/components/KnowledgeBaseView/NotebookCreateDialog.tsx src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
git commit -m "feat(kb): label embedding providers with their type and endpoint"
```

---

## Task 6: 模型 datalist 與排序

**Files:**
- Modify: `src/components/KnowledgeBaseView/NotebookCreateDialog.tsx`
- Test: `src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx`

- [ ] **Step 1: 寫失敗測試**

附加到 `NotebookCreateDialog.test.tsx` 檔尾：

```tsx
describe("NotebookCreateDialog model list", () => {
  beforeEach(() => {
    vi.mocked(listProviders).mockResolvedValue(PROVIDERS as never);
  });

  it("sorts likely embedding models ahead of chat models", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue([
      "Qwen3.6-35B-A3B-4bit",
      "llama3.1:8b",
      "nomic-embed-text",
      "bge-m3",
    ]);

    const { container } = renderDialog();

    await waitFor(() => {
      expect(container.querySelectorAll("datalist option").length).toBe(4);
    });

    const values = Array.from(
      container.querySelectorAll<HTMLOptionElement>("datalist option")
    ).map((o) => o.value);

    expect(values.slice(0, 2)).toEqual(["nomic-embed-text", "bge-m3"]);
    expect(values.slice(2)).toEqual(["Qwen3.6-35B-A3B-4bit", "llama3.1:8b"]);
  });

  it("still allows typing a name when listing fails", async () => {
    vi.mocked(kbListEmbeddingModels).mockRejectedValue(new Error("404"));

    const { container } = renderDialog();

    await waitFor(() => {
      expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalled();
    });

    const input = screen.getByPlaceholderText("例如：nomic-embed-text");
    expect(input).not.toBeDisabled();
    expect(container.querySelectorAll("datalist option").length).toBe(0);
  });

  it("reloads the list when the provider changes", async () => {
    vi.mocked(kbListEmbeddingModels).mockResolvedValue(["nomic-embed-text"]);

    const { container } = renderDialog();

    await waitFor(() => {
      expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalledWith("lmstudio");
    });

    // 不能用 getByRole("combobox")：<select> 與帶 list 屬性的 <input>
    // 兩者的 role 都是 combobox，清單載入後會匹配到兩個元素而拋錯。
    const select = container.querySelector("select")!;
    fireEvent.change(select, { target: { value: "ollama-local" } });

    await waitFor(() => {
      expect(vi.mocked(kbListEmbeddingModels)).toHaveBeenCalledWith("ollama-local");
    });
  });
});
```

- [ ] **Step 2: 執行測試確認會紅**

```bash
npx vitest run src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
```

Expected: 新增的 3 個測試 FAIL——目前沒有 `<datalist>`，也沒有呼叫 `kbListEmbeddingModels`。

- [ ] **Step 3: 實作排序啟發式**

`NotebookCreateDialog.tsx`，加在 `providerOptionLabel` 之後：

```tsx
// 只影響排序，不影響可選性——名稱不含這些字的 embedding 模型仍在清單裡，
// 只是排得比較後面，而使用者永遠可以直接手打。真正擋下誤選的是建立時的探測。
const EMBEDDING_NAME_HINTS = ["embed", "bge", "gte", "e5", "nomic", "minilm", "mxbai", "jina"];

function looksLikeEmbeddingModel(name: string): boolean {
  const lower = name.toLowerCase();
  return EMBEDDING_NAME_HINTS.some((hint) => lower.includes(hint));
}

function sortEmbeddingFirst(models: string[]): string[] {
  const likely = models.filter(looksLikeEmbeddingModel);
  const rest = models.filter((m) => !looksLikeEmbeddingModel(m));
  return [...likely, ...rest];
}
```

- [ ] **Step 4: 實作載入與 datalist**

在既有 state 宣告（第 22 行 `error` 之後）加入：

```tsx
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
```

在既有的 `useEffect`（第 24-30 行）之後加入：

```tsx
  useEffect(() => {
    if (!providerId) {
      setModels([]);
      return;
    }
    let cancelled = false;
    setModelsLoading(true);
    kbListEmbeddingModels(providerId)
      .then((list) => {
        if (!cancelled) setModels(sortEmbeddingFirst(list));
      })
      .catch(() => {
        // 列舉失敗不擋人：不少自架端點沒有 /v1/models，跳錯誤只是噪音。
        // 靜默退回純文字輸入，使用者仍可手打，建立時的探測會把關。
        if (!cancelled) setModels([]);
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => { cancelled = true; };
  }, [providerId]);
```

檔頭 import 加入：

```tsx
import { kbListEmbeddingModels } from "../../ipc/knowledgeBase";
```

第 84-91 行的 model 欄位換成：

```tsx
            <label className="kb-dialog__field">
              <span>{t.kb_create_model_label}</span>
              {modelsLoading ? (
                <input type="text" value={t.provider_model_loading} disabled readOnly />
              ) : (
                <>
                  <input
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder={t.kb_create_model_placeholder}
                    list="kb-embedding-models-list"
                  />
                  <datalist id="kb-embedding-models-list">
                    {models.map((m) => <option key={m} value={m} />)}
                  </datalist>
                </>
              )}
            </label>
```

- [ ] **Step 5: 執行測試確認通過**

```bash
npx vitest run src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
```

Expected: 5 個測試全部 PASS。

- [ ] **Step 6: 全套驗證**

```bash
npm run test && npx tsc -b && npm run lint
```

Expected: 全部通過，無型別錯誤、無 lint 錯誤。

- [ ] **Step 7: Commit**

```bash
git add src/components/KnowledgeBaseView/NotebookCreateDialog.tsx src/components/KnowledgeBaseView/NotebookCreateDialog.test.tsx
git commit -m "feat(kb): suggest embedding models with likely ones listed first"
```

---

## Task 7: 實機驗證

**Files:** 無（僅執行）

自動化測試涵蓋不到「datalist 在 WKWebView 上實際長什麼樣」。macOS 的 Tauri 用的是 WebKit，與測試環境的 jsdom 行為不同。

- [ ] **Step 1: 確認 dev server 埠未被占用**

```bash
lsof -iTCP:1420 -sTCP:LISTEN
```

Expected: 無輸出。若有殘留行程先 `kill <PID>`——`vite.config.ts` 設了 `strictPort: true`，1420 被占用時 vite 會直接退出，Tauri 視窗會是一片空白。

- [ ] **Step 2: 啟動**

```bash
npm run tauri:dev
```

- [ ] **Step 3: 逐項確認**

在知識庫分頁按「新增筆記本」，確認：

1. Embedding Provider 每個選項都顯示「名稱 — 類型 · 位址」
2. base_url 為 null 的 Ollama provider 顯示 `localhost:11434` 而非空白
3. 點 Embedding Model 輸入框出現建議清單，疑似 embedding 的排前面
4. 切換 provider 後清單跟著換
5. 故意選一個聊天模型（例如 `Qwen3.6-35B-A3B-4bit`）按建立 → 出現錯誤且**筆記本沒有被建立**
6. 選正確的 embedding 模型 → 建立成功

- [ ] **Step 4: 確認維度真的寫進去了**

```bash
sqlite3 "$(ls ~/Library/Application\ Support/com.aiterm.app/*.db 2>/dev/null | head -1)" \
  "SELECT name, embed_model, embed_dim FROM notebooks;"
```

Expected: 新建的筆記本 `embed_dim` 有值（例如 768 或 1024），不是 NULL。

若路徑不存在，先找出資料庫位置：

```bash
find ~/Library/Application\ Support/com.aiterm.app -name "*.db" 2>/dev/null
```

- [ ] **Step 5: 若有問題**

不要直接猜測修補。使用 `superpowers:systematic-debugging` 找出根因後再改。

---

## 完成後

依 CLAUDE.md，收尾前執行 `superpowers:verification-before-completion` 與 `superpowers:requesting-code-review`。

範圍外、已知但刻意不處理的項目：

- `set_notebook_embed_config`（`db/knowledge_base.rs:180-188`）仍是無呼叫者的死程式碼。它是未來「更換現有筆記本 embedding 模型」的地基，該功能需要清空並重建索引，不在本次範圍。
- 既有筆記本的 `embed_dim` 仍是 NULL。本次只讓新建的筆記本有值，沒有回填。
- `search_similar_chunks`（`db/knowledge_base.rs:354`）仍是全表掃描且不檢查維度一致性。目前不會踩到，因為 embedding 設定建立後無法更改。

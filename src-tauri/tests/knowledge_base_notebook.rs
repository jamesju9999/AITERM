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
        // Mirrors the `notebooks` table in KnowledgeBaseDb::init column-for-column.
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

/// 回傳空清單，連一個向量都沒有。真實世界會這樣：`embed_openai_compatible`
/// 對 `{"data":[]}`、`embed_ollama` 對 `{"embeddings":[]}` 都原封不動回 `Ok(vec![])`。
struct NoVectorsEmbedder;

#[async_trait]
impl Embedder for NoVectorsEmbedder {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(vec![])
    }
}

/// 用來證明「輸入不合法時根本不該花一趟 round trip 去探測」：被呼叫到就讓測試爆掉。
struct NeverCalledEmbedder;

#[async_trait]
impl Embedder for NeverCalledEmbedder {
    async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        unreachable!("輸入不合法時不該真的去探測模型");
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

    assert!(err.contains("此模型無法用於"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty(), "a notebook must not exist when its model was never verified");
}

#[tokio::test]
async fn empty_vector_counts_as_failure_and_writes_no_row() {
    let pool = setup_pool().await;
    let embedder = EmptyVectorEmbedder;

    let err = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "lmstudio", "weird-model", &embedder,
    ).await.expect_err("a zero-length vector is not a usable embedding");

    assert!(err.contains("沒有回傳可用的向量"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn no_vectors_at_all_counts_as_failure_and_writes_no_row() {
    let pool = setup_pool().await;
    let embedder = NoVectorsEmbedder;

    let err = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "lmstudio", "half-baked-gateway", &embedder,
    ).await.expect_err("an empty vector list is not a usable embedding");

    assert!(err.contains("沒有回傳可用的向量"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty());
}

#[tokio::test]
async fn empty_provider_id_is_rejected_without_probing() {
    let pool = setup_pool().await;

    let err = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "  ", "nomic-embed-text", &NeverCalledEmbedder,
    ).await.expect_err("an empty provider id must not produce a notebook");

    assert!(err.contains("provider"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty(), "a notebook must not exist without a provider to sync it");
}

#[tokio::test]
async fn empty_model_is_rejected_without_probing() {
    let pool = setup_pool().await;

    let err = create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "ollama-local", "  ", &NeverCalledEmbedder,
    ).await.expect_err("an empty model must not produce a notebook");

    assert!(err.contains("model"), "unexpected message: {err}");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty(), "a notebook must not exist without a model to sync it");
}

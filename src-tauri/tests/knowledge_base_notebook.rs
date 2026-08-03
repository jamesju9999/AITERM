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

    create_notebook_verified(
        &pool, "Docs", "/tmp/docs", "lmstudio", "weird-model", &embedder,
    ).await.expect_err("a zero-length vector is not a usable embedding");

    let rows = list_notebooks(&pool).await.expect("list notebooks");
    assert!(rows.is_empty());
}

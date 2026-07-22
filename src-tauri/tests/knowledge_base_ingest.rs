use std::path::Path;
use std::fs;
use std::sync::Arc;
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
        &pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder),
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

    sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {}).await.unwrap();
    let second_summary = sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {})
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

    sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {}).await.unwrap();
    fs::remove_file(&file_path).unwrap();
    let summary = sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {})
        .await.unwrap();

    assert_eq!(summary.deleted, 1);
    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert!(docs.is_empty());
}

struct PanicOnFileConverter {
    panic_on: String,
}
#[async_trait]
impl DocumentConverter for PanicOnFileConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == self.panic_on.as_str() {
            panic!("simulated conversion panic for {name}");
        }
        Ok(format!("# {name}\n\nconverted content for {name}"))
    }
}

#[tokio::test]
async fn a_panicking_converter_does_not_abort_the_whole_sync() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("good.txt"), "fine").unwrap();
    fs::write(dir.path().join("bad.txt"), "will panic").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None)
        .await.unwrap();

    let converter = Arc::new(PanicOnFileConverter { panic_on: "bad.txt".to_string() });
    let summary = sync_notebook(&pool, &notebook, converter, Arc::new(FakeEmbedder), |_| {})
        .await
        .expect("sync_notebook itself must not panic even though one file's converter does");

    assert_eq!(summary.indexed, 1, "the good file should still be indexed");
    assert_eq!(summary.failed, 1, "the panicking file should be recorded as failed, not crash the sync");

    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert_eq!(docs.len(), 2, "both files should have document rows (one ok, one error)");
}

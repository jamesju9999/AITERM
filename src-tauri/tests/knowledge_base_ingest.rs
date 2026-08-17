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

    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
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

    let hits = search_similar_chunks(&pool, &notebook.id, "", &[0.1, 0.2, 0.3], 10).await.unwrap();
    assert_eq!(hits.len(), 2);
}

#[tokio::test]
async fn sync_skips_unchanged_files_on_second_run() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
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
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
        .await.unwrap();

    sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {}).await.unwrap();
    fs::remove_file(&file_path).unwrap();
    let summary = sync_notebook(&pool, &notebook, Arc::new(FakeConverter), Arc::new(FakeEmbedder), |_| {})
        .await.unwrap();

    assert_eq!(summary.deleted, 1);
    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert!(docs.is_empty());
}

/// Produces markdown big enough that `chunk_markdown` splits it into well over
/// 100 chunks — simulates a large converted document (e.g. a multi-MB scan
/// report), which is what exposed the "whole file embedded in one HTTP call"
/// bug: a real local embedding server timed out on a ~920-chunk single request.
struct LargeContentConverter;
#[async_trait]
impl DocumentConverter for LargeContentConverter {
    async fn convert(&self, _path: &Path) -> Result<String, String> {
        let paragraph = "這是一段用來撐大檔案內容長度的測試文字。".repeat(100);
        let mut md = String::from("# Big Report\n\n");
        for _ in 0..200 {
            md.push_str(&paragraph);
            md.push('\n');
        }
        Ok(md)
    }
}

/// Records the size of every `embed()` call it receives, so tests can assert
/// on how ingest splits a file's chunks into requests instead of just on the
/// final result.
struct BatchTrackingEmbedder {
    batch_sizes: std::sync::Mutex<Vec<usize>>,
}
#[async_trait]
impl Embedder for BatchTrackingEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.batch_sizes.lock().unwrap().push(texts.len());
        Ok(texts.iter().map(|_| vec![0.1, 0.2, 0.3]).collect())
    }
}

#[tokio::test]
async fn large_file_embeds_in_bounded_batches_not_one_giant_request() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("big.pdf"), "stand-in bytes, content comes from the fake converter").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
        .await.unwrap();

    let embedder = Arc::new(BatchTrackingEmbedder { batch_sizes: std::sync::Mutex::new(Vec::new()) });
    let summary = sync_notebook(&pool, &notebook, Arc::new(LargeContentConverter), embedder.clone(), |_| {})
        .await.unwrap();

    assert_eq!(summary.indexed, 1);
    assert_eq!(summary.failed, 0);

    let batch_sizes = embedder.batch_sizes.lock().unwrap().clone();
    let total_chunks: usize = batch_sizes.iter().sum();
    assert!(total_chunks > 100, "test setup should produce well over 100 chunks, got {total_chunks}");
    assert!(
        batch_sizes.len() > 1,
        "a file with {total_chunks} chunks must be split across multiple embed() calls, not sent in one"
    );
    assert!(
        batch_sizes.iter().all(|&n| n <= 50),
        "no single embed() call should carry a huge slice of the file's chunks: {batch_sizes:?}"
    );

    let hits = search_similar_chunks(&pool, &notebook.id, "", &[0.1, 0.2, 0.3], total_chunks + 10)
        .await.unwrap();
    assert_eq!(hits.len(), total_chunks, "every chunk from every batch must still make it into storage");
}

struct EmptyContentConverter;
#[async_trait]
impl DocumentConverter for EmptyContentConverter {
    async fn convert(&self, _path: &Path) -> Result<String, String> {
        Ok(String::new())
    }
}

#[tokio::test]
async fn empty_converted_content_is_recorded_as_failure_not_success() {
    let pool = setup_pool().await;
    let dir = tempdir().unwrap();
    // Simulates a scanned/image-only PDF: conversion succeeds but yields no text.
    fs::write(dir.path().join("scanned.pdf"), "fake scanned pdf bytes").unwrap();
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
        .await.unwrap();

    let summary = sync_notebook(&pool, &notebook, Arc::new(EmptyContentConverter), Arc::new(FakeEmbedder), |_| {})
        .await.unwrap();

    assert_eq!(summary.indexed, 0, "a file with no extractable text must not count as indexed");
    assert_eq!(summary.failed, 1, "empty conversion output should be recorded as a failure, not silently ok");

    let docs = list_documents(&pool, &notebook.id).await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].status, "error");
    assert!(
        docs[0].error_message.as_deref().unwrap_or("").contains("沒有可用文字"),
        "error message should explain why: {:?}", docs[0].error_message
    );

    let hits = search_similar_chunks(&pool, &notebook.id, "", &[0.1, 0.2, 0.3], 10).await.unwrap();
    assert!(hits.is_empty(), "a failed conversion must not contribute any searchable chunks");
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
    let notebook = create_notebook(&pool, "NB", dir.path().to_str().unwrap(), None, None, 0)
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

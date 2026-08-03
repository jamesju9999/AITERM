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
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
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
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
    let (content, _) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "search_documents", &serde_json::json!({ "query": "" }),
    ).await;
    assert!(content.starts_with("Error:"));
}

#[tokio::test]
async fn read_document_returns_full_markdown_content() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
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
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
    let (content, _) = dispatch_tool(
        &pool, &notebook.id, &FakeEmbedder,
        "read_document", &serde_json::json!({ "path": "does-not-exist.md" }),
    ).await;
    assert!(content.starts_with("Error:"));
}

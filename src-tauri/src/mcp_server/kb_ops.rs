//! Business logic for the knowledge-base MCP tools (`list_notebooks`,
//! `search_documents`, `read_document`). Deliberately does NOT reuse
//! `knowledge_base::tools::dispatch_tool` — that function's signature is
//! bound to a single chat session's `notebook_id`/`embedder`, but an MCP
//! client must be able to name any notebook on any call. Shares the
//! formatting/truncation helpers (`format_search_hits`, `safe_truncate`)
//! with that module instead of duplicating them.

use sqlx::SqlitePool;

use crate::commands::knowledge_base::resolve_embedder_config;
use crate::config::ConfigStore;
use crate::db::knowledge_base::{get_document_by_path, get_notebook, list_notebooks as db_list_notebooks, search_similar_chunks};
use crate::knowledge_base::embedding::{Embedder, HttpEmbedder};
use crate::knowledge_base::tools::{format_search_hits, safe_truncate};
use crate::secret::SecretStore;

const MAX_READ_DOCUMENT_BYTES: usize = 100 * 1024;
const DEFAULT_TOP_K: u64 = 8;
const MAX_TOP_K: u64 = 20;

pub(crate) async fn list_notebooks(pool: &SqlitePool) -> Result<String, String> {
    let notebooks = db_list_notebooks(pool).await.map_err(|e| e.to_string())?;
    if notebooks.is_empty() {
        return Ok("No notebooks in the knowledge base.".to_string());
    }
    let list: Vec<serde_json::Value> = notebooks.iter().map(|n| serde_json::json!({
        "id": n.id, "name": n.name, "folder_path": n.folder_path,
    })).collect();
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

pub(crate) async fn search_documents(
    pool: &SqlitePool,
    config: &ConfigStore,
    secrets: &SecretStore,
    notebook_id: &str,
    query: &str,
    top_k: Option<u64>,
) -> Result<String, String> {
    if query.trim().is_empty() {
        return Err("query is empty".to_string());
    }
    let top_k = top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K) as usize;

    let notebook = match get_notebook(pool, notebook_id).await {
        Ok(nb) => nb,
        Err(sqlx::Error::RowNotFound) => return Err(format!("notebook not found: {notebook_id}")),
        Err(e) => return Err(e.to_string()),
    };
    let embed_provider_id = notebook.embed_provider_id
        .ok_or_else(|| "this notebook has no embedding provider configured".to_string())?;
    let embed_model = notebook.embed_model
        .ok_or_else(|| "this notebook has no embedding model configured".to_string())?;

    let mut embedder_cfg = resolve_embedder_config(config, secrets, &embed_provider_id)?;
    embedder_cfg.model = embed_model;
    let embedder = HttpEmbedder::new(embedder_cfg)?;

    let mut vectors = embedder.embed(&[query.to_string()]).await?;
    let query_embedding = vectors.pop().ok_or_else(|| "embedding provider returned no vector".to_string())?;

    let hits = search_similar_chunks(pool, notebook_id, query, &query_embedding, top_k)
        .await
        .map_err(|e| e.to_string())?;
    if hits.is_empty() {
        return Ok("No matching content found.".to_string());
    }
    Ok(format_search_hits(&hits))
}

pub(crate) async fn read_document(pool: &SqlitePool, notebook_id: &str, path: &str) -> Result<String, String> {
    let doc = get_document_by_path(pool, notebook_id, path)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no document found at path '{path}' in this notebook"))?;
    if doc.status != "ok" {
        return Err(format!("document has status '{}': {}", doc.status, doc.error_message.unwrap_or_default()));
    }
    let content = doc.markdown_cache.unwrap_or_default();
    if content.len() > MAX_READ_DOCUMENT_BYTES {
        Ok(format!(
            "{}\n\n[TRUNCATED: document exceeds size limit]",
            safe_truncate(&content, MAX_READ_DOCUMENT_BYTES)
        ))
    } else {
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use crate::db::knowledge_base::{create_notebook, replace_chunks, upsert_document};

    async fn setup_pool() -> SqlitePool {
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

    #[tokio::test]
    async fn list_notebooks_reports_created_notebooks() {
        let pool = setup_pool().await;
        create_notebook(&pool, "My Notes", "/tmp/notes", None, None, 0).await.unwrap();
        let out = list_notebooks(&pool).await.unwrap();
        assert!(out.contains("My Notes"));
    }

    #[tokio::test]
    async fn list_notebooks_reports_empty_state() {
        let pool = setup_pool().await;
        let out = list_notebooks(&pool).await.unwrap();
        assert!(out.contains("No notebooks"));
    }

    /// Two notebooks, each with their own document — proves search results
    /// are correctly scoped by notebook_id at the storage layer (this is
    /// what search_documents itself relies on via search_similar_chunks).
    #[tokio::test]
    async fn search_similar_chunks_is_scoped_to_the_given_notebook() {
        let pool = setup_pool().await;
        let nb_a = create_notebook(&pool, "A", "/tmp/a", None, None, 0).await.unwrap();
        let nb_b = create_notebook(&pool, "B", "/tmp/b", None, None, 0).await.unwrap();

        let doc_a = upsert_document(&pool, &nb_a.id, "a.md", 0, "h1", Some("content A"), "ok", None).await.unwrap();
        replace_chunks(&pool, &doc_a, &[("內容 A".into(), None, vec![1.0, 0.0, 0.0])]).await.unwrap();

        let doc_b = upsert_document(&pool, &nb_b.id, "b.md", 0, "h2", Some("content B"), "ok", None).await.unwrap();
        replace_chunks(&pool, &doc_b, &[("內容 B".into(), None, vec![1.0, 0.0, 0.0])]).await.unwrap();

        let hits_a = crate::db::knowledge_base::search_similar_chunks(&pool, &nb_a.id, "query", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        assert_eq!(hits_a.len(), 1);
        assert_eq!(hits_a[0].rel_path, "a.md");

        let hits_b = crate::db::knowledge_base::search_similar_chunks(&pool, &nb_b.id, "query", &[1.0, 0.0, 0.0], 10)
            .await
            .unwrap();
        assert_eq!(hits_b.len(), 1);
        assert_eq!(hits_b[0].rel_path, "b.md");
    }

    #[tokio::test]
    async fn search_documents_with_empty_query_returns_error() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", Some("p"), Some("m"), 0).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = crate::secret::SecretStore::new();
        let err = search_documents(&pool, &config, &secrets, &nb.id, "", None).await.unwrap_err();
        assert_eq!(err, "query is empty");
    }

    #[tokio::test]
    async fn search_documents_errors_on_notebook_with_no_embedder_configured() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = crate::secret::SecretStore::new();
        let err = search_documents(&pool, &config, &secrets, &nb.id, "query", None).await.unwrap_err();
        assert!(err.contains("embedding provider"), "{err}");
    }

    #[tokio::test]
    async fn read_document_returns_full_content() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        upsert_document(&pool, &nb.id, "report.md", 0, "hash1", Some("# Report\n\nfull content here"), "ok", None).await.unwrap();

        let out = read_document(&pool, &nb.id, "report.md").await.unwrap();
        assert_eq!(out, "# Report\n\nfull content here");
    }

    #[tokio::test]
    async fn read_document_errors_when_path_not_found() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let err = read_document(&pool, &nb.id, "missing.md").await.unwrap_err();
        assert!(err.contains("no document found"), "{err}");
    }

    #[tokio::test]
    async fn read_document_truncates_large_content() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let huge = "x".repeat(MAX_READ_DOCUMENT_BYTES + 1000);
        upsert_document(&pool, &nb.id, "huge.md", 0, "hashx", Some(&huge), "ok", None).await.unwrap();

        let out = read_document(&pool, &nb.id, "huge.md").await.unwrap();
        assert!(out.contains("TRUNCATED"));
        assert!(out.len() < MAX_READ_DOCUMENT_BYTES + 200);
    }
}

// src-tauri/tests/db_knowledge_base_integration.rs
use sqlx::sqlite::SqlitePoolOptions;
use aiterm_lib::db::knowledge_base::{
    create_notebook, get_notebook, list_notebooks, delete_notebook, mark_synced,
    upsert_document, list_documents, delete_document_by_path, replace_chunks,
    search_similar_chunks, cosine_similarity,
    KnowledgeBaseDb,
};

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

    sqlx::query(
        "CREATE TABLE kb_chat_sessions (
            id TEXT PRIMARY KEY NOT NULL, notebook_id TEXT NOT NULL, title TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("create kb_chat_sessions table");

    sqlx::query(
        "CREATE TABLE kb_chat_messages (
            id TEXT PRIMARY KEY NOT NULL, session_id TEXT NOT NULL, role TEXT NOT NULL,
            content TEXT NOT NULL, tool_calls_json TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("create kb_chat_messages table");

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

#[tokio::test]
async fn notebook_crud_roundtrip() {
    let pool = setup_pool().await;

    let created = create_notebook(&pool, "My Docs", "/tmp/docs", Some("ollama-local"), Some("nomic-embed-text"), 768)
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

#[tokio::test]
async fn real_init_creates_working_schema() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory pool");
    let db = KnowledgeBaseDb { pool };
    db.init().await.expect("init should succeed against a fresh in-memory db");

    let created = create_notebook(&db.pool, "Real Init Notebook", "/tmp/x", None, None, 0)
        .await.expect("create notebook against real init() schema");
    let fetched = get_notebook(&db.pool, &created.id).await.expect("get notebook");
    assert_eq!(fetched.name, "Real Init Notebook");
}

#[tokio::test]
async fn delete_notebook_cascades_to_documents_and_chunks() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();

    sqlx::query(
        "INSERT INTO documents (id, notebook_id, rel_path, mtime, content_hash, status)
         VALUES ('doc-1', ?, 'a.txt', 0, 'hash1', 'ok')"
    ).bind(&notebook.id).execute(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO chunks (id, document_id, chunk_index, text, embedding)
         VALUES ('chunk-1', 'doc-1', 0, 'some text', x'00')"
    ).execute(&pool).await.unwrap();

    let doc_count_before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM documents WHERE notebook_id = ?")
        .bind(&notebook.id).fetch_one(&pool).await.unwrap();
    assert_eq!(doc_count_before.0, 1);

    delete_notebook(&pool, &notebook.id).await.expect("delete notebook");

    let doc_count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM documents WHERE notebook_id = ?")
        .bind(&notebook.id).fetch_one(&pool).await.unwrap();
    let chunk_count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chunks WHERE document_id = 'doc-1'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(doc_count_after.0, 0, "documents should be deleted when notebook is deleted");
    assert_eq!(chunk_count_after.0, 0, "chunks should be deleted when notebook is deleted");
}

#[tokio::test]
async fn delete_notebook_cascades_to_chat_sessions() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();

    sqlx::query(
        "INSERT INTO kb_chat_sessions (id, notebook_id, title)
         VALUES ('session-1', ?, 'first question')"
    ).bind(&notebook.id).execute(&pool).await.unwrap();

    sqlx::query(
        "INSERT INTO kb_chat_messages (id, session_id, role, content)
         VALUES ('msg-1', 'session-1', 'user', 'hello')"
    ).execute(&pool).await.unwrap();

    let session_count_before: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kb_chat_sessions WHERE notebook_id = ?")
        .bind(&notebook.id).fetch_one(&pool).await.unwrap();
    assert_eq!(session_count_before.0, 1);

    delete_notebook(&pool, &notebook.id).await.expect("delete notebook");

    let session_count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kb_chat_sessions WHERE notebook_id = ?")
        .bind(&notebook.id).fetch_one(&pool).await.unwrap();
    let message_count_after: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM kb_chat_messages WHERE session_id = 'session-1'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(session_count_after.0, 0, "chat sessions should be deleted when notebook is deleted");
    assert_eq!(message_count_after.0, 0, "chat messages should be deleted when notebook is deleted");
}

#[test]
fn cosine_similarity_known_values() {
    assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
}

#[tokio::test]
async fn document_and_chunk_lifecycle() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.expect("create notebook");

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

    let hits = search_similar_chunks(&pool, &notebook.id, "", &[1.0, 0.0, 0.0], 10)
        .await.expect("search chunks");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].text, "chunk one about apples");
    assert!(hits[0].score > hits[1].score);

    delete_document_by_path(&pool, &notebook.id, "report.pdf").await.expect("delete document");
    let docs_final = list_documents(&pool, &notebook.id).await.expect("list after delete");
    assert!(docs_final.is_empty());
    let hits_final = search_similar_chunks(&pool, &notebook.id, "", &[1.0, 0.0, 0.0], 10)
        .await.expect("search after delete");
    assert!(hits_final.is_empty(), "deleting a document must cascade-delete its chunks");
}

#[tokio::test]
async fn search_excludes_chunks_from_error_status_documents() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();

    let doc_id = upsert_document(
        &pool, &notebook.id, "stale.txt", 0, "old-hash",
        Some("# Stale\n\nold content"), "error", Some("re-sync failed"),
    ).await.unwrap();

    replace_chunks(&pool, &doc_id, &[
        ("stale chunk content".into(), Some("Stale".into()), vec![1.0, 0.0, 0.0]),
    ]).await.unwrap();

    let hits = search_similar_chunks(&pool, &notebook.id, "", &[1.0, 0.0, 0.0], 10).await.unwrap();
    assert!(hits.is_empty(), "chunks belonging to an error-status document must never be returned by search, even if they still exist in the table");
}

// Regression test for a real reported bug: a question phrased quite differently
// from the source document's wording (e.g. a Chinese question about an
// English-only technical section) failed to surface the right chunk purely on
// cosine similarity, even though the chunk literally contains the query's key
// terms. Keyword-overlap boosting should let a lexically-matching chunk
// outrank a purely-more-similar-but-unrelated one.
#[tokio::test]
async fn keyword_match_can_outrank_higher_cosine_similarity() {
    let pool = setup_pool().await;
    let notebook = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();

    let doc_id = upsert_document(
        &pool, &notebook.id, "swift_sdk.pdf", 0, "hash1",
        Some("# Swift SDK"), "ok", None,
    ).await.unwrap();

    replace_chunks(&pool, &doc_id, &[
        // Higher raw cosine similarity to the query embedding, but no lexical
        // overlap with the query terms at all.
        ("unrelated filler content lorem ipsum".into(), Some("Intro".into()), vec![0.9, 0.1, 0.0]),
        // Lower raw cosine similarity, but literally contains the query's
        // technical terms — this is the chunk that should win after boosting.
        ("Retry of Tracker API calls documentation".into(), Some("7.3".into()), vec![0.8, 0.2, 0.0]),
    ]).await.unwrap();

    let hits = search_similar_chunks(
        &pool, &notebook.id, "Tracker API retry logic", &[1.0, 0.0, 0.0], 10,
    ).await.unwrap();

    assert_eq!(hits.len(), 2);
    assert_eq!(
        hits[0].text, "Retry of Tracker API calls documentation",
        "the lexically-matching chunk should be boosted above the purely-more-cosine-similar one"
    );
}

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

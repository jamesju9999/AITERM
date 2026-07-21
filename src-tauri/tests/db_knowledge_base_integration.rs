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

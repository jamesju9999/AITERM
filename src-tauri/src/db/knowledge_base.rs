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

    pub async fn init(&self) -> Result<(), sqlx::Error> {
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
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chunks WHERE document_id IN (SELECT id FROM documents WHERE notebook_id = ?)")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM documents WHERE notebook_id = ?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM notebooks WHERE id = ?").bind(id).execute(&mut *tx).await?;
    tx.commit().await
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

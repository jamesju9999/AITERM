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

    async fn init(&self) -> Result<(), sqlx::Error> {
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

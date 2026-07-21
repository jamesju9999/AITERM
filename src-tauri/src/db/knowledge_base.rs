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

fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for f in v {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    buf
}

fn decode_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

pub async fn upsert_document(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
    mtime: i64,
    content_hash: &str,
    markdown_cache: Option<&str>,
    status: &str,
    error_message: Option<&str>,
) -> Result<String, sqlx::Error> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await?;

    let id = match existing {
        Some((id,)) => {
            sqlx::query(
                "UPDATE documents SET mtime = ?, content_hash = ?, markdown_cache = ?, status = ?, error_message = ?
                 WHERE id = ?"
            )
            .bind(mtime).bind(content_hash).bind(markdown_cache).bind(status).bind(error_message)
            .bind(&id)
            .execute(pool).await?;
            id
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO documents (id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&id).bind(notebook_id).bind(rel_path).bind(mtime).bind(content_hash)
            .bind(markdown_cache).bind(status).bind(error_message)
            .execute(pool).await?;
            id
        }
    };
    Ok(id)
}

pub async fn list_documents(pool: &SqlitePool, notebook_id: &str) -> Result<Vec<DocumentRow>, sqlx::Error> {
    sqlx::query_as::<_, DocumentRow>(
        "SELECT id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message
         FROM documents WHERE notebook_id = ?"
    ).bind(notebook_id).fetch_all(pool).await
}

pub async fn delete_document_by_path(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
) -> Result<(), sqlx::Error> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await?;

    if let Some((doc_id,)) = existing {
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM chunks WHERE document_id = ?").bind(&doc_id).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM documents WHERE id = ?").bind(&doc_id).execute(&mut *tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

pub async fn replace_chunks(
    pool: &SqlitePool,
    document_id: &str,
    chunks: &[(String, Option<String>, Vec<f32>)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chunks WHERE document_id = ?").bind(document_id).execute(&mut *tx).await?;

    for (idx, (text, location_hint, embedding)) in chunks.iter().enumerate() {
        let id = uuid::Uuid::new_v4().to_string();
        let blob = encode_embedding(embedding);
        sqlx::query(
            "INSERT INTO chunks (id, document_id, chunk_index, text, location_hint, embedding)
             VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(&id).bind(document_id).bind(idx as i64).bind(text).bind(location_hint).bind(&blob)
        .execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub document_id: String,
    pub rel_path: String,
    pub text: String,
    pub location_hint: Option<String>,
    pub score: f32,
}

pub async fn search_similar_chunks(
    pool: &SqlitePool,
    notebook_id: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SearchHit>, sqlx::Error> {
    #[derive(FromRow)]
    struct Row {
        document_id: String,
        rel_path: String,
        text: String,
        location_hint: Option<String>,
        embedding: Vec<u8>,
    }

    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.document_id, d.rel_path, c.text, c.location_hint, c.embedding
         FROM chunks c JOIN documents d ON c.document_id = d.id
         WHERE d.notebook_id = ?"
    ).bind(notebook_id).fetch_all(pool).await?;

    let mut hits: Vec<SearchHit> = rows.into_iter().map(|r| {
        let embedding = decode_embedding(&r.embedding);
        let score = cosine_similarity(query_embedding, &embedding);
        SearchHit {
            document_id: r.document_id,
            rel_path: r.rel_path,
            text: r.text,
            location_hint: r.location_hint,
            score,
        }
    }).collect();

    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(top_k);
    Ok(hits)
}

// src-tauri/src/db/knowledge_base.rs
use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::fs;
use std::time::Duration;

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
        // `SqlitePool::connect` does NOT create a missing database file by default
        // (sqlx's SqliteConnectOptions defaults create_if_missing to false) — it
        // simply fails to open, and the fallback below would then silently swap in
        // an in-memory database with no error surfaced anywhere. That means every
        // notebook/document/chunk would vanish on app restart. Explicitly opt in to
        // file creation so a first run actually persists to disk.
        // MCP tool server（見 src-tauri/src/mcp_server/mod.rs）建立自己獨立的一份
        // KnowledgeBaseDb，跟這裡（app 本身管理的那份）寫同一個檔案。沒有
        // busy_timeout 的話，兩邊剛好同時寫入會直接回 SQLITE_BUSY，而不是等一下
        // 重試——對 MCP client 來說會是一個沒必要、偶發的查詢失敗。
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePool::connect_with(options).await.unwrap_or_else(|_| {
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

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kb_chat_sessions (
                id TEXT PRIMARY KEY NOT NULL,
                notebook_id TEXT NOT NULL,
                title TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS kb_chat_messages (
                id TEXT PRIMARY KEY NOT NULL,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls_json TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kb_chat_sessions_notebook ON kb_chat_sessions(notebook_id)")
            .execute(&self.pool).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kb_chat_messages_session ON kb_chat_messages(session_id)")
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
    embed_dim: i64,
) -> Result<NotebookRow, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notebooks (id, name, folder_path, embed_provider_id, embed_model, embed_dim)
         VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id).bind(name).bind(folder_path).bind(embed_provider_id).bind(embed_model).bind(embed_dim)
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
    sqlx::query("DELETE FROM kb_chat_messages WHERE session_id IN (SELECT id FROM kb_chat_sessions WHERE notebook_id = ?)")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kb_chat_sessions WHERE notebook_id = ?").bind(id).execute(&mut *tx).await?;
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

/// Weight applied to the keyword-overlap fraction before adding it to cosine
/// similarity. Cosine similarity is typically in the ~0.3-0.8 range for
/// plausible matches. Started at 0.15, but real-world testing showed that was
/// too weak: a query with several distinctive technical terms (e.g. "Tracker
/// API 指數退避 exponential backoff") only matched ~40% of its tokens against
/// the actually-relevant chunk (the rest of the query's terms legitimately
/// don't appear in that chunk), yielding a ~0.06 boost — nowhere near enough
/// to overcome unrelated documents scoring 0.7+ on pure semantic similarity,
/// so the correct chunk didn't even make the top 10.
const KEYWORD_BOOST_WEIGHT: f32 = 0.4;

/// Cheap lexical signal to supplement pure vector similarity: dense embeddings
/// can under-rank a chunk that literally contains the query's technical terms
/// (e.g. a Chinese question about an English-only section heading), especially
/// across languages. Splits the query into tokens on whitespace/punctuation,
/// drops single-character tokens (filters out most CJK function words like
/// 的/是/了 without needing a stopword list), and scores the fraction of
/// remaining tokens found as a case-insensitive substring of the chunk text.
fn keyword_boost(query: &str, text: &str) -> f32 {
    let text_lower = text.to_lowercase();
    let tokens: Vec<String> = query
        .split(|c: char| c.is_whitespace() || ",.;:!?、，。；：！？()（）[]「」".contains(c))
        .map(|s| s.to_lowercase())
        .filter(|s| s.chars().count() >= 2)
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let matched = tokens.iter().filter(|t| text_lower.contains(t.as_str())).count();
    (matched as f32 / tokens.len() as f32) * KEYWORD_BOOST_WEIGHT
}

pub async fn search_similar_chunks(
    pool: &SqlitePool,
    notebook_id: &str,
    query_text: &str,
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
         WHERE d.notebook_id = ? AND d.status = 'ok'"
    ).bind(notebook_id).fetch_all(pool).await?;

    let mut hits: Vec<SearchHit> = rows.into_iter().map(|r| {
        let embedding = decode_embedding(&r.embedding);
        let score = cosine_similarity(query_embedding, &embedding) + keyword_boost(query_text, &r.text);
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

pub async fn get_document_by_path(
    pool: &SqlitePool,
    notebook_id: &str,
    rel_path: &str,
) -> Result<Option<DocumentRow>, sqlx::Error> {
    sqlx::query_as::<_, DocumentRow>(
        "SELECT id, notebook_id, rel_path, mtime, content_hash, markdown_cache, status, error_message
         FROM documents WHERE notebook_id = ? AND rel_path = ?"
    ).bind(notebook_id).bind(rel_path).fetch_optional(pool).await
}

#[cfg(test)]
mod keyword_boost_tests {
    use super::{keyword_boost, KEYWORD_BOOST_WEIGHT};

    #[test]
    fn full_match_gives_max_boost() {
        let score = keyword_boost("Tracker API retry", "Retry of Tracker API calls");
        assert!((score - KEYWORD_BOOST_WEIGHT).abs() < 1e-6);
    }

    #[test]
    fn partial_match_is_proportional() {
        // 3 of 4 tokens ("tracker", "api", "retry") appear; "logic" does not.
        let score = keyword_boost("Tracker API retry logic", "Retry of Tracker API calls documentation");
        assert!((score - KEYWORD_BOOST_WEIGHT * 0.75).abs() < 1e-6);
    }

    #[test]
    fn no_match_gives_zero_boost() {
        let score = keyword_boost("Tracker API retry", "unrelated filler content");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn empty_query_gives_zero_boost() {
        assert_eq!(keyword_boost("", "some chunk text"), 0.0);
    }

    #[test]
    fn is_case_insensitive() {
        let score = keyword_boost("TRACKER api", "a chunk mentioning tracker API here");
        assert!((score - KEYWORD_BOOST_WEIGHT).abs() < 1e-6);
    }

    #[test]
    fn single_char_tokens_are_ignored() {
        // "a" and "在" (single CJK char) should be filtered; only "retry" counts,
        // and it's present, so this should still score as a full match, not be
        // diluted by unmatched noise tokens that were never real keywords.
        let score = keyword_boost("a 在 retry", "the retry mechanism");
        assert!((score - KEYWORD_BOOST_WEIGHT).abs() < 1e-6);
    }
}

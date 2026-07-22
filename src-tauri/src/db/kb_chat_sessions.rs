use sqlx::{SqlitePool, FromRow};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ChatSessionSummary {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ChatMessageRow {
    pub role: String,
    pub content: String,
    pub tool_calls_json: Option<String>,
    pub created_at: String,
}

pub async fn create_chat_session(
    pool: &SqlitePool,
    notebook_id: &str,
    title: &str,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO kb_chat_sessions (id, notebook_id, title) VALUES (?, ?, ?)")
        .bind(&id).bind(notebook_id).bind(title)
        .execute(pool).await?;
    Ok(id)
}

pub async fn list_chat_sessions(
    pool: &SqlitePool,
    notebook_id: &str,
) -> Result<Vec<ChatSessionSummary>, sqlx::Error> {
    sqlx::query_as::<_, ChatSessionSummary>(
        "SELECT id, title, updated_at FROM kb_chat_sessions WHERE notebook_id = ? ORDER BY updated_at DESC"
    ).bind(notebook_id).fetch_all(pool).await
}

pub async fn load_chat_session_messages(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<ChatMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, ChatMessageRow>(
        "SELECT role, content, tool_calls_json, created_at FROM kb_chat_messages \
         WHERE session_id = ? ORDER BY created_at ASC, rowid ASC"
    ).bind(session_id).fetch_all(pool).await
}

pub async fn delete_chat_session(pool: &SqlitePool, session_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM kb_chat_messages WHERE session_id = ?")
        .bind(session_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kb_chat_sessions WHERE id = ?")
        .bind(session_id).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn create_chat_message(
    pool: &SqlitePool,
    session_id: &str,
    role: &str,
    content: &str,
    tool_calls_json: Option<&str>,
) -> Result<(), sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO kb_chat_messages (id, session_id, role, content, tool_calls_json) \
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&id).bind(session_id).bind(role).bind(content).bind(tool_calls_json)
    .execute(&mut *tx).await?;
    sqlx::query("UPDATE kb_chat_sessions SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(session_id).execute(&mut *tx).await?;
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:")
            .await.unwrap();
        sqlx::query(
            "CREATE TABLE kb_chat_sessions (
                id TEXT PRIMARY KEY NOT NULL, notebook_id TEXT NOT NULL, title TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE kb_chat_messages (
                id TEXT PRIMARY KEY NOT NULL, session_id TEXT NOT NULL, role TEXT NOT NULL,
                content TEXT NOT NULL, tool_calls_json TEXT, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn create_list_load_roundtrip() {
        let pool = setup().await;
        let id = create_chat_session(&pool, "nb-1", "第一個問題").await.unwrap();

        create_chat_message(&pool, &id, "user", "這份文件在講什麼？", None).await.unwrap();
        create_chat_message(
            &pool, &id, "assistant", "這份文件在講 X。",
            Some(r#"[{"tool":"search_documents","args":{"query":"主題"},"result":"..."}]"#),
        ).await.unwrap();

        let sessions = list_chat_sessions(&pool, "nb-1").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "第一個問題");

        let messages = load_chat_session_messages(&pool, &id).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].tool_calls_json.is_some());
    }

    #[tokio::test]
    async fn list_scoped_by_notebook() {
        let pool = setup().await;
        create_chat_session(&pool, "nb-1", "A").await.unwrap();
        create_chat_session(&pool, "nb-2", "B").await.unwrap();

        let sessions = list_chat_sessions(&pool, "nb-1").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "A");
    }

    #[tokio::test]
    async fn delete_removes_session_and_messages() {
        let pool = setup().await;
        let id = create_chat_session(&pool, "nb-1", "temp").await.unwrap();
        create_chat_message(&pool, &id, "user", "hi", None).await.unwrap();

        delete_chat_session(&pool, &id).await.unwrap();

        assert_eq!(list_chat_sessions(&pool, "nb-1").await.unwrap().len(), 0);
        assert_eq!(load_chat_session_messages(&pool, &id).await.unwrap().len(), 0);
    }
}

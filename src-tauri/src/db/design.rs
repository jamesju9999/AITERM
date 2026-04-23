// src-tauri/src/db/design.rs
use sqlx::{SqlitePool, FromRow};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DesignSession {
    pub id: String,
    pub title: String,
    pub current_spec_draft: Option<String>,
    pub current_sdd_draft: Option<String>,
    pub current_plan_draft: Option<String>,
    pub context_summary: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DesignMessage {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: Option<String>,
}

pub async fn create_design_session(pool: &SqlitePool, title: &str) -> Result<String, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO design_sessions (id, title, status) VALUES (?, ?, 'draft')")
        .bind(&id)
        .bind(title)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn get_design_session(pool: &SqlitePool, id: &str) -> Result<DesignSession, sqlx::Error> {
    let row = sqlx::query_as::<_, DesignSession>(
        "SELECT id, title, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions WHERE id = ?"
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

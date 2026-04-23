// src-tauri/tests/db_design_integration.rs
use aiterm_lib::db::design::{create_design_session, get_design_session};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn test_create_and_get_design_session() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory DB");

    // 手動執行 Migration 以建立表
    sqlx::query(
        "CREATE TABLE design_sessions (
            id TEXT PRIMARY KEY NOT NULL,
            title TEXT NOT NULL,
            current_spec_draft TEXT,
            current_sdd_draft TEXT,
            current_plan_draft TEXT,
            context_summary TEXT,
            status TEXT NOT NULL DEFAULT 'draft',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("Failed to create table");
    
    let session_id = create_design_session(&pool, "New Feature").await.expect("Failed to create session");
    
    let session = get_design_session(&pool, &session_id).await.expect("Failed to get session");
    assert_eq!(session.title, "New Feature");
    assert_eq!(session.status, "draft");
}

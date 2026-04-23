// src-tauri/src/commands/design.rs
use tauri::State;
use crate::db::design::{DesignDb, create_design_session, get_design_session, DesignSession};

#[tauri::command]
pub async fn design_start_session(
    design_db: State<'_, DesignDb>,
    title: String,
) -> Result<String, String> {
    create_design_session(&design_db.pool, &title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_load_session(
    design_db: State<'_, DesignDb>,
    id: String,
) -> Result<DesignSession, String> {
    get_design_session(&design_db.pool, &id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn design_list_sessions(
    design_db: State<'_, DesignDb>,
) -> Result<Vec<DesignSession>, String> {
    sqlx::query_as::<_, DesignSession>(
        "SELECT id, title, current_spec_draft, current_sdd_draft, current_plan_draft, context_summary, status FROM design_sessions ORDER BY updated_at DESC"
    )
    .fetch_all(&design_db.pool)
    .await
    .map_err(|e| e.to_string())
}

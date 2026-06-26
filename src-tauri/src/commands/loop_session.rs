use tauri::State;
use serde::{Serialize, Deserialize};
use crate::db::loop_sessions::{
    LoopSessionDb, LoopSessionRow, LoopSessionSummary,
    upsert_loop_session, list_loop_sessions, load_loop_session, delete_loop_session, clear_all_loop_sessions,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LoopSessionSaveArgs {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub iteration: i64,
    pub config_json: String,
    pub history_json: String,
    pub shared_context: String,
    pub trace_json: String,
}

#[tauri::command]
pub async fn loop_session_save(
    args: LoopSessionSaveArgs,
    db: State<'_, LoopSessionDb>,
) -> Result<(), String> {
    upsert_loop_session(
        &db.pool,
        &args.id, &args.goal, &args.status,
        args.iteration,
        &args.config_json, &args.history_json,
        &args.shared_context, &args.trace_json,
    ).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_list(
    db: State<'_, LoopSessionDb>,
) -> Result<Vec<LoopSessionSummary>, String> {
    list_loop_sessions(&db.pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_load(
    id: String,
    db: State<'_, LoopSessionDb>,
) -> Result<LoopSessionRow, String> {
    load_loop_session(&db.pool, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_delete(
    id: String,
    db: State<'_, LoopSessionDb>,
) -> Result<(), String> {
    delete_loop_session(&db.pool, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn loop_session_clear_all(
    db: State<'_, LoopSessionDb>,
) -> Result<(), String> {
    clear_all_loop_sessions(&db.pool).await.map_err(|e| e.to_string())
}

/// Open native file-open dialog filtered to .loopstudio.json files.
/// Returns the selected path, or None if cancelled.
#[tauri::command]
pub async fn loop_project_pick_open() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter("Loop Studio 專案", &["loopstudio.json", "json"])
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

/// Open native file-save dialog for .loopstudio.json files.
/// Returns the chosen path, or None if cancelled.
#[tauri::command]
pub async fn loop_project_pick_save() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter("Loop Studio 專案", &["loopstudio.json", "json"])
        .set_file_name("loop-project.loopstudio.json")
        .save_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

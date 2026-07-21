use tauri::State;
use crate::db::knowledge_base::{
    KnowledgeBaseDb, NotebookRow,
    create_notebook, list_notebooks, delete_notebook,
};

#[tauri::command]
pub async fn kb_create_notebook(
    name: String,
    folder_path: String,
    embed_provider_id: Option<String>,
    embed_model: Option<String>,
    db: State<'_, KnowledgeBaseDb>,
) -> Result<NotebookRow, String> {
    create_notebook(
        &db.pool, &name, &folder_path,
        embed_provider_id.as_deref(), embed_model.as_deref(),
    ).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_list_notebooks(db: State<'_, KnowledgeBaseDb>) -> Result<Vec<NotebookRow>, String> {
    list_notebooks(&db.pool).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kb_delete_notebook(id: String, db: State<'_, KnowledgeBaseDb>) -> Result<(), String> {
    delete_notebook(&db.pool, &id).await.map_err(|e| e.to_string())
}

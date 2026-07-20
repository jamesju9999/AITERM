use tauri::{AppHandle, State};
use crate::ai::{router::AiRouter, ChatMessage, Locale, AiError};

#[tauri::command]
pub async fn code_assistant_chat(
    project_root: String,
    messages: Vec<ChatMessage>,
    session_id: String,
    provider_id: Option<String>,
    locale: Locale,
    app: AppHandle,
    router: State<'_, AiRouter>,
) -> Result<(), AiError> {
    if messages.is_empty() {
        return Err(AiError::InvalidInput { reason: "empty messages".into() });
    }
    if project_root.is_empty() {
        return Err(AiError::InvalidInput { reason: "project_root is empty".into() });
    }

    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await?,
        None => router.resolve().await?,
    };

    crate::code_assistant::run_chat(project_root, messages, provider, session_id, locale, app).await
}

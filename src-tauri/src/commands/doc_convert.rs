// src-tauri/src/commands/doc_convert.rs
use std::path::Path;
use tauri::AppHandle;

/// Convert a local file to Markdown, routing between anydoc and MarkItDown
/// per `document_convert::convert_document`. `provider_id` is used for
/// image vision when the file ends up going through MarkItDown.
#[tauri::command]
pub async fn document_convert(
    app: AppHandle,
    file_path: String,
    provider_id: Option<String>,
) -> Result<String, String> {
    crate::document_convert::convert_document(app, Path::new(&file_path), provider_id).await
}

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn document_convert_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter("Documents", crate::document_convert::SUPPORTED_EXTENSIONS)
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

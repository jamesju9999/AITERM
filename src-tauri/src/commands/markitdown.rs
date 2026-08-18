// src-tauri/src/commands/markitdown.rs

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn markitdown_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Documents",
            &[
                "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
                "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
                "txt", "md", "rst", "xml", "json", "yaml", "yml",
                "mp3", "wav", "m4a", "flac",
            ],
        )
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

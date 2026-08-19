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

// document_convert::SUPPORTED_EXTENSIONS deliberately has no audio entries —
// routing never needs them (anydoc simply doesn't recognize audio
// extensions, so they fall through to MarkItDown regardless of any list),
// and the knowledge-base folder scanner reuses the same constant but must
// NOT pick up audio files (it never gates on the separate doc_audio Python
// profile the way this single-file tool does). The file-picker filter is the
// one place that DOES need audio listed, so it's added on here rather than
// folded into the shared constant.
fn pick_file_extensions() -> Vec<&'static str> {
    crate::document_convert::SUPPORTED_EXTENSIONS
        .iter()
        .copied()
        .chain(["mp3", "wav", "m4a", "flac"])
        .collect()
}

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn document_convert_pick_file() -> Option<String> {
    let extensions = pick_file_extensions();
    rfd::AsyncFileDialog::new()
        .add_filter("Documents", &extensions)
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_file_extensions_includes_audio_formats_not_covered_by_either_engine() {
        // Regression test: document_convert_pick_file used to reuse
        // SUPPORTED_EXTENSIONS directly, which silently dropped audio
        // support that the old markitdown_pick_file filter had (mp3/wav/m4a/
        // flac aren't in SUPPORTED_EXTENSIONS, since neither anydoc nor the
        // KB scanner's routing list has any use for them).
        let extensions = pick_file_extensions();
        for audio_ext in ["mp3", "wav", "m4a", "flac"] {
            assert!(
                extensions.contains(&audio_ext),
                "{audio_ext} should be pickable in the manual doc-converter file dialog",
            );
        }
    }

    #[test]
    fn pick_file_extensions_still_includes_everything_supported_extensions_has() {
        for ext in crate::document_convert::SUPPORTED_EXTENSIONS {
            assert!(
                pick_file_extensions().contains(ext),
                "{ext} from SUPPORTED_EXTENSIONS is missing from the file-picker filter",
            );
        }
    }
}

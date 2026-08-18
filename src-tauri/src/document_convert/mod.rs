//! Routes document conversion between anydoc (fast, pure Rust, no Python) and
//! MarkItDown (Python sidecar; handles images via vision, audio transcription,
//! `.msg`, html, and plain-text formats that anydoc doesn't touch).

/// Which engine converts a given file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Anydoc,
    MarkItDown,
}

/// Decide which engine converts a file with this extension (no leading dot,
/// matched case-insensitively). Anydoc's own format table
/// (`anydoc::Format::from_extension`) is the source of truth for what it
/// supports — see `ANYDOC_EXTENSIONS`'s drift test below for why this isn't
/// a hand-maintained list here too.
pub fn engine_for_extension(ext: &str) -> Engine {
    if anydoc::Format::from_extension(ext).is_some() {
        Engine::Anydoc
    } else {
        Engine::MarkItDown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_and_pdf_formats_route_to_anydoc() {
        for ext in ["docx", "doc", "docm", "pdf", "pptx", "ppt", "xlsx", "xls", "csv", "epub", "rtf", "odt", "ods", "odp"] {
            assert_eq!(engine_for_extension(ext), Engine::Anydoc, "{ext} should route to anydoc");
        }
    }

    #[test]
    fn images_audio_msg_html_and_plain_text_route_to_markitdown() {
        for ext in ["jpg", "jpeg", "png", "gif", "webp", "mp3", "wav", "m4a", "flac", "msg", "html", "htm", "txt", "md", "rst", "xml", "json", "yaml", "yml"] {
            assert_eq!(engine_for_extension(ext), Engine::MarkItDown, "{ext} should route to markitdown");
        }
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        assert_eq!(engine_for_extension("DOCX"), Engine::Anydoc);
        assert_eq!(engine_for_extension("PNG"), Engine::MarkItDown);
    }

    #[test]
    fn unrecognized_extension_falls_back_to_markitdown() {
        // MarkItDown's converter.py already handles "no extension I recognize"
        // by erroring cleanly; anydoc's error for the same case is less
        // informative ("unrecognized file content and extension"), so an
        // unknown extension should not be routed to anydoc at all.
        assert_eq!(engine_for_extension("xyz123"), Engine::MarkItDown);
    }
}

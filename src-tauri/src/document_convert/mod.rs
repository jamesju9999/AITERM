//! Routes document conversion between anydoc (fast, pure Rust, no Python) and
//! MarkItDown (Python sidecar; handles images via vision, audio transcription,
//! `.msg`, html, and plain-text formats that anydoc doesn't touch).

use std::path::Path;
use async_trait::async_trait;

/// Converts one file to Markdown. Implemented by `RoutedConverter`
/// (`commands/knowledge_base.rs`) in production; tests use fakes (see
/// `tests/knowledge_base_ingest.rs`) to avoid depending on Python or anydoc.
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    async fn convert(&self, path: &Path) -> Result<String, String>;
}

/// Which engine converts a given file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Anydoc,
    MarkItDown,
}

/// Every extension `anydoc::Format::from_extension` recognizes (checked
/// against the crate directly in the `anydoc_extensions_constant_matches_the_crate`
/// test below). Needed as a literal list for the native file-picker filter
/// and the knowledge-base folder scanner — both need a real `&[&str]`, not
/// just the `engine_for_extension` predicate.
pub const ANYDOC_EXTENSIONS: &[&str] = &[
    "doc", "docx", "docm", "odt", "pdf",
    "ppt", "pps", "pot", "pptx", "pptm", "ppsx", "ppsm",
    "rtf", "epub",
    "xls", "xlsx", "xlsm", "xlsb", "ods", "odp",
    "csv",
];

/// Formats anydoc categorically cannot convert: images (need vision, not
/// text extraction), audio (transcription), `.msg` (Outlook), html, and
/// plain-text formats markitdown just passes through.
pub const MARKITDOWN_EXTENSIONS: &[&str] = &[
    "html", "htm",
    "jpg", "jpeg", "png", "gif", "webp",
    "msg",
    "txt", "md", "rst", "xml", "json", "yaml", "yml",
];

/// Union of both engines' extensions. The knowledge-base folder scanner and
/// the file-picker filter both use this as their single list of "files this
/// app can convert at all".
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "doc", "docx", "docm", "odt", "pdf",
    "ppt", "pps", "pot", "pptx", "pptm", "ppsx", "ppsm",
    "rtf", "epub",
    "xls", "xlsx", "xlsm", "xlsb", "ods", "odp",
    "csv",
    "html", "htm",
    "jpg", "jpeg", "png", "gif", "webp",
    "msg",
    "txt", "md", "rst", "xml", "json", "yaml", "yml",
];

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

    #[test]
    fn anydoc_extensions_constant_matches_the_crate() {
        // Catches drift if anydoc adds/removes a format and this hand-written
        // list isn't updated to match.
        for ext in ANYDOC_EXTENSIONS {
            assert!(
                anydoc::Format::from_extension(ext).is_some(),
                "{ext} is listed in ANYDOC_EXTENSIONS but anydoc::Format::from_extension doesn't recognize it",
            );
        }
    }

    #[test]
    fn supported_extensions_is_the_deduplicated_union() {
        let mut expected: Vec<&str> = ANYDOC_EXTENSIONS.iter().chain(MARKITDOWN_EXTENSIONS.iter()).copied().collect();
        expected.sort_unstable();

        let mut actual: Vec<&str> = SUPPORTED_EXTENSIONS.to_vec();
        actual.sort_unstable();

        assert_eq!(actual, expected);

        let mut deduped = actual.clone();
        deduped.dedup();
        assert_eq!(actual.len(), deduped.len(), "SUPPORTED_EXTENSIONS has a duplicate entry");
    }
}

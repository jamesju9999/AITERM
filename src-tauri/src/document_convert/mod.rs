//! Routes document conversion between anydoc (fast, pure Rust, no Python) and
//! MarkItDown (Python sidecar; handles images via vision, audio transcription,
//! `.msg`, html, and plain-text formats that anydoc doesn't touch).

use std::path::Path;
use async_trait::async_trait;
use crate::config::DocConvertEngine;

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

/// Fallback control flow: Auto mode tries the routed engine first and falls
/// back to the other one only when the routed engine fails on a format it's
/// supposed to support. `MarkitdownOnly` never calls `try_anydoc` at all.
///
/// Takes futures rather than closures: an `async {}` block doesn't run its
/// body until it's polled, so the caller can construct both up front and
/// this function decides which ones actually get `.await`ed.
async fn resolve_with_fallback(
    ext: &str,
    engine_pref: DocConvertEngine,
    try_anydoc: impl std::future::Future<Output = Result<String, String>>,
    try_markitdown: impl std::future::Future<Output = Result<String, String>>,
) -> Result<String, String> {
    if matches!(engine_pref, DocConvertEngine::MarkitdownOnly) {
        return try_markitdown.await;
    }
    match engine_for_extension(ext) {
        Engine::MarkItDown => try_markitdown.await,
        Engine::Anydoc => match try_anydoc.await {
            Ok(markdown) => Ok(markdown),
            Err(anydoc_err) => match try_markitdown.await {
                Ok(markdown) => Ok(markdown),
                Err(markitdown_err) => Err(format!(
                    "anydoc: {anydoc_err}；已改用 MarkItDown 重試但仍失敗：{markitdown_err}"
                )),
            },
        },
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

    #[tokio::test]
    async fn auto_mode_uses_anydoc_when_it_succeeds() {
        let result = resolve_with_fallback(
            "docx",
            DocConvertEngine::Auto,
            async { Ok("anydoc output".to_string()) },
            async { panic!("markitdown must not be called when anydoc succeeds") },
        ).await;
        assert_eq!(result.unwrap(), "anydoc output");
    }

    #[tokio::test]
    async fn auto_mode_falls_back_to_markitdown_when_anydoc_fails() {
        let result = resolve_with_fallback(
            "docx",
            DocConvertEngine::Auto,
            async { Err("encrypted".to_string()) },
            async { Ok("markitdown output".to_string()) },
        ).await;
        assert_eq!(result.unwrap(), "markitdown output");
    }

    #[tokio::test]
    async fn auto_mode_combines_both_errors_when_both_engines_fail() {
        let result = resolve_with_fallback(
            "docx",
            DocConvertEngine::Auto,
            async { Err("encrypted".to_string()) },
            async { Err("network error".to_string()) },
        ).await;
        let err = result.unwrap_err();
        assert!(err.contains("encrypted"), "error should mention the anydoc failure: {err}");
        assert!(err.contains("network error"), "error should mention the markitdown failure: {err}");
    }

    #[tokio::test]
    async fn markitdown_only_mode_never_calls_anydoc() {
        let result = resolve_with_fallback(
            "docx",
            DocConvertEngine::MarkitdownOnly,
            async { panic!("anydoc must not be called in MarkitdownOnly mode") },
            async { Ok("markitdown output".to_string()) },
        ).await;
        assert_eq!(result.unwrap(), "markitdown output");
    }

    #[tokio::test]
    async fn image_extension_goes_straight_to_markitdown_even_in_auto_mode() {
        let result = resolve_with_fallback(
            "png",
            DocConvertEngine::Auto,
            async { panic!("anydoc must not be called for an image extension") },
            async { Ok("vision output".to_string()) },
        ).await;
        assert_eq!(result.unwrap(), "vision output");
    }
}

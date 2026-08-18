//! Routes document conversion between anydoc (fast, pure Rust, no Python) and
//! MarkItDown (Python sidecar; handles images via vision, audio transcription,
//! `.msg`, html, and plain-text formats that anydoc doesn't touch).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use async_trait::async_trait;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncBufReadExt;
use serde::Deserialize;
use crate::config::{ConfigStore, DocConvertEngine, ProviderType};
use crate::secret::SecretStore;

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

fn converter_script_path(app: &AppHandle) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("MarkItDown")
        .join("converter.py");
    if dev_path.exists() {
        return dev_path;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("MarkItDown").join("converter.py");
        if prod_path.exists() {
            return prod_path;
        }
    }
    dev_path
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Done { markdown: String },
    Error { message: String },
}

/// Resolve LLM credentials for image vision from the configured provider.
/// Returns (provider_type_str, api_key, base_url, model) or None if unavailable.
fn resolve_vision_credentials(
    config: &ConfigStore,
    secrets: &SecretStore,
    provider_id: &str,
) -> Option<(String, String, String, String)> {
    let cfg = config.get_provider(provider_id)?;
    let api_key = secrets.get(provider_id).ok().flatten().unwrap_or_default();

    let (provider_type_str, base_url) = match cfg.provider_type {
        ProviderType::Openai => (
            "openai".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        ),
        ProviderType::Anthropic => (
            "anthropic".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        ),
        ProviderType::Ollama => (
            "ollama".to_string(),
            cfg.base_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        ),
        ProviderType::OpenaiCompatible => (
            "openai-compatible".to_string(),
            cfg.base_url.unwrap_or_default(),
        ),
        // GitHub Copilot and Google AI have complex OAuth/auth flows — skip for now
        _ => return None,
    };

    Some((provider_type_str, api_key, base_url, cfg.model))
}

/// Convert a local file to Markdown using MarkItDown.
/// Auto-installs Python deps on first use (fast no-op if already installed).
/// `provider_id` is used for image vision (passes AI credentials to converter.py).
async fn convert_with_markitdown(
    app: AppHandle,
    file_path: String,
    provider_id: Option<String>,
    config: tauri::State<'_, Arc<ConfigStore>>,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let script = converter_script_path(&app);
    let python = crate::python_env::ensure(&app, crate::python_env::profiles::Profile::DocCore)
        .await
        .map_err(String::from)?;
    let script_dir = script.parent().unwrap_or(script.as_path());

    // Resolve vision credentials (if a provider is selected)
    let vision_creds = provider_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .and_then(|id| resolve_vision_credentials(&config, &secrets, id));

    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(&script)
        .arg(&file_path)
        .current_dir(script_dir)
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // An AppImage's AppRun exports PYTHONHOME into every child; without this the
    // interpreter can't find its own standard library.
    crate::appimage_env::strip_appimage_env(cmd.as_std_mut());

    // Pass vision credentials as environment variables
    if let Some((provider_type_str, api_key, base_url, model)) = vision_creds {
        cmd.env("MARKITDOWN_LLM_PROVIDER_TYPE", provider_type_str)
           .env("MARKITDOWN_LLM_API_KEY", api_key)
           .env("MARKITDOWN_LLM_BASE_URL", base_url)
           .env("MARKITDOWN_LLM_MODEL", model);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn Python: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut result: Option<String> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Done { markdown }) => {
                result = Some(markdown);
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {}
        }
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() && result.is_none() {
        let detail = if stderr_output.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr_output.trim())
        };
        return Err(format!(
            "converter.py exited with code {:?}{}",
            status.code(),
            detail
        ));
    }

    result.ok_or_else(|| "converter.py did not emit markdown".to_string())
}

/// Runs anydoc's synchronous conversion on the blocking thread pool.
async fn convert_with_anydoc(file_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || anydoc::to_markdown(&file_path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("anydoc conversion task panicked: {e}"))?
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

    #[tokio::test]
    async fn anydoc_converts_a_real_csv_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.csv");
        std::fs::write(&path, "name,age\nAlice,30\nBob,25\n").unwrap();

        let markdown = convert_with_anydoc(path.to_string_lossy().to_string()).await
            .expect("anydoc should convert a simple CSV");

        assert!(markdown.contains("Alice"), "got: {markdown}");
        assert!(markdown.contains("Bob"), "got: {markdown}");
    }
}

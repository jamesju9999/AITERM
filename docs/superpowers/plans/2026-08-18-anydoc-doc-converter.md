# anydoc Document Converter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add [anydoc](https://github.com/firecrawl/anydoc) (a pure-Rust, no-Python document converter) as a second conversion engine alongside MarkItDown, routed by file extension with automatic fallback, exposed to the user as a two-way settings toggle.

**Architecture:** A new Tauri-agnostic `document_convert` module owns the routing decision (`engine_for_extension`), the fallback control flow (`resolve_with_fallback`), and the two engines' actual conversion calls. It is consumed by both the knowledge-base sync path (`RoutedConverter`, replacing `MarkItDownConverter`) and a renamed manual-conversion command (`document_convert`, replacing `markitdown_convert`). A new `DocConvertEngine` config enum (`Auto` / `MarkitdownOnly`) drives the routing decision and is exposed as a settings-page radio group.

**Tech Stack:** Rust (Tauri 2, `anydoc` crate, `async-trait`, `tokio`), React 19 + TypeScript.

**Spec:** `docs/superpowers/specs/2026-08-18-anydoc-doc-converter-design.md`

---

## Known scope decision made during planning (not covered by the spec)

The spec doesn't mention the *proactive* Python-install gate that both entry points currently show before every conversion (`DocConverterView.tsx`'s `pythonEnv.ensureProfile("doc_core")` call, and `KnowledgeBaseView/index.tsx`'s identical call in `handleSync`). Without addressing this, anydoc-covered files would still be blocked on a Python install prompt they don't need — defeating a chunk of the point of this feature.

- **`DocConverterView.tsx` (manual single-file tool):** gate becomes conditional (Task 18) — skipped when the file's extension is anydoc-covered and the engine setting is `Auto`. If MarkItDown ends up needed anyway (fallback) and Python isn't installed, the user sees a plain error message (already rendered nicely with `renderErrorWithLinks`) instead of the guided installer. Acceptable degradation for a rare edge case.
- **`KnowledgeBaseView/index.tsx` (folder sync):** gate is **left unchanged** (Task 21 only fixes a stale comment). A notebook can mix formats, and this app has no per-document failure UI today (verified: no component renders `error_message` or a sync `failed` count) — silently skipping the gate could turn a friendly install prompt into silent partial sync failures with no way for the user to see why. Keeping the existing proactive gate is the safer choice here; it costs users an occasionally-unnecessary Python setup step but never a silent failure. If this bugs you, flag it and it can be revisited as a separate follow-up (it would need a new command to scan a notebook's folder for MarkItDown-only files before deciding).

---

## Task 1: Add the anydoc dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the dependency**

Run:
```bash
cd src-tauri && cargo add anydoc
```

- [ ] **Step 2: Verify it resolves and the crate builds**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -20`
Expected: builds cleanly (new `anydoc = "..."` line now present in `Cargo.toml` under `[dependencies]`).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "build(terminal): add anydoc dependency"
```

---

## Task 2: `document_convert` module skeleton — `Engine` and `engine_for_extension`

**Files:**
- Create: `src-tauri/src/document_convert/mod.rs`
- Modify: `src-tauri/src/lib.rs:16` (module declaration, alphabetical with the existing `pub mod` list)

- [ ] **Step 1: Write the failing test**

Create `src-tauri/src/document_convert/mod.rs`:

```rust
//! Routes document conversion between anydoc (fast, pure Rust, no Python) and
//! MarkItDown (Python sidecar; handles images via vision, audio transcription,
//! `.msg`, html, and plain-text formats that anydoc doesn't touch).

/// Which engine converts a given file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Anydoc,
    MarkItDown,
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
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add to the `pub mod` list (alphabetical order, between `pub mod db;` and `pub mod enterprise;`):

```rust
pub mod document_convert;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -30`
Expected: FAIL — `engine_for_extension` not found.

- [ ] **Step 4: Implement `engine_for_extension`**

Add above the `#[cfg(test)]` block in `src-tauri/src/document_convert/mod.rs`:

```rust
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
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -30`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs src-tauri/src/lib.rs
git commit -m "feat(terminal): add anydoc/MarkItDown extension routing table"
```

---

## Task 3: Format constants (`ANYDOC_EXTENSIONS`, `MARKITDOWN_EXTENSIONS`, `SUPPORTED_EXTENSIONS`)

These are needed as real `&[&str]` slices (not just the predicate from Task 2) for the folder scanner and the native file-picker filter, which can't call an arbitrary function at their call sites' const-context / need a literal list to hand to `rfd`.

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/document_convert/mod.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -30`
Expected: FAIL — `ANYDOC_EXTENSIONS` not found.

- [ ] **Step 3: Add the constants**

Add above `engine_for_extension` in `src-tauri/src/document_convert/mod.rs`:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -30`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs
git commit -m "feat(terminal): add format constants for the document converter"
```

---

## Task 4: `DocConvertEngine` config type

**Files:**
- Modify: `src-tauri/src/config/types.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/config/types.rs` (after `execution_mode_roundtrips_toml`):

```rust
    #[test]
    fn doc_convert_engine_defaults_to_auto() {
        let cfg: AppConfig = toml::from_str("").expect("empty config should parse");
        assert_eq!(cfg.doc_convert_engine, DocConvertEngine::Auto);
    }

    #[test]
    fn doc_convert_engine_roundtrips_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W { e: DocConvertEngine }
        for (engine, expected) in [
            (DocConvertEngine::Auto, "auto"),
            (DocConvertEngine::MarkitdownOnly, "markitdown_only"),
        ] {
            let w = W { e: engine };
            let s = toml::to_string(&w).unwrap();
            assert!(s.contains(expected), "got: {s}");
            let d: W = toml::from_str(&s).unwrap();
            assert_eq!(d.e, w.e);
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib config::types:: 2>&1 | tail -30`
Expected: FAIL — `DocConvertEngine` not found.

- [ ] **Step 3: Add the enum and the `AppConfig` field**

In `src-tauri/src/config/types.rs`, add the enum near `SubmitShortcut` (after its definition, around line 318):

```rust
/// Which engine document conversion prefers. `Auto` routes anydoc-covered
/// formats to anydoc (faster, better quality, no Python) and everything
/// else to MarkItDown; `MarkitdownOnly` disables anydoc entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocConvertEngine {
    #[default]
    Auto,
    MarkitdownOnly,
}
```

Add the field to `AppConfig` (after `submit_shortcut`, around line 45):

```rust
    /// Which engine document conversion prefers (anydoc vs MarkItDown-only).
    #[serde(default)]
    pub doc_convert_engine: DocConvertEngine,
```

Add it to `impl Default for AppConfig` (after `submit_shortcut: SubmitShortcut::default(),`, around line 190):

```rust
            doc_convert_engine: DocConvertEngine::default(),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib config::types:: 2>&1 | tail -30`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(terminal): add doc_convert_engine config field"
```

---

## Task 5: Move `DocumentConverter` trait into `document_convert`

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`
- Modify: `src-tauri/src/knowledge_base/ingest.rs:1-22`

This is a pure relocation — no behavior change. `tests/knowledge_base_ingest.rs` imports `DocumentConverter` via `aiterm_lib::knowledge_base::ingest::DocumentConverter`, so `ingest.rs` re-exports it to keep that import path working unchanged.

- [ ] **Step 1: Add the trait to `document_convert`**

Add to `src-tauri/src/document_convert/mod.rs`, above the `Engine` enum:

```rust
use std::path::Path;
use async_trait::async_trait;

/// Converts one file to Markdown. Implemented by `RoutedConverter`
/// (`commands/knowledge_base.rs`) in production; tests use fakes (see
/// `tests/knowledge_base_ingest.rs`) to avoid depending on Python or anydoc.
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    async fn convert(&self, path: &Path) -> Result<String, String>;
}
```

- [ ] **Step 2: Remove the trait and the extension list from `ingest.rs`, re-export instead**

In `src-tauri/src/knowledge_base/ingest.rs`, remove:

```rust
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
    "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
    "txt", "md", "rst", "xml", "json", "yaml", "yml",
];
```

and:

```rust
/// 將檔案轉成 markdown 的抽象——正式環境由 MarkItDownConverter（Task 8）實作，
/// 測試用 fake 實作避免依賴 Python/MarkItDown。
#[async_trait]
pub trait DocumentConverter: Send + Sync {
    async fn convert(&self, path: &Path) -> Result<String, String>;
}
```

Replace both with, right after the existing `use` block at the top of the file:

```rust
pub use crate::document_convert::{DocumentConverter, SUPPORTED_EXTENSIONS};
```

Remove the now-unused `use async_trait::async_trait;` import from `ingest.rs`'s top `use` block (the trait definition it supported has moved out; `sync_notebook`'s `Arc<dyn DocumentConverter>` doesn't need it).

- [ ] **Step 3: Build and run the existing ingest/KB tests to confirm nothing broke**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -30 && cargo test --test knowledge_base_ingest 2>&1 | tail -40`
Expected: builds cleanly, all existing `knowledge_base_ingest` tests still pass unchanged (they still import `aiterm_lib::knowledge_base::ingest::DocumentConverter`).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs src-tauri/src/knowledge_base/ingest.rs
git commit -m "refactor(terminal): move DocumentConverter trait and SUPPORTED_EXTENSIONS into document_convert"
```

---

## Task 6: `resolve_with_fallback` — the fallback control flow

This is deliberately generic over two `Future`s (not closures — Rust futures are lazy, so passing an already-constructed `async {}` block as an argument doesn't run its body until it's `.await`ed), which makes it fully testable without touching Python or the real anydoc crate.

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src-tauri/src/document_convert/mod.rs`:

```rust
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
```

Add `use crate::config::DocConvertEngine;` to the top of `document_convert/mod.rs`'s main `use` block (not inside `tests`, since `resolve_with_fallback` itself needs it too).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -40`
Expected: FAIL — `resolve_with_fallback` not found.

- [ ] **Step 3: Implement `resolve_with_fallback`**

Add above the `#[cfg(test)]` block:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -40`
Expected: PASS (12 tests total).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs
git commit -m "feat(terminal): add anydoc/MarkItDown fallback control flow"
```

---

## Task 7: `convert_with_anydoc`

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`

anydoc's `to_markdown` is a synchronous, CPU-bound call — run it on tokio's blocking thread pool so a large document doesn't stall the async runtime.

- [ ] **Step 1: Write the smoke test**

Add to the `tests` module (needs `tempfile`, already a dev-dependency):

```rust
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test --lib document_convert::tests::anydoc_converts_a_real_csv_file 2>&1 | tail -20`
Expected: FAIL — `convert_with_anydoc` not found.

- [ ] **Step 3: Implement it**

Add above the `#[cfg(test)]` block:

```rust
/// Runs anydoc's synchronous conversion on the blocking thread pool.
async fn convert_with_anydoc(file_path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || anydoc::to_markdown(&file_path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("anydoc conversion task panicked: {e}"))?
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test --lib document_convert::tests::anydoc_converts_a_real_csv_file 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs
git commit -m "feat(terminal): add the anydoc conversion call"
```

---

## Task 8: Move MarkItDown's python-spawn logic into `document_convert`

This is a pure relocation of `commands/markitdown.rs`'s existing `markitdown_convert` body (script path resolution, vision credential resolution, spawning `python`, streaming stdout/stderr) — renamed to a plain async fn `convert_with_markitdown`, dropping the `#[tauri::command]` attribute. No logic changes.

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`
- Modify: `src-tauri/src/commands/markitdown.rs` (content removed here; the file itself is deleted in Task 11 once the thin command wrapper is rebuilt in `commands/doc_convert.rs`)

- [ ] **Step 1: Copy the moved code into `document_convert/mod.rs`**

Add to `src-tauri/src/document_convert/mod.rs`, above `convert_with_anydoc`:

```rust
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncBufReadExt;
use serde::Deserialize;

use crate::config::ConfigStore;
use crate::secret::SecretStore;

fn converter_script_path(app: &AppHandle) -> std::path::PathBuf {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
        crate::config::ProviderType::Openai => (
            "openai".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
        ),
        crate::config::ProviderType::Anthropic => (
            "anthropic".to_string(),
            cfg.base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        ),
        crate::config::ProviderType::Ollama => (
            "ollama".to_string(),
            cfg.base_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string()),
        ),
        crate::config::ProviderType::OpenaiCompatible => (
            "openai-compatible".to_string(),
            cfg.base_url.unwrap_or_default(),
        ),
        // GitHub Copilot and Google AI have complex OAuth/auth flows — skip for now
        _ => return None,
    };

    Some((provider_type_str, api_key, base_url, cfg.model))
}

/// Convert a local file to Markdown using MarkItDown (Python).
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
    crate::appimage_env::strip_appimage_env(cmd.as_std_mut());

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
```

- [ ] **Step 2: Empty out `commands/markitdown.rs`'s moved parts**

In `src-tauri/src/commands/markitdown.rs`, delete `converter_script_path`, `PythonLine`, `resolve_vision_credentials`, and the `markitdown_convert` command body — everything except `markitdown_pick_file` and its imports (this leftover file is fully removed in Task 11; this step just prevents duplicate-definition build errors in the meantime). Leave the file as:

```rust
// src-tauri/src/commands/markitdown.rs
#[tauri::command]
pub async fn markitdown_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Documents",
            &[
                "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
                "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
                "txt", "md", "rst", "xml", "json", "yaml", "yml",
            ],
        )
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}
```

- [ ] **Step 3: Check that `document_convert/mod.rs` itself compiles**

Run: `cd src-tauri && cargo check --lib 2>&1 | tail -60`
Expected: errors, but only from two known, not-yet-fixed call sites — `lib.rs`'s `markitdown::{markitdown_convert, markitdown_pick_file}` import (`markitdown_convert` no longer exists there; fixed in Task 14) and `commands/knowledge_base.rs`'s `MarkItDownConverter::convert` calling `crate::commands::markitdown::markitdown_convert` (fixed in Task 12). No errors should originate from `document_convert/mod.rs` itself — if there are any, fix them now before moving on.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs src-tauri/src/commands/markitdown.rs
git commit -m "refactor(terminal): move MarkItDown python-spawn logic into document_convert"
```

---

## Task 9: `convert_document` orchestration

**Files:**
- Modify: `src-tauri/src/document_convert/mod.rs`

- [ ] **Step 1: Implement it**

Add to `src-tauri/src/document_convert/mod.rs` (this is the module's public entry point — no dedicated unit test here since its two building blocks, `resolve_with_fallback` and `engine_for_extension`, are already covered; Task 10 explains why no further test is added). `Manager` is already imported by Task 8's code (`use tauri::{AppHandle, Manager};`), so `.state()` is available as a plain method call here too:

```rust
/// Convert one file to Markdown, routing between anydoc and MarkItDown by
/// extension (or forcing MarkItDown when `doc_convert_engine` is set to
/// `MarkitdownOnly`), with automatic fallback in Auto mode. This is the one
/// entry point both the knowledge-base sync path (`RoutedConverter`) and the
/// manual `document_convert` Tauri command call into.
pub async fn convert_document(
    app: AppHandle,
    path: &Path,
    vision_provider_id: Option<String>,
) -> Result<String, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let file_path = path.to_string_lossy().to_string();

    let config = app.state::<Arc<ConfigStore>>();
    let secrets = app.state::<Arc<SecretStore>>();
    let engine_pref = config.get().doc_convert_engine;

    resolve_with_fallback(
        &ext,
        engine_pref,
        convert_with_anydoc(file_path.clone()),
        convert_with_markitdown(app.clone(), file_path, vision_provider_id, config, secrets),
    ).await
}
```

- [ ] **Step 2: Build**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -40`
Expected: builds cleanly (`commands/knowledge_base.rs` still calling the old `markitdown_convert` is fixed in Task 12 — if that's the only remaining error, this step is done; anything inside `document_convert/mod.rs` itself must be clean).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/document_convert/mod.rs
git commit -m "feat(terminal): add convert_document orchestration entry point"
```

---

## Task 10: Confirm `document_convert` module test coverage is complete

**Files:** none changed — verification only.

`convert_document`'s only logic not already covered by a unit test is the `AppHandle::state()` plumbing that reads `doc_convert_engine` and fetches `ConfigStore`/`SecretStore`. This codebase has no existing helper for constructing a mock `AppHandle` with injected state anywhere in `src-tauri/src/` or `src-tauri/tests/` (checked: no hits for `tauri::test::mock_app`, `MockRuntime`, or a `test_app` helper). Building one from scratch is real new test infrastructure, not a bite-sized step, and isn't worth it for one function whose two building blocks are already fully tested:
- `resolve_with_fallback` (Task 6) covers all routing/fallback branches with fakes.
- `convert_with_anydoc` (Task 7) smoke-tests the real anydoc call.
- The `AppHandle::state()` plumbing itself is exercised for real in Task 15's `tauri:dev` manual check and Task 22's end-to-end check.

No new test is added in this task. This step exists to make that decision explicit rather than silently skipping coverage.

- [ ] **Step 1: Run the full document_convert test suite as a checkpoint**

Run: `cd src-tauri && cargo test --lib document_convert:: 2>&1 | tail -40`
Expected: PASS, all tests from Tasks 2, 3, 6, and 7 (12 tests).

No commit for this task (no files changed).

---

## Task 11: Replace `commands/markitdown.rs` with `commands/doc_convert.rs`

**Files:**
- Create: `src-tauri/src/commands/doc_convert.rs`
- Delete: `src-tauri/src/commands/markitdown.rs`
- Modify: `src-tauri/src/commands/mod.rs:15`

- [ ] **Step 1: Create the new thin command file**

Create `src-tauri/src/commands/doc_convert.rs`:

```rust
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
```

- [ ] **Step 2: Delete the old file**

```bash
git rm src-tauri/src/commands/markitdown.rs
```

- [ ] **Step 3: Update `commands/mod.rs`**

In `src-tauri/src/commands/mod.rs`, replace:

```rust
pub mod markitdown;
```

with (keeping alphabetical position — moves from between `mail` and `mcp` to between `db_export` and `design`... actually `doc_convert` sorts between `design` and `enterprise`):

```rust
pub mod doc_convert;
```

Place it alphabetically: remove `pub mod markitdown;` from its old spot (between `mail` and `mcp`) and add `pub mod doc_convert;` between `pub mod design;` and `pub mod enterprise;`.

- [ ] **Step 4: Build (expect remaining errors in `lib.rs` and `commands/knowledge_base.rs` — fixed in Tasks 12 and 14)**

Run: `cd src-tauri && cargo check --lib 2>&1 | tail -60`
Expected: errors only about `markitdown_convert`/`markitdown_pick_file`/`MarkItDownConverter` not found in `lib.rs` and `commands/knowledge_base.rs` — no errors inside `commands/doc_convert.rs` itself.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/doc_convert.rs src-tauri/src/commands/mod.rs
git commit -m "refactor(terminal): replace markitdown_convert command with routed document_convert"
```

---

## Task 12: `RoutedConverter` in `commands/knowledge_base.rs`

**Files:**
- Modify: `src-tauri/src/commands/knowledge_base.rs:59-77`

- [ ] **Step 1: Replace `MarkItDownConverter`**

In `src-tauri/src/commands/knowledge_base.rs`, replace:

```rust
struct MarkItDownConverter {
    app: AppHandle,
    vision_provider_id: Option<String>,
}

#[async_trait]
impl DocumentConverter for MarkItDownConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        let config = self.app.state::<Arc<ConfigStore>>();
        let secrets = self.app.state::<Arc<SecretStore>>();
        crate::commands::markitdown::markitdown_convert(
            self.app.clone(),
            path.to_string_lossy().to_string(),
            self.vision_provider_id.clone(),
            config,
            secrets,
        ).await
    }
}
```

with:

```rust
struct RoutedConverter {
    app: AppHandle,
    vision_provider_id: Option<String>,
}

#[async_trait]
impl DocumentConverter for RoutedConverter {
    async fn convert(&self, path: &Path) -> Result<String, String> {
        crate::document_convert::convert_document(
            self.app.clone(),
            path,
            self.vision_provider_id.clone(),
        ).await
    }
}
```

- [ ] **Step 2: Update the construction site in `kb_sync_notebook`**

In the same file, replace:

```rust
    let converter = MarkItDownConverter {
        app: app.clone(),
        vision_provider_id: Some(provider_id),
    };
```

with:

```rust
    let converter = RoutedConverter {
        app: app.clone(),
        vision_provider_id: Some(provider_id),
    };
```

- [ ] **Step 3: Check**

Run: `cd src-tauri && cargo check --lib 2>&1 | tail -60`
Expected: only `lib.rs`'s `markitdown::{markitdown_convert, markitdown_pick_file}` import remains broken (fixed in Task 14) — no errors from `commands/knowledge_base.rs` itself.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/knowledge_base.rs
git commit -m "refactor(terminal): route KB sync conversions through document_convert"
```

---

## Task 13: `set_doc_convert_engine` config command

**Files:**
- Modify: `src-tauri/src/commands/config.rs`

- [ ] **Step 1: Add the command**

In `src-tauri/src/commands/config.rs`, add after `set_submit_shortcut`:

```rust
#[tauri::command]
pub fn set_doc_convert_engine(
    engine: DocConvertEngine,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.doc_convert_engine = engine; }).map_err(|e| e.to_string())
}
```

Add `DocConvertEngine` to the existing import line:

```rust
use crate::config::{AppConfig, ConfigStore, DefaultTab, DocConvertEngine, ExecutionMode, SubmitShortcut};
```

- [ ] **Step 2: Build**

Run: `cd src-tauri && cargo check --lib 2>&1 | tail -30`
Expected: no new errors from this file.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/config.rs
git commit -m "feat(terminal): add set_doc_convert_engine command"
```

---

## Task 14: Wire everything into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Update the `markitdown` import**

Replace (around line 67):

```rust
    markitdown::{markitdown_convert, markitdown_pick_file},
```

with:

```rust
    doc_convert::{document_convert, document_convert_pick_file},
```

- [ ] **Step 2: Add `set_doc_convert_engine` to the config import**

Replace (around line 41-45):

```rust
    config::{
        get_config, is_appimage_integration_declined, is_claude_notif_declined, is_onboarding_done,
        set_appimage_integration_declined, set_claude_notif_declined,
        set_default_tab, set_execution_mode, set_max_agent_steps, set_onboarding_done, set_submit_shortcut,
    },
```

with:

```rust
    config::{
        get_config, is_appimage_integration_declined, is_claude_notif_declined, is_onboarding_done,
        set_appimage_integration_declined, set_claude_notif_declined,
        set_default_tab, set_doc_convert_engine, set_execution_mode, set_max_agent_steps,
        set_onboarding_done, set_submit_shortcut,
    },
```

- [ ] **Step 3: Update the `generate_handler!` list**

Replace (around line 386, right after `set_submit_shortcut,`):

```rust
            set_submit_shortcut,
```

with:

```rust
            set_submit_shortcut,
            set_doc_convert_engine,
```

Replace (around line 504-505):

```rust
            // MarkItDown
            markitdown_convert,
            markitdown_pick_file,
```

with:

```rust
            // Document conversion (anydoc + MarkItDown)
            document_convert,
            document_convert_pick_file,
```

- [ ] **Step 4: Full build**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -60`
Expected: builds cleanly with no errors.

- [ ] **Step 5: Full test suite**

Run: `cd src-tauri && cargo test --lib 2>&1 | tail -20 && cargo test --test knowledge_base_ingest --test knowledge_base_tools 2>&1 | tail -40`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "refactor(terminal): register the routed document-convert commands"
```

---

## Task 15: Manual verification (Rust side)

- [ ] **Step 1: Full workspace test run**

Run: `cd src-tauri && cargo test 2>&1 | tail -60`
Expected: all pass, no ignored/skipped surprises.

- [ ] **Step 2: Manual smoke test via `tauri:dev`**

Run the app (`npm run tauri:dev` from repo root), open the doc-converter tool, and drag in a real `.docx` or `.pdf` file. Confirm:
- Conversion succeeds without any Python-install prompt appearing (this is the frontend gate change from Task 18 — if Task 18 isn't done yet, skip this check and re-run it after).
- The output looks at least as good as before.

Also try a `.png` or other MarkItDown-only file and confirm the existing Python gate/vision behavior is unchanged.

- [ ] **Step 3: No commit for this task** (verification only).

---

## Task 16: Frontend config types — `DocConvertEngine`

**Files:**
- Modify: `src/ipc/config.ts`

- [ ] **Step 1: Add the type, field, and setter**

In `src/ipc/config.ts`, add after `export type DefaultTab = "terminal" | "database";`:

```typescript
export type DocConvertEngine = "auto" | "markitdown_only";
```

Add to the `AppConfig` interface, after `submit_shortcut: SubmitShortcut;`:

```typescript
  doc_convert_engine: DocConvertEngine;
```

Add after `setSubmitShortcut`:

```typescript
export const setDocConvertEngine = (engine: DocConvertEngine): Promise<void> =>
  invoke("set_doc_convert_engine", { engine });
```

- [ ] **Step 2: Fix the one full `AppConfig` literal in the test suite**

`src/components/Settings/ClaudeBridgePage.test.tsx` builds a `const BASE_CONFIG: AppConfig = {...}` without a type-cast escape hatch (`McpServersPage.test.tsx`'s `{ mcp_enabled: true } as AppConfig` uses `as`, so it isn't checked exhaustively and needs no change). Add the new field to `BASE_CONFIG` (`src/components/Settings/ClaudeBridgePage.test.tsx:23-42`), right after `submit_shortcut: "enter",`:

```typescript
  doc_convert_engine: "auto",
```

- [ ] **Step 3: Type-check**

Run: `npx tsc -b 2>&1 | tail -30`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/ipc/config.ts src/components/Settings/ClaudeBridgePage.test.tsx
git commit -m "feat(terminal): add doc_convert_engine to the config IPC types"
```

---

## Task 17: Frontend IPC — `docConvert.ts` replacing `markitdown.ts`

**Files:**
- Create: `src/ipc/docConvert.ts`
- Delete: `src/ipc/markitdown.ts`

- [ ] **Step 1: Create the new file**

Create `src/ipc/docConvert.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";

/**
 * Convert a local file to Markdown, routed between anydoc and MarkItDown
 * per the user's doc_convert_engine setting. Resolves with the Markdown
 * string, rejects with an error message on failure.
 */
export function documentConvert(filePath: string, providerId?: string): Promise<string> {
  return invoke<string>("document_convert", { filePath, providerId: providerId ?? null });
}

/**
 * Open a native OS file picker filtered to supported document formats.
 * Resolves with the selected file path, or null if the user cancelled.
 */
export function documentConvertPickFile(): Promise<string | null> {
  return invoke<string | null>("document_convert_pick_file");
}
```

- [ ] **Step 2: Delete the old file**

```bash
git rm src/ipc/markitdown.ts
```

(Task 18/19 update the two remaining consumers — `DocConverterView.tsx` and its test — in the same commit-worthy unit; do the delete now and let the next tasks' builds catch any straggler `import` sites.)

- [ ] **Step 3: Check for other consumers**

Run: `grep -rln "ipc/markitdown\|markitdownConvert\|markitdownPickFile" src/ 2>&1`
Expected: only `src/components/DocConverter/DocConverterView.tsx` and `src/components/DocConverter/DocConverterView.test.tsx` (handled in Tasks 18-19).

- [ ] **Step 4: Commit**

```bash
git add src/ipc/docConvert.ts
git commit -m "refactor(terminal): rename markitdown IPC wrapper to docConvert"
```

---

## Task 18: `DocConverterView.tsx` — rename + conditional Python gate

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.tsx`

- [ ] **Step 1: Update imports and add the anydoc extension set**

Replace:

```typescript
import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
```

with:

```typescript
import { documentConvert, documentConvertPickFile } from "../../ipc/docConvert";
import { getConfig } from "../../ipc/config";
import type { DocConvertEngine } from "../../ipc/config";
```

Replace the `AUDIO_EXTENSIONS`/`needsAudioProfile` block with (adds a helper extracting the extension, reused by both checks, plus the new anydoc set and gate predicate):

```typescript
/** Extensions that need the on-demand audio profile (speech-to-text via
 *  markitdown[audio-transcription]). Images are deliberately NOT included:
 *  converter.py bypasses markitdown for images and calls the vision API
 *  itself, falling back to Pillow when there's no vision-capable provider —
 *  and Pillow already ships in doc_core. There is no markitdown[image]
 *  extra (it doesn't exist), so images never need a second profile. */
const AUDIO_EXTENSIONS = new Set(["mp3", "wav", "m4a", "flac"]);

/** Extensions anydoc converts natively — no Python needed at all. Mirrors
 *  ANYDOC_EXTENSIONS in src-tauri/src/document_convert/mod.rs; keep in sync
 *  if that list changes. */
const ANYDOC_EXTENSIONS = new Set([
  "doc", "docx", "docm", "odt", "pdf",
  "ppt", "pps", "pot", "pptx", "pptm", "ppsx", "ppsm",
  "rtf", "epub",
  "xls", "xlsx", "xlsm", "xlsb", "ods", "odp",
  "csv",
]);

function extOf(fileName: string): string {
  // The picker/drag-drop handlers hand back a full path, not a bare file
  // name — strip to the last path segment first so a dot in a directory
  // name (e.g. "/Users/me/v1.2/report") isn't mistaken for an extension.
  const base = fileName.split(/[\\/]/).pop() ?? fileName;
  const dot = base.lastIndexOf(".");
  return dot < 0 ? "" : base.slice(dot + 1).toLowerCase();
}

export function needsAudioProfile(fileName: string): boolean {
  return AUDIO_EXTENSIONS.has(extOf(fileName));
}

/** Whether this file needs the MarkItDown Python profile at all, given the
 *  current engine setting. `markitdown_only` always needs it; under `auto`,
 *  only files anydoc can't convert do. */
export function needsMarkItDownProfile(fileName: string, engine: DocConvertEngine): boolean {
  if (engine === "markitdown_only") return true;
  return !ANYDOC_EXTENSIONS.has(extOf(fileName));
}
```

- [ ] **Step 2: Fetch the engine setting**

Add a new state field near the other `useState` calls:

```typescript
  const [docConvertEngine, setDocConvertEngineState] = useState<DocConvertEngine>("auto");
```

In the existing `useEffect` that fetches providers, also fetch the config (add alongside the existing `listProviders().then(...)` call, not nested inside it):

```typescript
  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
    getConfig().then((cfg) => setDocConvertEngineState(cfg.doc_convert_engine)).catch(console.error);
  }, []);
```

- [ ] **Step 3: Make the gate conditional in `processFilePath`**

Replace:

```typescript
    setGateProfile("doc_core");
    const coreReady = await pythonEnv.ensureProfile("doc_core");
    if (!coreReady) { setExtracting(false); return; }

    if (needsAudioProfile(filePath)) {
      const status = await pythonEnvStatus().catch(() => null);
      const audioInstalled = status?.installed.includes("doc_audio") ?? false;
      if (!audioInstalled) {
        // Not a delete — this one installs something, so it keeps a neutral OK.
        if (!(await confirm(t.python_env_audio_prompt, {
          okLabel: t.common_confirm,
          cancelLabel: t.common_cancel,
        }))) { setExtracting(false); return; }
        setGateProfile("doc_audio");
        const audioReady = await pythonEnv.ensureProfile("doc_audio");
        if (!audioReady) { setExtracting(false); return; }
      }
    }

    try {
      const markdown = await markitdownConvert(filePath, selectedProviderId || undefined);
```

with:

```typescript
    if (needsMarkItDownProfile(filePath, docConvertEngine)) {
      setGateProfile("doc_core");
      const coreReady = await pythonEnv.ensureProfile("doc_core");
      if (!coreReady) { setExtracting(false); return; }

      if (needsAudioProfile(filePath)) {
        const status = await pythonEnvStatus().catch(() => null);
        const audioInstalled = status?.installed.includes("doc_audio") ?? false;
        if (!audioInstalled) {
          // Not a delete — this one installs something, so it keeps a neutral OK.
          if (!(await confirm(t.python_env_audio_prompt, {
            okLabel: t.common_confirm,
            cancelLabel: t.common_cancel,
          }))) { setExtracting(false); return; }
          setGateProfile("doc_audio");
          const audioReady = await pythonEnv.ensureProfile("doc_audio");
          if (!audioReady) { setExtracting(false); return; }
        }
      }
    }

    try {
      const markdown = await documentConvert(filePath, selectedProviderId || undefined);
```

Update the `useCallback` dependency array for `processFilePath` (find the line ending `}, [selectedProviderId, pythonEnv.ensureProfile, t]);`) to include `docConvertEngine`:

```typescript
  }, [selectedProviderId, pythonEnv.ensureProfile, t, docConvertEngine]);
```

- [ ] **Step 4: Update the remaining two `markitdown*` call sites**

Replace:

```typescript
  const handleDropzoneClick = useCallback(async () => {
    const path = await markitdownPickFile();
```

with:

```typescript
  const handleDropzoneClick = useCallback(async () => {
    const path = await documentConvertPickFile();
```

- [ ] **Step 5: Type-check**

Run: `npx tsc -b 2>&1 | tail -40`
Expected: no errors.

- [ ] **Step 6: Commit**

```bash
git add src/components/DocConverter/DocConverterView.tsx
git commit -m "feat(terminal): skip the Python gate for anydoc-covered files in the doc converter"
```

---

## Task 19: `DocConverterView.test.tsx` — update mocks and add gate-skip tests

**Files:**
- Modify: `src/components/DocConverter/DocConverterView.test.tsx`

- [ ] **Step 1: Update the module mocks and imports**

Replace:

```typescript
vi.mock("../../ipc/markitdown", () => ({
  markitdownConvert: vi.fn(),
  markitdownPickFile: vi.fn(),
}));
```

with:

```typescript
vi.mock("../../ipc/docConvert", () => ({
  documentConvert: vi.fn(),
  documentConvertPickFile: vi.fn(),
}));
```

Add a mock for `getConfig` (new — the component now fetches it on mount):

```typescript
const getConfigMock = vi.fn();
vi.mock("../../ipc/config", () => ({
  getConfig: () => getConfigMock(),
}));
```

Replace:

```typescript
import { markitdownConvert, markitdownPickFile } from "../../ipc/markitdown";
```

with:

```typescript
import { documentConvert, documentConvertPickFile } from "../../ipc/docConvert";
```

- [ ] **Step 2: Default `getConfigMock` to `auto` in both `beforeEach` blocks**

In each `beforeEach` that currently sets up `pythonEnvStatusMock.mockResolvedValue(...)`, add right before or after it:

```typescript
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
```

(There are three `describe` blocks with their own `beforeEach` — `DocConverterView`, `DocConverterView audio profile candidate install`, and `DocConverterView pick-interpreter escape hatch`. Add it to all three.)

- [ ] **Step 3: Global find-and-replace of the remaining identifiers**

Replace every remaining `markitdownConvert` → `documentConvert` and `markitdownPickFile` → `documentConvertPickFile` in the file (both in `vi.mocked(...)` calls and `expect(...)` assertions). There are roughly 15 occurrences across the existing test bodies — the fixture data and assertions themselves (file paths, expected markdown, error messages) stay exactly the same, only the imported function names change.

- [ ] **Step 4: Add two new tests for the gate-skip behavior**

Add to the main `describe("DocConverterView", ...)` block:

```typescript
  it("skips the Python gate entirely for an anydoc-covered file under auto engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.docx");
    vi.mocked(documentConvert).mockResolvedValue("# report");
    renderView();
    await act(async () => {}); // let the config-fetch effect resolve

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).not.toHaveBeenCalled();
    expect(documentConvert).toHaveBeenCalledWith("/tmp/report.docx", undefined);
    expect(screen.getByText(/report\.docx/)).toBeInTheDocument();
  });

  it("still runs the Python gate for a MarkItDown-only file (image) under auto engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "auto" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/photo.png");
    vi.mocked(documentConvert).mockResolvedValue("# photo");
    renderView();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/photo.png", undefined);
  });

  it("runs the Python gate for every file, including anydoc-covered ones, under markitdown_only engine", async () => {
    getConfigMock.mockResolvedValue({ doc_convert_engine: "markitdown_only" });
    vi.mocked(documentConvertPickFile).mockResolvedValue("/tmp/report.docx");
    vi.mocked(documentConvert).mockResolvedValue("# report");
    renderView();
    await act(async () => {});

    await act(async () => {
      fireEvent.click(screen.getByText(/拖放或點擊選擇檔案/).closest("div")!);
    });

    expect(pythonEnvEnsure).toHaveBeenCalledWith("doc_core");
    expect(documentConvert).toHaveBeenCalledWith("/tmp/report.docx", undefined);
  });
```

- [ ] **Step 5: Run the tests**

Run: `npm run test -- DocConverterView 2>&1 | tail -80`
Expected: all pass, including the three new tests.

- [ ] **Step 6: Commit**

```bash
git add src/components/DocConverter/DocConverterView.test.tsx
git commit -m "test(terminal): cover the conditional Python gate in the doc converter"
```

---

## Task 20: Settings UI — engine radio group

**Files:**
- Modify: `src/lib/i18n.ts`
- Modify: `src/components/Settings/GeneralPage.tsx`

- [ ] **Step 1: Add zh-TW strings**

In `src/lib/i18n.ts`, insert right after `shortcut_ctrl_enter_desc: "按下 Ctrl+Enter 送出指令，直接按 Enter 換行。",` (line 128) and before `saving_indicator: "儲存中…",` (line 129):

```typescript
    doc_convert_engine: "文件轉換引擎",
    doc_convert_engine_desc: "選擇轉換 Word/PDF/PowerPoint 等文件時使用的引擎。",
    doc_convert_engine_auto_label: "自動（推薦）",
    doc_convert_engine_auto_desc: "優先使用內建的 anydoc 引擎，速度更快、品質更好；anydoc 不支援的格式（圖片、音檔、Outlook 信件、html 等）自動改用 MarkItDown。",
    doc_convert_engine_markitdown_only_label: "只用 MarkItDown",
    doc_convert_engine_markitdown_only_desc: "所有格式都使用 MarkItDown 轉換（舊行為）。",
```

- [ ] **Step 2: Add English strings**

In the same file, insert into the `enRaw` object right after `shortcut_ctrl_enter_desc: "Press Ctrl+Enter to submit; Enter alone for a new line.",` (line 1379) and before `saving_indicator: "Saving…",` (line 1380):

```typescript
    doc_convert_engine: "Document Conversion Engine",
    doc_convert_engine_desc: "Choose which engine converts Word/PDF/PowerPoint and similar documents.",
    doc_convert_engine_auto_label: "Auto (recommended)",
    doc_convert_engine_auto_desc: "Prefers the built-in anydoc engine — faster and higher quality. Formats anydoc can't handle (images, audio, Outlook mail, html, etc.) fall back to MarkItDown automatically.",
    doc_convert_engine_markitdown_only_label: "MarkItDown only",
    doc_convert_engine_markitdown_only_desc: "Convert every format with MarkItDown (previous behavior).",
```

- [ ] **Step 3: Add the settings section in `GeneralPage.tsx`**

In `src/components/Settings/GeneralPage.tsx`, update the imports:

```typescript
import { getConfig, setExecutionMode, setSubmitShortcut, setMaxAgentSteps, setDefaultTab, setDocConvertEngine, appimageIntegrationState, appimageIntegrate, appimageRemoveIntegration, setAppImageIntegrationDeclined } from "../../ipc/config";
import type { ExecutionMode, SubmitShortcut, DefaultTab, DocConvertEngine, AppImageIntegrationState } from "../../ipc/config";
```

Add state, near `shortcut`:

```typescript
  const [docConvertEngine, setDocConvertEngineState] = useState<DocConvertEngine>("auto");
```

Add to the `getConfig().then((cfg) => { ... })` block in the mount `useEffect`:

```typescript
      setDocConvertEngineState(cfg.doc_convert_engine ?? "auto");
```

Add the options array near `SHORTCUT_MODES`:

```typescript
  const DOC_CONVERT_ENGINE_MODES: { value: DocConvertEngine; label: string; desc: string }[] = [
    { value: "auto",             label: t.doc_convert_engine_auto_label,             desc: t.doc_convert_engine_auto_desc },
    { value: "markitdown_only",  label: t.doc_convert_engine_markitdown_only_label,  desc: t.doc_convert_engine_markitdown_only_desc },
  ];
```

Add the handler near `handleShortcutChange`:

```typescript
  const handleDocConvertEngineChange = async (newEngine: DocConvertEngine) => {
    setDocConvertEngineState(newEngine);
    setSaving(true);
    try { await setDocConvertEngine(newEngine); } finally { setSaving(false); }
  };
```

Add the section in the JSX, right after the `submit_shortcut` `<section>` block (after its closing `</section>`, before the `appearance` section):

```tsx
      <section className="settings-section">
        <h3>{t.doc_convert_engine}</h3>
        <p className="section-desc">{t.doc_convert_engine_desc}</p>
        <div className="mode-list">
          {DOC_CONVERT_ENGINE_MODES.map((m) => (
            <label key={m.value} className="mode-option">
              <input
                type="radio"
                name="doc_convert_engine"
                value={m.value}
                checked={docConvertEngine === m.value}
                onChange={() => handleDocConvertEngineChange(m.value)}
                disabled={saving}
              />
              <div className="mode-text">
                <span className="mode-label">{m.label}</span>
                <span className="mode-desc">{m.desc}</span>
              </div>
            </label>
          ))}
        </div>
      </section>
```

- [ ] **Step 4: Type-check**

Run: `npx tsc -b 2>&1 | tail -40`
Expected: no errors.

- [ ] **Step 5: Manual render check**

Run `npm run tauri:dev`, open Settings → General, confirm the new "文件轉換引擎" section renders with two radio options and persists the choice across a reload (check `~/.config`/AppData config TOML for the `doc_convert_engine` key after switching it, or re-open Settings and confirm the selection stuck).

- [ ] **Step 6: Commit**

```bash
git add src/lib/i18n.ts src/components/Settings/GeneralPage.tsx
git commit -m "feat(terminal): add document-conversion engine setting to General settings"
```

---

## Task 21: Fix the stale comment in `KnowledgeBaseView/index.tsx`

**Files:**
- Modify: `src/components/KnowledgeBaseView/index.tsx:216-218`

Per the scope decision at the top of this plan, the gate logic itself is intentionally left unchanged — this task only fixes the comment so it doesn't reference the renamed command.

- [ ] **Step 1: Update the comment**

Replace:

```typescript
    // The import walks the notebook's folder via markitdown_convert, which
    // only guarantees the doc_core profile — see DocConverterView for the
    // same gate on the single-file path.
```

with:

```typescript
    // The import walks the notebook's folder via document_convert, which
    // routes anydoc-covered formats to anydoc but still needs the doc_core
    // profile for anything MarkItDown handles. Unlike DocConverterView, this
    // gate runs unconditionally: a notebook can mix formats, sync failures
    // aren't surfaced per-document in this UI today, and skipping the gate
    // only for notebooks with zero MarkItDown-only files would need a new
    // backend check this plan deliberately doesn't add. See the "Known scope
    // decision" note in docs/superpowers/plans/2026-08-18-anydoc-doc-converter.md.
```

- [ ] **Step 2: Type-check**

Run: `npx tsc -b 2>&1 | tail -20`
Expected: no errors (comment-only change).

- [ ] **Step 3: Commit**

```bash
git add src/components/KnowledgeBaseView/index.tsx
git commit -m "docs(terminal): fix stale markitdown_convert reference in KB sync comment"
```

---

## Task 22: Final full verification

- [ ] **Step 1: Rust — full build and test suite**

Run: `cd src-tauri && cargo build --lib 2>&1 | tail -30 && cargo test 2>&1 | tail -60`
Expected: builds cleanly, all tests pass.

- [ ] **Step 2: Frontend — type check**

Run: `npx tsc -b 2>&1 | tail -40`
Expected: no errors.

- [ ] **Step 3: Frontend — lint**

Run: `npm run lint 2>&1 | tail -60`
Expected: no errors (warnings pre-existing elsewhere are fine; nothing new from files touched in this plan).

- [ ] **Step 4: Frontend — full test suite**

Run: `npm run test 2>&1 | tail -80`
Expected: all pass.

- [ ] **Step 5: Manual end-to-end check**

Run `npm run tauri:dev`:
1. Doc converter tool: convert a `.docx` and a `.pdf` — no Python prompt, fast conversion.
2. Doc converter tool: convert a `.png` — Python gate still appears as before (unless already installed).
3. Settings → General: switch to "只用 MarkItDown", convert the same `.docx` again — Python gate now appears.
4. Switch back to "自動", create/sync a knowledge-base notebook containing a `.docx` — confirm it still syncs successfully (gate still runs per the scope decision, but conversion quality/speed should reflect anydoc once past the gate).

- [ ] **Step 6: No commit for this task** (verification only — if any step surfaces a bug, fix it in a new small commit referencing which task's work it corrects).

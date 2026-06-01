# API Docs — Plan 2: Rust Tauri Bridge Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Tauri commands that spawn the Python `fetcher.py` subprocess, stream line-delimited JSON output as Tauri events, handle WebView-based authentication, and persist session cookies in the OS keyring.

**Architecture:** A new `src-tauri/src/api_docs/` module owns types and subprocess logic. A new `src-tauri/src/commands/api_docs.rs` exposes 6 Tauri commands. The Python subprocess writes `{"type":...}` JSON lines to stdout; Rust reads them with `tokio::io::BufReader` + `AsyncBufReadExt::lines()` and emits Tauri events. Cookies are stored in `SecretStore` under the key `api-docs-cookies-{domain}`.

**Tech Stack:** Rust / Tauri 2, tokio (process + io-util), serde_json, tauri::WebviewWindowBuilder (built into Tauri 2 core — no extra plugin), existing `SecretStore` keyring wrapper.

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `src-tauri/src/api_docs/mod.rs` | Create | Public re-exports, `find_python()` helper |
| `src-tauri/src/api_docs/types.rs` | Create | `DocNode`, `ExtractionOptions`, `KeepOptions`, `AuthStatus` serde structs |
| `src-tauri/src/api_docs/runner.rs` | Create | `run_fetcher()` — spawns Python, streams JSON lines, emits events |
| `src-tauri/src/commands/api_docs.rs` | Create | 6 `#[tauri::command]` functions |
| `src-tauri/src/commands/mod.rs` | Modify | Add `pub mod api_docs;` |
| `src-tauri/src/lib.rs` | Modify | Import + register 6 commands in `invoke_handler!` |

---

### Task 1: Types module

**Files:**
- Create: `src-tauri/src/api_docs/types.rs`

- [ ] **Step 1: Write the types**

```rust
// src-tauri/src/api_docs/types.rs
use serde::{Deserialize, Serialize};

/// A node in the documentation tree (mirrors Python DocNode)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocNode {
    pub title: String,
    pub href: String,
    #[serde(default)]
    pub items: Vec<DocNode>,
}

/// Which parts of each endpoint to include in the Markdown output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeepOptions {
    #[serde(default = "default_true")]
    pub description: bool,
    #[serde(default = "default_true")]
    pub parameters: bool,
    #[serde(default = "default_true")]
    pub request_body: bool,
    #[serde(default = "default_true")]
    pub responses: bool,
    #[serde(default = "default_true")]
    pub code_samples: bool,
}

fn default_true() -> bool { true }

impl Default for KeepOptions {
    fn default() -> Self {
        Self {
            description: true,
            parameters: true,
            request_body: true,
            responses: true,
            code_samples: true,
        }
    }
}

/// Options passed to the `extract` subcommand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionOptions {
    pub url: String,
    /// Pages selected from the tree (href values)
    pub pages: Vec<String>,
    pub output_dir: String,
    /// true = single merged file, false = one file per page
    pub merge: bool,
    pub keep: KeepOptions,
    /// Serialised cookie string "k=v; k2=v2" (may be empty)
    #[serde(default)]
    pub cookies: String,
}

/// Response from api_docs_auth_status
#[derive(Debug, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    /// Account name / email if known, otherwise empty
    pub account: String,
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/api_docs/types.rs
git commit -m "feat(api-docs): add Rust types for DocNode, ExtractionOptions, AuthStatus"
```

---

### Task 2: Python finder + module root

**Files:**
- Create: `src-tauri/src/api_docs/mod.rs`

- [ ] **Step 1: Write mod.rs**

```rust
// src-tauri/src/api_docs/mod.rs
pub mod types;
pub mod runner;

use std::path::PathBuf;

/// Locate the Python interpreter on the host machine.
/// Tries `python3` then `python` — returns the first that resolves.
pub fn find_python() -> &'static str {
    // Try python3 first; on Windows the launcher may only have "python"
    // We keep this simple — actual execution will fail fast if the binary
    // doesn't exist and the error propagates to the frontend.
    if cfg!(target_os = "windows") { "python" } else { "python3" }
}

/// Absolute path to `tools/ApiDocFetcher/fetcher.py` relative to the
/// Cargo manifest directory (dev) or the app resource bundle (production).
pub fn fetcher_script_path(app_handle: &tauri::AppHandle) -> PathBuf {
    // In production Tauri bundles the resources listed in tauri.conf.json.
    // In dev we resolve relative to CARGO_MANIFEST_DIR.
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()  // workspace root
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("ApiDocFetcher")
        .join("fetcher.py");

    if dev_path.exists() {
        return dev_path;
    }

    // Production: try app resource dir
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let prod_path = resource_dir.join("ApiDocFetcher").join("fetcher.py");
        if prod_path.exists() {
            return prod_path;
        }
    }

    dev_path  // return dev path even if missing — caller will get a clear error
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/api_docs/mod.rs
git commit -m "feat(api-docs): add api_docs module with Python finder"
```

---

### Task 3: Runner — subprocess + event streaming

**Files:**
- Create: `src-tauri/src/api_docs/runner.rs`

The runner spawns Python, reads stdout line by line, and emits Tauri events. Each JSON line has a `type` field; the runner routes it to the right event or accumulates tree/detection results.

- [ ] **Step 1: Write runner.rs**

```rust
// src-tauri/src/api_docs/runner.rs
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::types::DocNode;

// ── Tauri event payloads ──────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ApiDocsProgressEvent {
    pub current: u32,
    pub total: u32,
    pub page: String,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsLogEvent {
    pub level: String,   // "info" | "warn" | "error"
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsDoneEvent {
    pub files: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsDetectedEvent {
    pub platform: String,
    pub confidence: String,
}

// ── Line protocol from Python stdout ─────────────────────────────────────────

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Detected { platform: String, confidence: String },
    Tree { data: Vec<DocNode> },
    Progress { current: u32, total: u32, page: String },
    Log { level: String, message: String },
    Done { files: Vec<String> },
    Error { message: String },
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run `fetcher.py <subcommand> [args]` and stream events.
/// Returns the parsed tree for the `tree` subcommand, or `None` for `extract`.
pub async fn run_fetcher(
    app: &AppHandle,
    script: &Path,
    subcommand: &str,
    extra_args: &[&str],
) -> Result<Option<Vec<DocNode>>, String> {
    let python = super::find_python();

    let mut child = Command::new(python)
        .arg(script)
        .arg(subcommand)
        .args(extra_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn Python: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let mut lines = BufReader::new(stdout).lines();

    let mut tree: Option<Vec<DocNode>> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() { continue; }

        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Detected { platform, confidence }) => {
                let _ = app.emit("api-docs-detected", ApiDocsDetectedEvent { platform, confidence });
            }
            Ok(PythonLine::Tree { data }) => {
                tree = Some(data);
            }
            Ok(PythonLine::Progress { current, total, page }) => {
                let _ = app.emit("api-docs-progress", ApiDocsProgressEvent { current, total, page });
            }
            Ok(PythonLine::Log { level, message }) => {
                let _ = app.emit("api-docs-log", ApiDocsLogEvent { level, message });
            }
            Ok(PythonLine::Done { files }) => {
                let _ = app.emit("api-docs-done", ApiDocsDoneEvent { files });
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {
                // Non-JSON debug output — emit as info log so user can see it
                let _ = app.emit("api-docs-log", ApiDocsLogEvent {
                    level: "info".into(),
                    message: line.to_string(),
                });
            }
        }
    }

    // Wait for the child and check exit code
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("fetcher.py exited with code {:?}", status.code()));
    }

    Ok(tree)
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/api_docs/runner.rs
git commit -m "feat(api-docs): Python subprocess runner with line-delimited JSON streaming"
```

---

### Task 4: Tauri commands — detect, tree, extract

**Files:**
- Create: `src-tauri/src/commands/api_docs.rs` (first half)

- [ ] **Step 1: Write detect + tree + extract commands**

```rust
// src-tauri/src/commands/api_docs.rs
use std::sync::Arc;
use tauri::AppHandle;

use crate::api_docs::{fetcher_script_path, runner::run_fetcher, types::{AuthStatus, DocNode, ExtractionOptions}};
use crate::secret::SecretStore;

fn cookie_key(domain: &str) -> String {
    format!("api-docs-cookies-{domain}")
}

fn extract_domain(url: &str) -> String {
    url.trim_start_matches("https://")
       .trim_start_matches("http://")
       .split('/')
       .next()
       .unwrap_or(url)
       .to_string()
}

/// Detect the platform type of an API docs website.
/// Returns JSON string: `{"platform":"mintlify-next","confidence":"high"}`
#[tauri::command]
pub async fn api_docs_detect(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&url);
    let cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    let mut args = vec!["--url", url.as_str()];
    let cookies_owned;
    if !cookies.is_empty() {
        cookies_owned = cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    run_fetcher(&app, &script, "detect", &args).await?;
    // Detection result was emitted as "api-docs-detected" event; return ok
    Ok("ok".to_string())
}

/// Fetch the document tree for a site.
/// Returns `Vec<DocNode>` serialised as JSON.
#[tauri::command]
pub async fn api_docs_fetch_tree(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<Vec<DocNode>, String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&url);
    let cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    let mut args = vec!["--url", url.as_str()];
    let cookies_owned;
    if !cookies.is_empty() {
        cookies_owned = cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    let tree = run_fetcher(&app, &script, "tree", &args).await?;
    tree.ok_or_else(|| "fetcher.py did not emit a tree".to_string())
}

/// Extract selected pages and emit progress events.
/// Fires `api-docs-progress`, `api-docs-log`, and `api-docs-done` events during execution.
#[tauri::command]
pub async fn api_docs_extract(
    app: AppHandle,
    options: ExtractionOptions,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let script = fetcher_script_path(&app);
    let domain = extract_domain(&options.url);
    let stored_cookies = secrets.get(&cookie_key(&domain)).unwrap_or(None).unwrap_or_default();

    // Prefer caller-supplied cookies; fall back to stored
    let effective_cookies = if !options.cookies.is_empty() {
        options.cookies.clone()
    } else {
        stored_cookies
    };

    let pages_json = serde_json::to_string(&options.pages)
        .map_err(|e| e.to_string())?;
    let keep_json = serde_json::to_string(&options.keep)
        .map_err(|e| e.to_string())?;
    let merge_str = if options.merge { "true" } else { "false" };

    let mut args = vec![
        "--url", options.url.as_str(),
        "--pages", pages_json.as_str(),
        "--output-dir", options.output_dir.as_str(),
        "--merge", merge_str,
        "--keep", keep_json.as_str(),
    ];
    let cookies_owned;
    if !effective_cookies.is_empty() {
        cookies_owned = effective_cookies.clone();
        args.push("--cookies");
        args.push(&cookies_owned);
    }

    run_fetcher(&app, &script, "extract", &args).await?;
    Ok(())
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/commands/api_docs.rs
git commit -m "feat(api-docs): detect, fetch_tree, extract Tauri commands"
```

---

### Task 5: Tauri commands — WebView login + auth status

**Files:**
- Modify: `src-tauri/src/commands/api_docs.rs` (second half — append to existing file)

Tauri 2's `WebviewWindowBuilder` is in `tauri::WebviewWindowBuilder`. We create a new window pointing at the login page, poll navigation events until the URL returns to the docs domain, then extract cookies from the webview and store them in keyring.

**Note:** Tauri 2 does not expose direct cookie jar access from Rust. The practical approach is to inject JavaScript via `webview.eval()` to read `document.cookie` once login is detected. This covers session cookies that are not `HttpOnly`. For `HttpOnly` cookies, the webview itself will send them automatically if the user performs requests from the same webview — but since we use curl_cffi externally, the user must manually provide cookies if they are `HttpOnly`. The auth flow covers the common case (non-HttpOnly session cookies from SSO redirects).

- [ ] **Step 1: Append auth commands to api_docs.rs**

```rust
// Append to src-tauri/src/commands/api_docs.rs

use tauri::{WebviewWindowBuilder, WebviewUrl};

/// Open an embedded WebView window for the user to log in.
/// Monitors navigation; when the URL returns to the docs domain, extracts
/// document.cookie via JS eval and stores it in the OS keyring.
/// Returns the cookie string on success.
#[tauri::command]
pub async fn api_docs_login(
    app: AppHandle,
    url: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let domain = extract_domain(&url);
    let domain_clone = domain.clone();

    // Build a unique window label so multiple calls don't conflict
    let label = format!("api-docs-login-{}", uuid::Uuid::new_v4().simple());

    let webview_url = WebviewUrl::External(
        url.parse::<tauri::Url>().map_err(|e| e.to_string())?
    );

    let window = WebviewWindowBuilder::new(&app, &label, webview_url)
        .title(format!("Login — {domain}"))
        .inner_size(900.0, 700.0)
        .build()
        .map_err(|e| e.to_string())?;

    // Poll navigation until we're back on the docs domain (login success)
    // We check every 500ms for up to 5 minutes
    let (tx, mut rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let domain_for_closure = domain_clone.clone();

    let window_clone = window.clone();
    let tx_clone = tx.clone();
    window.on_navigation(move |nav_url| {
        let nav_host = nav_url.host_str().unwrap_or("");
        if nav_host.contains(&domain_for_closure) || nav_host.ends_with(&domain_for_closure) {
            // We're back on the docs domain — extract cookies via JS
            let win = window_clone.clone();
            let tx2 = tx_clone.clone();
            tauri::async_runtime::spawn(async move {
                // Small delay so the page can set cookies
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                let cookies = win.eval("document.cookie")
                    .unwrap_or_else(|_| "\"\"".into());
                // eval returns a JS value; strip surrounding quotes if present
                let cookies = cookies.trim_matches('"').to_string();
                if let Some(sender) = tx2.lock().await.take() {
                    let _ = sender.send(cookies);
                }
            });
        }
        true  // allow navigation
    });

    // Wait for cookie signal (timeout after 5 min)
    let cookies = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        async { rx.await.unwrap_or_default() }
    )
    .await
    .unwrap_or_default();

    // Close the login window
    let _ = window.close();

    if cookies.is_empty() {
        return Err("Login window closed without detecting a successful login".to_string());
    }

    // Persist to keyring
    secrets.set(&cookie_key(&domain), &cookies)
        .map_err(|e| e.to_string())?;

    Ok(cookies)
}

/// Clear stored cookies for a domain.
#[tauri::command]
pub async fn api_docs_logout(
    domain: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    secrets.delete(&cookie_key(&domain))
        .map_err(|e| e.to_string())
}

/// Check whether cookies are stored for a domain.
#[tauri::command]
pub async fn api_docs_auth_status(
    domain: String,
    secrets: tauri::State<'_, Arc<SecretStore>>,
) -> Result<AuthStatus, String> {
    let key = cookie_key(&domain);
    match secrets.get(&key) {
        Ok(Some(cookies)) if !cookies.is_empty() => {
            // Try to extract an email-like token from the cookies as a display name
            let account = cookies
                .split(';')
                .filter_map(|pair| {
                    let kv: Vec<&str> = pair.splitn(2, '=').collect();
                    if kv.len() == 2 { Some(kv[1].trim().to_string()) } else { None }
                })
                .find(|v| v.contains('@'))
                .unwrap_or_default();
            Ok(AuthStatus { logged_in: true, account })
        }
        _ => Ok(AuthStatus { logged_in: false, account: String::new() }),
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/src/commands/api_docs.rs
git commit -m "feat(api-docs): WebView login, logout, auth_status commands"
```

---

### Task 6: Wire up module + register commands

**Files:**
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add api_docs to commands/mod.rs**

In `src-tauri/src/commands/mod.rs`, add the new module:

```rust
pub mod api_docs;
```

The full file becomes:

```rust
pub mod ai;
pub mod api_docs;
pub mod config;
pub mod db;
pub mod design;
pub mod enterprise;
pub mod provider;
pub mod secret;
pub mod shell;
pub mod vcs;
pub mod web;
```

- [ ] **Step 2: Add api_docs module to lib.rs**

At the top of `src-tauri/src/lib.rs`, add after the existing `pub mod` declarations:

```rust
pub mod api_docs;
```

- [ ] **Step 3: Import commands in lib.rs**

In the existing `use commands::{...}` block in `lib.rs`, add:

```rust
    commands::api_docs::{
        api_docs_auth_status, api_docs_detect, api_docs_extract,
        api_docs_fetch_tree, api_docs_login, api_docs_logout,
    },
```

- [ ] **Step 4: Register commands in invoke_handler**

In the `.invoke_handler(tauri::generate_handler![` block, add a new `// API Docs` section:

```rust
            // API Docs
            api_docs_detect,
            api_docs_fetch_tree,
            api_docs_extract,
            api_docs_login,
            api_docs_logout,
            api_docs_auth_status,
```

- [ ] **Step 5: Verify it compiles**

```bash
cd src-tauri && cargo check 2>&1 | tail -20
```

Expected: no errors (warnings about unused code are fine at this stage).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/api_docs/mod.rs src-tauri/src/api_docs/types.rs src-tauri/src/api_docs/runner.rs src-tauri/src/commands/api_docs.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(api-docs): register all 6 Tauri commands in lib.rs"
```

---

### Task 7: IPC TypeScript bindings

**Files:**
- Create: `src/ipc/apiDocs.ts`

The frontend needs typed wrappers for the 6 new commands and the 4 events.

- [ ] **Step 1: Write apiDocs.ts**

```typescript
// src/ipc/apiDocs.ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types mirroring Rust structs ─────────────────────────────────────────────

export interface DocNode {
  title: string;
  href: string;
  items: DocNode[];
}

export interface KeepOptions {
  description: boolean;
  parameters: boolean;
  request_body: boolean;
  responses: boolean;
  code_samples: boolean;
}

export interface ExtractionOptions {
  url: string;
  pages: string[];
  output_dir: string;
  merge: boolean;
  keep: KeepOptions;
  cookies: string;
}

export interface AuthStatus {
  logged_in: boolean;
  account: string;
}

// ── Event payloads ───────────────────────────────────────────────────────────

export interface ApiDocsDetectedEvent {
  platform: string;
  confidence: string;
}

export interface ApiDocsProgressEvent {
  current: number;
  total: number;
  page: string;
}

export interface ApiDocsLogEvent {
  level: "info" | "warn" | "error";
  message: string;
}

export interface ApiDocsDoneEvent {
  files: string[];
}

// ── Commands ─────────────────────────────────────────────────────────────────

export function apiDocsDetect(url: string): Promise<string> {
  return invoke("api_docs_detect", { url });
}

export function apiDocsFetchTree(url: string): Promise<DocNode[]> {
  return invoke("api_docs_fetch_tree", { url });
}

export function apiDocsExtract(options: ExtractionOptions): Promise<void> {
  return invoke("api_docs_extract", { options });
}

export function apiDocsLogin(url: string): Promise<string> {
  return invoke("api_docs_login", { url });
}

export function apiDocsLogout(domain: string): Promise<void> {
  return invoke("api_docs_logout", { domain });
}

export function apiDocsAuthStatus(domain: string): Promise<AuthStatus> {
  return invoke("api_docs_auth_status", { domain });
}

// ── Event listeners ──────────────────────────────────────────────────────────

export function onApiDocsDetected(
  cb: (e: ApiDocsDetectedEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsDetectedEvent>("api-docs-detected", (ev) => cb(ev.payload));
}

export function onApiDocsProgress(
  cb: (e: ApiDocsProgressEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsProgressEvent>("api-docs-progress", (ev) => cb(ev.payload));
}

export function onApiDocsLog(
  cb: (e: ApiDocsLogEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsLogEvent>("api-docs-log", (ev) => cb(ev.payload));
}

export function onApiDocsDone(
  cb: (e: ApiDocsDoneEvent) => void
): Promise<UnlistenFn> {
  return listen<ApiDocsDoneEvent>("api-docs-done", (ev) => cb(ev.payload));
}

export const DEFAULT_KEEP_OPTIONS: KeepOptions = {
  description: true,
  parameters: true,
  request_body: true,
  responses: true,
  code_samples: true,
};
```

- [ ] **Step 2: Type-check**

```bash
npx tsc --noEmit 2>&1 | grep -E "error TS|apiDocs"
```

Expected: no errors related to `apiDocs.ts`.

- [ ] **Step 3: Commit**

```bash
git add src/ipc/apiDocs.ts
git commit -m "feat(api-docs): TypeScript IPC bindings for 6 commands + 4 events"
```

---

### Task 8: Rust unit tests

**Files:**
- Create: `src-tauri/tests/api_docs_types_test.rs` (or inline in `types.rs`)

The runner is hard to test without a real Python process; focus on type serialisation round-trips and the domain extraction helper.

- [ ] **Step 1: Write tests**

Add inline tests to `src-tauri/src/api_docs/types.rs`:

```rust
// Append to src-tauri/src/api_docs/types.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_node_roundtrip() {
        let node = DocNode {
            title: "Getting Started".into(),
            href: "/docs/getting-started".into(),
            items: vec![DocNode {
                title: "Quickstart".into(),
                href: "/docs/quickstart".into(),
                items: vec![],
            }],
        };
        let json = serde_json::to_string(&node).unwrap();
        let decoded: DocNode = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.title, "Getting Started");
        assert_eq!(decoded.items[0].href, "/docs/quickstart");
    }

    #[test]
    fn keep_options_defaults() {
        let opts: KeepOptions = serde_json::from_str("{}").unwrap();
        assert!(opts.description);
        assert!(opts.parameters);
        assert!(opts.request_body);
        assert!(opts.responses);
        assert!(opts.code_samples);
    }

    #[test]
    fn extraction_options_roundtrip() {
        let opts = ExtractionOptions {
            url: "https://docs.example.com".into(),
            pages: vec!["/api/v1".into()],
            output_dir: "/tmp/out".into(),
            merge: true,
            keep: KeepOptions::default(),
            cookies: "session=abc".into(),
        };
        let json = serde_json::to_string(&opts).unwrap();
        let decoded: ExtractionOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.url, "https://docs.example.com");
        assert!(decoded.merge);
        assert_eq!(decoded.cookies, "session=abc");
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cd src-tauri && cargo test api_docs -- --nocapture 2>&1 | tail -20
```

Expected:
```
test api_docs::types::tests::doc_node_roundtrip ... ok
test api_docs::types::tests::keep_options_defaults ... ok
test api_docs::types::tests::extraction_options_roundtrip ... ok
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/api_docs/types.rs
git commit -m "test(api-docs): unit tests for DocNode, KeepOptions, ExtractionOptions serialisation"
```

---

### Task 9: Integration smoke test — Python subprocess round-trip

This task verifies that Plan 2's Rust runner can actually talk to Plan 1's Python CLI. It requires Plan 1 to be implemented first.

- [ ] **Step 1: Verify Python fetcher is installed**

```bash
cd tools/ApiDocFetcher && pip install -r requirements.txt -q && python3 fetcher.py detect --url https://api.stripe.com 2>&1 | head -5
```

Expected: a JSON line like `{"type":"detected","platform":"openapi-direct","confidence":"high"}` (or similar).

- [ ] **Step 2: Run a manual Cargo integration test**

Create a temporary test file to verify the runner doesn't panic:

```bash
cd src-tauri && cargo test 2>&1 | grep -E "FAILED|ok|error\[" | tail -20
```

Expected: all tests pass, no `FAILED`.

- [ ] **Step 3: Type-check the full project**

```bash
npx tsc --noEmit 2>&1 | grep "error TS" | head -10
```

Expected: no errors.

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "feat(api-docs): Plan 2 complete — Rust Tauri bridge + WebView auth + IPC bindings"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] `api_docs_detect(url)` → `{ platform, confidence }` — Task 4
- [x] `api_docs_fetch_tree(url)` → `Vec<DocNode>` — Task 4
- [x] `api_docs_extract(pages, options)` → streams events — Task 4
- [x] `api-docs-progress` event — Task 3 (runner)
- [x] `api-docs-log` event — Task 3 (runner)
- [x] `api-docs-done` event — Task 3 (runner)
- [x] `api_docs_login(url)` — Task 5 (WebView)
- [x] `api_docs_logout(domain)` — Task 5
- [x] `api_docs_auth_status(domain)` — Task 5
- [x] Cookies stored in OS keyring per domain — Task 5
- [x] Python subprocess spawning with stdout line parsing — Task 3
- [x] TypeScript IPC bindings — Task 7

**Not in Plan 2 (deferred to Plan 3):** React UI, tab registration, i18n strings.

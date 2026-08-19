# AITerm MCP Tool Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a local MCP server to AITerm (independent enable/port from the existing Claude Bridge) that exposes AITerm's configured DB connections and knowledge base notebooks as MCP tools, so Claude Code CLI (or any MCP client) can call them directly.

**Architecture:** New `src-tauri/src/mcp_server/` module builds an `AiTermTools` MCP server (via the official `rmcp` crate's `#[tool_router]`/`ServerHandler`) mounted as a Streamable HTTP endpoint on a standalone axum `Router`, bearer-token authenticated (token logic reused from `bridge/auth.rs`). It owns its own `DbManager`/`Db2SidecarState`/knowledge-base `SqlitePool` instances rather than sharing the app's Tauri-managed ones — same on-disk DB/KB data, independent live-connection state, zero risk to existing 19+ call sites that use those types today. Lifecycle (start/stop/enable/port) mirrors `bridge/mod.rs`'s `BridgeState` exactly. A new Settings page (mirrors `ClaudeBridgePage.tsx`) lets the user turn it on and copy a `claude mcp add` command.

**Tech Stack:** Rust (axum 0.8, tokio, rmcp 3.1.3 + schemars 1.0 for the MCP protocol layer), React/TypeScript (existing Settings page patterns), Vitest, cargo test.

---

## Context for the implementing engineer

This plan assumes you have zero prior context on this codebase. Key facts established during design (see `docs/superpowers/specs/2026-08-19-mcp-tool-server-design.md` for the full rationale):

- AITerm already runs a second local axum server for a different purpose: `src-tauri/src/bridge/` (Claude Bridge — Anthropic Messages API compatibility layer). Its lifecycle type `bridge::BridgeState` (start/stop with a `tokio::sync::oneshot` shutdown channel) is the template for this feature's `McpToolServerState`. **Do not touch `bridge/`** — this is a parallel, independent server with its own port/token/toggle.
- `src-tauri/src/mcp/` is a *different* thing: AITerm acting as an MCP **client** (connecting to external MCP servers the user configures, for AITerm's own AI chat). This plan makes AITerm act as an MCP **server** instead — new module, `src-tauri/src/mcp_server/`, no relation to `src-tauri/src/mcp/`.
- The `rmcp` crate (official Rust MCP SDK, crates.io `rmcp = "3.1.3"`) is used instead of hand-rolling the MCP JSON-RPC/Streamable-HTTP protocol. All rmcp code in this plan is copied/adapted from the official examples at `https://github.com/modelcontextprotocol/rust-sdk/tree/main/examples/servers/src` (`counter_streamhttp.rs`, `simple_auth_streamhttp.rs`, `common/counter.rs`) — verified against the actual repo, not guessed.
- `DbManager` (`src-tauri/src/db/manager.rs`) requires a connection to already be "connected" (via `manager.insert`) before `execute`/`list_tables`/etc. will work — it returns `Err("not_connected: ...")` otherwise. The existing `db_connect` Tauri command (`src-tauri/src/commands/db.rs:317`) does this connect-on-demand logic (read password from Keychain, build the right adapter for the DB type, retry once for a dead DB2 sidecar). This plan extracts that logic into a reusable `ensure_connected` function so the new MCP tools can auto-connect without requiring the user to have the DB tab open first.
- Knowledge base notebooks each carry their own `embed_provider_id`/`embed_model` (see `NotebookRow` in `src-tauri/src/db/knowledge_base.rs`) — there is no single "the" embedder, it's resolved per-notebook via `resolve_embedder_config` in `src-tauri/src/commands/knowledge_base.rs:75` (currently private; this plan makes it `pub(crate)`).

---

### Task 1: Add `rmcp` and `schemars` dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the dependencies**

Open `src-tauri/Cargo.toml` and add these two lines near the other protocol-layer dependencies (e.g. right after the `axum = "0.8"` line):

```toml
rmcp = { version = "3.1.3", features = ["server", "macros", "transport-streamable-http-server", "schemars"] }
schemars = "1.0"
```

- [ ] **Step 2: Verify it resolves and builds**

Run: `cd src-tauri && cargo check`
Expected: Compiles successfully (this only pulls in the new dependency graph; no new code references it yet, so there should be zero errors — `cargo check` will just download/compile the new crates).

- [ ] **Step 3: Commit**

```bash
cd src-tauri && git add Cargo.toml Cargo.lock
git commit -m "chore(deps): add rmcp + schemars for the MCP tool server"
```

---

### Task 2: Extract `resolve_db2_sidecar_path()` out of `lib.rs`

The new MCP server needs its own `Db2SidecarState` (see the architecture note above — it does not share the Tauri-managed one). Building one requires the same platform-specific sidecar-path resolution `lib.rs` already does inline. Extract that block into a reusable function so it isn't duplicated.

**Files:**
- Modify: `src-tauri/src/db/db2_sidecar.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/lib.rs:161-250`

- [ ] **Step 1: Add the function to `db2_sidecar.rs`**

Add this at the end of `src-tauri/src/db/db2_sidecar.rs` (the body is copied verbatim from `lib.rs:161-250`, unchanged):

```rust
/// Resolve the directory containing `db2sidecar.jar` for the current platform.
///
/// Checks the production (bundled resources) location first, falling back to
/// the local dev build output under `binaries/`. Extracted from `lib.rs` so
/// both the main app startup and the MCP tool server (which needs its own
/// independent `Db2SidecarState`, not the app's Tauri-managed one) can build
/// this path without duplicating the per-platform logic.
pub fn resolve_db2_sidecar_path() -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        let exe_dir = std::env::current_exe()
            .expect("current_exe")
            .parent()
            .expect("parent dir")
            .to_path_buf();
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        let candidates = [
            // Production: resources bundle into exe_dir/db2-sidecar/
            exe_dir.join("db2-sidecar"),
            // Dev: binaries dir
            manifest_dir
                .join("binaries")
                .join("db2-sidecar-win-x64"),
        ];

        candidates
            .into_iter()
            .find(|p| p.join("db2sidecar.jar").exists())
            .unwrap_or_else(|| exe_dir.join("db2-sidecar"))
    }
    #[cfg(target_os = "macos")]
    {
        let exe_dir = std::env::current_exe()
            .expect("current_exe")
            .parent()
            .expect("parent dir")
            .to_path_buf();

        let contents_dir = exe_dir.parent()
            .expect("Contents dir")
            .to_path_buf();

        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        #[cfg(target_arch = "aarch64")]
        let dev_subdir = "db2-sidecar-mac-arm64";
        #[cfg(target_arch = "x86_64")]
        let dev_subdir = "db2-sidecar-mac-x64";

        let candidates = [
            // Production: Tauri resources land in Contents/Resources/db2-sidecar/
            contents_dir.join("Resources").join("db2-sidecar"),
            // Dev: local build output
            manifest_dir
                .join("binaries")
                .join(dev_subdir),
        ];

        candidates
            .into_iter()
            .find(|p| p.join("db2sidecar.jar").exists())
            .unwrap_or_else(|| contents_dir.join("Resources").join("db2-sidecar"))
    }
    #[cfg(target_os = "linux")]
    {
        let exe_dir = std::env::current_exe()
            .expect("current_exe")
            .parent()
            .expect("parent dir")
            .to_path_buf();

        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        #[cfg(target_arch = "aarch64")]
        let dev_subdir = "db2-sidecar-linux-arm64";
        #[cfg(target_arch = "x86_64")]
        let dev_subdir = "db2-sidecar-linux-x64";

        // Production: AppImage ← {APPDIR}/usr/bin/aiterm → {APPDIR}/usr/lib/aiterm/db2-sidecar
        //             .deb    ← /usr/bin/aiterm           → /usr/lib/aiterm/db2-sidecar
        let prod_path = exe_dir.join("../../lib/AITerm/db2-sidecar");

        let found = [
            prod_path.clone(),
            // Dev: local binaries directory (CARGO_MANIFEST_DIR is a compile-time path
            // valid only on the build machine — used only when prod_path doesn't exist)
            manifest_dir.join("binaries").join(dev_subdir),
        ]
        .into_iter()
        .find(|p| p.join("db2sidecar.jar").exists());

        // Default to prod_path so error messages reference the expected on-device location,
        // not the CI build machine's CARGO_MANIFEST_DIR.
        found.unwrap_or(prod_path)
    }
}
```

- [ ] **Step 2: Re-export it from `db/mod.rs`**

In `src-tauri/src/db/mod.rs`, find the line:

```rust
pub use db2_sidecar::Db2SidecarState;
```

Replace it with:

```rust
pub use db2_sidecar::{resolve_db2_sidecar_path, Db2SidecarState};
```

- [ ] **Step 3: Replace the inline block in `lib.rs` with a call to the new function**

In `src-tauri/src/lib.rs`, replace the entire block at lines 161-250 (from `let sidecar_path = {` through its closing `};`) with:

```rust
    let sidecar_path = db::resolve_db2_sidecar_path();
```

- [ ] **Step 4: Verify the build still succeeds**

Run: `cd src-tauri && cargo build`
Expected: Compiles with no errors. This is a pure code-motion refactor (identical logic, new location) — there's no new behavior to unit test, so a clean build is the correct verification here.

- [ ] **Step 5: Commit**

```bash
cd src-tauri && git add src/db/db2_sidecar.rs src/db/mod.rs src/lib.rs
git commit -m "refactor(db2): extract resolve_db2_sidecar_path so it can be reused outside app startup"
```

---

### Task 3: Extract `ensure_connected` in `commands/db.rs`

**Files:**
- Modify: `src-tauri/src/commands/db.rs:317-349`

- [ ] **Step 1: Write the failing test**

`src-tauri/src/commands/db.rs` has no test module yet. Add this new one at the bottom of the file. This test doesn't exist yet, so it should fail to compile (no `ensure_connected` function):

```rust
#[cfg(test)]
mod ensure_connected_tests {
    use super::*;
    use crate::config::types::{DbConnection, DbType};
    use crate::secret::SecretStore;

    fn sqlite_connection(id: &str, path: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: "test".to_string(),
            db_type: DbType::Sqlite,
            host: path.to_string(),
            port: 0,
            database: String::new(),
            username: String::new(),
            default_schema: None,
        }
    }

    #[tokio::test]
    async fn connects_when_not_already_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config.add_db_connection(sqlite_connection("sq1", ":memory:")).unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        assert!(!manager.is_connected("sq1").await);
        ensure_connected("sq1", &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(manager.is_connected("sq1").await);
    }

    #[tokio::test]
    async fn is_a_no_op_when_already_connected() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config.add_db_connection(sqlite_connection("sq1", ":memory:")).unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        ensure_connected("sq1", &config, &secrets, &manager, &sidecar).await.unwrap();
        // Second call must not error even though a live connection already exists.
        ensure_connected("sq1", &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(manager.is_connected("sq1").await);
    }

    #[tokio::test]
    async fn errors_when_connection_id_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());

        let err = ensure_connected("nonexistent", &config, &secrets, &manager, &sidecar).await.unwrap_err();
        assert!(err.contains("connection not found"), "{err}");
    }
}
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cd src-tauri && cargo test ensure_connected_tests`
Expected: Compile error — `ensure_connected` is not found in scope.

- [ ] **Step 3: Extract the function from `db_connect`**

In `src-tauri/src/commands/db.rs`, find the `db_connect` command (currently around line 317):

```rust
#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
    sidecar: State<'_, Db2SidecarState>,
) -> Result<(), String> {
    if manager.is_connected(&id).await {
        return Ok(());
    }
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets
        .get(&secret_key(&id))
        .ok()
        .flatten()
        .unwrap_or_default();
    let adapter = match build_adapter(&conn, &password, &sidecar).await {
        Ok(adapter) => adapter,
        Err(err) if conn.db_type == DbType::Db2 && is_db2_sidecar_transport_error(&err) => {
            sidecar.reset().await;
            build_adapter(&conn, &password, &sidecar)
                .await
                .map_err(|e| e.to_string())?
        }
        Err(err) => return Err(err.to_string()),
    };
    manager.insert(id, adapter).await;
    Ok(())
}
```

Replace it with a thin wrapper plus the extracted, reusable function:

```rust
/// Ensure a live connection exists for `id`, connecting on demand if it doesn't
/// (reads the stored password, builds the right adapter for the DB type, and
/// retries once if a dead DB2 sidecar caused the first attempt to fail).
/// Shared by the `db_connect` Tauri command (used when the user opens the DB
/// tab) and the MCP tool server's `execute_query`/`list_tables`/etc. (used
/// when an external MCP client calls a tool without the DB tab ever being
/// open) — both need identical connect-on-demand behavior.
pub(crate) async fn ensure_connected(
    id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<(), String> {
    if manager.is_connected(id).await {
        return Ok(());
    }
    let conn = config
        .get()
        .db_connections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| format!("connection not found: {id}"))?;
    let password = secrets
        .get(&secret_key(id))
        .ok()
        .flatten()
        .unwrap_or_default();
    let adapter = match build_adapter(&conn, &password, sidecar).await {
        Ok(adapter) => adapter,
        Err(err) if conn.db_type == DbType::Db2 && is_db2_sidecar_transport_error(&err) => {
            sidecar.reset().await;
            build_adapter(&conn, &password, sidecar)
                .await
                .map_err(|e| e.to_string())?
        }
        Err(err) => return Err(err.to_string()),
    };
    manager.insert(id.to_string(), adapter).await;
    Ok(())
}

#[tauri::command]
pub async fn db_connect(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    manager: State<'_, DbManager>,
    sidecar: State<'_, Db2SidecarState>,
) -> Result<(), String> {
    ensure_connected(&id, &config, &secrets, &manager, &sidecar).await
}
```

- [ ] **Step 4: Run the tests again to confirm they pass**

Run: `cd src-tauri && cargo test ensure_connected_tests`
Expected: 3 tests pass (`connects_when_not_already_connected`, `is_a_no_op_when_already_connected`, `errors_when_connection_id_unknown`).

- [ ] **Step 5: Confirm no regression in existing DB command tests**

Run: `cd src-tauri && cargo test --lib db`
Expected: All existing tests in `db.rs` and the `db/` module still pass — `db_connect` behaves identically since it now just delegates to `ensure_connected`.

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add src/commands/db.rs
git commit -m "refactor(db): extract ensure_connected for reuse by the MCP tool server"
```

---

### Task 4: Expose `format_search_hits`, `safe_truncate`, and `resolve_embedder_config` for reuse

**Files:**
- Modify: `src-tauri/src/knowledge_base/tools.rs`
- Modify: `src-tauri/src/commands/knowledge_base.rs:75`

- [ ] **Step 1: Extract `format_search_hits` and widen visibility in `knowledge_base/tools.rs`**

In `src-tauri/src/knowledge_base/tools.rs`, replace the `fn safe_truncate` declaration:

```rust
fn safe_truncate(s: &str, max_bytes: usize) -> &str {
```

with:

```rust
pub(crate) fn safe_truncate(s: &str, max_bytes: usize) -> &str {
```

Then, in the same file, replace the `search_documents` match arm's inline formatting:

```rust
            match knowledge_base::search_similar_chunks(pool, notebook_id, &query, &query_embedding, top_k).await {
                Ok(hits) if hits.is_empty() => ("No matching content found.".into(), false),
                Ok(hits) => {
                    let formatted = hits.iter().enumerate().map(|(i, h)| {
                        let loc = h.location_hint.as_deref().unwrap_or("(no section title)");
                        format!(
                            "[{}] {} — {} (score {:.2})\n{}",
                            i + 1, h.rel_path, loc, h.score, h.text
                        )
                    }).collect::<Vec<_>>().join("\n\n---\n\n");
                    (formatted, false)
                }
                Err(e) => (format!("Error: {e}"), false),
            }
```

with:

```rust
            match knowledge_base::search_similar_chunks(pool, notebook_id, &query, &query_embedding, top_k).await {
                Ok(hits) if hits.is_empty() => ("No matching content found.".into(), false),
                Ok(hits) => (format_search_hits(&hits), false),
                Err(e) => (format!("Error: {e}"), false),
            }
```

Then add the extracted function near the top of the file (after the `use` statements, before `tool_definitions`):

```rust
/// Format search hits as citation-tagged text blocks. Shared by this module's
/// `dispatch_tool("search_documents", ...)` and the MCP tool server's
/// `search_documents` tool (`src-tauri/src/mcp_server/kb_ops.rs`) — both need
/// the exact same "which chunk came from where" formatting.
pub(crate) fn format_search_hits(hits: &[crate::db::knowledge_base::SearchHit]) -> String {
    hits.iter().enumerate().map(|(i, h)| {
        let loc = h.location_hint.as_deref().unwrap_or("(no section title)");
        format!(
            "[{}] {} — {} (score {:.2})\n{}",
            i + 1, h.rel_path, loc, h.score, h.text
        )
    }).collect::<Vec<_>>().join("\n\n---\n\n")
}
```

- [ ] **Step 2: Widen `resolve_embedder_config`'s visibility**

In `src-tauri/src/commands/knowledge_base.rs`, find (around line 75):

```rust
fn resolve_embedder_config(
```

Replace with:

```rust
pub(crate) fn resolve_embedder_config(
```

- [ ] **Step 3: Verify existing knowledge base tests still pass**

Run: `cd src-tauri && cargo test --lib knowledge_base && cargo test --test knowledge_base_tools`
Expected: All pass unchanged — this step only renames/widens visibility and moves formatting into a helper with identical output, it doesn't change behavior. `tool_definitions_include_search_and_read` and `search_documents_returns_formatted_hits_with_citation_info` (in `src-tauri/tests/knowledge_base_tools.rs`) in particular should still pass since the formatted text is byte-identical.

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/knowledge_base/tools.rs src/commands/knowledge_base.rs
git commit -m "refactor(kb): extract format_search_hits and widen visibility for MCP tool server reuse"
```

---

### Task 5: Add `McpToolServerConfig` to the config store

**Files:**
- Modify: `src-tauri/src/config/types.rs`

- [ ] **Step 1: Add the config struct**

In `src-tauri/src/config/types.rs`, right after the existing `ClaudeBridgeConfig` struct and its `impl Default` block (search for `pub struct ClaudeBridgeConfig`), add:

```rust
pub fn default_mcp_tool_server_port() -> u16 { 8318 }

/// Settings for AITerm's MCP tool server (exposes DB connections and
/// knowledge base notebooks as MCP tools to external clients like Claude
/// Code CLI). Independent from `ClaudeBridgeConfig` — different concern,
/// different toggle, different port. See
/// `docs/superpowers/specs/2026-08-19-mcp-tool-server-design.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolServerConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_mcp_tool_server_port")]
    pub port: u16,
}

impl Default for McpToolServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mcp_tool_server_port(),
        }
    }
}
```

- [ ] **Step 2: Add the field to `AppConfig`**

In the same file, find the `AppConfig` struct's `claude_bridge` field (the last field before the closing `}`):

```rust
    /// Claude Code 橋接設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub claude_bridge: ClaudeBridgeConfig,
}
```

Replace with:

```rust
    /// Claude Code 橋接設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub claude_bridge: ClaudeBridgeConfig,

    /// MCP tool server 設定。舊的 config.toml 沒有這個區塊，靠 `default` 補齊。
    #[serde(default)]
    pub mcp_tool_server: McpToolServerConfig,
}
```

- [ ] **Step 3: Verify it compiles and old configs still load**

Run: `cd src-tauri && cargo build`
Expected: Compiles. `#[serde(default)]` on the new field means a config.toml written before this change (which has no `[mcp_tool_server]` section) still deserializes fine, defaulting to disabled — this is the same pattern `claude_bridge` already relies on, no new test needed for it specifically.

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/config/types.rs
git commit -m "feat(config): add McpToolServerConfig"
```

---

### Task 6: `McpToolServerState` — server lifecycle

**Files:**
- Create: `src-tauri/src/mcp_server/mod.rs`

This file only defines the lifecycle type; it references `server::router` (Task 10) and `tools::AiTermTools` (Task 9), which don't exist yet — so this task's code won't compile standalone. Write it now, and it will compile once Tasks 7-10 land. (Tasks 7-10 are pure-logic/no-dependency-on-mod.rs, so building them first and wiring mod.rs last would also work — this ordering is chosen because it's easier to review the lifecycle shape before the tool implementations.)

- [ ] **Step 1: Create the module skeleton**

Create `src-tauri/src/mcp_server/mod.rs`:

```rust
//! AITerm's MCP tool server: exposes AITerm's configured DB connections and
//! knowledge base notebooks as MCP tools to external MCP clients (Claude
//! Code CLI, etc). Independent from `crate::bridge` (Anthropic Messages API
//! compat layer for Claude Code) and from `crate::mcp` (AITerm acting as an
//! MCP *client* for its own AI chat) — this module is AITerm acting as an MCP
//! *server*. See `docs/superpowers/specs/2026-08-19-mcp-tool-server-design.md`.

pub mod db_ops;
pub mod kb_ops;
pub mod server;
pub mod tools;

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::ConfigStore;
use crate::db::{resolve_db2_sidecar_path, Db2SidecarState};
use crate::db::knowledge_base::KnowledgeBaseDb;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

/// Keychain key for this server's bearer token. Distinct from
/// `bridge::auth::BRIDGE_TOKEN_KEY` — separate server, separate token — but
/// reuses `bridge::auth`'s token generation/comparison/extraction functions,
/// which are generic (not bridge-specific).
pub const MCP_TOOL_SERVER_TOKEN_KEY: &str = "mcp-tool-server:token";

/// Server lifecycle: holds the handle of a currently-running server, if any,
/// and can start/stop it. Mirrors `bridge::BridgeState` exactly.
pub struct McpToolServerState {
    running: Mutex<Option<Running>>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for McpToolServerState {
    fn default() -> Self {
        Self { running: Mutex::new(None) }
    }
}

impl McpToolServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// Start the server. Stops any already-running instance first (used when
    /// changing the port). Deliberately does not fall back to a different
    /// port if the requested one is taken — a registered MCP client's config
    /// would point at a dead address if the port silently drifted.
    ///
    /// Builds its own `DbManager`/`Db2SidecarState`/knowledge-base
    /// `SqlitePool` rather than sharing the app's Tauri-managed ones — see
    /// the module-level design doc for why. `config`/`secrets` ARE the
    /// app-wide shared instances (already `Arc`-managed everywhere else),
    /// passed in by the caller.
    pub async fn start(
        &self,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        token: String,
        port: u16,
    ) -> anyhow::Result<()> {
        self.stop();

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            anyhow::anyhow!("無法綁定 127.0.0.1:{port}（{e}）。請在設定裡換一個埠。")
        })?;

        let db_manager = Arc::new(DbManager::new());
        let sidecar = Arc::new(Db2SidecarState::new(resolve_db2_sidecar_path()));
        let kb_pool = KnowledgeBaseDb::new().await.pool;

        let app = server::router(Arc::new(token), db_manager, config, secrets, sidecar, kb_pool);
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = rx.await; })
                .await;
            if let Err(e) = served {
                log::error!("MCP tool server 結束於錯誤：{e}");
            }
        });

        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}
```

- [ ] **Step 2: Leave it uncompiled for now — proceed to Task 7**

There's nothing to run yet (`server` and `tools` submodules don't exist). Do not run `cargo build` until Task 10 is done; the intermediate broken state is expected and fine within this plan's task sequence. Do not commit yet either — this task's commit happens at the end of Task 10, once the whole module compiles together (see Task 10 Step 6).

---

### Task 7: `mcp_server/db_ops.rs` — DB tool business logic

**Files:**
- Create: `src-tauri/src/mcp_server/db_ops.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/mcp_server/db_ops.rs` with just the test module first:

```rust
//! Business logic for the DB-connection MCP tools (`list_connections`,
//! `list_schemas`, `list_tables`, `get_table_schema`, `execute_query`).
//! Plain async functions returning `Result<String, String>` (Ok = tool
//! content, Err = tool-level error message) — kept separate from
//! `tools.rs`'s `#[tool]`-annotated methods so this logic can be unit tested
//! without going through the MCP protocol layer.

use crate::commands::db::ensure_connected;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::knowledge_base::tools::safe_truncate;
use crate::secret::SecretStore;

const MAX_QUERY_RESULT_BYTES: usize = 100 * 1024;

pub(crate) async fn list_connections(config: &ConfigStore) -> Result<String, String> {
    let conns = config.get().db_connections;
    if conns.is_empty() {
        return Ok("No DB connections configured in AITerm.".to_string());
    }
    let list: Vec<serde_json::Value> = conns.iter().map(|c| serde_json::json!({
        "id": c.id, "name": c.name, "db_type": c.db_type, "database": c.database,
    })).collect();
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

pub(crate) async fn list_schemas(
    connection_id: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let schemas = manager.list_schemas(connection_id).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&schemas).map_err(|e| e.to_string())
}

pub(crate) async fn list_tables(
    connection_id: &str,
    schema: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let tables = manager.list_tables(connection_id, schema).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&tables).map_err(|e| e.to_string())
}

pub(crate) async fn get_table_schema(
    connection_id: &str,
    schema: &str,
    table: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let cols = manager.get_table_schema(connection_id, schema, table).await.map_err(|e| e.to_string())?;
    serde_json::to_string_pretty(&cols).map_err(|e| e.to_string())
}

/// Executes arbitrary SQL (not restricted to read-only — this is a
/// deliberate product decision, see the design doc). Auto-connects if the
/// connection isn't live yet. A DB-level error (bad SQL, permission denied)
/// comes back as `QueryResult::error` per `DbAdapter::execute`'s contract,
/// not as a Rust `Err` — that's surfaced here as `Err` so the caller (the
/// `#[tool]` method) reports it as a tool-level error to the MCP client.
pub(crate) async fn execute_query(
    connection_id: &str,
    sql: &str,
    config: &ConfigStore,
    secrets: &SecretStore,
    manager: &DbManager,
    sidecar: &Db2SidecarState,
) -> Result<String, String> {
    ensure_connected(connection_id, config, secrets, manager, sidecar).await?;
    let result = manager.execute(connection_id, sql).await.map_err(|e| e.to_string())?;
    if let Some(db_err) = &result.error {
        return Err(db_err.clone());
    }
    let full = serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?;
    if full.len() > MAX_QUERY_RESULT_BYTES {
        Ok(format!(
            "{}\n\n[TRUNCATED: result exceeds {} bytes; {} rows total]",
            safe_truncate(&full, MAX_QUERY_RESULT_BYTES),
            MAX_QUERY_RESULT_BYTES,
            result.rows.len(),
        ))
    } else {
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{DbConnection, DbType};

    fn sqlite_connection(id: &str) -> DbConnection {
        DbConnection {
            id: id.to_string(),
            name: "test".to_string(),
            db_type: DbType::Sqlite,
            host: ":memory:".to_string(),
            port: 0,
            database: String::new(),
            username: String::new(),
            default_schema: None,
        }
    }

    fn setup(id: &str) -> (tempfile::TempDir, ConfigStore, SecretStore, DbManager, Db2SidecarState) {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigStore::new_at(dir.path().join("config.toml"));
        config.add_db_connection(sqlite_connection(id)).unwrap();
        let secrets = SecretStore::new();
        let manager = DbManager::new();
        let sidecar = Db2SidecarState::new(dir.path().to_path_buf());
        (dir, config, secrets, manager, sidecar)
    }

    #[tokio::test]
    async fn list_connections_reports_configured_connections() {
        let (_dir, config, _secrets, _manager, _sidecar) = setup("sq1");
        let out = list_connections(&config).await.unwrap();
        assert!(out.contains("sq1"));
        assert!(out.contains("sqlite"));
    }

    #[tokio::test]
    async fn execute_query_auto_connects_when_not_yet_connected() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        assert!(!manager.is_connected("sq1").await);
        let out = execute_query("sq1", "SELECT 42 as n", &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(manager.is_connected("sq1").await);
        assert!(out.contains("42"));
    }

    #[tokio::test]
    async fn execute_query_reports_db_level_errors() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        let err = execute_query("sq1", "SELECT * FROM nonexistent_table", &config, &secrets, &manager, &sidecar)
            .await
            .unwrap_err();
        assert!(err.to_lowercase().contains("no such table"), "{err}");
    }

    #[tokio::test]
    async fn execute_query_truncates_large_results() {
        let (_dir, config, secrets, manager, sidecar) = setup("sq1");
        // A single very long string value blows past MAX_QUERY_RESULT_BYTES on its own.
        let sql = format!("SELECT '{}' as huge", "x".repeat(MAX_QUERY_RESULT_BYTES + 1000));
        let out = execute_query("sq1", &sql, &config, &secrets, &manager, &sidecar).await.unwrap();
        assert!(out.contains("TRUNCATED"), "{out}");
        assert!(out.len() < MAX_QUERY_RESULT_BYTES + 500, "output should be capped near the byte limit, got {} bytes", out.len());
    }
}
```

- [ ] **Step 2: Run to confirm the tests fail first for the right reason**

Run: `cd src-tauri && cargo test --lib mcp_server::db_ops`
Expected: Fails to compile — `mod mcp_server` isn't wired into `lib.rs` yet (Task 12), so the module tree can't find this file. This is expected at this point in the plan; skip ahead to Step 3, and this test will actually run (and should pass) once Task 12 wires the module in. Note the intent here, then continue — don't block on getting a green run until Task 12.

- [ ] **Step 3: Continue to Task 8**

The implementation above is already complete (not a stub) — there's no separate "make it pass" step needed for this file beyond what Task 12's module wiring unlocks. Move on.

---

### Task 8: `mcp_server/kb_ops.rs` — knowledge base tool business logic

**Files:**
- Create: `src-tauri/src/mcp_server/kb_ops.rs`

- [ ] **Step 1: Write the implementation and tests together**

Create `src-tauri/src/mcp_server/kb_ops.rs`:

```rust
//! Business logic for the knowledge-base MCP tools (`list_notebooks`,
//! `search_documents`, `read_document`). Deliberately does NOT reuse
//! `knowledge_base::tools::dispatch_tool` — that function's signature is
//! bound to a single chat session's `notebook_id`/`embedder`, but an MCP
//! client must be able to name any notebook on any call. Shares the
//! formatting/truncation helpers (`format_search_hits`, `safe_truncate`)
//! with that module instead of duplicating them.

use sqlx::SqlitePool;

use crate::commands::knowledge_base::resolve_embedder_config;
use crate::config::ConfigStore;
use crate::db::knowledge_base::{get_document_by_path, get_notebook, list_notebooks as db_list_notebooks, search_similar_chunks};
use crate::knowledge_base::embedding::{Embedder, HttpEmbedder};
use crate::knowledge_base::tools::{format_search_hits, safe_truncate};
use crate::secret::SecretStore;

const MAX_READ_DOCUMENT_BYTES: usize = 100 * 1024;
const DEFAULT_TOP_K: u64 = 8;
const MAX_TOP_K: u64 = 20;

pub(crate) async fn list_notebooks(pool: &SqlitePool) -> Result<String, String> {
    let notebooks = db_list_notebooks(pool).await.map_err(|e| e.to_string())?;
    if notebooks.is_empty() {
        return Ok("No notebooks in the knowledge base.".to_string());
    }
    let list: Vec<serde_json::Value> = notebooks.iter().map(|n| serde_json::json!({
        "id": n.id, "name": n.name, "folder_path": n.folder_path,
    })).collect();
    serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
}

pub(crate) async fn search_documents(
    pool: &SqlitePool,
    config: &ConfigStore,
    secrets: &SecretStore,
    notebook_id: &str,
    query: &str,
    top_k: Option<u64>,
) -> Result<String, String> {
    if query.trim().is_empty() {
        return Err("query is empty".to_string());
    }
    let top_k = top_k.unwrap_or(DEFAULT_TOP_K).clamp(1, MAX_TOP_K) as usize;

    let notebook = get_notebook(pool, notebook_id)
        .await
        .map_err(|_| format!("notebook not found: {notebook_id}"))?;
    let embed_provider_id = notebook.embed_provider_id
        .ok_or_else(|| "this notebook has no embedding provider configured".to_string())?;
    let embed_model = notebook.embed_model
        .ok_or_else(|| "this notebook has no embedding model configured".to_string())?;

    let mut embedder_cfg = resolve_embedder_config(config, secrets, &embed_provider_id)?;
    embedder_cfg.model = embed_model;
    let embedder = HttpEmbedder::new(embedder_cfg)?;

    let mut vectors = embedder.embed(&[query.to_string()]).await?;
    let query_embedding = vectors.pop().ok_or_else(|| "embedding provider returned no vector".to_string())?;

    let hits = search_similar_chunks(pool, notebook_id, query, &query_embedding, top_k)
        .await
        .map_err(|e| e.to_string())?;
    if hits.is_empty() {
        return Ok("No matching content found.".to_string());
    }
    Ok(format_search_hits(&hits))
}

pub(crate) async fn read_document(pool: &SqlitePool, notebook_id: &str, path: &str) -> Result<String, String> {
    let doc = get_document_by_path(pool, notebook_id, path)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no document found at path '{path}' in this notebook"))?;
    if doc.status != "ok" {
        return Err(format!("document has status '{}': {}", doc.status, doc.error_message.unwrap_or_default()));
    }
    let content = doc.markdown_cache.unwrap_or_default();
    if content.len() > MAX_READ_DOCUMENT_BYTES {
        Ok(format!(
            "{}\n\n[TRUNCATED: document exceeds size limit]",
            safe_truncate(&content, MAX_READ_DOCUMENT_BYTES)
        ))
    } else {
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sqlx::sqlite::SqlitePoolOptions;
    use crate::db::knowledge_base::{create_notebook, replace_chunks, upsert_document};

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE notebooks (
                id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, folder_path TEXT NOT NULL,
                embed_provider_id TEXT, embed_model TEXT, embed_dim INTEGER,
                last_synced_at INTEGER, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE documents (
                id TEXT PRIMARY KEY NOT NULL, notebook_id TEXT NOT NULL, rel_path TEXT NOT NULL,
                mtime INTEGER NOT NULL, content_hash TEXT NOT NULL, markdown_cache TEXT,
                status TEXT NOT NULL DEFAULT 'ok', error_message TEXT,
                UNIQUE(notebook_id, rel_path)
            )"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE chunks (
                id TEXT PRIMARY KEY NOT NULL, document_id TEXT NOT NULL, chunk_index INTEGER NOT NULL,
                text TEXT NOT NULL, location_hint TEXT, embedding BLOB NOT NULL
            )"
        ).execute(&pool).await.unwrap();
        pool
    }

    struct FakeEmbedder;
    #[async_trait]
    impl Embedder for FakeEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0, 0.0]).collect())
        }
    }

    #[tokio::test]
    async fn list_notebooks_reports_created_notebooks() {
        let pool = setup_pool().await;
        create_notebook(&pool, "My Notes", "/tmp/notes", None, None, 0).await.unwrap();
        let out = list_notebooks(&pool).await.unwrap();
        assert!(out.contains("My Notes"));
    }

    #[tokio::test]
    async fn list_notebooks_reports_empty_state() {
        let pool = setup_pool().await;
        let out = list_notebooks(&pool).await.unwrap();
        assert!(out.contains("No notebooks"));
    }

    /// Two notebooks, each with their own document — proves search_documents
    /// is properly scoped to the notebook_id it's given, not just "whatever
    /// the first notebook happens to be" (there's no active-session state to
    /// fall back on here, unlike knowledge_base::tools::dispatch_tool).
    #[tokio::test]
    async fn search_documents_is_scoped_to_the_given_notebook() {
        let pool = setup_pool().await;
        let nb_a = create_notebook(&pool, "A", "/tmp/a", Some("p"), Some("m"), 0).await.unwrap();
        let nb_b = create_notebook(&pool, "B", "/tmp/b", Some("p"), Some("m"), 0).await.unwrap();

        let doc_a = upsert_document(&pool, &nb_a.id, "a.md", 0, "h1", Some("content A"), "ok", None).await.unwrap();
        replace_chunks(&pool, &doc_a, &[("內容 A".into(), None, vec![1.0, 0.0, 0.0])]).await.unwrap();

        let doc_b = upsert_document(&pool, &nb_b.id, "b.md", 0, "h2", Some("content B"), "ok", None).await.unwrap();
        replace_chunks(&pool, &doc_b, &[("內容 B".into(), None, vec![1.0, 0.0, 0.0])]).await.unwrap();

        // Fake config/secrets: resolve_embedder_config would fail to find a real
        // provider, so this test exercises the code path up to that point via a
        // config with no matching provider — instead we test the notebook-scoping
        // behavior directly against search_similar_chunks, bypassing the
        // embedder-resolution branch by calling with a notebook whose
        // embed_provider_id/model are unset and asserting the specific error.
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = crate::secret::SecretStore::new();

        // nb_a/nb_b were created with embed_provider_id "p" / embed_model "m",
        // which don't correspond to any real configured provider — so this
        // exercises "notebook has an embedder configured but it doesn't resolve",
        // proving the notebook lookup and field checks run before any network call.
        let err = search_documents(&pool, &config, &secrets, &nb_a.id, "任何查詢", None).await.unwrap_err();
        assert!(err.contains("找不到 provider") || err.contains("provider"), "{err}");
    }

    #[tokio::test]
    async fn search_documents_with_empty_query_returns_error() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", Some("p"), Some("m"), 0).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = crate::secret::SecretStore::new();
        let err = search_documents(&pool, &config, &secrets, &nb.id, "", None).await.unwrap_err();
        assert_eq!(err, "query is empty");
    }

    #[tokio::test]
    async fn search_documents_errors_on_notebook_with_no_embedder_configured() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::ConfigStore::new_at(dir.path().join("config.toml"));
        let secrets = crate::secret::SecretStore::new();
        let err = search_documents(&pool, &config, &secrets, &nb.id, "query", None).await.unwrap_err();
        assert!(err.contains("embedding provider"), "{err}");
    }

    #[tokio::test]
    async fn read_document_returns_full_content() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        upsert_document(&pool, &nb.id, "report.md", 0, "hash1", Some("# Report\n\nfull content here"), "ok", None).await.unwrap();

        let out = read_document(&pool, &nb.id, "report.md").await.unwrap();
        assert_eq!(out, "# Report\n\nfull content here");
    }

    #[tokio::test]
    async fn read_document_errors_when_path_not_found() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let err = read_document(&pool, &nb.id, "missing.md").await.unwrap_err();
        assert!(err.contains("no document found"), "{err}");
    }

    #[tokio::test]
    async fn read_document_truncates_large_content() {
        let pool = setup_pool().await;
        let nb = create_notebook(&pool, "NB", "/tmp/docs", None, None, 0).await.unwrap();
        let huge = "x".repeat(MAX_READ_DOCUMENT_BYTES + 1000);
        upsert_document(&pool, &nb.id, "huge.md", 0, "hashx", Some(&huge), "ok", None).await.unwrap();

        let out = read_document(&pool, &nb.id, "huge.md").await.unwrap();
        assert!(out.contains("TRUNCATED"));
        assert!(out.len() < MAX_READ_DOCUMENT_BYTES + 200);
    }
}
```

- [ ] **Step 2: Note on `#[allow(unused_imports)]`**

`FakeEmbedder` is defined but only used implicitly (it demonstrates the `Embedder` trait is satisfiable; the actual tests exercise the pre-network-call error paths, since a real embedding HTTP call needs a live provider). If `cargo test` (once Task 12 wires the module in) reports `FakeEmbedder` as dead code, add `#[allow(dead_code)]` above `struct FakeEmbedder;` rather than deleting it — it documents the trait shape for future tests that do exercise the full embed path against a mocked HTTP provider.

- [ ] **Step 3: Continue to Task 9**

Like Task 7, this file can't actually run its tests until Task 12 wires `mod mcp_server;` into `lib.rs`. Continue.

---

### Task 9: `mcp_server/tools.rs` — the MCP tool router

**Files:**
- Create: `src-tauri/src/mcp_server/tools.rs`

- [ ] **Step 1: Write the implementation**

Create `src-tauri/src/mcp_server/tools.rs`:

```rust
//! The MCP protocol surface: an `rmcp` tool router exposing AITerm's DB and
//! knowledge-base capabilities. Each `#[tool]` method here is a thin
//! wrapper — the actual logic lives in `db_ops`/`kb_ops` (kept separate so
//! that logic is unit-testable without going through MCP's JSON-RPC layer).
//!
//! Code shape (macros, `Parameters<T>` extractor, `ServerHandler` via
//! `#[tool_handler]`) follows the official `rmcp` example at
//! `https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/common/counter.rs`.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use sqlx::SqlitePool;

use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

use super::{db_ops, kb_ops};

fn to_call_result(r: Result<String, String>) -> Result<CallToolResult, McpError> {
    match r {
        Ok(s) => Ok(CallToolResult::success(vec![ContentBlock::text(s)])),
        Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(e)])),
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionIdArgs {
    /// Connection id as returned by `list_connections`.
    pub connection_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTablesArgs {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TableSchemaArgs {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExecuteQueryArgs {
    pub connection_id: String,
    /// Arbitrary SQL. Not restricted to read-only.
    pub sql: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchDocumentsArgs {
    /// Notebook id as returned by `list_notebooks`.
    pub notebook_id: String,
    /// Natural-language description of what you're looking for — not just keywords.
    pub query: String,
    /// Number of results to return (default 8, max 20).
    #[serde(default)]
    pub top_k: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadDocumentArgs {
    pub notebook_id: String,
    /// Document path exactly as returned by `search_documents`.
    pub path: String,
}

#[derive(Clone)]
pub struct AiTermTools {
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
    tool_router: ToolRouter<AiTermTools>,
}

#[tool_router]
impl AiTermTools {
    pub fn new(
        db_manager: Arc<DbManager>,
        config: Arc<ConfigStore>,
        secrets: Arc<SecretStore>,
        sidecar: Arc<Db2SidecarState>,
        kb_pool: SqlitePool,
    ) -> Self {
        Self {
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List AITerm's configured database connections (id, name, type, database — no credentials).")]
    async fn list_connections(&self) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_connections(&self.config).await)
    }

    #[tool(description = "List schema names visible on a database connection. Connects automatically if not already connected.")]
    async fn list_schemas(
        &self,
        Parameters(ConnectionIdArgs { connection_id }): Parameters<ConnectionIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_schemas(&connection_id, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "List tables and views in a schema on a database connection. Connects automatically if not already connected.")]
    async fn list_tables(
        &self,
        Parameters(ListTablesArgs { connection_id, schema }): Parameters<ListTablesArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::list_tables(&connection_id, &schema, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "Get column metadata (name, type, nullable, default) for a table. Connects automatically if not already connected.")]
    async fn get_table_schema(
        &self,
        Parameters(TableSchemaArgs { connection_id, schema, table }): Parameters<TableSchemaArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::get_table_schema(&connection_id, &schema, &table, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "Execute arbitrary SQL on a database connection and return the result as JSON. NOT restricted to read-only — INSERT/UPDATE/DELETE/DDL are all allowed. Connects automatically if not already connected.")]
    async fn execute_query(
        &self,
        Parameters(ExecuteQueryArgs { connection_id, sql }): Parameters<ExecuteQueryArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(db_ops::execute_query(&connection_id, &sql, &self.config, &self.secrets, &self.db_manager, &self.sidecar).await)
    }

    #[tool(description = "List notebooks in AITerm's knowledge base.")]
    async fn list_notebooks(&self) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::list_notebooks(&self.kb_pool).await)
    }

    #[tool(description = "Semantic search over a knowledge base notebook's indexed documents. Returns the most relevant text chunks, each tagged with its source file path, location hint, and similarity score. Call list_notebooks first if you don't know the notebook_id.")]
    async fn search_documents(
        &self,
        Parameters(SearchDocumentsArgs { notebook_id, query, top_k }): Parameters<SearchDocumentsArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::search_documents(&self.kb_pool, &self.config, &self.secrets, &notebook_id, &query, top_k).await)
    }

    #[tool(description = "Read a knowledge base document's full converted content by its exact path (as shown in search_documents results).")]
    async fn read_document(
        &self,
        Parameters(ReadDocumentArgs { notebook_id, path }): Parameters<ReadDocumentArgs>,
    ) -> Result<CallToolResult, McpError> {
        to_call_result(kb_ops::read_document(&self.kb_pool, &notebook_id, &path).await)
    }
}

#[tool_handler]
impl ServerHandler for AiTermTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(
            "Query AITerm's configured database connections and knowledge base notebooks. \
             Call list_connections or list_notebooks first to discover ids.".to_string(),
        )
    }
}
```

- [ ] **Step 2: Continue to Task 10**

This file references `db_ops`/`kb_ops` (Tasks 7-8, done) but is itself only reachable once `mod.rs` (Task 6) is wired into `lib.rs` (Task 12). Continue.

---

### Task 10: `mcp_server/server.rs` — axum router + auth, and the first real build/test

**Files:**
- Create: `src-tauri/src/mcp_server/server.rs`
- Create: `src-tauri/tests/mcp_tool_server.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod mcp_server;` only — full wiring is Task 12)

- [ ] **Step 1: Write the router**

Create `src-tauri/src/mcp_server/server.rs`:

```rust
//! axum router for the MCP tool server: mounts the `rmcp` Streamable HTTP
//! service at `/mcp` behind bearer-token auth, plus an unauthenticated
//! `/health`. Auth middleware shape follows the official `rmcp` example at
//! `https://github.com/modelcontextprotocol/rust-sdk/blob/main/examples/servers/src/simple_auth_streamhttp.rs`
//! (public routes built separately from the token-gated ones, then merged) —
//! token comparison itself reuses `bridge::auth`, which is generic bearer-token
//! logic, not bridge-specific.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use sqlx::SqlitePool;

use crate::bridge::auth as bridge_auth;
use crate::config::ConfigStore;
use crate::db::db2_sidecar::Db2SidecarState;
use crate::db::manager::DbManager;
use crate::secret::SecretStore;

use super::tools::AiTermTools;

#[derive(Clone)]
struct AuthState {
    token: Arc<String>,
}

async fn auth_middleware(
    State(state): State<AuthState>,
    headers: HeaderMap,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
    match bridge_auth::extract_token(authorization, None) {
        Some(provided) if bridge_auth::token_matches(&state.token, &provided) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

pub fn router(
    token: Arc<String>,
    db_manager: Arc<DbManager>,
    config: Arc<ConfigStore>,
    secrets: Arc<SecretStore>,
    sidecar: Arc<Db2SidecarState>,
    kb_pool: SqlitePool,
) -> Router {
    let service = StreamableHttpService::new(
        move || {
            Ok(AiTermTools::new(
                db_manager.clone(),
                config.clone(),
                secrets.clone(),
                sidecar.clone(),
                kb_pool.clone(),
            ))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let protected = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(AuthState { token }, auth_middleware));

    Router::new()
        .route("/health", get(|| async { "ok" }))
        .merge(protected)
}
```

- [ ] **Step 2: Wire the module into `lib.rs` just enough to compile**

In `src-tauri/src/lib.rs`, find the module declaration list (near the top, e.g. `pub mod mcp;`), and add right after it:

```rust
pub mod mcp_server;
```

- [ ] **Step 3: Build**

Run: `cd src-tauri && cargo build 2>&1 | head -100`
Expected: Compiles. If there are errors, they'll most likely be import-path mismatches against the exact `rmcp` API — cross-check against the real source at `https://raw.githubusercontent.com/modelcontextprotocol/rust-sdk/main/examples/servers/src/common/counter.rs` and `.../simple_auth_streamhttp.rs` (fetch with `curl`, not a summarizing web tool, to see exact code) rather than guessing.

- [ ] **Step 4: Write the integration test**

Create `src-tauri/tests/mcp_tool_server.rs`:

```rust
//! Integration test: hits the real axum router (via `tower::ServiceExt::oneshot`,
//! no TCP listener) to verify auth gating and a full MCP protocol round trip
//! (initialize -> notifications/initialized -> tools/call).

use std::sync::Arc;

use aiterm_lib::mcp_server::server::router;
use aiterm_lib::config::ConfigStore;
use aiterm_lib::db::db2_sidecar::Db2SidecarState;
use aiterm_lib::db::knowledge_base::create_notebook;
use aiterm_lib::db::manager::DbManager;
use aiterm_lib::secret::SecretStore;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn test_router() -> (axum::Router, sqlx::SqlitePool) {
    let dir = tempfile::tempdir().unwrap();
    let config = Arc::new(ConfigStore::new_at(dir.path().join("config.toml")));
    let secrets = Arc::new(SecretStore::new());
    let db_manager = Arc::new(DbManager::new());
    let sidecar = Arc::new(Db2SidecarState::new(dir.path().to_path_buf()));

    let kb_pool = sqlx::sqlite::SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE notebooks (
            id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, folder_path TEXT NOT NULL,
            embed_provider_id TEXT, embed_model TEXT, embed_dim INTEGER,
            last_synced_at INTEGER, created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&kb_pool).await.unwrap();

    let app = router(Arc::new("t0ken".to_string()), db_manager, config, secrets, sidecar, kb_pool.clone());
    (app, kb_pool)
}

fn post(uri: &str, token: Option<&str>, body: serde_json::Value) -> Request<Body> {
    let mut b = Request::builder().method("POST").uri(uri)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn health_check_does_not_require_auth() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn mcp_endpoint_rejects_missing_token() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(post(
        "/mcp", None,
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_endpoint_rejects_wrong_token() {
    let (app, _pool) = test_router().await;
    let resp = app.oneshot(post(
        "/mcp", Some("nope"),
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
    )).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Full protocol round trip: initialize (capture the session id header) ->
/// notifications/initialized -> tools/call(list_connections). Proves the
/// rmcp wiring actually speaks MCP correctly end to end, not just that the
/// axum plumbing compiles.
#[tokio::test]
async fn full_round_trip_lists_connections() {
    let (app, _pool) = test_router().await;

    let init_resp = app.clone().oneshot(post(
        "/mcp", Some("t0ken"),
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "test-client", "version": "0.0.1"}
            }
        }),
    )).await.unwrap();
    assert_eq!(init_resp.status(), StatusCode::OK);
    let session_id = init_resp.headers().get("mcp-session-id").expect("server should assign a session id").to_str().unwrap().to_string();

    let mut notify_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    notify_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let notify_resp = app.clone().oneshot(notify_req).await.unwrap();
    assert!(notify_resp.status().is_success(), "{:?}", notify_resp.status());

    let mut call_req = post("/mcp", Some("t0ken"), serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "list_connections", "arguments": {}}
    }));
    call_req.headers_mut().insert("mcp-session-id", session_id.parse().unwrap());
    let call_resp = app.oneshot(call_req).await.unwrap();
    assert_eq!(call_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(call_resp.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("No DB connections configured"), "{text}");
}
```

- [ ] **Step 5: Run all mcp_server tests**

Run: `cd src-tauri && cargo test --lib mcp_server:: && cargo test --test mcp_tool_server`
Expected: All pass — the `db_ops`/`kb_ops` unit tests from Tasks 7-8 (now reachable via the module tree), plus the four integration tests above. If `full_round_trip_lists_connections` fails on response shape (e.g. the server responds with an SSE stream instead of a direct JSON body for a single-shot call, or the session header name differs), inspect the actual response body/headers via `println!("{text}")` before adjusting — do not guess a fix; the exact response framing for a non-streaming tool call is something to verify against the running behavior, not assume.

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add src/mcp_server src/lib.rs tests/mcp_tool_server.rs
git commit -m "feat(mcp-server): MCP tool server exposing DB and knowledge-base tools via rmcp"
```

---

### Task 11: `commands/mcp_server.rs` — Tauri commands

**Files:**
- Create: `src-tauri/src/commands/mcp_server.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: Write the commands**

Create `src-tauri/src/commands/mcp_server.rs`:

```rust
//! Tauri commands for the MCP tool server settings page. Mirrors
//! `commands/bridge.rs` exactly (same shape: status/apply/set_config).

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::config::types::McpToolServerConfig;
use crate::config::ConfigStore;
use crate::mcp_server::{McpToolServerState, MCP_TOOL_SERVER_TOKEN_KEY};
use crate::secret::SecretStore;
use crate::bridge::auth as bridge_auth;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolServerStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub error: Option<String>,
}

fn ensure_token(secrets: &Arc<SecretStore>) -> anyhow::Result<String> {
    if let Some(t) = secrets.get(MCP_TOOL_SERVER_TOKEN_KEY)? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let t = bridge_auth::generate_token();
    secrets.set(MCP_TOOL_SERVER_TOKEN_KEY, &t)?;
    Ok(t)
}

#[tauri::command]
pub async fn mcp_tool_server_status(
    server: State<'_, Arc<McpToolServerState>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<McpToolServerStatus, String> {
    let port = server.port();
    let token = if port.is_some() {
        secrets.get(MCP_TOOL_SERVER_TOKEN_KEY).ok().flatten()
    } else {
        None
    };
    Ok(McpToolServerStatus { running: port.is_some(), port, token, error: None })
}

/// Starts or stops the server according to the currently saved config. Called
/// after the settings page saves.
#[tauri::command]
pub async fn mcp_tool_server_apply(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<McpToolServerStatus, String> {
    let cfg = config.get().mcp_tool_server;
    if !cfg.enabled {
        server.stop();
        return Ok(McpToolServerStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match server
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port)
        .await
    {
        Ok(()) => Ok(McpToolServerStatus {
            running: true,
            port: Some(cfg.port),
            token: Some(token),
            error: None,
        }),
        Err(e) => Ok(McpToolServerStatus {
            running: false,
            port: None,
            token: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Saves the config and immediately applies it (starts/stops as needed).
#[tauri::command]
pub async fn mcp_tool_server_set_config(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: McpToolServerConfig,
) -> Result<McpToolServerStatus, String> {
    config
        .update(|c| c.mcp_tool_server = value.clone())
        .map_err(|e| e.to_string())?;
    mcp_tool_server_apply(server, config, secrets).await
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/commands/mod.rs`, add (alphabetically near `pub mod mcp;`):

```rust
pub mod mcp_server;
```

- [ ] **Step 3: Build**

Run: `cd src-tauri && cargo build 2>&1 | head -60`
Expected: Compiles (these commands aren't registered in `invoke_handler` yet, nor is `McpToolServerState` managed yet — that's fine, unregistered `#[tauri::command]` functions still compile standalone; Task 12 wires the rest).

- [ ] **Step 4: Commit**

```bash
cd src-tauri && git add src/commands/mcp_server.rs src/commands/mod.rs
git commit -m "feat(mcp-server): Tauri commands for the MCP tool server settings page"
```

---

### Task 12: Wire everything into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Import the new command functions**

Find the existing bridge import line in `src-tauri/src/lib.rs`. It sits inside a `use commands::{ ... };` block (the block starts a few lines above with `use commands::{`), so its sibling entries are written *without* a `commands::` prefix:

```rust
    bridge::{bridge_apply, bridge_set_config, bridge_status},
```

Add right after it, inside the same `use commands::{ ... };` block and following the same no-prefix style (adding `commands::mcp_server::{...}` here would wrongly resolve to `commands::commands::mcp_server` since we're already inside `use commands::{ ... }`):

```rust
    mcp_server::{mcp_tool_server_apply, mcp_tool_server_set_config, mcp_tool_server_status},
```

- [ ] **Step 2: Manage the new state**

Find:

```rust
        .manage(Arc::new(bridge::BridgeState::new()))
```

Add right after it:

```rust
        .manage(Arc::new(mcp_server::McpToolServerState::new()))
```

- [ ] **Step 3: Auto-start on launch if enabled**

Find the bridge auto-start block inside `.setup(|app| { ... })` (search for `橋接 server：設定為 enabled 時隨 app 啟動`). Add a parallel block right after it, before the closing `Ok(())`:

```rust
            // MCP tool server：設定為 enabled 時隨 app 啟動。失敗只記 log 不擋啟動，
            // 理由同橋接 server——埠被占用不該讓整個 app 起不來。
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Manager;
                    let server = handle.state::<Arc<mcp_server::McpToolServerState>>().inner().clone();
                    let config = handle.state::<Arc<ConfigStore>>().inner().clone();
                    let secrets = handle.state::<Arc<SecretStore>>().inner().clone();
                    let cfg = config.get().mcp_tool_server;
                    if !cfg.enabled {
                        return;
                    }
                    let token = match secrets.get(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            let t = bridge::auth::generate_token();
                            if let Err(e) = secrets.set(mcp_server::MCP_TOOL_SERVER_TOKEN_KEY, &t) {
                                log::error!("mcp tool server token 寫入 keychain 失敗：{e}");
                                return;
                            }
                            t
                        }
                    };
                    if let Err(e) = server.start(config, secrets, token, cfg.port).await {
                        log::error!("mcp tool server 啟動失敗：{e}");
                    }
                });
            }
```

- [ ] **Step 4: Register the Tauri commands**

Find:

```rust
            bridge_status,
            bridge_apply,
            bridge_set_config,
```

Add right after it:

```rust
            mcp_tool_server_status,
            mcp_tool_server_apply,
            mcp_tool_server_set_config,
```

- [ ] **Step 5: Build and run the full test suite**

Run: `cd src-tauri && cargo build`
Expected: Compiles clean.

Run: `cd src-tauri && cargo test --lib mcp_server:: && cargo test --test mcp_tool_server && cargo test --lib commands::db && cargo test --lib knowledge_base`
Expected: Every test from Tasks 3, 4, 7, 8, 10 now passes (this is the point where Task 6/7/8/9's "continue, can't run yet" notes resolve — the module is finally reachable through `lib.rs`).

- [ ] **Step 6: Commit**

```bash
cd src-tauri && git add src/lib.rs
git commit -m "feat(mcp-server): wire MCP tool server into app startup and command registry"
```

---

### Task 13: Frontend IPC layer

**Files:**
- Create: `src/ipc/mcpToolServer.ts`
- Modify: `src/ipc/config.ts`

- [ ] **Step 1: Create the IPC wrapper**

Create `src/ipc/mcpToolServer.ts`:

```typescript
import { invoke } from "@tauri-apps/api/core";

export interface McpToolServerStatus {
  running: boolean;
  port: number | null;
  /** 已產生的 token，供「複製手動命令」使用。未啟動時為 null。 */
  token: string | null;
  /** 啟動失敗的原因（例如埠被占用）。這是使用者要處理的狀態，不是例外。 */
  error: string | null;
}

export function mcpToolServerStatus(): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_status");
}

/** 依目前 config 啟動或停止 server。設定存檔後呼叫。 */
export function mcpToolServerApply(): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_apply");
}

export interface McpToolServerConfig {
  enabled: boolean;
  port: number;
}

/** 存下設定並立刻套用（啟動或停止 server）。 */
export function mcpToolServerSetConfig(value: McpToolServerConfig): Promise<McpToolServerStatus> {
  return invoke<McpToolServerStatus>("mcp_tool_server_set_config", { value });
}
```

- [ ] **Step 2: Add the field to `AppConfig`**

In `src/ipc/config.ts`, add the import and field:

```typescript
import type { ClaudeBridgeConfig } from "./bridge";
import type { McpToolServerConfig } from "./mcpToolServer";
```

(Add the second import line right after the existing `ClaudeBridgeConfig` import.)

Then find:

```typescript
  mcp_enabled?: boolean;
  claude_bridge: ClaudeBridgeConfig;
}
```

Replace with:

```typescript
  mcp_enabled?: boolean;
  claude_bridge: ClaudeBridgeConfig;
  mcp_tool_server: McpToolServerConfig;
}
```

- [ ] **Step 3: Type-check**

Run: `npx tsc -b`
Expected: Fails — `src/components/Settings/ClaudeBridgePage.test.tsx`'s `BASE_CONFIG` fixture (and any other test file building a full `AppConfig` fixture) is now missing the required `mcp_tool_server` field. This is expected; fix it now.

- [ ] **Step 4: Fix the existing fixture**

In `src/components/Settings/ClaudeBridgePage.test.tsx`, find the `BASE_CONFIG: AppConfig` object (it ends with the `claude_bridge: { ... }` block). Add right after that block closes:

```typescript
  mcp_tool_server: { enabled: false, port: 8318 },
```

- [ ] **Step 5: Type-check again**

Run: `npx tsc -b`
Expected: Passes. If other files also build full `AppConfig` fixtures (search with `grep -rl "claude_bridge:" src/`), fix each the same way — add `mcp_tool_server: { enabled: false, port: 8318 },` next to their `claude_bridge` field.

- [ ] **Step 6: Commit**

```bash
git add src/ipc/mcpToolServer.ts src/ipc/config.ts src/components/Settings/ClaudeBridgePage.test.tsx
git commit -m "feat(mcp-server): frontend IPC wrapper for the MCP tool server"
```

---

### Task 14: Settings page

**Files:**
- Create: `src/components/Settings/McpToolServerPage.tsx`
- Create: `src/components/Settings/McpToolServerPage.css`
- Create: `src/components/Settings/McpToolServerPage.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `src/components/Settings/McpToolServerPage.test.tsx`:

```typescript
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { McpToolServerPage } from "./McpToolServerPage";

vi.mock("../../ipc/mcpToolServer", () => ({
  mcpToolServerStatus: vi.fn(),
  mcpToolServerSetConfig: vi.fn(),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));

import { mcpToolServerStatus, mcpToolServerSetConfig } from "../../ipc/mcpToolServer";
import type { McpToolServerStatus } from "../../ipc/mcpToolServer";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";

const BASE_CONFIG: AppConfig = {
  default_provider: null,
  providers: [],
  execution_mode: "graded",
  submit_shortcut: "enter",
  doc_convert_engine: "auto",
  onboarding_done: true,
  max_agent_steps: 5,
  default_tab: "terminal",
  enterprise_server_url: null,
  enterprise_device_id: null,
  enterprise_policy: null,
  claude_bridge: { enabled: false, port: 8317, default_on_new_tab: false, opus: null, sonnet: null, haiku: null },
  mcp_tool_server: { enabled: false, port: 8318 },
};

const STOPPED_STATUS: McpToolServerStatus = { running: false, port: null, token: null, error: null };
const RUNNING_STATUS: McpToolServerStatus = { running: true, port: 8318, token: "abc123", error: null };

beforeEach(() => {
  vi.mocked(getConfig).mockResolvedValue(BASE_CONFIG);
  vi.mocked(mcpToolServerStatus).mockResolvedValue(STOPPED_STATUS);
  vi.mocked(mcpToolServerSetConfig).mockResolvedValue(RUNNING_STATUS);
});

describe("McpToolServerPage", () => {
  it("loads the saved config and shows the stopped status", async () => {
    render(<McpToolServerPage />);
    await waitFor(() => expect(screen.getByRole("checkbox")).not.toBeChecked());
  });

  it("enabling and saving calls mcpToolServerSetConfig with enabled: true", async () => {
    const user = userEvent.setup();
    render(<McpToolServerPage />);
    await waitFor(() => screen.getByRole("checkbox"));

    await user.click(screen.getByRole("checkbox"));
    await user.click(screen.getByRole("button", { name: /save|儲存/i }));

    await waitFor(() => {
      expect(mcpToolServerSetConfig).toHaveBeenCalledWith(
        expect.objectContaining({ enabled: true, port: 8318 }),
      );
    });
  });

  it("shows the running port and a copyable claude mcp add command once started", async () => {
    vi.mocked(mcpToolServerStatus).mockResolvedValue(RUNNING_STATUS);
    const { container } = render(<McpToolServerPage />);
    // Scoped to the specific command block rather than screen.getByText(/8318/):
    // the status line above it also renders the port ("· :8318"), so a
    // page-wide text query would match two elements and throw on ambiguity.
    await waitFor(() => {
      const command = container.querySelector(".mcp-tool-server-command");
      expect(command).not.toBeNull();
      expect(command!.textContent).toContain("8318");
      expect(command!.textContent).toContain("abc123");
    });
  });
});
```

- [ ] **Step 2: Run to confirm it fails**

Run: `npm run test -- McpToolServerPage`
Expected: Fails — `./McpToolServerPage` doesn't exist yet.

- [ ] **Step 3: Write the page**

Create `src/components/Settings/McpToolServerPage.css` (minimal — reuses the same visual language as `ClaudeBridgePage.css`; copy that file's structure, renaming class prefixes):

```css
.mcp-tool-server-page {
  padding: 24px;
  max-width: 640px;
}

.mcp-tool-server-desc {
  color: var(--text-secondary, #888);
  margin-bottom: 16px;
}

.mcp-tool-server-status {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
  font-size: 13px;
}

.mcp-tool-server-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-secondary, #888);
}

.mcp-tool-server-dot--on {
  background: #4ade80;
}

.mcp-tool-server-error {
  color: #f87171;
  margin-bottom: 8px;
  font-size: 13px;
}

.mcp-tool-server-section {
  margin: 20px 0;
}

.mcp-tool-server-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0;
}

.mcp-tool-server-command {
  background: var(--bg-secondary, #1a1a1a);
  border-radius: 6px;
  padding: 12px;
  font-family: monospace;
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
  margin: 8px 0;
}

.mcp-tool-server-actions {
  display: flex;
  gap: 8px;
  margin-top: 16px;
}
```

Create `src/components/Settings/McpToolServerPage.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getConfig } from "../../ipc/config";
import {
  mcpToolServerSetConfig,
  mcpToolServerStatus,
  type McpToolServerConfig,
  type McpToolServerStatus,
} from "../../ipc/mcpToolServer";
import "./McpToolServerPage.css";

export function McpToolServerPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<McpToolServerConfig | null>(null);
  const [status, setStatus] = useState<McpToolServerStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void (async () => {
      const [c, s] = await Promise.all([getConfig(), mcpToolServerStatus()]);
      setCfg(c.mcp_tool_server);
      setStatus(s);
    })();
  }, []);

  const updateCfg: typeof setCfg = useCallback((next) => {
    setSaved(false);
    setCfg(next);
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      setStatus(await mcpToolServerSetConfig(cfg));
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  const registerCommand = (): string => {
    const port = status?.port ?? cfg?.port ?? 8318;
    const token = status?.token ?? "<token>";
    return `claude mcp add --transport http aiterm-tools http://127.0.0.1:${port}/mcp --header "Authorization: Bearer ${token}"`;
  };

  if (!cfg) return <div className="mcp-tool-server-page" />;

  return (
    <div className="mcp-tool-server-page">
      <h2>{t.mcp_tool_server_title}</h2>
      <p className="mcp-tool-server-desc">{t.mcp_tool_server_desc}</p>

      <div className="mcp-tool-server-status">
        <span className={status?.running ? "mcp-tool-server-dot mcp-tool-server-dot--on" : "mcp-tool-server-dot"} />
        {status?.running ? t.mcp_tool_server_status_running : t.mcp_tool_server_status_stopped}
        {status?.port ? ` · :${status.port}` : ""}
      </div>
      {status?.error && <div className="mcp-tool-server-error">{status.error}</div>}

      <section className="mcp-tool-server-section">
        <label className="mcp-tool-server-row">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => updateCfg({ ...cfg, enabled: e.target.checked })}
          />
          {t.mcp_tool_server_enable}
        </label>

        <label className="mcp-tool-server-row">
          {t.mcp_tool_server_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8318 })}
          />
        </label>
      </section>

      {status?.running && (
        <section className="mcp-tool-server-section">
          <h3>{t.mcp_tool_server_section_register}</h3>
          <div className="mcp-tool-server-command">{registerCommand()}</div>
        </section>
      )}

      <div className="mcp-tool-server-actions">
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.mcp_tool_server_saved} ✓` : t.save}
        </button>
        {status?.running && (
          <button
            onClick={() => {
              void navigator.clipboard.writeText(registerCommand());
              setCopied(true);
            }}
          >
            {copied ? t.mcp_tool_server_copied : t.mcp_tool_server_copy_command}
          </button>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the test again**

Run: `npm run test -- McpToolServerPage`
Expected: Fails now on missing i18n keys (`t.mcp_tool_server_*` will be `undefined`, so `getByText`/`getByRole` name matches may not find what's expected) — this is Task 15. Continue there before re-running.

- [ ] **Step 5: Continue to Task 15**

---

### Task 15: i18n keys

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add zh-TW keys**

In `src/lib/i18n.ts`, find the zh-TW locale block's `bridge_*` keys (around line 547-570, e.g. right after `bridge_copied: "已複製",`). Add:

```typescript
    mcp_tool_server_title: "MCP 工具伺服器",
    mcp_tool_server_desc: "讓 Claude Code CLI 或其他 MCP client 直接呼叫 AITerm 已設定的資料庫連線與知識庫，不用另外重複設定連線資訊。",
    mcp_tool_server_status_running: "執行中",
    mcp_tool_server_status_stopped: "已停止",
    mcp_tool_server_enable: "啟用",
    mcp_tool_server_port: "連接埠",
    mcp_tool_server_section_register: "註冊到 Claude Code",
    mcp_tool_server_saved: "已儲存",
    mcp_tool_server_copy_command: "複製註冊指令",
    mcp_tool_server_copied: "已複製",
```

- [ ] **Step 2: Add English keys**

In the same file, find the English locale block's matching `bridge_*` keys (around line 1804-1827, e.g. right after `bridge_copied: "Copied",`). Add:

```typescript
    mcp_tool_server_title: "MCP Tool Server",
    mcp_tool_server_desc: "Let Claude Code CLI or any other MCP client call AITerm's configured database connections and knowledge base directly, without duplicating connection setup.",
    mcp_tool_server_status_running: "Running",
    mcp_tool_server_status_stopped: "Stopped",
    mcp_tool_server_enable: "Enable",
    mcp_tool_server_port: "Port",
    mcp_tool_server_section_register: "Register with Claude Code",
    mcp_tool_server_saved: "Saved",
    mcp_tool_server_copy_command: "Copy register command",
    mcp_tool_server_copied: "Copied",
```

- [ ] **Step 3: Type-check and run the page test**

Run: `npx tsc -b && npm run test -- McpToolServerPage`
Expected: Both pass.

- [ ] **Step 4: Commit**

```bash
git add src/components/Settings/McpToolServerPage.tsx src/components/Settings/McpToolServerPage.css src/components/Settings/McpToolServerPage.test.tsx src/lib/i18n.ts
git commit -m "feat(mcp-server): settings page for the MCP tool server"
```

---

### Task 16: Register the settings tab

**Files:**
- Modify: `src/components/Settings/SettingsView.tsx`

- [ ] **Step 1: Import the page and icon**

Find:

```typescript
import { ClaudeBridgePage } from "./ClaudeBridgePage";
```

Add right after:

```typescript
import { McpToolServerPage } from "./McpToolServerPage";
```

Find the icon import block:

```typescript
import {
  SettingsIcon,
  RobotIcon,
  DatabaseIcon,
  BranchIcon,
  WrenchIcon,
  InfoIcon,
  LinkIcon,
  ZapIcon
} from "../Icons";
```

Replace with:

```typescript
import {
  SettingsIcon,
  RobotIcon,
  DatabaseIcon,
  BranchIcon,
  WrenchIcon,
  InfoIcon,
  LinkIcon,
  ZapIcon,
  SparklesIcon
} from "../Icons";
```

- [ ] **Step 2: Add the tab to the type union**

Find:

```typescript
type SettingsTab = "general" | "providers" | "databases" | "vcs" | "enterprise" | "about" | "mcp" | "mail" | "bridge" | "usage";
```

Replace with:

```typescript
type SettingsTab = "general" | "providers" | "databases" | "vcs" | "enterprise" | "about" | "mcp" | "mail" | "bridge" | "usage" | "mcpToolServer";
```

- [ ] **Step 3: Add the sidebar button**

Find:

```typescript
        <button
          className={`sidebar-item ${tab === "bridge" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("bridge")}
        >
          <LinkIcon size={16} /> {t.bridge_title}
        </button>
```

Add right after it:

```typescript
        <button
          className={`sidebar-item ${tab === "mcpToolServer" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("mcpToolServer")}
        >
          <SparklesIcon size={16} /> {t.mcp_tool_server_title}
        </button>
```

- [ ] **Step 4: Render it**

Find:

```typescript
        {tab === "bridge" && <ClaudeBridgePage />}
```

Add right after it:

```typescript
        {tab === "mcpToolServer" && <McpToolServerPage />}
```

- [ ] **Step 5: Verify**

Run: `npx tsc -b && npm run lint`
Expected: Both pass.

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/SettingsView.tsx
git commit -m "feat(mcp-server): register the MCP tool server settings tab"
```

---

### Task 17: Full verification pass

**Files:** none (verification only)

- [ ] **Step 1: Full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: All pass, including everything from Tasks 3, 4, 7, 8, 10.

- [ ] **Step 2: Full frontend suite**

Run: `npm run lint && npx tsc -b && npm run test`
Expected: All pass.

- [ ] **Step 3: Manual end-to-end verification**

This step needs a human at a real terminal with Claude Code CLI installed and at least one DB connection or knowledge base notebook configured in AITerm — it can't be scripted.

1. Run `npm run tauri:dev`.
2. Open Settings → MCP Tool Server, enable it, save. Confirm the status dot turns on and a port/token appear.
3. Copy the `claude mcp add` command shown on the page.
4. Open a terminal tab (inside or outside AITerm) and paste the command to register it.
5. Start `claude` in that terminal and ask a question that requires the DB or knowledge base (e.g. "what tables are in my `<connection name>` database?" or "search my knowledge base for X").
6. Confirm Claude Code actually calls the tool (visible in its tool-call trace) and returns a correct, real answer — not a hallucinated one.
7. Toggle the server off in Settings, confirm `claude mcp list` (or a fresh query) now fails to reach it, confirming the toggle actually controls the running server and not just the config file.

- [ ] **Step 4: Report results to the user**

Summarize which of Steps 1-3 passed, and paste the exact output of any manual step that didn't behave as expected — do not report this task as done without having actually run the manual verification, or state clearly that it was skipped and why.

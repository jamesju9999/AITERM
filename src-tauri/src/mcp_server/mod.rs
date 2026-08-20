//! AITerm's MCP tool server: exposes AITerm's configured DB connections and
//! knowledge base notebooks as MCP tools to external MCP clients (Claude
//! Code CLI, etc). Independent from `crate::bridge` (Anthropic Messages API
//! compat layer for Claude Code) and from `crate::mcp` (AITerm acting as an
//! MCP *client* for its own AI chat) — this module is AITerm acting as an MCP
//! *server*. See `docs/superpowers/specs/2026-08-19-mcp-tool-server-design.md`.

pub mod coordination_ops;
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
    /// `SqlitePool` rather than sharing the app's Tauri-managed ones, to
    /// avoid refactoring ~20 existing call sites that currently hold those
    /// types un-`Arc`'d in Tauri state — see the module-level design doc for
    /// the full tradeoff analysis. `config`/`secrets` ARE the app-wide shared
    /// instances (already `Arc`-managed everywhere else), passed in by the
    /// caller.
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

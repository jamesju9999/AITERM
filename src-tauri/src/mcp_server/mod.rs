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
use crate::pty::PtyManager;
use crate::secret::SecretStore;

use coordination_ops::CoordinationRegistry;

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
    task: tokio::task::JoinHandle<()>,
}

/// 舊伺服器收到關閉訊號後，只要還有連線沒斷（Claude Code 等客戶端的 keep-alive），
/// axum 的 graceful shutdown 就不會結束、連接埠也不會放掉。等太久就直接中止它。
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

impl Running {
    /// 通知關閉，並等它真的放掉連接埠（最多 `grace`，超過就中止任務、強制放掉）。
    async fn stop_and_wait(self, grace: std::time::Duration) {
        let Running { shutdown, mut task, .. } = self;
        let _ = shutdown.send(());
        if tokio::time::timeout(grace, &mut task).await.is_err() {
            task.abort();
            let _ = task.await;
        }
    }
}

/// 剛放掉的連接埠偶爾還要一瞬間才能再綁；短暫重試，別一次失敗就放棄。
async fn bind_with_retry(addr: SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    let mut last = None;
    for _ in 0..10 {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
    Err(last.expect("迴圈至少跑過一次"))
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
        app_handle: Option<tauri::AppHandle>,
        pty_manager: Arc<PtyManager>,
    ) -> anyhow::Result<()> {
        // 先關舊的並等它放掉連接埠：只送關閉訊號就直接綁，舊的 listener 還握著埠，
        // 設定頁每次「儲存」都會撞上 Address already in use（實機遇到）。
        let old = self.running.lock().take();
        if let Some(old) = old {
            old.stop_and_wait(STOP_GRACE).await;
        }

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = bind_with_retry(addr).await.map_err(|e| {
            anyhow::anyhow!("無法綁定 127.0.0.1:{port}（{e}）。請在設定裡換一個埠。")
        })?;

        let db_manager = Arc::new(DbManager::new());
        let sidecar = Arc::new(Db2SidecarState::new(resolve_db2_sidecar_path()));
        let kb_pool = KnowledgeBaseDb::new().await.pool;
        let coordination_registry = Arc::new(CoordinationRegistry::new());

        let app = server::router(
            Arc::new(token),
            db_manager,
            config,
            secrets,
            sidecar,
            kb_pool,
            pty_manager,
            app_handle,
            coordination_registry,
        );
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = rx.await; })
                .await;
            if let Err(e) = served {
                log::error!("MCP tool server 結束於錯誤：{e}");
            }
        });

        *self.running.lock() = Some(Running { port, shutdown: tx, task });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
            // 不等：同步呼叫端（停用開關）不需要馬上重綁；任務隨後自己結束。
            drop(r.task);
        }
    }
}

#[cfg(test)]
mod rebind_tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    /// 起一個只回 200 的伺服器，並讓一個客戶端保持連線不斷（像 Claude Code 的 keep-alive）。
    async fn serve_with_idle_client() -> (Running, SocketAddr, tokio::net::TcpStream) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" }));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).with_graceful_shutdown(async { let _ = rx.await; }).await;
        });
        let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
        c.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        (Running { port: addr.port(), shutdown: tx, task }, addr, c)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn binding_right_after_only_sending_the_shutdown_signal_still_hits_the_old_listener() {
        // 對照組：舊做法（送訊號後立刻重綁，沒讓舊任務有機會處理）。證明測試會重現 Address already in use。
        let (running, addr, _client) = serve_with_idle_client().await;
        let _ = running.shutdown.send(());
        assert!(tokio::net::TcpListener::bind(addr).await.is_err(), "舊做法：舊任務還沒處理關閉，埠仍被握著");
        running.task.abort();
    }

    #[tokio::test]
    async fn stop_and_wait_releases_the_port_even_with_an_idle_client_connected() {
        let (running, addr, _client) = serve_with_idle_client().await;
        running.stop_and_wait(std::time::Duration::from_millis(300)).await;
        let rebound = bind_with_retry(addr).await;
        assert!(rebound.is_ok(), "關閉後要能立刻重綁同一個埠：{:?}", rebound.err());
    }
}

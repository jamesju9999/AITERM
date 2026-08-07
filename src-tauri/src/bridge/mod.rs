//! Claude Code 橋接：一個綁 127.0.0.1 的 Anthropic Messages API 相容 server，
//! 把 Claude Code CLI 的請求翻譯到 AITerm 已設定的任一 AI 供應商。
//!
//! 為什麼不擴充 `ai::AiProvider`：Claude Code 需要串流 tool_use、thinking
//! 區塊與 cache_control，這些保真度 AITerm 自己的 UI 完全用不到。把它們塞
//! 進共用 trait 會讓 7 支 client、Agent loop 與 chat hook 全進入爆炸半徑，
//! 受益者卻只有一個消費者。因此另開模組，但憑證解析與端點常數共用
//! （見 `upstream/` 各 adapter）。

pub mod anthropic;
pub mod auth;
pub mod env;
pub mod factory;
pub mod model_map;
pub mod server;
pub mod stream;
pub mod tool_meta;
pub mod upstream;

use std::net::SocketAddr;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::config::ConfigStore;
use crate::secret::SecretStore;
use tool_meta::ToolMetaCache;

/// 橋接 server 的生命週期：持有目前執行中 server 的 handle，能 start/stop。
pub struct BridgeState {
    running: Mutex<Option<Running>>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for BridgeState {
    fn default() -> Self {
        Self { running: Mutex::new(None) }
    }
}

impl BridgeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// 啟動 server。已經在跑就先停掉（換 port 時會用到）。
    ///
    /// 埠被占用時回錯誤而不是換一個 —— 環境變數只能在分頁 spawn 的瞬間決定，
    /// 埠若會漂移，已開的分頁會指向死位址。
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

        let app = server::router(server::AppState {
            config,
            secrets,
            token: Arc::new(token),
            // 每次 start() 都是一個新的 server 生命週期，快取跟著重建即可
            // ——它只是把「200 而非 400」機率最大化，不需要跨 restart 存活。
            tool_meta: Arc::new(ToolMetaCache::default()),
        });
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let served = axum::serve(listener, app)
                .with_graceful_shutdown(async { let _ = rx.await; })
                .await;
            if let Err(e) = served {
                log::error!("bridge server 結束於錯誤：{e}");
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

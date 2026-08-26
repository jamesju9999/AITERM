//! 遠端終端機共享：讓同區網的同事用一組 6 位短碼連進本機的某個終端機分頁，
//! 看畫面並（經同意後）接手控制。
//!
//! 與 `crate::bridge`（Claude Code 的 Anthropic API 相容層）和
//! `crate::mcp_server`（AITerm 對外的 MCP 工具 server）並列但**刻意分開**：
//! 那兩個綁 127.0.0.1，這一個綁區網介面。把路由混進 mcp_server 等於順手把
//! `execute_query`（允許任意 SQL）暴露到辦公室網路。
//!
//! 設計文件：`docs/superpowers/specs/2026-08-26-remote-terminal-sharing-design.md`

pub mod protocol;
pub mod registry;
pub mod server;
pub mod tls;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use parking_lot::Mutex;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

use crate::pty::manager::PtyManager;
use protocol::ConnectionExporter;
use registry::ShareRegistry;

/// rustls 0.23 要求行程層級的預設加密供應者。裝一次就好；重複呼叫會回
/// `Err`，直接忽略——那代表別人已經裝過了，不是錯誤。
fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Server 生命週期。鏡像 `mcp_server::McpToolServerState`，但有兩個關鍵差異：
/// 綁的是 `0.0.0.0`（區網可達）而不是 `127.0.0.1`，而且**只在有分頁正在分享
/// 時存在**——最後一個分享停止就關閉，不留常駐監聽。
pub struct ShareServerState {
    running: Mutex<Option<Running>>,
    pub registry: Arc<ShareRegistry>,
}

struct Running {
    port: u16,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

impl Default for ShareServerState {
    fn default() -> Self {
        Self { running: Mutex::new(None), registry: Arc::new(ShareRegistry::new()) }
    }
}

impl ShareServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn port(&self) -> Option<u16> {
        self.running.lock().as_ref().map(|r| r.port)
    }

    /// 啟動 server（若尚未啟動）。綁 `0.0.0.0:0` 讓 OS 挑一個空閒 port——與
    /// bridge/mcp_server 不同，這裡沒有外部設定檔記著位址，所以浮動 port 不
    /// 會讓任何東西指向死地址。
    pub async fn start_if_needed(&self, pty: Arc<PtyManager>) -> anyhow::Result<u16> {
        self.start_if_needed_on_port(pty, 0).await
    }

    /// 同 `start_if_needed`，但綁指定的 port。`0` 表示交給 OS 挑。
    ///
    /// 手動的區網連通性檢查需要固定 port：另一台機器必須**先知道**要連哪裡，
    /// 而浮動 port 逼人先把 server 跑起來才看得到位址。
    pub async fn start_if_needed_on_port(
        &self,
        pty: Arc<PtyManager>,
        port: u16,
    ) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
        let port = listener.local_addr()?.port();
        let app = server::router(pty, Arc::clone(&self.registry));
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 自己的 accept 迴圈而不是 axum::serve——見下方「TLS 的接線」。
        tokio::spawn(serve_tls(listener, app, identity, rx));
        *self.running.lock() = Some(Running { port, shutdown: tx });
        Ok(port)
    }

    /// 沒有分頁在分享時關閉 server。呼叫端在每次 `stop_share` 之後叫這支。
    pub fn stop_if_idle(&self) {
        if self.registry.any_active() {
            return;
        }
        if let Some(r) = self.running.lock().take() {
            let _ = r.shutdown.send(());
        }
    }
}

/// TLS accept 迴圈。每條連線握手完成後先導出金鑰 material，塞進 request
/// extension（SAS 由 Task 7b 的承諾流程用它算出），
/// 再交給 axum router。
///
/// 用 `serve_connection_with_upgrades`（不是 `serve_connection`）——WebSocket
/// 是 HTTP upgrade，用錯那支的話升級請求會被當成普通請求處理，ws 永遠接不起來。
async fn serve_tls(
    listener: tokio::net::TcpListener,
    app: Router,
    identity: tls::ShareIdentity,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let server_config = match rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![identity.cert_der.clone()], identity.key_der.clone_key())
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("共享 server TLS 設定失敗：{e}");
            return;
        }
    };
    let acceptor = TlsAcceptor::from(Arc::new(server_config));

    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((s, _peer)) => s,
                Err(e) => {
                    log::error!("共享 server accept 失敗：{e}");
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };

        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let Ok(tls_stream) = acceptor.accept(stream).await else {
                // 握手失敗（對方不講 TLS、或憑證被拒）——安靜放掉這條連線。
                return;
            };

            // 握手已完成，可以導出金鑰 material 了。握手前呼叫會失敗。
            //
            // 導不出來就**放掉這條連線**，不要用零值或預設值頂替：那等於讓
            // 一條沒有身分保證的連線混進來，而使用者畫面上照樣會顯示一組看
            // 起來正常的 4 位數。
            let exporter = {
                let (_io, conn) = tls_stream.get_ref();
                match tls::exporter_material(conn) {
                    Ok(m) => m,
                    Err(e) => {
                        log::warn!("共享連線導出金鑰 material 失敗，放棄這條連線：{e}");
                        return;
                    }
                }
            };

            let io = TokioIo::new(tls_stream);
            let svc = hyper::service::service_fn(move |mut req: hyper::Request<Incoming>| {
                req.extensions_mut().insert(ConnectionExporter(exporter));
                let mut app = app.clone();
                async move { app.call(req).await }
            });

            let _ = HyperBuilder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(io, svc)
                .await;
        });
    }
}

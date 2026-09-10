//! 遠端終端機共享：協定、短碼註冊表、TLS 身分與 SAS、mDNS 廣播、
//! server（`server`）與觀看端（`viewer`）。
//!
//! 把事件推播給 GUI 的那一層留在 `app` crate 的 `share::viewer_manager`。

pub mod events;
pub mod mdns;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod tls;
pub mod viewer;

/// rustls 0.23 要求行程層級的預設加密供應者。裝一次就好；重複呼叫會回
/// `Err`，直接忽略——那代表別人已經裝過了，不是錯誤。
pub fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HyperBuilder;
use parking_lot::Mutex;
use tokio_rustls::TlsAcceptor;
use tower_service::Service;

use crate::pty::PtyManager;
use events::ShareEvents;
use protocol::ConnectionExporter;
use registry::ShareRegistry;

/// Server 生命週期。鏡像 `mcp_server::McpToolServerState`，但有兩個關鍵差異：
/// 綁的是 `0.0.0.0`（區網可達）而不是 `127.0.0.1`，而且**只在有分頁正在分享
/// 時存在**——最後一個分享停止就關閉，不留常駐監聽。
pub struct ShareServerState {
    running: Mutex<Option<Running>>,
    pub registry: Arc<ShareRegistry>,
}

struct Running {
    port: u16,
    bound: SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    /// `None` 代表這台機器上 mDNS daemon 啟動失敗（例如環境不允許
    /// multicast）——分享功能本身**不能**因為這樣就失敗，只是不會被自動
    /// 發現，使用者退回手動位址一樣能連。
    mdns: Option<mdns::MdnsAdvertiser>,
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

    /// 實際綁定到的位址，沒在跑時回 `None`。
    pub fn bound_addr(&self) -> Option<SocketAddr> {
        self.running.lock().as_ref().map(|r| r.bound)
    }

    /// 啟動 server（若尚未啟動）。綁 `0.0.0.0:0` 讓 OS 挑一個空閒 port——與
    /// bridge/mcp_server 不同，這裡沒有外部設定檔記著位址，所以浮動 port 不
    /// 會讓任何東西指向死地址。
    pub async fn start_if_needed(
        &self,
        pty: Arc<PtyManager>,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on(pty, std::net::Ipv4Addr::UNSPECIFIED, 0, events).await
    }

    /// 同 `start_if_needed`，但綁指定的 port。`0` 表示交給 OS 挑。
    ///
    /// 手動的區網連通性檢查需要固定 port：另一台機器必須**先知道**要連哪裡，
    /// 而浮動 port 逼人先把 server 跑起來才看得到位址。
    pub async fn start_if_needed_on_port(
        &self,
        pty: Arc<PtyManager>,
        port: u16,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        self.start_if_needed_on(pty, std::net::Ipv4Addr::UNSPECIFIED, port, events).await
    }

    /// 啟動 server（若尚未啟動），綁在指定的位址與 port。
    ///
    /// `port` 為 `0` 表示交給 OS 挑。位址之所以是參數而不是寫死 `0.0.0.0`：
    /// headless 的 CLI host 常常跑在只有一張網卡該被暴露的機器上，而
    /// `--bind 127.0.0.1` 配 SSH tunnel 是那台機器上最保守的用法。
    pub async fn start_if_needed_on(
        &self,
        pty: Arc<PtyManager>,
        addr: std::net::Ipv4Addr,
        port: u16,
        events: Arc<dyn ShareEvents>,
    ) -> anyhow::Result<u16> {
        if let Some(p) = self.port() {
            return Ok(p);
        }
        ensure_crypto_provider();
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((addr, port))).await?;
        let bound = listener.local_addr()?;
        let app_router = server::router(pty, Arc::clone(&self.registry), events);
        let identity = tls::ShareIdentity::generate()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // 自己的 accept 迴圈而不是 axum::serve——見下方「TLS 的接線」。
        tokio::spawn(serve_tls(listener, app_router, identity, rx));
        let mdns = match mdns::MdnsAdvertiser::start() {
            Ok(a) => Some(a),
            Err(e) => {
                log::warn!("mDNS daemon 啟動失敗，這次分享不會被自動發現：{e}");
                None
            }
        };
        *self.running.lock() = Some(Running { port: bound.port(), bound, shutdown: tx, mdns });
        Ok(bound.port())
    }

    /// 沒有分頁在分享時關閉 server。呼叫端在每次 `stop_share` 之後叫這支。
    pub fn stop_if_idle(&self) {
        if self.registry.any_active() {
            return;
        }
        if let Some(r) = self.running.lock().take() {
            if let Some(mdns) = &r.mdns {
                mdns.shutdown();
            }
            let _ = r.shutdown.send(());
        }
    }

    /// 幫某個分頁的短碼註冊 mDNS 廣播。server 還沒啟動（`running` 是
    /// `None`）或這次啟動時 mDNS daemon 沒能起來時，安靜地什麼都不做——
    /// 呼叫端（`commands::share::share_start`）不需要關心這兩種情況。
    pub fn mdns_register(&self, tab_id: &str, code: &str) {
        let mut running = self.running.lock();
        let Some(r) = running.as_mut() else { return };
        let port = r.port;
        if let Some(mdns) = r.mdns.as_mut() {
            mdns.register(tab_id, code, port);
        }
    }

    /// 取消某個分頁的 mDNS 廣播。
    pub fn mdns_unregister(&self, tab_id: &str) {
        let mut running = self.running.lock();
        let Some(r) = running.as_mut() else { return };
        if let Some(mdns) = r.mdns.as_mut() {
            mdns.unregister(tab_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn binding_to_loopback_does_not_listen_on_all_interfaces() {
        // `--bind 127.0.0.1` 的意義就是「不要暴露在網路上」。若位址參數被忽略、
        // 實際還是綁 0.0.0.0，這個承諾就是假的，而使用者不會有任何跡象——
        // 從 loopback 連得上，看起來一切正常。
        let state = ShareServerState::new();
        let pty = Arc::new(crate::pty::PtyManager::new());
        let port = state
            .start_if_needed_on(pty, std::net::Ipv4Addr::LOCALHOST, 0, Arc::new(events::SilentEvents))
            .await
            .expect("server starts");

        // loopback 連得上
        assert!(
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.is_ok(),
            "loopback should be reachable"
        );

        // 但實際綁定的位址必須是 loopback，不是萬用位址
        assert_eq!(state.bound_addr().map(|a| a.ip()), Some(std::net::Ipv4Addr::LOCALHOST.into()));
    }
}

//! 啟動 server：綁埠、跑 TLS accept 迴圈。
use crate::state::AppState;
use crate::tls::Identity;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct Running {
    pub addr: SocketAddr,
    pub state: Arc<AppState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Running {
    pub async fn stop(mut self) {
        crate::channels::stop(&self.state);
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = self.task.await;
    }
    /// 等到伺服器結束（正常執行時就是永遠）。
    pub async fn wait(&mut self) {
        let _ = (&mut self.task).await;
    }
}

pub async fn start(bind: SocketAddr, state: Arc<AppState>, identity: Identity) -> std::io::Result<Running> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let app = crate::routes::build(state.clone());
    crate::channels::reload(&state).await;
    crate::channels::start_ticker(&state);
    crate::channels::start_email(&state);
    crate::channels::start_telegram(&state);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(crate::tls::serve(listener, app, identity, rx));
    Ok(Running { addr, state, shutdown: Some(tx), task })
}

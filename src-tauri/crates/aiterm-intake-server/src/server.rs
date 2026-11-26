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
        crate::runner::stop(&self.state);
        crate::orchestrator::stop(&self.state);
        crate::scheduler::stop(&self.state);
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
    // 行程重啟後，上一個行程領的卡片／題目已經沒人在做：立刻放回，不必等租約逾期。
    // 驗收被強制結束時留下的伺服器（佔著驗收用的埠）也收掉。
    crate::verifier::cleanup_stale(&state).await;
    crate::skills::cleanup_stale(&state.data_dir);
    // 先把上一個行程留下的孤兒 Claude 殺掉：它還在改同一個 worktree，接續的對話不能和它同時寫。
    if let Ok(ids) = aiterm_intake_core::board_store::all_session_ids(&state.pools.intake).await {
        let n = crate::runner::kill_orphans(&ids);
        if n > 0 {
            log::warn!("上一個行程留下 {n} 個還在跑的 Claude Code，已結束");
        }
    }
    let now = crate::db::now();
    aiterm_intake_core::board_store::release_claims_by(&state.pools.intake, crate::runner::WHO, now).await.ok();
    aiterm_intake_core::store::release_claims_by(&state.pools.intake, crate::runner::WHO, now).await.ok();
    crate::runner::start(&state);
    crate::orchestrator::start(&state);
    crate::scheduler::start(&state);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(crate::tls::serve(listener, app, identity, rx));
    Ok(Running { addr, state, shutdown: Some(tx), task })
}

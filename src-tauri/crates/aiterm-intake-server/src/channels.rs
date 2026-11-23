//! 收訊管道的啟停：LINE webhook 與「新問題自動開題」的定時掃描。
//! 設定或祕密變動後呼叫 `reload`。網頁表單與 Email、Telegram 需要寄信／IMAP，等 server 支援後再接上。
use crate::state::AppState;
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::line_server::{self, LineState, LINE_SECRET_ID, LINE_TOKEN_ID};
use aiterm_intake_core::telegram_source::Candidate;
use aiterm_intake_core::{config, pipeline};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 多久掃一次「該不該把群組訊息開成題目」。
pub const TICK_EVERY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Channels {
    line: Mutex<Option<tokio::task::JoinHandle<()>>>,
    line_on: AtomicBool,
    ticker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

pub fn line_listening(st: &AppState) -> bool {
    st.channels.line_on.load(Ordering::SeqCst)
}

/// 每個專案目前的設定（歸屬判斷用）。
async fn candidates(st: &AppState) -> Vec<Candidate> {
    let mut out = Vec::new();
    for p in crate::routes_data::project_list(st).await {
        let cfg = config::load(&st.pools.intake, &p.id).await;
        out.push(Candidate { id: p.id, name: p.name, cfg });
    }
    out
}

fn secret(st: &AppState, id: &str) -> Option<String> {
    st.secrets.get(id).filter(|s| !s.trim().is_empty())
}

/// 依目前設定（重新）啟動 LINE webhook：任一專案啟用 LINE 且已設 Channel secret 才開。
pub async fn reload(st: &Arc<AppState>) {
    if let Some(h) = st.channels.line.lock().take() {
        h.abort();
    }
    st.channels.line_on.store(false, Ordering::SeqCst);
    let cands = candidates(st).await;
    let Some(secret_value) = secret(st, LINE_SECRET_ID) else { return };
    if !cands.iter().any(|c| c.cfg.line_enabled) {
        return;
    }
    let port = config::load_global(&st.pools.intake).await.line_port;
    let addr = std::net::SocketAddr::from((st.channel_bind, port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("LINE webhook 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (for_cands, for_change) = (st.clone(), st.clone());
    let state = Arc::new(LineState {
        pool: st.pools.intake.clone(),
        secret: secret_value,
        attach_root: st.data_dir.join("attachments"),
        token: secret(st, LINE_TOKEN_ID),
        api_base: st.line_api_base.lock().clone(),
        candidates: Box::new(move || {
            let st = for_cands.clone();
            Box::pin(async move { candidates(&st).await })
        }),
        on_change: Box::new(move || for_change.events.changed()),
    });
    let flag = st.clone();
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, line_server::router(state)).await {
            log::warn!("LINE webhook 結束：{e}");
        }
        flag.channels.line_on.store(false, Ordering::SeqCst);
    });
    st.channels.line_on.store(true, Ordering::SeqCst);
    *st.channels.line.lock() = Some(task);
}

/// 掃一次：LINE 群組的訊息安靜夠久、我方沒回、AI 判斷是新問題，就開成題目。回傳有沒有開出新題。
pub async fn tick_once(st: &Arc<AppState>) -> bool {
    let cands = candidates(st).await;
    let opened = pipeline::line_triage(&st.pools.intake, &cands, st.ai.as_ref(), &st.data_dir.join("attachments")).await;
    if opened {
        st.events.changed();
    }
    opened
}

pub fn start_ticker(st: &Arc<AppState>) {
    let me = st.clone();
    let task = tokio::spawn(async move {
        let mut every = tokio::time::interval(TICK_EVERY);
        loop {
            every.tick().await;
            tick_once(&me).await;
        }
    });
    *st.channels.ticker.lock() = Some(task);
}

pub fn stop(st: &AppState) {
    if let Some(h) = st.channels.line.lock().take() {
        h.abort();
    }
    if let Some(h) = st.channels.ticker.lock().take() {
        h.abort();
    }
    st.channels.line_on.store(false, Ordering::SeqCst);
}

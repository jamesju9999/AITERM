//! LINE webhook 伺服器的 AITERM 外殼：依設定啟停、讀系統鑰匙圈。收訊邏輯與推送在 aiterm-intake-core 的 `line_server`。
pub use aiterm_intake_core::line_server::*;
use aiterm_intake_core::attach;
use aiterm_intake_core::telegram_source::Candidate;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Default)]
pub struct LineServer {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    listening: AtomicBool,
}

impl LineServer {
    pub fn listening(&self) -> bool {
        self.listening.load(Ordering::SeqCst)
    }
}

fn secret_of<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<String> {
    app.try_state::<Arc<crate::secret::SecretStore>>()?.get(id).ok().flatten().filter(|s| !s.trim().is_empty())
}

/// 依目前設定（重新）啟動 webhook 伺服器：任一專案啟用 LINE 且已設 Channel secret 才開。
pub async fn restart<R: Runtime>(app: &AppHandle<R>) {
    let Some(server) = app.try_state::<Arc<LineServer>>() else { return };
    if let Some(h) = server.handle.lock().take() {
        h.abort();
    }
    server.listening.store(false, Ordering::SeqCst);
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    let any_enabled = super::all_configs(app, &db.pool).await.into_iter().any(|(_, _, c)| c.line_enabled);
    let Some(secret) = secret_of(app, LINE_SECRET_ID) else { return };
    if !any_enabled {
        return;
    }
    let port = super::config::load_global(&db.pool).await.line_port;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("intake line: 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (a_cands, a_change) = (app.clone(), app.clone());
    let st = Arc::new(LineState {
        pool: db.pool.clone(),
        secret,
        attach_root: attach::root(),
        token: secret_of(app, LINE_TOKEN_ID),
        api_base: API_BASE.to_string(),
        candidates: Box::new(move || {
            let app = a_cands.clone();
            Box::pin(async move {
                let Some(db) = app.try_state::<super::IntakeDb>() else { return Vec::new() };
                super::all_configs(&app, &db.pool).await.into_iter().map(|(id, name, cfg)| Candidate { id, name, cfg }).collect()
            })
        }),
        on_change: Box::new(move || {
            use tauri::Emitter;
            let _ = a_change.emit(super::CHANGED_EVENT, ());
        }),
    });
    let flag: Arc<LineServer> = server.inner().clone();
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router(st)).await {
            log::warn!("intake line: 伺服器結束：{e}");
        }
        flag.listening.store(false, Ordering::SeqCst);
    });
    server.listening.store(true, Ordering::SeqCst);
    *server.handle.lock() = Some(task);
}

/// 回覆用憑證；沒設定時回人看得懂的錯誤。
pub fn token<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    secret_of(app, LINE_TOKEN_ID).ok_or_else(|| "尚未設定 LINE 的 Channel access token".to_string())
}

/// 供設定頁顯示：憑證是否已設定（憑證本身不外傳）與監聽狀態。
#[derive(serde::Serialize)]
pub struct Status {
    pub has_secret: bool,
    pub has_token: bool,
    pub port: u16,
    pub listening: bool,
}

pub async fn status<R: Runtime>(app: &AppHandle<R>) -> Status {
    let port = match app.try_state::<super::IntakeDb>() {
        Some(db) => super::config::load_global(&db.pool).await.line_port,
        None => super::config::GlobalConfig::default().line_port,
    };
    Status {
        has_secret: secret_of(app, LINE_SECRET_ID).is_some(),
        has_token: secret_of(app, LINE_TOKEN_ID).is_some(),
        port,
        listening: app.try_state::<Arc<LineServer>>().is_some_and(|s| s.listening()),
    }
}


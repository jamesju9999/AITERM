//! 網頁表單伺服器的 AITERM 外殼：依設定啟停。路由與邏輯在 aiterm-intake-core 的 `web`。
pub use aiterm_intake_core::web::*;
use aiterm_intake_core::attach;
use aiterm_intake_core::config::WebMode;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

#[derive(Default)]
pub struct WebServer {
    handle: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// 依目前設定（重新）啟動網頁表單伺服器。
pub async fn restart<R: Runtime>(app: &AppHandle<R>) {
    let Some(server) = app.try_state::<Arc<WebServer>>() else { return };
    if let Some(h) = server.handle.lock().take() {
        h.abort();
    }
    let Some(db) = app.try_state::<super::IntakeDb>() else { return };
    // 任一專案啟用網頁表單就開伺服器；任一專案設為區網就綁 0.0.0.0。
    let modes: Vec<WebMode> = super::all_configs(app, &db.pool).await.into_iter().map(|(_, _, c)| c.web_mode).collect();
    let host = if modes.contains(&WebMode::Lan) {
        [0, 0, 0, 0]
    } else if modes.contains(&WebMode::Local) {
        [127, 0, 0, 1]
    } else {
        return;
    };
    let port = aiterm_intake_core::config::load_global(&db.pool).await.web_port;
    let addr = std::net::SocketAddr::from((host, port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::warn!("intake web: 無法綁定 {addr}：{e}");
            return;
        }
    };
    let (a1, a2, a3) = (app.clone(), app.clone(), app.clone());
    let st = Arc::new(WebState {
        pool: db.pool.clone(),
        otps: Otps::default(),
        attach_root: attach::root(),
        on_change: Box::new(move || {
            use tauri::Emitter;
            let _ = a1.emit(super::CHANGED_EVENT, ());
        }),
        projects: Box::new(move || super::open_projects(&a3)),
        send_mail: Box::new(move |project_id, to, subject, body| {
            let app = a2.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = super::runtime::send_plain_mail(&app, &project_id, &to, &subject, &body).await {
                    log::warn!("intake web: 寄驗證碼失敗：{e}");
                }
            });
        }),
    });
    let task = tauri::async_runtime::spawn(async move {
        if let Err(e) = axum::serve(listener, router(st)).await {
            log::warn!("intake web: 伺服器結束：{e}");
        }
    });
    *server.handle.lock() = Some(task);
}


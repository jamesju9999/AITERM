//! 讓 AITerm 被當成終端機使用：把「開這個資料夾／跑這個指令」的請求，
//! 從冷啟動 argv、第二次啟動、macOS Opened 三個入口收斂成同一個佇列。

pub mod parse;
pub mod queue;

pub use parse::{args_from_file_urls, parse_args, LaunchRequest};
pub use queue::LaunchQueue;

use tauri::{AppHandle, Emitter, Manager, State};

/// 有新請求入列時發給前端的事件（無酬載——前端收到後自己 `take_launch_requests`）。
pub const PENDING_EVENT: &str = "launch-request-pending";

/// 前端取走所有待處理請求。
#[tauri::command]
pub fn take_launch_requests(queue: State<'_, LaunchQueue>) -> Vec<LaunchRequest> {
    queue.take()
}

/// 入列並通知前端。沒有請求就什麼都不做。
pub fn enqueue_and_notify(app: &AppHandle, requests: Vec<LaunchRequest>) {
    if requests.is_empty() {
        return;
    }
    app.state::<LaunchQueue>().push(requests);
    if let Err(e) = app.emit(PENDING_EVENT, ()) {
        log::warn!("emit {PENDING_EVENT} failed: {e}");
    }
}

/// 第二次啟動（single-instance 外掛的 callback）：解析、入列、把視窗拉到前景。
pub fn on_second_instance(app: &AppHandle, argv: Vec<String>, cwd: String) {
    let requests = parse_args(&argv, Some(std::path::Path::new(&cwd)));
    enqueue_and_notify(app, requests);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// macOS「用 AITerm 開啟」／拖到 Dock 圖示。其它平台沒有這個事件。
pub fn on_run_event(app: &AppHandle, event: &tauri::RunEvent) {
    #[cfg(target_os = "macos")]
    {
        if let tauri::RunEvent::Opened { urls } = event {
            let argv = args_from_file_urls(urls);
            enqueue_and_notify(app, parse_args(&argv, None));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, event);
    }
}

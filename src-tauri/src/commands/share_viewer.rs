//! 觀看端的 Tauri commands。
//!
//! 傳輸跑在 Rust（見 `share::viewer` 的說明），前端只負責發起連線、
//! 送按鍵、訂閱事件。

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::share::viewer_manager::ViewerManager;

#[tauri::command]
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    viewers: State<'_, Arc<ViewerManager>>,
    app: AppHandle,
) -> Result<String, String> {
    viewers
        .connect(app, host, port, code, display_name)
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn share_viewer_send(
    conn_id: String,
    data: String,
    viewers: State<'_, Arc<ViewerManager>>,
) -> Result<(), String> {
    viewers
        .send(&conn_id, data.into_bytes())
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub async fn share_viewer_disconnect(
    conn_id: String,
    viewers: State<'_, Arc<ViewerManager>>,
) -> Result<(), String> {
    viewers.disconnect(&conn_id);
    Ok(())
}

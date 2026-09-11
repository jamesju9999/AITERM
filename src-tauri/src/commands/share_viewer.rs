//! 觀看端的 Tauri commands。
//!
//! 傳輸跑在 Rust（見 `share::viewer` 的說明），前端只負責發起連線、
//! 送按鍵、訂閱事件。

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::share::viewer_manager::{Connected, ViewerManager};

#[tauri::command]
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    key: Option<String>,
    viewers: State<'_, Arc<ViewerManager>>,
    app: AppHandle,
) -> Result<Connected, String> {
    viewers
        .connect(app, host, port, code, display_name, key)
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

/// 前端訂閱好所有事件了，放行事件 pump。
///
/// **這不是可有可無的最佳化。** 沒有它的話，主控端瞬間核准（CLI host 的金鑰
/// 模式就是這樣）時，`Granted` 與它後面那批畫面重播會在前端掛載訂閱之前就
/// 送出去，而 Tauri 事件不重播——畫面會永遠停在「等待對方同意」。
#[tauri::command]
pub async fn share_viewer_ready(
    conn_id: String,
    viewers: State<'_, Arc<ViewerManager>>,
) -> Result<(), String> {
    viewers.mark_ready(&conn_id);
    Ok(())
}

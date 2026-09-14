//! 觀看端的 Tauri commands。
//!
//! 傳輸跑在 Rust（見 `share::viewer` 的說明），前端只負責發起連線、
//! 送按鍵、訂閱事件。

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::commands::remote_hosts::resolve_connect_key;
use crate::secret::SecretStore;
use crate::share::viewer_manager::{Connected, ViewerManager};

#[tauri::command]
pub async fn share_viewer_connect(
    host: String,
    port: u16,
    code: String,
    display_name: String,
    key: Option<String>,
    // 地址簿的條目 id。帶了它就由後端自己去 keychain 取金鑰，`key` 不採用——
    // 已存的金鑰因此從頭到尾不跨 IPC。
    saved_host_id: Option<String>,
    viewers: State<'_, Arc<ViewerManager>>,
    secrets: State<'_, Arc<SecretStore>>,
    app: AppHandle,
) -> Result<Connected, String> {
    // **不要寫成 `secrets.get(k).ok().flatten()`。** SecretStore::get 回的是
    // Result<Option<String>>：Ok(None) 是「這台沒有這把金鑰」，Err 是「keychain
    // 讀不到」。壓成同一個 None 的話，keychain 鎖住會顯示成「金鑰不在這台電腦
    // 上，請重新輸入」，使用者就會去重貼一把其實好好的金鑰。
    let key = resolve_connect_key(saved_host_id.as_deref(), key, |k| {
        // 用 `{e:#}` 而不是 `.to_string()`：SecretStore 內部用 anyhow 的
        // with_context 包了一層，`.to_string()` 只會拿到最外層那句
        // 「opening keychain entry for ...」，真正的原因被吃掉。
        // （write_entry 已經為寫入路徑修過同一個問題，讀取路徑沒有。）
        secrets.get(k).map_err(|e| format!("{e:#}"))
    })?;
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

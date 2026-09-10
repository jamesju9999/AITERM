//! GUI 專屬：把一個 PTY session 的輸出接到 Tauri 事件上。
//!
//! session 的管理本身住在 `aiterm_core::pty::manager`。這裡只有「輸出去哪裡」
//! 這一件事——而那正是唯一依賴 Tauri 的部分。

use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use portable_pty::PtySize;
use tauri::{AppHandle, Emitter};

use aiterm_core::pty::error::PtyResult;
use aiterm_core::pty::events::{data_event_name, PtyDataPayload};
use aiterm_core::pty::PtyManager;

/// Spawn 一個 session 並把輸出接到 `pty://data/{id}` 事件。
///
/// `bridge_env` 非 None 時，把 Claude Code 橋接的環境變數注入這個分頁。
/// 環境變數只能在 spawn 的瞬間決定，所以事後無法對已開的分頁切換。
pub fn create_with_app(
    manager: &PtyManager,
    app: AppHandle,
    size: PtySize,
    cwd: Option<PathBuf>,
    bridge_env: Option<(u16, String)>,
) -> PtyResult<String> {
    let (envs, env_removals) = match bridge_env {
        Some((port, token)) => (
            crate::bridge::env::bridge_envs(port, &token),
            crate::bridge::env::ENV_TO_REMOVE.iter().map(|s| s.to_string()).collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let event_name = data_event_name(&id);

    manager.create_with_callback_and_id(size, id, cwd, envs, env_removals, move |chunk| {
        let payload = PtyDataPayload { base64: BASE64.encode(&chunk) };
        if let Err(e) = app.emit(&event_name, payload) {
            eprintln!("emit {event_name} failed: {e}");
        }
    })
}

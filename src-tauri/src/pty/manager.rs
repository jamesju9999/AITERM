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

/// 對指定 session 提權，輸出併入同一個 `pty://data/{id}` 事件；連線狀態變化
/// 額外發 `pty://elevation-state/{id}`，讓前端徽章能顯示/消失。
///
/// **會阻塞呼叫的執行緒**（UAC 對話框等待時間 + 具名管線連線沒有逾時）——見
/// `aiterm_core::pty::elevated::spawn_windows` 的文件註解。呼叫端（`pty_elevate`
/// Tauri command）必須用 `tokio::task::spawn_blocking` 之類的機制呼叫這個函式，
/// 不能直接從 async runtime 的 worker 執行緒或 UI 事件迴圈呼叫。
#[cfg(windows)]
pub fn elevate_with_app(
    manager: &PtyManager,
    app: AppHandle,
    id: String,
    shell_variant: aiterm_core::pty::cd_parser::ShellVariant,
) -> PtyResult<bool> {
    let data_event = data_event_name(&id);
    let app_for_output = app.clone();
    let state_event = aiterm_core::pty::events::elevation_state_event_name(&id);
    let app_for_disconnect = app.clone();
    let state_event_for_disconnect = state_event.clone();

    let started = manager.elevate(
        &id,
        shell_variant,
        move |chunk| {
            let payload = PtyDataPayload { base64: BASE64.encode(&chunk) };
            if let Err(e) = app_for_output.emit(&data_event, payload) {
                eprintln!("emit {data_event} failed: {e}");
            }
        },
        move || {
            let payload = aiterm_core::pty::events::ElevationStatePayload { elevated: false };
            if let Err(e) = app_for_disconnect.emit(&state_event_for_disconnect, payload) {
                eprintln!("emit {state_event_for_disconnect} failed: {e}");
            }
        },
    )?;

    if started {
        let payload = aiterm_core::pty::events::ElevationStatePayload { elevated: true };
        if let Err(e) = app.emit(&state_event, payload) {
            eprintln!("emit {state_event} failed: {e}");
        }
    }
    Ok(started)
}

#[cfg(not(windows))]
pub fn elevate_with_app(
    _manager: &PtyManager,
    _app: AppHandle,
    _id: String,
    _shell_variant: aiterm_core::pty::cd_parser::ShellVariant,
) -> PtyResult<bool> {
    Err(aiterm_core::pty::error::PtyError::Internal(
        "elevation not supported on this platform".into(),
    ))
}

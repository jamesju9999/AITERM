//! Tauri commands for reading and updating app configuration.

use std::sync::Arc;
use tauri::State;

use crate::config::{AppConfig, ConfigStore, DefaultTab, DocConvertEngine, ExecutionMode, SubmitShortcut, SuggestionAcceptKey};

#[tauri::command]
pub fn get_config(config: State<Arc<ConfigStore>>) -> AppConfig {
    config.get()
}

#[tauri::command]
pub fn set_execution_mode(
    mode: ExecutionMode,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.execution_mode = mode; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_onboarding_done(config: State<Arc<ConfigStore>>) -> bool {
    config.get().onboarding_done
}

#[tauri::command]
pub fn set_onboarding_done(config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config.update(|cfg| { cfg.onboarding_done = true; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_appimage_integration_declined(config: State<Arc<ConfigStore>>) -> bool {
    config.get().appimage_integration_declined
}

#[tauri::command]
pub fn set_appimage_integration_declined(config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|cfg| { cfg.appimage_integration_declined = true; })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_claude_notif_declined(config: State<Arc<ConfigStore>>) -> bool {
    config.get().claude_notif_declined
}

#[tauri::command]
pub fn set_claude_notif_declined(config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|cfg| { cfg.claude_notif_declined = true; })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_suggestion_accept_key(
    key: SuggestionAcceptKey,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.suggestion_accept_key = key; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_submit_shortcut(
    shortcut: SubmitShortcut,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.submit_shortcut = shortcut; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_doc_convert_engine(
    engine: DocConvertEngine,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.doc_convert_engine = engine; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_max_agent_steps(
    steps: u32,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.max_agent_steps = steps; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_default_tab(
    tab: DefaultTab,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.default_tab = tab; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_global_hotkey(
    hotkey: String,
    app: tauri::AppHandle,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    let old = config.get().global_hotkey;
    // 先註冊再存檔：註冊失敗時設定檔不變，舊快捷鍵也已由 apply_hotkey 還原。
    crate::resident::apply_hotkey(&app, &old, &hotkey)?;
    config.update(|cfg| { cfg.global_hotkey = hotkey; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_keep_in_tray(enabled: bool, config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config.update(|cfg| { cfg.keep_in_tray = enabled; }).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(
    enabled: bool,
    app: tauri::AppHandle,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    crate::resident::apply_autostart(&app, enabled)?;
    config.update(|cfg| { cfg.autostart = enabled; }).map_err(|e| e.to_string())
}

/// 視窗 ✕：能縮到托盤就隱藏並回傳 true；false 代表呼叫端應走原本的結束流程。
#[tauri::command]
pub fn hide_main_window_for_close(app: tauri::AppHandle) -> bool {
    crate::resident::hide_for_close(&app)
}

/// 語系改變時更新托盤選單文字。
#[tauri::command]
pub fn set_tray_labels(show: String, new_tab: String, quit: String, app: tauri::AppHandle) -> Result<(), String> {
    crate::resident::set_labels(&app, &show, &new_tab, &quit)
}


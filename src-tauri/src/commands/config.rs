//! Tauri commands for reading and updating app configuration.

use std::sync::Arc;
use tauri::State;

use crate::config::{AppConfig, ConfigStore, DefaultTab, ExecutionMode, SubmitShortcut};

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
pub fn set_submit_shortcut(
    shortcut: SubmitShortcut,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.submit_shortcut = shortcut; }).map_err(|e| e.to_string())
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


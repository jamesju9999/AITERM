//! Tauri commands for reading and updating app configuration.

use std::sync::Arc;
use tauri::State;

use crate::config::{AppConfig, ConfigStore, ExecutionMode, SubmitShortcut};

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
pub fn set_submit_shortcut(
    shortcut: SubmitShortcut,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.submit_shortcut = shortcut; }).map_err(|e| e.to_string())
}

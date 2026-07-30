//! Tauri surface for the managed Python environment.

use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::config::ConfigStore;
use crate::python_env::{self, profiles::Profile, EnvStatus};

#[tauri::command]
pub fn python_env_status(app: AppHandle) -> EnvStatus {
    python_env::status(&app)
}

#[tauri::command]
pub async fn python_env_ensure(app: AppHandle, profile: Profile) -> Result<(), String> {
    python_env::ensure(&app, profile).await.map(|_| ()).map_err(String::from)
}

#[tauri::command]
pub async fn python_env_reset(app: AppHandle, purge_runtimes: bool) -> Result<(), String> {
    python_env::reset(&app, purge_runtimes).await.map_err(String::from)
}

#[tauri::command]
pub fn python_env_set_interpreter(
    path: Option<String>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    config
        .update(|cfg| {
            cfg.python_interpreter = path.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
        })
        .map_err(|e| format!("儲存設定失敗：{e}"))
}

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
    let normalized = path.as_ref().map(|p| p.trim().to_string()).filter(|p| !p.is_empty());

    // Verify before storing: the settings page shows the new source immediately,
    // so an unusable path would leave the UI claiming an interpreter that only
    // fails later, when a rebuild finally hands it to uv. MarkItDown needs 3.10+.
    if let Some(candidate) = &normalized {
        let mut cmd = std::process::Command::new(candidate);
        cmd.arg("-c").arg("import sys; exit(0 if sys.version_info >= (3, 10) else 1)");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        match cmd.status() {
            Ok(s) if s.success() => {}
            Ok(_) => return Err("這個 Python 版本太舊，需要 3.10 或更新版本".to_string()),
            Err(e) => return Err(format!("無法執行這個路徑，請確認它是 Python 執行檔：{e}")),
        }
    }

    config
        .update(|cfg| cfg.python_interpreter = normalized)
        .map_err(|e| format!("儲存設定失敗：{e}"))
}

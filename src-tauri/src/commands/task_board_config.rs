//! Get/set for the task-board settings block. Plain config read/write — no
//! server to start or stop, unlike `commands/mcp_server.rs`. The scheduler
//! re-reads `config.get().task_board` on every wake, so a changed
//! `max_concurrent` takes effect on the next scheduler tick with no restart.

use std::sync::Arc;

use tauri::State;

use crate::config::types::TaskBoardConfig;
use crate::config::ConfigStore;

#[tauri::command]
pub fn task_board_get_config(config: State<Arc<ConfigStore>>) -> TaskBoardConfig {
    config.get().task_board
}

#[tauri::command]
pub fn task_board_set_config(
    value: TaskBoardConfig,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    let clamped = TaskBoardConfig {
        max_concurrent: value.max_concurrent.clamp(1, 16),
        claude_command: {
            let c = value.claude_command.trim();
            if c.is_empty() { "claude".to_string() } else { c.to_string() }
        },
        project_paths: value.project_paths,
    };
    config.update(|c| c.task_board = clamped).map_err(|e| e.to_string())
}

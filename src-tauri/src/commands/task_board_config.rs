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

/// 把設定對話框裡**使用者可編輯的**欄位套到現有設定上。
///
/// `project_paths` 刻意保留 `current` 的值、不從 `incoming` 取：它不是
/// 這個對話框管的欄位（由 `projects_create` / `projects_open` /
/// `projects_remove` 維護），而且前端的 `TaskBoardConfig` 型別根本沒有
/// 這個欄位——送過來的 payload 缺它，`#[serde(default)]` 會補成空陣列。
/// 若照抄 `incoming.project_paths`，使用者只要在設定裡改一次並行數，
/// 整份專案清單就會被清空。
pub(crate) fn apply_editable_task_board_fields(
    current: &TaskBoardConfig,
    incoming: TaskBoardConfig,
) -> TaskBoardConfig {
    TaskBoardConfig {
        max_concurrent: incoming.max_concurrent.clamp(1, 16),
        claude_command: {
            let c = incoming.claude_command.trim();
            if c.is_empty() { "claude".to_string() } else { c.to_string() }
        },
        project_paths: current.project_paths.clone(),
    }
}

#[tauri::command]
pub fn task_board_set_config(
    value: TaskBoardConfig,
    config: State<Arc<ConfigStore>>,
) -> Result<(), String> {
    config
        .update(|c| {
            let merged = apply_editable_task_board_fields(&c.task_board, value);
            c.task_board = merged;
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current() -> TaskBoardConfig {
        TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "claude".to_string(),
            project_paths: vec!["/projects/a".to_string(), "/projects/b".to_string()],
        }
    }

    /// 前端送來的 payload 沒有 project_paths（TS 的 TaskBoardConfig
    /// 型別就沒這個欄位），serde 補成空陣列。存一次設定不可以把
    /// 使用者的專案清單清光。
    #[test]
    fn saving_settings_does_not_wipe_the_project_list() {
        let incoming = TaskBoardConfig {
            max_concurrent: 3,
            claude_command: "claude".to_string(),
            project_paths: Vec::new(),
        };
        let merged = apply_editable_task_board_fields(&current(), incoming);
        assert_eq!(merged.max_concurrent, 3);
        assert_eq!(
            merged.project_paths,
            vec!["/projects/a".to_string(), "/projects/b".to_string()]
        );
    }

    /// 就算 payload 裡真的帶了 project_paths 也一律忽略——這個指令
    /// 不是管理專案清單的地方。
    #[test]
    fn project_paths_in_the_payload_are_ignored() {
        let incoming = TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "claude".to_string(),
            project_paths: vec!["/injected".to_string()],
        };
        let merged = apply_editable_task_board_fields(&current(), incoming);
        assert_eq!(
            merged.project_paths,
            vec!["/projects/a".to_string(), "/projects/b".to_string()]
        );
    }

    #[test]
    fn max_concurrent_is_clamped_to_1_16() {
        let mk = |n: u32| TaskBoardConfig {
            max_concurrent: n,
            claude_command: "claude".to_string(),
            project_paths: Vec::new(),
        };
        assert_eq!(apply_editable_task_board_fields(&current(), mk(0)).max_concurrent, 1);
        assert_eq!(apply_editable_task_board_fields(&current(), mk(99)).max_concurrent, 16);
        assert_eq!(apply_editable_task_board_fields(&current(), mk(7)).max_concurrent, 7);
    }

    #[test]
    fn a_blank_claude_command_falls_back_to_claude() {
        let incoming = TaskBoardConfig {
            max_concurrent: 5,
            claude_command: "   ".to_string(),
            project_paths: Vec::new(),
        };
        assert_eq!(
            apply_editable_task_board_fields(&current(), incoming).claude_command,
            "claude"
        );
    }
}

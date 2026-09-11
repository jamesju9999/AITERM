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
        auto_close_finished_tabs: incoming.auto_close_finished_tabs,
        notify_desktop_on_finish: incoming.notify_desktop_on_finish,
        notify_telegram_on_finish: incoming.notify_telegram_on_finish,
        stuck_timeout_secs: incoming.stuck_timeout_secs.clamp(60, 21_600),
        isolate_with_worktree: incoming.isolate_with_worktree,
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
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
            stuck_timeout_secs: 1200,
            isolate_with_worktree: true,
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
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
            stuck_timeout_secs: 1200,
            isolate_with_worktree: true,
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
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
            stuck_timeout_secs: 1200,
            isolate_with_worktree: true,
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
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
            stuck_timeout_secs: 1200,
            isolate_with_worktree: true,
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
            auto_close_finished_tabs: true,
            notify_desktop_on_finish: true,
            notify_telegram_on_finish: true,
            stuck_timeout_secs: 1200,
            isolate_with_worktree: true,
        };
        assert_eq!(
            apply_editable_task_board_fields(&current(), incoming).claude_command,
            "claude"
        );
    }

    #[test]
    fn auto_close_finished_tabs_is_taken_from_incoming() {
        let mut incoming = current();
        incoming.auto_close_finished_tabs = false;
        assert!(!apply_editable_task_board_fields(&current(), incoming).auto_close_finished_tabs);
    }

    #[test]
    fn notify_flags_are_taken_from_incoming() {
        let mut incoming = current();
        incoming.notify_desktop_on_finish = false;
        incoming.notify_telegram_on_finish = false;
        let merged = apply_editable_task_board_fields(&current(), incoming);
        assert!(!merged.notify_desktop_on_finish);
        assert!(!merged.notify_telegram_on_finish);
    }

    /// `apply_editable_task_board_fields` 是使用者可編輯欄位的白名單——
    /// 新欄位若忘了列進去，前端存檔會是靜靜的 no-op：設定頁看起來有變、
    /// 重開就打回原形，而且不會有任何錯誤。
    #[test]
    fn isolate_with_worktree_is_taken_from_incoming() {
        let mut incoming = current();
        incoming.isolate_with_worktree = false;
        assert!(!apply_editable_task_board_fields(&current(), incoming).isolate_with_worktree);

        let mut incoming = current();
        incoming.isolate_with_worktree = true;
        assert!(apply_editable_task_board_fields(&current(), incoming).isolate_with_worktree);
    }

    #[test]
    fn stuck_timeout_secs_is_taken_from_incoming() {
        let mut incoming = current();
        incoming.stuck_timeout_secs = 300;
        assert_eq!(apply_editable_task_board_fields(&current(), incoming).stuck_timeout_secs, 300);
    }

    #[test]
    fn stuck_timeout_secs_is_clamped_to_60_21600() {
        let mut incoming = current();
        incoming.stuck_timeout_secs = 0;
        assert_eq!(apply_editable_task_board_fields(&current(), incoming).stuck_timeout_secs, 60);

        let mut incoming = current();
        incoming.stuck_timeout_secs = 999_999;
        assert_eq!(apply_editable_task_board_fields(&current(), incoming).stuck_timeout_secs, 21_600);
    }
}

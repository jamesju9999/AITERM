use std::sync::Arc;

use tauri::State;

use super::tree::Snapshot;
use super::ClaudeActivity;

/// 側欄掛載時先取一次快照（事件可能在它訂閱之前就送出了）。
#[tauri::command]
pub fn claude_activity_snapshot(activity: State<'_, Arc<ClaudeActivity>>, session_id: String) -> Snapshot {
    activity.snapshot(&session_id)
}

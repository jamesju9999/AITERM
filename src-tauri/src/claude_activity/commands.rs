use std::sync::Arc;

use tauri::State;

use super::tree::{Snapshot, ToolDetail};
use super::ClaudeActivity;

/// 側欄掛載時先取一次快照（事件可能在它訂閱之前就送出了）。
#[tauri::command]
pub fn claude_activity_snapshot(activity: State<'_, Arc<ClaudeActivity>>, session_id: String) -> Snapshot {
    activity.snapshot(&session_id)
}

/// 點選工具節點時取完整輸入與輸出（不放進每次事件，避免事件變大）。
#[tauri::command]
pub fn claude_activity_tool_detail(
    activity: State<'_, Arc<ClaudeActivity>>,
    session_id: String,
    tool_id: String,
) -> Option<ToolDetail> {
    activity.tool_detail(&session_id, &tool_id)
}

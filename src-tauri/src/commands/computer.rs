//! 電腦操作（GUI 控制）的設定頁與授權對話框用的指令。工具本身走 MCP，不在這裡。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::computer::approval::{Decision, TauriApprover};
use crate::computer::audit::AuditEntry;
use crate::computer::ComputerService;
use crate::config::ConfigStore;

#[derive(Serialize)]
pub struct ComputerStatus {
    pub enabled: bool,
    pub accessibility: bool,
    pub screen_recording: bool,
    pub allowed_apps: Vec<String>,
}

#[tauri::command]
pub fn computer_status(
    service: State<Arc<ComputerService>>,
    config: State<Arc<ConfigStore>>,
) -> ComputerStatus {
    let cfg = config.get();
    let perms = service.permissions();
    ComputerStatus {
        enabled: cfg.mcp_tool_server.computer_enabled,
        accessibility: perms.accessibility,
        screen_recording: perms.screen_recording,
        allowed_apps: cfg.computer_allowed_apps,
    }
}

/// 電腦操作總開關。它管的是 Ask AI 與 MCP 工具伺服器兩條路，所以有自己的設定頁與
/// 指令，不跟著 MCP 工具伺服器的儲存走。工具清單與執行時都即時讀設定，改了就生效。
#[tauri::command]
pub fn computer_set_enabled(enabled: bool, config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|c| c.mcp_tool_server.computer_enabled = enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_revoke_app(key: String, config: State<Arc<ConfigStore>>) -> Result<(), String> {
    config
        .update(|c| c.computer_allowed_apps.retain(|k| k != &key))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_audit_tail(limit: Option<usize>, service: State<Arc<ComputerService>>) -> Vec<AuditEntry> {
    service.audit().tail(limit.unwrap_or(50).min(500))
}

/// 前端授權對話框的回覆。id 已逾時（後端早已視為拒絕）回傳 false。
#[tauri::command]
pub fn computer_resolve_approval(id: String, decision: Decision, approver: State<Arc<TauriApprover>>) -> bool {
    approver.resolve(&id, decision)
}

/// 開啟系統設定的對應隱私權頁面。`pane`：`accessibility` 或 `screen`。
#[tauri::command]
pub fn computer_open_privacy_settings(pane: String) -> Result<(), String> {
    let anchor = match pane.as_str() {
        "accessibility" => "Privacy_Accessibility",
        "screen" => "Privacy_ScreenCapture",
        other => return Err(format!("未知的設定頁「{other}」")),
    };
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}"))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = anchor;
        Err("目前只支援 macOS".to_string())
    }
}

/// 對話模式的 `run_command` 工具在執行指令前呼叫：依「執行模式」與指令風險決定直接跑、問使用者、或拒絕。
/// 回傳 `allowed`；不允許時 `reason` 是要回給模型看的原因。
#[tauri::command]
pub async fn terminal_command_gate(
    command: String,
    // 介面語系：授權框與拒絕原因用它產生文字。舊呼叫端沒帶時用繁中。
    locale: Option<crate::ai::Locale>,
    approver: State<'_, Arc<TauriApprover>>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<crate::terminal_tool::GateResult, String> {
    let mode = config.get().execution_mode;
    let locale = locale.unwrap_or(crate::ai::Locale::ZhTw);
    Ok(crate::terminal_tool::gate(approver.inner().as_ref(), mode, &command, locale).await)
}

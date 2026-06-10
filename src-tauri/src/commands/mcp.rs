// src-tauri/src/commands/mcp.rs

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Mutex;

use crate::config::{types::{McpServerConfig, McpTransport}, ConfigStore};
use crate::mcp::{McpManager, McpToolInfo, McpToolResult};

pub type McpManagerState = Arc<Mutex<McpManager>>;

/// Info sent to the frontend for a single server (includes live status).
#[derive(Debug, Serialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub status: String,
    pub tool_count: usize,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct McpServerInput {
    pub id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub url: Option<String>,
}

#[tauri::command]
pub async fn list_mcp_servers(
    config: State<'_, Arc<ConfigStore>>,
    mcp: State<'_, McpManagerState>,
) -> Result<Vec<McpServerInfo>, String> {
    let cfg = config.get();
    let manager = mcp.lock().await;
    let statuses: std::collections::HashMap<_, _> = manager
        .list_server_statuses()
        .into_iter()
        .collect();

    let infos = cfg.mcp_servers.iter().map(|s| {
        let (status_str, tool_count, error_message) = match statuses.get(&s.id) {
            Some(crate::mcp::types::McpServerStatus::Connected { tool_count }) =>
                ("connected".into(), *tool_count, None),
            Some(crate::mcp::types::McpServerStatus::Connecting) =>
                ("connecting".into(), 0, None),
            Some(crate::mcp::types::McpServerStatus::Error { message }) =>
                ("error".into(), 0, Some(message.clone())),
            Some(crate::mcp::types::McpServerStatus::Disabled) | None =>
                ("disabled".into(), 0, None),
        };
        McpServerInfo {
            id: s.id.clone(),
            name: s.name.clone(),
            enabled: s.enabled,
            transport: s.transport,
            command: s.command.clone(),
            args: s.args.clone(),
            url: s.url.clone(),
            status: status_str,
            tool_count,
            error_message,
        }
    }).collect();

    Ok(infos)
}

#[tauri::command]
pub async fn add_mcp_server(
    input: McpServerInput,
    config: State<'_, Arc<ConfigStore>>,
    mcp: State<'_, McpManagerState>,
) -> Result<(), String> {
    let id = input.id.as_deref().map(String::from).unwrap_or_else(|| {
        input.name.to_lowercase().replace(|c: char| !c.is_alphanumeric(), "-")
    });

    // Check for duplicate
    if config.get().mcp_servers.iter().any(|s| s.id == id) {
        return Err(format!("MCP server with id '{id}' already exists"));
    }

    let server_cfg = McpServerConfig {
        id: id.clone(),
        name: input.name,
        enabled: input.enabled,
        transport: input.transport,
        command: input.command,
        args: input.args,
        env: input.env,
        url: input.url,
    };

    config.update(|cfg| {
        cfg.mcp_servers.push(server_cfg.clone());
    }).map_err(|e| e.to_string())?;

    if server_cfg.enabled {
        let mut manager = mcp.lock().await;
        manager.reconnect(&server_cfg).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn update_mcp_server(
    input: McpServerInput,
    config: State<'_, Arc<ConfigStore>>,
    mcp: State<'_, McpManagerState>,
) -> Result<(), String> {
    let id = input.id.as_deref()
        .ok_or("id is required for update")?
        .to_string();

    // Verify server exists
    if !config.get().mcp_servers.iter().any(|s| s.id == id) {
        return Err(format!("MCP server '{id}' not found"));
    }

    let server_cfg = McpServerConfig {
        id: id.clone(),
        name: input.name,
        enabled: input.enabled,
        transport: input.transport,
        command: input.command,
        args: input.args,
        env: input.env,
        url: input.url,
    };

    config.update(|cfg| {
        if let Some(pos) = cfg.mcp_servers.iter().position(|s| s.id == id) {
            cfg.mcp_servers[pos] = server_cfg.clone();
        }
    }).map_err(|e| e.to_string())?;

    let mut manager = mcp.lock().await;
    manager.remove_server(&id);
    if server_cfg.enabled {
        manager.reconnect(&server_cfg).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn remove_mcp_server(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    mcp: State<'_, McpManagerState>,
) -> Result<(), String> {
    config.update(|cfg| {
        cfg.mcp_servers.retain(|s| s.id != id);
    }).map_err(|e| e.to_string())?;

    mcp.lock().await.remove_server(&id);
    Ok(())
}

#[tauri::command]
pub async fn get_mcp_tools(
    mcp: State<'_, McpManagerState>,
) -> Result<Vec<McpToolInfo>, String> {
    Ok(mcp.lock().await.list_tool_infos())
}

#[tauri::command]
pub async fn execute_mcp_tool(
    encoded_name: String,
    args: serde_json::Value,
    mcp: State<'_, McpManagerState>,
) -> Result<McpToolResult, String> {
    mcp.lock().await
        .call_tool(&encoded_name, args)
        .await
        .map_err(|e| e.to_string())
}

/// Read the Claude Desktop config and return importable server configs.
#[tauri::command]
pub async fn import_claude_desktop_mcp() -> Result<Vec<McpServerConfig>, String> {
    let path = claude_desktop_config_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|_| "找不到 Claude Desktop 設定檔".to_string())?;

    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("無法解析 Claude Desktop 設定: {e}"))?;

    let servers = json["mcpServers"].as_object()
        .ok_or_else(|| "Claude Desktop 設定中找不到 mcpServers".to_string())?;

    let configs = servers.iter().map(|(name, cfg)| {
        McpServerConfig {
            id: name.clone(),
            name: name.clone(),
            enabled: true,
            transport: crate::config::types::McpTransport::Stdio,
            command: cfg["command"].as_str().map(String::from),
            args: cfg["args"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect())
                .unwrap_or_default(),
            env: cfg["env"].as_object()
                .map(|obj| obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect())
                .unwrap_or_default(),
            url: None,
        }
    }).collect();

    Ok(configs)
}

#[tauri::command]
pub async fn set_mcp_enabled(
    enabled: bool,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    config.update(|cfg| { cfg.mcp_enabled = enabled; }).map_err(|e| e.to_string())
}

// ── Platform-specific Claude Desktop config path ──────────────────────────────

#[cfg(target_os = "macos")]
fn claude_desktop_config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/Claude/claude_desktop_config.json")
}

#[cfg(target_os = "windows")]
fn claude_desktop_config_path() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("Claude/claude_desktop_config.json")
}

#[cfg(target_os = "linux")]
fn claude_desktop_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("Claude/claude_desktop_config.json")
}

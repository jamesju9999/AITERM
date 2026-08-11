// src-tauri/src/commands/mcp.rs

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

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
    /// Only the env var keys are exposed (not values) for security.
    pub env_keys: Vec<String>,
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
            env_keys: s.env.keys().cloned().collect(),
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

    // If env is empty, preserve the existing env vars (env is not sent back to
    // the frontend for security, so an empty map means "unchanged").
    let existing_env = config.get()
        .mcp_servers.iter()
        .find(|s| s.id == id)
        .map(|s| s.env.clone())
        .unwrap_or_default();
    let env = if input.env.is_empty() { existing_env } else { input.env };

    let server_cfg = McpServerConfig {
        id: id.clone(),
        name: input.name,
        enabled: input.enabled,
        transport: input.transport,
        command: input.command,
        args: input.args,
        env,
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
    // 兩段式：先短暫鎖住 manager 查出是哪個連線，**放掉鎖之後**才送請求。
    //
    // 曾經寫成 `mcp.lock().await.call_tool(...).await`，那個 MutexGuard 會活到
    // 整個陳述式結束——也就是整個工具呼叫期間都握著 manager 的鎖。只要有一次
    // 呼叫卡住不回（server 沒回應、stdio 塞住），`list_mcp_servers` 就再也拿
    // 不到鎖，設定頁的 MCP 清單永遠讀不出來，看起來像「設定被刪了」。
    let (transport, raw_name) = {
        let manager = mcp.lock().await;
        manager.resolve_tool(&encoded_name).map_err(|e| e.to_string())?
    };
    crate::mcp::McpManager::call_tool_on(transport, &raw_name, args)
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

// ── MCP package installation ──────────────────────────────────────────────────

#[derive(Clone, serde::Serialize)]
pub struct McpInstallLogEvent {
    pub session_id: String,
    pub line: String,
    pub is_error: bool,
    pub done: bool,
    pub success: bool,
}

#[tauri::command]
pub async fn install_mcp_package(
    app: tauri::AppHandle,
    command: String,
    args: Vec<String>,
    session_id: String,
) -> Result<(), String> {
    // On Windows: wrap in `cmd /C` so .cmd scripts (npx, pip) resolve correctly.
    // On macOS: wrap in `/bin/zsh -l -c` so login PATH (Homebrew, nvm) is inherited.
    // On Linux: spawn directly.
    #[cfg(windows)]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(&command).args(&args);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut parts = vec![command.clone()];
        parts.extend_from_slice(&args);
        let shell_cmd = shell_words::join(&parts);
        let mut c = tokio::process::Command::new("/bin/zsh");
        c.args(["-l", "-c", &shell_cmd]);
        c
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut cmd = {
        let mut c = tokio::process::Command::new(&command);
        c.args(&args);
        c
    };

    cmd.stdout(std::process::Stdio::piped())
       .stderr(std::process::Stdio::piped())
       .kill_on_drop(true); // Fix 1: kill child if this future is dropped

    // Fix 3: On Windows, suppress the console window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let mut child = cmd.spawn()
        .map_err(|e| {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                format!("command not found: {command}")
            } else {
                e.to_string()
            };
            let _ = app.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id.clone(),
                    line: msg.clone(),
                    is_error: true,
                    done: true,
                    success: false,
                },
            );
            msg
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let session_id_out = session_id.clone();
    let app_out = app.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_out.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id_out.clone(),
                    line,
                    is_error: false,
                    done: false,
                    success: false,
                },
            );
        }
    });

    let session_id_err = session_id.clone();
    let app_err = app.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app_err.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id_err.clone(),
                    line,
                    is_error: true,
                    done: false,
                    success: false,
                },
            );
        }
    });

    let result = timeout(Duration::from_secs(60), child.wait()).await;

    match result {
        Ok(Ok(status)) => {
            let _ = out_task.await;
            let _ = err_task.await;
            if status.success() {
                let _ = app.emit(
                    "mcp-install-log",
                    McpInstallLogEvent {
                        session_id: session_id.clone(),
                        line: String::new(),
                        is_error: false,
                        done: true,
                        success: true,
                    },
                );
                Ok(())
            } else {
                let msg = format!("process exited with code {}", status.code().unwrap_or(-1));
                let _ = app.emit(
                    "mcp-install-log",
                    McpInstallLogEvent {
                        session_id: session_id.clone(),
                        line: msg.clone(),
                        is_error: true,
                        done: true,
                        success: false,
                    },
                );
                Err(msg)
            }
        }
        Ok(Err(e)) => {
            let _ = out_task.await;
            let _ = err_task.await;
            let _ = app.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id.clone(),
                    line: e.to_string(),
                    is_error: true,
                    done: true,
                    success: false,
                },
            );
            Err(e.to_string())
        }
        Err(_) => {
            // Fix 1+2: Kill the process and abort I/O tasks on timeout.
            let _ = child.kill().await;
            out_task.abort();
            err_task.abort();
            let msg = "install timed out after 60 seconds".to_string();
            let _ = app.emit(
                "mcp-install-log",
                McpInstallLogEvent {
                    session_id: session_id.clone(),
                    line: msg.clone(),
                    is_error: true,
                    done: true,
                    success: false,
                },
            );
            Err(msg)
        }
    }
}

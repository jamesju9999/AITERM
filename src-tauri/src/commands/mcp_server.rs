//! Tauri commands for the MCP tool server settings page. Mirrors
//! `commands/bridge.rs` exactly (same shape: status/apply/set_config).

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::bridge::auth as bridge_auth;
use crate::config::types::McpToolServerConfig;
use crate::config::ConfigStore;
use crate::mcp_server::{McpToolServerState, MCP_TOOL_SERVER_TOKEN_KEY};
use crate::secret::SecretStore;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolServerStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub token: Option<String>,
    pub error: Option<String>,
}

fn ensure_token(secrets: &Arc<SecretStore>) -> anyhow::Result<String> {
    if let Some(t) = secrets.get(MCP_TOOL_SERVER_TOKEN_KEY)? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let t = bridge_auth::generate_token();
    secrets.set(MCP_TOOL_SERVER_TOKEN_KEY, &t)?;
    Ok(t)
}

#[tauri::command]
pub async fn mcp_tool_server_status(
    server: State<'_, Arc<McpToolServerState>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<McpToolServerStatus, String> {
    let port = server.port();
    let token = if port.is_some() {
        secrets.get(MCP_TOOL_SERVER_TOKEN_KEY).ok().flatten()
    } else {
        None
    };
    Ok(McpToolServerStatus { running: port.is_some(), port, token, error: None })
}

/// Starts or stops the server according to the currently saved config. Called
/// after the settings page saves.
#[tauri::command]
pub async fn mcp_tool_server_apply(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<McpToolServerStatus, String> {
    let cfg = config.get().mcp_tool_server;
    if !cfg.enabled {
        server.stop();
        return Ok(McpToolServerStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match server
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port)
        .await
    {
        Ok(()) => Ok(McpToolServerStatus {
            running: true,
            port: Some(cfg.port),
            token: Some(token),
            error: None,
        }),
        Err(e) => Ok(McpToolServerStatus {
            running: false,
            port: None,
            token: None,
            error: Some(e.to_string()),
        }),
    }
}

/// Saves the config and immediately applies it (starts/stops as needed).
#[tauri::command]
pub async fn mcp_tool_server_set_config(
    server: State<'_, Arc<McpToolServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: McpToolServerConfig,
) -> Result<McpToolServerStatus, String> {
    config
        .update(|c| c.mcp_tool_server = value.clone())
        .map_err(|e| e.to_string())?;
    mcp_tool_server_apply(server, config, secrets).await
}

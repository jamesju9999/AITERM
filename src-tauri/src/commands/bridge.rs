//! Claude Code 橋接的前端指令。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::bridge::{auth, BridgeState};
use crate::config::types::ClaudeBridgeConfig;
use crate::config::ConfigStore;
use crate::secret::SecretStore;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    pub running: bool,
    pub port: Option<u16>,
    /// 已設定的 token（給「複製手動命令」用）。未啟用時為 None。
    pub token: Option<String>,
    pub error: Option<String>,
}

/// 取得（必要時產生）橋接 token。
fn ensure_token(secrets: &Arc<SecretStore>) -> anyhow::Result<String> {
    if let Some(t) = secrets.get(auth::BRIDGE_TOKEN_KEY)? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let t = auth::generate_token();
    secrets.set(auth::BRIDGE_TOKEN_KEY, &t)?;
    Ok(t)
}

#[tauri::command]
pub async fn bridge_status(
    bridge: State<'_, Arc<BridgeState>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<BridgeStatus, String> {
    let port = bridge.port();
    let token = if port.is_some() {
        secrets.get(auth::BRIDGE_TOKEN_KEY).ok().flatten()
    } else {
        None
    };
    Ok(BridgeStatus { running: port.is_some(), port, token, error: None })
}

/// 依目前 config 啟動或停止 server。設定頁存檔後呼叫。
#[tauri::command]
pub async fn bridge_apply(
    bridge: State<'_, Arc<BridgeState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<BridgeStatus, String> {
    let cfg = config.get().claude_bridge;
    if !cfg.enabled {
        bridge.stop();
        return Ok(BridgeStatus { running: false, port: None, token: None, error: None });
    }

    let token = ensure_token(&secrets).map_err(|e| e.to_string())?;
    match bridge
        .start(config.inner().clone(), secrets.inner().clone(), token.clone(), cfg.port)
        .await
    {
        Ok(()) => Ok(BridgeStatus {
            running: true,
            port: Some(cfg.port),
            token: Some(token),
            error: None,
        }),
        Err(e) => Ok(BridgeStatus {
            running: false,
            port: None,
            token: None,
            // 回成 status.error 而非 Err：埠被占用是使用者要處理的狀態，
            // 不是程式錯誤，UI 要能把它顯示在區塊裡。
            error: Some(e.to_string()),
        }),
    }
}

/// 存下橋接設定並立刻套用（啟動或停止 server）。
#[tauri::command]
pub async fn bridge_set_config(
    bridge: State<'_, Arc<BridgeState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: ClaudeBridgeConfig,
) -> Result<BridgeStatus, String> {
    config
        .update(|c| c.claude_bridge = value.clone())
        .map_err(|e| e.to_string())?;
    bridge_apply(bridge, config, secrets).await
}

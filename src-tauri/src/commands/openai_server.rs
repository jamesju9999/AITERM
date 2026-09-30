//! OpenAI 相容 server 的前端指令。結構仿 `commands/bridge.rs`。

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::bridge::openai_server::{validate, OpenAiServerState};
use crate::bridge::auth;
use crate::config::types::OpenAiServerConfig;
use crate::config::ConfigStore;
use crate::secret::SecretStore;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiServerStatus {
    pub running: bool,
    pub port: Option<u16>,
    /// 目前是否綁在全介面（區網可連）。
    pub lan: bool,
    /// 設定頁要顯示／複製的 API key。key 本來就要給使用者貼進外部工具，所以
    /// 只要 server 有啟用就回傳。
    pub token: Option<String>,
    pub error: Option<String>,
}

fn stopped() -> OpenAiServerStatus {
    OpenAiServerStatus { running: false, port: None, lan: false, token: None, error: None }
}

/// 取得（必要時產生）API key。
fn ensure_token(secrets: &Arc<SecretStore>) -> anyhow::Result<String> {
    if let Some(t) = secrets.get(auth::OPENAI_SERVER_TOKEN_KEY)? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let t = auth::generate_token();
    secrets.set(auth::OPENAI_SERVER_TOKEN_KEY, &t)?;
    Ok(t)
}

#[tauri::command]
pub async fn openai_server_status(
    server: State<'_, Arc<OpenAiServerState>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<OpenAiServerStatus, String> {
    let port = server.port();
    let token = if port.is_some() {
        secrets.get(auth::OPENAI_SERVER_TOKEN_KEY).ok().flatten()
    } else {
        None
    };
    Ok(OpenAiServerStatus { running: port.is_some(), port, lan: server.lan(), token, error: None })
}

async fn apply_inner(
    server: &Arc<OpenAiServerState>,
    config: &Arc<ConfigStore>,
    secrets: &Arc<SecretStore>,
) -> Result<OpenAiServerStatus, String> {
    let cfg = config.get().openai_server;
    if !cfg.enabled {
        server.stop();
        return Ok(stopped());
    }
    let token = ensure_token(secrets).map_err(|e| e.to_string())?;
    match server
        .start(config.clone(), secrets.clone(), token.clone(), cfg.port, cfg.allow_lan)
        .await
    {
        Ok(()) => Ok(OpenAiServerStatus {
            running: true,
            port: Some(cfg.port),
            lan: cfg.allow_lan,
            token: Some(token),
            error: None,
        }),
        // 回成 status.error 而非 Err：埠被占用是使用者要處理的狀態，不是程式錯誤。
        Err(e) => Ok(OpenAiServerStatus { error: Some(e.to_string()), ..stopped() }),
    }
}

/// 依目前 config 啟動或停止 server。
#[tauri::command]
pub async fn openai_server_apply(
    server: State<'_, Arc<OpenAiServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<OpenAiServerStatus, String> {
    apply_inner(server.inner(), config.inner(), secrets.inner()).await
}

/// 驗證、存下設定並立刻套用。驗證失敗回 `Err`，前端直接顯示，不動現有設定。
#[tauri::command]
pub async fn openai_server_set_config(
    server: State<'_, Arc<OpenAiServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    value: OpenAiServerConfig,
) -> Result<OpenAiServerStatus, String> {
    validate(&value)?;
    config
        .update(|c| c.openai_server = value.clone())
        .map_err(|e| e.to_string())?;
    apply_inner(server.inner(), config.inner(), secrets.inner()).await
}

/// 產生新的 API key，舊 key 立即失效；server 在跑就重啟套用。
#[tauri::command]
pub async fn openai_server_regenerate_key(
    server: State<'_, Arc<OpenAiServerState>>,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<OpenAiServerStatus, String> {
    secrets
        .set(auth::OPENAI_SERVER_TOKEN_KEY, &auth::generate_token())
        .map_err(|e| e.to_string())?;
    apply_inner(server.inner(), config.inner(), secrets.inner()).await
}

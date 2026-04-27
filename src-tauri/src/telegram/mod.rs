use std::sync::Arc;
use tauri::{AppHandle, Manager, Emitter};
use serde::{Deserialize, Serialize};
use log::{info, warn, error};

use crate::config::ConfigStore;
use crate::secret::SecretStore;

const TELEGRAM_SECRET_ID: &str = "telegram_bot_token";

#[derive(Debug, Serialize, Deserialize)]
pub struct TelegramConfigData {
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    #[allow(dead_code)]
    message_id: i64,
    chat: TelegramChat,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelegramMessagePayload {
    pub text: String,
}

pub struct TelegramState {
    pub active_task: Option<tokio::task::JoinHandle<()>>,
}

/// Initialize Telegram background polling if configured.
pub fn init(app: &AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        restart_polling(&app_handle).await;
    });
}

async fn restart_polling(app: &AppHandle) {
    let state = app.state::<tokio::sync::Mutex<TelegramState>>();
    let mut state_guard = state.lock().await;

    // Stop existing task
    if let Some(task) = state_guard.active_task.take() {
        task.abort();
    }

    let config_store = app.state::<Arc<ConfigStore>>();
    let secret_store = app.state::<Arc<SecretStore>>();

    let chat_id = config_store.get().telegram_chat_id;
    let bot_token = secret_store.get(TELEGRAM_SECRET_ID).ok().flatten();

    if let (Some(token), Some(chat_id_str)) = (bot_token, chat_id) {
        if let Ok(allowed_chat_id) = chat_id_str.parse::<i64>() {
            let app_clone = app.clone();
            let task = tokio::spawn(async move {
                poll_updates(app_clone, token, allowed_chat_id).await;
            });
            state_guard.active_task = Some(task);
            info!("Telegram polling task started for chat ID {}", allowed_chat_id);
        }
    }
}

async fn poll_updates(app: AppHandle, token: String, allowed_chat_id: i64) {
    let client = reqwest::Client::new();
    let mut offset = 0i64;
    let base_url = format!("https://api.telegram.org/bot{}", token);

    loop {
        let url = format!("{}/getUpdates?offset={}&timeout=30", base_url, offset);
        match client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<TelegramUpdateResponse>().await {
                    if data.ok {
                        for update in data.result {
                            offset = offset.max(update.update_id + 1);
                            if let Some(msg) = update.message {
                                if msg.chat.id == allowed_chat_id {
                                    if let Some(text) = msg.text {
                                        let payload = TelegramMessagePayload { text };
                                        let _ = app.emit("telegram-message-received", payload);
                                    }
                                } else {
                                    warn!("Received message from unauthorized chat ID: {}", msg.chat.id);
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Telegram polling error: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }
}

#[tauri::command]
pub async fn telegram_get_config(
    config_store: tauri::State<'_, Arc<ConfigStore>>,
    secret_store: tauri::State<'_, Arc<SecretStore>>,
) -> Result<TelegramConfigData, String> {
    let bot_token = secret_store.get(TELEGRAM_SECRET_ID).unwrap_or(None);
    let chat_id = config_store.get().telegram_chat_id;
    Ok(TelegramConfigData { bot_token, chat_id })
}

#[tauri::command]
pub async fn telegram_set_config(
    app: AppHandle,
    config: TelegramConfigData,
    config_store: tauri::State<'_, Arc<ConfigStore>>,
    secret_store: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    if let Some(ref token) = config.bot_token {
        if token.trim().is_empty() {
            let _ = secret_store.delete(TELEGRAM_SECRET_ID);
        } else {
            let _ = secret_store.set(TELEGRAM_SECRET_ID, token);
        }
    }

    config_store.update(|cfg| {
        cfg.telegram_chat_id = config.chat_id;
    }).map_err(|e| e.to_string())?;

    // Restart polling with new config
    restart_polling(&app).await;

    Ok(())
}

#[tauri::command]
pub async fn telegram_send_message(
    text: String,
    config_store: tauri::State<'_, Arc<ConfigStore>>,
    secret_store: tauri::State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let bot_token = secret_store.get(TELEGRAM_SECRET_ID).map_err(|e| e.to_string())?.ok_or("No bot token configured")?;
    let chat_id = config_store.get().telegram_chat_id.ok_or("No chat ID configured")?;

    let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
    let client = reqwest::Client::new();
    let res = client.post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("Failed to send message: {}", res.status()));
    }

    Ok(())
}

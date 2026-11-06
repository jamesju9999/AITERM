use std::sync::Arc;
use tauri::{AppHandle, Manager, Emitter};
use serde::{Deserialize, Serialize};
use log::{info, warn, error};

use crate::config::ConfigStore;
use crate::secret::SecretStore;

pub mod questions;
pub use questions::mark_question_closed;

pub(crate) const TELEGRAM_SECRET_ID: &str = "telegram_bot_token";

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
    /// 按下訊息上的按鈕（Agent 問題的選項）。
    callback_query: Option<TelegramCallback>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
    /// 發話的人（群組或私訊皆有）；用來顯示提問者姓名。
    from: Option<TelegramUser>,
    text: Option<String>,
    /// 圖片訊息的說明文字（問題內容寫在這裡）。
    caption: Option<String>,
    /// 同一張圖會有多種尺寸，取最大的。
    photo: Option<Vec<TelegramPhoto>>,
    /// 以「檔案」形式傳的圖片。
    document: Option<TelegramDocument>,
    /// 使用者「回覆」某則訊息時，被回覆的那則（用來對應到 Agent 的問題）。
    reply_to_message: Option<Box<TelegramMessage>>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramPhoto {
    file_id: String,
    file_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TelegramDocument {
    file_id: String,
    /// 使用者傳的原始檔名（log、pdf…）；下載後沿用它當附件名稱。
    file_name: Option<String>,
}

impl TelegramMessage {
    /// 發話者的顯示名稱：姓名優先，沒有就用 @username。
    fn sender_display(&self) -> String {
        let Some(u) = &self.from else { return String::new() };
        let full = [u.first_name.as_deref(), u.last_name.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        if !full.trim().is_empty() {
            full
        } else {
            u.username.as_deref().map(|n| format!("@{n}")).unwrap_or_default()
        }
    }

    /// 訊息文字：一般訊息用 text，圖片訊息用 caption。
    fn body_text(&self) -> Option<&str> {
        self.text.as_deref().or(self.caption.as_deref())
    }

    /// 這則訊息帶的附件 `(file_id, 檔名)`：最大尺寸的 photo，以及以「檔案」形式傳的任何文件（log、pdf…）。
    fn attachment_ids(&self) -> Vec<(String, String)> {
        let mut v = Vec::new();
        if let Some(best) = self.photo.as_ref().and_then(|p| p.iter().max_by_key(|x| x.file_size.unwrap_or(0))) {
            v.push((best.file_id.clone(), String::new()));
        }
        if let Some(d) = &self.document {
            v.push((d.file_id.clone(), d.file_name.clone().unwrap_or_default()));
        }
        v
    }
}

#[derive(Debug, Deserialize)]
struct TelegramCallback {
    id: String,
    data: Option<String>,
    message: Option<TelegramMessage>,
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
        // 明確要求按鈕事件：之前若有人用 allowed_updates 限縮過，Telegram 會一直沿用。
        let url = format!(
            "{}/getUpdates?offset={}&timeout=30&allowed_updates=%5B%22message%22%2C%22callback_query%22%5D",
            base_url, offset
        );
        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();

                // 409 means another process is long-polling the same bot token.
                // Telegram only allows one getUpdates connection per bot at a time.
                if status == reqwest::StatusCode::CONFLICT {
                    error!(
                        "Another instance is already polling this bot (HTTP 409). \
                         Close other AITerm windows / processes that share this bot token, \
                         or remove any webhook via deleteWebhook."
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    continue;
                }

                match resp.json::<TelegramUpdateResponse>().await {
                    Ok(data) => {
                        if !data.ok {
                            warn!("Telegram API returned ok=false (status {})", status);
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            continue;
                        }
                        for update in data.result {
                            offset = offset.max(update.update_id + 1);
                            // Agent 問題的選項按鈕。
                            if let Some(cb) = update.callback_query {
                                let in_chat = cb.message.as_ref().is_some_and(|m| m.chat.id == allowed_chat_id);
                                let (app, token) = (app.clone(), token.clone());
                                tauri::async_runtime::spawn(async move {
                                    let reply = match (in_chat, cb.data.as_deref()) {
                                        (true, Some(data)) => match crate::commands::questions::answer_from_telegram_button(&app, data).await {
                                            Ok(q) => format!("已回答 {}", q.label()),
                                            Err(e) => e,
                                        },
                                        _ => "無法處理".to_string(),
                                    };
                                    questions::ack_callback(&token, &cb.id, &reply).await;
                                });
                                continue;
                            }
                            if let Some(msg) = update.message {
                                if msg.chat.id == allowed_chat_id {
                                    // 「回覆」Agent 的問題訊息＝回答那一題；不是問題訊息就照舊送到作用中的分頁。
                                    if let (Some(replied), Some(text)) = (msg.reply_to_message.as_ref(), msg.text.as_ref()) {
                                        match crate::commands::questions::answer_from_telegram_reply(&app, replied.message_id, text).await {
                                            Ok(Some(_)) => continue,
                                            Ok(None) => {}
                                            Err(e) => {
                                                warn!("telegram answer: {e}");
                                                continue;
                                            }
                                        }
                                    }
                                    // 問題收集：主人用 `/ask ` 開頭的訊息進佇列，不當一般對話轉送。
                                    {
                                        let photos = msg.attachment_ids();
                                        if msg.body_text().is_some() || !photos.is_empty() {
                                            let text = msg.body_text().unwrap_or("");
                                            if crate::intake::on_telegram(&app, msg.chat.id, msg.message_id, text, true, photos, &msg.sender_display()).await {
                                                continue;
                                            }
                                        }
                                    }
                                    if let Some(text) = msg.text {
                                        let payload = TelegramMessagePayload { text };
                                        if let Err(e) = app.emit("telegram-message-received", payload) {
                                            error!("Failed to emit telegram-message-received: {}", e);
                                        }
                                    }
                                } else {
                                    // 白名單內的其他聊天室可以送問題進問題收集；其餘照舊只記警告。
                                    {
                                        let photos = msg.attachment_ids();
                                        if msg.body_text().is_some() || !photos.is_empty() {
                                            let text = msg.body_text().unwrap_or("");
                                            if crate::intake::on_telegram(&app, msg.chat.id, msg.message_id, text, false, photos, &msg.sender_display()).await {
                                                continue;
                                            }
                                        }
                                    }
                                    warn!("Received message from unauthorized chat ID: {} (allowed: {})", msg.chat.id, allowed_chat_id);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse Telegram response (status {}): {}", status, e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }
                }
            }
            Err(e) => {
                error!("Telegram polling HTTP error: {}", e);
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

/// 回覆到指定聊天室（可選擇回覆某則訊息）。過長的內容會切成多則。
pub async fn send_to_chat(token: &str, chat_id: i64, text: &str, reply_to: Option<i64>) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let client = reqwest::Client::new();
    for (i, part) in crate::intake::telegram_source::split_message(text, 4000).into_iter().enumerate() {
        let mut body = serde_json::json!({ "chat_id": chat_id, "text": part });
        if i == 0 {
            if let Some(id) = reply_to {
                body["reply_parameters"] = serde_json::json!({ "message_id": id, "allow_sending_without_reply": true });
            }
        }
        let res = client.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("Telegram 回覆失敗：HTTP {}", res.status()));
        }
    }
    Ok(())
}

/// 下載 Telegram 上的檔案（先 getFile 取路徑，再下載）。超過 `max_bytes` 視為失敗。
pub async fn download_file(token: &str, file_id: &str, name: &str, max_bytes: usize) -> Result<crate::intake::attach::Raw, String> {
    let client = reqwest::Client::new();
    let info: serde_json::Value = client
        .get(format!("https://api.telegram.org/bot{token}/getFile"))
        .query(&[("file_id", file_id)])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let path = info.pointer("/result/file_path").and_then(|v| v.as_str()).ok_or("getFile 沒有回傳 file_path")?.to_string();
    if info.pointer("/result/file_size").and_then(|v| v.as_u64()).is_some_and(|n| n as usize > max_bytes) {
        return Err("檔案太大".into());
    }
    let bytes = client
        .get(format!("https://api.telegram.org/file/bot{token}/{path}"))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > max_bytes {
        return Err("檔案太大".into());
    }
    let name = if name.trim().is_empty() { path.rsplit('/').next().unwrap_or("file").to_string() } else { name.to_string() };
    Ok(crate::intake::attach::Raw { name, bytes: bytes.to_vec() })
}

#[cfg(test)]
mod sender_display_tests {
    use super::*;

    fn msg(json: &str) -> TelegramMessage {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn name_comes_from_first_and_last_name_then_username_then_nothing() {
        let both = msg(r#"{"message_id":1,"chat":{"id":5},"from":{"first_name":"小明","last_name":"王","username":"ming"}}"#);
        assert_eq!(both.sender_display(), "小明 王");
        let user_only = msg(r#"{"message_id":1,"chat":{"id":5},"from":{"username":"ming"}}"#);
        assert_eq!(user_only.sender_display(), "@ming");
        let none = msg(r#"{"message_id":1,"chat":{"id":5}}"#);
        assert_eq!(none.sender_display(), "");
    }

    #[test]
    fn photos_and_documents_of_any_type_are_collected_with_their_names() {
        let m = msg(r#"{"message_id":1,"chat":{"id":5},"photo":[{"file_id":"small","file_size":10},{"file_id":"big","file_size":900}],"document":{"file_id":"doc1","file_name":"server.log","mime_type":"text/plain"}}"#);
        assert_eq!(m.attachment_ids(), vec![("big".to_string(), String::new()), ("doc1".to_string(), "server.log".to_string())]);
        let pdf = msg(r#"{"message_id":2,"chat":{"id":5},"document":{"file_id":"d2","file_name":"報告.pdf","mime_type":"application/pdf"}}"#);
        assert_eq!(pdf.attachment_ids(), vec![("d2".to_string(), "報告.pdf".to_string())], "不再只收圖片");
        assert!(msg(r#"{"message_id":3,"chat":{"id":5},"text":"hi"}"#).attachment_ids().is_empty());
    }
}

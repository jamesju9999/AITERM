//! Server 的 Telegram：用機器人長輪詢（getUpdates）收訊息，規則與桌面版相同（`/ask` 主人、白名單聊天室），
//! 並用同一個機器人回覆。機器人金鑰放祕密庫（`telegram_bot_token`），不隨設定回傳。
//!
//! 同一個機器人同一時間只能有一個程式在輪詢（Telegram 會回 409）：給 server 用的機器人，
//! 不要同時設在任何一台桌面版 AITerm 的 Telegram 設定裡。
use crate::state::AppState;
use aiterm_intake_core::attach::{self, Raw};
use aiterm_intake_core::host::{Notifier, Secrets};
use aiterm_intake_core::queue::Ingested;
use aiterm_intake_core::telegram_source::{self, Candidate};
use aiterm_intake_core::{config, store};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

pub const TOKEN_SECRET: &str = "telegram_bot_token";
pub const SETTING_KEY: &str = "server_telegram";
pub const API_BASE: &str = "https://api.telegram.org";
const POLL_TIMEOUT_SECS: u64 = 25;

/// server 的 Telegram 設定。主人聊天室＝可以用 `/ask 問題` 直接提問的那一個（通常是管理員自己）。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct TelegramSettings {
    /// 主人的聊天室 id；空字串＝沒有主人，只有白名單內的聊天室能提問。
    pub owner_chat_id: String,
}

pub async fn load(pool: &sqlx::SqlitePool) -> TelegramSettings {
    store::get_setting(pool, SETTING_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub async fn save(pool: &sqlx::SqlitePool, s: &TelegramSettings) -> Result<(), String> {
    let id = s.owner_chat_id.trim();
    if !id.is_empty() && id.parse::<i64>().is_err() {
        return Err("主人聊天室 id 必須是數字（群組是負數）".into());
    }
    let clean = TelegramSettings { owner_chat_id: id.to_string() };
    store::set_setting(pool, SETTING_KEY, &serde_json::to_string(&clean).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// 目前的連線狀態，給 `status` 顯示。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct Health {
    pub last_ok: Option<i64>,
    pub error: Option<String>,
}

// ---- Telegram 的訊息格式（只取用得到的欄位） ----

#[derive(Debug, Deserialize)]
struct UpdatesResponse {
    ok: bool,
    #[serde(default)]
    result: Vec<Update>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    message_id: i64,
    chat: Chat,
    from: Option<User>,
    text: Option<String>,
    caption: Option<String>,
    photo: Option<Vec<Photo>>,
    document: Option<Document>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct User {
    first_name: Option<String>,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Photo {
    file_id: String,
    file_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Document {
    file_id: String,
    file_name: Option<String>,
}

impl Message {
    fn sender_display(&self) -> String {
        let Some(u) = &self.from else { return String::new() };
        let full = [u.first_name.as_deref(), u.last_name.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        if !full.trim().is_empty() { full } else { u.username.as_deref().map(|n| format!("@{n}")).unwrap_or_default() }
    }

    fn body_text(&self) -> Option<&str> {
        self.text.as_deref().or(self.caption.as_deref())
    }

    /// 附件 `(file_id, 檔名)`：最大尺寸的圖，以及以檔案形式傳的文件。
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

fn token(st: &AppState) -> Option<String> {
    st.secrets.get(TOKEN_SECRET).filter(|t| !t.trim().is_empty())
}

/// 有機器人金鑰且至少一個專案啟用 Telegram 才輪詢。
async fn candidates(st: &AppState) -> Vec<Candidate> {
    let mut out = Vec::new();
    for p in crate::routes_data::project_list(st).await {
        let cfg = config::load(&st.pools.intake, &p.id).await;
        if cfg.telegram_enabled {
            out.push(Candidate { id: p.id, name: p.name, cfg });
        }
    }
    out
}

/// 是否至少有一個專案啟用了 Telegram（沒有的話 server 不會輪詢，金鑰設了也收不到）。
pub async fn has_enabled_project(st: &AppState) -> bool {
    !candidates(st).await.is_empty()
}

/// 下載 Telegram 上的檔案（先 getFile 取路徑，再下載）。超過 `max_bytes` 視為失敗。
async fn download_file(client: &reqwest::Client, base: &str, token: &str, file_id: &str, name: &str) -> Result<Raw, String> {
    let info: serde_json::Value = client.get(format!("{base}/bot{token}/getFile")).query(&[("file_id", file_id)]).send().await.map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let path = info.pointer("/result/file_path").and_then(|v| v.as_str()).ok_or("getFile 沒有回傳 file_path")?.to_string();
    if info.pointer("/result/file_size").and_then(|v| v.as_u64()).is_some_and(|n| n as usize > attach::MAX_BYTES) {
        return Err("檔案太大".into());
    }
    let bytes = client.get(format!("{base}/file/bot{token}/{path}")).send().await.map_err(|e| e.to_string())?.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() > attach::MAX_BYTES {
        return Err("檔案太大".into());
    }
    let name = if name.trim().is_empty() { path.rsplit('/').next().unwrap_or("file").to_string() } else { name.to_string() };
    Ok(Raw { name, bytes: bytes.to_vec() })
}

/// 處理一則訊息；有開出新題目時回 `true`。
async fn handle(st: &Arc<AppState>, client: &reqwest::Client, base: &str, token_value: &str, cands: &[Candidate], owner: Option<i64>, msg: Message) -> bool {
    let files = msg.attachment_ids();
    let text = msg.body_text().unwrap_or("").to_string();
    if text.is_empty() && files.is_empty() {
        return false;
    }
    let is_owner = owner == Some(msg.chat.id);
    let default_project = config::load_global(&st.pools.intake).await.telegram_default_project;
    let has_files = !files.is_empty();
    let (c2, b2, t2) = (client.clone(), base.to_string(), token_value.to_string());
    // 只有確認寄件者被允許才會呼叫：陌生人不能藉由傳檔讓 server 去下載東西。
    let fetch = move || async move {
        let mut out = Vec::new();
        for (id, name) in files.into_iter().take(attach::MAX_FILES) {
            match download_file(&c2, &b2, &t2, &id, &name).await {
                Ok(raw) => out.push(raw),
                Err(e) => log::warn!("Telegram 附件下載失敗：{e}"),
            }
        }
        out
    };
    let root = st.data_dir.join("attachments");
    match telegram_source::route(&st.pools.intake, cands, &default_project, msg.chat.id, msg.message_id, &text, is_owner, &root, has_files, fetch).await {
        Ok(Ingested::Queued(id)) => {
            store::set_sender_name(&st.pools.intake, &id, &msg.sender_display(), &telegram_source::sender_key(msg.chat.id)).await.ok();
            true
        }
        Ok(_) => false,
        Err(e) => {
            log::warn!("Telegram 入庫失敗：{e}");
            false
        }
    }
}

/// 長輪詢一次。回傳新開出的題目數；沒設定就直接回 0。
pub async fn poll_once(st: &Arc<AppState>, offset: &mut i64) -> Result<usize, String> {
    let Some(token_value) = token(st) else { return Ok(0) };
    let cands = candidates(st).await;
    if cands.is_empty() {
        return Ok(0);
    }
    let owner = load(&st.pools.intake).await.owner_chat_id.trim().parse::<i64>().ok();
    let base = st.telegram_api_base.lock().clone();
    let client = reqwest::Client::builder().timeout(Duration::from_secs(POLL_TIMEOUT_SECS + 15)).build().map_err(|e| e.to_string())?;
    let url = format!("{base}/bot{token_value}/getUpdates?offset={offset}&timeout={POLL_TIMEOUT_SECS}&allowed_updates=%5B%22message%22%5D");
    let resp = client.get(&url).send().await.map_err(|e| format!("連線失敗：{e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::CONFLICT {
        return Err("另一個程式正在輪詢同一個機器人（HTTP 409）：請關掉其他使用這個機器人的 AITerm，或先移除它的 webhook".into());
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("機器人金鑰無效（HTTP 401）".into());
    }
    let data: UpdatesResponse = resp.json().await.map_err(|e| format!("回應不是預期的格式（HTTP {status}）：{e}"))?;
    if !data.ok {
        return Err(format!("Telegram 回傳 ok=false（HTTP {status}）"));
    }
    let mut opened = 0;
    for update in data.result {
        *offset = (*offset).max(update.update_id + 1);
        if let Some(msg) = update.message {
            if handle(st, &client, &base, &token_value, &cands, owner, msg).await {
                opened += 1;
            }
        }
    }
    Ok(opened)
}

/// 持續輪詢；出錯就記錄並稍後重試。
pub async fn run(st: Arc<AppState>) {
    let mut offset = 0i64;
    loop {
        let began = std::time::Instant::now();
        match poll_once(&st, &mut offset).await {
            Ok(n) => {
                // 真的 Telegram 會把請求掛住最多 25 秒；很快就回來又沒東西，等一下避免空轉。
                if n == 0 && began.elapsed() < Duration::from_millis(500) {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                {
                    let mut h = st.channels.telegram_health.lock();
                    if token(&st).is_some() {
                        h.last_ok = Some(crate::db::now());
                    }
                    h.error = None;
                }
                if n > 0 {
                    log::info!("Telegram：收進 {n} 題");
                    st.events.changed();
                }
                // 沒設定時 poll_once 立刻回來：等一下再看，不要空轉。
                if token(&st).is_none() {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
            Err(e) => {
                {
                    let mut h = st.channels.telegram_health.lock();
                    if h.error.as_deref() != Some(e.as_str()) {
                        log::warn!("Telegram 收訊失敗：{e}");
                    }
                    h.error = Some(e);
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
}

/// 回覆到指定聊天室（可回覆某則訊息）；過長的內容切成多則。
pub async fn send_to_chat(base: &str, token: &str, chat_id: i64, text: &str, reply_to: Option<i64>) -> Result<(), String> {
    let url = format!("{base}/bot{token}/sendMessage");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
    for (i, part) in telegram_source::split_message(text, 4000).into_iter().enumerate() {
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

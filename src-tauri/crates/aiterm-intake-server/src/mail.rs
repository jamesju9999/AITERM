//! Server 自己的寄信設定（SMTP）：網頁表單寄驗證碼、用信件回覆網頁表單的題目。
//! 密碼放在祕密庫（`smtp_password`），不隨設定回傳。
use crate::state::AppState;
use aiterm_intake_core::host::Secrets;
use aiterm_intake_core::smtp;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

pub const SETTING_KEY: &str = "server_mail";
pub const PASSWORD_SECRET: &str = "smtp_password";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct MailSettings {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    /// 寄件地址（From）。空字串時用登入帳號。
    pub from: String,
}

impl Default for MailSettings {
    fn default() -> Self {
        Self { smtp_host: String::new(), smtp_port: 587, username: String::new(), from: String::new() }
    }
}

impl MailSettings {
    pub fn sender(&self) -> &str {
        if self.from.trim().is_empty() { self.username.trim() } else { self.from.trim() }
    }
}

pub async fn load(pool: &SqlitePool) -> MailSettings {
    aiterm_intake_core::store::get_setting(pool, SETTING_KEY).await.ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub async fn save(pool: &SqlitePool, s: &MailSettings) -> Result<(), String> {
    if s.smtp_port == 0 {
        return Err("埠號不能是 0".into());
    }
    aiterm_intake_core::store::set_setting(pool, SETTING_KEY, &serde_json::to_string(s).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())
}

/// 主機、帳號、寄件地址與密碼都有才算可寄信。
pub fn configured(s: &MailSettings, secrets: &dyn Secrets) -> bool {
    !s.smtp_host.trim().is_empty() && !s.sender().is_empty() && secrets.get(PASSWORD_SECRET).is_some()
}

/// 測試用：設了以後信不會真的寄出，而是丟進這個通道 `(收件人, 主旨, 內文)`。
pub type MailTap = tokio::sync::mpsc::UnboundedSender<(String, String, String)>;

fn tapped(st: &AppState, to: &str, subject: &str, body: &str) -> bool {
    match st.mail_tap.lock().as_ref() {
        Some(tx) => {
            let _ = tx.send((to.to_string(), subject.to_string(), body.to_string()));
            true
        }
        None => false,
    }
}

async fn settings_and_password(st: &AppState) -> Result<(MailSettings, String), String> {
    let s = load(&st.pools.intake).await;
    let password = st.secrets.get(PASSWORD_SECRET).filter(|p| !p.is_empty()).ok_or("server 尚未設定寄信密碼")?;
    if s.smtp_host.trim().is_empty() || s.sender().is_empty() {
        return Err("server 尚未設定寄信帳號".into());
    }
    Ok((s, password))
}

/// 寄一封一般信（驗證碼）。
pub async fn send_plain(st: &AppState, to: &str, subject: &str, body: &str) -> Result<(), String> {
    if tapped(st, to, subject, body) {
        return Ok(());
    }
    let (s, password) = settings_and_password(st).await?;
    let msg = smtp::build_plain(s.sender(), to, subject, body)?;
    smtp::send(&s.smtp_host, s.smtp_port, s.username.trim(), &password, msg).await
}

/// 回覆網頁表單的題目（沒有可串信的 Message-ID，也沒有其他收件人）。
pub async fn send_reply(st: &AppState, to: &str, orig_subject: &str, body: &str) -> Result<(), String> {
    if tapped(st, to, &smtp::reply_subject(orig_subject), body) {
        return Ok(());
    }
    let (s, password) = settings_and_password(st).await?;
    let msg = smtp::build_reply(&smtp::Outgoing { from: s.sender(), to, orig_subject, orig_message_id: "", also_to: &[], cc: &[], body })?;
    smtp::send(&s.smtp_host, s.smtp_port, s.username.trim(), &password, msg).await
}

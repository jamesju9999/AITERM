// src-tauri/src/commands/mail.rs
use std::sync::Arc;
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::config::{ConfigStore, MailAccountConfig};
use crate::db::mail::{self as mail_db, MailDb, MailMessageRow};
use crate::mail::poller::{mail_secret_key, restart_account, stop_account};
use crate::secret::SecretStore;

#[derive(Debug, Deserialize)]
pub struct MailAccountInput {
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub poll_interval_secs: Option<u32>,
}

#[tauri::command]
pub async fn mail_add_account(
    app: AppHandle,
    input: MailAccountInput,
    config_store: State<'_, Arc<ConfigStore>>,
    secret_store: State<'_, Arc<SecretStore>>,
) -> Result<MailAccountConfig, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let account = MailAccountConfig {
        id: id.clone(),
        email: input.email,
        imap_host: input.imap_host,
        imap_port: input.imap_port,
        smtp_host: input.smtp_host,
        smtp_port: input.smtp_port,
        username: input.username,
        poll_interval_secs: input.poll_interval_secs.unwrap_or(300),
    };

    secret_store.set(&mail_secret_key(&id), &input.password).map_err(|e| e.to_string())?;
    config_store.add_mail_account(account.clone()).map_err(|e| e.to_string())?;
    restart_account(&app, &id).await;

    Ok(account)
}

#[tauri::command]
pub async fn mail_remove_account(
    app: AppHandle,
    id: String,
    config_store: State<'_, Arc<ConfigStore>>,
    secret_store: State<'_, Arc<SecretStore>>,
    mail_db: State<'_, MailDb>,
) -> Result<(), String> {
    stop_account(&app, &id).await;
    config_store.remove_mail_account(&id).map_err(|e| e.to_string())?;
    // Best-effort: config is already removed; ignore Keychain errors so
    // the command doesn't fail if the secret was never stored.
    let _ = secret_store.delete(&mail_secret_key(&id));
    mail_db::delete_account_data(&mail_db.pool, &id).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn mail_list_accounts(config_store: State<'_, Arc<ConfigStore>>) -> Vec<MailAccountConfig> {
    config_store.get().mail_accounts
}

#[tauri::command]
pub async fn mail_list_messages(
    account_id: String,
    mail_db: State<'_, MailDb>,
) -> Result<Vec<MailMessageRow>, String> {
    mail_db::list_messages(&mail_db.pool, &account_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mail_mark_read(
    message_id: String,
    mail_db: State<'_, MailDb>,
) -> Result<(), String> {
    mail_db::mark_read_locally(&mail_db.pool, &message_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mail_count_unread(mail_db: State<'_, MailDb>) -> Result<i64, String> {
    mail_db::count_unread(&mail_db.pool).await.map_err(|e| e.to_string())
}

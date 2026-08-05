// src-tauri/src/mail/poller.rs
use std::sync::Arc;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::ai::router::AiRouter;
use crate::config::{ConfigStore, MailAccountConfig};
use crate::db::mail::{self as mail_db, MailDb, NewMessage};
use crate::secret::SecretStore;

use super::classify::classify_message;
use super::client::fetch_new_messages;
use super::manager::MailState;
use super::parse::parse_raw_message;

/// Keychain key for an account's IMAP/SMTP password. Matches the existing
/// `"{domain}:{id}"` convention (see `commands/db.rs::secret_key`).
pub fn mail_secret_key(account_id: &str) -> String {
    format!("mail:{account_id}")
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MailSyncEvent {
    Summary { account_id: String, message_id: String },
    Important { account_id: String, message_id: String, subject: String, summary: String },
}

pub const MAIL_SYNC_EVENT: &str = "mail-sync-event";

/// Start (or restart) every configured account's polling task. Called once
/// from `lib.rs`'s `.setup()` on app launch.
pub async fn restart_all(app: &AppHandle) {
    let config_store = app.state::<Arc<ConfigStore>>();
    let account_ids: Vec<String> = config_store.get().mail_accounts.into_iter().map(|a| a.id).collect();
    for account_id in account_ids {
        restart_account(app, &account_id).await;
    }
}

/// Abort the existing task for this account (if any) and spawn a fresh one.
/// Called after an account is added, and would be called after a future
/// "edit account" command too (not in Phase 1 scope).
pub async fn restart_account(app: &AppHandle, account_id: &str) {
    let state = app.state::<tokio::sync::Mutex<MailState>>();
    let mut guard = state.lock().await;
    if let Some(task) = guard.tasks.remove(account_id) {
        task.abort();
    }

    let config_store = app.state::<Arc<ConfigStore>>();
    let Some(account) = config_store.get().mail_accounts.into_iter().find(|a| a.id == account_id) else {
        return;
    };

    let app_clone = app.clone();
    let id_for_map = account.id.clone();
    let task = tokio::spawn(async move {
        poll_loop(app_clone, account).await;
    });
    guard.tasks.insert(id_for_map, task);
}

/// Abort an account's task without starting a new one. Called before removing an account.
pub async fn stop_account(app: &AppHandle, account_id: &str) {
    let state = app.state::<tokio::sync::Mutex<MailState>>();
    let mut guard = state.lock().await;
    if let Some(task) = guard.tasks.remove(account_id) {
        task.abort();
    }
}

async fn poll_loop(app: AppHandle, account: MailAccountConfig) {
    loop {
        if let Err(e) = poll_once(&app, &account).await {
            log::warn!("mail poll failed for account {}: {e}", account.id);
        }
        tokio::time::sleep(std::time::Duration::from_secs(account.poll_interval_secs as u64)).await;
    }
}

async fn poll_once(app: &AppHandle, account: &MailAccountConfig) -> anyhow::Result<()> {
    let secret_store = app.state::<Arc<SecretStore>>();
    let password = secret_store
        .get(&mail_secret_key(&account.id))?
        .ok_or_else(|| anyhow::anyhow!("no password stored for account {}", account.id))?;

    let mail_db = app.state::<MailDb>();
    let since_uid = mail_db::get_last_seen_uid(&mail_db.pool, &account.id).await?;

    let raw_messages = fetch_new_messages(
        &account.imap_host,
        account.imap_port,
        &account.username,
        &password,
        since_uid,
    ).await?;

    if raw_messages.is_empty() {
        return Ok(());
    }

    let router = app.state::<AiRouter>();
    let provider = router.resolve().await?;

    let mut max_uid = since_uid.unwrap_or(0);
    for raw in raw_messages {
        max_uid = max_uid.max(raw.uid);

        let Some(parsed) = parse_raw_message(&raw.raw) else {
            log::warn!("mail: could not parse message uid={} for account {}", raw.uid, account.id);
            continue;
        };

        let mut classification = classify_message(provider.clone(), &parsed.sender, &parsed.subject, &parsed.body_text)
            .await
            .unwrap_or_else(|e| {
                log::warn!("mail classification failed for uid={}: {e}", raw.uid);
                Default::default()
            });
        // The prompt asks the model to never mark promotional mail as
        // important too, but LLMs don't reliably honor soft constraints —
        // enforce it here so a misclassification can't trigger a spurious
        // "important" OS notification for what's actually a marketing email.
        if classification.is_promotional {
            classification.is_important = false;
        }

        let row = mail_db::insert_message(&mail_db.pool, NewMessage {
            account_id: &account.id,
            uid: raw.uid,
            sender: &parsed.sender,
            subject: &parsed.subject,
            date: parsed.date.as_deref(),
            body_text: &parsed.body_text,
            ai_summary: Some(&classification.summary),
            is_important: classification.is_important,
            is_promotional: classification.is_promotional,
        }).await?;

        let _ = app.emit(MAIL_SYNC_EVENT, MailSyncEvent::Summary {
            account_id: account.id.clone(),
            message_id: row.id.clone(),
        });

        if classification.is_important {
            let _ = app.emit(MAIL_SYNC_EVENT, MailSyncEvent::Important {
                account_id: account.id.clone(),
                message_id: row.id,
                subject: parsed.subject,
                summary: classification.summary,
            });
        }
    }

    mail_db::set_last_seen_uid(&mail_db.pool, &account.id, max_uid).await?;
    Ok(())
}

/// Called once from `lib.rs`'s `.setup()` closure.
pub fn init(app: &AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        restart_all(&app_handle).await;
    });
}

// src-tauri/src/commands/mail.rs
use std::sync::Arc;
use std::time::Duration;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

use crate::config::{ConfigStore, MailAccountConfig};
use crate::db::mail::{self as mail_db, MailDb, MailMessageRow};
use crate::mail::client;
use crate::mail::manager::{DeleteRequest, MailState};
use crate::mail::poller::{emit_removed, mail_secret_key, restart_account, stop_account};
use crate::secret::SecretStore;

/// How long the UI waits for the account's task to service a delete.
///
/// The task only looks at its delete queue while parked in IDLE or in the
/// fallback sleep — never mid-sync, and a sync that classifies a batch of new
/// mail is one LLM call per message, so it can run for minutes. Without a
/// ceiling the delete button would simply hang for that long with no
/// explanation. Timing out is not a cancellation: the request stays queued and
/// may still succeed, which is why the message below says so rather than
/// claiming nothing happened.
const DELETE_TIMEOUT: Duration = Duration::from_secs(60);

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

/// Verify credentials before an account is saved. The frontend calls this
/// first, so the overwhelmingly likely first-run mistakes — a Google account
/// password instead of an App Password, or IMAP not enabled — surface as an
/// error the user can read, rather than as a background poller silently
/// retrying a failing login every cycle (which is itself enough to trip a
/// provider's account-protection).
#[tauri::command]
pub async fn mail_test_connection(input: MailAccountInput) -> Result<(), String> {
    client::test_connection(&input.imap_host, input.imap_port, &input.username, &input.password)
        .await
        .map_err(|e| e.to_string())
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
        // Clamped, not just defaulted: 0 would make the poller's
        // sleep(Duration::from_secs(..)) a no-op and turn polling into an
        // unthrottled IMAP reconnect loop. The frontend clamps too, but this
        // command is callable by anything in the webview — and clamping here
        // also keeps the persisted config honest about what the poller does.
        poll_interval_secs: input.poll_interval_secs.map(|s| s.max(60)).unwrap_or(300),
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

/// Move one message to the server's Trash folder, then drop the local row.
///
/// The only command in the mail feature that writes to IMAP. It is routed
/// through the account's existing background task rather than opening its own
/// connection, so it runs on the one authenticated session that account has and
/// can never interleave with a sync or a reconciliation in progress.
///
/// Strictly ordered: the local row is removed only *after* the server reports
/// the move succeeded. A failed move therefore leaves the message exactly where
/// it was, both on the server and in the cache, and the error text reaches the
/// UI so the user is never told mail was deleted when it wasn't.
#[tauri::command]
pub async fn mail_delete_message(
    app: AppHandle,
    message_id: String,
    mail_db: State<'_, MailDb>,
) -> Result<(), String> {
    // The UID and the owning account come from the cached row, not from the
    // frontend: a UID is only meaningful together with the account whose
    // mailbox it indexes into, and taking both from one row is what keeps a
    // delete from ever being aimed at another account's mailbox.
    let Some(row) = mail_db::get_message(&mail_db.pool, &message_id).await.map_err(|e| e.to_string())? else {
        // Reconciliation may have removed it between the list render and the
        // click. Nothing to do, and nothing to move on the server either.
        return Err("that message is no longer in the local cache".to_string());
    };

    // Cloned out from under the lock: the delete below can take a while, and
    // holding `MailState` for it would block every other account's start/stop.
    let sender = {
        let state = app.state::<tokio::sync::Mutex<MailState>>();
        let guard = state.lock().await;
        guard.tasks.get(&row.account_id).map(|task| task.delete.clone())
    };
    let Some(sender) = sender else {
        return Err("this account has no running mail connection to delete through".to_string());
    };

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    sender
        .send(DeleteRequest { uid: row.uid, reply: reply_tx })
        .await
        .map_err(|_| "the account's mail connection has stopped".to_string())?;

    match tokio::time::timeout(DELETE_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => return Err(e),
        // The task ended without answering.
        Ok(Err(_)) => return Err("the account's mail connection ended before the message could be moved to Trash".to_string()),
        Err(_) => return Err(format!(
            "the mail connection is busy and did not move the message within {}s; it may still be in progress",
            DELETE_TIMEOUT.as_secs()
        )),
    }

    // Past this point the message is in Trash on the server. `0` rows means a
    // reconciliation noticed it was gone and removed the row first — a race,
    // not a failure, and emitting a second removal for it would be noise.
    let removed = mail_db::delete_message_locally(&mail_db.pool, &message_id)
        .await
        .map_err(|e| e.to_string())?;
    if removed > 0 {
        emit_removed(&app, &row.account_id, removed);
    }
    Ok(())
}

#[tauri::command]
pub async fn mail_count_unread(mail_db: State<'_, MailDb>) -> Result<i64, String> {
    mail_db::count_unread(&mail_db.pool).await.map_err(|e| e.to_string())
}

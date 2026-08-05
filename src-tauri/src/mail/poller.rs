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
    /// Cached messages disappeared (deleted/archived on the server, or dropped
    /// wholesale because UIDVALIDITY changed). Carries `account_id` like the
    /// others so the Mail tab's existing per-account refetch and the unread
    /// badge's refresh both pick removals up with no extra wiring.
    Removed { account_id: String, removed_count: u64 },
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
        // Floored here as well as in mail_add_account: this is the only place
        // the interval is actually consumed, so it is the only clamp that also
        // covers a hand-edited config, where a 0 would make this sleep a no-op
        // and turn polling into an unthrottled IMAP reconnect loop.
        let interval = (account.poll_interval_secs as u64).max(60);
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
}

async fn poll_once(app: &AppHandle, account: &MailAccountConfig) -> anyhow::Result<()> {
    let secret_store = app.state::<Arc<SecretStore>>();
    let password = secret_store
        .get(&mail_secret_key(&account.id))?
        .ok_or_else(|| anyhow::anyhow!("no password stored for account {}", account.id))?;

    let mail_db = app.state::<MailDb>();
    let since_uid = mail_db::get_last_seen_uid(&mail_db.pool, &account.id).await?;
    let stored_uid_validity = mail_db::get_uid_validity(&mail_db.pool, &account.id).await?;

    // Resolved *before* any IMAP traffic: without a classifier the poll can't
    // proceed at all, and resolving afterwards meant a user with mail but no
    // AI provider configured downloaded up to a full batch window of message
    // bodies (attachments included) on every single cycle, bailed here, never
    // reached `set_last_seen_uid`, and re-downloaded the identical messages
    // forever — a fast track to Gmail's daily IMAP bandwidth cap, with
    // nothing to show for it in the UI.
    //
    // This does resolve on cycles that turn out to have no new mail, which is
    // fine: `resolve()` reads config plus the keychain and constructs a
    // client — no network, except for an OAuth token refresh that is due
    // anyway and happens at most once per token lifetime.
    let router = app.state::<AiRouter>();
    let provider = router.resolve().await?;

    let outcome = fetch_new_messages(
        &account.imap_host,
        account.imap_port,
        &account.username,
        &password,
        since_uid,
        stored_uid_validity,
    ).await?;

    // Before anything is inserted: under a new UIDVALIDITY the cached rows are
    // keyed by UIDs that no longer identify anything, and the batches below
    // were already re-seeded from scratch by the client, so keeping them would
    // mean stale rows plus UNIQUE(account_id, uid) collisions against the new
    // numbering.
    if let Some(server_uid_validity) = outcome.uid_validity {
        if outcome.uid_validity_changed {
            log::warn!(
                "mail: UIDVALIDITY for account {} changed from {:?} to {}; dropping the cached messages and re-syncing",
                account.id, stored_uid_validity, server_uid_validity
            );
            let dropped = mail_db::reset_for_uid_validity(&mail_db.pool, &account.id, server_uid_validity).await?;
            if dropped > 0 {
                emit_removed(app, &account.id, dropped);
            }
        } else {
            mail_db::set_uid_validity(&mail_db.pool, &account.id, server_uid_validity).await?;
        }
    }

    for batch in outcome.batches {
        for raw in batch.messages {
            // Deliberate: a message that fails to parse is still covered by
            // the batch's `max_uid` below and thus never retried. Retrying a
            // message that will never parse would stall this account's poll
            // loop on it forever, which is worse than permanently skipping
            // it — this is a one-way skip, not an oversight.
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

            // Log-and-continue rather than `?`-propagate: a single failed
            // insert (e.g. transient SQLITE_BUSY) must not abort the whole
            // batch, since that would leave this batch's `set_last_seen_uid`
            // below unreached and wedge this account's `last_seen_uid` at the
            // pre-failure UID forever — every subsequent poll would re-fetch
            // the same batch and hit the same failure (or a UNIQUE constraint
            // violation on the messages already inserted this cycle) on the
            // very first message.
            let row = match mail_db::insert_message(&mail_db.pool, NewMessage {
                account_id: &account.id,
                uid: raw.uid,
                sender: &parsed.sender,
                subject: &parsed.subject,
                date: parsed.date.as_deref(),
                body_text: &parsed.body_text,
                ai_summary: Some(&classification.summary),
                is_important: classification.is_important,
                is_promotional: classification.is_promotional,
            }).await {
                Ok(row) => row,
                Err(e) => {
                    log::warn!("mail: failed to insert message uid={} for account {}: {e}", raw.uid, account.id);
                    continue;
                }
            };

            if let Err(e) = app.emit(MAIL_SYNC_EVENT, MailSyncEvent::Summary {
                account_id: account.id.clone(),
                message_id: row.id.clone(),
            }) {
                log::error!("mail: failed to emit {MAIL_SYNC_EVENT} (summary) for account {}: {e}", account.id);
            }

            if classification.is_important {
                if let Err(e) = app.emit(MAIL_SYNC_EVENT, MailSyncEvent::Important {
                    account_id: account.id.clone(),
                    message_id: row.id,
                    subject: parsed.subject,
                    summary: classification.summary,
                }) {
                    log::error!("mail: failed to emit {MAIL_SYNC_EVENT} (important) for account {}: {e}", account.id);
                }
            }
        }

        // Commit progress per batch, not once at the end: a failure partway
        // through a large first sync must not throw away the batches that
        // already landed in the DB, or the next cycle would re-fetch and
        // re-classify them (and hit the UNIQUE(account_id, uid) constraint)
        // instead of moving forward.
        mail_db::set_last_seen_uid(&mail_db.pool, &account.id, batch.max_uid).await?;
    }

    // Mail deleted or archived on the server has to leave the local cache too,
    // or the Mail tab keeps listing messages that no longer exist and the
    // unread badge counts them forever.
    //
    // `server_uids` is `None` when the UID SEARCH ALL didn't come back cleanly,
    // and that case never reaches the delete at all — a reconciliation we
    // couldn't perform must be a no-op, not a mass delete. Note also that this
    // deliberately does not touch `last_seen_uid`: removing a local row must
    // never make the next poll re-download anything.
    if let Some(server_uids) = outcome.server_uids {
        match mail_db::delete_messages_absent_from_server(&mail_db.pool, &account.id, &server_uids).await {
            Ok(0) => {}
            Ok(removed) => {
                log::info!("mail: removed {removed} message(s) no longer on the server for account {}", account.id);
                emit_removed(app, &account.id, removed);
            }
            // Log-and-continue for the same reason as the insert path: a
            // transient DB error here must not fail the poll and undo the
            // batches that already committed.
            Err(e) => log::warn!("mail: deletion reconciliation failed for account {}: {e}", account.id),
        }
    }

    Ok(())
}

fn emit_removed(app: &AppHandle, account_id: &str, removed_count: u64) {
    if let Err(e) = app.emit(MAIL_SYNC_EVENT, MailSyncEvent::Removed {
        account_id: account_id.to_string(),
        removed_count,
    }) {
        log::error!("mail: failed to emit {MAIL_SYNC_EVENT} (removed) for account {account_id}: {e}");
    }
}

/// Called once from `lib.rs`'s `.setup()` closure.
pub fn init(app: &AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        restart_all(&app_handle).await;
    });
}

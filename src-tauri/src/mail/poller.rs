// src-tauri/src/mail/poller.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

use crate::ai::router::AiRouter;
use crate::config::{ConfigStore, MailAccountConfig};
use crate::db::mail::{self as mail_db, MailDb, NewMessage};
use crate::secret::SecretStore;

use super::classify::classify_message;
use super::client::{IdleOutcome, MailConnection};
use super::manager::{MailState, MailTask};
use super::parse::parse_raw_message;

/// IMAP must not be hit more than once a minute no matter what the config
/// says. Mirrored in `mail_add_account` and in the settings form, but this is
/// the only clamp that also covers a hand-edited config file — where a 0 would
/// otherwise turn the fallback poll and the reconnect backoff into unthrottled
/// loops.
const MIN_INTERVAL_SECS: u64 = 60;

/// First delay before reconnecting. Short on purpose: the ordinary reason to
/// be here is a connection that simply ended (a sync error, a server hanging
/// up a long-lived socket), not an outage — and the doubling below is what
/// handles an outage.
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(5);

/// How long a stop waits for the task to send DONE + LOGOUT before it gives up
/// and aborts. Two round trips on a healthy connection; the cap only matters
/// for a wedged one, which must not make removing an account hang the UI.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Stop the existing task for this account (if any) and spawn a fresh one.
/// Called after an account is added, and would be called after a future
/// "edit account" command too (not in Phase 1 scope).
pub async fn restart_account(app: &AppHandle, account_id: &str) {
    // The old task is stopped *before* the new one is spawned, so an account
    // never briefly holds two IMAP connections — which matters more now that a
    // connection is long-lived rather than one-per-poll.
    //
    // Note that this means the stop and the spawn are no longer one atomic
    // step under the `MailState` lock — the stop takes that lock itself, and
    // holding it while awaiting a task that may need seconds to log out would
    // stall every other account. Two *concurrent* restarts of the same account
    // could therefore both spawn. No caller does that today: add-account uses
    // a fresh id, `restart_all` is sequential, and each is one user action.
    stop_account(app, account_id).await;

    let config_store = app.state::<Arc<ConfigStore>>();
    let Some(account) = config_store.get().mail_accounts.into_iter().find(|a| a.id == account_id) else {
        return;
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let app_clone = app.clone();
    let id_for_map = account.id.clone();
    let handle = tokio::spawn(async move {
        poll_loop(app_clone, account, shutdown_rx).await;
    });

    let state = app.state::<tokio::sync::Mutex<MailState>>();
    let mut guard = state.lock().await;
    guard.tasks.insert(id_for_map, MailTask { handle, shutdown: shutdown_tx });
}

/// Stop an account's task without starting a new one. Called before removing
/// an account.
///
/// Gracefully, not `abort()`. The task now holds a persistent connection and
/// spends nearly all its time parked in IDLE; aborting there drops the socket
/// mid-command with no LOGOUT, leaving a session the server only reclaims on
/// its own timeout — and providers cap concurrent IMAP connections per account
/// (Gmail at about 15). Signalling instead makes the IDLE resolve as
/// `ManualInterrupt`, so the task sends DONE, logs out, and frees the slot at
/// once. `abort()` remains the fallback for a connection that has wedged.
///
/// This also orders correctly against `mail_remove_account`, which deletes the
/// account's rows right after: awaiting the stop means the task cannot insert
/// a message into a table row set that is about to be deleted.
pub async fn stop_account(app: &AppHandle, account_id: &str) {
    // Taken out from under the lock before awaiting anything: the graceful
    // stop below costs a round trip or two, and holding `MailState` for that
    // long would block every other account's start/stop.
    let task = {
        let state = app.state::<tokio::sync::Mutex<MailState>>();
        let mut guard = state.lock().await;
        guard.tasks.remove(account_id)
    };
    let Some(task) = task else { return };

    let _ = task.shutdown.send(true);
    let abort_handle = task.handle.abort_handle();
    if tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, task.handle).await.is_err() {
        log::warn!("mail: account {account_id} did not stop within {GRACEFUL_SHUTDOWN_TIMEOUT:?}; aborting");
        abort_handle.abort();
    }
}

/// What triggered a sync — the only thing that varies between syncs on a
/// long-lived connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncTrigger {
    /// The first sync after (re)connecting.
    Connected,
    /// IDLE reported that *something* changed in the mailbox.
    IdleNotification,
    /// The IDLE liveness timeout expired with the server having said nothing.
    LivenessTimeout,
    /// A fallback poll, on a server that does not advertise IDLE.
    PollInterval,
}

/// Whether this sync also reconciles server-side deletions (`UID SEARCH ALL`).
///
/// `UID SEARCH ALL` returns every UID the mailbox holds, so on a large INBOX it
/// is by far the heaviest part of a sync — and IDLE fires a notification per
/// delivered message. Reconciling on every one would turn a busy inbox into a
/// stream of whole-mailbox scans, so a notification ordinarily syncs new mail
/// only and lets deletions wait.
///
/// They cannot wait for the liveness timeout, though, tempting as that is:
/// async-imap resets that timer on *any* server response, keepalives included,
/// and servers such as Dovecot emit `* OK Still here` every couple of minutes —
/// so on a healthy connection the timeout may never fire at all, and a
/// connection can stay healthy for days. Elapsed time since the last
/// reconciliation is the honest backstop, and it keeps the worst-case staleness
/// of a deleted message at one interval, exactly as the old timer-based poller
/// had it.
fn should_reconcile(
    trigger: SyncTrigger,
    since_last_reconcile: Duration,
    interval: Duration,
) -> bool {
    match trigger {
        SyncTrigger::Connected | SyncTrigger::LivenessTimeout | SyncTrigger::PollInterval => true,
        SyncTrigger::IdleNotification => since_last_reconcile >= interval,
    }
}

/// One connection's lifetime, from the outside.
enum ConnectionExit {
    /// The account was asked to stop. Do not reconnect.
    Shutdown,
    /// The connection is gone. `progressed` records whether it managed at
    /// least one successful sync first, which is what tells a connection that
    /// merely ended apart from a server that is refusing us and must not be
    /// hammered.
    Retry { progressed: bool },
}

/// How a connection's inner loop ended, from the inside.
enum SessionExit {
    Shutdown,
    Retry,
}

/// One long-lived connection per account, re-established with backoff whenever
/// it dies. Mail arrives by push (IMAP IDLE) rather than on a timer; the
/// configured interval is now the IDLE liveness check and the fallback polling
/// period for servers that cannot push.
async fn poll_loop(app: AppHandle, account: MailAccountConfig, mut shutdown: watch::Receiver<bool>) {
    let interval = effective_interval(account.poll_interval_secs);

    let mut consecutive_failures: u32 = 0;
    loop {
        match run_connection(&app, &account, interval, &mut shutdown).await {
            ConnectionExit::Shutdown => return,
            ConnectionExit::Retry { progressed } => {
                consecutive_failures =
                    if progressed { 0 } else { consecutive_failures.saturating_add(1) };
            }
        }

        let delay = reconnect_delay(consecutive_failures, interval);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wait_for_shutdown(&mut shutdown) => return,
        }
    }
}

/// Open one connection, run it for as long as it lives, and log out of it.
///
/// This is the *only* place a live session is logged out, and it does so for
/// every way `drive_session` can end — success, error, or shutdown — because
/// `drive_session` hands the connection back whenever it still owns one. The
/// old shape relied on a single `fetch` call being wrapped in
/// connect/logout; with a session that outlives many syncs, that invariant is
/// carried by ownership instead: nothing below `MailConnection::connect` can
/// keep a session, so nothing below can skip this logout.
async fn run_connection(
    app: &AppHandle,
    account: &MailAccountConfig,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> ConnectionExit {
    let secret_store = app.state::<Arc<SecretStore>>();
    let password = match secret_store.get(&mail_secret_key(&account.id)) {
        Ok(Some(password)) => password,
        Ok(None) => {
            log::warn!("mail: no password stored for account {}", account.id);
            return ConnectionExit::Retry { progressed: false };
        }
        Err(e) => {
            log::warn!("mail: could not read the password for account {}: {e}", account.id);
            return ConnectionExit::Retry { progressed: false };
        }
    };

    // No session exists yet, so an error here has nothing to log out of.
    let conn = match MailConnection::connect(
        &account.imap_host,
        account.imap_port,
        &account.username,
        &password,
    ).await {
        Ok(conn) => conn,
        Err(e) => {
            log::warn!("mail: could not connect account {}: {e}", account.id);
            return ConnectionExit::Retry { progressed: false };
        }
    };

    let (conn, exit, progressed) = drive_session(app, account, conn, interval, shutdown).await;
    if let Some(conn) = conn {
        conn.logout().await;
    }

    match exit {
        SessionExit::Shutdown => ConnectionExit::Shutdown,
        SessionExit::Retry => ConnectionExit::Retry { progressed },
    }
}

/// Drive one live connection until it ends.
///
/// Returns the connection whenever it is still alive, so the caller can log
/// out exactly once. `None` is returned only when the session died together
/// with the socket, where there is nothing left to log out of.
async fn drive_session(
    app: &AppHandle,
    account: &MailAccountConfig,
    mut conn: MailConnection,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> (Option<MailConnection>, SessionExit, bool) {
    // Always sync first on a (re)connect. IDLE only reports what happens while
    // we are listening, so this is what catches whatever arrived while the
    // account had no connection, and it reconciles deletions. It also SELECTs
    // INBOX — which IDLE requires — so the IDLE loop below is deliberately
    // unreachable unless this succeeded.
    let mut last_reconcile = Instant::now();
    let reconcile = should_reconcile(SyncTrigger::Connected, last_reconcile.elapsed(), interval);
    if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile).await {
        log::warn!("mail sync failed for account {}: {e}", account.id);
        return (Some(conn), SessionExit::Retry, false);
    }

    if !conn.supports_idle() {
        // Unchanged pre-IDLE behavior, for servers that cannot push.
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = wait_for_shutdown(shutdown) => return (Some(conn), SessionExit::Shutdown, true),
            }
            let reconcile = should_reconcile(SyncTrigger::PollInterval, last_reconcile.elapsed(), interval);
            if reconcile {
                last_reconcile = Instant::now();
            }
            if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile).await {
                log::warn!("mail poll failed for account {}: {e}", account.id);
                return (Some(conn), SessionExit::Retry, true);
            }
        }
    }

    loop {
        // A stop signalled while the sync above was in flight would otherwise
        // cost an IDLE and a DONE round trip before being noticed, and the
        // stop is on a budget.
        if *shutdown.borrow() {
            return (Some(conn), SessionExit::Shutdown, true);
        }

        let (returned, outcome) = match conn.idle_wait(interval, wait_for_shutdown(shutdown)).await {
            Ok(returned) => returned,
            Err(e) => {
                log::warn!("mail: IDLE ended for account {}: {e}", account.id);
                return (None, SessionExit::Retry, true);
            }
        };
        conn = returned;

        let trigger = match outcome {
            // IDLE says only that *something* changed (EXISTS/EXPUNGE/FETCH),
            // never what — so the one useful reaction is an ordinary sync.
            IdleOutcome::NewData => SyncTrigger::IdleNotification,
            IdleOutcome::Timeout => SyncTrigger::LivenessTimeout,
            IdleOutcome::Interrupted => return (Some(conn), SessionExit::Shutdown, true),
        };

        let reconcile = should_reconcile(trigger, last_reconcile.elapsed(), interval);
        if reconcile {
            last_reconcile = Instant::now();
        }

        if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile).await {
            log::warn!("mail sync failed for account {}: {e}", account.id);
            return (Some(conn), SessionExit::Retry, true);
        }
    }
}

/// Resolves once this account has been asked to stop — immediately if it
/// already has been, so a signal that arrived while we were mid-sync is never
/// missed.
async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        // The sender is dropped only when the task's registry entry went away
        // without a signal; stopping is the right reading of that too.
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// The interval the poller actually uses, floored.
fn effective_interval(configured_secs: u32) -> Duration {
    Duration::from_secs((configured_secs as u64).max(MIN_INTERVAL_SECS))
}

/// How long to wait before reconnecting, doubling per consecutive failure and
/// capped at `cap` (the account's interval). A server that is down must not be
/// hammered — and unlike the old poller, whose fixed sleep bounded reconnect
/// attempts for free, this loop reconnects as soon as a connection ends.
fn reconnect_delay(consecutive_failures: u32, cap: Duration) -> Duration {
    // Clamped before shifting: `1u64 << 64` is undefined behavior in the sense
    // that Rust panics on it, and a long outage reaches large counts.
    let factor = 1u64 << consecutive_failures.min(32);
    Duration::from_secs(RECONNECT_BASE_DELAY.as_secs().saturating_mul(factor)).min(cap)
}

async fn sync_and_persist(
    app: &AppHandle,
    account: &MailAccountConfig,
    conn: &mut MailConnection,
    reconcile: bool,
) -> anyhow::Result<()> {
    let mail_db = app.state::<MailDb>();
    let since_uid = mail_db::get_last_seen_uid(&mail_db.pool, &account.id).await?;
    let stored_uid_validity = mail_db::get_uid_validity(&mail_db.pool, &account.id).await?;

    // Resolved *before* the sync fetches anything: without a classifier this
    // cannot proceed at all, and resolving afterwards meant a user with mail
    // but no AI provider configured downloaded up to a full batch window of
    // message bodies (attachments included) on every single cycle, bailed
    // here, never reached `set_last_seen_uid`, and re-downloaded the identical
    // messages forever — a fast track to Gmail's daily IMAP bandwidth cap,
    // with nothing to show for it in the UI.
    //
    // Resolved per sync rather than once per connection, because a connection
    // now lives for days: a provider the user configures at noon has to reach
    // the mail that arrives at one. It is cheap — `resolve()` reads config
    // plus the keychain and constructs a client, with no network beyond an
    // OAuth refresh that is due anyway.
    let router = app.state::<AiRouter>();
    let provider = router.resolve().await?;

    let outcome = conn.sync(since_uid, stored_uid_validity, reconcile).await?;

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

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: Duration = Duration::from_secs(300);

    #[test]
    fn a_fresh_idle_notification_skips_the_full_mailbox_scan() {
        assert!(
            !should_reconcile(SyncTrigger::IdleNotification, Duration::from_secs(1), INTERVAL),
            "IDLE fires per delivered message; a UID SEARCH ALL per notification would scan the whole mailbox each time"
        );
    }

    #[test]
    fn a_stale_reconciliation_is_refreshed_even_under_a_stream_of_idle_notifications() {
        assert!(
            should_reconcile(SyncTrigger::IdleNotification, INTERVAL, INTERVAL),
            "keepalives reset the IDLE timer, so the liveness timeout alone can leave deletions unreconciled for days"
        );
    }

    #[test]
    fn every_other_trigger_reconciles_deletions_however_recent_the_last_one() {
        assert!(
            should_reconcile(SyncTrigger::Connected, Duration::ZERO, INTERVAL),
            "a reconnect must reconcile: anything deleted while we were disconnected was never notified"
        );
        assert!(
            should_reconcile(SyncTrigger::LivenessTimeout, Duration::ZERO, INTERVAL),
            "a liveness timeout means the server went quiet, which is exactly when the local cache may have drifted"
        );
        assert!(
            should_reconcile(SyncTrigger::PollInterval, Duration::ZERO, INTERVAL),
            "the non-IDLE fallback must keep reconciling on every poll, exactly as the old timer-based poller did"
        );
    }

    #[test]
    fn an_interval_below_the_floor_is_raised_to_it() {
        assert_eq!(
            effective_interval(30),
            Duration::from_secs(60),
            "a hand-edited config must not be able to poll IMAP faster than once a minute"
        );
        assert_eq!(
            effective_interval(0),
            Duration::from_secs(60),
            "0 would make the fallback sleep a no-op and the reconnect backoff unthrottled"
        );
    }

    #[test]
    fn an_interval_above_the_floor_is_honored() {
        assert_eq!(
            effective_interval(300),
            Duration::from_secs(300),
            "the configured interval is what bounds IDLE liveness checks; clamping it would be wrong"
        );
    }

    #[test]
    fn the_first_reconnect_is_prompt() {
        assert_eq!(
            reconnect_delay(0, Duration::from_secs(300)),
            RECONNECT_BASE_DELAY,
            "a connection that merely ended must come back quickly, not after a whole interval"
        );
    }

    #[test]
    fn each_consecutive_failure_doubles_the_delay() {
        let cap = Duration::from_secs(300);
        assert_eq!(reconnect_delay(1, cap), Duration::from_secs(10));
        assert_eq!(reconnect_delay(2, cap), Duration::from_secs(20));
        assert_eq!(reconnect_delay(3, cap), Duration::from_secs(40));
    }

    #[test]
    fn the_delay_never_exceeds_the_cap() {
        let cap = Duration::from_secs(300);
        assert_eq!(
            reconnect_delay(10, cap),
            cap,
            "5s doubled ten times is 85 minutes; the configured interval is the ceiling"
        );
        assert_eq!(
            reconnect_delay(u32::MAX, cap),
            cap,
            "a long outage must saturate at the cap, not panic on an over-wide shift"
        );
    }
}

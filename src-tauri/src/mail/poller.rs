// src-tauri/src/mail/poller.rs
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, watch};

use crate::ai::router::AiRouter;
use crate::config::{ConfigStore, MailAccountConfig};
use crate::db::mail::{self as mail_db, MailDb, NewMessage};
use crate::secret::SecretStore;

use super::classify::classify_message;
use super::client::{IdleOutcome, MailClientError, MailConnection};
use super::manager::{DeleteRequest, MailState, MailTask};
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

/// How long a session must hold *steady state* to count as "came up properly",
/// and so to clear the reconnect backoff.
///
/// The question the backoff needs answered is *not* "did a sync succeed" — a
/// connection can sync once and then fail the same way forever (a server that
/// answers `BAD` to IDLE despite advertising it, a middlebox that kills the
/// socket the moment IDLE is issued). Counting that as progress pins the delay
/// at `RECONNECT_BASE_DELAY` and turns the loop into a login-and-abandon storm,
/// which is exactly how an account hits a provider's connection cap.
///
/// Measured from the point `drive_session` enters its wait loop, *not* from
/// login — see `steady_lifetime`. Timed from login it was a proxy that the slow
/// paths defeated: a server whose `SELECT INBOX` or `UID FETCH` hangs until
/// `FETCH_TIMEOUT` produced a 30-second "lifetime" for a session that never
/// worked at all, cleared the counter, and retried 5 seconds later, forever.
const HEALTHY_SESSION_LIFETIME: Duration = Duration::from_secs(10);

/// How many deletes may be queued for one account's task at a time. Small on
/// purpose: the task services them one at a time between IDLEs, and a user
/// clicking faster than that should be told the account is busy rather than
/// building a backlog of writes they can no longer see the outcome of.
const DELETE_QUEUE_DEPTH: usize = 4;

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
    /// This account can no longer reach its mail server, so its Mail tab has
    /// silently stopped updating. `message` is the underlying failure verbatim
    /// (`login failed: …`, `connection error: …`), which is the part a user can
    /// actually act on — a wrong App Password reads very differently from a
    /// refused TCP connection.
    ///
    /// Emitted on the healthy → failing *transition* only, never per retry: see
    /// `report_health`.
    ConnectionFailed { account_id: String, message: String },
    /// The account is reaching its mail server again. The counterpart
    /// transition, and the only thing that clears the failure in the UI.
    ConnectionRestored { account_id: String },
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
    // stall every other account. Two concurrent restarts of the same account
    // can therefore both spawn (`restart_all` snapshots the account list at
    // its start, so an account added during startup is one way in).
    //
    // Benign, and not by luck: `tasks.insert` drops the `MailTask` it evicts,
    // which drops its `watch::Sender`, which makes the orphaned task's
    // `wait_for_shutdown` return on `changed().is_err()`. The loser shuts
    // itself down the same graceful way, LOGOUT included.
    stop_account(app, account_id).await;

    let config_store = app.state::<Arc<ConfigStore>>();
    let Some(account) = config_store.get().mail_accounts.into_iter().find(|a| a.id == account_id) else {
        return;
    };

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (delete_tx, delete_rx) = mpsc::channel::<DeleteRequest>(DELETE_QUEUE_DEPTH);
    let app_clone = app.clone();
    let id_for_map = account.id.clone();
    let handle = tokio::spawn(async move {
        poll_loop(app_clone, account, shutdown_rx, delete_rx).await;
    });

    let state = app.state::<tokio::sync::Mutex<MailState>>();
    let mut guard = state.lock().await;
    guard.tasks.insert(id_for_map, MailTask { handle, shutdown: shutdown_tx, delete: delete_tx });
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
    /// The reconciliation deadline came due while we sat in IDLE.
    ReconcileDue,
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
/// so on a healthy connection the timeout may never fire at all.
///
/// The backstop is therefore an explicit deadline: `time_until_reconcile` wakes
/// the IDLE itself (`SyncTrigger::ReconcileDue`), which is what actually bounds
/// staleness at one interval. The elapsed-time condition below is the second
/// half of the same rule, for the case where a notification arrives after the
/// deadline has already passed — it costs nothing and keeps the two paths from
/// disagreeing about when a reconciliation is due.
fn should_reconcile(
    trigger: SyncTrigger,
    since_last_reconcile: Duration,
    interval: Duration,
) -> bool {
    match trigger {
        SyncTrigger::Connected
        | SyncTrigger::LivenessTimeout
        | SyncTrigger::ReconcileDue
        | SyncTrigger::PollInterval => true,
        SyncTrigger::IdleNotification => since_last_reconcile >= interval,
    }
}

/// How long the next IDLE may block before it must be woken to reconcile.
///
/// Zero once the deadline has passed, which makes the wake immediate rather
/// than waiting out another whole interval.
fn time_until_reconcile(since_last_reconcile: Duration, interval: Duration) -> Duration {
    interval.saturating_sub(since_last_reconcile)
}

/// The reconnect backoff counter after one connection ended.
///
/// `lifetime` is how long the session held *steady state* — the IDLE (or
/// fallback poll) loop it reaches only after a sync has succeeded — and is
/// `ZERO` for every exit that never got there, however long the attempt took.
/// A session that reached steady state and lost it again inside
/// `HEALTHY_SESSION_LIFETIME` counts as a failure; anything longer clears the
/// counter.
///
/// Timing from login instead was the bug this replaces. The session clock then
/// included the startup sync, and the slow failures live in the startup sync:
/// a `SELECT INBOX` or `UID FETCH` that hangs until `FETCH_TIMEOUT` reported 30
/// seconds of "lifetime" for a connection that never once worked, which cleared
/// the counter and pinned the retry at `RECONNECT_BASE_DELAY` — one login every
/// ~35s, forever, with the backoff never growing at all. Measured from steady
/// state, the rule says exactly what it means: the session got up and stayed up.
fn next_failure_count(current: u32, lifetime: Duration) -> u32 {
    if lifetime >= HEALTHY_SESSION_LIFETIME {
        0
    } else {
        current.saturating_add(1)
    }
}

/// What, if anything, the UI must be told after a change in an account's
/// connection health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthReport {
    /// Nothing changed. The overwhelmingly common answer.
    Quiet,
    Failed,
    Recovered,
}

/// Whether an account's connection health crossed a boundary the user has to
/// see, given the state last reported to them and whether it is working now.
///
/// Transitions only. A failing account retries on the backoff schedule above,
/// which at the start of an outage is every five seconds: reporting each
/// attempt would put an event on the bus and a banner repaint in the Mail tab
/// at that rate for as long as the outage lasts, which is how a diagnostic
/// becomes noise the user learns to ignore.
///
/// "Working" is deliberately *reaching* steady state rather than staying there
/// for `HEALTHY_SESSION_LIFETIME`. The user's question is "is my mail arriving",
/// and it starts arriving the moment the first sync of a connection succeeds;
/// tying recovery to the backoff's threshold instead would leave the banner up
/// for the entire life of a working connection, since that threshold is only
/// known once the connection has already ended.
fn health_report(was_failing: bool, working: bool) -> HealthReport {
    match (was_failing, working) {
        (false, false) => HealthReport::Failed,
        (true, true) => HealthReport::Recovered,
        _ => HealthReport::Quiet,
    }
}

/// Emit the transition `health_report` decides on, and record it in `failing`.
///
/// Deliberately no OS notification: a connection that is down is not worth
/// interrupting the user for — the poller is already retrying, and the Mail tab
/// says so when they next look at it.
fn report_health(app: &AppHandle, account_id: &str, failing: &mut bool, working: bool, reason: &str) {
    let event = match health_report(*failing, working) {
        HealthReport::Quiet => return,
        HealthReport::Failed => {
            *failing = true;
            MailSyncEvent::ConnectionFailed {
                account_id: account_id.to_string(),
                message: reason.to_string(),
            }
        }
        HealthReport::Recovered => {
            *failing = false;
            MailSyncEvent::ConnectionRestored { account_id: account_id.to_string() }
        }
    };
    if let Err(e) = app.emit(MAIL_SYNC_EVENT, event) {
        log::error!("mail: failed to emit {MAIL_SYNC_EVENT} (connection health) for account {account_id}: {e}");
    }
}

/// Stop every account's task, concurrently. Called from the app's exit hook.
///
/// Concurrently because the stops are independent and each is allowed
/// `GRACEFUL_SHUTDOWN_TIMEOUT`: run in sequence, N accounts would hold the quit
/// for 5N seconds in the worst case, which is exactly the sort of delay that
/// gets a cleanup hook deleted.
pub async fn stop_all(app: &AppHandle) {
    let account_ids: Vec<String> = {
        let state = app.state::<tokio::sync::Mutex<MailState>>();
        let guard = state.lock().await;
        guard.tasks.keys().cloned().collect()
    };
    if account_ids.is_empty() {
        return;
    }

    log::info!("mail: stopping {} account task(s) before exit", account_ids.len());
    futures_util::future::join_all(account_ids.iter().map(|id| stop_account(app, id))).await;
    log::info!("mail: stopped {} account task(s)", account_ids.len());
}

/// One connection's lifetime, from the outside.
enum ConnectionExit {
    /// The account was asked to stop. Do not reconnect.
    Shutdown,
    /// The connection is gone. `steady_lifetime` is how long the session held
    /// steady state — `ZERO` when it never reached it — and is what the backoff
    /// uses to tell a connection that ran from one that never worked.
    Retry { steady_lifetime: Duration },
}

/// How a connection's inner loop ended, from the inside.
enum SessionExit {
    Shutdown,
    Retry { steady_lifetime: Duration },
}

/// How long a session has held steady state, or `ZERO` if it never reached it.
///
/// The clock that `HEALTHY_SESSION_LIFETIME` is compared against. It starts
/// after the first successful sync — not at login — so that every failure
/// *during* startup counts as a failure no matter how slowly it failed.
fn steady_lifetime(steady_since: Option<Instant>) -> Duration {
    steady_since.map_or(Duration::ZERO, |since| since.elapsed())
}

/// One long-lived connection per account, re-established with backoff whenever
/// it dies. Mail arrives by push (IMAP IDLE) rather than on a timer; the
/// configured interval is now the IDLE liveness check and the fallback polling
/// period for servers that cannot push.
async fn poll_loop(
    app: AppHandle,
    account: MailAccountConfig,
    mut shutdown: watch::Receiver<bool>,
    // Owned here rather than per connection so a delete queued while the
    // account is reconnecting is still waiting when the new session comes up.
    mut delete_rx: mpsc::Receiver<DeleteRequest>,
) {
    let interval = effective_interval(account.poll_interval_secs);

    let mut consecutive_failures: u32 = 0;
    // The health last reported to the UI, owned here so it survives across
    // connections — which is what makes `report_health` able to emit on
    // transitions rather than on every attempt.
    let mut failing = false;
    loop {
        match run_connection(&app, &account, interval, &mut shutdown, &mut delete_rx, &mut failing).await {
            ConnectionExit::Shutdown => return,
            ConnectionExit::Retry { steady_lifetime } => {
                consecutive_failures = next_failure_count(consecutive_failures, steady_lifetime);
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
    delete_rx: &mut mpsc::Receiver<DeleteRequest>,
    failing: &mut bool,
) -> ConnectionExit {
    let secret_store = app.state::<Arc<SecretStore>>();
    let password = match secret_store.get(&mail_secret_key(&account.id)) {
        Ok(Some(password)) => password,
        Ok(None) => {
            log::warn!("mail: no password stored for account {}", account.id);
            report_health(app, &account.id, failing, false, "no password is stored for this account");
            return ConnectionExit::Retry { steady_lifetime: Duration::ZERO };
        }
        Err(e) => {
            log::warn!("mail: could not read the password for account {}: {e}", account.id);
            report_health(app, &account.id, failing, false, &format!("could not read the stored password: {e}"));
            return ConnectionExit::Retry { steady_lifetime: Duration::ZERO };
        }
    };

    // No session exists yet, so an error here has nothing to log out of — and
    // nothing to report as a lifetime either, however long the attempt took.
    // A connect that hangs for the full timeout must not look like a session
    // that ran for 30 seconds, or every hung connect would clear the backoff.
    let conn = match MailConnection::connect(
        &account.imap_host,
        account.imap_port,
        &account.username,
        &password,
    ).await {
        Ok(conn) => conn,
        Err(e) => {
            log::warn!("mail: could not connect account {}: {e}", account.id);
            // `MailClientError`'s own wording, verbatim: "login failed: …" tells
            // the user to check their App Password, "connection error: …" tells
            // them to check the host or their network. Inventing a taxonomy
            // here would only throw that distinction away.
            report_health(app, &account.id, failing, false, &e.to_string());
            return ConnectionExit::Retry { steady_lifetime: Duration::ZERO };
        }
    };

    // Connecting can take up to three timeouts; a stop signalled in the middle
    // of it would otherwise not be seen until after the first sync, which is
    // the longest uninterruptible stretch there is.
    if *shutdown.borrow() {
        conn.logout().await;
        return ConnectionExit::Shutdown;
    }

    // `drive_session` reports health itself: recovery is only observable from
    // the inside, at the moment the session reaches steady state, and a
    // connection that ends *after* reaching it is an ordinary reconnect rather
    // than something to put in front of the user.
    let (conn, exit) = drive_session(app, account, conn, interval, shutdown, delete_rx, failing).await;
    if let Some(conn) = conn {
        conn.logout().await;
    }

    match exit {
        SessionExit::Shutdown => ConnectionExit::Shutdown,
        SessionExit::Retry { steady_lifetime } => ConnectionExit::Retry { steady_lifetime },
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
    delete_rx: &mut mpsc::Receiver<DeleteRequest>,
    failing: &mut bool,
) -> (Option<MailConnection>, SessionExit) {
    // Checked before the first sync, not only after it: that sync classifies
    // every message it finds, one LLM call each, so on a first sync it is
    // minutes long. A stop that arrives just before it must not have to wait
    // it out. (`sync_and_persist` also checks before the fetch and between
    // messages, which is what bounds a stop that arrives *during* it.)
    if *shutdown.borrow() {
        return (Some(conn), SessionExit::Shutdown);
    }

    // Always sync first on a (re)connect. IDLE only reports what happens while
    // we are listening, so this is what catches whatever arrived while the
    // account had no connection, and it reconciles deletions. It also SELECTs
    // INBOX — which IDLE requires — so the IDLE loop below is deliberately
    // unreachable unless this succeeded.
    let mut last_reconcile = Instant::now();
    let reconcile = should_reconcile(SyncTrigger::Connected, last_reconcile.elapsed(), interval);
    if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile, shutdown).await {
        log::warn!("mail sync failed for account {}: {e}", account.id);
        // The one exit in this function that never reached steady state, and
        // so the one that means "this account is not working". Every exit
        // below has already had a sync succeed on this connection, which is
        // an ordinary reconnect and not the user's problem.
        report_health(app, &account.id, failing, false, &e.to_string());
        return (Some(conn), SessionExit::Retry { steady_lifetime: Duration::ZERO });
    }

    // The sync above returns `Ok(())` without doing anything when a stop was
    // already pending, so without this a stop landing there would be read as a
    // sync that succeeded — declaring steady state, and telling a user whose
    // account is failing that it just recovered.
    if *shutdown.borrow() {
        return (Some(conn), SessionExit::Shutdown);
    }

    // Steady state: a sync has succeeded, so mail is flowing, and the clock the
    // reconnect backoff measures starts here rather than at login. Everything
    // above is startup, and a failure in startup must count as a failure
    // however long it took to fail.
    let steady_since = Some(Instant::now());
    report_health(app, &account.id, failing, true, "");

    if !conn.supports_idle() {
        // Unchanged pre-IDLE behavior, for servers that cannot push — plus the
        // same delete arm the IDLE loop has, or a delete against a non-pushing
        // server would sit in the queue until the poll interval elapsed and
        // then be serviced only by luck.
        loop {
            let mut pending_delete: Option<DeleteRequest> = None;
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = wait_for_shutdown(shutdown) => return (Some(conn), SessionExit::Shutdown),
                request = delete_rx.recv() => match request {
                    Some(request) => pending_delete = Some(request),
                    // Every sender is gone, which happens only when this task's
                    // registry entry was dropped — the same thing
                    // `wait_for_shutdown` reads as a stop. Returning here
                    // rather than falling through to a sync keeps an orphaned
                    // task from spinning on a channel that is closed forever.
                    None => return (Some(conn), SessionExit::Shutdown),
                },
            }
            if let Some(request) = pending_delete {
                // Same rule as the IDLE loop: a stop that arrived alongside the
                // request wins, and dropping the request answers its caller.
                if *shutdown.borrow() {
                    return (Some(conn), SessionExit::Shutdown);
                }
                match handle_delete(&mut conn, request).await {
                    DeleteExit::Continue => continue,
                    DeleteExit::Reconnect => {
                        return (Some(conn), SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) })
                    }
                }
            }
            let reconcile = should_reconcile(SyncTrigger::PollInterval, last_reconcile.elapsed(), interval);
            if reconcile {
                last_reconcile = Instant::now();
            }
            if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile, shutdown).await {
                log::warn!("mail poll failed for account {}: {e}", account.id);
                return (Some(conn), SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) });
            }
        }
    }

    loop {
        // A stop signalled while the sync above was in flight would otherwise
        // cost an IDLE and a DONE round trip before being noticed, and the
        // stop is on a budget.
        if *shutdown.borrow() {
            return (Some(conn), SessionExit::Shutdown);
        }

        let reconcile_in = time_until_reconcile(last_reconcile.elapsed(), interval);
        // A delete rides in on the *same* interrupt the shutdown signal uses,
        // rather than on a new arm inside `idle_wait`: that keeps the client
        // free of any knowledge of delete requests, and reuses the
        // `StopSource`-drop machinery that already ends an IDLE at a defined
        // point so DONE is still sent. Which of the two fired is decided below,
        // exactly as it already was for shutdown-versus-unexplained.
        //
        // `Receiver::recv` is cancel-safe, so a request that arrives just as
        // the IDLE ends for some other reason stays queued rather than being
        // dropped on the floor.
        let mut pending_delete: Option<DeleteRequest> = None;
        let waited = {
            let interrupt = async {
                tokio::select! {
                    _ = wait_for_shutdown(shutdown) => {}
                    request = delete_rx.recv() => { pending_delete = request; }
                }
            };
            conn.idle_wait(interval, reconcile_in, interrupt).await
        };
        let (returned, outcome) = match waited {
            Ok(returned) => returned,
            Err(e) => {
                log::warn!("mail: IDLE ended for account {}: {e}", account.id);
                // The request never reached the server, and the caller is
                // waiting on a reply it would otherwise only get by timing out.
                if let Some(request) = pending_delete {
                    let _ = request.reply.send(Err(format!(
                        "the connection to the mail server ended before the message could be moved to Trash: {e}"
                    )));
                }
                return (None, SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) });
            }
        };
        conn = returned;

        // Serviced before the outcome is classified rather than inside the
        // `Interrupted` arm below, because a request can arrive in the same
        // instant the IDLE ends for some *other* reason — a push landing
        // concurrently makes the outcome `NewData` even though the delete was
        // taken off the channel. Handling it only under `Interrupted` would
        // drop that request and report "the connection ended" to a user whose
        // connection is perfectly fine.
        //
        // Skipped when a stop is pending: the IDLE is over and DONE is sent, so
        // the session is free for a command, but a delete is three or four
        // round trips and the graceful stop is on a 5s budget — overrunning it
        // costs the LOGOUT this connection is guaranteed. The request is
        // dropped instead, which tells the caller the connection ended. That is
        // true, and safe: nothing was moved.
        let serviced_delete = pending_delete.is_some() && !*shutdown.borrow();
        if serviced_delete {
            let request = pending_delete.take().expect("guarded by is_some");
            // `handle_delete` never consumes the connection, so the loop goes
            // back into IDLE below and `run_connection`'s single LOGOUT still
            // ends this session — a delete must not leave it parked outside
            // IDLE, nor bypass that logout.
            match handle_delete(&mut conn, request).await {
                DeleteExit::Continue => {}
                DeleteExit::Reconnect => {
                    return (Some(conn), SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) })
                }
            }
        }

        let trigger = match outcome {
            // IDLE says only that *something* changed (EXISTS/EXPUNGE/FETCH),
            // never what — so the one useful reaction is an ordinary sync.
            IdleOutcome::NewData => SyncTrigger::IdleNotification,
            IdleOutcome::Timeout => SyncTrigger::LivenessTimeout,
            // The reconcile deadline and a stop can become ready in the same
            // poll of the `select!` inside `idle_wait`, and it is free to pick
            // the deadline. Servicing it would then cost a provider resolve, a
            // `UID SEARCH ALL` and a fetch before the check at the top of this
            // loop was reached again — all of it inside the five seconds the
            // stop has to send LOGOUT in. Ordered above the plain arm on
            // purpose: below it, it would never match.
            IdleOutcome::ReconcileDue if *shutdown.borrow() => {
                return (Some(conn), SessionExit::Shutdown)
            }
            IdleOutcome::ReconcileDue => SyncTrigger::ReconcileDue,
            // Only a stop we actually signalled ends the account's task.
            // Reading any other interrupt as a stop would silently kill mail
            // for this account until the app is restarted, so an unexplained
            // one reconnects instead — the one outcome that is always safe.
            IdleOutcome::Interrupted if *shutdown.borrow() => {
                return (Some(conn), SessionExit::Shutdown)
            }
            // Our own interrupt, and the delete it carried is already done.
            // Straight back into IDLE: a delete is not new mail, so there is
            // nothing to sync and nothing to reconcile.
            IdleOutcome::Interrupted if serviced_delete => continue,
            IdleOutcome::Interrupted => {
                log::warn!(
                    "mail: IDLE for account {} was interrupted with no stop pending; reconnecting",
                    account.id
                );
                return (Some(conn), SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) });
            }
        };

        let reconcile = should_reconcile(trigger, last_reconcile.elapsed(), interval);
        if reconcile {
            last_reconcile = Instant::now();
        }

        if let Err(e) = sync_and_persist(app, account, &mut conn, reconcile, shutdown).await {
            log::warn!("mail sync failed for account {}: {e}", account.id);
            return (Some(conn), SessionExit::Retry { steady_lifetime: steady_lifetime(steady_since) });
        }
    }
}

/// What a serviced delete leaves the session fit for.
enum DeleteExit {
    /// The session is clean; carry on with it.
    Continue,
    /// The session may be desynced; drop it and reconnect.
    Reconnect,
}

/// Move one message to Trash on the live session, and answer the caller.
///
/// Deliberately does *not* touch the local database. The command that queued
/// this request removes the row only after seeing a successful reply, which is
/// what makes a failed move structurally incapable of deleting the user's
/// cached mail: there is no code path here that could.
///
/// The two error classes are not the same kind of problem. An `Unsupported`
/// refusal is decided either before any byte is written or straight after a
/// LIST that completed normally, so the session is untouched and reconnecting
/// would achieve nothing but churn (and this is a user-repeatable action — a
/// server with no Trash folder would otherwise turn every click into a
/// reconnect). Anything else may have left a response unread on the stream,
/// where every later command reads the wrong tag, so the connection goes.
async fn handle_delete(conn: &mut MailConnection, request: DeleteRequest) -> DeleteExit {
    let result = conn.move_to_trash(request.uid).await;
    let exit = match &result {
        Ok(()) => DeleteExit::Continue,
        Err(MailClientError::Unsupported(_)) => DeleteExit::Continue,
        Err(_) => DeleteExit::Reconnect,
    };
    if let Err(e) = &result {
        log::warn!("mail: could not move uid={} to Trash: {e}", request.uid);
    }
    // A dropped receiver means the caller already gave up (it times out); the
    // move still happened, and reconciliation will catch the local row up.
    let _ = request.reply.send(result.map_err(|e| e.to_string()));
    exit
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
    // Clamped before shifting: an over-wide shift panics in debug and silently
    // masks the shift amount (so `<< 64` becomes `<< 0`, i.e. no delay at all)
    // in release. A long outage reaches counts that large easily.
    let factor = 1u64 << consecutive_failures.min(32);
    Duration::from_secs(RECONNECT_BASE_DELAY.as_secs().saturating_mul(factor)).min(cap)
}

async fn sync_and_persist(
    app: &AppHandle,
    account: &MailAccountConfig,
    conn: &mut MailConnection,
    reconcile: bool,
    shutdown: &watch::Receiver<bool>,
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

    // The last free moment before the network phase. Everything from here to
    // the batch loop is IMAP I/O — a SELECT and a UID SEARCH worth up to
    // `FETCH_TIMEOUT` each, then up to `TOTAL_FETCH_BUDGET` of UID FETCHes —
    // and `resolve()` above can itself have just spent a round trip refreshing
    // an OAuth token, which is exactly the window a stop lands in.
    if *shutdown.borrow() {
        return Ok(());
    }

    // `should_stop` is what makes the rest of that phase abandonable: without
    // it the check above is the last one for up to two minutes, far past the
    // graceful-stop budget, so the task gets aborted and the session is left on
    // the server with no LOGOUT.
    let outcome = conn.sync(since_uid, stored_uid_validity, reconcile, || *shutdown.borrow()).await?;

    // A stop landed between two fetch batches. Treated exactly like the
    // mid-classification stop further down: return *without*
    // `set_last_seen_uid`, because the UIDs this sync planned to fetch and
    // never did sit below batches it did fetch, and advancing the cursor over
    // them would skip them permanently. Nothing has been classified or
    // inserted yet, so there is nothing to keep — the fetched batches are
    // simply fetched again on the next connection, at no LLM cost.
    // Reconciliation is skipped too (`server_uids` is already `None` for an
    // interrupted fetch): a partial view of the mailbox is not something to
    // reconcile against.
    if outcome.interrupted {
        return Ok(());
    }

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
        let mut stopped_mid_batch = false;
        for raw in batch.messages {
            // The only cancellation point inside a sync, and it has to be here
            // rather than between batches: every message below is an LLM call,
            // so one batch alone can outlast the whole graceful-stop budget.
            if *shutdown.borrow() {
                stopped_mid_batch = true;
                break;
            }

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

        if stopped_mid_batch {
            // Deliberately returns *without* `set_last_seen_uid`: this batch's
            // `max_uid` covers messages we never got to, and advancing the
            // cursor past them would skip them permanently. That is the right
            // trade, but it is not a free one, and it is worth being honest
            // about the price. The messages this batch already inserted are
            // fetched again on the next connection, and `classify_message`
            // above runs *before* `insert_message` — so each duplicate costs a
            // fresh LLM call, and the insert that follows it is a plain INSERT
            // that then trips `UNIQUE(account_id, uid)` and surfaces as the
            // `failed to insert message uid=…` warning below, which reads like
            // a real error but is expected here. The waste is bounded at
            // `FETCH_BATCH_SIZE - 1` (9) redundant classifications, since every
            // earlier batch has already committed its cursor. Skipping mail
            // permanently is the worse outcome by a wide margin.
            //
            // Skips the reconciliation too — a partial view of the mailbox is
            // not something to reconcile against.
            return Ok(());
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

/// The one way a removal reaches the UI. Shared with `mail_delete_message` on
/// purpose: a user-initiated delete and a reconciliation both simply mean
/// "cached rows for this account went away", and the Mail list's refetch and
/// the unread badge's refresh already both hang off this single event.
pub fn emit_removed(app: &AppHandle, account_id: &str, removed_count: u64) {
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
            should_reconcile(SyncTrigger::ReconcileDue, Duration::ZERO, INTERVAL),
            "the deadline wake exists only to reconcile; skipping it would make the wake pointless"
        );
        assert!(
            should_reconcile(SyncTrigger::PollInterval, Duration::ZERO, INTERVAL),
            "the non-IDLE fallback must keep reconciling on every poll, exactly as the old timer-based poller did"
        );
    }

    // The case the previous round got wrong: one notification early in a
    // connection, then silence. `should_reconcile` alone never fires again
    // (elapsed stays below the interval and nothing re-evaluates it), and the
    // liveness timeout cannot be the backstop because keepalives reset it. The
    // deadline handed to the next IDLE is what actually bounds the staleness.
    #[test]
    fn a_mailbox_that_goes_quiet_after_a_notification_still_reconciles_within_one_interval() {
        let after_notification = Duration::from_secs(30);

        assert!(
            !should_reconcile(SyncTrigger::IdleNotification, after_notification, INTERVAL),
            "the notification itself must stay cheap"
        );
        assert_eq!(
            time_until_reconcile(after_notification, INTERVAL),
            Duration::from_secs(270),
            "so the IDLE it re-enters must be woken for the remainder of the interval, not left to a liveness timeout keepalives keep pushing out"
        );
    }

    #[test]
    fn an_overdue_reconciliation_wakes_the_next_idle_immediately() {
        assert_eq!(
            time_until_reconcile(INTERVAL * 2, INTERVAL),
            Duration::ZERO,
            "a deadline already passed must not wait out another whole interval"
        );
    }

    #[test]
    fn a_session_that_dies_immediately_counts_as_a_failure_however_much_it_did_first() {
        // The IDLE-broken case: connect, sync fine, then IDLE fails at once,
        // over and over. Counting the successful sync as progress pinned the
        // delay at 5s forever — a login and an abandoned session every 5s,
        // which caps a Gmail account inside about 75 seconds.
        assert_eq!(
            next_failure_count(1, steady_lifetime_of(Duration::from_secs(1))),
            2,
            "a session that reached steady state and lost it a second later has not proved anything works"
        );
    }

    #[test]
    fn a_session_that_ran_for_a_while_clears_the_backoff() {
        assert_eq!(
            next_failure_count(7, steady_lifetime_of(Duration::from_secs(3600))),
            0,
            "an hour of uptime means the next failure is a fresh one, not a continuing outage"
        );
        assert_eq!(
            // The exact boundary, so passed straight in rather than through
            // `steady_lifetime_of`, whose `elapsed()` would land just above it.
            next_failure_count(7, HEALTHY_SESSION_LIFETIME),
            0,
            "the threshold itself counts as healthy"
        );
    }

    // The residual the previous round left behind. The session clock used to
    // start at login, so it included the startup sync — and the slow failures
    // live in the startup sync. A server whose `SELECT INBOX` or `UID FETCH`
    // hangs until `FETCH_TIMEOUT` (30s) produced a "lifetime" of 30s for a
    // connection that never worked once, which cleared the counter and left the
    // delay at 5s: one login every ~35 seconds, forever, with the backoff never
    // growing at all.
    #[test]
    fn a_sync_that_hangs_until_it_times_out_never_counted_as_uptime() {
        let never_reached_steady_state = steady_lifetime(None);

        assert_eq!(
            never_reached_steady_state,
            Duration::ZERO,
            "an exit before the wait loop must report nothing, however long the attempt took"
        );

        let mut failures = 0u32;
        let delays: Vec<Duration> = (0..4)
            .map(|_| {
                failures = next_failure_count(failures, never_reached_steady_state);
                reconnect_delay(failures, INTERVAL)
            })
            .collect();
        assert_eq!(
            delays,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(40),
                Duration::from_secs(80),
            ],
            "a sync that fails slowly must back off exactly like one that fails fast; a flat 5s here is a login storm"
        );
    }

    // The other half of the same residual: a first sync on a large mailbox over
    // a slow link can take longer than HEALTHY_SESSION_LIFETIME all by itself,
    // so a login-timed clock read a connection that died the instant it reached
    // IDLE as twelve healthy seconds.
    #[test]
    fn a_slow_startup_sync_does_not_count_towards_the_healthy_threshold() {
        // Logged in 12 seconds ago, all of it spent in the startup sync, and
        // IDLE fails the instant that sync returns.
        let logged_in_at = Instant::now() - Duration::from_secs(12);
        let steady_state_reached_at = Instant::now();
        // Precondition, not a claim about the code under test: this only
        // reproduces the bug if the old login-timed clock would have called
        // this session healthy.
        assert!(logged_in_at.elapsed() >= HEALTHY_SESSION_LIFETIME);

        let lifetime = steady_lifetime(Some(steady_state_reached_at));

        assert!(
            lifetime < HEALTHY_SESSION_LIFETIME,
            "the clock must start at steady state, so a 12s sync followed by an instant IDLE failure is not 12s of health"
        );
        assert_eq!(
            next_failure_count(0, lifetime),
            1,
            "and the attempt must therefore count as a failure"
        );
    }

    /// A session that reached steady state `held` ago.
    fn steady_lifetime_of(held: Duration) -> Duration {
        steady_lifetime(Some(Instant::now() - held))
    }

    #[test]
    fn the_failure_count_saturates_instead_of_wrapping() {
        assert_eq!(
            next_failure_count(u32::MAX, Duration::ZERO),
            u32::MAX,
            "wrapping to 0 would silently reset the backoff during the longest outages"
        );
    }

    #[test]
    fn a_connection_that_keeps_dying_on_arrival_backs_off_instead_of_retrying_every_five_seconds() {
        let mut failures = 0u32;
        let delays: Vec<Duration> = (0..5)
            .map(|_| {
                // Each attempt: connects, syncs, dies a second later.
                failures = next_failure_count(failures, Duration::from_secs(1));
                reconnect_delay(failures, INTERVAL)
            })
            .collect();

        assert_eq!(
            delays,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(20),
                Duration::from_secs(40),
                Duration::from_secs(80),
                Duration::from_secs(160),
            ],
            "a persistently broken connection must escalate; a flat 5s here is a login-and-abandon storm against the provider"
        );
    }

    // Fix 1's whole point: a wrong password, a revoked App Password or a dead
    // network used to reach the user as nothing but a `log::warn!` in a log
    // already full of unrelated heartbeat warnings.
    #[test]
    fn the_first_failure_is_reported_to_the_user() {
        assert_eq!(
            health_report(false, false),
            HealthReport::Failed,
            "an account that stops reaching its server must say so; the Mail tab otherwise just stops updating"
        );
    }

    // The property that keeps the diagnostic from becoming noise. A failing
    // account retries every 5s at the start of an outage, so reporting per
    // attempt would mean an event and a banner repaint at that rate for as long
    // as the outage lasts.
    #[test]
    fn a_failing_account_is_reported_once_and_not_once_per_retry() {
        let mut failing = false;
        let mut reports = Vec::new();
        // Ten consecutive failed attempts — a plain outage.
        for _ in 0..10 {
            let report = health_report(failing, false);
            if report != HealthReport::Quiet {
                failing = report == HealthReport::Failed;
            }
            reports.push(report);
        }

        // Asserted as the whole sequence rather than as a count plus a spot
        // check: "how many" and "which one" are one fact here, and splitting
        // them would leave the second assertion unable to fail on its own.
        let mut expected = vec![HealthReport::Quiet; 10];
        expected[0] = HealthReport::Failed;
        assert_eq!(
            reports, expected,
            "the first attempt reports, and the nine retries behind it say nothing"
        );
    }

    #[test]
    fn recovery_is_reported_so_the_banner_can_clear() {
        assert_eq!(
            health_report(true, true),
            HealthReport::Recovered,
            "a failure the user can never see the end of is worse than no failure report at all"
        );
    }

    // Also the ordinary-reconnect case — a server hanging up a long-lived
    // socket, a laptop waking. The next attempt reaches steady state, so the
    // health never left "working" and no error may flash at a user whose mail
    // is arriving perfectly normally.
    #[test]
    fn an_account_that_is_working_and_stays_working_says_nothing() {
        assert_eq!(
            health_report(false, true),
            HealthReport::Quiet,
            "every successful sync — and every ordinary reconnect — would otherwise emit an event"
        );
    }

    #[test]
    fn an_outage_that_ends_and_returns_is_reported_each_time() {
        let mut failing = false;
        let mut events = Vec::new();
        // down, down, up, up, down, up
        for working in [false, false, true, true, false, true] {
            let report = health_report(failing, working);
            match report {
                HealthReport::Quiet => {}
                HealthReport::Failed => { failing = true; events.push(report); }
                HealthReport::Recovered => { failing = false; events.push(report); }
            }
        }

        assert_eq!(
            events,
            vec![
                HealthReport::Failed,
                HealthReport::Recovered,
                HealthReport::Failed,
                HealthReport::Recovered,
            ],
            "each crossing must be reported exactly once, and none may be swallowed"
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

// src-tauri/src/mail/client.rs
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use futures_util::TryStreamExt;

/// Ceiling on a single IMAP round trip (login, SELECT, SEARCH, one UID FETCH).
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Ceiling on the whole batch loop. Applied *between* batches, so it never
/// discards a batch that already succeeded: once the budget is spent the loop
/// simply stops starting new batches and returns what it has, and the
/// remaining UIDs are picked up by the next poll.
const TOTAL_FETCH_BUDGET: Duration = Duration::from_secs(120);

/// How many messages the *first* poll of an account pulls down. A brand new
/// account has no `since_uid`, and "everything in the mailbox" is not a
/// workable answer on a real INBOX: the UID set would overflow the IMAP
/// command-length limit (Gmail answers BAD) and the bodies could never be
/// downloaded inside `FETCH_TIMEOUT`, so the first poll would fail, never
/// record a `last_seen_uid`, and repeat that identical failure forever.
/// Seeding from the newest N messages instead gives the user immediate
/// content and bounded work.
const FIRST_POLL_MAX_MESSAGES: usize = 50;

/// UIDs per `UID FETCH` command. Keeps each command short and each round
/// trip inside the timeout, and is the unit at which the poller persists
/// `last_seen_uid`. Deliberately well below `FIRST_POLL_MAX_MESSAGES`: when
/// the two were equal the first poll was a single batch, which made the
/// batching machinery inert on exactly the sync it exists to protect — one
/// slow fetch and the whole seed was lost, forever, every cycle.
const FETCH_BATCH_SIZE: usize = 10;

type ImapSession = async_imap::Session<tokio_native_tls::TlsStream<TcpStream>>;

#[derive(Debug, thiserror::Error)]
pub enum MailClientError {
    #[error("connection error: {0}")]
    Connect(String),
    #[error("login failed: {0}")]
    Login(String),
    #[error("IMAP command failed: {0}")]
    Command(String),
    #[error("IMAP operation timed out after {0:?}")]
    Timeout(Duration),
}

pub struct RawMessage {
    pub uid: i64,
    pub raw: Vec<u8>,
}

/// One `UID FETCH` worth of messages, plus the highest UID that batch
/// *asked* for. The caller advances `last_seen_uid` to `max_uid` — not to
/// the highest UID it actually got a body for — so a UID the server declines
/// to return (deleted between SEARCH and FETCH, say) is skipped once instead
/// of being re-requested on every future poll forever.
pub struct MessageBatch {
    pub max_uid: i64,
    pub messages: Vec<RawMessage>,
}

/// Everything one poll learned from the mailbox.
pub struct PollOutcome {
    /// The mailbox's UIDVALIDITY as reported by SELECT, or `None` from a server
    /// that does not support UIDs at all.
    pub uid_validity: Option<i64>,
    /// The stored UIDVALIDITY existed and disagreed with the server's, so every
    /// UID the caller has cached for this account — `last_seen_uid` included —
    /// belongs to a previous incarnation of the mailbox and means nothing now.
    pub uid_validity_changed: bool,
    /// The complete `UID SEARCH ALL` result: every UID INBOX currently holds.
    ///
    /// `None` — not an empty vec — whenever that SEARCH did not come back
    /// cleanly. The caller deletes cached messages missing from this set, so
    /// collapsing a failed command into `Some(vec![])` would read as "the
    /// mailbox is empty" and destroy the entire local cache. The two cases are
    /// kept apart in the type so that mistake cannot be made downstream.
    pub server_uids: Option<Vec<i64>>,
    pub batches: Vec<MessageBatch>,
}

/// Fetch messages newer than `since_uid` from INBOX (or, on the first poll
/// for an account, the newest `FIRST_POLL_MAX_MESSAGES`), in batches of
/// `FETCH_BATCH_SIZE`, using `BODY.PEEK[]` so the server's `\Seen` flag is
/// never touched — AITerm tracks read/unread locally (see
/// `db/mail.rs::mark_read_locally`) so it doesn't clobber the read state the
/// user sees in their phone's mail app.
///
/// Batches come back oldest-first and are meant to be processed and committed
/// one at a time: if the fetch dies partway through a large sync, the batches
/// that already made it are returned as `Ok` (the failure is logged) so the
/// caller can commit them and make forward progress, rather than throwing
/// away work that succeeded and retrying it identically next cycle. Only a
/// failure on the *first* batch — where there is no progress to preserve —
/// surfaces as `Err`. Timeouts flow through that same path: they are applied
/// per round trip, never around the loop, because a timeout wrapped around
/// the loop drops the whole future and destroys every batch collected so far,
/// which is precisely the failure mode this batching exists to survive.
///
/// The returned `PollOutcome` also carries what the same session learned about
/// the mailbox as a whole — its UIDVALIDITY, and the full `UID SEARCH ALL` UID
/// set the caller reconciles server-side deletions against. Both ride along on
/// this one connection rather than opening a second.
pub async fn fetch_new_messages(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    since_uid: Option<i64>,
    stored_uid_validity: Option<i64>,
) -> Result<PollOutcome, MailClientError> {
    let mut session = connect_and_login(host, port, username, password).await?;

    // From here on, every exit path (success, error, or empty result) must
    // log out gracefully instead of dropping the session — an abrupt
    // non-LOGOUT socket close leaves state on the server side until its own
    // idle-timeout notices, and some providers (e.g. Gmail) cap concurrent
    // connections per account, so a recurring per-poll error here could
    // otherwise accumulate ungraceful terminations against the mail server.
    //
    // That is also why the timeouts live *inside* `fetch_selected`, around
    // each individual round trip, rather than wrapping this call from the
    // outside: an outer timeout drops the future mid-await, which skips the
    // logout below — and a hung connection is precisely the case that leaves
    // a dangling connection on the server. Timing out per round trip instead
    // produces an ordinary `Err` that flows through the same cleanup path
    // (and, for a batch fetch, through the partial-success path too).
    let result = fetch_selected(&mut session, since_uid, stored_uid_validity).await;
    logout(session).await;
    result
}

/// Verify that credentials actually work: connect, log in, and SELECT INBOX,
/// then log out. Used by `mail_test_connection` so the UI can tell the user
/// their password/App Password/IMAP setting is wrong at the moment they add
/// the account, instead of failing silently in the background poller.
pub async fn test_connection(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<(), MailClientError> {
    let mut session = connect_and_login(host, port, username, password).await?;
    let result = with_timeout(async {
        session
            .select("INBOX")
            .await
            .map(|_| ())
            .map_err(|e| MailClientError::Command(e.to_string()))
    })
    .await;
    logout(session).await;
    result
}

/// Bounded by the same timeout, but safe to abandon on expiry: no session
/// exists yet, so there is nothing to log out of.
async fn connect_and_login(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<ImapSession, MailClientError> {
    with_timeout(connect_and_login_inner(host, port, username, password)).await
}

async fn connect_and_login_inner(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<ImapSession, MailClientError> {
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| MailClientError::Connect(e.to_string()))?;

    let native_connector = native_tls::TlsConnector::new()
        .map_err(|e| MailClientError::Connect(e.to_string()))?;
    let connector = tokio_native_tls::TlsConnector::from(native_connector);
    let tls_stream = connector
        .connect(host, tcp)
        .await
        .map_err(|e| MailClientError::Connect(e.to_string()))?;

    let client = async_imap::Client::new(tls_stream);
    client
        .login(username, password)
        .await
        .map_err(|(e, _client)| MailClientError::Login(e.to_string()))
}

/// Best-effort graceful close. Bounded too, so a wedged connection can't hang
/// the poll loop in cleanup — if it does time out we simply drop the session.
async fn logout(mut session: ImapSession) {
    let _ = tokio::time::timeout(FETCH_TIMEOUT, session.logout()).await;
}

async fn with_timeout<T>(
    fut: impl Future<Output = Result<T, MailClientError>>,
) -> Result<T, MailClientError> {
    tokio::time::timeout(FETCH_TIMEOUT, fut)
        .await
        .unwrap_or(Err(MailClientError::Timeout(FETCH_TIMEOUT)))
}

async fn fetch_selected(
    session: &mut ImapSession,
    since_uid: Option<i64>,
    stored_uid_validity: Option<i64>,
) -> Result<PollOutcome, MailClientError> {
    let mailbox = with_timeout(async {
        session
            .select("INBOX")
            .await
            .map_err(|e| MailClientError::Command(e.to_string()))
    })
    .await?;
    let uid_validity = mailbox.uid_validity.map(|v| v as i64);

    // Decided here rather than by the caller because it changes the query we
    // are about to send: under a new UIDVALIDITY the stored `since_uid` indexes
    // into the old numbering, so honoring it would search `UID {stale+1}:*` in
    // a mailbox whose UIDs may have restarted at 1 — matching nothing, on
    // every poll, silently and forever. Re-seed from scratch instead.
    let uid_validity_changed = uid_validity_changed(stored_uid_validity, uid_validity);
    let effective_since_uid = if uid_validity_changed { None } else { since_uid };

    let search_query = match effective_since_uid {
        Some(uid) => format!("UID {}:*", uid + 1),
        None => "1:*".to_string(),
    };
    let uids = with_timeout(async {
        session
            .uid_search(&search_query)
            .await
            .map_err(|e| MailClientError::Command(e.to_string()))
    })
    .await?;

    let planned = plan_fetch_batches(uids.into_iter().collect(), effective_since_uid);

    let batches = collect_batches(session, planned, Instant::now() + TOTAL_FETCH_BUDGET).await?;

    // Deliberately *after* the fetch loop, never before: a UID SEARCH ALL taken
    // first would not list the messages this poll is about to insert, and the
    // caller — which deletes cached mail missing from this set — would erase
    // them the instant they landed. Taken last, everything we fetched was in
    // the mailbox at or before this point.
    let server_uids = search_all_uids(session).await;

    Ok(PollOutcome { uid_validity, uid_validity_changed, server_uids, batches })
}

/// Whether our cached UIDs belong to a different incarnation of the mailbox.
///
/// Only a stored value that disagrees with a reported one counts. No stored
/// value means a first poll (or a database upgraded from before the column
/// existed) — there is nothing to invalidate. No reported value means a server
/// that doesn't do UIDVALIDITY, which tells us nothing; treating that as
/// "changed" would wipe and re-download the whole cache on every poll.
fn uid_validity_changed(stored: Option<i64>, server: Option<i64>) -> bool {
    matches!((stored, server), (Some(stored), Some(server)) if stored != server)
}

/// `UID SEARCH ALL` — every UID INBOX currently holds, numbers only, no bodies.
///
/// A failure is `None`, and is logged and swallowed rather than propagated: the
/// messages this poll already fetched are worth committing even when
/// reconciliation can't run, and skipping one cycle's deletions costs nothing
/// but a stale row until the next poll.
async fn search_all_uids(session: &mut ImapSession) -> Option<Vec<i64>> {
    let result = with_timeout(async {
        session
            .uid_search("ALL")
            .await
            .map_err(|e| MailClientError::Command(e.to_string()))
    })
    .await;

    match result {
        Ok(uids) => Some(uids.into_iter().map(|uid| uid as i64).collect()),
        Err(e) => {
            log::warn!("mail: UID SEARCH ALL failed, skipping deletion reconciliation this cycle: {e}");
            None
        }
    }
}

/// One batch fetch. Abstracted over the session purely so the loop below —
/// which decides what survives a mid-sync failure — can be tested without a
/// live IMAP server.
trait BatchSource {
    async fn fetch_batch(&mut self, uid_batch: &[u32]) -> Result<Vec<RawMessage>, MailClientError>;
}

impl BatchSource for ImapSession {
    async fn fetch_batch(&mut self, uid_batch: &[u32]) -> Result<Vec<RawMessage>, MailClientError> {
        with_timeout(fetch_uid_batch(self, uid_batch)).await
    }
}

/// Run the planned batches in order, keeping every batch that succeeds.
///
/// A failing batch (including a timed-out one) stops the loop but does not
/// throw away its predecessors — they are returned as `Ok` so the caller can
/// commit them and advance `last_seen_uid` past them. Only a failure with no
/// progress behind it surfaces as `Err`. `deadline` bounds the total time the
/// same way: it is checked *between* batches, so it can delay progress but
/// never destroy it.
async fn collect_batches<S: BatchSource>(
    source: &mut S,
    planned: Vec<Vec<u32>>,
    deadline: Instant,
) -> Result<Vec<MessageBatch>, MailClientError> {
    let mut batches: Vec<MessageBatch> = Vec::new();
    for uid_batch in planned {
        // The first batch is always attempted: there is no progress to
        // protect yet, and returning `Ok(vec![])` for an untried fetch would
        // look to the caller like "no new mail".
        if !batches.is_empty() && Instant::now() >= deadline {
            log::warn!("mail: fetch budget exhausted after {} batch(es)", batches.len());
            break;
        }
        let max_uid = uid_batch.iter().copied().max().unwrap_or(0) as i64;
        match source.fetch_batch(&uid_batch).await {
            Ok(messages) => batches.push(MessageBatch { max_uid, messages }),
            Err(e) => {
                if batches.is_empty() {
                    return Err(e);
                }
                // Keep what already succeeded so the caller can commit it;
                // the remaining UIDs are simply picked up next poll.
                log::warn!("mail: fetch failed after {} batch(es): {e}", batches.len());
                break;
            }
        }
    }

    Ok(batches)
}

async fn fetch_uid_batch(
    session: &mut ImapSession,
    uid_batch: &[u32],
) -> Result<Vec<RawMessage>, MailClientError> {
    let uid_set = uid_batch.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
    let fetches = session
        .uid_fetch(&uid_set, "(UID BODY.PEEK[])")
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let mut messages = Vec::new();
    for fetch in fetches {
        if let (Some(uid), Some(body)) = (fetch.uid, fetch.body()) {
            messages.push(RawMessage { uid: uid as i64, raw: body.to_vec() });
        }
    }
    // A fetch item without a UID or a body is dropped, and since `max_uid` is
    // the highest *requested* UID it is dropped permanently — that is the
    // intended tradeoff (a message deleted between SEARCH and FETCH must not
    // wedge the poll loop forever), but it must not be silent, or a server
    // returning incomplete responses would lose mail with no trace at all.
    if messages.len() < uid_batch.len() {
        log::warn!(
            "mail: server returned {} of {} requested message(s); the missing UIDs are skipped permanently",
            messages.len(),
            uid_batch.len()
        );
    }
    Ok(messages)
}

/// Decide which UIDs to fetch, and in what batches, from the raw UID SEARCH
/// result. Pure so it can be tested without a live IMAP server.
///
/// Batches are ascending (oldest first) and never exceed `FETCH_BATCH_SIZE`.
/// On a first poll (`since_uid == None`) the *newest* `FIRST_POLL_MAX_MESSAGES`
/// are kept, since seeding an account should show recent mail, not the oldest
/// mail in the archive.
fn plan_fetch_batches(uids: Vec<u32>, since_uid: Option<i64>) -> Vec<Vec<u32>> {
    // Per RFC 3501, "UID {n}:*" can resolve to a *reversed* range when there
    // is no mail with UID >= n (the server may report the highest UID as
    // the "start" and n as the "end", re-including UID n itself). Re-check
    // against `since_uid` client-side so a mailbox with no new mail never
    // re-yields an already-seen UID and causes a duplicate refetch.
    let mut new_uids: Vec<u32> = uids
        .into_iter()
        .filter(|uid| since_uid.map_or(true, |since| (*uid as i64) > since))
        .collect();
    new_uids.sort_unstable();

    if since_uid.is_none() && new_uids.len() > FIRST_POLL_MAX_MESSAGES {
        new_uids.drain(..new_uids.len() - FIRST_POLL_MAX_MESSAGES);
    }

    new_uids.chunks(FETCH_BATCH_SIZE).map(|chunk| chunk.to_vec()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatten(batches: &[Vec<u32>]) -> Vec<u32> {
        batches.iter().flatten().copied().collect()
    }

    #[test]
    fn first_poll_keeps_only_the_newest_capped_window() {
        let uids: Vec<u32> = (1..=120).collect();

        let batches = plan_fetch_batches(uids, None);

        let planned = flatten(&batches);
        assert_eq!(planned.len(), FIRST_POLL_MAX_MESSAGES, "first poll must be capped");
        let newest_window: Vec<u32> = (121 - FIRST_POLL_MAX_MESSAGES as u32..=120).collect();
        assert_eq!(planned, newest_window, "the window must be the newest UIDs, not the oldest");
    }

    #[test]
    fn first_poll_below_the_cap_takes_everything() {
        let batches = plan_fetch_batches(vec![3, 1, 2], None);
        assert_eq!(flatten(&batches), vec![1, 2, 3]);
    }

    #[test]
    fn subsequent_poll_takes_everything_newer_than_since_uid_uncapped() {
        let uids: Vec<u32> = (1..=200).collect();

        let batches = plan_fetch_batches(uids, Some(10));

        let planned = flatten(&batches);
        assert!(
            planned.len() > FIRST_POLL_MAX_MESSAGES,
            "subsequent polls must not be capped, got {} UIDs",
            planned.len()
        );
        assert_eq!(planned, (11..=200).collect::<Vec<u32>>(), "every UID newer than since_uid must be planned");
    }

    #[test]
    fn subsequent_poll_drops_already_seen_uids() {
        // A reversed "UID n:*" range can re-include since_uid itself.
        assert!(
            plan_fetch_batches(vec![7, 8, 9], Some(9)).is_empty(),
            "nothing newer than since_uid means nothing to fetch"
        );
        assert_eq!(
            flatten(&plan_fetch_batches(vec![7, 8, 9], Some(7))),
            vec![8, 9],
            "since_uid itself must be excluded but everything above it kept"
        );
    }

    #[test]
    fn batches_never_exceed_the_chunk_size_and_stay_ascending() {
        let uids: Vec<u32> = (1..=120).collect();

        let batches = plan_fetch_batches(uids, Some(0));

        assert!(
            batches.iter().all(|b| b.len() <= FETCH_BATCH_SIZE && !b.is_empty()),
            "every batch must be non-empty and within {FETCH_BATCH_SIZE}: {:?}",
            batches.iter().map(|b| b.len()).collect::<Vec<_>>()
        );
        assert_eq!(flatten(&batches), (1..=120).collect::<Vec<u32>>(), "batches must cover every UID in ascending order");
    }

    #[test]
    fn empty_search_result_produces_no_batches() {
        assert!(plan_fetch_batches(Vec::new(), None).is_empty());
        assert!(plan_fetch_batches(Vec::new(), Some(42)).is_empty());
    }

    #[test]
    fn first_poll_is_split_into_many_batches_not_one_giant_fetch() {
        // The whole point of batching: the first sync must be committable in
        // pieces. Sizes are hard-coded rather than derived from the constants
        // so that making FETCH_BATCH_SIZE == FIRST_POLL_MAX_MESSAGES again —
        // which silently collapses the first poll back to a single
        // all-or-nothing fetch — fails here.
        let batches = plan_fetch_batches((1..=120).collect(), None);

        assert_eq!(batches.len(), 5, "a 50-message first poll must be 5 batches, got {:?}", batches.iter().map(|b| b.len()).collect::<Vec<_>>());
        assert!(batches.iter().all(|b| b.len() == 10), "each first-poll batch must hold 10 UIDs: {:?}", batches.iter().map(|b| b.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_uid_validity_that_disagrees_with_the_stored_one_invalidates_the_cache() {
        assert!(
            uid_validity_changed(Some(1), Some(2)),
            "a mailbox reporting a different UIDVALIDITY has renumbered; the cached UIDs mean nothing"
        );
    }

    #[test]
    fn a_matching_uid_validity_leaves_the_cache_alone() {
        assert!(
            !uid_validity_changed(Some(2), Some(2)),
            "the ordinary case must not wipe the cache and re-download the mailbox every poll"
        );
    }

    #[test]
    fn a_first_poll_with_nothing_stored_is_not_a_change() {
        assert!(
            !uid_validity_changed(None, Some(2)),
            "a first poll (or a database upgraded from before the column) has nothing to invalidate"
        );
    }

    #[test]
    fn a_server_that_reports_no_uid_validity_is_not_a_change() {
        assert!(
            !uid_validity_changed(Some(2), None),
            "a server that omits UIDVALIDITY tells us nothing; guessing 'changed' would wipe the cache every poll"
        );
        assert!(!uid_validity_changed(None, None));
    }

    fn msg(uid: i64) -> RawMessage {
        RawMessage { uid, raw: Vec::new() }
    }

    struct FakeSource {
        outcomes: Vec<Result<Vec<RawMessage>, MailClientError>>,
        requested: Vec<Vec<u32>>,
    }

    impl FakeSource {
        fn new(outcomes: Vec<Result<Vec<RawMessage>, MailClientError>>) -> Self {
            Self { outcomes, requested: Vec::new() }
        }
    }

    impl BatchSource for FakeSource {
        async fn fetch_batch(&mut self, uid_batch: &[u32]) -> Result<Vec<RawMessage>, MailClientError> {
            self.requested.push(uid_batch.to_vec());
            assert!(!self.outcomes.is_empty(), "collect_batches fetched more batches than the test scripted");
            self.outcomes.remove(0)
        }
    }

    fn far_future() -> Instant {
        Instant::now() + Duration::from_secs(3600)
    }

    #[tokio::test]
    async fn a_timed_out_batch_keeps_the_batches_that_already_succeeded() {
        let mut source = FakeSource::new(vec![
            Ok(vec![msg(1)]),
            Ok(vec![msg(2)]),
            Err(MailClientError::Timeout(FETCH_TIMEOUT)),
            Ok(vec![msg(4)]),
        ]);

        let batches = collect_batches(&mut source, vec![vec![1], vec![2], vec![3], vec![4]], far_future())
            .await
            .expect("a timeout after progress must not fail the whole fetch");

        assert_eq!(
            source.requested,
            vec![vec![1u32], vec![2], vec![3]],
            "the loop must stop at the failing batch instead of pressing on"
        );
        assert_eq!(
            batches.iter().map(|b| b.max_uid).collect::<Vec<_>>(),
            vec![1, 2],
            "both batches that succeeded before the timeout must survive it"
        );
    }

    #[tokio::test]
    async fn a_failure_on_the_very_first_batch_surfaces_as_an_error() {
        let mut source = FakeSource::new(vec![Err(MailClientError::Timeout(FETCH_TIMEOUT))]);

        let result = collect_batches(&mut source, vec![vec![1], vec![2]], far_future()).await;

        assert!(
            matches!(result, Err(MailClientError::Timeout(_))),
            "with no progress to preserve the failure must surface, not masquerade as an empty inbox"
        );
    }

    #[tokio::test]
    async fn an_exhausted_budget_stops_new_batches_but_keeps_the_finished_ones() {
        let mut source = FakeSource::new(vec![Ok(vec![msg(1)]), Ok(vec![msg(2)]), Ok(vec![msg(3)])]);
        let already_passed = Instant::now() - Duration::from_secs(1);

        let batches = collect_batches(&mut source, vec![vec![1], vec![2], vec![3]], already_passed)
            .await
            .expect("running out of budget is not an error");

        assert_eq!(source.requested, vec![vec![1u32]], "the first batch must run, and no batch may start after the budget is gone");
        assert_eq!(
            batches.iter().map(|b| b.max_uid).collect::<Vec<_>>(),
            vec![1],
            "the batch that finished must still be returned for the caller to commit"
        );
    }
}

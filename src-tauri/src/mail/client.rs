// src-tauri/src/mail/client.rs
use std::future::Future;
use std::time::Duration;
use tokio::net::TcpStream;
use futures_util::TryStreamExt;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

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
/// `last_seen_uid`.
const FETCH_BATCH_SIZE: usize = 50;

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
/// surfaces as `Err`.
pub async fn fetch_new_messages(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    since_uid: Option<i64>,
) -> Result<Vec<MessageBatch>, MailClientError> {
    let mut session = connect_and_login(host, port, username, password).await?;

    // From here on, every exit path (success, error, or empty result) must
    // log out gracefully instead of dropping the session — an abrupt
    // non-LOGOUT socket close leaves state on the server side until its own
    // idle-timeout notices, and some providers (e.g. Gmail) cap concurrent
    // connections per account, so a recurring per-poll error here could
    // otherwise accumulate ungraceful terminations against the mail server.
    //
    // That is also why the timeout is applied *inside* this function, to the
    // fetch work alone, rather than wrapping the whole function from the
    // outside: an outer timeout drops the future mid-await, which skips the
    // logout below — and a hung connection is precisely the case that leaves
    // a dangling connection on the server. Timing out here instead produces
    // an ordinary `Err` that flows through the same cleanup path.
    let result = with_timeout(fetch_selected(&mut session, since_uid)).await;
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
) -> Result<Vec<MessageBatch>, MailClientError> {
    session
        .select("INBOX")
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let search_query = match since_uid {
        Some(uid) => format!("UID {}:*", uid + 1),
        None => "1:*".to_string(),
    };
    let uids = session
        .uid_search(&search_query)
        .await
        .map_err(|e| MailClientError::Command(e.to_string()))?;

    let planned = plan_fetch_batches(uids.into_iter().collect(), since_uid);

    let mut batches: Vec<MessageBatch> = Vec::new();
    for uid_batch in planned {
        let max_uid = uid_batch.iter().copied().max().unwrap_or(0) as i64;
        match fetch_uid_batch(session, &uid_batch).await {
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
}

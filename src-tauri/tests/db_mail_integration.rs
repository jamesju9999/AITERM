// src-tauri/tests/db_mail_integration.rs
use sqlx::sqlite::SqlitePoolOptions;
use aiterm_lib::db::mail::{
    init_schema, insert_message, list_messages, get_message, mark_read_locally, count_unread,
    get_last_seen_uid, set_last_seen_uid, delete_account_data, list_uids,
    get_uid_validity, set_uid_validity, reset_for_uid_validity,
    uids_absent_from_server, delete_messages_absent_from_server, NewMessage,
};

async fn empty_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory DB")
}

/// The real schema, so these tests break if a migration ever stops running.
async fn setup_pool() -> sqlx::SqlitePool {
    let pool = empty_pool().await;
    init_schema(&pool).await.expect("init_schema");
    pool
}

/// `mail_poll_state` exactly as the shipped release created it — no
/// `uid_validity` column. Stands in for the user's installed `mail.db`.
async fn setup_pre_uid_validity_pool() -> sqlx::SqlitePool {
    let pool = empty_pool().await;

    sqlx::query(
        "CREATE TABLE mail_poll_state (
            account_id TEXT PRIMARY KEY NOT NULL,
            last_seen_uid INTEGER,
            last_polled_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("create legacy mail_poll_state table");

    sqlx::query(
        "CREATE TABLE mail_messages (
            id TEXT PRIMARY KEY NOT NULL,
            account_id TEXT NOT NULL,
            uid INTEGER NOT NULL,
            sender TEXT NOT NULL,
            subject TEXT NOT NULL,
            date TEXT,
            body_text TEXT NOT NULL,
            ai_summary TEXT,
            is_important INTEGER NOT NULL DEFAULT 0,
            is_promotional INTEGER NOT NULL DEFAULT 0,
            is_read_locally INTEGER NOT NULL DEFAULT 0,
            fetched_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(account_id, uid)
        )"
    ).execute(&pool).await.expect("create legacy mail_messages table");

    pool
}

async fn sorted_uids(pool: &sqlx::SqlitePool, account_id: &str) -> Vec<i64> {
    let mut uids = list_uids(pool, account_id).await.unwrap();
    uids.sort_unstable();
    uids
}

fn sample_message<'a>(account_id: &'a str, uid: i64) -> NewMessage<'a> {
    NewMessage {
        account_id,
        uid,
        sender: "alice@example.com",
        subject: "Test Subject",
        date: Some("2026-08-04T00:00:00Z"),
        body_text: "Hello world.",
        ai_summary: Some("A test email."),
        is_important: true,
        is_promotional: false,
    }
}

#[tokio::test]
async fn insert_and_list_messages() {
    let pool = setup_pool().await;
    let inserted = insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    assert_eq!(inserted.sender, "alice@example.com");
    assert!(inserted.is_important);
    assert!(!inserted.is_read_locally);

    let messages = list_messages(&pool, "acct-1").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, inserted.id);
}

#[tokio::test]
async fn mark_read_locally_flips_flag_only_for_that_message() {
    let pool = setup_pool().await;
    let m1 = insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    let m2 = insert_message(&pool, sample_message("acct-1", 2)).await.unwrap();

    mark_read_locally(&pool, &m1.id).await.unwrap();

    let m1_reloaded = get_message(&pool, &m1.id).await.unwrap().unwrap();
    let m2_reloaded = get_message(&pool, &m2.id).await.unwrap().unwrap();
    assert!(m1_reloaded.is_read_locally);
    assert!(!m2_reloaded.is_read_locally);
}

#[tokio::test]
async fn count_unread_counts_across_accounts() {
    let pool = setup_pool().await;
    insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    let m2 = insert_message(&pool, sample_message("acct-2", 1)).await.unwrap();
    mark_read_locally(&pool, &m2.id).await.unwrap();

    assert_eq!(count_unread(&pool).await.unwrap(), 1);
}

#[tokio::test]
async fn poll_state_roundtrips_and_upserts() {
    let pool = setup_pool().await;
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), None);

    set_last_seen_uid(&pool, "acct-1", 5).await.unwrap();
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(5));

    // Second call must UPDATE, not fail on the PRIMARY KEY.
    set_last_seen_uid(&pool, "acct-1", 9).await.unwrap();
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(9));
}

#[tokio::test]
async fn delete_account_data_removes_messages_and_poll_state() {
    let pool = setup_pool().await;
    insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    set_last_seen_uid(&pool, "acct-1", 1).await.unwrap();

    delete_account_data(&pool, "acct-1").await.unwrap();

    assert!(list_messages(&pool, "acct-1").await.unwrap().is_empty());
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), None);
}

#[tokio::test]
async fn unique_constraint_rejects_duplicate_uid_for_same_account() {
    let pool = setup_pool().await;
    insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    let result = insert_message(&pool, sample_message("acct-1", 1)).await;
    assert!(result.is_err(), "duplicate (account_id, uid) must be rejected");
}

// ---------------------------------------------------------------------------
// Deletion reconciliation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reconciliation_removes_exactly_the_uids_the_server_no_longer_has() {
    let pool = setup_pool().await;
    for uid in [1, 2, 3] {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }

    let removed = delete_messages_absent_from_server(&pool, "acct-1", &[1, 3]).await.unwrap();

    assert_eq!(removed, 1, "only the one UID missing from the server set may be deleted");
    assert_eq!(sorted_uids(&pool, "acct-1").await, vec![1, 3], "the UIDs the server still has must survive");
}

#[tokio::test]
async fn reconciliation_only_touches_the_account_it_was_given() {
    let pool = setup_pool().await;
    insert_message(&pool, sample_message("acct-1", 7)).await.unwrap();
    // Same UID under a different account: the server set below belongs to
    // acct-1 only and says nothing about acct-2's mailbox.
    insert_message(&pool, sample_message("acct-2", 7)).await.unwrap();

    delete_messages_absent_from_server(&pool, "acct-1", &[99]).await.unwrap();

    assert_eq!(sorted_uids(&pool, "acct-2").await, vec![7], "another account's mail must be untouched");
}

/// The disaster case. A `UID SEARCH ALL` that comes back empty — a server
/// hiccup, a parser change, an unexpected reply — must never be read as "the
/// whole mailbox was deleted". Losing the cache here is unrecoverable: the
/// bodies are gone and `last_seen_uid` keeps the poller from re-fetching them.
#[tokio::test]
async fn an_empty_server_uid_set_deletes_nothing() {
    let pool = setup_pool().await;
    for uid in [1, 2, 3] {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }

    let removed = delete_messages_absent_from_server(&pool, "acct-1", &[]).await.unwrap();

    assert_eq!(removed, 0, "an empty server set must delete nothing");
    assert_eq!(sorted_uids(&pool, "acct-1").await, vec![1, 2, 3], "the cache must survive an empty server set intact");
}

#[test]
fn uids_absent_from_server_refuses_to_condemn_anything_on_an_empty_server_set() {
    assert!(
        uids_absent_from_server(&[1, 2, 3], &[]).is_empty(),
        "an empty server set must condemn nothing, however many UIDs are cached"
    );
}

#[test]
fn uids_absent_from_server_condemns_only_the_missing_ones() {
    assert_eq!(
        uids_absent_from_server(&[1, 2, 3, 4], &[2, 4]),
        vec![1, 3],
        "exactly the cached UIDs absent from the server set"
    );
    assert!(
        uids_absent_from_server(&[], &[1, 2]).is_empty(),
        "an empty cache has nothing to condemn"
    );
}

/// Our cache is deliberately a subset of the mailbox — the first poll keeps
/// only the newest 50. Those older server-side UIDs we never downloaded must
/// not make anything look deleted, and the ones we did keep must not be
/// condemned just for sitting at the top of the mailbox.
#[tokio::test]
async fn reconciliation_leaves_a_deliberate_subset_of_the_mailbox_alone() {
    let pool = setup_pool().await;
    for uid in 51..=100 {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }
    let server_uids: Vec<i64> = (1..=100).collect();

    let removed = delete_messages_absent_from_server(&pool, "acct-1", &server_uids).await.unwrap();

    assert_eq!(removed, 0, "caching only part of the mailbox must not read as deletions");
    assert_eq!(sorted_uids(&pool, "acct-1").await.len(), 50, "the cached window itself must survive intact");
}

/// Reconciliation must never rewind the poll cursor: `last_seen_uid` is what
/// stops the poller re-downloading (and re-classifying) mail, so dropping a
/// deleted row must not put its UID back in scope.
#[tokio::test]
async fn reconciliation_does_not_disturb_the_poll_cursor() {
    let pool = setup_pool().await;
    for uid in [1, 2, 3] {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }
    set_last_seen_uid(&pool, "acct-1", 3).await.unwrap();
    set_uid_validity(&pool, "acct-1", 55).await.unwrap();

    delete_messages_absent_from_server(&pool, "acct-1", &[1, 2]).await.unwrap();

    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(3), "last_seen_uid must survive a deletion");
    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(55), "uid_validity must survive a deletion");
}

/// The delete is chunked to stay under SQLite's bound-parameter limit, so a
/// removal spanning several chunks must run every one of them and total them
/// up — not stop at the first chunk, and not report only the last chunk's count.
#[tokio::test]
async fn a_bulk_removal_deletes_every_chunk_not_just_the_first() {
    let pool = setup_pool().await;
    for uid in 1..=1000 {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }

    // One survivor, so this is not the empty-server-set no-op path.
    let removed = delete_messages_absent_from_server(&pool, "acct-1", &[1]).await.unwrap();

    assert_eq!(removed, 999, "every chunk's deletions must be counted, not just the last chunk's");
    assert_eq!(sorted_uids(&pool, "acct-1").await, vec![1], "every chunk must actually be deleted, not just the first");
}

// ---------------------------------------------------------------------------
// UIDVALIDITY
// ---------------------------------------------------------------------------

#[tokio::test]
async fn uid_validity_roundtrips_and_upserts_without_disturbing_last_seen_uid() {
    let pool = setup_pool().await;
    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), None, "an unseen account has no stored UIDVALIDITY");

    set_last_seen_uid(&pool, "acct-1", 42).await.unwrap();
    set_uid_validity(&pool, "acct-1", 7).await.unwrap();
    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(7));
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(42), "recording UIDVALIDITY must not move the poll cursor");

    // The unchanged-value path runs on every single poll: it must UPDATE, not
    // fail on the PRIMARY KEY, and must still leave the cursor alone.
    set_uid_validity(&pool, "acct-1", 7).await.unwrap();
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(42));
}

#[tokio::test]
async fn set_uid_validity_creates_the_poll_state_row_on_a_first_poll() {
    let pool = setup_pool().await;

    set_uid_validity(&pool, "acct-1", 7).await.unwrap();

    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(7), "the first poll of a brand new account must record UIDVALIDITY");
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), None, "and must not invent a poll cursor");
}

#[tokio::test]
async fn reset_for_uid_validity_drops_the_cache_and_the_stale_poll_cursor() {
    let pool = setup_pool().await;
    for uid in [1, 2, 3] {
        insert_message(&pool, sample_message("acct-1", uid)).await.unwrap();
    }
    set_last_seen_uid(&pool, "acct-1", 5000).await.unwrap();
    set_uid_validity(&pool, "acct-1", 1).await.unwrap();

    let dropped = reset_for_uid_validity(&pool, "acct-1", 2).await.unwrap();

    assert_eq!(dropped, 3, "every cached message must be reported as dropped");
    assert!(list_messages(&pool, "acct-1").await.unwrap().is_empty(), "UIDs from the old mailbox incarnation must not linger");
    assert_eq!(
        get_last_seen_uid(&pool, "acct-1").await.unwrap(), None,
        "the stale cursor must be cleared, or the next poll searches UID 5001:* in a mailbox that restarted at 1 and finds nothing forever"
    );
    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(2), "the new UIDVALIDITY must be recorded");
}

#[tokio::test]
async fn reset_for_uid_validity_only_touches_the_account_it_was_given() {
    let pool = setup_pool().await;
    insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    insert_message(&pool, sample_message("acct-2", 1)).await.unwrap();
    set_last_seen_uid(&pool, "acct-2", 9).await.unwrap();

    reset_for_uid_validity(&pool, "acct-1", 2).await.unwrap();

    assert_eq!(list_messages(&pool, "acct-2").await.unwrap().len(), 1, "another account's mail must be untouched");
    assert_eq!(get_last_seen_uid(&pool, "acct-2").await.unwrap(), Some(9), "another account's cursor must be untouched");
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

/// `mail_poll_state` is created with CREATE TABLE IF NOT EXISTS, so an
/// already-installed `mail.db` never gains `uid_validity` from the CREATE. The
/// migration has to add it explicitly — and must not cost the user the live
/// data already in that file.
#[tokio::test]
async fn init_schema_adds_uid_validity_to_a_database_created_without_it() {
    let pool = setup_pre_uid_validity_pool().await;
    insert_message(&pool, sample_message("acct-1", 1)).await.unwrap();
    set_last_seen_uid(&pool, "acct-1", 4321).await.unwrap();

    init_schema(&pool).await.expect("migrating an installed database must succeed");

    assert_eq!(
        get_uid_validity(&pool, "acct-1").await.unwrap(), None,
        "the column must exist and read as 'not known yet' rather than failing the query"
    );
    assert_eq!(get_last_seen_uid(&pool, "acct-1").await.unwrap(), Some(4321), "the existing poll cursor must survive the upgrade");
    assert_eq!(list_messages(&pool, "acct-1").await.unwrap().len(), 1, "existing cached mail must survive the upgrade");

    set_uid_validity(&pool, "acct-1", 9).await.unwrap();
    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(9), "the migrated column must be writable");
}

/// `init_schema` runs on every launch, and SQLite has no
/// `ADD COLUMN IF NOT EXISTS` — a second run must not blow up on the column it
/// added the first time.
#[tokio::test]
async fn init_schema_is_idempotent_across_launches() {
    let pool = setup_pre_uid_validity_pool().await;
    init_schema(&pool).await.unwrap();
    set_uid_validity(&pool, "acct-1", 7).await.unwrap();

    init_schema(&pool).await.expect("a second launch must not fail on the already-added column");

    assert_eq!(get_uid_validity(&pool, "acct-1").await.unwrap(), Some(7), "re-running the migration must not clobber stored state");
}

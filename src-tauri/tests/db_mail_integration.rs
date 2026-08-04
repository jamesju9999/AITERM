// src-tauri/tests/db_mail_integration.rs
use sqlx::sqlite::SqlitePoolOptions;
use aiterm_lib::db::mail::{
    insert_message, list_messages, get_message, mark_read_locally, count_unread,
    get_last_seen_uid, set_last_seen_uid, delete_account_data, NewMessage,
};

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("Failed to create in-memory DB");

    sqlx::query(
        "CREATE TABLE mail_poll_state (
            account_id TEXT PRIMARY KEY NOT NULL,
            last_seen_uid INTEGER,
            last_polled_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ).execute(&pool).await.expect("create mail_poll_state table");

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
    ).execute(&pool).await.expect("create mail_messages table");

    pool
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

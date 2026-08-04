# AI 信箱助理 — Phase 1（帳號管理 + 輪詢 + AI 摘要 + 重要性/廣告信判斷 + 通知）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add multi-account IMAP polling to AITerm that fetches new mail while the app is open, classifies each message with AI (summary / is_important / is_promotional) in one call, persists it to a new `mail.db` SQLite database, and notifies the user — OS notification for important mail, an unread-count badge on the Mail tab otherwise. Search, attachment parsing, and AI-drafted replies are **out of scope** for this plan (later phases per `docs/superpowers/specs/2026-08-04-ai-mail-assistant-design.md`).

**Architecture:** New backend module `src-tauri/src/mail/` (domain logic: IMAP client, RFC822 parsing, AI classification, per-account polling task) + `src-tauri/src/db/mail.rs` (SQLite) + `src-tauri/src/commands/mail.rs` (`#[tauri::command]` entry points), following the exact three-way split already used by `knowledge_base`/`db`/`commands`. Account metadata lives in `AppConfig.mail_accounts` (mirrors `db_connections`/`vcs_connections`); passwords live in the OS keychain via the existing `SecretStore`. One background tokio task per account is spawned/aborted through a `MailState { tasks: HashMap<account_id, JoinHandle<()>> }`, following `telegram::TelegramState`'s exact spawn/abort pattern. Frontend adds a new `"mail"` tab type (like `"knowledge-base"`) plus a Settings page for account CRUD.

**Tech Stack:** Rust/Tauri 2 backend (sqlx/SQLite, tokio), new deps `async-imap` (tokio runtime feature) + `tokio-native-tls` + `native-tls` for IMAP-over-TLS, `mail-parser` for RFC822 parsing, `tauri-plugin-notification` for OS notifications. React 19 frontend, `@tauri-apps/plugin-notification` JS package.

**A note on external-API risk:** Every Rust code sample below that touches `async-imap` (Task 8) was verified against that crate's current docs.rs pages, but `async-imap` doesn't publish a single canonical "connect with tokio + native-tls" example — the TLS wiring in Task 8 is my best-verified reconstruction from the `Client::new(stream)` + `tokio-native-tls` docs, not a copied example. If `cargo check` in Task 8 reports a signature mismatch, that is expected friction with a fast-moving external crate, not a plan error — fix it against the version that actually resolves in `Cargo.lock` and continue.

---

## Task 1: Add dependencies (Rust + npm), no application code yet

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `src-tauri/src/lib.rs:222-230` (plugin registration)
- Modify: `package.json`

- [ ] **Step 1: Add Rust dependencies**

In `src-tauri/Cargo.toml`, add to the `[dependencies]` section (alongside the existing `reqwest`/`keyring`/`sqlx` lines):

```toml
async-imap = { version = "0.11", default-features = false, features = ["runtime-tokio"] }
tokio-native-tls = "0.3"
native-tls = "0.2"
mail-parser = "0.11"
tauri-plugin-notification = "2"
```

- [ ] **Step 2: Register the notification plugin**

In `src-tauri/src/lib.rs`, find the plugin chain (currently lines 222-230):

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
```

Add one line after `tauri_plugin_process::init()`:

```rust
        .plugin(tauri_plugin_notification::init())
```

- [ ] **Step 3: Add the notification permission**

In `src-tauri/capabilities/default.json`, add `"notification:default"` to the `permissions` array (after `"process:allow-restart"`):

```json
    "process:allow-restart",
    "notification:default"
```

- [ ] **Step 4: Add the JS package**

Run:
```bash
npm install @tauri-apps/plugin-notification
```
Expected: `package.json` gains a `"@tauri-apps/plugin-notification": "^2.x.x"` line under `dependencies`.

- [ ] **Step 5: Verify it all compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles cleanly (no application code references the new crates yet, so this only validates the dependency graph resolves).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/capabilities/default.json src-tauri/src/lib.rs package.json package-lock.json
git commit -m "chore(mail): add imap/mail-parser/notification dependencies"
```

---

## Task 2: Account config storage (`AppConfig.mail_accounts`)

**Files:**
- Modify: `src-tauri/src/config/types.rs`
- Modify: `src-tauri/src/config/mod.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src-tauri/src/config/mod.rs` (right after the existing `db_connection_crud` test, ~line 278):

```rust
    #[test]
    fn mail_account_crud() {
        use crate::config::types::MailAccountConfig;
        let (store, _) = temp_store();

        let account = MailAccountConfig {
            id: "acct-1".into(),
            email: "me@example.com".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
            username: "me@example.com".into(),
            poll_interval_secs: 300,
        };

        store.add_mail_account(account.clone()).unwrap();
        assert_eq!(store.get().mail_accounts.len(), 1);
        assert_eq!(store.get().mail_accounts[0].email, "me@example.com");

        store.remove_mail_account("acct-1").unwrap();
        assert!(store.get().mail_accounts.is_empty());
    }

    #[test]
    fn app_config_has_mail_accounts_default() {
        let cfg = AppConfig::default();
        assert!(cfg.mail_accounts.is_empty());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib config:: -- --nocapture`
Expected: FAIL to compile — `MailAccountConfig` and `add_mail_account`/`remove_mail_account` don't exist yet.

- [ ] **Step 3: Add `MailAccountConfig` to `config/types.rs`**

Add near `VcsConnection` (after line 337):

```rust
/// A saved mail account (IMAP/SMTP). Password lives in Keychain under "mail:{id}".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailAccountConfig {
    pub id: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    /// How often to poll while the app is open. Defaults to 5 minutes.
    #[serde(default = "default_mail_poll_interval_secs")]
    pub poll_interval_secs: u32,
}

fn default_mail_poll_interval_secs() -> u32 { 300 }
```

Add the field to `AppConfig` (after `vcs_connections`, ~line 56):

```rust
    /// Saved mail accounts (passwords stored separately in Keychain).
    #[serde(default)]
    pub mail_accounts: Vec<MailAccountConfig>,
```

Add to `impl Default for AppConfig` (after `vcs_connections: vec![],`, ~line 135):

```rust
            mail_accounts: vec![],
```

- [ ] **Step 4: Add CRUD methods to `ConfigStore` in `config/mod.rs`**

Add after `remove_vcs_connection` (~line 97):

```rust
    /// Add a new mail account config.
    pub fn add_mail_account(&self, account: MailAccountConfig) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.mail_accounts.push(account);
        })
    }

    /// Remove a mail account by id.
    pub fn remove_mail_account(&self, id: &str) -> anyhow::Result<()> {
        self.update(|cfg| {
            cfg.mail_accounts.retain(|a| a.id != id);
        })
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib config:: -- --nocapture`
Expected: PASS, including `mail_account_crud` and `app_config_has_mail_accounts_default`, plus all pre-existing config tests still passing (`app_config_default_is_empty`, `db_connection_crud`, etc — confirms the new field's `#[serde(default)]` doesn't break TOML round-tripping of old config files).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/config/types.rs src-tauri/src/config/mod.rs
git commit -m "feat(mail): add MailAccountConfig to AppConfig with add/remove"
```

---

## Task 3: `mail.db` SQLite layer

**Files:**
- Create: `src-tauri/src/db/mail.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Create: `src-tauri/tests/db_mail_integration.rs`

- [ ] **Step 1: Write the failing integration test**

Create `src-tauri/tests/db_mail_integration.rs` (mirrors `src-tauri/tests/db_knowledge_base_integration.rs`'s `setup_pool` pattern exactly):

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test db_mail_integration`
Expected: FAIL to compile — `aiterm_lib::db::mail` module doesn't exist yet.

- [ ] **Step 3: Create `src-tauri/src/db/mail.rs`**

```rust
// src-tauri/src/db/mail.rs
use sqlx::{SqlitePool, FromRow};
use sqlx::sqlite::SqliteConnectOptions;
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use std::fs;

pub struct MailDb {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct MailMessageRow {
    pub id: String,
    pub account_id: String,
    pub uid: i64,
    pub sender: String,
    pub subject: String,
    pub date: Option<String>,
    pub body_text: String,
    pub ai_summary: Option<String>,
    pub is_important: bool,
    pub is_promotional: bool,
    pub is_read_locally: bool,
    pub fetched_at: String,
}

/// Fields needed to insert a new message. Borrowed strings — the caller
/// (poller.rs) already owns the parsed data for the duration of the insert.
pub struct NewMessage<'a> {
    pub account_id: &'a str,
    pub uid: i64,
    pub sender: &'a str,
    pub subject: &'a str,
    pub date: Option<&'a str>,
    pub body_text: &'a str,
    pub ai_summary: Option<&'a str>,
    pub is_important: bool,
    pub is_promotional: bool,
}

impl MailDb {
    pub async fn new() -> Self {
        let app_data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("AITERM");
        fs::create_dir_all(&app_data_dir).ok();
        let db_path = app_data_dir.join("mail.db");
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap_or_else(|_| {
            SqlitePool::connect_lazy("sqlite::memory:").unwrap()
        });
        let db = Self { pool };
        db.init().await.ok();
        db
    }

    pub async fn init(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS mail_poll_state (
                account_id TEXT PRIMARY KEY NOT NULL,
                last_seen_uid INTEGER,
                last_polled_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )"
        ).execute(&self.pool).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS mail_messages (
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
        ).execute(&self.pool).await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_mail_messages_account ON mail_messages(account_id)")
            .execute(&self.pool).await?;

        Ok(())
    }
}

pub async fn insert_message(pool: &SqlitePool, msg: NewMessage<'_>) -> Result<MailMessageRow, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO mail_messages
            (id, account_id, uid, sender, subject, date, body_text, ai_summary, is_important, is_promotional)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(msg.account_id)
    .bind(msg.uid)
    .bind(msg.sender)
    .bind(msg.subject)
    .bind(msg.date)
    .bind(msg.body_text)
    .bind(msg.ai_summary)
    .bind(msg.is_important)
    .bind(msg.is_promotional)
    .execute(pool).await?;
    Ok(get_message(pool, &id).await?.expect("just-inserted row must exist"))
}

pub async fn get_message(pool: &SqlitePool, id: &str) -> Result<Option<MailMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MailMessageRow>(
        "SELECT id, account_id, uid, sender, subject, date, body_text, ai_summary,
                is_important, is_promotional, is_read_locally, fetched_at
         FROM mail_messages WHERE id = ?"
    ).bind(id).fetch_optional(pool).await
}

pub async fn list_messages(pool: &SqlitePool, account_id: &str) -> Result<Vec<MailMessageRow>, sqlx::Error> {
    sqlx::query_as::<_, MailMessageRow>(
        "SELECT id, account_id, uid, sender, subject, date, body_text, ai_summary,
                is_important, is_promotional, is_read_locally, fetched_at
         FROM mail_messages WHERE account_id = ? ORDER BY fetched_at DESC"
    ).bind(account_id).fetch_all(pool).await
}

pub async fn mark_read_locally(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mail_messages SET is_read_locally = 1 WHERE id = ?")
        .bind(id).execute(pool).await?;
    Ok(())
}

pub async fn count_unread(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM mail_messages WHERE is_read_locally = 0")
        .fetch_one(pool).await?;
    Ok(row.0)
}

pub async fn get_last_seen_uid(pool: &SqlitePool, account_id: &str) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "SELECT last_seen_uid FROM mail_poll_state WHERE account_id = ?"
    ).bind(account_id).fetch_optional(pool).await?;
    Ok(row.and_then(|(uid,)| uid))
}

pub async fn set_last_seen_uid(pool: &SqlitePool, account_id: &str, uid: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mail_poll_state (account_id, last_seen_uid, last_polled_at)
         VALUES (?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(account_id) DO UPDATE SET
            last_seen_uid = excluded.last_seen_uid,
            last_polled_at = CURRENT_TIMESTAMP"
    ).bind(account_id).bind(uid).execute(pool).await?;
    Ok(())
}

pub async fn delete_account_data(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM mail_messages WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM mail_poll_state WHERE account_id = ?")
        .bind(account_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
```

- [ ] **Step 4: Register the module**

In `src-tauri/src/db/mod.rs`, add after `pub mod kb_chat_sessions;` (line 12):

```rust
pub mod mail;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test db_mail_integration`
Expected: PASS — all 6 tests green, including `unique_constraint_rejects_duplicate_uid_for_same_account` (confirms the `UNIQUE(account_id, uid)` constraint prevents the poller from double-inserting a message it's already seen).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/db/mail.rs src-tauri/src/db/mod.rs src-tauri/tests/db_mail_integration.rs
git commit -m "feat(mail): add mail.db SQLite layer (messages + poll cursor)"
```

---

## Task 4: RFC822 message parsing (pure logic, no network)

**Files:**
- Create: `src-tauri/src/mail/mod.rs`
- Create: `src-tauri/src/mail/parse.rs`
- Modify: `src-tauri/src/lib.rs:1-16` (module declaration)
- Create: `src-tauri/tests/mail_parse.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/mail_parse.rs`:

```rust
// src-tauri/tests/mail_parse.rs
use aiterm_lib::mail::parse::parse_raw_message;

const SAMPLE_EML: &str = "From: Alice <alice@example.com>\r\n\
Subject: Test Subject\r\n\
Date: Mon, 1 Jan 2026 12:00:00 +0000\r\n\
Content-Type: text/plain\r\n\
\r\n\
Hello world.\r\n";

#[test]
fn parses_sender_subject_date_and_body() {
    let parsed = parse_raw_message(SAMPLE_EML.as_bytes()).expect("should parse");
    assert_eq!(parsed.sender, "alice@example.com");
    assert_eq!(parsed.subject, "Test Subject");
    assert!(parsed.date.is_some());
    assert!(parsed.body_text.contains("Hello world."));
}

#[test]
fn falls_back_to_html_body_when_no_plain_text_part() {
    let html_only = "From: Bob <bob@example.com>\r\n\
Subject: HTML only\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>Hi there</p>\r\n";
    let parsed = parse_raw_message(html_only.as_bytes()).expect("should parse");
    assert!(parsed.body_text.contains("Hi there"));
}

#[test]
fn missing_subject_falls_back_to_placeholder() {
    let no_subject = "From: Carol <carol@example.com>\r\n\
\r\n\
Body only.\r\n";
    let parsed = parse_raw_message(no_subject.as_bytes()).expect("should parse");
    assert_eq!(parsed.subject, "(no subject)");
}

#[test]
fn empty_bytes_return_none() {
    assert!(parse_raw_message(b"").is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test mail_parse`
Expected: FAIL to compile — `aiterm_lib::mail` module doesn't exist yet.

- [ ] **Step 3: Create `src-tauri/src/mail/mod.rs`**

```rust
// src-tauri/src/mail/mod.rs
pub mod parse;
```

- [ ] **Step 4: Create `src-tauri/src/mail/parse.rs`**

```rust
// src-tauri/src/mail/parse.rs
use mail_parser::MessageParser;

#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub sender: String,
    pub subject: String,
    pub date: Option<String>,
    pub body_text: String,
}

/// Parse a raw RFC822 byte slice into sender/subject/date/body. Returns
/// `None` only when mail-parser finds no headers at all (garbage input) —
/// see mail-parser's `MessageParser::parse` docs: "if no headers are found
/// None is returned".
pub fn parse_raw_message(raw: &[u8]) -> Option<ParsedMessage> {
    let message = MessageParser::default().parse(raw)?;

    let sender = message
        .from()
        .and_then(|addr| addr.first())
        .and_then(|a| a.address())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let subject = message
        .subject()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(no subject)".to_string());

    let date = message.date().map(|d| d.to_rfc3339());

    let body_text = message
        .body_text(0)
        .or_else(|| message.body_html(0))
        .map(|s| s.to_string())
        .unwrap_or_default();

    Some(ParsedMessage { sender, subject, date, body_text })
}
```

- [ ] **Step 5: Register the `mail` module in `lib.rs`**

In `src-tauri/src/lib.rs`, add to the `pub mod` list (after `pub mod knowledge_base;`, keeping alphabetical order):

```rust
pub mod mail;
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test mail_parse`
Expected: PASS — all 4 tests green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/mail/mod.rs src-tauri/src/mail/parse.rs src-tauri/src/lib.rs src-tauri/tests/mail_parse.rs
git commit -m "feat(mail): parse RFC822 messages into sender/subject/date/body"
```

---

## Task 5: Expose `extract_json_from_response` for reuse

**Files:**
- Modify: `src-tauri/src/commands/ai.rs:50`

- [ ] **Step 1: Widen visibility**

In `src-tauri/src/commands/ai.rs`, change line 50 from:

```rust
fn extract_json_from_response(raw: &str) -> String {
```

to:

```rust
pub(crate) fn extract_json_from_response(raw: &str) -> String {
```

This is the only change — the function body is untouched. It's needed so `mail/classify.rs` (Task 6) can reuse the same "strip `<think>` blocks / markdown fences / leading prose" defensive parsing that `ai_query` already relies on, instead of duplicating ~50 lines of parsing logic.

- [ ] **Step 2: Verify nothing broke**

Run: `cd src-tauri && cargo test --lib ai:: && cargo test --test ai_query_command`
Expected: PASS — pure visibility change, existing behavior is identical.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "refactor(ai): widen extract_json_from_response to pub(crate) for reuse by mail classifier"
```

---

## Task 6: AI classification (summary + is_important + is_promotional)

**Files:**
- Create: `src-tauri/src/mail/classify.rs`
- Modify: `src-tauri/src/mail/mod.rs`
- Create: `src-tauri/tests/mail_classify.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/tests/mail_classify.rs` (mirrors the `MockProvider` in `src-tauri/tests/ai_query_command.rs`):

```rust
// src-tauri/tests/mail_classify.rs
use aiterm_lib::ai::{AiError, AiProvider, GenerateChunk, GenerateRequest};
use aiterm_lib::mail::classify::{classify_message, build_mail_classify_prompt};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

struct MockProvider {
    chunks: Vec<&'static str>,
}

#[async_trait]
impl AiProvider for MockProvider {
    fn id(&self) -> &str { "mock" }
    fn display_name(&self) -> &str { "Mock" }

    async fn generate(
        &self,
        _req: GenerateRequest,
        tx: mpsc::Sender<GenerateChunk>,
    ) -> Result<(), AiError> {
        for (i, c) in self.chunks.iter().enumerate() {
            let done = i + 1 == self.chunks.len();
            let _ = tx.send(GenerateChunk { delta: c.to_string(), done, usage: None }).await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), AiError> { Ok(()) }
}

#[tokio::test]
async fn parses_valid_classification_json() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![r#"{"summary":"老闆問你今天能不能開會","is_important":true,"is_promotional":false}"#],
    });
    let result = classify_message(provider, "boss@example.com", "Quick meeting?", "Can we meet today?")
        .await
        .expect("classify ok");
    assert_eq!(result.summary, "老闆問你今天能不能開會");
    assert!(result.is_important);
    assert!(!result.is_promotional);
}

#[tokio::test]
async fn strips_markdown_fence_before_parsing() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec!["```json\n", r#"{"summary":"週年慶特賣","is_important":false,"is_promotional":true}"#, "\n```"],
    });
    let result = classify_message(provider, "deals@shop.com", "50% off everything!", "Sale sale sale")
        .await
        .expect("classify ok");
    assert!(result.is_promotional);
    assert!(!result.is_important);
}

#[tokio::test]
async fn missing_fields_default_to_false_and_empty_summary() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec![r#"{}"#],
    });
    let result = classify_message(provider, "a@b.com", "subj", "body").await.expect("classify ok");
    assert_eq!(result.summary, "");
    assert!(!result.is_important);
    assert!(!result.is_promotional);
}

#[tokio::test]
async fn malformed_json_is_an_error_not_a_panic() {
    let provider: Arc<dyn AiProvider> = Arc::new(MockProvider {
        chunks: vec!["not json at all"],
    });
    let result = classify_message(provider, "a@b.com", "subj", "body").await;
    assert!(result.is_err());
}

#[test]
fn prompt_is_deterministic_and_forbids_dual_classification() {
    let a = build_mail_classify_prompt();
    let b = build_mail_classify_prompt();
    assert_eq!(a, b);
    assert!(a.contains("is_important"));
    assert!(a.contains("is_promotional"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --test mail_classify`
Expected: FAIL to compile — `aiterm_lib::mail::classify` doesn't exist yet.

- [ ] **Step 3: Create `src-tauri/src/mail/classify.rs`**

```rust
// src-tauri/src/mail/classify.rs
use std::sync::Arc;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::ai::{AiError, AiProvider, ChatMessage, GenerateChunk, GenerateRequest, QueryMode};
use crate::commands::ai::extract_json_from_response;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MailClassification {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub is_important: bool,
    #[serde(default)]
    pub is_promotional: bool,
}

/// System prompt for the mail triage classifier. Pure function of no
/// arguments — every call produces byte-identical output, which is what
/// `prompt_is_deterministic_and_forbids_dual_classification` checks.
pub fn build_mail_classify_prompt() -> String {
    r#"You are an email triage assistant. Given an email's sender, subject, and body,
output ONLY a JSON object, no prose, no markdown fences, no extra keys.

Schema:
{
  "summary": "one or two sentence summary of the email, in Traditional Chinese (繁體中文)",
  "is_important": true or false,
  "is_promotional": true or false
}

Rules:
1. is_important is true only if the email requires the user's timely attention or action.
2. is_promotional is true if this is a marketing/advertising/newsletter email.
3. An email is never both is_important and is_promotional — promotional email is never important.
4. Automated notifications that need no action (e.g. "your package shipped") are not important."#.to_string()
}

/// Classify one email: summarize it and flag importance/promotional status
/// in a single AI call. Truncates the body to ~4000 chars before sending —
/// enough for triage, without spending tokens on a full attachment-length body.
pub async fn classify_message(
    provider: Arc<dyn AiProvider>,
    sender: &str,
    subject: &str,
    body_text: &str,
) -> Result<MailClassification, AiError> {
    let truncated_body: String = body_text.chars().take(4000).collect();
    let user_content = format!("From: {sender}\nSubject: {subject}\n\n{truncated_body}");

    let req = GenerateRequest {
        system_prompt: build_mail_classify_prompt(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::Value::String(user_content),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: Default::default(),
        mode: QueryMode::SingleCommand,
        max_tokens: None,
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_for_spawn = provider.clone();
    let join = tokio::spawn(async move { provider_for_spawn.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done { break; }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(join_err) => return Err(AiError::Network { message: join_err.to_string() }),
    }

    let cleaned = extract_json_from_response(&buf);
    serde_json::from_str(&cleaned).map_err(|e| AiError::ModelError {
        reason: e.to_string(),
        raw: buf.chars().take(300).collect(),
    })
}
```

- [ ] **Step 4: Register the submodule**

In `src-tauri/src/mail/mod.rs`, add:

```rust
pub mod classify;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd src-tauri && cargo test --test mail_classify`
Expected: PASS — all 5 tests green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/mail/classify.rs src-tauri/src/mail/mod.rs src-tauri/tests/mail_classify.rs
git commit -m "feat(mail): AI classification (summary + importance + promotional) in one call"
```

---

## Task 7: IMAP fetch client (network I/O — manual verification required)

**Files:**
- Create: `src-tauri/src/mail/client.rs`
- Modify: `src-tauri/src/mail/mod.rs`

There is no existing IMAP-mocking infrastructure in this codebase (wiremock mocks HTTP, not the IMAP wire protocol), and standing up a disposable IMAP test server is out of scope for this task. This function is therefore **not covered by an automated test** — it gets verified manually in Task 13 against a real mailbox. Keep it small and push all testable logic (parsing) into Task 4's `parse.rs`, which already has full coverage.

- [ ] **Step 1: Create `src-tauri/src/mail/client.rs`**

```rust
// src-tauri/src/mail/client.rs
use tokio::net::TcpStream;
use futures::TryStreamExt;

#[derive(Debug, thiserror::Error)]
pub enum MailClientError {
    #[error("connection error: {0}")]
    Connect(String),
    #[error("login failed: {0}")]
    Login(String),
    #[error("IMAP command failed: {0}")]
    Command(String),
}

pub struct RawMessage {
    pub uid: i64,
    pub raw: Vec<u8>,
}

/// Fetch every message with UID greater than `since_uid` (or the whole
/// mailbox if `since_uid` is `None`, i.e. first poll) from INBOX, using
/// `BODY.PEEK[]` so the server's `\Seen` flag is never touched — AITerm
/// tracks read/unread locally (see `db/mail.rs::mark_read_locally`) so it
/// doesn't clobber the read state the user sees in their phone's mail app.
pub async fn fetch_new_messages(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    since_uid: Option<i64>,
) -> Result<Vec<RawMessage>, MailClientError> {
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
    let mut session = client
        .login(username, password)
        .await
        .map_err(|(e, _client)| MailClientError::Login(e.to_string()))?;

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

    let mut new_uids: Vec<u32> = uids
        .into_iter()
        .filter(|uid| since_uid.map_or(true, |since| (*uid as i64) > since))
        .collect();
    new_uids.sort_unstable();

    if new_uids.is_empty() {
        session.logout().await.ok();
        return Ok(Vec::new());
    }

    let uid_set = new_uids.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
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

    session.logout().await.ok();
    Ok(messages)
}
```

- [ ] **Step 2: Register the submodule**

In `src-tauri/src/mail/mod.rs`, add:

```rust
pub mod client;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles. If `async-imap`'s actual method signatures differ from what's written above (see the external-API-risk note at the top of this plan), fix the mismatches here now — common drift points are `uid_fetch`'s return type needing an explicit stream adapter, or `Fetch::uid`'s exact numeric type (`Uid` vs `u32`) needing an `as i64` or `.into()` instead of a bare cast.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/mail/client.rs src-tauri/src/mail/mod.rs
git commit -m "feat(mail): IMAP fetch client (BODY.PEEK, UID-based incremental fetch)"
```

---

## Task 8: `MailState` — per-account background task registry

**Files:**
- Create: `src-tauri/src/mail/manager.rs`
- Modify: `src-tauri/src/mail/mod.rs`

- [ ] **Step 1: Create `src-tauri/src/mail/manager.rs`**

```rust
// src-tauri/src/mail/manager.rs
use std::collections::HashMap;

/// Holds one background polling task per mail account. Managed as Tauri
/// state wrapped in `tokio::sync::Mutex` (see lib.rs) — mirrors
/// `telegram::TelegramState`'s single-task spawn/abort pattern, extended to
/// a HashMap since mail supports multiple simultaneous accounts.
pub struct MailState {
    pub tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl MailState {
    pub fn new() -> Self {
        Self { tasks: HashMap::new() }
    }
}

impl Default for MailState {
    fn default() -> Self { Self::new() }
}
```

This is a plain data holder — no logic to unit test here; its behavior is exercised through `poller.rs`'s `restart_account`/`stop_account` in Task 9, which are verified manually in Task 13 (they require a live Tauri `AppHandle`, same constraint noted for `ai_query` in `ai_query_command.rs:61-64`).

- [ ] **Step 2: Register the submodule**

In `src-tauri/src/mail/mod.rs`, add:

```rust
pub mod manager;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/mail/manager.rs src-tauri/src/mail/mod.rs
git commit -m "feat(mail): MailState task registry (one JoinHandle per account)"
```

---

## Task 9: Poller — ties fetch + parse + classify + persist + emit together

**Files:**
- Create: `src-tauri/src/mail/poller.rs`
- Modify: `src-tauri/src/mail/mod.rs`

This is the orchestration layer; every piece it calls (`parse_raw_message`, `classify_message`, the `db/mail.rs` functions) already has unit coverage from Tasks 3/4/6. The orchestration itself needs a live `AppHandle` + Tauri managed state to run, so — like `ai_query` and the Telegram poller before it — it's verified manually in Task 13, not by an automated test.

- [ ] **Step 1: Create `src-tauri/src/mail/poller.rs`**

```rust
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

        let classification = classify_message(provider.clone(), &parsed.sender, &parsed.subject, &parsed.body_text)
            .await
            .unwrap_or_else(|e| {
                log::warn!("mail classification failed for uid={}: {e}", raw.uid);
                Default::default()
            });

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
```

- [ ] **Step 2: Register the submodule**

In `src-tauri/src/mail/mod.rs`, add:

```rust
pub mod poller;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles. (It won't be reachable from any command yet — that's Task 10 — but `cargo check` still type-checks it as part of the `mail` module tree.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/mail/poller.rs src-tauri/src/mail/mod.rs
git commit -m "feat(mail): poller — fetch, parse, classify, persist, emit per account"
```

---

## Task 10: Tauri commands

**Files:**
- Create: `src-tauri/src/commands/mail.rs`
- Modify: `src-tauri/src/commands/mod.rs`

- [ ] **Step 1: Create `src-tauri/src/commands/mail.rs`**

```rust
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
    let _ = secret_store.delete(&mail_secret_key(&id));
    config_store.remove_mail_account(&id).map_err(|e| e.to_string())?;
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
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/commands/mod.rs`, add (keeping the file's existing ordering — it is not strictly alphabetical, so just add it near the other short domain modules, e.g. after `pub mod knowledge_base;` if present, otherwise at the end):

```rust
pub mod mail;
```

- [ ] **Step 3: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/mail.rs src-tauri/src/commands/mod.rs
git commit -m "feat(mail): add/remove/list account and list/mark-read message commands"
```

---

## Task 11: Wire everything into `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Import the new commands**

In the big `use commands::{ ... };` block (`lib.rs:22-87`), add a new entry (alphabetically near `knowledge_base`):

```rust
    mail::{
        mail_add_account, mail_remove_account, mail_list_accounts,
        mail_list_messages, mail_mark_read, mail_count_unread,
    },
```

- [ ] **Step 2: Import `MailDb` and `MailState`**

Near the existing `use db::{...}` line (`lib.rs:89`), change:

```rust
use db::{design::DesignDb, loop_sessions::LoopSessionDb, manager::DbManager, Db2SidecarState};
```

to:

```rust
use db::{design::DesignDb, loop_sessions::LoopSessionDb, mail::MailDb, manager::DbManager, Db2SidecarState};
```

Add near the other top-level `use` lines (e.g. after `use pty::PtyManager;`):

```rust
use mail::manager::MailState;
```

- [ ] **Step 3: Construct `MailDb` alongside the other DB pools**

In `run()`, after line 117 (`let kb_db = ...`):

```rust
    let mail_db = tauri::async_runtime::block_on(async { MailDb::new().await });
```

- [ ] **Step 4: Register managed state**

After `.manage(kb_db)` (`lib.rs:238`), add two lines:

```rust
        .manage(mail_db)
        .manage(tokio::sync::Mutex::new(MailState::new()))
```

- [ ] **Step 5: Start polling on launch**

In the `.setup(|app| { ... })` closure (`lib.rs:245-250`), add a call alongside `telegram::init`:

```rust
        .setup(|app| {
            telegram::init(app.handle());
            mail::poller::init(app.handle());
            enterprise::agent::init(app.handle());
            commands::appimage::repair_integration_on_startup();
            Ok(())
        })
```

- [ ] **Step 6: Register the commands with `invoke_handler!`**

In the `tauri::generate_handler![...]` list, add a new section after the `// Knowledge Base` block (after `kb_delete_chat_session,`):

```rust
            // Mail
            mail_add_account,
            mail_remove_account,
            mail_list_accounts,
            mail_list_messages,
            mail_mark_read,
            mail_count_unread,
```

- [ ] **Step 7: Verify the whole crate builds**

Run: `cd src-tauri && cargo check`
Expected: compiles with no errors.

- [ ] **Step 8: Run the full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS — every pre-existing test still passes, plus all mail tests from Tasks 2/3/4/5/6.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(mail): wire MailDb/MailState/commands/poller into the Tauri app"
```

---

## Task 12: Frontend — Mail tab registration (icon, TabType, NewTabPicker, TerminalApp)

**Files:**
- Modify: `src/components/Icons.tsx`
- Modify: `src/components/TabBar/index.tsx`
- Modify: `src/components/TabBar/index.css`
- Modify: `src/components/NewTabPicker/index.tsx`
- Modify: `src/lib/i18n.ts`
- Create: `src/components/MailView/MailView.tsx`
- Create: `src/components/MailView/index.ts`
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Add `MailIcon`**

In `src/components/Icons.tsx`, add after `LibraryIcon` (~line 483):

```tsx
// 30. Mail Icon
export function MailIcon({ size = 16, ...props }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.2"
      strokeLinecap="round"
      strokeLinejoin="round"
      {...props}
    >
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m3 7 9 6 9-6" />
    </svg>
  );
}
```

- [ ] **Step 2: Add `"mail"` to `TabType` and `getTabIcon`**

In `src/components/TabBar/index.tsx`:

Line 21, add `"mail"` to the union:
```tsx
export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base" | "mail";
```

Add `MailIcon` to the import from `"../Icons"` (line 5-18).

In `getTabIcon` (~line 64), add a case before `default`:
```tsx
    case "mail": return <MailIcon size={18} />;
```

- [ ] **Step 3: Add an unread-count badge prop**

Still in `src/components/TabBar/index.tsx`:

Add to `TabBarProps` (after `hasUpdate?: boolean;`, line 50):
```tsx
  mailUnreadCount?: number;
```

Add to the function's destructured params (after `hasUpdate = false`, line 79):
```tsx
  mailUnreadCount = 0,
```

At the tab icon render site (line 161), wrap with a badge:
```tsx
            <span className="aiterm-tab-icon" style={{ position: "relative" }}>
              {getTabIcon(tab.type)}
              {tab.type === "mail" && mailUnreadCount > 0 && (
                <span className="mail-unread-badge">{mailUnreadCount > 99 ? "99+" : mailUnreadCount}</span>
              )}
            </span>
```

- [ ] **Step 4: Add the badge CSS**

In `src/components/TabBar/index.css`, add after the `.update-badge--tile` rule (~line 204):

```css
.mail-unread-badge {
  position: absolute;
  top: -6px;
  right: -8px;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  background: #f97316;
  border-radius: 7px;
  border: 1.5px solid var(--bg-secondary, #151515);
  color: #fff;
  font-size: 9px;
  font-weight: 700;
  line-height: 11px;
  text-align: center;
  pointer-events: none;
}
```

- [ ] **Step 5: Add the NewTabPicker entry**

In `src/components/NewTabPicker/index.tsx`:

Add `MailIcon` to the import (line 4-14).

Add to the `items` array (~line 51):
```tsx
    { type: "mail",           icon: <MailIcon size={18} />,       label: t.mail_tab,           desc: t.new_mail_desc },
```

- [ ] **Step 6: Add i18n strings**

In `src/lib/i18n.ts`, add to the `zhTW` object (near the other `*_tab`/`new_*_desc` pairs, e.g. next to `knowledge_base_tab`):

```typescript
    mail_tab: "信箱",
    new_mail_desc: "AI 摘要信箱、通知重要信件",
    mail_no_accounts: "尚未設定信箱帳號",
    mail_add_account: "新增信箱帳號",
    mail_select_account: "選擇信箱帳號",
```

Add the matching keys to the `enRaw` object:

```typescript
    mail_tab: "Mail",
    new_mail_desc: "AI-summarized inbox with important-mail alerts",
    mail_no_accounts: "No mail accounts configured yet",
    mail_add_account: "Add mail account",
    mail_select_account: "Select a mail account",
```

- [ ] **Step 7: Create `MailView`**

Create `src/components/MailView/MailView.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useLocale } from "../../contexts/LocaleContext";
import {
  MAIL_SYNC_EVENT,
  mailListAccounts,
  mailListMessages,
  type MailAccount,
  type MailMessage,
  type MailSyncEvent,
} from "../../ipc/mail";
import "./MailView.css";

interface MailViewProps {
  isActive: boolean;
}

export function MailView({ isActive }: MailViewProps) {
  const { t } = useLocale();
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [selectedAccountId, setSelectedAccountId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MailMessage[]>([]);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    mailListAccounts().then((list) => {
      if (!mountedRef.current) return;
      setAccounts(list);
      setSelectedAccountId((prev) => prev ?? list[0]?.id ?? null);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    if (!selectedAccountId) {
      setMessages([]);
      return;
    }
    mailListMessages(selectedAccountId).then((list) => {
      if (mountedRef.current) setMessages(list);
    }).catch(() => {});
  }, [selectedAccountId]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let active = true;
    listen<MailSyncEvent>(MAIL_SYNC_EVENT, (event) => {
      if (!active) return;
      if (event.payload.account_id !== selectedAccountId) return;
      mailListMessages(event.payload.account_id).then((list) => {
        if (mountedRef.current) setMessages(list);
      }).catch(() => {});
    }).then((fn) => {
      if (!active) {
        Promise.resolve(fn()).catch(() => {});
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.error("[mail-sync-event] listener registration failed:", err);
    });
    return () => {
      active = false;
      if (unlisten) {
        try { Promise.resolve(unlisten()).catch(() => {}); } catch {}
      }
    };
  }, [selectedAccountId]);

  if (!isActive) return null;

  if (accounts.length === 0) {
    return <div className="mail-view mail-view--empty">{t.mail_no_accounts}</div>;
  }

  return (
    <div className="mail-view">
      <select
        className="mail-view__account-select"
        value={selectedAccountId ?? ""}
        onChange={(e) => setSelectedAccountId(e.target.value)}
        aria-label={t.mail_select_account}
      >
        {accounts.map((a) => (
          <option key={a.id} value={a.id}>{a.email}</option>
        ))}
      </select>
      <ul className="mail-view__list">
        {messages.map((m) => (
          <li key={m.id} className={`mail-view__item ${m.is_read_locally ? "" : "mail-view__item--unread"}`}>
            <div className="mail-view__item-sender">{m.sender}</div>
            <div className="mail-view__item-subject">{m.subject}</div>
            <div className="mail-view__item-summary">{m.ai_summary}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
```

Create `src/components/MailView/MailView.css`:

```css
.mail-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  padding: 12px;
  overflow: hidden;
}

.mail-view--empty {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary, #888);
}

.mail-view__account-select {
  margin-bottom: 8px;
  padding: 6px 8px;
}

.mail-view__list {
  list-style: none;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  flex: 1;
}

.mail-view__item {
  padding: 10px 8px;
  border-bottom: 1px solid var(--border-color, #2a2a2a);
}

.mail-view__item--unread {
  font-weight: 600;
}

.mail-view__item-sender {
  font-size: 12px;
  color: var(--text-secondary, #888);
}

.mail-view__item-subject {
  font-size: 14px;
}

.mail-view__item-summary {
  font-size: 12px;
  color: var(--text-secondary, #888);
  margin-top: 2px;
}
```

Create `src/components/MailView/index.ts`:

```typescript
export { MailView } from "./MailView";
```

- [ ] **Step 8: Wire the tab into `TerminalApp.tsx`**

In `src/components/TerminalApp.tsx`:

Add the import (near `import { KnowledgeBaseView } from "./KnowledgeBaseView";`, line 14):
```tsx
import { MailView } from "./MailView";
```

Widen `handlePickerSelect`'s type union (line 154) to include `"mail"`:
```tsx
  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base" | "mail") => {
```

Add a title branch (after `if (type === "knowledge-base") title = t.knowledge_base_tab;`, line 165):
```tsx
    if (type === "mail") title = t.mail_tab;
```

Add `t.mail_tab` to the `useCallback` dependency array (line 169), appending `, t.mail_tab`.

Add the render branch (after the `knowledge-base` branch, ~line 351):
```tsx
              ) : tab.type === "mail" ? (
                <MailView isActive={isActive} />
```

- [ ] **Step 9: Type-check**

Run: `npx tsc -b`
Expected: no errors.

- [ ] **Step 10: Commit**

```bash
git add src/components/Icons.tsx src/components/TabBar/index.tsx src/components/TabBar/index.css \
        src/components/NewTabPicker/index.tsx src/lib/i18n.ts \
        src/components/MailView/ src/components/TerminalApp.tsx
git commit -m "feat(mail): add Mail tab (list view, unread badge, i18n)"
```

---

## Task 13: Frontend — `ipc/mail.ts` typed wrapper

**Files:**
- Create: `src/ipc/mail.ts`

- [ ] **Step 1: Create `src/ipc/mail.ts`**

```typescript
import { invoke } from "@tauri-apps/api/core";

export interface MailAccount {
  id: string;
  email: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  poll_interval_secs: number;
}

export interface MailAccountInput {
  email: string;
  imap_host: string;
  imap_port: number;
  smtp_host: string;
  smtp_port: number;
  username: string;
  password: string;
  poll_interval_secs?: number;
}

export interface MailMessage {
  id: string;
  account_id: string;
  uid: number;
  sender: string;
  subject: string;
  date: string | null;
  body_text: string;
  ai_summary: string | null;
  is_important: boolean;
  is_promotional: boolean;
  is_read_locally: boolean;
  fetched_at: string;
}

export type MailSyncEvent =
  | { kind: "summary"; account_id: string; message_id: string }
  | { kind: "important"; account_id: string; message_id: string; subject: string; summary: string };

export const MAIL_SYNC_EVENT = "mail-sync-event";

export function mailAddAccount(input: MailAccountInput): Promise<MailAccount> {
  return invoke<MailAccount>("mail_add_account", { input });
}

export function mailRemoveAccount(id: string): Promise<void> {
  return invoke<void>("mail_remove_account", { id });
}

export function mailListAccounts(): Promise<MailAccount[]> {
  return invoke<MailAccount[]>("mail_list_accounts");
}

export function mailListMessages(accountId: string): Promise<MailMessage[]> {
  return invoke<MailMessage[]>("mail_list_messages", { accountId });
}

export function mailMarkRead(messageId: string): Promise<void> {
  return invoke<void>("mail_mark_read", { messageId });
}

export function mailCountUnread(): Promise<number> {
  return invoke<number>("mail_count_unread");
}
```

- [ ] **Step 2: Type-check**

Run: `npx tsc -b`
Expected: no errors (this file has no test of its own — it's exercised by the Vitest tests in Tasks 14/15, which mock `invoke`).

- [ ] **Step 3: Commit**

```bash
git add src/ipc/mail.ts
git commit -m "feat(mail): typed IPC wrapper for mail commands"
```

---

## Task 14: Frontend — `useMailSync` hook (global unread count + OS notification)

**Files:**
- Create: `src/hooks/useMailSync.ts`
- Create: `src/hooks/useMailSync.test.ts`
- Modify: `src/components/TerminalApp.tsx`

This hook is mounted once in `TerminalApp.tsx` (the always-mounted shell), not inside `MailView` — important mail should notify the user even if they've never opened the Mail tab this session.

- [ ] **Step 1: Write the failing test**

Create `src/hooks/useMailSync.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { useMailSync } from "./useMailSync";

const listeners: Record<string, (event: { payload: unknown }) => void> = {};

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((eventName: string, cb: (event: { payload: unknown }) => void) => {
    listeners[eventName] = cb;
    return Promise.resolve(() => { delete listeners[eventName]; });
  }),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  requestPermission: vi.fn().mockResolvedValue("granted"),
  sendNotification: vi.fn(),
}));

vi.mock("../ipc/mail", () => ({
  MAIL_SYNC_EVENT: "mail-sync-event",
  mailCountUnread: vi.fn().mockResolvedValue(2),
}));

describe("useMailSync", () => {
  beforeEach(() => {
    for (const key of Object.keys(listeners)) delete listeners[key];
    vi.clearAllMocks();
  });

  it("loads the initial unread count on mount", async () => {
    const { result } = renderHook(() => useMailSync());
    await waitFor(() => expect(result.current.unreadCount).toBe(2));
  });

  it("sends an OS notification when an important mail-sync-event arrives", async () => {
    const { sendNotification } = await import("@tauri-apps/plugin-notification");
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "important", account_id: "a1", message_id: "m1", subject: "老闆找你", summary: "問今天能否開會" },
    });

    await waitFor(() => expect(sendNotification).toHaveBeenCalledWith({ title: "老闆找你", body: "問今天能否開會" }));
  });

  it("does not send a notification for a plain summary event", async () => {
    const { sendNotification } = await import("@tauri-apps/plugin-notification");
    renderHook(() => useMailSync());
    await waitFor(() => expect(listeners["mail-sync-event"]).toBeDefined());

    listeners["mail-sync-event"]({
      payload: { kind: "summary", account_id: "a1", message_id: "m1" },
    });

    await new Promise((r) => setTimeout(r, 0));
    expect(sendNotification).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npm run test -- useMailSync`
Expected: FAIL — `src/hooks/useMailSync.ts` doesn't exist yet.

- [ ] **Step 3: Create `src/hooks/useMailSync.ts`**

```typescript
import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { MAIL_SYNC_EVENT, mailCountUnread, type MailSyncEvent } from "../ipc/mail";

export function useMailSync() {
  const [unreadCount, setUnreadCount] = useState(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refreshUnread = () => {
    mailCountUnread().then((count) => {
      if (mountedRef.current) setUnreadCount(count);
    }).catch(() => {});
  };

  useEffect(() => {
    refreshUnread();

    let unlisten: UnlistenFn | null = null;
    let active = true;

    listen<MailSyncEvent>(MAIL_SYNC_EVENT, async (event) => {
      if (!active) return;
      refreshUnread();

      if (event.payload.kind === "important") {
        let granted = await isPermissionGranted();
        if (!granted) {
          const permission = await requestPermission();
          granted = permission === "granted";
        }
        if (granted) {
          sendNotification({ title: event.payload.subject, body: event.payload.summary });
        }
      }
    }).then((fn) => {
      if (!active) {
        Promise.resolve(fn()).catch(() => {});
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.error("[mail-sync-event] listener registration failed:", err);
    });

    return () => {
      active = false;
      if (unlisten) {
        try { Promise.resolve(unlisten()).catch(() => {}); } catch {}
      }
    };
  }, []);

  return { unreadCount };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npm run test -- useMailSync`
Expected: PASS — all 3 tests green.

- [ ] **Step 5: Mount the hook in `TerminalApp.tsx` and pass the count to `TabBar`**

In `src/components/TerminalApp.tsx`, add the import:

```tsx
import { useMailSync } from "../hooks/useMailSync";
```

Inside the `TerminalApp` component body (near other top-level hooks), add:

```tsx
  const { unreadCount: mailUnreadCount } = useMailSync();
```

Find the `<TabBar ... />` usage and add the prop:

```tsx
  <TabBar
    ...
    mailUnreadCount={mailUnreadCount}
  />
```

- [ ] **Step 6: Type-check and run the full frontend suite**

Run: `npx tsc -b && npm run test`
Expected: no type errors; all tests (including the new 3) pass.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useMailSync.ts src/hooks/useMailSync.test.ts src/components/TerminalApp.tsx
git commit -m "feat(mail): useMailSync hook — global unread count + OS notification for important mail"
```

---

## Task 15: Frontend — Settings page for account CRUD

**Files:**
- Create: `src/components/Settings/MailAccountsPage.tsx`
- Create: `src/components/Settings/MailAccountsPage.test.tsx`
- Modify: `src/components/Settings/SettingsView.tsx`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: Add the remaining i18n strings**

In `src/lib/i18n.ts`, add to `zhTW` (near the Task 12 mail strings):

```typescript
    mail_accounts_settings_title: "信箱帳號",
    mail_email: "電子郵件地址",
    mail_imap_host: "IMAP 伺服器",
    mail_imap_port: "IMAP 連接埠",
    mail_smtp_host: "SMTP 伺服器",
    mail_smtp_port: "SMTP 連接埠",
    mail_username: "登入帳號",
    mail_password: "密碼 / App Password",
    mail_poll_interval: "輪詢間隔（秒）",
    mail_add: "新增帳號",
    mail_remove: "移除",
```

Add matching keys to `enRaw`:

```typescript
    mail_accounts_settings_title: "Mail Accounts",
    mail_email: "Email address",
    mail_imap_host: "IMAP server",
    mail_imap_port: "IMAP port",
    mail_smtp_host: "SMTP server",
    mail_smtp_port: "SMTP port",
    mail_username: "Username",
    mail_password: "Password / App Password",
    mail_poll_interval: "Poll interval (seconds)",
    mail_add: "Add account",
    mail_remove: "Remove",
```

- [ ] **Step 2: Write the failing test**

Create `src/components/Settings/MailAccountsPage.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MailAccountsPage } from "./MailAccountsPage";
import { LocaleProvider } from "../../contexts/LocaleContext";

const mockAccounts = [
  { id: "a1", email: "me@example.com", imap_host: "imap.example.com", imap_port: 993, smtp_host: "smtp.example.com", smtp_port: 587, username: "me@example.com", poll_interval_secs: 300 },
];

vi.mock("../../ipc/mail", () => ({
  mailListAccounts: vi.fn(() => Promise.resolve(mockAccounts)),
  mailAddAccount: vi.fn(() => Promise.resolve(mockAccounts[0])),
  mailRemoveAccount: vi.fn(() => Promise.resolve()),
}));

function renderPage() {
  return render(
    <LocaleProvider>
      <MailAccountsPage />
    </LocaleProvider>
  );
}

describe("MailAccountsPage", () => {
  beforeEach(() => vi.clearAllMocks());

  it("lists existing accounts on mount", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByText("me@example.com")).toBeInTheDocument());
  });

  it("calls mailRemoveAccount and refetches when Remove is clicked", async () => {
    const { mailRemoveAccount, mailListAccounts } = await import("../../ipc/mail");
    renderPage();
    await waitFor(() => expect(screen.getByText("me@example.com")).toBeInTheDocument());

    fireEvent.click(screen.getByRole("button", { name: /remove|移除/i }));

    await waitFor(() => expect(mailRemoveAccount).toHaveBeenCalledWith("a1"));
    expect(mailListAccounts).toHaveBeenCalledTimes(2); // initial mount + post-remove refetch
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `npm run test -- MailAccountsPage`
Expected: FAIL — `MailAccountsPage` doesn't exist yet.

- [ ] **Step 4: Create `src/components/Settings/MailAccountsPage.tsx`**

```tsx
import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { mailAddAccount, mailListAccounts, mailRemoveAccount, type MailAccount } from "../../ipc/mail";

const emptyForm = {
  email: "",
  imap_host: "",
  imap_port: 993,
  smtp_host: "",
  smtp_port: 587,
  username: "",
  password: "",
  poll_interval_secs: 300,
};

export function MailAccountsPage() {
  const { t } = useLocale();
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [form, setForm] = useState(emptyForm);

  const refresh = () => {
    mailListAccounts().then(setAccounts).catch(() => {});
  };

  useEffect(() => { refresh(); }, []);

  const handleAdd = async (e: React.FormEvent) => {
    e.preventDefault();
    await mailAddAccount(form);
    setForm(emptyForm);
    refresh();
  };

  const handleRemove = async (id: string) => {
    await mailRemoveAccount(id);
    refresh();
  };

  return (
    <div className="mail-accounts-page">
      <h2>{t.mail_accounts_settings_title}</h2>

      <ul>
        {accounts.map((a) => (
          <li key={a.id}>
            {a.email}
            <button onClick={() => handleRemove(a.id)}>{t.mail_remove}</button>
          </li>
        ))}
      </ul>

      <form onSubmit={handleAdd}>
        <label>
          {t.mail_email}
          <input value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} required />
        </label>
        <label>
          {t.mail_imap_host}
          <input value={form.imap_host} onChange={(e) => setForm({ ...form, imap_host: e.target.value })} required />
        </label>
        <label>
          {t.mail_imap_port}
          <input type="number" value={form.imap_port} onChange={(e) => setForm({ ...form, imap_port: Number(e.target.value) })} required />
        </label>
        <label>
          {t.mail_smtp_host}
          <input value={form.smtp_host} onChange={(e) => setForm({ ...form, smtp_host: e.target.value })} required />
        </label>
        <label>
          {t.mail_smtp_port}
          <input type="number" value={form.smtp_port} onChange={(e) => setForm({ ...form, smtp_port: Number(e.target.value) })} required />
        </label>
        <label>
          {t.mail_username}
          <input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })} required />
        </label>
        <label>
          {t.mail_password}
          <input type="password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })} required />
        </label>
        <label>
          {t.mail_poll_interval}
          <input type="number" value={form.poll_interval_secs} onChange={(e) => setForm({ ...form, poll_interval_secs: Number(e.target.value) })} />
        </label>
        <button type="submit">{t.mail_add}</button>
      </form>
    </div>
  );
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `npm run test -- MailAccountsPage`
Expected: PASS — both tests green.

- [ ] **Step 6: Wire the page into `SettingsView.tsx`**

In `src/components/Settings/SettingsView.tsx`:

Add the import:
```tsx
import { MailAccountsPage } from "./MailAccountsPage";
```

Add `MailIcon` (or reuse an existing icon) to the `Icons` import — mirror the `DatabaseIcon` import.

Widen `SettingsTab` (line 21):
```tsx
type SettingsTab = "general" | "providers" | "databases" | "vcs" | "mail" | "enterprise" | "about" | "mcp";
```

Add a sidebar button (after the `"vcs"` button, ~line 59):
```tsx
        <button
          className={`sidebar-item ${tab === "mail" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("mail")}
        >
          <MailIcon size={16} /> {t.mail_accounts_settings_title}
        </button>
```

Add the content branch (~line 90):
```tsx
        {tab === "mail" && <MailAccountsPage />}
```

- [ ] **Step 7: Type-check and run the full frontend suite**

Run: `npx tsc -b && npm run test`
Expected: no type errors; all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/components/Settings/MailAccountsPage.tsx src/components/Settings/MailAccountsPage.test.tsx \
        src/components/Settings/SettingsView.tsx src/lib/i18n.ts
git commit -m "feat(mail): Settings page for mail account CRUD"
```

---

## Task 16: Manual end-to-end verification

This phase adds a genuinely new I/O boundary (live IMAP servers, OS keychain, OS notifications) that automated tests can't fully cover. Do this before considering Phase 1 done.

- [ ] **Step 1: Full build**

Run: `npm run build && cd src-tauri && cargo test && cargo check`
Expected: all green.

- [ ] **Step 2: Run the dev app**

Run: `npm run tauri:dev`

- [ ] **Step 3: Add a real test mailbox**

Open Settings → Mail, add an account using a disposable test mailbox (e.g. a Gmail account with an [app password](https://support.google.com/accounts/answer/185833) — Gmail requires an app password for raw IMAP login, not the account password). Use `imap.gmail.com:993` / `smtp.gmail.com:587`.

Expected: the form submits without error, the new account appears in the list.

- [ ] **Step 4: Verify polling picks up new mail**

Open a new Mail tab. Send yourself a test email from another account. Wait up to `poll_interval_secs` (default 300s — temporarily set it lower, e.g. 30, via the form for faster testing).

Expected: the message appears in the Mail tab's list with an AI-generated summary; the tab icon's unread badge increments.

- [ ] **Step 5: Verify importance triggers a notification**

Send yourself an email with clearly urgent content (e.g. subject "URGENT: need your approval today").

Expected: an OS notification pops up with that email's subject and AI summary (first run: the OS may prompt for notification permission — grant it).

- [ ] **Step 6: Verify a promotional email does NOT notify**

Forward yourself a marketing/newsletter email, or send one with obvious promotional content ("50% off — Sale ends today!").

Expected: no OS notification; the message still appears in the list with the unread badge incremented (confirms `is_promotional` classification runs correctly even though Phase 1 has no UI that acts on it yet — that's Phase 6).

- [ ] **Step 7: Verify removing an account stops polling and clears data**

Remove the test account from Settings → Mail.

Expected: the account disappears from Settings and from the Mail tab's account selector; no further notifications arrive for it even after waiting past the poll interval.

- [ ] **Step 8: Verify `\Seen` is not touched server-side**

Check the test mailbox from a real mail client (webmail or phone) for the messages AITerm fetched.

Expected: they still show as unread there — confirms `BODY.PEEK[]` in Task 7 is working as intended (AITerm's read-tracking is local-only, per the spec's explicit design decision).

- [ ] **Step 9: Report results to the user**

Summarize what worked and what didn't (if anything required a fix to `async-imap` wiring per Task 7's risk note, mention what changed).

// src-tauri/src/mail/manager.rs
use std::collections::HashMap;

/// One account's background task, plus the switch that asks it to stop.
///
/// The switch is what an abort-only handle can't do. The task holds a
/// *persistent* IMAP connection that spends nearly all of its life parked in
/// IDLE; aborting it there drops the TLS socket mid-command with no LOGOUT,
/// which leaves a session on the server and burns one of the concurrent-IMAP
/// slots providers cap (Gmail allows about 15). Flipping this instead lets the
/// task end its IDLE properly, send DONE, and log out.
pub struct MailTask {
    pub handle: tokio::task::JoinHandle<()>,
    pub shutdown: tokio::sync::watch::Sender<bool>,
    /// Where `mail_delete_message` hands a delete to the account's task.
    ///
    /// The task owns the account's only authenticated session and spends
    /// nearly all its life parked in IDLE, so a delete has to reach *it* rather
    /// than opening a second connection: a login per delete burns one of the
    /// concurrent-IMAP slots providers cap (Gmail at about 15) and, because the
    /// two connections would then be reading the same mailbox independently,
    /// lets a delete interleave with a running sync. Going through the task
    /// serializes them by construction.
    pub delete: tokio::sync::mpsc::Sender<DeleteRequest>,
}

/// One user-initiated "move this message to Trash".
///
/// Carries a `oneshot` back rather than being fire-and-forget: this is the
/// feature's only write to the server, and the user has to be told whether it
/// actually happened. The reply is `Result<(), String>` because the error is
/// headed for the UI, not for another Rust caller.
pub struct DeleteRequest {
    pub uid: i64,
    pub reply: tokio::sync::oneshot::Sender<Result<(), String>>,
}

/// Holds one background sync task per mail account. Managed as Tauri
/// state wrapped in `tokio::sync::Mutex` (see lib.rs) — mirrors
/// `telegram::TelegramState`'s single-task spawn/abort pattern, extended to
/// a HashMap since mail supports multiple simultaneous accounts.
pub struct MailState {
    pub tasks: HashMap<String, MailTask>,
}

impl MailState {
    pub fn new() -> Self {
        Self { tasks: HashMap::new() }
    }
}

impl Default for MailState {
    fn default() -> Self { Self::new() }
}

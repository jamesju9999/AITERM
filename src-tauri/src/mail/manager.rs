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

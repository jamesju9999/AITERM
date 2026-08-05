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

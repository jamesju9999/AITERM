use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use parking_lot::Mutex;
use portable_pty::PtySize;
use tauri::{AppHandle, Emitter};

use super::error::{PtyError, PtyResult};
use super::events::{data_event_name, PtyDataPayload};
use super::session::PtySession;
use super::shell::{default_shell, ShellSpec};

#[derive(Default)]
pub struct PtyManager {
    sessions: Mutex<HashMap<String, Arc<PtySession>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// High-level: spawn a session and wire its output to a Tauri event.
    ///
    /// `bridge_env` 非 None 時，把 Claude Code 橋接的環境變數注入這個分頁。
    /// 環境變數只能在 spawn 的瞬間決定，所以事後無法對已開的分頁切換。
    pub fn create_with_app(
        &self,
        app: AppHandle,
        size: PtySize,
        cwd: Option<PathBuf>,
        bridge_env: Option<(u16, String)>,
    ) -> PtyResult<String> {
        let mut shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;

        if let Some((port, token)) = bridge_env {
            shell.envs.extend(crate::bridge::env::bridge_envs(port, &token));
            shell
                .env_removals
                .extend(crate::bridge::env::ENV_TO_REMOVE.iter().map(|s| s.to_string()));
        }

        // Pre-generate id so the closure can reference it.
        let id = uuid::Uuid::new_v4().to_string();
        let event_name = data_event_name(&id);
        let app_for_thread = app.clone();

        let session = PtySession::spawn_with_id(shell, size, id.clone(), cwd, move |chunk| {
            let payload = PtyDataPayload {
                base64: BASE64.encode(&chunk),
            };
            if let Err(e) = app_for_thread.emit(&event_name, payload) {
                eprintln!("emit {event_name} failed: {e}");
            }
        })?;

        self.sessions.lock().insert(id.clone(), Arc::new(session));
        Ok(id)
    }

    /// Low-level: spawn a session with a raw data callback. Used by tests.
    pub fn create_with_callback<F>(&self, size: PtySize, on_data: F) -> PtyResult<String>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;
        let session = PtySession::spawn(shell, size, None, on_data)?;
        let id = session.id.clone();
        self.sessions.lock().insert(id.clone(), Arc::new(session));
        Ok(id)
    }

    pub fn write(&self, id: &str, data: &[u8]) -> PtyResult<()> {
        self.get(id)?.write(data)
    }

    pub fn resize(&self, id: &str, size: PtySize) -> PtyResult<()> {
        self.get(id)?.resize(size)
    }

    pub fn close(&self, id: &str) -> PtyResult<()> {
        let session = self
            .sessions
            .lock()
            .remove(id)
            .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))?;
        session.kill()?;
        // Dropping the last Arc below will join the reader thread.
        drop(session);
        Ok(())
    }

    pub fn get_cwd(&self, id: &str) -> Option<PathBuf> {
        self.sessions.lock().get(id).map(|s| s.get_cwd())
    }

    pub fn get_shell_variant(&self, id: &str) -> Option<super::cd_parser::ShellVariant> {
        self.sessions.lock().get(id).map(|s| s.shell_variant())
    }

    /// Return recent terminal output (ANSI-stripped) for the given session.
    pub fn get_recent_output(&self, id: &str, max_bytes: usize) -> Option<String> {
        self.sessions.lock().get(id).and_then(|s| s.get_recent_output(max_bytes))
    }

    /// Bell-byte count for the given session, or `None` if the session
    /// doesn't exist. See `PtySession::bell_count` for what this counts.
    pub fn bell_count(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.bell_count())
    }

    fn get(&self, id: &str) -> PtyResult<Arc<PtySession>> {
        self.sessions
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| PtyError::SessionNotFound(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn manager_creates_and_closes_session() {
        let manager = PtyManager::new();
        let (tx, rx) = mpsc::channel::<Vec<u8>>();

        let id = manager
            .create_with_callback(
                PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("create session");

        // Read some initial output (shell banner / prompt) to confirm it's alive,
        // but don't fail the test if the shell is quiet — just proceed.
        let _ = rx.recv_timeout(Duration::from_secs(2));

        manager.close(&id).expect("close session");

        // After close, a subsequent write must fail with SessionNotFound.
        let err = manager.write(&id, b"noop").unwrap_err();
        assert!(matches!(err, PtyError::SessionNotFound(_)));
    }

    #[test]
    fn manager_get_cwd_returns_none_for_missing() {
        let manager = PtyManager::new();
        assert!(manager.get_cwd("no-such-id").is_none());
    }

    #[test]
    fn manager_write_missing_session_errors() {
        let manager = PtyManager::new();
        let err = manager.write("no-such-id", b"x").unwrap_err();
        assert!(matches!(err, PtyError::SessionNotFound(_)));
    }
}

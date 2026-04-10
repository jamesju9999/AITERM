use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::PtySize;

use super::error::{PtyError, PtyResult};
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

    /// Low-level: spawn a session with a raw data callback. Used by tests.
    pub fn create_with_callback<F>(&self, size: PtySize, on_data: F) -> PtyResult<String>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;
        let session = PtySession::spawn(shell, size, on_data)?;
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
    fn manager_write_missing_session_errors() {
        let manager = PtyManager::new();
        let err = manager.write("no-such-id", b"x").unwrap_err();
        assert!(matches!(err, PtyError::SessionNotFound(_)));
    }
}

use std::collections::HashMap;
use std::path::PathBuf;
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

    /// 用指定的 id 與環境變數調整 spawn 一個 session，輸出丟給 `on_data`。
    ///
    /// **id 由呼叫端給**：GUI 端要先有 id 才能組出 `pty://data/{id}` 的事件
    /// 名稱，再把發事件的 closure 傳進來。
    ///
    /// `envs` / `env_removals` 讓呼叫端注入自己的環境變數（GUI 用它接
    /// Claude Code 橋接）——core 不知道也不該知道那是什麼。
    pub fn create_with_callback_and_id<F>(
        &self,
        size: PtySize,
        id: String,
        cwd: Option<PathBuf>,
        envs: Vec<(String, String)>,
        env_removals: Vec<String>,
        on_data: F,
    ) -> PtyResult<String>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let mut shell: ShellSpec = default_shell().ok_or(PtyError::NoShellAvailable)?;
        shell.envs.extend(envs);
        shell.env_removals.extend(env_removals);
        let session = PtySession::spawn_with_id(shell, size, id.clone(), cwd, on_data)?;
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

    /// Raw (not ANSI-stripped) recent output for the given session. See
    /// `PtySession::get_recent_raw`.
    pub fn get_recent_raw(&self, id: &str, max_bytes: usize) -> Option<Vec<u8>> {
        self.sessions.lock().get(id).and_then(|s| s.get_recent_raw(max_bytes))
    }

    /// Subscribe to a session's raw output, or `None` if it doesn't exist.
    /// See `PtySession::subscribe`.
    pub fn subscribe(&self, id: &str) -> Option<tokio::sync::broadcast::Receiver<Vec<u8>>> {
        self.sessions.lock().get(id).map(|s| s.subscribe())
    }

    /// Atomic snapshot-plus-subscribe for a session. See
    /// `PtySession::subscribe_with_history` for why sharing must use this
    /// instead of `get_recent_raw` followed by `subscribe`.
    pub fn subscribe_with_history(
        &self,
        id: &str,
        max_bytes: usize,
    ) -> Option<(Option<Vec<u8>>, tokio::sync::broadcast::Receiver<Vec<u8>>)> {
        self.sessions.lock().get(id).map(|s| s.subscribe_with_history(max_bytes))
    }

    /// Current terminal size (cols, rows) for the given session.
    pub fn size(&self, id: &str) -> Option<(u16, u16)> {
        self.sessions.lock().get(id).map(|s| s.size())
    }

    /// Bell-byte count for the given session, or `None` if the session
    /// doesn't exist. See `PtySession::bell_count` for what this counts.
    pub fn bell_count(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.bell_count())
    }

    /// Marker-byte count for the given session, or `None` if the session
    /// doesn't exist. See `PtySession::marker_count` for what this counts.
    pub fn marker_count(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.marker_count())
    }

    /// Last OSC 133 exit code seen for the given session, or `None` if the
    /// session doesn't exist or no marker has been seen yet. See
    /// `PtySession::last_exit_code`.
    pub fn last_exit_code(&self, id: &str) -> Option<i32> {
        self.sessions.lock().get(id).and_then(|s| s.last_exit_code())
    }

    /// Milliseconds since the given session last produced output, or `None`
    /// if the session doesn't exist. See `PtySession::ms_since_output`.
    pub fn ms_since_output(&self, id: &str) -> Option<u64> {
        self.sessions.lock().get(id).map(|s| s.ms_since_output())
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

    #[test]
    fn manager_marker_count_returns_none_for_missing() {
        let manager = PtyManager::new();
        assert!(manager.marker_count("no-such-id").is_none());
    }

    #[test]
    fn manager_marker_count_returns_zero_for_a_fresh_session() {
        let manager = PtyManager::new();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let id = manager
            .create_with_callback(
                PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("create session");
        assert_eq!(manager.marker_count(&id), Some(0));
    }

    #[test]
    fn create_with_callback_and_id_uses_the_id_it_was_given() {
        // GUI 端要先有 id 才能組出 `pty://data/{id}` 的事件名稱，再把發事件的
        // closure 傳進來。若這支自己另外產一個 id，事件會發到一個沒有人在聽的
        // 名字上，分頁永遠停在「initializing…」——而且不會有任何錯誤。
        let manager = PtyManager::new();
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let id = manager
            .create_with_callback_and_id(
                PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
                "chosen-id".to_string(),
                None,
                Vec::new(),
                Vec::new(),
                move |chunk| {
                    let _ = tx.send(chunk);
                },
            )
            .expect("create session");
        assert_eq!(id, "chosen-id");
        assert_eq!(manager.marker_count("chosen-id"), Some(0));
    }
}

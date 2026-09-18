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

/// True when `elevated` already holds a channel in the `Connected` state.
///
/// Used by `PtyManager::elevate` to avoid re-launching a second elevation
/// flow (a second UAC prompt + a second sidecar process + named pipe) for a
/// session that already has one connected — see `elevate`'s doc comment,
/// which promises exactly this. Also used by `is_elevated`, which needs the
/// identical check.
///
/// Deliberately a free function taking `&Mutex<Option<ElevatedChannel>>`
/// rather than inlined into `elevate`: `elevate` itself is `#[cfg(windows)]`
/// (it calls the real `spawn_windows`), but this check has nothing
/// Windows-specific about it — `ElevatedChannel` and `ElevatedState` are
/// ordinary cross-platform types (see `elevated.rs`, whose own tests already
/// build channels from a mock transport). Splitting it out means the
/// single-flight guard itself is unit-testable on every platform, including
/// macOS/Linux CI, without needing a real elevated session.
fn already_connected(elevated: &Mutex<Option<super::elevated::ElevatedChannel>>) -> bool {
    elevated
        .lock()
        .as_ref()
        .map(|c| c.state() == super::elevated::ElevatedState::Connected)
        .unwrap_or(false)
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

    /// 對指定 session 啟動提權流程（Windows-only；其他平台回
    /// `PtyError::Internal("elevation not supported on this platform")`）。
    /// `on_output` 收到提權 shell 的原始位元組，呼叫端負責接回 output ring
    /// buffer + Tauri 事件。
    #[cfg(windows)]
    pub fn elevate<F, D>(
        &self,
        id: &str,
        shell_variant: super::cd_parser::ShellVariant,
        mut on_output: F,
        on_disconnect: D,
    ) -> PtyResult<bool>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
        D: FnMut() + Send + 'static,
    {
        let session = self.get(id)?;
        // 已經有一個連線中的提權 channel：不再發第二次 UAC、不再開第二個
        // sidecar，直接沿用——這正是上面 doc comment 承諾的行為。沒有這個
        // 檢查的話，同一個分頁在第一次 `spawn_windows` 還卡在等待使用者回應
        // UAC 對話框的那幾分鐘內，如果又有一條指令被判定為權限不足而再次觸發
        // `pty_elevate`，就會併發打開兩個 UAC 對話框、兩個 sidecar 行程與具
        // 名管線，而兩者之中先完成的那個 `ElevatedChannel` 會被後完成的直接
        // 覆蓋掉、成為孤兒（洩漏執行緒/管線 handle/提權行程，且之後可能對
        // 「還活著」的第二個 channel 誤發一次 `elevated: false` 的斷線事件）。
        // 只處理「已經連線成功」這個案例；「正在等 UAC 回應、還沒連上」那個
        // 更難的視窗留給之後——見 `elevate` 的 caller 端。
        if already_connected(&session.elevated) {
            return Ok(true);
        }
        // 把呼叫端的 `on_output` 包一層：先餵進這個 session 自己的 ring
        // buffer / broadcast channel（`PtySession::ingest_external_output`），
        // 再呼叫呼叫端原本的 closure。沒有這一層，提權輸出只會抵達呼叫端
        // （GUI 層 `elevate_with_app` 目前只拿去重新 emit `pty://data/{id}`
        // 給前端畫面），永遠不會進 `output_ring`——而 `ai_query`/`ai_chat`
        // 靠的 `context::snapshot()` 只讀 `output_ring`，於是 AI 永遠看不到
        // 提權指令的輸出，違背這個功能存在的目的。見
        // `docs/superpowers/specs/2026-09-17-windows-elevated-pty-session-design.md`。
        //
        // 這裡包、而不是留給呼叫端自己包：`PtySession::ingest_external_output`
        // 是 `pub(crate)`，`elevate` 本來就已經透過 `self.get(id)?` 拿到
        // `Arc<PtySession>`，是唯一同時看得到「session 本體」與「呼叫端傳
        // 進來的 on_output」兩者的地方。
        let session_for_output = Arc::clone(&session);
        // 記前 10 筆而不是只記第一筆。只記第一筆是個實際踩過的坑：兩端都只有
        // 「第一筆」的紀錄時，「16 位元組之後就真的沒東西了」跟「有東西但沒被
        // 記下來」在 log 上完全無法區分，害好幾輪推論建立在一個沒被證實的前提
        // 上。連內容一起記，才能認出那到底是 ConPTY 的初始序列還是 shell 的輸出。
        let mut frames: u32 = 0;
        let wrapped_on_output = move |chunk: Vec<u8>| {
            frames += 1;
            if frames <= 10 {
                let shown = chunk.len().min(200);
                super::elevated::elevated_log_step(&format!(
                    "host received elevated frame #{frames}, {} bytes: {:?}",
                    chunk.len(),
                    String::from_utf8_lossy(&chunk[..shown])
                ));
            }
            session_for_output.ingest_external_output(&chunk);
            on_output(chunk);
        };
        match super::elevated::spawn_windows(id, shell_variant, session.size(), wrapped_on_output, on_disconnect)
            .map_err(|e| PtyError::Internal(format!("elevate: {e}")))?
        {
            Some(channel) => {
                *session.elevated.lock() = Some(channel);
                Ok(true)
            }
            None => Ok(false), // 使用者取消 UAC
        }
    }

    #[cfg(not(windows))]
    pub fn elevate<F, D>(
        &self,
        _id: &str,
        _shell_variant: super::cd_parser::ShellVariant,
        _on_output: F,
        _on_disconnect: D,
    ) -> PtyResult<bool>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
        D: FnMut() + Send + 'static,
    {
        Err(PtyError::Internal("elevation not supported on this platform".into()))
    }

    pub fn is_elevated(&self, id: &str) -> Option<bool> {
        self.sessions.lock().get(id).map(|s| already_connected(&s.elevated))
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
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    // ── single-flight guard for `elevate` (`already_connected`) ────────────
    //
    // `PtyManager::elevate` itself is `#[cfg(windows)]` (it calls the real
    // `spawn_windows`, which needs `ShellExecuteExW`), so it can't be
    // exercised directly here. `already_connected` is the free function that
    // holds the actual guard logic and has nothing platform-specific about
    // it, so these tests build `ElevatedChannel`s directly — same mock-
    // transport pattern `elevated.rs`'s own tests use — to cover the
    // invariant `elevate` depends on.

    /// A reader whose `read` never returns: it always blocks. Used to keep an
    /// `ElevatedChannel`'s background reader thread parked mid-read (as it
    /// would be while genuinely connected) instead of hitting EOF, so the
    /// channel's state stays `Connected` for the life of the test. The
    /// spawned thread is intentionally never joined — it's harmless to leak
    /// for the remainder of the test binary's process.
    struct BlockingReader;
    impl Read for BlockingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
    }

    #[test]
    fn already_connected_is_false_when_no_channel_is_set() {
        let elevated: Mutex<Option<super::super::elevated::ElevatedChannel>> = Mutex::new(None);
        assert!(!already_connected(&elevated));
    }

    #[test]
    fn already_connected_is_true_for_a_connected_channel() {
        let writer: Box<dyn Write + Send> = Box::new(Vec::<u8>::new());
        let channel = super::super::elevated::ElevatedChannel::new(
            BlockingReader,
            writer,
            |_bytes| {},
            || {},
        );
        let elevated = Mutex::new(Some(channel));
        assert!(
            already_connected(&elevated),
            "a freshly connected channel must report already_connected"
        );
    }

    #[test]
    fn already_connected_is_false_once_the_channel_has_disconnected() {
        // Empty reader hits EOF immediately, flipping the channel to
        // Disconnected almost right away (mirrors elevated.rs's own
        // `disconnect_callback_fires_when_transport_hits_eof` test).
        let reader = std::io::Cursor::new(Vec::<u8>::new());
        let writer: Box<dyn Write + Send> = Box::new(Vec::<u8>::new());
        let (disc_tx, disc_rx) = mpsc::channel::<()>();
        let channel = super::super::elevated::ElevatedChannel::new(reader, writer, |_| {}, move || {
            let _ = disc_tx.send(());
        });
        disc_rx.recv_timeout(Duration::from_secs(2)).expect("must disconnect");

        let elevated = Mutex::new(Some(channel));
        assert!(
            !already_connected(&elevated),
            "a disconnected channel must not short-circuit a fresh elevate() call"
        );
    }

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

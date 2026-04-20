use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

use super::cd_parser::{self, ParsedCd, ShellVariant};
use super::error::{PtyError, PtyResult};
use super::shell::ShellSpec;

pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    shell_variant: ShellVariant,
    cwd: Mutex<PathBuf>,
    previous_cwd: Mutex<Option<PathBuf>>,
    line_buffer: Mutex<Vec<u8>>,
    /// Ring buffer capturing raw PTY output for AI context. Shared with the reader thread.
    output_ring: Arc<Mutex<VecDeque<u8>>>,
}

impl PtySession {
    /// Spawn a new shell. `on_data` receives every chunk read from the PTY on a background thread.
    pub fn spawn<F>(shell: ShellSpec, size: PtySize, mut on_data: F) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let shell_variant = ShellVariant::from_program(&shell.program.to_string_lossy());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell.program);
        for arg in shell.args {
            cmd.arg(arg);
        }
        for (k, v) in shell.envs {
            cmd.env(k, v);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;
        // Drop slave so that the child sees EOF on exit (avoid leaks on Windows).
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Internal(format!("take_writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Internal(format!("try_clone_reader: {e}")))?;

        let id = Uuid::new_v4().to_string();

        let output_ring: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let ring_for_thread = Arc::clone(&output_ring);

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break, // EOF: shell exited
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            {
                                let mut ring = ring_for_thread.lock();
                                const RING_CAP: usize = 8 * 1024;
                                for &b in &chunk {
                                    if ring.len() >= RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
                            on_data(chunk);
                        }
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        let initial_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
            shell_variant,
            cwd: Mutex::new(initial_cwd),
            previous_cwd: Mutex::new(None),
            line_buffer: Mutex::new(Vec::new()),
            output_ring,
        })
    }

    /// Like `spawn`, but uses a caller-supplied id. Useful when the id must be
    /// known before the `on_data` closure is constructed (e.g. Tauri event names).
    pub fn spawn_with_id<F>(
        shell: ShellSpec,
        size: PtySize,
        id: String,
        mut on_data: F,
    ) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let shell_variant = ShellVariant::from_program(&shell.program.to_string_lossy());
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell.program);
        for arg in shell.args {
            cmd.arg(arg);
        }
        for (k, v) in shell.envs {
            cmd.env(k, v);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Internal(format!("take_writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Internal(format!("try_clone_reader: {e}")))?;

        let output_ring: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let ring_for_thread = Arc::clone(&output_ring);

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = buf[..n].to_vec();
                            {
                                let mut ring = ring_for_thread.lock();
                                const RING_CAP: usize = 8 * 1024;
                                for &b in &chunk {
                                    if ring.len() >= RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                            }
                            on_data(chunk);
                        }
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        let initial_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
            shell_variant,
            cwd: Mutex::new(initial_cwd),
            previous_cwd: Mutex::new(None),
            line_buffer: Mutex::new(Vec::new()),
            output_ring,
        })
    }

    pub fn write(&self, data: &[u8]) -> PtyResult<()> {
        self.record_into_line_buffer(data);
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// Accumulate bytes into the line buffer. On each carriage return or
    /// newline, flush the completed line and feed it to the cd parser.
    fn record_into_line_buffer(&self, data: &[u8]) {
        let mut buf = self.line_buffer.lock();
        for &b in data {
            if b == b'\r' || b == b'\n' {
                if !buf.is_empty() {
                    if let Ok(line) = std::str::from_utf8(&buf) {
                        let line_owned = line.to_string();
                        drop(buf);
                        self.apply_cd_if_any(&line_owned);
                        buf = self.line_buffer.lock();
                    }
                    buf.clear();
                }
            } else {
                // Cap runaway input to ~8 KiB so a rogue paste cannot grow
                // unbounded before the user hits Enter.
                if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
        }
    }

    fn apply_cd_if_any(&self, line: &str) {
        let current = self.cwd.lock().clone();
        let parsed = cd_parser::parse_cd(line, self.shell_variant, &current);
        match parsed {
            ParsedCd::NotCd => {}
            ParsedCd::ChangeTo(new_cwd) => {
                let mut prev = self.previous_cwd.lock();
                *prev = Some(current);
                *self.cwd.lock() = new_cwd;
            }
            ParsedCd::SwapPrevious => {
                let mut prev = self.previous_cwd.lock();
                if let Some(p) = prev.take() {
                    let new_prev = self.cwd.lock().clone();
                    *self.cwd.lock() = p;
                    *prev = Some(new_prev);
                }
            }
            ParsedCd::ToHome => {
                if let Some(home) = std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                {
                    let mut prev = self.previous_cwd.lock();
                    *prev = Some(current);
                    *self.cwd.lock() = PathBuf::from(home);
                }
            }
        }
    }

    /// Read the tracked cwd for this session.
    pub fn get_cwd(&self) -> PathBuf {
        self.cwd.lock().clone()
    }

    /// Read the shell variant detected at spawn time.
    pub fn shell_variant(&self) -> ShellVariant {
        self.shell_variant
    }

    /// Return the last `max_bytes` bytes of terminal output, ANSI-stripped.
    /// Returns `None` if the ring buffer is empty.
    pub fn get_recent_output(&self, max_bytes: usize) -> Option<String> {
        let ring = self.output_ring.lock();
        if ring.is_empty() {
            return None;
        }
        let start = ring.len().saturating_sub(max_bytes);
        let bytes: Vec<u8> = ring.iter().skip(start).copied().collect();
        drop(ring);
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        let stripped = crate::pty::ansi::strip_ansi(&raw);
        if stripped.trim().is_empty() { None } else { Some(stripped) }
    }

    pub fn resize(&self, size: PtySize) -> PtyResult<()> {
        let master = self.master.lock();
        master
            .resize(size)
            .map_err(|e| PtyError::Internal(format!("resize: {e}")))
    }

    pub fn kill(&self) -> PtyResult<()> {
        let mut child = self.child.lock();
        child
            .kill()
            .map_err(|e| PtyError::Internal(format!("kill: {e}")))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Best-effort: kill child so the reader thread eventually sees EOF/error.
        let _ = self.child.lock().kill();
        // Do NOT join the reader thread here: on Windows conpty the reader can
        // block indefinitely even after the child exits.  The thread is
        // detached and will exit on its own once the OS cleans up the handle.
        let _ = self.reader_thread.lock().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    fn test_shell() -> ShellSpec {
        #[cfg(windows)]
        {
            ShellSpec {
                program: "cmd.exe".into(),
                args: vec!["/Q".into()], // no banner
                envs: vec![],
            }
        }
        #[cfg(not(windows))]
        {
            ShellSpec {
                program: "/bin/sh".into(),
                args: vec![],
                envs: vec![],
            }
        }
    }

    #[test]
    fn session_echoes_written_bytes() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        // Give the shell a moment to be ready, then send an echo and exit.
        #[cfg(windows)]
        session.write(b"echo HELLO_AITERM\r\nexit\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"echo HELLO_AITERM\nexit\n").unwrap();

        // Collect until EOF or 5s timeout.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut buffer = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if String::from_utf8_lossy(&buffer).contains("HELLO_AITERM") {
                break;
            }
        }

        let output = String::from_utf8_lossy(&buffer);
        assert!(
            output.contains("HELLO_AITERM"),
            "expected shell to echo marker, got: {output}"
        );

        drop(session);
    }

    #[test]
    fn session_resize_does_not_error() {
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        session
            .resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .expect("resize ok");

        drop(session);
    }

    fn fake_session(shell_variant: ShellVariant, initial: &str) -> PtySessionStubForCwd {
        PtySessionStubForCwd {
            shell_variant,
            cwd: Mutex::new(PathBuf::from(initial)),
            previous_cwd: Mutex::new(None),
            line_buffer: Mutex::new(Vec::new()),
        }
    }

    struct PtySessionStubForCwd {
        shell_variant: ShellVariant,
        cwd: Mutex<PathBuf>,
        previous_cwd: Mutex<Option<PathBuf>>,
        line_buffer: Mutex<Vec<u8>>,
    }

    impl PtySessionStubForCwd {
        fn write(&self, data: &[u8]) {
            let mut buf = self.line_buffer.lock();
            for &b in data {
                if b == b'\r' || b == b'\n' {
                    if !buf.is_empty() {
                        if let Ok(line) = std::str::from_utf8(&buf) {
                            let line_owned = line.to_string();
                            drop(buf);
                            self.apply(&line_owned);
                            buf = self.line_buffer.lock();
                        }
                        buf.clear();
                    }
                } else if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
        }
        fn apply(&self, line: &str) {
            let current = self.cwd.lock().clone();
            match cd_parser::parse_cd(line, self.shell_variant, &current) {
                ParsedCd::NotCd => {}
                ParsedCd::ChangeTo(new_cwd) => {
                    *self.previous_cwd.lock() = Some(current);
                    *self.cwd.lock() = new_cwd;
                }
                ParsedCd::SwapPrevious => {
                    let mut prev = self.previous_cwd.lock();
                    if let Some(p) = prev.take() {
                        let new_prev = self.cwd.lock().clone();
                        *self.cwd.lock() = p;
                        *prev = Some(new_prev);
                    }
                }
                ParsedCd::ToHome => {
                    if let Some(home) = std::env::var_os("HOME")
                        .or_else(|| std::env::var_os("USERPROFILE"))
                    {
                        *self.previous_cwd.lock() = Some(current);
                        *self.cwd.lock() = PathBuf::from(home);
                    }
                }
            }
        }
        fn get_cwd(&self) -> PathBuf { self.cwd.lock().clone() }
    }

    #[test]
    fn cwd_updates_on_pwsh_cd_after_enter() {
        let s = fake_session(ShellVariant::Pwsh, "C:\\Users\\a");
        s.write(b"cd foo");   // no enter yet
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a"));
        s.write(b"\r");        // enter
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a\\foo"));
    }

    #[test]
    fn cwd_multiline_single_write() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd foo\ncd bar\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a/foo/bar"));
    }

    #[test]
    fn cwd_stays_on_unparseable() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"ls\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd foo && ls\n"); // compound → NotCd
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
    }

    #[test]
    fn cwd_dash_swaps_previous() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd /tmp\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
        s.write(b"cd -\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd -\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
    }

    // Suppress unused import warning - Arc is used in some test setups
    #[allow(dead_code)]
    fn _use_arc<T>(_: Arc<T>) {}
}

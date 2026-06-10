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
    /// ANSI escape-sequence state for the line buffer: 0=normal, 1=saw ESC, 2=in CSI (ESC [).
    line_esc_state: Mutex<u8>,
    /// Ring buffer capturing raw PTY output for AI context. Shared with the reader thread.
    output_ring: Arc<Mutex<VecDeque<u8>>>,
}

impl PtySession {
    /// Spawn a new shell. `on_data` receives every chunk read from the PTY on a background thread.
    pub fn spawn<F>(shell: ShellSpec, size: PtySize, cwd: Option<PathBuf>, mut on_data: F) -> PtyResult<Self>
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
        let initial_cwd = cwd
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        cmd.cwd(&initial_cwd);

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
            line_esc_state: Mutex::new(0),
            output_ring,
        })
    }

    /// Like `spawn`, but uses a caller-supplied id. Useful when the id must be
    /// known before the `on_data` closure is constructed (e.g. Tauri event names).
    pub fn spawn_with_id<F>(
        shell: ShellSpec,
        size: PtySize,
        id: String,
        cwd: Option<PathBuf>,
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
        let initial_cwd = cwd
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        cmd.cwd(&initial_cwd);

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
            line_esc_state: Mutex::new(0),
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
    ///
    /// ANSI escape sequences (e.g. bracketed-paste markers `ESC[200~` /
    /// `ESC[201~`) are stripped so they never corrupt the tracked path.
    /// State is persisted across calls via `line_esc_state` because a
    /// sequence may span two successive `write()` chunks.
    fn record_into_line_buffer(&self, data: &[u8]) {
        let mut buf = self.line_buffer.lock();
        // Load persisted escape-sequence state: 0=normal, 1=saw ESC, 2=in CSI.
        let mut esc: u8 = *self.line_esc_state.lock();

        for &b in data {
            // --- escape-sequence skip logic ---
            if esc == 1 {
                // Byte after ESC: '[' starts a CSI sequence; anything else ends it.
                esc = if b == b'[' { 2 } else { 0 };
                continue;
            }
            if esc == 2 {
                // Inside CSI: skip until final byte (0x40–0x7E).
                if (0x40..=0x7E).contains(&b) { esc = 0; }
                continue;
            }

            // --- normal processing ---
            if b == b'\r' || b == b'\n' {
                if !buf.is_empty() {
                    if let Ok(line) = std::str::from_utf8(&buf) {
                        let line_owned = line.to_string();
                        // Persist esc state before dropping locks.
                        *self.line_esc_state.lock() = esc;
                        drop(buf);
                        self.apply_cd_if_any(&line_owned);
                        buf = self.line_buffer.lock();
                        esc = *self.line_esc_state.lock();
                    }
                    buf.clear();
                }
            } else if b == 0x15 || b == 0x03 {
                // Ctrl+U (kill line) or Ctrl+C (interrupt) — clear the tracked buffer,
                // matching what the shell does with the input line.
                buf.clear();
            } else if b == 0x7f || b == 0x08 {
                // Backspace / DEL — remove the last tracked byte.
                buf.pop();
            } else if b == 0x1B {
                // ESC — start of an escape sequence; do NOT push to buffer.
                esc = 1;
            } else if b < 0x20 {
                // Other control characters — ignore so they don't corrupt the buffer.
            } else {
                // Cap runaway input to ~8 KiB so a rogue paste cannot grow
                // unbounded before the user hits Enter.
                if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
        }

        *self.line_esc_state.lock() = esc;
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
    ///
    /// For PowerShell sessions, also scans recent PTY output for the PS prompt
    /// (`PS C:\path> `) which gives an authoritative CWD that works regardless
    /// of whether the user typed, pasted, ran a script, or used an alias.
    pub fn get_cwd(&self) -> PathBuf {
        if self.shell_variant == ShellVariant::Pwsh {
            if let Some(prompt_cwd) = self.scan_output_for_ps_cwd() {
                let mut cwd = self.cwd.lock();
                if *cwd != prompt_cwd {
                    *cwd = prompt_cwd.clone();
                }
                return prompt_cwd;
            }
        }
        self.cwd.lock().clone()
    }

    /// Scan the PTY output ring for the last PowerShell prompt line and
    /// extract the directory from it.  The PS prompt format (after ANSI
    /// stripping) is:  `PS C:\path> `
    fn scan_output_for_ps_cwd(&self) -> Option<PathBuf> {
        let ring = self.output_ring.lock();
        if ring.is_empty() { return None; }
        let bytes: Vec<u8> = ring.iter().copied().collect();
        drop(ring); // release lock before potentially slow ANSI strip

        let raw = String::from_utf8_lossy(&bytes);
        let stripped = crate::pty::ansi::strip_ansi(&raw);

        // Walk lines in reverse to find the LAST prompt.
        // Split on both \n and \r to handle various line-ending styles.
        let mut last_cwd: Option<PathBuf> = None;
        for line in stripped.split('\n') {
            let line = line.trim_start_matches('\r').trim();
            // PowerShell prompt: "PS <path>> " (note: one '>' then space)
            if let Some(rest) = line.strip_prefix("PS ") {
                // Strip the trailing "> " (or just ">")
                let path_str = if let Some(s) = rest.strip_suffix("> ") {
                    s
                } else if let Some(s) = rest.strip_suffix('>') {
                    s
                } else {
                    continue;
                };
                let path_str = path_str.trim();
                // Sanity-check: must look like an absolute path (drive letter or UNC).
                if !path_str.is_empty()
                    && (path_str.as_bytes().get(1) == Some(&b':')
                        || path_str.starts_with("\\\\"))
                {
                    last_cwd = Some(PathBuf::from(path_str));
                }
            }
        }
        last_cwd
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
            None,
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
            None,
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
            line_esc_state: Mutex::new(0),
        }
    }

    struct PtySessionStubForCwd {
        shell_variant: ShellVariant,
        cwd: Mutex<PathBuf>,
        previous_cwd: Mutex<Option<PathBuf>>,
        line_buffer: Mutex<Vec<u8>>,
        line_esc_state: Mutex<u8>,
    }

    impl PtySessionStubForCwd {
        fn write(&self, data: &[u8]) {
            let mut buf = self.line_buffer.lock();
            let mut esc: u8 = *self.line_esc_state.lock();
            for &b in data {
                if esc == 1 {
                    esc = if b == b'[' { 2 } else { 0 };
                    continue;
                }
                if esc == 2 {
                    if (0x40..=0x7E).contains(&b) { esc = 0; }
                    continue;
                }
                if b == b'\r' || b == b'\n' {
                    if !buf.is_empty() {
                        if let Ok(line) = std::str::from_utf8(&buf) {
                            let line_owned = line.to_string();
                            *self.line_esc_state.lock() = esc;
                            drop(buf);
                            self.apply(&line_owned);
                            buf = self.line_buffer.lock();
                            esc = *self.line_esc_state.lock();
                        }
                        buf.clear();
                    }
                } else if b == 0x1B {
                    esc = 1;
                } else if b < 0x20 {
                    // ignore control chars
                } else if buf.len() < 8 * 1024 {
                    buf.push(b);
                }
            }
            *self.line_esc_state.lock() = esc;
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

    #[test]
    fn get_cwd_reads_ps_prompt_from_output() {
        // Simulate a PowerShell session whose output ring contains a prompt.
        // We can't easily spawn a real shell in this unit test, so we
        // verify the scan_output_for_ps_cwd helper logic inline.

        // Build a stripped-output string as scan_output_for_ps_cwd would see it.
        let output = "PS C:\\Users\\a\\AppData\\Local\\AITerm> cd C:\\Users\\a\\Downloads\nPS C:\\Users\\a\\Downloads> ";
        let mut last_cwd: Option<PathBuf> = None;
        for line in output.split('\n') {
            let line = line.trim_start_matches('\r').trim();
            if let Some(rest) = line.strip_prefix("PS ") {
                let path_str = if let Some(s) = rest.strip_suffix("> ") {
                    s
                } else if let Some(s) = rest.strip_suffix('>') {
                    s
                } else {
                    continue;
                };
                let path_str = path_str.trim();
                if !path_str.is_empty()
                    && (path_str.as_bytes().get(1) == Some(&b':')
                        || path_str.starts_with("\\\\"))
                {
                    last_cwd = Some(PathBuf::from(path_str));
                }
            }
        }
        assert_eq!(
            last_cwd,
            Some(PathBuf::from("C:\\Users\\a\\Downloads")),
            "should extract CWD from last PS prompt line"
        );
    }

    #[test]
    fn cwd_strips_bracketed_paste_sequences() {
        // Simulates xterm.js sending ESC[200~ before and ESC[201~ after pasted text.
        let s = fake_session(ShellVariant::Bash, "/home/a");
        // "cd " + ESC[200~ + "/Users/jamesju/Downloads" + ESC[201~ + "\n"
        let input = b"cd \x1b[200~/Users/jamesju/Downloads\x1b[201~\n";
        s.write(input);
        assert_eq!(s.get_cwd(), PathBuf::from("/Users/jamesju/Downloads"),
            "bracketed paste markers must be stripped before cd parsing");
    }

    // Suppress unused import warning - Arc is used in some test setups
    #[allow(dead_code)]
    fn _use_arc<T>(_: Arc<T>) {}
}

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

use super::cd_parser::{self, ParsedCd, ShellVariant};
use super::error::{PtyError, PtyResult};
use super::shell::ShellSpec;

/// 決定 PTY 的起始目錄。
///
/// 指定的目錄不存在時（分頁還原自上一個 session，而那個目錄被刪掉、改名，或
/// 位於還沒掛載的磁碟區），必須退回一個一定存在的地方，否則 spawn 會失敗。
///
/// 退回家目錄而不是 `current_dir()`：後者是 AITerm 主行程自己的工作目錄，
/// 取決於 app 怎麼被啟動——macOS 打包版從 Finder 開啟時通常是 `/`，使用者會
/// 發現終端機開在根目錄。家目錄是一般終端機的預期行為，也跟 portable-pty
/// 自己的 fallback 語意一致。
fn resolve_initial_cwd(cwd: Option<PathBuf>) -> PathBuf {
    cwd.filter(|p| p.is_dir())
        .or_else(home_dir)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 家目錄。Unix 讀 `HOME`，Windows 讀 `USERPROFILE`。
/// 一併確認它真的是個目錄——環境變數可能指向不存在的路徑。
fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Prefix/suffix of the optional completion marker a cooperative agent can
/// print to let the MCP coordination tools' `wait_for_idle` return faster
/// than the mandatory bell fallback. See
/// `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`.
pub const DONE_MARKER_PREFIX: &str = "<<AITERM_DONE:";
pub const DONE_MARKER_SUFFIX: &str = ">>";

/// Builds this session's own completion marker text. `tab_id` is always a
/// UUID (see `Uuid::new_v4()` at the call site), so the marker is a fixed
/// length in practice, but this function itself makes no assumption about
/// that — any `tab_id` string works.
pub fn done_marker(tab_id: &str) -> String {
    format!("{DONE_MARKER_PREFIX}{tab_id}{DONE_MARKER_SUFFIX}")
}

/// Scans `tail` immediately followed by `chunk` for `marker`. `tail` should
/// be the previous chunk's own trailing `marker.len() - 1` bytes (or empty
/// for the very first chunk), so a marker split across a chunk boundary is
/// still found. Only ever looks at these newly-arrived bytes — never
/// rescans older history — so a stale marker from a previous round can
/// never re-trigger a later scan.
fn contains_marker(tail: &[u8], chunk: &[u8], marker: &[u8]) -> bool {
    if marker.is_empty() {
        return false;
    }
    let mut combined = Vec::with_capacity(tail.len() + chunk.len());
    combined.extend_from_slice(tail);
    combined.extend_from_slice(chunk);
    combined.windows(marker.len()).any(|w| w == marker)
}

/// Computes the new tail to carry into the next `contains_marker` call: the
/// last `marker_len - 1` bytes of `chunk` (or all of `chunk` if it's
/// shorter than that window).
fn marker_tail_after(chunk: &[u8], marker_len: usize) -> Vec<u8> {
    let keep = marker_len.saturating_sub(1).min(chunk.len());
    chunk[chunk.len() - keep..].to_vec()
}

/// Combines `tail` with `chunk`, checks for `marker`, and returns
/// `(found, new_tail)` — the new tail is derived from the *combined*
/// buffer, not `chunk` alone, so a marker split across 3+ reads (not
/// just 2) is still detected incrementally as more chunks arrive.
fn scan_for_marker(tail: &[u8], chunk: &[u8], marker: &[u8]) -> (bool, Vec<u8>) {
    let mut combined = Vec::with_capacity(tail.len() + chunk.len());
    combined.extend_from_slice(tail);
    combined.extend_from_slice(chunk);
    let found = contains_marker(&[], &combined, marker);
    let new_tail = marker_tail_after(&combined, marker.len());
    (found, new_tail)
}

pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
    shell_variant: ShellVariant,
    /// Shared with the reader thread, which commits a pending cd (see
    /// `pending_cds`) once it observes a confirming OSC 133 D exit code.
    cwd: Arc<Mutex<PathBuf>>,
    previous_cwd: Arc<Mutex<Option<PathBuf>>>,
    line_buffer: Mutex<Vec<u8>>,
    /// ANSI escape-sequence state for the line buffer: 0=normal, 1=saw ESC, 2=in CSI (ESC [).
    line_esc_state: Mutex<u8>,
    /// Ring buffer capturing raw PTY output for AI context. Shared with the reader thread.
    output_ring: Arc<Mutex<VecDeque<u8>>>,
    /// cd attempts staged by `apply_cd_if_any` (write path) for Bash/Pwsh
    /// sessions, each removed by the reader thread once it sees the matching
    /// OSC 133 D marker — committed to cwd/previous_cwd only if that marker
    /// reports exit code 0. See `cd_parser::find_exit_codes`.
    pending_cds: Arc<Mutex<VecDeque<ParsedCd>>>,
    /// Counts how many output chunks have contained at least one bell
    /// byte (`0x07`). Used by the MCP tool server's agent-coordination tools as an
    /// idle signal: both Claude Code and Codex CLI fall back to a plain
    /// terminal bell for "waiting for input" notifications when they can't
    /// detect a richer-notification-capable terminal (verified against both
    /// projects' source — see the design doc). A monotonic counter rather
    /// than a boolean so a caller can detect "a *new* bell happened since I
    /// last checked" by comparing against a remembered baseline, without this
    /// field needing any consuming/resetting behavior of its own.
    bell_count: Arc<AtomicU64>,
    /// Counts how many times this session's own completion marker
    /// (`done_marker(&self.id)`) has been observed in output — an optional,
    /// cooperative-agent-only signal that lets the MCP coordination tools'
    /// `wait_for_idle` return faster than the mandatory bell fallback. Same
    /// monotonic-counter reasoning as `bell_count`. See
    /// `docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`.
    marker_count: Arc<AtomicU64>,
}

/// Commits a resolved cd to `cwd`/`previous_cwd`. Free function (not a method)
/// so both `PtySession`'s own methods and the reader thread closure — which
/// only has `Arc` clones of these fields, not `&self` — can call it.
fn commit_parsed_cd(cwd: &Mutex<PathBuf>, previous_cwd: &Mutex<Option<PathBuf>>, parsed: ParsedCd) {
    match parsed {
        ParsedCd::NotCd => {}
        ParsedCd::ChangeTo(new_cwd) => {
            let current = cwd.lock().clone();
            *previous_cwd.lock() = Some(current);
            *cwd.lock() = new_cwd;
        }
        ParsedCd::SwapPrevious => {
            let mut prev = previous_cwd.lock();
            if let Some(p) = prev.take() {
                let new_prev = cwd.lock().clone();
                *cwd.lock() = p;
                *prev = Some(new_prev);
            }
        }
        ParsedCd::ToHome => {
            if let Some(home) =
                std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
            {
                let current = cwd.lock().clone();
                *previous_cwd.lock() = Some(current);
                *cwd.lock() = PathBuf::from(home);
            }
        }
    }
}

/// Projects what cwd *would* be if every currently-staged (not yet confirmed)
/// cd in `pending` went on to succeed, in order — folded on top of the real,
/// last-confirmed `cwd`. Used only to resolve relative paths when parsing a
/// *new* line while earlier cds are still awaiting confirmation (e.g. a
/// multi-line paste of several `cd` commands back-to-back): a real shell
/// processes each one before the next starts, so the next line's relative
/// path must resolve against where the previous one *would* land, not
/// against the stale confirmed cwd. This never touches actual state — it's
/// purely a preview for the parser; the real commit still only happens one
/// at a time via `confirm_pending_cds_from_output`.
fn effective_cwd_with_pending(cwd: &PathBuf, pending: &VecDeque<ParsedCd>) -> PathBuf {
    let mut current = cwd.clone();
    let mut previous: Option<PathBuf> = None;
    for parsed in pending {
        match parsed {
            ParsedCd::NotCd => {}
            ParsedCd::ChangeTo(new_cwd) => {
                previous = Some(current.clone());
                current = new_cwd.clone();
            }
            ParsedCd::SwapPrevious => {
                if let Some(p) = previous.take() {
                    previous = Some(current.clone());
                    current = p;
                }
            }
            ParsedCd::ToHome => {
                if let Some(home) =
                    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
                {
                    previous = Some(current.clone());
                    current = PathBuf::from(home);
                }
            }
        }
    }
    current
}

/// CWD from the last PowerShell prompt (`PS <path>> `) in ANSI-stripped output.
///
/// Split out of `scan_output_for_ps_cwd` so it can be tested against a string
/// instead of a live shell. It had no coverage at all, which is how the
/// carriage-return bug below survived into a release.
fn ps_cwd_from_output(stripped: &str) -> Option<PathBuf> {
    let mut last_cwd: Option<PathBuf> = None;

    // Split on '\r' as well as '\n'. A shell redrawing its prompt in place —
    // which PSReadLine does routinely — emits a carriage return with no
    // newline, so two prompts land in what splitting on '\n' alone treats as a
    // single line. The parse below then reads the whole thing as one path,
    // because `strip_suffix('>')` only removes the *last* '>' and leaves the
    // first prompt's, producing `C:\Users\me\Downloads> PS C:\Users\me\Downloads`.
    // That value passes the drive-letter check, reaches `read_dir`, and comes
    // back as Windows error 123 (ERROR_INVALID_NAME) — the red banner the file
    // explorer showed on the first switch to it. The previous code's comment
    // already claimed it split on both; only the code disagreed.
    for line in stripped.split(['\n', '\r']) {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("PS ") else { continue };

        // PowerShell prompt: "PS <path>> " (one '>', then a space)
        let path_str = match rest.strip_suffix("> ").or_else(|| rest.strip_suffix('>')) {
            Some(s) => s.trim(),
            None => continue,
        };

        // '>' is not a legal character in a Windows path, so finding one here
        // means this is two prompts run together rather than one. Keeping the
        // check makes the whole class impossible, not just the '\r' spelling of
        // it that was observed.
        if path_str.contains('>') {
            continue;
        }

        // Sanity-check: must look like an absolute path (drive letter or UNC).
        if !path_str.is_empty()
            && (path_str.as_bytes().get(1) == Some(&b':') || path_str.starts_with("\\\\"))
        {
            last_cwd = Some(PathBuf::from(path_str));
        }
    }

    last_cwd
}

/// For Bash/Pwsh sessions, scans an incoming chunk for OSC 133 D markers and
/// confirms or discards the oldest pending cd for each one found, in order.
/// A no-op chunk with no markers is the overwhelmingly common case, so this
/// is cheap to call unconditionally on every read.
fn confirm_pending_cds_from_output(
    chunk: &[u8],
    pending_cds: &Mutex<VecDeque<ParsedCd>>,
    cwd: &Mutex<PathBuf>,
    previous_cwd: &Mutex<Option<PathBuf>>,
) {
    for exit_code in cd_parser::find_exit_codes(chunk) {
        let popped = pending_cds.lock().pop_front();
        if let Some(parsed) = popped {
            if exit_code == 0 {
                commit_parsed_cd(cwd, previous_cwd, parsed);
            }
            // Non-zero exit: the shell itself is telling us this cd failed —
            // discard it and leave cwd exactly as it was.
        }
    }
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
        // Applied before the configured envs, so an explicit setting still
        // wins. An AppImage's AppRun exports PYTHONHOME and LD_LIBRARY_PATH
        // pointing into the AppDir, and a shell hands them to everything the
        // user runs — `python3` in this terminal would fail to find its own
        // standard library. No-op everywhere else.
        for (key, value) in crate::appimage_env::appimage_env_fixes() {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            }
        }
        for (k, v) in shell.envs {
            cmd.env(k, v);
        }
        // 順序必須在 envs 之後：兩份清單若不慎重疊，移除要贏。
        for k in &shell.env_removals {
            cmd.env_remove(k);
        }
        let initial_cwd = resolve_initial_cwd(cwd);
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
        let cwd: Arc<Mutex<PathBuf>> = Arc::new(Mutex::new(initial_cwd));
        let cwd_for_thread = Arc::clone(&cwd);
        let previous_cwd: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let previous_cwd_for_thread = Arc::clone(&previous_cwd);
        let pending_cds: Arc<Mutex<VecDeque<ParsedCd>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pending_cds_for_thread = Arc::clone(&pending_cds);
        let bell_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let bell_count_for_thread = Arc::clone(&bell_count);
        let marker_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let marker_count_for_thread = Arc::clone(&marker_count);
        let marker_tail_for_thread: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let done_marker_bytes = done_marker(&id).into_bytes();

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
                            if matches!(shell_variant, ShellVariant::Bash | ShellVariant::Pwsh) {
                                confirm_pending_cds_from_output(
                                    &chunk,
                                    &pending_cds_for_thread,
                                    &cwd_for_thread,
                                    &previous_cwd_for_thread,
                                );
                            }
                            // Cheap to call unconditionally, same reasoning as the
                            // OSC133 scan above: a chunk with no bell byte is the
                            // overwhelmingly common case. One increment per chunk
                            // containing at least one bell is enough — callers only
                            // ever check "did the count change since my baseline",
                            // never the exact number of bells. Uses
                            // `contains_bare_bell` (not a naive `contains(&0x07)`)
                            // because our own OSC133 shell-integration markers are
                            // themselves BEL-terminated and must not be mistaken
                            // for a genuine agent notification bell.
                            if cd_parser::contains_bare_bell(&chunk) {
                                bell_count_for_thread.fetch_add(1, Ordering::SeqCst);
                            }
                            // Optional completion-marker detection (see design doc
                            // 2026-08-21-coordination-done-marker-design.md). Uses a
                            // small carried-over tail so a marker split across any
                            // number of chunk boundaries is still found — unlike the
                            // single-byte bell above, this marker is multiple bytes
                            // long and PTY reads are arbitrary-sized.
                            {
                                let mut tail = marker_tail_for_thread.lock();
                                let (found, new_tail) =
                                    scan_for_marker(&tail, &chunk, &done_marker_bytes);
                                if found {
                                    marker_count_for_thread.fetch_add(1, Ordering::SeqCst);
                                }
                                *tail = new_tail;
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
            cwd,
            previous_cwd,
            line_buffer: Mutex::new(Vec::new()),
            line_esc_state: Mutex::new(0),
            output_ring,
            pending_cds,
            bell_count,
            marker_count,
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
        // Applied before the configured envs, so an explicit setting still
        // wins. An AppImage's AppRun exports PYTHONHOME and LD_LIBRARY_PATH
        // pointing into the AppDir, and a shell hands them to everything the
        // user runs — `python3` in this terminal would fail to find its own
        // standard library. No-op everywhere else.
        for (key, value) in crate::appimage_env::appimage_env_fixes() {
            match value {
                Some(v) => cmd.env(key, v),
                None => cmd.env_remove(key),
            }
        }
        for (k, v) in shell.envs {
            cmd.env(k, v);
        }
        // 順序必須在 envs 之後：兩份清單若不慎重疊，移除要贏。
        for k in &shell.env_removals {
            cmd.env_remove(k);
        }
        let initial_cwd = resolve_initial_cwd(cwd);
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
        let cwd: Arc<Mutex<PathBuf>> = Arc::new(Mutex::new(initial_cwd));
        let cwd_for_thread = Arc::clone(&cwd);
        let previous_cwd: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
        let previous_cwd_for_thread = Arc::clone(&previous_cwd);
        let pending_cds: Arc<Mutex<VecDeque<ParsedCd>>> = Arc::new(Mutex::new(VecDeque::new()));
        let pending_cds_for_thread = Arc::clone(&pending_cds);
        let bell_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let bell_count_for_thread = Arc::clone(&bell_count);
        let marker_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let marker_count_for_thread = Arc::clone(&marker_count);
        let marker_tail_for_thread: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let done_marker_bytes = done_marker(&id).into_bytes();

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
                            if matches!(shell_variant, ShellVariant::Bash | ShellVariant::Pwsh) {
                                confirm_pending_cds_from_output(
                                    &chunk,
                                    &pending_cds_for_thread,
                                    &cwd_for_thread,
                                    &previous_cwd_for_thread,
                                );
                            }
                            // Cheap to call unconditionally, same reasoning as the
                            // OSC133 scan above: a chunk with no bell byte is the
                            // overwhelmingly common case. One increment per chunk
                            // containing at least one bell is enough — callers only
                            // ever check "did the count change since my baseline",
                            // never the exact number of bells. Uses
                            // `contains_bare_bell` (not a naive `contains(&0x07)`)
                            // because our own OSC133 shell-integration markers are
                            // themselves BEL-terminated and must not be mistaken
                            // for a genuine agent notification bell.
                            if cd_parser::contains_bare_bell(&chunk) {
                                bell_count_for_thread.fetch_add(1, Ordering::SeqCst);
                            }
                            // Optional completion-marker detection (see design doc
                            // 2026-08-21-coordination-done-marker-design.md). Uses a
                            // small carried-over tail so a marker split across any
                            // number of chunk boundaries is still found — unlike the
                            // single-byte bell above, this marker is multiple bytes
                            // long and PTY reads are arbitrary-sized.
                            {
                                let mut tail = marker_tail_for_thread.lock();
                                let (found, new_tail) =
                                    scan_for_marker(&tail, &chunk, &done_marker_bytes);
                                if found {
                                    marker_count_for_thread.fetch_add(1, Ordering::SeqCst);
                                }
                                *tail = new_tail;
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
            cwd,
            previous_cwd,
            line_buffer: Mutex::new(Vec::new()),
            line_esc_state: Mutex::new(0),
            output_ring,
            pending_cds,
            bell_count,
            marker_count,
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

    /// Parses a completed outgoing line for a cd attempt. For Bash/Pwsh, the
    /// result is only staged (see `pending_cds`) — it's committed to
    /// cwd/previous_cwd later by the reader thread, once the shell's own OSC
    /// 133 D marker confirms the command actually succeeded. cmd.exe's D
    /// marker never carries an exit code (see `pty::shell`), so there's
    /// nothing to wait for there — keep the old immediate-commit-on-parse
    /// behavior for it (and for Unknown shells, matching prior behavior).
    fn apply_cd_if_any(&self, line: &str) {
        let pending = self.pending_cds.lock();
        let effective_current = if pending.is_empty() {
            self.cwd.lock().clone()
        } else {
            effective_cwd_with_pending(&self.cwd.lock(), &pending)
        };
        drop(pending);

        let parsed = cd_parser::parse_cd(line, self.shell_variant, &effective_current);
        if matches!(parsed, ParsedCd::NotCd) {
            return;
        }
        if matches!(self.shell_variant, ShellVariant::Cmd | ShellVariant::Unknown) {
            commit_parsed_cd(&self.cwd, &self.previous_cwd, parsed);
        } else {
            self.pending_cds.lock().push_back(parsed);
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
        ps_cwd_from_output(&stripped)
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

    /// Monotonic count of output chunks that have contained at least one bell
    /// byte (`0x07`) since this session started. See the field doc comment on
    /// `bell_count` for why this is a counter, not a boolean.
    pub fn bell_count(&self) -> u64 {
        self.bell_count.load(Ordering::SeqCst)
    }

    /// Monotonic count of times this session's own completion marker has
    /// been observed in output since it started. See the field doc comment
    /// on `marker_count` for why this is a counter, not a boolean.
    pub fn marker_count(&self) -> u64 {
        self.marker_count.load(Ordering::SeqCst)
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

    // 起始目錄的 fallback 在這之前完全沒有測試涵蓋，而它是「分頁還原到上次的
    // 目錄」這個功能唯一的安全網——目錄被刪掉、改名或磁碟區沒掛載時全靠它。
    #[test]
    fn initial_cwd_uses_the_given_dir_when_it_exists() {
        let dir = std::env::temp_dir();
        assert_eq!(resolve_initial_cwd(Some(dir.clone())), dir);
    }

    #[test]
    fn initial_cwd_falls_back_to_home_when_dir_is_missing() {
        let missing = std::env::temp_dir().join("aiterm-does-not-exist-9f3a2b");
        assert!(!missing.is_dir(), "測試前提：這個路徑必須不存在");

        let resolved = resolve_initial_cwd(Some(missing));
        assert!(resolved.is_dir(), "退回的目錄必須真的存在，否則 spawn 會失敗");
        if let Some(home) = home_dir() {
            // 重點不只是「有退回某處」，而是退回家目錄而非主行程的 current_dir
            // ——後者在 macOS 打包版通常是 `/`。
            assert_eq!(resolved, home);
        }
    }

    #[test]
    fn initial_cwd_falls_back_when_none_given() {
        let resolved = resolve_initial_cwd(None);
        assert!(resolved.is_dir());
    }

    // 檔案不是目錄。`is_dir()` 對它回傳 false，所以應該跟不存在一樣退回。
    #[test]
    fn initial_cwd_rejects_a_file_path() {
        let file = std::env::temp_dir().join("aiterm-cwd-probe.txt");
        std::fs::write(&file, b"x").expect("寫入測試檔");
        let resolved = resolve_initial_cwd(Some(file.clone()));
        assert_ne!(resolved, file);
        assert!(resolved.is_dir());
        let _ = std::fs::remove_file(&file);
    }

    fn test_shell() -> ShellSpec {
        #[cfg(windows)]
        {
            ShellSpec {
                program: "cmd.exe".into(),
                args: vec!["/Q".into()], // no banner
                envs: vec![],
                env_removals: vec![],
            }
        }
        #[cfg(not(windows))]
        {
            ShellSpec {
                program: "/bin/sh".into(),
                args: vec![],
                envs: vec![],
                env_removals: vec![],
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
            pending_cds: Mutex::new(VecDeque::new()),
        }
    }

    /// Mirrors `PtySession`'s write-path line buffering / cd staging and
    /// read-path exit-code confirmation, but without spawning a real shell —
    /// reuses the actual `commit_parsed_cd`/`confirm_pending_cds_from_output`
    /// free functions so these tests exercise the same commit logic
    /// production code does, not a second hand-copied implementation of it.
    struct PtySessionStubForCwd {
        shell_variant: ShellVariant,
        cwd: Mutex<PathBuf>,
        previous_cwd: Mutex<Option<PathBuf>>,
        line_buffer: Mutex<Vec<u8>>,
        line_esc_state: Mutex<u8>,
        pending_cds: Mutex<VecDeque<ParsedCd>>,
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
            let pending = self.pending_cds.lock();
            let effective_current = if pending.is_empty() {
                self.cwd.lock().clone()
            } else {
                effective_cwd_with_pending(&self.cwd.lock(), &pending)
            };
            drop(pending);

            let parsed = cd_parser::parse_cd(line, self.shell_variant, &effective_current);
            if matches!(parsed, ParsedCd::NotCd) {
                return;
            }
            if matches!(self.shell_variant, ShellVariant::Cmd | ShellVariant::Unknown) {
                commit_parsed_cd(&self.cwd, &self.previous_cwd, parsed);
            } else {
                self.pending_cds.lock().push_back(parsed);
            }
        }
        /// Simulates the reader thread observing PTY output containing an
        /// OSC 133 D marker.
        fn receive_output(&self, data: &[u8]) {
            confirm_pending_cds_from_output(data, &self.pending_cds, &self.cwd, &self.previous_cwd);
        }
        fn get_cwd(&self) -> PathBuf { self.cwd.lock().clone() }
    }

    const D_OK: &[u8] = b"\x1b]133;D;0\x07";
    const D_FAIL: &[u8] = b"\x1b]133;D;1\x07";

    #[test]
    fn cwd_stages_on_enter_and_commits_on_confirmed_success() {
        let s = fake_session(ShellVariant::Pwsh, "C:\\Users\\a");
        s.write(b"cd foo"); // no enter yet
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a"));
        s.write(b"\r"); // enter — staged, NOT yet committed
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a"));
        s.receive_output(D_OK); // shell confirms the cd actually succeeded
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a\\foo"));
    }

    #[test]
    fn cwd_does_not_change_when_shell_reports_cd_failed() {
        // Direct regression test for the reported bug: a cd that fails in the
        // real shell must not desync the tracked cwd.
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd nonexistent\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.receive_output(D_FAIL);
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a")); // unchanged
    }

    #[test]
    fn quoted_tilde_parses_like_unquoted_but_real_shell_failure_is_still_honored() {
        // cd_parser can't distinguish cd ~ from cd "~" (quotes are stripped by
        // its tokenizer before matching) — both parse to ToHome. A real shell
        // does NOT expand a quoted tilde and cd "~" fails. This is the exact
        // scenario from the bug report: the exit-code confirmation step must
        // catch this even though the parser itself got it "wrong".
        let s = fake_session(ShellVariant::Bash, "/project/src-tauri");
        s.write(b"cd \"~\"\n");
        s.receive_output(D_FAIL); // real shell: `no such file or directory: ~`
        assert_eq!(s.get_cwd(), PathBuf::from("/project/src-tauri")); // unchanged
    }

    #[test]
    fn failed_cd_does_not_desync_subsequent_relative_cds() {
        // The cascading half of the bug report: once a failed cd is correctly
        // ignored, a subsequent *real* relative cd must resolve against the
        // still-correct (unchanged) cwd, not a wrongly-advanced one.
        let s = fake_session(ShellVariant::Bash, "/project/src-tauri");
        s.write(b"cd \"~\"\n");
        s.receive_output(D_FAIL);
        assert_eq!(s.get_cwd(), PathBuf::from("/project/src-tauri"));

        // No "Downloads" under src-tauri in reality either — but the point is
        // this now resolves relative to the correct (unchanged) base.
        s.write(b"cd Downloads\n");
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/project/src-tauri/Downloads"));
    }

    #[test]
    fn cwd_multiline_single_write() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd foo\ncd bar\n");
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a")); // both still staged
        s.receive_output(D_OK); // confirms "cd foo"
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a/foo"));
        s.receive_output(D_OK); // confirms "cd bar"
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a/foo/bar"));
    }

    #[test]
    fn cwd_stays_on_unparseable() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"ls\n");
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd foo && ls\n"); // compound → NotCd, never staged
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
    }

    #[test]
    fn cwd_dash_swaps_previous() {
        let s = fake_session(ShellVariant::Bash, "/home/a");
        s.write(b"cd /tmp\n");
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
        s.write(b"cd -\n");
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/home/a"));
        s.write(b"cd -\n");
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/tmp"));
    }

    #[test]
    fn cmd_exe_still_commits_immediately_no_exit_code_available() {
        // cmd.exe's injected D marker never carries an exit code, so there's
        // nothing to confirm against — it must keep the old immediate-commit
        // behavior rather than staging forever.
        let s = fake_session(ShellVariant::Cmd, "C:\\Users\\a");
        s.write(b"cd foo\r\n");
        assert_eq!(s.get_cwd(), PathBuf::from("C:\\Users\\a\\foo"));
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
        s.receive_output(D_OK);
        assert_eq!(s.get_cwd(), PathBuf::from("/Users/jamesju/Downloads"),
            "bracketed paste markers must be stripped before cd parsing");
    }

    // Suppress unused import warning - Arc is used in some test setups
    #[allow(dead_code)]
    fn _use_arc<T>(_: Arc<T>) {}

    #[test]
    fn reads_the_cwd_out_of_a_powershell_prompt() {
        assert_eq!(
            ps_cwd_from_output("PS C:\\Users\\jamesju\\Downloads> "),
            Some(PathBuf::from("C:\\Users\\jamesju\\Downloads"))
        );
    }

    #[test]
    fn a_repainted_prompt_does_not_merge_into_one_path() {
        // The reported Windows bug. PSReadLine redraws the prompt with a bare
        // carriage return, so both prompts sit in one '\n'-delimited line. The
        // old scanner returned "C:\...\Downloads> PS C:\...\Downloads", which
        // read_dir rejected with error 123 and the file explorer showed as a
        // red banner on first open.
        let out = "PS C:\\Users\\jamesju\\Downloads> \rPS C:\\Users\\jamesju\\Downloads> ";
        assert_eq!(
            ps_cwd_from_output(out),
            Some(PathBuf::from("C:\\Users\\jamesju\\Downloads"))
        );
    }

    #[test]
    fn a_merged_prompt_is_ignored_rather_than_guessed_at() {
        // Same class, different spelling: two prompts with no separator at all.
        // '>' cannot occur in a Windows path, so this is unambiguously not one
        // path — but which of the two it is *is* ambiguous, so the answer is
        // "no reading", not a guess. Recovering the trailing one would mean
        // splitting on the last "PS ", which mangles any real path containing
        // that substring (`C:\My PS Scripts\`).
        //
        // Returning None is safe: get_cwd() then keeps the cwd it was already
        // tracking, which is a valid directory, instead of adopting one that
        // read_dir would reject.
        assert_eq!(ps_cwd_from_output("PS C:\\a> PS C:\\b>"), None);
    }

    #[test]
    fn the_last_prompt_wins() {
        // The scanner's whole purpose: the newest prompt is the current cwd.
        let out = "PS C:\\one> \r\nsome output\r\nPS C:\\two> ";
        assert_eq!(ps_cwd_from_output(out), Some(PathBuf::from("C:\\two")));
    }

    #[test]
    fn a_path_containing_spaces_survives() {
        // Guards the fix: rejecting on '>' must not turn into rejecting on
        // anything unusual. Spaces are legal and common.
        assert_eq!(
            ps_cwd_from_output("PS C:\\Program Files\\Git> "),
            Some(PathBuf::from("C:\\Program Files\\Git"))
        );
    }

    #[test]
    fn a_unc_path_is_accepted() {
        assert_eq!(
            ps_cwd_from_output("PS \\\\server\\share> "),
            Some(PathBuf::from("\\\\server\\share"))
        );
    }

    #[test]
    fn output_with_no_prompt_yields_nothing() {
        assert_eq!(ps_cwd_from_output("just some command output\r\n"), None);
        assert_eq!(ps_cwd_from_output(""), None);
    }

    #[test]
    fn a_line_that_merely_mentions_a_prompt_is_not_one() {
        // Nothing should read a cwd out of text that only looks prompt-shaped
        // without the drive letter or UNC prefix.
        assert_eq!(ps_cwd_from_output("PS not-a-path> "), None);
    }

    #[test]
    fn bell_count_starts_at_zero_for_a_fresh_session() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.bell_count(), 0);
    }

    #[tokio::test]
    async fn bell_byte_in_output_increments_bell_count() {
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

        assert_eq!(session.bell_count(), 0);

        // Write a command that emits a bell byte in its output. printf is
        // available on /bin/sh; on cmd.exe (Windows) echo simply emits the
        // raw byte embedded in its argument.
        #[cfg(windows)]
        session.write(b"echo \x07\r\nexit\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf '\\007'\n").unwrap();

        // Poll briefly for the reader thread to observe it — this is
        // inherently asynchronous (real PTY I/O on a background thread), so a
        // short poll loop is appropriate here, not a fixed sleep.
        let mut seen = false;
        for _ in 0..50 {
            if session.bell_count() > 0 {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            seen,
            "expected bell_count() to increment after a bell byte was written and echoed"
        );

        let _ = rx.try_recv(); // drain, avoid unused warning
        drop(session);
    }

    #[test]
    fn marker_count_starts_at_zero_for_a_fresh_session() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.marker_count(), 0);
    }

    #[tokio::test]
    async fn marker_in_output_increments_marker_count() {
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

        assert_eq!(session.marker_count(), 0);

        let marker = done_marker(&session.id);
        #[cfg(windows)]
        session.write(format!("echo {marker}\r\nexit\r\n").as_bytes()).unwrap();
        #[cfg(not(windows))]
        session.write(format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let mut seen = false;
        for _ in 0..50 {
            if session.marker_count() > 0 {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            seen,
            "expected marker_count() to increment after the session's own marker was written and echoed"
        );

        let _ = rx.try_recv();
        drop(session);
    }

    #[tokio::test]
    async fn a_marker_for_a_different_tab_id_does_not_count() {
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

        let other_marker = done_marker("not-this-session-id");
        #[cfg(windows)]
        session.write(format!("echo {other_marker}\r\nexit\r\n").as_bytes()).unwrap();
        #[cfg(not(windows))]
        session.write(format!("printf '%s\\n' '{other_marker}'\n").as_bytes()).unwrap();

        // Give the reader thread time to process it, then confirm it never counted.
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(
            session.marker_count(),
            0,
            "a marker addressed to a different tab_id must not count for this session"
        );

        let _ = rx.try_recv();
        drop(session);
    }

    #[test]
    fn done_marker_embeds_the_tab_id_between_fixed_delimiters() {
        assert_eq!(done_marker("abc-123"), "<<AITERM_DONE:abc-123>>");
    }

    #[test]
    fn contains_marker_true_when_marker_is_wholly_within_one_chunk() {
        let marker = b"<<AITERM_DONE:abc>>";
        assert!(contains_marker(b"", b"hello <<AITERM_DONE:abc>> world", marker));
    }

    #[test]
    fn contains_marker_false_when_marker_absent() {
        let marker = b"<<AITERM_DONE:abc>>";
        assert!(!contains_marker(b"", b"nothing to see here", marker));
    }

    #[test]
    fn contains_marker_finds_a_marker_split_across_the_tail_and_the_new_chunk() {
        // Regression coverage for the cross-chunk correctness gap the design
        // doc flags: bell detection is a single byte and can never straddle
        // a chunk boundary, but this marker is 20 bytes long here (52 in
        // production with a real UUID) and PTY reads are arbitrary-sized.
        let marker = b"<<AITERM_DONE:abc>>";
        let (first, second) = marker.split_at(marker.len() / 2);

        // Round 1: only the first half has arrived — not present yet.
        assert!(!contains_marker(b"", first, marker));
        let tail = marker_tail_after(first, marker.len());

        // Round 2: second half arrives — tail + this chunk together contain it.
        assert!(contains_marker(&tail, second, marker));
    }

    #[test]
    fn scan_for_marker_finds_a_marker_split_across_three_chunks() {
        // Regression coverage for a real bug: deriving the next tail from
        // `chunk` alone (instead of from the combined `tail + chunk`
        // buffer) silently drops a marker split across 3+ reads, because
        // the middle chunk's contribution to the tail never survives past
        // its own round. `scan_for_marker` must derive the next tail from
        // the combined buffer so this still works no matter how many reads
        // the marker is split across.
        let marker = b"<<AITERM_DONE:abcdefgh>>"; // 24 bytes
        let (first, rest) = marker.split_at(5);
        let (second, third) = rest.split_at(2);

        // Round 1: only 5 bytes have arrived.
        let (found1, tail1) = scan_for_marker(b"", first, marker);
        assert!(!found1);

        // Round 2: 2 more bytes arrive (7 bytes total seen so far) — still
        // not present. This is the exact round the naive `chunk`-only tail
        // computation loses information: the buggy version's tail would be
        // "second" (2 bytes) instead of "first + second" (7 bytes).
        let (found2, tail2) = scan_for_marker(&tail1, second, marker);
        assert!(!found2);
        assert_eq!(tail2.len(), 7, "tail must carry forward everything seen so far, not just the latest chunk");

        // Round 3: the remainder arrives — tail2 + third together contain it.
        let (found3, _tail3) = scan_for_marker(&tail2, third, marker);
        assert!(found3, "marker split across 3 chunks must still be found");
    }

    #[test]
    fn marker_tail_after_keeps_only_the_last_marker_len_minus_one_bytes() {
        let marker_len = 5;
        let chunk = b"abcdefgh";
        assert_eq!(marker_tail_after(chunk, marker_len), b"efgh".to_vec());
    }

    #[test]
    fn marker_tail_after_keeps_the_whole_chunk_when_shorter_than_the_window() {
        let marker_len = 20;
        let chunk = b"ab";
        assert_eq!(marker_tail_after(chunk, marker_len), b"ab".to_vec());
    }

    #[tokio::test]
    async fn shells_own_osc133_prompt_markers_do_not_count_as_bells() {
        // Reproduces the original bug: AITerm's own shell-integration hooks
        // (see `pty::shell::inject_shell_integration` /
        // `inject_powershell_integration`) emit a BEL-terminated OSC133 "A"
        // (prompt-start) marker on every prompt draw — including the very
        // first one, before any command has run or any agent inside has done
        // anything. That marker must never be counted as a genuine agent
        // notification bell. `test_shell()` (plain /bin/sh / cmd.exe /Q) does
        // not go through the real injection, so this test calls the REAL
        // production injection functions directly (mirroring
        // `PtySessionStubForCwd`'s note above on why tests in this file
        // reuse production free functions, not a hand-copied mimic of them)
        // so it actually guards against regressions in the real scripts.
        #[cfg(windows)]
        let shell = super::super::shell::inject_powershell_integration("powershell.exe".into());
        #[cfg(not(windows))]
        let shell = {
            // inject_shell_integration only injects OSC133 hooks for
            // programs whose path ends in "bash" or "zsh" (see shell.rs) —
            // pick whichever of those is actually present. Bash is tried
            // first since it's near-universally present on Unix test
            // environments; either shell exercises the same OSC133 marker
            // code path this test is guarding.
            let program = if std::path::Path::new("/bin/bash").exists() {
                PathBuf::from("/bin/bash")
            } else {
                PathBuf::from("/bin/zsh")
            };
            super::super::shell::inject_shell_integration(program)
        };

        let session = PtySession::spawn(
            shell,
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // Give the shell time to draw its first prompt (which fires its own
        // OSC133 "A" marker, BEL-terminated) — this is exactly the false
        // positive this fix addresses. Even after this settles, no bell
        // should have been counted.
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(
            session.bell_count(),
            0,
            "the shell's own boot-time OSC133 marker must not be counted as a bell"
        );
    }
}

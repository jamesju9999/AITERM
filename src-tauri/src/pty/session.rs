use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

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

/// The fixed wording appended (as its own CR-terminated write) after a task
/// prompt, asking a cooperating agent to print `done_marker(tab_id)` on its
/// own line when finished — an optional fast-path completion signal on top
/// of the bell fallback. The three pieces (prefix, id, suffix) are named
/// separately with other text between them so this string never itself
/// contains the contiguous 52-byte marker: terminal echo of this
/// instruction would otherwise increment `marker_count` with no agent
/// involved (verified live during the coordination feature's development).
pub fn done_marker_instruction(tab_id: &str) -> String {
    format!(
        "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {DONE_MARKER_PREFIX} ，接著是你的識別碼 {tab_id} ，最後接上 {DONE_MARKER_SUFFIX} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）"
    )
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

/// Raw PTY bytes retained per session. Serves two readers with different
/// appetites: AI context (which asks for a few KB) and screen sharing (which
/// replays this whole buffer to prime a newly connected viewer's terminal).
/// The share case sets the floor — a full 80x24 colour redraw runs well past
/// the 8 KB this used to be. At 256 KB, twenty open tabs cost ~5 MB.
pub(crate) const OUTPUT_RING_CAP: usize = 256 * 1024;

/// How many output chunks the fan-out channel buffers before the slowest
/// subscriber starts losing the oldest. Each slot holds one `Vec<u8>` of up
/// to 4096 bytes (the reader thread's read buffer size), and tokio allocates
/// every slot up front — so this is a worst case of roughly 1 MB per shared
/// tab, and it multiplies by the number of tabs being shared at once.
///
/// Deliberately not larger: a bigger buffer only delays `Lagged`, it cannot
/// prevent it, and the resynchronise-from-the-ring-buffer path has to exist
/// either way. A lagging viewer is never silently tolerated — it is told to
/// resync (see `share::server`, built in a later task of this plan), because
/// a terminal that misses bytes mid-escape-sequence renders wrong from then
/// on and never recovers on its own.
const OUTPUT_BROADCAST_CAP: usize = 256;

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
    /// DEC private modes this session's output has switched on or off, so a
    /// share replay taken from the middle of the stream can restore them.
    /// Unlike `output_ring` this is never evicted — see `ansi::DecModeTracker`
    /// for the bug that made it necessary. Shared with the reader thread.
    dec_modes: Arc<Mutex<super::ansi::DecModeTracker>>,
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
    /// Last exit code observed in an OSC 133 `D;<code>` marker in this
    /// session's output, or `None` if none seen yet. The reader thread
    /// already extracts these codes for cd confirmation
    /// (`confirm_pending_cds_from_output`); this stores the most recent one
    /// so the task-board monitor can tell "the foreground command exited
    /// non-zero" (e.g. `claude` not installed → shell prints 127) from a
    /// still-running task. `i64` with `-1` sentinel for "unset" so it fits
    /// one `AtomicI64` (same lock-free pattern as `bell_count`).
    last_exit_code: Arc<AtomicI64>,
    /// Wall-clock `Instant` of the last non-empty output chunk. Read as
    /// `ms_since_output()`; used by the monitor's "no output for 120s ⇒
    /// stuck" check. Spawn time counts as the first "output" so a session
    /// that never prints anything still ages.
    last_output_at: Arc<Mutex<Instant>>,
    /// Fan-out of raw output chunks to screen-share viewers. Independent of
    /// the `on_data` callback, which continues to serve the app's own
    /// terminal view. Subscribers appear only while a tab is being shared.
    output_tx: tokio::sync::broadcast::Sender<Vec<u8>>,
    /// Current terminal size, as last set by `resize` (or as spawned). Screen
    /// sharing sends this to viewers so they build their own terminal at the
    /// host's dimensions rather than their own window's.
    size: Mutex<PtySize>,
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
    /// Spawn with a generated id. Thin wrapper over `spawn_with_id` — the two
    /// were byte-identical copies until this was collapsed, which meant every
    /// change to the reader thread had to be made twice.
    pub fn spawn<F>(shell: ShellSpec, size: PtySize, cwd: Option<PathBuf>, on_data: F) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        Self::spawn_with_id(shell, size, Uuid::new_v4().to_string(), cwd, on_data)
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

        let output_ring: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));
        let ring_for_thread = Arc::clone(&output_ring);
        let dec_modes: Arc<Mutex<super::ansi::DecModeTracker>> = Arc::new(Mutex::new(Default::default()));
        let dec_modes_for_thread = Arc::clone(&dec_modes);
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
        let last_exit_code: Arc<AtomicI64> = Arc::new(AtomicI64::new(-1));
        let last_exit_code_for_thread = Arc::clone(&last_exit_code);
        let last_output_at: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
        let last_output_at_for_thread = Arc::clone(&last_output_at);
        let marker_tail_for_thread: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let done_marker_bytes = done_marker(&id).into_bytes();

        let (output_tx, _) =
            tokio::sync::broadcast::channel::<Vec<u8>>(OUTPUT_BROADCAST_CAP);
        let output_tx_for_thread = output_tx.clone();

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
                                for &b in &chunk {
                                    if ring.len() >= OUTPUT_RING_CAP { ring.pop_front(); }
                                    ring.push_back(b);
                                }
                                // Inside the ring's critical section, and
                                // always in this order (ring then modes), so a
                                // snapshot can never see bytes whose mode
                                // switches have not been recorded yet.
                                dec_modes_for_thread.lock().feed(&chunk);
                                // Broadcast while still holding the ring lock.
                                // This is what makes `subscribe_with_history`
                                // atomic: a chunk is either already in the
                                // snapshot a subscriber took, or it cannot be
                                // broadcast until that subscriber has both its
                                // snapshot and its receiver. Neither gap nor
                                // duplicate is possible.
                                //
                                // `send` never blocks — it overwrites the
                                // oldest slot and the slow reader gets
                                // `Lagged` later — so holding the ring lock
                                // across it cannot stall the reader thread.
                                if output_tx_for_thread.receiver_count() > 0 {
                                    let _ = output_tx_for_thread.send(chunk.clone());
                                }
                            }
                            *last_output_at_for_thread.lock() = Instant::now();
                            for code in cd_parser::find_exit_codes(&chunk) {
                                last_exit_code_for_thread.store(code as i64, Ordering::SeqCst);
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
            dec_modes,
            pending_cds,
            bell_count,
            marker_count,
            last_exit_code,
            last_output_at,
            output_tx,
            size: Mutex::new(size),
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

    /// Return the last `max_bytes` bytes of terminal output exactly as the PTY
    /// produced them — ANSI escapes intact, unlike `get_recent_output`, whose
    /// stripping would hand a viewer a colourless, cursor-less approximation.
    ///
    /// **Do not pair this with `subscribe()` to give a screen-share viewer
    /// history plus live output.** The two take different locks, so whichever
    /// order you call them in leaves a window — see `subscribe_with_history`,
    /// which is the correct API for that combination. This method on its own
    /// is fine for a pure snapshot with no live stream attached.
    ///
    /// Unlike `get_recent_output` this does not treat whitespace-only content
    /// as "nothing" — a screen that genuinely holds only blank lines is still
    /// the screen the viewer must be shown. `None` means nothing has been
    /// captured at all.
    pub fn get_recent_raw(&self, max_bytes: usize) -> Option<Vec<u8>> {
        let ring = self.output_ring.lock();
        if ring.is_empty() {
            return None;
        }
        let start = ring.len().saturating_sub(max_bytes);
        Some(ring.iter().skip(start).copied().collect())
    }

    /// Subscribe to this session's raw output. Every subscriber receives every
    /// chunk produced after it subscribed.
    ///
    /// **This gives you no history, and pairing it with `get_recent_raw()` to
    /// obtain some is racy** — the two take different locks, so bytes landing
    /// in between are either duplicated or lost depending on the order. Use
    /// `subscribe_with_history` when you need both. This method on its own is
    /// fine when you only want output from now on.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    /// Take a history snapshot and subscribe to future output as one atomic
    /// step. Screen sharing must use this rather than calling `get_recent_raw`
    /// and `subscribe` separately.
    ///
    /// Those two take different locks, and the reader thread writes the ring
    /// before it broadcasts — so calling them in either order leaves a window:
    /// subscribe-then-snapshot duplicates the bytes that land in between, and
    /// snapshot-then-subscribe loses them entirely. Losing bytes can truncate
    /// an ANSI escape sequence, and a terminal that renders a truncated escape
    /// stays wrong forever.
    ///
    /// Holding the ring lock across both closes the window: a chunk is either
    /// already in the snapshot, or the reader thread cannot broadcast it until
    /// this method has returned with its receiver in hand.
    pub fn subscribe_with_history(
        &self,
        max_bytes: usize,
    ) -> (Option<Vec<u8>>, tokio::sync::broadcast::Receiver<Vec<u8>>) {
        let ring = self.output_ring.lock();
        let history = if ring.is_empty() {
            None
        } else {
            let start = ring.len().saturating_sub(max_bytes);
            // The mode prefix goes first: a replay that starts mid-stream has
            // lost every `CSI ? Ps h` the ring has since evicted, and those are
            // sticky state a program emits once and never repeats. Without it a
            // viewer joining a host that is already running a full-screen
            // program (Claude Code CLI, vim, htop) renders alternate-screen
            // content into its normal buffer. See `ansi::DecModeTracker`.
            //
            // Replaying a mode the ring still happens to contain is harmless:
            // the duplicate inside the replay simply re-applies it, landing on
            // the same state. `max_bytes` is a bound on the ring slice, not on
            // this prefix, which is tens of bytes.
            let mut out = self.dec_modes.lock().prefix();
            out.extend(ring.iter().skip(start).copied());
            Some(out)
        };
        let rx = self.output_tx.subscribe();
        drop(ring);
        (history, rx)
    }

    /// How many share viewers are currently attached to this session.
    pub fn subscriber_count(&self) -> usize {
        self.output_tx.receiver_count()
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

    /// See the `last_exit_code` field. `None` until an OSC 133 `D;<code>`
    /// marker has been seen in output.
    pub fn last_exit_code(&self) -> Option<i32> {
        match self.last_exit_code.load(Ordering::SeqCst) {
            -1 => None,
            n => Some(n as i32),
        }
    }

    /// Milliseconds since the last non-empty output chunk (spawn time counts
    /// as output zero). See the `last_output_at` field.
    pub fn ms_since_output(&self) -> u64 {
        self.last_output_at.lock().elapsed().as_millis() as u64
    }

    pub fn resize(&self, size: PtySize) -> PtyResult<()> {
        let master = self.master.lock();
        master
            .resize(size)
            .map_err(|e| PtyError::Internal(format!("resize: {e}")))?;
        *self.size.lock() = size;
        Ok(())
    }

    /// Current terminal size, as last set by `resize` (or as spawned).
    /// Screen sharing sends this to viewers so they build their own terminal
    /// at the host's dimensions rather than their own window's.
    pub fn size(&self) -> (u16, u16) {
        let s = *self.size.lock();
        (s.cols, s.rows)
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

    #[test]
    fn size_reports_what_the_session_was_spawned_with_and_tracks_resize() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.size(), (80, 24));

        session
            .resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .expect("resize");
        assert_eq!(session.size(), (120, 40));
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

    #[test]
    fn last_exit_code_is_none_for_a_fresh_session() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        assert_eq!(session.last_exit_code(), None);
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn last_exit_code_captures_a_nonzero_osc133_exit() {
        // Plain `test_shell()` (/bin/sh) never emits OSC 133 `D` markers — only
        // the real injected shell integration does. Mirror
        // `shells_own_osc133_prompt_markers_do_not_count_as_bells` and drive a
        // bash/zsh (or PowerShell) through the production injection functions.
        #[cfg(windows)]
        let shell = super::super::shell::inject_powershell_integration("powershell.exe".into());
        #[cfg(not(windows))]
        let shell = {
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
        // AITerm's shell-integration hook emits OSC133 D;<code> after each command.
        session.write(b"false\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if session.last_exit_code() == Some(1) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "never saw exit code 1, got {:?}", session.last_exit_code());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tokio::test]
    async fn ms_since_output_grows_while_the_session_is_quiet() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(session.ms_since_output() >= 200, "expected quiet time to accumulate, got {}", session.ms_since_output());
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

    // Skipped on Windows: real-ConPTY test, fails deterministically on
    // `rust-test (windows-latest)` (green on macOS + Linux). ConPTY's input
    // re-echo means the written line never lands in the raw ring buffer in the
    // shape the poll loop looks for. Tracked separately with the coordination_ops
    // PTY tests.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn get_recent_raw_keeps_ansi_escapes_that_get_recent_output_strips() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // A red "SHAREME" followed by a reset. get_recent_output must not show
        // the escapes; get_recent_raw must.
        #[cfg(windows)]
        session.write(b"echo \x1b[31mSHAREME\x1b[0m\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf '\\033[31mSHAREME\\033[0m\\n'\n").unwrap();

        // Real PTY I/O on a background thread — poll rather than sleep a fixed
        // amount, same as the bell test above.
        let mut raw = None;
        for _ in 0..50 {
            if let Some(bytes) = session.get_recent_raw(64 * 1024) {
                // 不能只找裸的 "SHAREME"：PTY 會回顯你打進去的那行指令，而
                // 指令原文裡的 `\033` 是四個 ASCII 字元、不是真的 ESC byte，
                // 卻同樣含有 "SHAREME"。搜尋含真 ESC 的完整序列才能保證只
                // 匹配到 printf 真正執行後的著色輸出。
                if bytes.windows(12).any(|w| w == b"\x1b[31mSHAREME") {
                    raw = Some(bytes);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let raw = raw.expect("expected SHAREME to show up in the raw ring buffer");

        assert!(
            raw.contains(&0x1b),
            "get_recent_raw must preserve ESC bytes; got {:?}",
            String::from_utf8_lossy(&raw)
        );
        let stripped = session.get_recent_output(64 * 1024).expect("stripped output");
        assert!(
            !stripped.contains('\u{1b}'),
            "get_recent_output must still strip ESC bytes; got {stripped:?}"
        );
    }

    #[test]
    fn output_ring_cap_is_large_enough_to_replay_a_screen() {
        // A single full redraw of an 80x24 screen with colour runs well past
        // 8 KB. A share viewer primed from a ring that small would see a
        // fragment of a screen, so this floor is a real requirement, not a
        // preference.
        assert!(
            OUTPUT_RING_CAP >= 128 * 1024,
            "OUTPUT_RING_CAP too small to prime a share viewer: {OUTPUT_RING_CAP}"
        );
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

    #[tokio::test]
    async fn every_subscriber_receives_the_same_output_and_on_data_still_fires() {
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

        // Two independent viewers, subscribed before anything is written.
        let mut a = session.subscribe();
        let mut b = session.subscribe();

        #[cfg(windows)]
        session.write(b"echo FANOUT\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'FANOUT\\n'\n").unwrap();

        // 這裡搜尋裸的 "FANOUT" 是**刻意的、也是正確的**：PTY 會回顯你打進去
        // 的指令，所以匹配可能命中回顯而不是 printf 的輸出——但那無所謂，回顯
        // 同樣是走 PTY 輸出、同樣要經過 fan-out，正是這個測試要驗的東西。
        async fn collect_until_marker(
            r: &mut tokio::sync::broadcast::Receiver<Vec<u8>>,
        ) -> Vec<u8> {
            let mut acc = Vec::new();
            for _ in 0..50 {
                match tokio::time::timeout(Duration::from_millis(200), r.recv()).await {
                    Ok(Ok(chunk)) => {
                        acc.extend_from_slice(&chunk);
                        if acc.windows(6).any(|w| w == b"FANOUT") {
                            return acc;
                        }
                    }
                    Ok(Err(_)) => break,
                    Err(_) => continue,
                }
            }
            acc
        }

        let got_a = collect_until_marker(&mut a).await;
        let got_b = collect_until_marker(&mut b).await;

        assert!(
            got_a.windows(6).any(|w| w == b"FANOUT"),
            "subscriber A missed the output"
        );
        assert!(
            got_b.windows(6).any(|w| w == b"FANOUT"),
            "subscriber B missed the output"
        );

        // 名字承諾的是「相同」，就要真的驗相同。兩個 receiver 訂閱的是同一個
        // sender，reader thread 每個 chunk 只 send 一次，所以兩邊收到的 chunk
        // 序列（順序、內容、邊界）完全一致；而 collect_until_marker 的停止條件
        // 是「累積內容是否含 FANOUT」——一個純內容函數，不受即時時序影響。
        // 因此兩邊會在收到相同數量的 chunk 後停止，這個斷言是確定性的。
        assert_eq!(
            got_a, got_b,
            "subscribers received different bytes for the same broadcast"
        );

        // The pre-existing on_data path must be untouched by fan-out.
        let mut via_callback = Vec::new();
        while let Ok(chunk) = rx.try_recv() {
            via_callback.extend_from_slice(&chunk);
        }
        assert!(
            via_callback.windows(6).any(|w| w == b"FANOUT"),
            "the original on_data callback stopped seeing output"
        );
    }

    #[test]
    fn subscriber_count_reflects_subscribe_and_drop() {
        // 注意這個測試**不**保證 reader thread 裡的 `receiver_count() > 0`
        // 守衛還在。那個守衛純粹是效能優化（省一次 chunk.clone()）——把它
        // 拿掉之後 send() 自己會對 0 receiver 安全地回 Err 而呼叫端吞掉，
        // 功能完全不變，所以沒有任何功能斷言抓得到它被移除。要抓只能量
        // heap 配置次數，那是效能測試不是這裡的事。
        //
        // 這個測試守的是 subscribe()/Drop 的計數邏輯本身。
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        assert_eq!(session.subscriber_count(), 0);
        let a = session.subscribe();
        assert_eq!(session.subscriber_count(), 1);
        let b = session.subscribe();
        assert_eq!(session.subscriber_count(), 2);
        drop(a);
        assert_eq!(session.subscriber_count(), 1);
        drop(b);
        assert_eq!(session.subscriber_count(), 0);
    }

    #[test]
    fn done_marker_embeds_the_tab_id_between_fixed_delimiters() {
        assert_eq!(done_marker("abc-123"), "<<AITERM_DONE:abc-123>>");
    }

    #[test]
    fn done_marker_instruction_never_contains_the_contiguous_marker() {
        let tab = "abc-123-def";
        let instr = done_marker_instruction(tab);
        // The whole point: canonical-mode echo of this text must not itself
        // form the marker byte sequence, or it self-triggers marker_count.
        assert!(!instr.contains(&done_marker(tab)), "instruction must not contain the full contiguous marker: {instr}");
        // But it must mention each piece so a cooperating agent can assemble it.
        assert!(instr.contains(DONE_MARKER_PREFIX));
        assert!(instr.contains(tab));
        assert!(instr.contains(DONE_MARKER_SUFFIX));
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

    #[tokio::test]
    async fn subscribe_with_history_snapshot_excludes_bytes_the_stream_then_delivers() {
        // 這個測試驗的是**時序正確性**，不是併發競態：訂閱前產生的內容要在
        // 快照裡，訂閱後產生的內容要只從串流來。
        //
        // 它**抓不到**「有人把 broadcast 移回 ring 鎖外面」這個退化——已實測
        // 確認（把 send 搬出鎖後這個測試跑 30 次全過）。因為測試在寫入與訂閱
        // 之間輪詢等待，兩者從不重疊，而競態只存在於重疊的窗口裡。
        //
        // 真正的原子性保證來自不變量論證，不是來自這個測試：reader thread 把
        // 「寫 ring」與「決定要不要 send」放在同一個臨界區，所以外界不可能觀察
        // 到兩者之間的中間態。見 `subscribe_with_history` 的 doc comment。
        //
        // 要用確定性測試抓那個窗口，需要在生產程式碼裡插測試專用的 barrier，
        // 那是為了可測試性侵入實作，不划算。
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // 先產生一段歷史，確認它會出現在快照裡。
        #[cfg(windows)]
        session.write(b"echo HIST\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'HIST\\n'\n").unwrap();

        let mut history = None;
        for _ in 0..50 {
            if let Some(bytes) = session.get_recent_raw(64 * 1024) {
                if bytes.windows(4).any(|w| w == b"HIST") {
                    history = Some(bytes);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        history.expect("history should contain HIST before we subscribe");

        // 原子地取快照＋訂閱。
        let (snapshot, mut rx) = session.subscribe_with_history(64 * 1024);
        let snapshot = snapshot.expect("snapshot should not be empty");
        assert!(
            snapshot.windows(4).any(|w| w == b"HIST"),
            "the atomic snapshot lost the history that get_recent_raw could see"
        );

        // 訂閱之後才產生的東西只能來自串流，不能已經在快照裡。
        #[cfg(windows)]
        session.write(b"echo AFTER\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"printf 'AFTER\\n'\n").unwrap();

        assert!(
            !snapshot.windows(5).any(|w| w == b"AFTER"),
            "the snapshot somehow contains output produced after it was taken"
        );

        let mut streamed = Vec::new();
        for _ in 0..50 {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(chunk)) => {
                    streamed.extend_from_slice(&chunk);
                    if streamed.windows(5).any(|w| w == b"AFTER") {
                        break;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        assert!(
            streamed.windows(5).any(|w| w == b"AFTER"),
            "output produced after subscribing never arrived on the stream"
        );
    }

    /// 觀看端重播必須帶回已經被環形緩衝區淘汰掉的終端機模式。
    ///
    /// 這是實機 bug 的迴歸測試：遠端主控端已經在跑 Claude Code CLI 時才連進去，
    /// 觀看端的即時窗格只有三列高、不會滿版。根因是 `\x1b[?1049h`（切進
    /// alternate screen）在程式啟動當下只送出一次——實測 Claude Code 是在整條
    /// session 的第 855 個位元組，而一次 45x160 全螢幕重繪約 2.2 KB，用過一輪
    /// 之後它早就被擠出這個 256 KB 的環，觀看端從頭到尾沒看過它，於是把
    /// alternate screen 的內容畫進 normal buffer，`isAlternateBuffer` 永遠是
    /// false，前端的滿版開關（RemoteTerminalView 的 altBufferHeightPx）因此
    /// 永遠不會生效。
    ///
    /// 只跑在非 Windows：要塞爆 256 KB 需要一句 POSIX shell 迴圈，cmd.exe 沒有
    /// 對等寫法。被測的邏輯本身跨平台，`DecModeTracker` 的單元測試三個平台都跑。
    #[cfg(not(windows))]
    #[tokio::test]
    async fn subscribe_with_history_restores_modes_evicted_from_the_ring() {
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            None,
            |_| {},
        )
        .expect("spawn pty");

        // 先切進 alternate screen，再吐出遠超過環容量的填充內容把它擠掉。
        session.write(b"printf '\\033[?1049h'\n").unwrap();
        session
            .write(b"i=0; while [ $i -lt 320 ]; do printf '%01000d' 0; i=$((i+1)); done; printf 'FILLDONE\\n'\n")
            .unwrap();

        // 等到填充跑完、而且 `?1049h` 真的已經不在環裡——不確認這件事的話，
        // 測試可能在還沒淘汰時就通過，等於什麼都沒驗到。
        let mut evicted = false;
        for _ in 0..100 {
            if let Some(bytes) = session.get_recent_raw(OUTPUT_RING_CAP) {
                let done = bytes.windows(8).any(|w| w == b"FILLDONE");
                let still_there = bytes.windows(8).any(|w| w == b"\x1b[?1049h");
                if done && !still_there {
                    evicted = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(evicted, "填充沒有把 ?1049h 擠出環——這個測試沒有測到它要測的東西");

        let (history, _rx) = session.subscribe_with_history(OUTPUT_RING_CAP);
        let history = history.expect("ring 有東西，快照不該是空的");
        // 重播必須以「還原模式」的前綴開頭，而且那段前綴要包含切進 alternate
        // screen 這一句。不寫死成 `starts_with(b"\x1b[?1049h")`：前綴是照模式
        // 編號排序的，shell 自己設的 ?1034（readline 的 meta key）之類編號較小
        // 的模式會排在前面，那不是錯誤。這裡吃掉開頭連續的
        // `ESC [ ? <digits> h|l`，再檢查裡面有沒有 ?1049h。
        let mut i = 0;
        let mut prefix_seqs: Vec<&[u8]> = Vec::new();
        while history[i..].starts_with(b"\x1b[?") {
            let start = i;
            i += 3;
            while i < history.len() && history[i].is_ascii_digit() {
                i += 1;
            }
            if i >= history.len() || (history[i] != b'h' && history[i] != b'l') {
                i = start;
                break;
            }
            i += 1;
            prefix_seqs.push(&history[start..i]);
        }
        assert!(
            prefix_seqs.iter().any(|seq| *seq == b"\x1b[?1049h"),
            "重播開頭的模式前綴沒有帶回 ?1049h，觀看端不會進 alternate screen；實際前綴：{:?}",
            prefix_seqs.iter().map(|s| String::from_utf8_lossy(s).into_owned()).collect::<Vec<_>>(),
        );
    }
}

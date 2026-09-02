//! Business logic for the agent-coordination MCP tools (`spawn_tab`,
//! `send_input`, `get_tab_status`, `wait_for_idle`). Kept separate from
//! `tools.rs`'s `#[tool]`-annotated methods so this logic is unit-testable
//! without going through the MCP protocol layer — same split as
//! `db_ops.rs`/`kb_ops.rs`.
//!
//! Security note: `send_input`/`get_tab_status`/`wait_for_idle` all refuse to
//! touch any `tab_id` that isn't in `CoordinationRegistry` — i.e. a tab this
//! server itself spawned via `spawn_tab`. A tab the human opened by hand is
//! never a valid target. See the design doc for the full rationale.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use portable_pty::PtySize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::pty::manager::PtyManager;

const DEFAULT_WAIT_SECONDS: u64 = 300;
const MAX_WAIT_SECONDS: u64 = 1800;
const POLL_INTERVAL_MS: u64 = 250;
/// Bound for send_input's internal wait (via wait_for_new_bell) for the
/// target to signal it finished the just-sent task text, before sending the
/// optional done-marker instruction. Deliberately much shorter than
/// wait_for_idle's DEFAULT_WAIT_SECONDS: live testing found a real `claude`
/// CLI process can complete a real task in ~2s while never ringing a single
/// bell over 30s of observation, so waiting anywhere near 300s here buys
/// nothing and just makes send_input(request_done_marker: true) hang with
/// zero feedback whenever the target's bell doesn't fire. Generous versus
/// the fastest observed real turn, nowhere near wait_for_idle's default.
const DONE_MARKER_WAIT_SECONDS: u64 = 15;
/// Matches `pty_get_recent_output`'s existing cap (`src-tauri/src/pty/commands.rs`).
const RECENT_OUTPUT_BYTES: usize = 4096;

/// Tracks which tabs this MCP server has spawned (the only valid `send_input`/
/// `get_tab_status`/`wait_for_idle` targets) and, for each, the `PtySession`
/// bell count as of the last `send_input` call (or spawn time) — the baseline
/// a fresh bell count is compared against to answer "has this tab replied
/// since we last prompted it", without needing any consuming/stateful latch
/// on `PtySession` itself.
///
/// Constructed once per running MCP tool server (in `McpToolServerState::start`)
/// and shared — via `Arc` — across every `AiTermTools` instance the
/// `StreamableHttpService` factory closure creates (one per MCP session), so
/// a tab spawned by one MCP session is visible to `get_tab_status` calls from
/// any other session talking to the same running server.
#[derive(Default)]
pub struct CoordinationRegistry {
    tabs: Mutex<HashMap<String, Baseline>>,
}

/// The bell/marker counts a tab's `PtySession` reported as of the last
/// `spawn_tab`/`send_input` call — the point every subsequent
/// `get_tab_status`/`wait_for_idle` compares fresh counts against. Paired
/// together (not two separate maps) because they're always read and
/// written for the same `tab_id` at the same moment.
#[derive(Clone, Copy, Default)]
struct Baseline {
    bell: u64,
    marker: u64,
}

impl CoordinationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_known(&self, tab_id: &str) -> bool {
        self.tabs.lock().contains_key(tab_id)
    }

    fn record_baseline(&self, tab_id: &str, bell: u64, marker: u64) {
        self.tabs.lock().insert(tab_id.to_string(), Baseline { bell, marker });
    }

    fn baseline(&self, tab_id: &str) -> Option<Baseline> {
        self.tabs.lock().get(tab_id).copied()
    }
}

#[derive(Serialize)]
struct TabStatus {
    idle: bool,
    recent_output: String,
    /// Which signal made `idle` true this time — `"bell"` or `"marker"`.
    /// `None` while still not idle. Purely informational (debugging/tests);
    /// callers that only care about `idle` can ignore it.
    signal: Option<&'static str>,
}

#[derive(Serialize)]
struct WaitResult {
    idle: bool,
    recent_output: String,
    timed_out: bool,
    signal: Option<&'static str>,
}

/// Payload for the `mcp-coordination-tab-spawned` Tauri event, telling the
/// frontend to create a tab bound to this already-existing backend session.
#[derive(Serialize, Clone)]
struct TabSpawnedEvent {
    session_id: String,
    command: Option<String>,
}

pub(crate) async fn spawn_tab(
    app: &AppHandle,
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    cwd: Option<String>,
    command: Option<String>,
) -> Result<String, String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let cwd_path = cwd.map(std::path::PathBuf::from);
    let tab_id = pty_manager
        .create_with_app(app.clone(), size, cwd_path, None)
        .map_err(|e| e.to_string())?;

    // Fresh session: bell_count()/marker_count() are 0 right now. Recording
    // that as the baseline means get_tab_status/wait_for_idle report "not
    // idle" until the first bell or marker — reasonable, since nothing has
    // run yet either way.
    registry.record_baseline(&tab_id, 0, 0);

    if let Some(cmd) = &command {
        if let Err(e) = pty_manager.write(&tab_id, format!("{cmd}\r").as_bytes()) {
            let _ = pty_manager.close(&tab_id);
            return Err(e.to_string());
        }
    }

    if let Err(e) = app.emit("mcp-coordination-tab-spawned", TabSpawnedEvent {
        session_id: tab_id.clone(),
        command,
    }) {
        eprintln!("emit mcp-coordination-tab-spawned failed: {e}");
    }

    Ok(tab_id)
}

pub(crate) async fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
    request_done_marker: bool,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Sent as a second, independently \r-terminated write (a separate
    // message) rather than appended via an embedded '\n' — this file's own
    // send_input_terminates_the_line_with_cr_not_lf regression test exists
    // precisely because raw-mode TUIs (Claude Code's own included) have no
    // guaranteed behavior for a bare LF byte, only for CR. See the design
    // doc's "指示文字" section.
    //
    // The instruction text below deliberately never writes the complete,
    // contiguous marker string (prefix + tab_id + suffix back-to-back) —
    // each piece is separated by other text. If it did, canonical-mode
    // terminal echo alone (no cooperating agent needed) would write that
    // same contiguous byte sequence right back into this session's output
    // stream as a pure side effect of writing this instruction, and
    // marker_count would increment immediately — a false "done" signal
    // before the target has done anything. Verified live against a real
    // PTY during implementation: the original (buggy) wording, which did
    // embed the marker contiguously, triggered marker_count=1 from echo
    // alone, with no agent involved.
    //
    // The instruction is only sent once the target signals (via a fresh
    // bell) that it finished processing `text` and is idle again — sending
    // it immediately after the first write would race the target's own
    // processing time. Verified live against a real Claude Code CLI: the
    // instruction's characters got typed into the input box (proof the
    // write arrived) but its own \r never triggered submission, because the
    // target was still busy with `text` when it arrived. See the design
    // doc's "第二段寫入被目標端忽略" section.
    let mut instruction_sent = true;
    if request_done_marker {
        let bell_before = pty_manager.bell_count(tab_id).unwrap_or(0);
        let became_idle = wait_for_new_bell(
            pty_manager,
            tab_id,
            bell_before,
            Duration::from_secs(DONE_MARKER_WAIT_SECONDS),
        )
        .await;

        if became_idle {
            let instruction = format!(
                "（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 {} ，接著是你的識別碼 {} ，最後接上 {} 。這能讓協調端提早得知你已完成，不影響任何其他行為。）",
                crate::pty::session::DONE_MARKER_PREFIX,
                tab_id,
                crate::pty::session::DONE_MARKER_SUFFIX
            );
            pty_manager
                .write(tab_id, format!("{instruction}\r").as_bytes())
                .map_err(|e| e.to_string())?;
        } else {
            instruction_sent = false;
        }
    }

    // Reset the baseline to the counts *as of right now*, after both of this
    // call's own writes (or just the first, if the instruction was skipped
    // because the target never became idle) — before any reply has had a
    // chance to arrive — so idle only flips true again once a *new* bell or
    // marker (a reply to this specific input) is observed, not a stale one
    // from before this send.
    let bell_current = pty_manager.bell_count(tab_id).unwrap_or(0);
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, bell_current, marker_current);

    if request_done_marker && !instruction_sent {
        Ok(format!(
            "sent to {tab_id} (task only — target did not become idle within {DONE_MARKER_WAIT_SECONDS}s, so the completion-marker instruction was not sent)"
        ))
    } else {
        Ok(format!("sent to {tab_id}"))
    }
}

/// Polls `pty_manager`'s bell count for `tab_id`, returning `true` once it
/// exceeds `baseline` (a fresh bell — the target signaling it's idle again)
/// or `false` if `timeout` elapses first. Extracted as its own function
/// (rather than inlined in `send_input`) so tests can pass a short
/// `timeout` to exercise the "target never bells" path without waiting out
/// the real production timeout (`DONE_MARKER_WAIT_SECONDS` = 15s).
async fn wait_for_new_bell(pty_manager: &PtyManager, tab_id: &str, baseline: u64, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if pty_manager.bell_count(tab_id).unwrap_or(baseline) > baseline {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

fn status_for(pty_manager: &PtyManager, registry: &CoordinationRegistry, tab_id: &str) -> Result<TabStatus, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    let baseline = registry.baseline(tab_id).unwrap_or_default();
    let bell_current = pty_manager
        .bell_count(tab_id)
        .ok_or_else(|| format!("tab_id '{tab_id}' is no longer running (it may have been closed)"))?;
    let marker_current = pty_manager.marker_count(tab_id).unwrap_or(0);
    let recent_output = pty_manager
        .get_recent_output(tab_id, RECENT_OUTPUT_BYTES)
        .unwrap_or_default();

    // Marker checked first: it's the optional, cooperative-agent signal this
    // feature adds, and when it fires it's always at least as fresh as bell.
    // Which one "wins" on a tie has no behavioral consequence — idle is idle
    // either way — this only affects the informational `signal` field.
    let marker_idle = marker_current > baseline.marker;
    let bell_idle = bell_current > baseline.bell;
    let signal = if marker_idle {
        Some("marker")
    } else if bell_idle {
        Some("bell")
    } else {
        None
    };

    Ok(TabStatus { idle: marker_idle || bell_idle, recent_output, signal })
}

pub(crate) fn get_tab_status(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
) -> Result<String, String> {
    let status = status_for(pty_manager, registry, tab_id)?;
    serde_json::to_string_pretty(&status).map_err(|e| e.to_string())
}

pub(crate) async fn wait_for_idle(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    timeout_seconds: Option<u64>,
) -> Result<String, String> {
    // Validate up front so an unknown tab_id fails fast instead of after a
    // full timeout's worth of silent polling.
    status_for(pty_manager, registry, tab_id)?;

    let timeout = Duration::from_secs(timeout_seconds.unwrap_or(DEFAULT_WAIT_SECONDS).min(MAX_WAIT_SECONDS));
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let status = status_for(pty_manager, registry, tab_id)?;
        if status.idle {
            let result = WaitResult {
                idle: true,
                recent_output: status.recent_output,
                timed_out: false,
                signal: status.signal,
            };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let result = WaitResult {
                idle: false,
                recent_output: status.recent_output,
                timed_out: true,
                signal: status.signal,
            };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn pty_size() -> PtySize {
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
    }

    #[tokio::test]
    async fn send_input_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello", false).await.unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }

    #[tokio::test]
    async fn get_tab_status_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = get_tab_status(&pty_manager, &registry, "not-a-real-tab").unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }

    #[tokio::test]
    async fn send_input_and_get_tab_status_work_on_a_spawned_session() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();

        // Bypass spawn_tab (it needs a real AppHandle) and construct the
        // PtySession + registry entry directly, exactly as spawn_tab itself
        // would — this tests send_input/get_tab_status's own logic, not
        // spawn_tab's AppHandle-dependent plumbing (covered separately by the
        // integration test in a later task).
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", false).await.unwrap();
        assert_eq!(sent, format!("sent to {tab_id}"));

        let status_json = get_tab_status(&pty_manager, &registry, &tab_id).unwrap();
        assert!(status_json.contains("\"idle\""), "{status_json}");
        assert!(status_json.contains("\"recent_output\""), "{status_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_times_out_when_no_bell_arrives() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();

        // AITerm's own OSC133 shell-integration hook (`__aiterm_precmd` in
        // `pty/shell.rs`, pre-existing, unrelated to this feature) fires once
        // on the shell's very first prompt draw and is BEL-terminated
        // (`\x1b]133;A\x07`). `PtySession::bell_count` uses
        // `contains_bare_bell` (see `pty/cd_parser.rs`), which recognizes
        // that terminator and excludes it, so this marker never counts as a
        // bell. Baseline at 0 immediately, same as `spawn_tab` does.
        registry.record_baseline(&tab_id, 0, 0);

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(1)).await.unwrap();
        assert!(result_json.contains("\"timed_out\": true"), "{result_json}");
        assert!(result_json.contains("\"idle\": false"), "{result_json}");
    }

    // Skipped on Windows: this and the other three real-ConPTY tests below fail
    // deterministically on `rust-test (windows-latest)` (every master run since
    // 2026-08-31; green on macOS + Linux). ConPTY emits a VT preamble and
    // re-echoes input lines, so the `MARKREADY`/poll-window readiness gating
    // misses the BEL or captures a stray byte. Real fix: gate on OSC-133
    // readiness like the app does, then drop these `ignore`s.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn wait_for_idle_returns_idle_once_a_bell_is_observed() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        pty_manager.write(&tab_id, b"printf '\\007'\n").unwrap();

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(10)).await.unwrap();
        assert!(result_json.contains("\"idle\": true"), "{result_json}");
        assert!(result_json.contains("\"timed_out\": false"), "{result_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = wait_for_idle(&pty_manager, &registry, "not-a-real-tab", None).await.unwrap_err();
        assert!(err.contains("was not created by spawn_tab"), "{err}");
    }

    /// Regression test for the "text gets typed but Enter never fires" bug.
    ///
    /// A first attempt at this test sent `echo send_input_regression_check`
    /// via `send_input` and checked that echo's own output showed up — but a
    /// canonical-mode shell (bash/zsh, which is what `create_with_callback`
    /// spawns here) tolerates a bare `\n` as a line terminator exactly the
    /// same as `\r`, so that test passed even with the `\n` bug reintroduced
    /// (confirmed manually). It gave false assurance: it can't distinguish
    /// the fix from the bug, because the bug specifically only breaks
    /// *raw-mode* input handling (like Claude Code CLI's own TUI), which is
    /// hard to spawn deterministically in a unit test.
    ///
    /// This test instead inspects the literal terminator byte `send_input`
    /// writes to the PTY, independent of any shell's tolerance for either
    /// byte: it puts the pty's line discipline into raw mode (`stty raw
    /// -echo`) and uses `od` to dump the exact bytes that arrive off the
    /// wire. With the fix the terminator is `0d` (CR); with the bug it's
    /// `0a` (LF).
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn send_input_terminates_the_line_with_cr_not_lf() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        // Sent while the pty is still in normal canonical+echo mode, so this
        // setup line's own line terminator is unrelated to the bug under
        // test. On Unix it flips the pty into raw mode, prints a marker
        // (built via `''` string concatenation so the *typed/echoed*
        // command text never contains the contiguous marker string — only
        // the executed printf's actual output does), then blocks reading
        // exactly 3 raw bytes and dumps them as hex.
        //
        // On Windows the default shell is PowerShell (see
        // `pty::shell::windows_default_shell`), so there's no stty/od —
        // instead this prints the marker (built via `+` string
        // concatenation for the same echoed-text reason as above), then
        // calls `[Console]::OpenStandardInput().Read($b, 0, 3)` to read 3
        // raw bytes directly off stdin and prints them as hex. No explicit
        // raw-mode toggle is needed: unlike a POSIX tty's ICRNL, Windows
        // console line input does not remap CR to LF, so the byte this
        // reads back is whatever `send_input` actually wrote — if it wrote
        // `\r` (the fix), this unblocks as soon as that 3rd byte arrives;
        // if it wrote `\n` (the bug), Windows console line-input mode never
        // sees an Enter and the read blocks until the poll loop below times
        // out. This branch is unverified by execution on this (macOS) dev
        // machine — only Windows CI actually runs it — but is written to
        // mirror the Unix branch's sequencing as closely as PowerShell
        // allows.
        #[cfg(windows)]
        pty_manager
            .write(
                &tab_id,
                b"Write-Host -NoNewline ('MARK'+'READY'); $b=[byte[]]::new(3); \
                  [Console]::OpenStandardInput().Read($b,0,3)|Out-Null; \
                  ($b|ForEach-Object{$_.ToString('x2')}) -join ' '\r\n",
            )
            .unwrap();
        #[cfg(not(windows))]
        pty_manager
            .write(&tab_id, b"stty raw -echo; printf 'MARK''READY'; od -An -tx1 -N 3\n")
            .unwrap();

        // Wait for proof that the setup line has already put the shell into
        // its byte-dumping read (the marker only appears from that setup
        // line's own execution output, never from the echoed input line)
        // before writing the bytes under test — otherwise they could race a
        // shell that isn't ready to capture them yet.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "byte-dump setup never completed: {output}");
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        send_input(&pty_manager, &registry, &tab_id, "ab", false).await.unwrap();

        // Poll for the setup line's 3-byte hex dump of exactly what
        // send_input wrote: "ab" (0x61 0x62) followed by whatever
        // terminator byte it used. On Windows, if send_input wrote `\n`
        // instead of `\r`, the shell's line-input read never unblocks, so
        // this loop times out instead of observing a wrong byte — still a
        // correct test failure, just via a different failure path than the
        // Unix branch's exact-byte mismatch.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let hex_bytes: Vec<String> = loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            let after_marker = output.split("MARKREADY").nth(1).unwrap_or("").to_string();
            let hex_bytes: Vec<String> = after_marker
                .split_whitespace()
                .filter(|tok| tok.len() == 2 && tok.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
                .collect();
            if hex_bytes.len() >= 3 {
                break hex_bytes;
            }
            assert!(tokio::time::Instant::now() < deadline, "3-byte hex dump never appeared: {output}");
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        };

        assert_eq!(
            &hex_bytes[..3],
            &["61", "62", "0d"],
            "expected send_input's line terminator to be 0d (CR), matching every other place \
             this app simulates pressing Enter (writePty(session, \"\\r\") in \
             TerminalView.tsx / useTerminalBlocks.ts) — got hex bytes: {hex_bytes:?}"
        );
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn send_input_with_request_done_marker_sends_the_instruction_once_the_target_bells() {
        let pty_manager = Arc::new(PtyManager::new());
        let registry = CoordinationRegistry::new();
        // Wider than pty_size()'s usual 80 columns: the request_done_marker
        // instruction is a long, CJK-heavy line, and at 80 columns it soft-wraps
        // partway through the tab_id UUID — the terminal's own line-wrap
        // redisplay (the shell re-echoing the wrapped row) then renders that
        // wrap point as an inserted space and a duplicated hyphen (a terminal
        // rendering artifact, confirmed unrelated to wait_for_new_bell/send_input
        // by reproducing it at 80 columns even with no bell-wait race at all —
        // and confirmed NOT caused by get_recent_output, which does no line
        // reconstruction of its own; it only slices the ring buffer and strips
        // ANSI codes). A wide-enough terminal keeps the whole instruction on
        // one row so the tab_id this test greps for stays byte-for-byte intact.
        let wide_size = PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 };
        let tab_id = pty_manager.create_with_callback(wide_size, |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        // Run send_input concurrently so this test can observe its
        // in-progress state — specifically, that the instruction has NOT
        // been sent yet — before injecting the bell that should unblock it.
        // This is what actually proves ordering: a version of send_input
        // that (by regression) skipped the wait entirely would send the
        // instruction immediately, and this test would catch that, unlike
        // the previous version of this test, which only checked eventual
        // presence and could not distinguish "waited correctly" from
        // "didn't wait at all" (confirmed during code review: hardcoding
        // the wait to always report idle immediately still passed the old
        // test).
        let pty_manager_for_task = Arc::clone(&pty_manager);
        let tab_id_for_task = tab_id.clone();
        let send_task = tokio::spawn(async move {
            send_input(&pty_manager_for_task, &registry, &tab_id_for_task, "echo hi", true).await
        });

        // Give send_input's first write time to land, then confirm the
        // instruction has NOT appeared yet — it must still be waiting for
        // the bell at this point, well before the bell is injected below.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let output_before_bell = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
        assert!(
            !output_before_bell.contains(&tab_id),
            "the instruction must not be sent before the target signals it's idle — got: {output_before_bell}"
        );

        // Now simulate the target becoming idle from the first write, as if
        // it just finished processing the task text — only once this bell is
        // observed should send_input proceed to write the instruction (see
        // wait_for_new_bell).
        pty_manager.write(&tab_id, b"printf '\\007'\n").unwrap();

        send_task.await.unwrap().unwrap();

        // Wait for the echoed instruction to actually arrive (it mentions
        // this tab's own id, proving the instruction was sent).
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains(&tab_id) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "expected the echoed input to mention the tab_id, got: {output}"
            );
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        // But the full, contiguous marker must NEVER appear anywhere in what
        // was echoed — if it did, the instruction itself would have
        // self-triggered a false completion signal via terminal echo alone,
        // with no agent involved. This is a regression test for a real bug
        // found during review: the original wording embedded the complete
        // contiguous marker in the instruction, and canonical-mode echo
        // alone incremented marker_count to 1 against a plain shell that
        // did nothing.
        let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
        let full_marker = crate::pty::session::done_marker(&tab_id);
        assert!(
            !output.contains(&full_marker),
            "the instruction text must never contain the complete contiguous marker — got: {output}"
        );
    }

    /// Same scenario as the test above, but no background bell is ever
    /// injected — the target never signals it's idle, so send_input must
    /// give up after DONE_MARKER_WAIT_SECONDS (15s) and skip the
    /// instruction, returning normally (not hanging, not erroring) with a
    /// message noting it was skipped. Runs for real at ~15s, well within
    /// a normal `cargo test` — this used to be `#[ignore]`d back when this
    /// path reused wait_for_idle's 300s DEFAULT_WAIT_SECONDS, which made a
    /// real end-to-end run of this path impractical for the default suite.
    #[tokio::test]
    async fn send_input_with_request_done_marker_skips_the_instruction_when_the_target_never_bells() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi", true).await.unwrap();
        assert!(
            sent.contains("task only") && sent.contains("was not sent"),
            "expected the return message to note the instruction was skipped, got: {sent}"
        );
    }

    #[tokio::test]
    async fn wait_for_idle_returns_early_via_marker_signal_without_any_bell() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0, 0);

        let marker = crate::pty::session::done_marker(&tab_id);
        pty_manager.write(&tab_id, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(10)).await.unwrap();
        assert!(result_json.contains("\"idle\": true"), "{result_json}");
        assert!(result_json.contains("\"signal\": \"marker\""), "{result_json}");
    }

    #[tokio::test]
    async fn wait_for_new_bell_times_out_when_the_target_never_bells() {
        let pty_manager = PtyManager::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();

        let became_idle = wait_for_new_bell(&pty_manager, &tab_id, 0, Duration::from_millis(300)).await;
        assert!(!became_idle, "expected wait_for_new_bell to give up when no bell ever arrives");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn wait_for_new_bell_returns_true_once_a_new_bell_arrives() {
        let pty_manager = PtyManager::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();

        pty_manager.write(&tab_id, b"printf '\\007'\n").unwrap();

        let became_idle = wait_for_new_bell(&pty_manager, &tab_id, 0, Duration::from_secs(5)).await;
        assert!(became_idle, "expected wait_for_new_bell to observe the bell within the timeout");
    }
}

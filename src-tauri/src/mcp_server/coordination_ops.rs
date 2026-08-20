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
    tabs: Mutex<HashMap<String, u64>>,
}

impl CoordinationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_known(&self, tab_id: &str) -> bool {
        self.tabs.lock().contains_key(tab_id)
    }

    fn record_baseline(&self, tab_id: &str, baseline: u64) {
        self.tabs.lock().insert(tab_id.to_string(), baseline);
    }

    fn baseline(&self, tab_id: &str) -> Option<u64> {
        self.tabs.lock().get(tab_id).copied()
    }
}

#[derive(Serialize)]
struct TabStatus {
    idle: bool,
    recent_output: String,
}

#[derive(Serialize)]
struct WaitResult {
    idle: bool,
    recent_output: String,
    timed_out: bool,
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

    // Fresh session: bell_count() is 0 right now. Recording that as the
    // baseline means get_tab_status/wait_for_idle report "not idle" until the
    // first bell — reasonable, since nothing has run yet either way.
    registry.record_baseline(&tab_id, 0);

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

pub(crate) fn send_input(
    pty_manager: &PtyManager,
    registry: &CoordinationRegistry,
    tab_id: &str,
    text: &str,
) -> Result<String, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    pty_manager
        .write(tab_id, format!("{text}\r").as_bytes())
        .map_err(|e| e.to_string())?;

    // Reset the baseline to the count *as of right now*, before any reply has
    // had a chance to arrive — so idle only flips true again once a *new*
    // bell (a reply to this specific input) is observed, not a stale one from
    // before this send.
    let current = pty_manager.bell_count(tab_id).unwrap_or(0);
    registry.record_baseline(tab_id, current);

    Ok(format!("sent to {tab_id}"))
}

fn status_for(pty_manager: &PtyManager, registry: &CoordinationRegistry, tab_id: &str) -> Result<TabStatus, String> {
    if !registry.is_known(tab_id) {
        return Err(format!(
            "tab_id '{tab_id}' was not created by spawn_tab — this tool can only target tabs it spawned itself, never a tab the user opened by hand"
        ));
    }
    let baseline = registry.baseline(tab_id).unwrap_or(0);
    let current = pty_manager
        .bell_count(tab_id)
        .ok_or_else(|| format!("tab_id '{tab_id}' is no longer running (it may have been closed)"))?;
    let recent_output = pty_manager
        .get_recent_output(tab_id, RECENT_OUTPUT_BYTES)
        .unwrap_or_default();
    Ok(TabStatus { idle: current > baseline, recent_output })
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
            let result = WaitResult { idle: true, recent_output: status.recent_output, timed_out: false };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            let result = WaitResult { idle: false, recent_output: status.recent_output, timed_out: true };
            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pty_size() -> PtySize {
        PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
    }

    #[tokio::test]
    async fn send_input_rejects_a_tab_id_not_in_the_registry() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let err = send_input(&pty_manager, &registry, "not-a-real-tab", "hello").unwrap_err();
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
        registry.record_baseline(&tab_id, 0);

        let sent = send_input(&pty_manager, &registry, &tab_id, "echo hi").unwrap();
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
        registry.record_baseline(&tab_id, 0);

        let result_json = wait_for_idle(&pty_manager, &registry, &tab_id, Some(1)).await.unwrap();
        assert!(result_json.contains("\"timed_out\": true"), "{result_json}");
        assert!(result_json.contains("\"idle\": false"), "{result_json}");
    }

    #[tokio::test]
    async fn wait_for_idle_returns_idle_once_a_bell_is_observed() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0);

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
    async fn send_input_terminates_the_line_with_cr_not_lf() {
        let pty_manager = PtyManager::new();
        let registry = CoordinationRegistry::new();
        let tab_id = pty_manager.create_with_callback(pty_size(), |_| {}).unwrap();
        registry.record_baseline(&tab_id, 0);

        // Sent while the pty is still in normal canonical+echo mode, so this
        // setup line's own `\n` terminator is unrelated to the bug under
        // test. It flips the pty into raw mode, prints a marker (built via
        // `''` string concatenation so the *typed/echoed* command text never
        // contains the contiguous marker string — only the executed
        // printf's actual output does), then blocks reading exactly 3 raw
        // bytes and dumps them as hex.
        pty_manager
            .write(&tab_id, b"stty raw -echo; printf 'MARK''READY'; od -An -tx1 -N 3\n")
            .unwrap();

        // Wait for proof that `stty raw -echo` has already run (the marker
        // only appears from printf's actual output, never from the echoed
        // input line) before writing the bytes under test — otherwise they
        // could race a still-canonical pty.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty_manager.get_recent_output(&tab_id, RECENT_OUTPUT_BYTES).unwrap_or_default();
            if output.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "raw mode setup never completed: {output}");
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        }

        send_input(&pty_manager, &registry, &tab_id, "ab").unwrap();

        // Poll for od's 3-byte hex dump of exactly what send_input wrote:
        // "ab" (0x61 0x62) followed by whatever terminator byte it used.
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
            assert!(tokio::time::Instant::now() < deadline, "od never dumped 3 bytes: {output}");
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
}

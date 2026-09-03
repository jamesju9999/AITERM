//! Turning a queued card into a live dispatch: compose the prompt text,
//! spawn a visible PTY tab running the configured `claude` command, wait for
//! it to settle, then type the prompt in using the same CR-terminated /
//! done-marker-instruction sequencing `coordination_ops::send_input` uses.

use std::time::Duration;

use portable_pty::PtySize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::pty::manager::PtyManager;
use crate::pty::session::done_marker_instruction;

/// Body text plus, if any attachments, one trailing line pointing `claude` at
/// their on-disk paths (they've already been copied into the task dir).
pub fn build_prompt(body: &str, attachment_paths: &[String]) -> String {
    if attachment_paths.is_empty() {
        return body.to_string();
    }
    let list = attachment_paths.join("、");
    format!("{body}\n\n（相關附件：{list}）")
}

/// Max wait for `claude` to finish its cold start (spec measured ~3.7s) before
/// we type the prompt. If it's still noisy at this point we send anyway.
const SETTLE_TIMEOUT_MS: u64 = 30_000;
/// The session is "settled" once it has produced no output for this long.
const SETTLE_QUIET_MS: u64 = 800;
const POLL_MS: u64 = 250;
/// Same as `coordination_ops::DONE_MARKER_WAIT_SECONDS` — how long to wait for
/// `claude` to ring a fresh bell (signalling it finished reading the prompt)
/// before sending the optional done-marker instruction.
const DONE_MARKER_WAIT_SECONDS: u64 = 15;
/// Gap between writing the prompt body and writing the standalone `\r` that
/// submits it (see `run_on_session`). Long enough that the two writes don't
/// land in the same PTY read() burst on the target's side; short enough not
/// to add noticeable dispatch latency.
const SUBMIT_DELAY_MS: u64 = 120;

/// bell/marker counts captured right after the prompt (and optional
/// instruction) were written — the baseline the monitor compares fresh
/// counts against to detect "claude replied to *this* prompt".
#[derive(Debug, Clone, Copy)]
pub struct DispatchResult {
    pub bell_baseline: u64,
    pub marker_baseline: u64,
}

/// Payload for `mcp-coordination-tab-spawned` — the event the frontend
/// already listens for to adopt a backend-spawned session as a visible tab.
/// Same field names as `coordination_ops`'s copy.
#[derive(Serialize, Clone)]
struct TabSpawnedEvent {
    session_id: String,
    command: Option<String>,
}

/// Wait until `tab_id` has produced no output for `SETTLE_QUIET_MS`, or
/// `SETTLE_TIMEOUT_MS` elapses. Lets `claude` finish booting before we type.
async fn wait_until_settled(pty: &PtyManager, tab_id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(SETTLE_TIMEOUT_MS);
    loop {
        let quiet = pty.ms_since_output(tab_id).unwrap_or(u64::MAX);
        if quiet >= SETTLE_QUIET_MS || tokio::time::Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}

/// Type `prompt` into an already-running session (a `claude` REPL, normally).
/// The prompt body and the submitting `\r` are deliberately two separate
/// writes, not one `format!("{prompt}\r")` burst: a multi-line prompt (a
/// task's `body` is a free-form textarea, and `build_prompt` itself inserts
/// a blank line before the attachment note whenever there are attachments)
/// contains embedded `\n` bytes, and a raw-mode TUI receiving a burst that
/// contains embedded newlines commonly treats the whole thing as a paste —
/// which fills the input box but does NOT auto-submit even if that same
/// burst happens to end in `\r` (confirmed live: an attachment-bearing task
/// sat typed-but-unsubmitted in Claude Code's input until the 120s-stuck
/// timeout failed it). A short delay before the standalone `\r` write keeps
/// the two writes from being coalesced back into one read() on the far end.
/// Then, if `request_done_marker`, wait for a fresh bell and send
/// `done_marker_instruction` as a further, independent CR-terminated write
/// (same reasoning `coordination_ops::send_input` already documents for that
/// step). Returns the post-write bell/marker baselines.
pub async fn run_on_session(
    pty: &PtyManager,
    tab_id: &str,
    prompt: &str,
    request_done_marker: bool,
) -> Result<DispatchResult, String> {
    pty.write(tab_id, prompt.as_bytes()).map_err(|e| e.to_string())?;
    tokio::time::sleep(Duration::from_millis(SUBMIT_DELAY_MS)).await;
    pty.write(tab_id, b"\r").map_err(|e| e.to_string())?;

    if request_done_marker {
        // Wait for a bell as a best-effort signal that `claude` finished
        // reading the prompt and is ready for more input — but send the
        // instruction regardless of whether one actually arrived. A real
        // `claude` CLI in this environment has been observed to complete a
        // whole task without ever ringing a single bell (see the
        // coordination-done-marker design doc). Gating the instruction send
        // on `became_idle`, as an earlier version of this function did,
        // meant a `claude` session that simply doesn't bell was NEVER even
        // asked to print the completion marker — confirmed live: a task
        // that actually finished (visible in its own tab) still landed on
        // the monitor's 120s-stuck path and got marked failed, because
        // neither signal it's waiting for ever fired. The wait still has
        // value (gives `claude` time to become idle before we write more,
        // same reasoning `coordination_ops::send_input` documents), it's
        // just not treated as a gate on whether to bother at all.
        let bell_before = pty.bell_count(tab_id).unwrap_or(0);
        crate::mcp_server::coordination_ops::wait_for_new_bell(
            pty,
            tab_id,
            bell_before,
            Duration::from_secs(DONE_MARKER_WAIT_SECONDS),
        )
        .await;
        let instr = done_marker_instruction(tab_id);
        pty.write(tab_id, format!("{instr}\r").as_bytes()).map_err(|e| e.to_string())?;
    }

    Ok(DispatchResult {
        bell_baseline: pty.bell_count(tab_id).unwrap_or(0),
        marker_baseline: pty.marker_count(tab_id).unwrap_or(0),
    })
}

/// Full dispatch: create a visible tab in `project_dir` running
/// `claude_command`, emit the adopt event, wait for it to settle, type the
/// prompt in. Returns `(tab_id, DispatchResult)`.
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    prompt: &str,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(app.clone(), size, Some(std::path::PathBuf::from(project_dir)), None)
        .map_err(|e| e.to_string())?;

    if let Err(e) = pty.write(&tab_id, format!("{claude_command}\r").as_bytes()) {
        let _ = pty.close(&tab_id);
        return Err(e.to_string());
    }
    if let Err(e) = app.emit(
        "mcp-coordination-tab-spawned",
        TabSpawnedEvent { session_id: tab_id.clone(), command: Some(claude_command.to_string()) },
    ) {
        eprintln!("emit mcp-coordination-tab-spawned failed: {e}");
    }

    wait_until_settled(pty, &tab_id).await;
    let result = run_on_session(pty, &tab_id, prompt, true).await?;
    Ok((tab_id, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_just_the_body_when_there_are_no_attachments() {
        assert_eq!(build_prompt("Do the thing", &[]), "Do the thing");
    }

    #[test]
    fn prompt_appends_one_line_listing_attachment_paths() {
        let p = build_prompt(
            "Refactor per the spec",
            &["/data/tasks/x/attachments/spec.md".into(), "/data/tasks/x/attachments/before.png".into()],
        );
        assert!(p.starts_with("Refactor per the spec"));
        assert!(p.contains("/data/tasks/x/attachments/spec.md"));
        assert!(p.contains("/data/tasks/x/attachments/before.png"));
        // Attachment note is on its own line, after a blank line.
        assert!(p.contains("\n\n"));
    }

    #[test]
    fn blank_body_still_produces_the_attachment_note() {
        let p = build_prompt("", &["/a/b.txt".into()]);
        assert!(p.contains("/a/b.txt"));
    }

    use crate::pty::manager::PtyManager;

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_types_the_prompt_into_an_existing_session() {
        let pty = PtyManager::new();
        // A plain shell stands in for `claude` — it echoes typed lines back.
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        let res = run_on_session(&pty, &tab_id, "echo TASKBOARD_PROMPT_MARKER", false)
            .await
            .unwrap();
        let _ = res.marker_baseline; // field exists / no panic

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if out.contains("TASKBOARD_PROMPT_MARKER") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "prompt never echoed: {out}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Regression test for a real bug: a multi-line prompt (e.g. `build_prompt`
    /// with attachments, or just a multi-line task body) sat typed-but-never-
    /// submitted in Claude Code's input box, because the old code sent the
    /// whole thing — embedded newlines and the submitting `\r` — as one single
    /// `pty.write`. Mirrors `coordination_ops::send_input_terminates_the_line_
    /// with_cr_not_lf`'s technique: put the pty in raw mode and dump the exact
    /// bytes that arrive, rather than trusting a canonical-mode shell's
    /// tolerance for either terminator. This proves our own write pipeline
    /// delivers the multi-line body byte-for-byte (embedded LF preserved, not
    /// converted or dropped) followed by a real standalone CR — it cannot
    /// prove Claude Code's specific TUI then submits on it (that needs a live
    /// check against the real binary), but it locks in the one thing this
    /// commit actually controls: two real, distinct writes with the right
    /// bytes, not one burst that smuggled the `\r` inside embedded-newline
    /// content.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_sends_a_multiline_prompt_verbatim_then_a_standalone_cr() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        // Same setup as the coordination_ops.rs test this mirrors: flip the
        // pty into raw mode, print a marker (built via concatenation so the
        // *echoed* setup command itself never contains the contiguous
        // marker), then block reading exactly N raw bytes and dump them as
        // hex. N = len("line one\nline two") + 1 (the trailing CR).
        let prompt = "line one\nline two";
        let expect_len = prompt.len() + 1;
        #[cfg(not(windows))]
        pty.write(
            &tab_id,
            format!("stty raw -echo; printf 'MARK''READY'; od -An -tx1 -N {expect_len}\n").as_bytes(),
        )
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if output.contains("MARKREADY") {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "byte-dump setup never completed: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        run_on_session(&pty, &tab_id, prompt, false).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let hex_bytes: Vec<String> = loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            let after_marker = output.split("MARKREADY").nth(1).unwrap_or("").to_string();
            let hex_bytes: Vec<String> = after_marker
                .split_whitespace()
                .filter(|tok| tok.len() == 2 && tok.chars().all(|c| c.is_ascii_hexdigit()))
                .map(str::to_string)
                .collect();
            if hex_bytes.len() >= expect_len {
                break hex_bytes;
            }
            assert!(tokio::time::Instant::now() < deadline, "{expect_len}-byte hex dump never appeared: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };

        let expected: Vec<String> = prompt
            .bytes()
            .chain(std::iter::once(b'\r'))
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            &hex_bytes[..expect_len],
            expected.as_slice(),
            "expected the multi-line prompt's exact bytes (embedded 0a preserved) followed by a standalone 0d — got: {hex_bytes:?}"
        );
    }

    /// Regression test for a real bug found live: a task that genuinely
    /// finished (visible completing in its own tab) still got marked failed
    /// by the monitor's 120s-stuck path, because the done-marker instruction
    /// was never sent — the old code only sent it after observing a fresh
    /// bell, and a real `claude` CLI has been observed to complete a whole
    /// turn without ever ringing one. No bell is injected here at all;
    /// the instruction (which mentions the tab_id) must still be sent once
    /// the wait elapses. Runs for real at ~DONE_MARKER_WAIT_SECONDS (15s),
    /// same as the sibling test this mirrors in coordination_ops.rs.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_sends_the_done_marker_instruction_even_when_the_target_never_bells() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        run_on_session(&pty, &tab_id, "echo hi", true).await.unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let output = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
            if output.contains(&tab_id) {
                break; // the instruction mentions its own tab_id
            }
            assert!(tokio::time::Instant::now() < deadline, "done-marker instruction was never sent: {output}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

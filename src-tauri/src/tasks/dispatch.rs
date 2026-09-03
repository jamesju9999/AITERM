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
/// Mirrors `coordination_ops::send_input`: CR-terminated write; then, if
/// `request_done_marker`, wait for a fresh bell and send
/// `done_marker_instruction` as a second independent CR-terminated write.
/// Returns the post-write bell/marker baselines.
pub async fn run_on_session(
    pty: &PtyManager,
    tab_id: &str,
    prompt: &str,
    request_done_marker: bool,
) -> Result<DispatchResult, String> {
    pty.write(tab_id, format!("{prompt}\r").as_bytes()).map_err(|e| e.to_string())?;

    if request_done_marker {
        let bell_before = pty.bell_count(tab_id).unwrap_or(0);
        let became_idle = crate::mcp_server::coordination_ops::wait_for_new_bell(
            pty,
            tab_id,
            bell_before,
            Duration::from_secs(DONE_MARKER_WAIT_SECONDS),
        )
        .await;
        if became_idle {
            let instr = done_marker_instruction(tab_id);
            pty.write(tab_id, format!("{instr}\r").as_bytes()).map_err(|e| e.to_string())?;
        }
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
}

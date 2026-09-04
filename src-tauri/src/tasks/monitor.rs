//! Watches one dispatched task's PTY session until it reaches a terminal
//! outcome. Signals, in priority order:
//!   1. control channel fires Cancel or MarkDone → Cancelled / Success
//!   2. fresh bell or done-marker observed (Auto mode only) → Success
//!   3. OSC133 non-zero exit code          → Failed("claude 以 exit code N 結束")
//!   4. OSC133 exit code 0                 → Success (claude exited cleanly)
//!   5. no output for `quiet_stuck_ms` (Auto mode only) → Failed("疑似卡住（120 秒無輸出）")
//!
//! Interactive mode (`WatchMode::Interactive`) skips signals ②⑤ — see that
//! variant's doc comment for why. Reuses `PtyManager`'s existing per-session
//! counters — no new protocol.

use std::time::Duration;

use tokio::sync::oneshot;

use crate::pty::manager::PtyManager;

#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Success,
    Failed(String),
    Cancelled,
}

impl TaskOutcome {
    /// The `outcome` string stored in `tasks.db`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskOutcome::Success => "success",
            TaskOutcome::Failed(_) => "failed",
            TaskOutcome::Cancelled => "cancelled",
        }
    }
    pub fn error_message(&self) -> Option<&str> {
        match self {
            TaskOutcome::Failed(m) => Some(m.as_str()),
            _ => None,
        }
    }
}

/// External control signal for a running `watch()`. Sent through the same
/// oneshot channel that used to only carry cancellation — `SchedulerHandle`
/// exposes `cancel()`/`mark_done()` as two thin wrappers over one `send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchControl {
    Cancel,
    MarkDone,
}

/// Which soft completion signals `watch()` trusts. Both modes still trust
/// the hard signal — the Claude Code process actually exiting (③④) — since
/// that can't be a false positive from mid-conversation chatter the way
/// bell/marker/silence can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// bell/marker → success, 120s silence → stuck-failed. Current
    /// behavior for tasks dispatched to run unattended.
    Auto,
    /// A human is expected to be chatting in this tab — bell fires on
    /// every reply, and thinking for well over 120s is normal, so both of
    /// those signals would misfire constantly. Completion is signalled
    /// externally instead (`WatchControl::MarkDone`).
    Interactive,
}

/// bell/marker counts as of the moment the prompt was sent (from
/// `dispatch::DispatchResult`). `Default` = all zero, for tests that don't
/// prime the session first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Baselines {
    pub bell: u64,
    pub marker: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Thresholds {
    /// No output for this long ⇒ stuck. Production: 120_000.
    pub quiet_stuck_ms: u64,
    /// Poll interval. Production: 250.
    pub poll_ms: u64,
    /// Don't declare "stuck" until at least this long after watch start, so a
    /// slow cold start isn't mistaken for a hang. Production: `quiet_stuck_ms`.
    pub min_run_ms: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { quiet_stuck_ms: 120_000, poll_ms: 250, min_run_ms: 120_000 }
    }
}

pub async fn watch(
    pty: &PtyManager,
    tab_id: &str,
    mut control: oneshot::Receiver<WatchControl>,
    baselines: Baselines,
    thresholds: Thresholds,
    mode: WatchMode,
) -> TaskOutcome {
    let start = tokio::time::Instant::now();
    let exit_baseline = pty.last_exit_code(tab_id);
    loop {
        // 1. external control signal
        match control.try_recv() {
            Ok(WatchControl::Cancel) => return TaskOutcome::Cancelled,
            Ok(WatchControl::MarkDone) => return TaskOutcome::Success,
            Err(oneshot::error::TryRecvError::Closed) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Session gone (tab closed out from under us) — treat as cancelled.
        let Some(bell) = pty.bell_count(tab_id) else {
            return TaskOutcome::Cancelled;
        };
        let marker = pty.marker_count(tab_id).unwrap_or(0);

        // 2. reply signal — Auto mode only (see WatchMode::Interactive's doc).
        if mode == WatchMode::Auto && (marker > baselines.marker || bell > baselines.bell) {
            return TaskOutcome::Success;
        }

        // 3/4. process exited *since we started watching* (a pre-existing exit
        // code — e.g. Windows' injected prompt emits OSC133 D;0 on the very
        // first draw — must not count as this task finishing). Both modes,
        // this is a hard signal either way.
        let current_exit = pty.last_exit_code(tab_id);
        if current_exit != exit_baseline {
            if let Some(code) = current_exit {
                if code != 0 {
                    return TaskOutcome::Failed(format!("claude 以 exit code {code} 結束"));
                }
                return TaskOutcome::Success;
            }
        }

        // 5. stuck — Auto mode only.
        let ran_ms = start.elapsed().as_millis() as u64;
        let quiet_ms = pty.ms_since_output(tab_id).unwrap_or(0);
        if mode == WatchMode::Auto && ran_ms >= thresholds.min_run_ms && quiet_ms >= thresholds.quiet_stuck_ms {
            return TaskOutcome::Failed("疑似卡住（120 秒無輸出）".to_string());
        }

        tokio::time::sleep(Duration::from_millis(thresholds.poll_ms)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::manager::PtyManager;
    use crate::pty::session::done_marker;
    use portable_pty::PtySize;
    use std::time::Duration;

    fn size() -> PtySize {
        PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }
    }

    fn test_thresholds() -> Thresholds {
        Thresholds { quiet_stuck_ms: 60_000, poll_ms: 50, min_run_ms: 0 }
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn marker_in_output_yields_success() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let marker = done_marker(&tab);
        pty.write(&tab, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn nonzero_exit_yields_failed() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains('3'), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "PowerShell shell-integration writes OSC133 D;0 on its very first \
                  prompt unconditionally (no __aiterm_cmd_running guard, unlike the \
                  zsh/bash hooks), so last_exit_code is Some(0) at startup and watch \
                  returns Success before the stuck threshold — real-ConPTY quirk, \
                  tracked separately"
    )]
    async fn silence_past_the_threshold_yields_failed_stuck() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("expected Failed(stuck), got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn a_stale_exit_code_from_before_watch_does_not_count_as_completion() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        pty.write(&tab, b"true\n").unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if pty.last_exit_code(&tab) == Some(0) { break; }
            assert!(tokio::time::Instant::now() < deadline, "shell never emitted D;0");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("stale Some(0) must not yield {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_signal_yields_cancelled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }

    #[tokio::test]
    async fn mark_done_control_signal_yields_success() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        tx.send(WatchControl::MarkDone).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn interactive_mode_still_fails_on_nonzero_exit() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Interactive).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains('3'), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // Proves the marker/bell signal is genuinely ignored in Interactive
    // mode — not by waiting a fixed duration and assuming (a coincidence-
    // based test would pass just as well against broken code that merely
    // runs slowly), but by racing watch() against a real timeout: if it's
    // STILL RUNNING after a window well past what Auto mode would need to
    // return, that's a real, observed fact about the future's state, not a
    // guess. Then proves the loop is genuinely alive (not just slow) by
    // cancelling it and checking it reacts.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn interactive_mode_does_not_treat_a_marker_as_completion() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let marker = done_marker(&tab);
        // `&& sleep 2` keeps the shell mid-command (so OSC133 D — the hard
        // exit signal, intentionally still active in Interactive mode —
        // does not fire within this test's observation window and get
        // confused for what's actually being tested here: that the marker
        // itself is ignored). The Cancel sent below aborts watch() long
        // before the sleep would complete.
        pty.write(&tab, format!("printf '%s\\n' '{marker}' && sleep 2\n").as_bytes()).unwrap();

        let watch_fut = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Interactive);
        tokio::pin!(watch_fut);

        let still_running = tokio::time::timeout(Duration::from_millis(500), &mut watch_fut).await.is_err();
        assert!(still_running, "watch() returned early in Interactive mode — marker signal was not ignored");

        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch_fut.await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }

    // Same non-coincidental-timeout technique as the marker test above, for
    // the 120s-stuck signal.
    #[tokio::test]
    async fn interactive_mode_does_not_time_out_on_silence() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        // Never write anything — session silent from the start.
        let thresholds = Thresholds { quiet_stuck_ms: 200, poll_ms: 50, min_run_ms: 100 };

        let watch_fut = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Interactive);
        tokio::pin!(watch_fut);

        let still_running = tokio::time::timeout(Duration::from_millis(600), &mut watch_fut).await.is_err();
        assert!(still_running, "watch() returned early in Interactive mode — stuck-timeout signal was not ignored");

        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch_fut.await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }
}

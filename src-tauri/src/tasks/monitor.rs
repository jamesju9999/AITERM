//! Watches one dispatched task's PTY session until it reaches a terminal
//! outcome. Signals, in priority order:
//!   1. cancel channel fired               → Cancelled
//!   2. fresh bell or done-marker observed → Success
//!   3. OSC133 non-zero exit code          → Failed("claude 以 exit code N 結束")
//!   4. OSC133 exit code 0                 → Success (claude exited cleanly)
//!   5. no output for `quiet_stuck_ms`     → Failed("疑似卡住（120 秒無輸出）")
//! Reuses `PtyManager`'s existing per-session counters — no new protocol.

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
    mut cancel: oneshot::Receiver<()>,
    baselines: Baselines,
    thresholds: Thresholds,
) -> TaskOutcome {
    let start = tokio::time::Instant::now();
    loop {
        // 1. cancel
        match cancel.try_recv() {
            Ok(()) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Closed) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Session gone (tab closed out from under us) — treat as cancelled.
        let Some(bell) = pty.bell_count(tab_id) else {
            return TaskOutcome::Cancelled;
        };
        let marker = pty.marker_count(tab_id).unwrap_or(0);

        // 2. reply signal
        if marker > baselines.marker || bell > baselines.bell {
            return TaskOutcome::Success;
        }

        // 3/4. process exited
        if let Some(code) = pty.last_exit_code(tab_id) {
            if code != 0 {
                return TaskOutcome::Failed(format!("claude 以 exit code {code} 結束"));
            }
            return TaskOutcome::Success;
        }

        // 5. stuck
        let ran_ms = start.elapsed().as_millis() as u64;
        let quiet_ms = pty.ms_since_output(tab_id).unwrap_or(0);
        if ran_ms >= thresholds.min_run_ms && quiet_ms >= thresholds.quiet_stuck_ms {
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
        let (_tx, rx) = tokio::sync::oneshot::channel();
        let marker = done_marker(&tab);
        pty.write(&tab, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn nonzero_exit_yields_failed() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
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
        let (_tx, rx) = tokio::sync::oneshot::channel();
        // Never write anything: session is quiet from the start.
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("expected Failed(stuck), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_signal_yields_cancelled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tx.send(()).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds()).await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }
}

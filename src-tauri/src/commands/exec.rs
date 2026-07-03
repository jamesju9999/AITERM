use serde::Serialize;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const MAX_OUTPUT_CHARS: usize = 10_000;

#[derive(Debug, Serialize)]
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

/// Run a shell command with a hard timeout, capturing stdout/stderr.
/// Unix: `sh -c`, Windows: `cmd /C`. Output is truncated to MAX_OUTPUT_CHARS.
pub async fn run_command(
    command: &str,
    cwd: Option<&str>,
    timeout_ms: u64,
) -> Result<ExecResult, String> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let mut stdout_pipe = child.stdout.take().ok_or("failed to capture stdout")?;
    let mut stderr_pipe = child.stderr.take().ok_or("failed to capture stderr")?;

    // Read pipes concurrently so a killed child still yields partial output.
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buf).await;
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buf).await;
        buf
    });

    let (exit_code, timed_out) = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("wait failed: {e}")),
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };

    let stdout_buf = stdout_task.await.unwrap_or_default();
    let stderr_buf = stderr_task.await.unwrap_or_default();

    Ok(ExecResult {
        stdout: truncate_lossy(&stdout_buf),
        stderr: truncate_lossy(&stderr_buf),
        exit_code,
        timed_out,
    })
}

fn truncate_lossy(buf: &[u8]) -> String {
    let s = String::from_utf8_lossy(buf);
    if s.chars().count() > MAX_OUTPUT_CHARS {
        let mut t: String = s.chars().take(MAX_OUTPUT_CHARS).collect();
        t.push_str("\n[...truncated]");
        t
    } else {
        s.into_owned()
    }
}

#[tauri::command]
pub async fn agent_exec(
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<ExecResult, String> {
    run_command(&command, cwd.as_deref(), timeout_ms.unwrap_or(60_000)).await
}

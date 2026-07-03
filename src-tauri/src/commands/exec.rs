use serde::Serialize;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

const MAX_OUTPUT_CHARS: usize = 10_000;
/// Hard cap on bytes stored per stream. Reading continues past this cap (to
/// drain the pipe and avoid backpressure) but further data is discarded.
const MAX_BUFFER_BYTES: usize = 200_000;
/// Grace period for reader tasks to reach EOF after the child exits/is killed.
const READER_GRACE_MS: u64 = 2_000;

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

    // Read pipes chunk-by-chunk into shared buffers so partial output is
    // retrievable even if a reader task never reaches EOF (e.g. an orphaned
    // descendant keeps the write end of the pipe open after the child dies).
    let stdout_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

    let mut stdout_task = {
        let buf = Arc::clone(&stdout_buf);
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stdout_pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut b = buf.lock().await;
                        let room = MAX_BUFFER_BYTES.saturating_sub(b.len());
                        b.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
            }
        })
    };
    let mut stderr_task = {
        let buf = Arc::clone(&stderr_buf);
        tokio::spawn(async move {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut b = buf.lock().await;
                        let room = MAX_BUFFER_BYTES.saturating_sub(b.len());
                        b.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
            }
        })
    };

    let (exit_code, timed_out) = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => {
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!("wait failed: {e}"));
        }
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };

    // kill() only reaches the direct sh/cmd child. Orphaned grandchildren
    // survive it (process-group kill is a possible future hardening) and can
    // hold the inherited pipe open past EOF, so give the readers a short grace
    // period, then abort them and take whatever the shared buffers hold.
    if timeout(
        Duration::from_millis(READER_GRACE_MS),
        async { let _ = (&mut stdout_task).await; let _ = (&mut stderr_task).await; },
    )
    .await
    .is_err()
    {
        stdout_task.abort();
        stderr_task.abort();
    }

    let stdout_bytes = stdout_buf.lock().await;
    let stderr_bytes = stderr_buf.lock().await;

    Ok(ExecResult {
        stdout: truncate_lossy(&stdout_bytes),
        stderr: truncate_lossy(&stderr_bytes),
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

// src-tauri/src/api_docs/runner.rs
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use super::types::DocNode;

// ── Tauri event payloads ──────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ApiDocsProgressEvent {
    pub current: u32,
    pub total: u32,
    pub page: String,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsLogEvent {
    pub level: String,   // "info" | "warn" | "error"
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsDoneEvent {
    pub files: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ApiDocsDetectedEvent {
    pub platform: String,
    pub confidence: String,
}

// ── Line protocol from Python stdout ─────────────────────────────────────────

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Detected { platform: String, confidence: String },
    Tree { data: Vec<DocNode> },
    Progress { current: u32, total: u32, page: String },
    Log { level: String, message: String },
    Done { files: Vec<String> },
    Error { message: String },
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run `fetcher.py <subcommand> [args]` and stream events.
/// Returns the parsed tree for the `tree` subcommand, or `None` for `extract`.
pub async fn run_fetcher(
    app: &AppHandle,
    script: &Path,
    subcommand: &str,
    extra_args: &[&str],
) -> Result<Option<Vec<DocNode>>, String> {
    let python = super::find_python();

    // Auto-install Python dependencies on first use (fast no-op if already installed).
    if let Some(script_dir) = script.parent() {
        let req_file = script_dir.join("requirements.txt");
        if req_file.exists() {
            let _ = app.emit("api-docs-log", ApiDocsLogEvent {
                level: "info".into(),
                message: "Checking Python dependencies…".into(),
            });
            match Command::new(python)
                .args(["-m", "pip", "install", "-r"])
                .arg(&req_file)
                .args(["--quiet", "--disable-pip-version-check"])
                .output()
                .await
            {
                Ok(out) if !out.status.success() => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let _ = app.emit("api-docs-log", ApiDocsLogEvent {
                        level: "warn".into(),
                        message: format!("pip install warning: {}", stderr.trim()),
                    });
                }
                Err(e) => {
                    let _ = app.emit("api-docs-log", ApiDocsLogEvent {
                        level: "warn".into(),
                        message: format!("Could not run pip: {e}"),
                    });
                }
                _ => {}
            }
        }
    }

    let mut child = Command::new(python)
        .arg(script)
        .arg(subcommand)
        .args(extra_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn Python: {e}"))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut lines = BufReader::new(stdout).lines();

    // Collect stderr concurrently to avoid pipe-buffer deadlocks and capture error messages.
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut tree: Option<Vec<DocNode>> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() { continue; }

        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Detected { platform, confidence }) => {
                let _ = app.emit("api-docs-detected", ApiDocsDetectedEvent { platform, confidence });
            }
            Ok(PythonLine::Tree { data }) => {
                tree = Some(data);
            }
            Ok(PythonLine::Progress { current, total, page }) => {
                let _ = app.emit("api-docs-progress", ApiDocsProgressEvent { current, total, page });
            }
            Ok(PythonLine::Log { level, message }) => {
                let _ = app.emit("api-docs-log", ApiDocsLogEvent { level, message });
            }
            Ok(PythonLine::Done { files }) => {
                let _ = app.emit("api-docs-done", ApiDocsDoneEvent { files });
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {
                // Non-JSON debug output — emit as info log so user can see it
                let _ = app.emit("api-docs-log", ApiDocsLogEvent {
                    level: "info".into(),
                    message: line.to_string(),
                });
            }
        }
    }

    // Wait for the child and check exit code
    let stderr_output = stderr_task.await.unwrap_or_default();
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() {
        let detail = if stderr_output.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr_output.trim())
        };
        return Err(format!("fetcher.py exited with code {:?}{}", status.code(), detail));
    }

    Ok(tree)
}

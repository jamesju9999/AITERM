// src-tauri/src/commands/markitdown.rs
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncBufReadExt;
use serde::Deserialize;

fn find_python() -> &'static str {
    if cfg!(target_os = "windows") { "python" } else { "python3" }
}

fn converter_script_path(app: &AppHandle) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_path = manifest_dir
        .parent()
        .unwrap_or(&manifest_dir)
        .join("tools")
        .join("MarkItDown")
        .join("converter.py");
    if dev_path.exists() {
        return dev_path;
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("MarkItDown").join("converter.py");
        if prod_path.exists() {
            return prod_path;
        }
    }
    dev_path
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PythonLine {
    Done { markdown: String },
    Error { message: String },
}

/// Convert a local file to Markdown using MarkItDown.
/// Auto-installs Python deps on first use (fast no-op if already installed).
#[tauri::command]
pub async fn markitdown_convert(app: AppHandle, file_path: String) -> Result<String, String> {
    let script = converter_script_path(&app);
    let python = find_python();
    let script_dir = script.parent().unwrap_or(script.as_path());
    let req_file = script_dir.join("requirements.txt");

    // Auto-install deps (same pattern as api_docs/runner.rs)
    if req_file.exists() {
        #[allow(unused_mut)]
        let mut pip_cmd = tokio::process::Command::new(python);
        pip_cmd
            .args(["-m", "pip", "install", "-r"])
            .arg(&req_file)
            .args(["--quiet", "--disable-pip-version-check"])
            .current_dir(script_dir);
        #[cfg(target_os = "linux")]
        pip_cmd.args(["--user", "--break-system-packages"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            pip_cmd.creation_flags(0x08000000);
        }
        let _ = pip_cmd.output().await;
    }

    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(&script)
        .arg(&file_path)
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn Python: {e}"))?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let mut result: Option<String> = None;

    while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<PythonLine>(line) {
            Ok(PythonLine::Done { markdown }) => {
                result = Some(markdown);
            }
            Ok(PythonLine::Error { message }) => {
                return Err(message);
            }
            Err(_) => {}
        }
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    let status = child.wait().await.map_err(|e| e.to_string())?;
    if !status.success() && result.is_none() {
        let detail = if stderr_output.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", stderr_output.trim())
        };
        return Err(format!(
            "converter.py exited with code {:?}{}",
            status.code(),
            detail
        ));
    }

    result.ok_or_else(|| "converter.py did not emit markdown".to_string())
}

/// Open a native OS file picker and return the selected path, or None if cancelled.
#[tauri::command]
pub async fn markitdown_pick_file() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Documents",
            &[
                "xlsx", "xls", "csv", "docx", "pdf", "pptx", "html", "htm",
                "jpg", "jpeg", "png", "gif", "webp", "epub", "msg",
                "txt", "md", "rst", "xml", "json",
            ],
        )
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().to_string())
}

//! Manages the Python environment the app's Python-backed features need.
//!
//! Everything runs through the bundled uv binary: it installs an interpreter,
//! creates a venv under app data, and installs per-profile requirements. No
//! feature touches the user's own Python installation.

pub mod commands;
pub mod marker;
pub mod paths;
pub mod profiles;

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncBufReadExt;

use commands::CommandSpec;
use profiles::Profile;

/// Serialises environment preparation. Two features can ask at once (knowledge
/// base import and doc conversion), and two uv processes writing the same venv
/// is a corruption waiting to happen.
static ENSURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Serialize)]
pub struct PythonEnvLogEvent {
    pub level: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PythonEnvError {
    #[error("找不到內建的 uv 執行檔。開發環境請先執行 scripts/setup-uv-mac.sh（或對應平台的 setup-uv 腳本）。")]
    UvMissing,

    #[error("無法取得 Python：{0}")]
    PythonUnavailable(String),

    #[error("建立 Python 環境失敗：{0}")]
    VenvFailed(String),

    #[error("安裝 {profile} 相依套件失敗：{output}")]
    InstallFailed { profile: String, output: String },

    #[error("套件需要在本機編譯，但找不到編譯工具鏈：{0}")]
    ToolchainMissing(String),

    #[error("{0}")]
    Io(String),
}

impl From<PythonEnvError> for String {
    fn from(e: PythonEnvError) -> String {
        e.to_string()
    }
}

/// Heuristic for "this failed because there's no compiler", which needs a
/// different remedy than any other install failure.
fn looks_like_compile_failure(output: &str) -> bool {
    const MARKERS: [&str; 4] = [
        "command 'cc' failed",
        "command 'gcc' failed",
        "Microsoft Visual C++",
        "error: linker",
    ];
    MARKERS.iter().any(|m| output.contains(m))
}

/// Interpreter the user pointed us at when uv can't fetch one (offline or
/// behind a proxy). `None` means "let uv manage it" (the default path).
fn user_interpreter(app: &AppHandle) -> Option<PathBuf> {
    let config = app.state::<std::sync::Arc<crate::config::ConfigStore>>();
    config
        .get()
        .python_interpreter
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

/// Prepare the environment for `profile` and return its interpreter.
///
/// Idempotent and cheap once warm: the marker file short-circuits the install
/// step, so the common path is a couple of filesystem checks.
pub async fn ensure(app: &AppHandle, profile: Profile) -> Result<PathBuf, PythonEnvError> {
    let _guard = ENSURE_LOCK.lock().await;

    // `paths::app_data` falls back to "." when app_data_dir() fails, which would
    // silently build the venv in the process's working directory. Failing is
    // better than writing a multi-hundred-MB environment somewhere the user
    // never looks and the app can't find again.
    if app.path().app_data_dir().is_err() {
        return Err(PythonEnvError::Io(
            "無法取得應用程式資料目錄，請確認磁碟權限".to_string(),
        ));
    }

    let uv = paths::uv_binary().ok_or(PythonEnvError::UvMissing)?;
    let venv = paths::venv_dir(app);
    let runtimes = paths::runtime_dir(app);
    let interpreter = user_interpreter(app);

    let mut python = paths::venv_interpreter(&venv);
    if !python.exists() {
        if interpreter.is_none() {
            log(app, "info", "正在取得 Python…");
            run(app, commands::install_python(&uv, &runtimes))
                .await
                .map_err(PythonEnvError::PythonUnavailable)?;
        }
        log(app, "info", "正在建立 Python 環境…");
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(PythonEnvError::VenvFailed)?;
    }

    // A venv can survive on disk but stop working (deleted files, an OS
    // upgrade moving dylibs). Rebuild once before giving up.
    if !interpreter_works(&python).await {
        log(app, "warn", "Python 環境無法執行，正在重建…");
        let _ = tokio::fs::remove_dir_all(&venv).await;
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(PythonEnvError::VenvFailed)?;
        python = paths::venv_interpreter(&venv);
        if !interpreter_works(&python).await {
            return Err(PythonEnvError::VenvFailed(
                "重建後仍無法執行 Python".to_string(),
            ));
        }
    }

    let requirements = requirements_path(app, profile);
    // Defaulting to "install" on Err is right — a first run and an unreadable
    // marker should both install — but a missing requirements file is a
    // packaging bug, not a first run. Log before collapsing them, or that
    // distinction is gone by the time uv fails on a path that doesn't exist.
    let needs = marker::needs_install(&venv, profile, &requirements).unwrap_or_else(|e| {
        log::warn!("could not read install marker for {}: {e:#}", profile.marker_key());
        true
    });
    if needs {
        log(app, "info", "正在安裝相依套件（首次使用需要一些時間）…");
        run(app, commands::install_requirements(&uv, &python, &requirements))
            .await
            .map_err(|output| {
                if looks_like_compile_failure(&output) {
                    PythonEnvError::ToolchainMissing(tail(&output))
                } else {
                    PythonEnvError::InstallFailed {
                        profile: profile.marker_key().to_string(),
                        output: tail(&output),
                    }
                }
            })?;
        marker::record_installed(&venv, profile, &requirements)
            .map_err(|e| PythonEnvError::Io(e.to_string()))?;
    }

    Ok(python)
}

/// `tools/<dir>/<file>` in dev, the resource bundle in production.
fn requirements_path(app: &AppHandle, profile: Profile) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("tools")
        .join(profile.tool_dir())
        .join(profile.requirements_file());
    if dev.exists() {
        return dev;
    }
    app.path()
        .resource_dir()
        .map(|r| r.join(profile.tool_dir()).join(profile.requirements_file()))
        .unwrap_or(dev)
}

async fn interpreter_works(python: &Path) -> bool {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg("-c").arg("import sys");
    no_window(&mut cmd);
    matches!(cmd.status().await, Ok(s) if s.success())
}

/// Run a spec, streaming both streams to the log panel. On failure returns the
/// combined output so the caller can classify it.
async fn run(app: &AppHandle, spec: CommandSpec) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(&spec.env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("stdout not piped")?;
    let stderr = child.stderr.take().ok_or("stderr not piped")?;

    // stderr on its own task, matching api_docs/runner.rs:133. A `select!` over
    // both readers would break out of the loop on whichever stream ended first
    // and lose the other's remaining output — which is exactly the output that
    // explains a failure, since uv reports errors on stderr.
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut collected = String::new();
    let mut out_lines = tokio::io::BufReader::new(stdout).lines();
    while let Some(line) = out_lines.next_line().await.map_err(|e| e.to_string())? {
        collected.push_str(&line);
        collected.push('\n');
        log(app, "info", &line);
    }

    let stderr_output = stderr_task.await.unwrap_or_default();
    for line in stderr_output.lines() {
        log(app, "warn", line);
    }

    let status = child.wait().await.map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        collected.push_str(&stderr_output);
        Err(collected)
    }
}

fn tail(output: &str) -> String {
    output.lines().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn log(app: &AppHandle, level: &str, message: &str) {
    let _ = app.emit(
        "python-env-log",
        PythonEnvLogEvent { level: level.into(), message: message.into() },
    );
}

fn no_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_uv_names_the_setup_script_for_the_current_platform() {
        let msg = PythonEnvError::UvMissing.to_string();
        assert!(msg.contains("setup-uv"), "should point at the setup script: {msg}");
    }

    #[test]
    fn install_failure_keeps_the_tail_of_the_output() {
        let err = PythonEnvError::InstallFailed {
            profile: "doc_core".into(),
            output: "ERROR: could not build wheel for curl_cffi".into(),
        };
        assert!(err.to_string().contains("curl_cffi"));
    }

    #[test]
    fn compile_failures_are_called_out_as_toolchain_problems() {
        // A missing compiler is the one install failure a user can't fix by
        // retrying, so it must not read like a generic pip error.
        assert!(looks_like_compile_failure("error: command 'cc' failed"));
        assert!(looks_like_compile_failure("Microsoft Visual C++ 14.0 or greater is required"));
        assert!(!looks_like_compile_failure("ERROR: No matching distribution found"));
    }
}

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
    #[error("Python 環境元件遺失，請重新安裝 AITerm。（開發環境：需先執行 scripts/setup-uv-* 取得 uv 執行檔）")]
    UvMissing,

    #[error("內建的 uv 執行檔無法啟動（可能是權限或 macOS 隔離屬性問題）：{0}")]
    UvUnusable(String),

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
            emit_log(app, "info", "正在取得 Python…");
            run(app, commands::install_python(&uv, &runtimes))
                .await
                .map_err(|f| match f {
                    RunFailure::NotExecutable(e) => PythonEnvError::UvUnusable(e),
                    RunFailure::Failed(e) => PythonEnvError::PythonUnavailable(e),
                })?;
        }
        emit_log(app, "info", "正在建立 Python 環境…");
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(|f| match f {
            RunFailure::NotExecutable(e) => PythonEnvError::UvUnusable(e),
            RunFailure::Failed(e) => PythonEnvError::VenvFailed(e),
        })?;
    }

    // A venv can survive on disk but stop working (deleted files, an OS
    // upgrade moving dylibs). Rebuild once before giving up.
    //
    // `create_venv` now always passes `--clear`, so this `remove_dir_all` is no
    // longer what makes uv accept a pre-existing directory — that's covered
    // unconditionally. It stays anyway: this branch is a different failure mode
    // (an interpreter that's there but broken, not one that's merely missing),
    // and starting from a directory that's actually gone — rather than trusting
    // `--clear` to fully undo whatever left the interpreter broken — is the
    // more conservative recovery. The result is ignored deliberately: if
    // removal fails partway, `--clear` still gets a chance to finish the job.
    if !interpreter_works(&python).await {
        emit_log(app, "warn", "Python 環境無法執行，正在重建…");
        let _ = tokio::fs::remove_dir_all(&venv).await;
        run(
            app,
            commands::create_venv(&uv, &venv, &runtimes, interpreter.as_deref()),
        )
        .await
        .map_err(|f| match f {
            RunFailure::NotExecutable(e) => PythonEnvError::UvUnusable(e),
            RunFailure::Failed(e) => PythonEnvError::VenvFailed(e),
        })?;
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
        emit_log(app, "info", "正在安裝相依套件（首次使用需要一些時間）…");
        run(app, commands::install_requirements(&uv, &python, &requirements))
            .await
            .map_err(|f| match f {
                RunFailure::NotExecutable(e) => PythonEnvError::UvUnusable(e),
                RunFailure::Failed(output) => {
                    if looks_like_compile_failure(&output) {
                        PythonEnvError::ToolchainMissing(tail(&output))
                    } else {
                        PythonEnvError::InstallFailed {
                            profile: profile.marker_key().to_string(),
                            output: tail(&output),
                        }
                    }
                }
            })?;
        marker::record_installed(&venv, profile, &requirements)
            .map_err(|e| PythonEnvError::Io(e.to_string()))?;
    }

    Ok(python)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStatus {
    pub uv_available: bool,
    pub python_version: Option<String>,
    pub installed: Vec<Profile>,
    pub venv_path: String,
    pub user_interpreter: Option<String>,
}

/// Snapshot for the settings page and the feature gates. Never runs uv.
///
/// Reads `pyvenv.cfg` rather than running `python --version`. This is called
/// every time the settings page or a feature gate wants state, and paying for a
/// process spawn each time is the wrong trade for reading one small file. (A
/// sync `std::process::Command` could set CREATE_NO_WINDOW itself via
/// CommandExt, so the Windows console flash alone wouldn't have decided this —
/// the per-call cost did.) It reports the version the venv was built with, not
/// whether the interpreter still runs — `ensure()` owns that check via
/// `interpreter_works`.
pub fn status(app: &AppHandle) -> EnvStatus {
    let venv = paths::venv_dir(app);
    EnvStatus {
        uv_available: paths::uv_binary().is_some(),
        python_version: venv_python_version(&venv),
        installed: marker::installed_profiles(&venv),
        venv_path: venv.to_string_lossy().into_owned(),
        user_interpreter: user_interpreter(app).map(|p| p.to_string_lossy().into_owned()),
    }
}

/// Version recorded in the venv's `pyvenv.cfg`, which uv writes as
/// `version_info = 3.12.13`. `None` when there's no venv or no such key.
fn venv_python_version(venv: &Path) -> Option<String> {
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).ok()?;
    cfg.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "version_info")
        .map(|(_, value)| value.trim().to_string())
}

/// Delete the venv (and optionally the downloaded interpreters). The next
/// `ensure` rebuilds from scratch.
///
/// A `remove_dir_all` that fails partway (a loaded dylib on Windows, permissions
/// on Unix) leaves the directory present but partly emptied, and ensure()'s
/// "no interpreter yet" branch creates the venv without clearing first — so
/// recovery then depends on uv tolerating a non-empty target. Reporting the
/// error lets the user retry, which is the reliable path out.
pub async fn reset(app: &AppHandle, purge_runtimes: bool) -> Result<(), PythonEnvError> {
    // Same guard as ensure(), and it matters more here: paths::app_data falls
    // back to "." when app_data_dir() fails, so without this a reset would
    // recursively delete ./python-env relative to the process's working
    // directory. Writing to the wrong place is bad; deleting there is worse.
    if app.path().app_data_dir().is_err() {
        return Err(PythonEnvError::Io(
            "無法取得應用程式資料目錄，請確認磁碟權限".to_string(),
        ));
    }

    let _guard = ENSURE_LOCK.lock().await;
    let venv = paths::venv_dir(app);
    let runtimes = paths::runtime_dir(app);
    for dir in dirs_to_purge(&venv, &runtimes, purge_runtimes) {
        remove_if_present(dir).await?;
    }
    Ok(())
}

/// Directories a reset removes. Split out so the purge_runtimes branch is
/// testable without an AppHandle.
fn dirs_to_purge<'a>(venv: &'a Path, runtimes: &'a Path, purge_runtimes: bool) -> Vec<&'a Path> {
    if purge_runtimes {
        vec![venv, runtimes]
    } else {
        vec![venv]
    }
}

async fn remove_if_present(dir: &Path) -> Result<(), PythonEnvError> {
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PythonEnvError::Io(format!("刪除 {} 失敗：{e}", dir.display()))),
    }
}

/// `tools/<dir>/<file>` in dev, the resource bundle in production.
fn requirements_path(app: &AppHandle, profile: Profile) -> PathBuf {
    let dev_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .join("tools");
    let resource_root = app.path().resource_dir().ok();
    resolve_requirements(&dev_root, resource_root.as_deref(), profile)
}

/// Split from `requirements_path` so the dev-first, resource-fallback choice is
/// testable without an AppHandle.
fn resolve_requirements(dev_root: &Path, resource_root: Option<&Path>, profile: Profile) -> PathBuf {
    let dev = dev_root.join(profile.tool_dir()).join(profile.requirements_file());
    if dev.exists() {
        return dev;
    }
    resource_root
        .map(|r| r.join(profile.tool_dir()).join(profile.requirements_file()))
        .unwrap_or(dev)
}

/// Whether the venv's interpreter runs at all.
///
/// Deliberately shallow: it does not touch site-packages, so a half-written
/// package left by an interrupted install still passes. That case has no
/// automatic recovery — the marker hash is unchanged, so nothing re-installs —
/// and the user's way out is the settings page's rebuild action (Task 12).
async fn interpreter_works(python: &Path) -> bool {
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg("-c").arg("import sys");
    no_window(&mut cmd);
    matches!(cmd.status().await, Ok(s) if s.success())
}

/// Why a uv invocation didn't succeed. The distinction matters: a binary that
/// can't be launched needs the setup script or a quarantine fix, while a
/// process that ran and failed needs its own output read.
enum RunFailure {
    NotExecutable(String),
    Failed(String),
}

/// Run a spec, streaming both streams to the log panel. On failure returns the
/// combined output so the caller can classify it.
async fn run(app: &AppHandle, spec: CommandSpec) -> Result<(), RunFailure> {
    let mut cmd = tokio::process::Command::new(&spec.program);
    cmd.args(&spec.args)
        .envs(&spec.env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    no_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| RunFailure::NotExecutable(e.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| RunFailure::Failed("stdout not piped".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| RunFailure::Failed("stderr not piped".to_string()))?;

    // stderr on its own task, matching api_docs/runner.rs:133. A `select!` over
    // both readers would break out of the loop on whichever stream ended first
    // and lose the other's remaining output — which is exactly the output that
    // explains a failure, since uv reports errors on stderr.
    //
    // Emitting inside this loop (rather than collecting and emitting once the
    // task finishes) matters for a progress panel: uv writes warnings to
    // stderr throughout the run, and batching them to the end would show them
    // after a later "installation complete" stdout line, misrepresenting when
    // they actually happened.
    let app_for_stderr = app.clone();
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            emit_log(&app_for_stderr, "warn", &line);
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    let mut collected = String::new();
    let mut out_lines = tokio::io::BufReader::new(stdout).lines();
    while let Some(line) = out_lines
        .next_line()
        .await
        .map_err(|e| RunFailure::Failed(e.to_string()))?
    {
        collected.push_str(&line);
        collected.push('\n');
        emit_log(app, "info", &line);
    }

    let stderr_output = stderr_task.await.unwrap_or_default();

    let status = child
        .wait()
        .await
        .map_err(|e| RunFailure::Failed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        collected.push_str(&stderr_output);
        Err(RunFailure::Failed(collected))
    }
}

/// Last lines of a failure, for showing the user. 40 rather than 20: a compile
/// failure's decisive line ("command 'cc' failed") often sits above the trailing
/// compiler noise.
fn tail(output: &str) -> String {
    let lines: Vec<&str> = output.lines().collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

fn emit_log(app: &AppHandle, level: &str, message: &str) {
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
    fn missing_uv_message_tells_users_what_to_do_and_still_hints_developers() {
        // This message reaches end users too (a broken package build), who can't
        // run a setup script and shouldn't be told to — so it must lead with an
        // action they *can* take (reinstall), while still keeping the
        // "setup-uv" hint for the from-source / contributor case.
        let msg = PythonEnvError::UvMissing.to_string();
        assert!(msg.contains("重新安裝"), "should tell the end user what to do: {msg}");
        assert!(msg.contains("setup-uv"), "should still hint at the setup script for contributors: {msg}");
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
        assert!(looks_like_compile_failure("error: command 'gcc' failed with exit code 1"));
        assert!(looks_like_compile_failure("error: linker `cc` not found"));
        assert!(!looks_like_compile_failure("ERROR: No matching distribution found"));
    }

    #[test]
    fn an_unlaunchable_uv_is_reported_as_unusable_not_as_a_venv_failure() {
        // Permission or quarantine problems need the same remedy as a missing
        // binary; folding them into VenvFailed hides that.
        let msg = PythonEnvError::UvUnusable("Permission denied".into()).to_string();
        assert!(msg.contains("uv"));
        assert!(msg.contains("Permission denied"));
    }

    #[test]
    fn tail_keeps_short_output_intact() {
        assert_eq!(tail("one\ntwo"), "one\ntwo");
    }

    #[test]
    fn tail_keeps_the_last_lines_in_order() {
        let long: String = (1..=50).map(|n| format!("line{n}\n")).collect();
        let kept = tail(&long);
        assert!(kept.starts_with("line11"));
        assert!(kept.ends_with("line50"));
        assert_eq!(kept.lines().count(), 40);
    }

    #[test]
    fn requirements_prefer_the_dev_tree_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("MarkItDown");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("requirements.txt"), b"markitdown\n").unwrap();

        let found = resolve_requirements(dir.path(), Some(Path::new("/resources")), Profile::DocCore);
        assert!(found.starts_with(dir.path()));
    }

    #[test]
    fn requirements_fall_back_to_the_resource_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let found = resolve_requirements(dir.path(), Some(Path::new("/resources")), Profile::DocCore);
        assert_eq!(found, Path::new("/resources/MarkItDown/requirements.txt"));
    }

    #[test]
    fn env_status_serializes_to_the_field_names_the_frontend_reads() {
        // src/ipc/pythonEnv.ts hand-writes this shape, and nothing checks the two
        // stay aligned — renaming a field here would compile fine and leave the
        // settings page reading undefined. Pin the wire names.
        let json = serde_json::to_value(EnvStatus {
            uv_available: true,
            python_version: Some("3.12.13".into()),
            installed: vec![Profile::DocCore],
            venv_path: "/data/python-env".into(),
            user_interpreter: None,
        })
        .unwrap();

        assert_eq!(json["uvAvailable"], serde_json::json!(true));
        assert_eq!(json["pythonVersion"], serde_json::json!("3.12.13"));
        assert_eq!(json["installed"], serde_json::json!(["doc_core"]));
        assert_eq!(json["venvPath"], serde_json::json!("/data/python-env"));
        assert_eq!(json["userInterpreter"], serde_json::json!(null));
    }

    #[test]
    fn reads_the_python_version_from_pyvenv_cfg() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyvenv.cfg"),
            "home = /opt/homebrew/opt/python@3.12/bin\nimplementation = CPython\nversion_info = 3.12.13\n",
        )
        .unwrap();
        assert_eq!(venv_python_version(dir.path()).as_deref(), Some("3.12.13"));
    }

    #[test]
    fn version_is_none_without_a_venv() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(venv_python_version(dir.path()), None);
    }

    #[test]
    fn version_is_none_when_pyvenv_cfg_has_no_version_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyvenv.cfg"), "implementation = CPython\n").unwrap();
        assert_eq!(venv_python_version(dir.path()), None);
    }

    #[tokio::test]
    async fn removing_a_missing_directory_is_not_an_error() {
        // reset() runs on a fresh install too, where neither directory exists.
        let dir = tempfile::tempdir().unwrap();
        assert!(remove_if_present(&dir.path().join("never-existed")).await.is_ok());
    }

    #[tokio::test]
    async fn removing_an_existing_directory_deletes_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("python-env");
        std::fs::create_dir_all(target.join("bin")).unwrap();

        remove_if_present(&target).await.unwrap();

        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn removing_a_locked_directory_reports_the_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("python-env");
        std::fs::create_dir_all(target.join("bin")).unwrap();
        // Clearing write permission on the parent makes the child undeletable.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o500)).unwrap();

        let err = remove_if_present(&target).await.unwrap_err();
        assert!(err.to_string().contains("python-env"), "message should name the path: {err}");

        // Restore so tempdir cleanup can run.
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn a_rebuild_keeps_the_downloaded_runtimes() {
        let dirs = dirs_to_purge(Path::new("/env"), Path::new("/rt"), false);
        assert_eq!(dirs, vec![Path::new("/env")]);
    }

    #[test]
    fn a_full_delete_removes_the_runtimes_too() {
        let dirs = dirs_to_purge(Path::new("/env"), Path::new("/rt"), true);
        assert_eq!(dirs, vec![Path::new("/env"), Path::new("/rt")]);
    }
}

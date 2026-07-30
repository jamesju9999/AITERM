//! Where the managed Python environment lives.
//!
//! Tauri drops `externalBin` next to the executable and strips the
//! target-triple suffix when bundling, but leaves it in place during `tauri
//! dev`. Rather than reconstruct the triple (Rust exposes no constant for it,
//! and adding a build script just for this isn't worth it), the dev lookup
//! scans for a `uv-*` entry.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const UV_STEM: &str = "uv";

/// The venv the app manages. Deleting this directory is always safe.
pub fn venv_dir(app: &AppHandle) -> PathBuf {
    app_data(app).join("python-env")
}

/// Where uv installs interpreters. Kept under app data (rather than uv's
/// default `~/.local/share/uv`) so uninstalling the app can clean it up.
pub fn runtime_dir(app: &AppHandle) -> PathBuf {
    app_data(app).join("python-runtimes")
}

/// The interpreter inside a venv.
pub fn venv_interpreter(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

/// Locate the bundled uv binary, or `None` if it wasn't shipped.
pub fn uv_binary() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let plain = exe_dir.join(exe_name(UV_STEM));
    if plain.exists() {
        return Some(plain);
    }
    // `tauri dev` and local `cargo run` both leave the suffixed name in place.
    if let Some(found) = find_suffixed_uv(&exe_dir) {
        return Some(found);
    }
    let dev_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
    find_suffixed_uv(&dev_dir)
}

/// First `uv-<triple>` entry in `dir`, if any.
fn find_suffixed_uv(dir: &Path) -> Option<PathBuf> {
    let prefix = format!("{UV_STEM}-");
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() }
}

fn app_data(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn venv_interpreter_uses_the_platform_layout() {
        let venv = std::path::Path::new("/tmp/python-env");
        let py = venv_interpreter(venv);
        if cfg!(windows) {
            assert!(py.ends_with("Scripts/python.exe") || py.ends_with("Scripts\\python.exe"));
        } else {
            assert!(py.ends_with("bin/python"));
        }
    }

    #[test]
    fn finds_a_triple_suffixed_uv_in_a_dev_binaries_dir() {
        let dir = tempdir().unwrap();
        let name = if cfg!(windows) { "uv-x86_64-pc-windows-msvc.exe" } else { "uv-aarch64-apple-darwin" };
        std::fs::write(dir.path().join(name), b"").unwrap();

        let found = find_suffixed_uv(dir.path()).expect("should find the suffixed binary");
        assert_eq!(found.file_name().unwrap().to_str().unwrap(), name);
    }

    #[test]
    fn ignores_unrelated_files_when_looking_for_uv() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("db2sidecar.jar"), b"").unwrap();
        assert!(find_suffixed_uv(dir.path()).is_none());
    }
}

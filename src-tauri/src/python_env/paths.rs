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

const VENV_NAME: &str = "python-env";
const RUNTIME_NAME: &str = "python-runtimes";

/// The venv the app manages. Safe to delete — but only once the caller has
/// confirmed app_local_data_dir() resolved, since this falls back to "."
/// otherwise.
pub fn venv_dir(app: &AppHandle) -> PathBuf {
    local_root(app).join(VENV_NAME)
}

/// Where uv installs interpreters. Kept under app data (rather than uv's
/// default `~/.local/share/uv`) so uninstalling the app can clean it up.
pub fn runtime_dir(app: &AppHandle) -> PathBuf {
    local_root(app).join(RUNTIME_NAME)
}

/// Where 1.3.0 and 1.3.1 kept both directories.
///
/// On macOS and Linux this resolves to the same path as [`local_root`],
/// which is what makes [`migrate_legacy_layout`] a no-op there. On Windows it
/// is `%APPDATA%` — the roaming profile, which in a managed domain is copied
/// to the server on every logon and logoff. Several hundred megabytes of venv
/// and interpreter is exactly what must not be in there, and it could never be
/// useful roamed: it is full of native binaries built for one machine.
pub fn legacy_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Move a pre-1.3.2 environment out of the roaming profile.
///
/// The interpreters are relocatable by design (python-build-standalone builds
/// resolve their own files relative to the executable), so they are moved and
/// the user is spared re-downloading ~60 MB.
///
/// The venv cannot come with them: its `pyvenv.cfg` records `home = <absolute
/// path>` pointing at the interpreter in the *old* runtime directory, so it is
/// broken the moment the runtimes move. Deleting it is what makes the next
/// `ensure()` rebuild it — cheaply, since uv's wheel cache is untouched by any
/// of this.
///
/// Only those two directories are touched. The parent is left alone whether or
/// not it ends up empty: it is not ours, and on Windows it neighbours the
/// config store.
pub fn migrate_legacy_layout(old_root: &Path, new_root: &Path) {
    if old_root == new_root {
        return; // macOS, Linux, and any Windows install already migrated.
    }

    let old_runtimes = old_root.join(RUNTIME_NAME);
    let new_runtimes = new_root.join(RUNTIME_NAME);
    if old_runtimes.is_dir() && !new_runtimes.exists() {
        if let Some(parent) = new_runtimes.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Both roots live under the same user profile, so this is a rename on
        // one volume: atomic, and instant regardless of size.
        match std::fs::rename(&old_runtimes, &new_runtimes) {
            Ok(()) => log::info!("moved python runtimes out of the roaming profile"),
            Err(e) => log::warn!("could not move python runtimes ({e}); removing them instead"),
        }
    }

    // Anything still in the old location is now unreachable — either the move
    // failed, or a newer install already created its own copy. Leaving it would
    // strand hundreds of megabytes in a directory the user never opens.
    for stale in [old_root.join(VENV_NAME), old_runtimes] {
        if stale.exists() {
            if let Err(e) = std::fs::remove_dir_all(&stale) {
                log::warn!("could not remove {}: {e}", stale.display());
            }
        }
    }
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
///
/// A dev machine can accumulate binaries for several triples, and `read_dir`
/// order is platform- and filesystem-dependent — so prefer the one whose
/// triple carries this machine's architecture (picking another would fail at
/// exec time with an error that says nothing about where it came from), and
/// sort so the remaining case is at least deterministic.
fn find_suffixed_uv(dir: &Path) -> Option<PathBuf> {
    let prefix = format!("{UV_STEM}-");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    candidates.sort();
    candidates
        .iter()
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(std::env::consts::ARCH))
        })
        .cloned()
        .or_else(|| candidates.pop())
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() }
}

/// Root of everything this module manages. Paired with [`legacy_root`].
pub fn local_root(app: &AppHandle) -> PathBuf {
    app.path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Builds a pre-1.3.2 layout: both directories under `root`, each holding
    /// one file so a directory that merely exists can be told from one whose
    /// contents survived.
    fn legacy_layout(root: &Path) {
        std::fs::create_dir_all(root.join(RUNTIME_NAME).join("cpython-3.12.13")).unwrap();
        std::fs::write(
            root.join(RUNTIME_NAME).join("cpython-3.12.13").join("marker"),
            b"runtime",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(VENV_NAME)).unwrap();
        std::fs::write(root.join(VENV_NAME).join("pyvenv.cfg"), b"version_info = 3.12.13\n").unwrap();
    }

    #[test]
    fn the_two_roots_differ_only_on_windows() {
        // Load-bearing for everything below. Tauri derives app_data_dir from
        // dirs::data_dir and app_local_data_dir from dirs::data_local_dir, and
        // the whole migration rests on those being one directory outside
        // Windows — if that ever stopped holding, macOS and Linux would start
        // deleting the environment they are actively using. Asserted rather
        // than assumed, because being wrong here destroys user data silently.
        let roaming = dirs::data_dir();
        let local = dirs::data_local_dir();
        if cfg!(windows) {
            assert_ne!(roaming, local, "the move is pointless if these are the same");
        } else {
            assert_eq!(roaming, local, "migration must stay a no-op here");
        }
    }

    #[test]
    fn migration_does_nothing_when_both_roots_are_the_same_directory() {
        // macOS and Linux: app_data_dir and app_local_data_dir resolve to one
        // path, so the "old" location is the live one. Touching it would delete
        // a working environment on every launch.
        let dir = tempdir().unwrap();
        legacy_layout(dir.path());

        migrate_legacy_layout(dir.path(), dir.path());

        assert!(dir.path().join(VENV_NAME).join("pyvenv.cfg").exists());
        assert!(dir.path().join(RUNTIME_NAME).join("cpython-3.12.13").join("marker").exists());
    }

    #[test]
    fn the_runtimes_are_moved_with_their_contents_intact() {
        // Moving rather than re-downloading is the whole point: python-build-
        // standalone installs are relocatable, and this saves ~60 MB.
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        legacy_layout(old.path());

        migrate_legacy_layout(old.path(), new.path());

        let moved = new.path().join(RUNTIME_NAME).join("cpython-3.12.13").join("marker");
        assert_eq!(std::fs::read(&moved).unwrap(), b"runtime");
        assert!(!old.path().join(RUNTIME_NAME).exists(), "nothing should be left behind");
    }

    #[test]
    fn the_old_venv_is_deleted_rather_than_moved() {
        // Its pyvenv.cfg records an absolute `home =` pointing into the old
        // runtime directory, so carrying it across would produce a venv whose
        // interpreter reference is already broken.
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        legacy_layout(old.path());

        migrate_legacy_layout(old.path(), new.path());

        assert!(!old.path().join(VENV_NAME).exists());
        assert!(!new.path().join(VENV_NAME).exists(), "it must not be carried over either");
    }

    #[test]
    fn an_already_migrated_install_keeps_its_runtimes_and_loses_the_stale_copy() {
        // Second run, or a machine where the new location was populated first:
        // the live directory must win, and the roaming copy must not be left to
        // sit there.
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        legacy_layout(old.path());
        std::fs::create_dir_all(new.path().join(RUNTIME_NAME)).unwrap();
        std::fs::write(new.path().join(RUNTIME_NAME).join("keep"), b"live").unwrap();

        migrate_legacy_layout(old.path(), new.path());

        assert_eq!(std::fs::read(new.path().join(RUNTIME_NAME).join("keep")).unwrap(), b"live");
        assert!(!old.path().join(RUNTIME_NAME).exists(), "the stale copy must be reclaimed");
    }

    #[test]
    fn migration_leaves_the_old_parent_and_anything_else_in_it_alone() {
        // On Windows this directory neighbours the config store. Only the two
        // directories this module created are ours to remove.
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();
        legacy_layout(old.path());
        std::fs::write(old.path().join("config.json"), b"{}").unwrap();

        migrate_legacy_layout(old.path(), new.path());

        assert!(old.path().exists(), "the parent itself must survive");
        assert_eq!(std::fs::read(old.path().join("config.json")).unwrap(), b"{}");
    }

    #[test]
    fn migration_on_a_machine_that_never_had_the_old_layout_is_harmless() {
        // The overwhelmingly common case, run on every ensure().
        let old = tempdir().unwrap();
        let new = tempdir().unwrap();

        migrate_legacy_layout(old.path(), new.path());

        assert!(!new.path().join(RUNTIME_NAME).exists());
        assert!(!new.path().join(VENV_NAME).exists());
    }

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

    #[test]
    fn prefers_the_binary_matching_this_machines_architecture() {
        let dir = tempdir().unwrap();
        // Both are plausible on a dev machine that ran the setup script under
        // different architectures; picking the wrong one fails at exec time.
        let other = if std::env::consts::ARCH == "aarch64" {
            "uv-x86_64-apple-darwin"
        } else {
            "uv-aarch64-apple-darwin"
        };
        std::fs::write(dir.path().join(other), b"").unwrap();
        let mine = format!("uv-{}-some-target", std::env::consts::ARCH);
        std::fs::write(dir.path().join(&mine), b"").unwrap();

        let found = find_suffixed_uv(dir.path()).expect("should find a binary");
        assert_eq!(found.file_name().unwrap().to_str().unwrap(), mine);
    }

    #[test]
    fn ignores_directories_that_look_like_the_binary() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("uv-aarch64-apple-darwin")).unwrap();
        assert!(find_suffixed_uv(dir.path()).is_none());
    }
}

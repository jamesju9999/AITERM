//! Undoes the environment an AppImage injects into every child process.
//!
//! Linux AppImages run through linuxdeploy's `AppRun.wrapped` (AppImageKit's
//! AppRun), which exports, among others:
//!
//! ```text
//! PYTHONHOME=$APPDIR/usr/
//! PYTHONPATH=$APPDIR/usr/share/pyshared/:$PYTHONPATH
//! LD_LIBRARY_PATH=$APPDIR/usr/lib/:…:$LD_LIBRARY_PATH
//! ```
//!
//! Those exist for AppImages that bundle their own Python. Ours doesn't: the
//! AppDir has no `lib/python3.x` at all, so a Python we spawn inherits a
//! `PYTHONHOME` pointing at a directory with no standard library and dies
//! during interpreter startup:
//!
//! ```text
//! Fatal Python error: init_fs_encoding: failed to get the Python codec of the filesystem encoding
//! ModuleNotFoundError: No module named 'encodings'
//! ```
//!
//! That is the whole of the v1.3.0 Ubuntu failure: `uv venv` reported success
//! (it only writes files and symlinks), and every interpreter spawned from that
//! venv then failed identically, so rebuilding could never help.
//!
//! `LD_LIBRARY_PATH` is stripped for the same class of reason rather than as a
//! precaution: the AppDir ships `libssl.so.3`, `libcrypto.so.3`,
//! `libsqlite3.so.0` and `libffi.so.8` built against the Ubuntu 22.04 base, and
//! AppRun puts that directory *ahead* of the system's — so native wheels would
//! resolve against build-base-era libraries instead of the host's. This
//! codebase has already been bitten by exactly this precedence once, in the
//! bundled-libwayland workaround in `release.yml`.
//!
//! Only entries that point into `$APPDIR` are removed. A user's own
//! `LD_LIBRARY_PATH` survives — clobbering it would trade this bug for a
//! different one.

/// Environment changes that undo AppRun's injection. `None` means "unset".
///
/// Split from the process-spawning wrapper so the whole policy is testable on
/// macOS and Windows, where no AppImage exists to reproduce against.
fn fixes(
    appdir: Option<&str>,
    get: impl Fn(&str) -> Option<String>,
) -> Vec<(&'static str, Option<String>)> {
    // No APPDIR means no AppImage: a .deb install, `tauri dev`, macOS, Windows.
    // Every one of those has an environment we have no business touching.
    let Some(appdir) = appdir.filter(|d| !d.is_empty()) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    // PYTHONHOME is all-or-nothing — there is no "partly wrong" home directory,
    // and any value inside the AppDir is wrong for an interpreter outside it.
    if get("PYTHONHOME").is_some_and(|v| v.starts_with(appdir)) {
        out.push(("PYTHONHOME", None));
    }

    // These two are path lists, so drop only the AppDir entries.
    for var in ["PYTHONPATH", "LD_LIBRARY_PATH"] {
        let Some(value) = get(var) else { continue };
        let entries: Vec<&str> = value.split(':').filter(|e| !e.is_empty()).collect();
        let kept: Vec<&str> = entries
            .iter()
            .copied()
            .filter(|e| !e.starts_with(appdir))
            .collect();
        if kept.len() == entries.len() {
            continue; // Nothing of ours in there; leave it exactly as it is.
        }
        out.push((var, (!kept.is_empty()).then(|| kept.join(":"))));
    }

    out
}

/// The changes this process's environment needs before it is handed to a child.
/// Empty outside an AppImage.
///
/// Exposed for callers that don't build a `std::process::Command` —
/// `portable_pty::CommandBuilder` is the one that matters — so they can apply
/// the same policy through their own API instead of this module growing a
/// dependency on theirs.
pub fn appimage_env_fixes() -> Vec<(&'static str, Option<String>)> {
    fixes(std::env::var("APPDIR").ok().as_deref(), |k| {
        std::env::var(k).ok()
    })
}

/// Apply [`appimage_env_fixes`] to a command that is about to run Python (or
/// uv, which runs the interpreter itself to read its metadata during `uv pip
/// install`).
///
/// Takes `std::process::Command` so tokio callers can pass `cmd.as_std_mut()`
/// and share one implementation.
pub fn strip_appimage_env(cmd: &mut std::process::Command) {
    for (key, value) in appimage_env_fixes() {
        match value {
            Some(v) => cmd.env(key, v),
            None => cmd.env_remove(key),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `get` closure from a fixed list, so tests never touch the real
    /// process environment (which is shared, and mutating it races other tests).
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn nothing_is_changed_outside_an_appimage() {
        // .deb, tauri dev, macOS, Windows: APPDIR is unset and the environment
        // belongs entirely to the user.
        let changes = fixes(
            None,
            env(&[("PYTHONHOME", "/opt/py"), ("LD_LIBRARY_PATH", "/opt/lib")]),
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn an_empty_appdir_is_treated_as_no_appimage() {
        // Guards against `APPDIR=""`, where a `starts_with` test would match
        // every path and wipe the user's whole environment.
        let changes = fixes(Some(""), env(&[("LD_LIBRARY_PATH", "/opt/lib")]));
        assert!(changes.is_empty());
    }

    #[test]
    fn the_injected_pythonhome_is_unset() {
        // The actual v1.3.0 Ubuntu failure: this one variable is what made a
        // freshly created venv unable to start.
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[("PYTHONHOME", "/tmp/.mount_AITermX/usr/")]),
        );
        assert_eq!(changes, vec![("PYTHONHOME", None)]);
    }

    #[test]
    fn a_pythonhome_the_user_set_themselves_is_left_alone() {
        // Only AppRun's value is ours to remove.
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[("PYTHONHOME", "/opt/python3.12")]),
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn appdir_entries_are_dropped_from_ld_library_path_and_the_rest_kept() {
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[(
                "LD_LIBRARY_PATH",
                "/tmp/.mount_AITermX/usr/lib/:/tmp/.mount_AITermX/usr/lib64/:/opt/oracle/lib",
            )]),
        );
        assert_eq!(
            changes,
            vec![("LD_LIBRARY_PATH", Some("/opt/oracle/lib".to_string()))]
        );
    }

    #[test]
    fn a_path_list_that_is_entirely_appdir_is_unset_rather_than_left_empty() {
        // Setting it to "" is not the same as unsetting: an empty entry in
        // LD_LIBRARY_PATH means "the current directory" to the dynamic loader.
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[("LD_LIBRARY_PATH", "/tmp/.mount_AITermX/usr/lib/")]),
        );
        assert_eq!(changes, vec![("LD_LIBRARY_PATH", None)]);
    }

    #[test]
    fn a_path_list_with_no_appdir_entries_is_not_touched_at_all() {
        // Not even rewritten to an equivalent value — a no-op must stay a no-op,
        // or the diff between "we changed nothing" and "we normalised it"
        // disappears.
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[("LD_LIBRARY_PATH", "/opt/oracle/lib:/usr/local/lib")]),
        );
        assert!(changes.is_empty());
    }

    #[test]
    fn the_injected_pythonpath_is_filtered_the_same_way() {
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[(
                "PYTHONPATH",
                "/tmp/.mount_AITermX/usr/share/pyshared/:/home/me/lib",
            )]),
        );
        assert_eq!(
            changes,
            vec![("PYTHONPATH", Some("/home/me/lib".to_string()))]
        );
    }

    #[test]
    fn all_three_injected_variables_are_handled_in_one_pass() {
        // AppRun sets all of them together, so the real case is all three at
        // once — not one at a time as the focused tests above check.
        let appdir = "/tmp/.mount_AITermX";
        let changes = fixes(
            Some(appdir),
            env(&[
                ("PYTHONHOME", "/tmp/.mount_AITermX/usr/"),
                ("PYTHONPATH", "/tmp/.mount_AITermX/usr/share/pyshared/"),
                ("LD_LIBRARY_PATH", "/tmp/.mount_AITermX/usr/lib/:/usr/lib"),
            ]),
        );
        assert_eq!(
            changes,
            vec![
                ("PYTHONHOME", None),
                ("PYTHONPATH", None),
                ("LD_LIBRARY_PATH", Some("/usr/lib".to_string())),
            ]
        );
    }

    #[test]
    fn empty_entries_do_not_survive_as_the_current_directory() {
        // "a::b" already means "a:.:b" to the loader; our filter drops the empty
        // entry, which is a small hardening rather than a behaviour change.
        let changes = fixes(
            Some("/tmp/.mount_AITermX"),
            env(&[("LD_LIBRARY_PATH", "/tmp/.mount_AITermX/usr/lib/::/opt/lib")]),
        );
        assert_eq!(changes, vec![("LD_LIBRARY_PATH", Some("/opt/lib".to_string()))]);
    }
}

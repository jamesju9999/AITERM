//! Proves `strip_appimage_env` actually changes what a spawned child sees.
//!
//! The unit tests in `python_env::appimage` cover the policy — which variables
//! to drop and which to keep — but they call a pure function and so cannot tell
//! a correct list of changes from a list that never reaches a process. This
//! file closes that gap by spawning something and reading its environment back.
//!
//! It lives in its own integration-test binary because it mutates the process
//! environment, which is global while cargo runs unit tests across threads.
//! One test per binary makes that safe. `#![cfg(unix)]`: AppImages are
//! Linux-only, and the fake interpreter below is a shell script.
#![cfg(unix)]

use aiterm_lib::appimage_env::strip_appimage_env;
use std::os::unix::fs::PermissionsExt;

#[test]
fn a_spawned_child_does_not_inherit_apprun_s_pythonhome() {
    let dir = tempfile::tempdir().unwrap();
    let appdir = dir.path().to_str().unwrap().to_string();

    let script = dir.path().join("fake-python");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"HOME=${PYTHONHOME:-unset} LD=${LD_LIBRARY_PATH:-unset}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Exactly what linuxdeploy's AppRun exports, with one entry of the user's
    // own on LD_LIBRARY_PATH so the test can tell "stripped" from "wiped".
    std::env::set_var("APPDIR", &appdir);
    std::env::set_var("PYTHONHOME", format!("{appdir}/usr/"));
    std::env::set_var("LD_LIBRARY_PATH", format!("{appdir}/usr/lib/:/opt/keepme"));

    // Baseline first: without it, a test asserting "the child sees no
    // PYTHONHOME" would also pass if the variable had never been set at all.
    let before = std::process::Command::new(&script).output().unwrap();
    let before = String::from_utf8(before.stdout).unwrap();
    assert!(
        before.contains(&format!("HOME={appdir}/usr/")),
        "the child should inherit the injected value before the fix: {before}"
    );

    let mut cmd = std::process::Command::new(&script);
    strip_appimage_env(&mut cmd);
    let after = String::from_utf8(cmd.output().unwrap().stdout).unwrap();

    assert!(
        after.contains("HOME=unset"),
        "PYTHONHOME must not reach the interpreter: {after}"
    );
    assert!(
        after.contains("LD=/opt/keepme"),
        "the user's own LD_LIBRARY_PATH entry must survive: {after}"
    );
}

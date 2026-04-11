//! Integration test for e2e cwd tracking: spawn a real shell, send `cd`
//! commands, verify `PtyManager::get_cwd` matches the shell's actual cwd.
//!
//! This is the "真金火煉" test mentioned in spec §8.2 — unit tests only cover
//! the string parsing; this verifies the write-hook actually fires for a live
//! shell on this OS.

use aiterm_lib::pty::PtyManager;
use portable_pty::PtySize;
use std::sync::mpsc;
use std::time::Duration;

fn small_size() -> PtySize {
    PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }
}

#[test]
fn tracks_cd_through_real_shell() {
    let manager = PtyManager::new();
    let (tx, _rx) = mpsc::channel::<Vec<u8>>();

    let id = manager
        .create_with_callback(small_size(), move |chunk| {
            let _ = tx.send(chunk);
        })
        .expect("create session");

    // Give the shell time to finish printing its banner/prompt before we
    // start sending commands. On Windows conpty this is flaky under 200 ms.
    std::thread::sleep(Duration::from_millis(500));

    let initial = manager.get_cwd(&id).expect("initial cwd");

    // Platform-specific: cmd.exe on Windows, sh on Unix. default_shell() in
    // M0 prefers pwsh → powershell → cmd on Windows. Our parser handles all
    // three, so we just send a parent-dir change that works everywhere.
    #[cfg(windows)]
    manager.write(&id, b"cd ..\r\n").unwrap();
    #[cfg(not(windows))]
    manager.write(&id, b"cd ..\n").unwrap();

    // Allow the write hook to process.
    std::thread::sleep(Duration::from_millis(100));

    let after = manager.get_cwd(&id).expect("after cwd");
    assert_ne!(after, initial, "cwd should have changed after `cd ..`");
    assert_eq!(
        after,
        initial.parent().expect("initial had a parent").to_path_buf(),
        "cwd should now be the parent of the initial cwd"
    );

    // Now cd back into a child and verify.
    let child_name = initial
        .file_name()
        .expect("initial had a file name")
        .to_string_lossy()
        .to_string();
    #[cfg(windows)]
    manager
        .write(&id, format!("cd {}\r\n", child_name).as_bytes())
        .unwrap();
    #[cfg(not(windows))]
    manager
        .write(&id, format!("cd {}\n", child_name).as_bytes())
        .unwrap();

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(manager.get_cwd(&id).unwrap(), initial);

    manager.close(&id).ok();
}

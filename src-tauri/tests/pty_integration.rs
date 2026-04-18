use std::sync::mpsc;
use std::time::Duration;

use portable_pty::PtySize;

use aiterm_lib::pty::manager::PtyManager;

#[test]
fn full_lifecycle_create_write_read_close() {
    let manager = PtyManager::new();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    let id = manager
        .create_with_callback(
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("create session");

    #[cfg(windows)]
    manager.write(&id, b"echo INTEGRATION_OK\r\nexit\r\n").unwrap();
    #[cfg(not(windows))]
    manager.write(&id, b"echo INTEGRATION_OK\nexit\n").unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut buffer = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => buffer.extend_from_slice(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if String::from_utf8_lossy(&buffer).contains("INTEGRATION_OK") {
            break;
        }
    }

    assert!(
        String::from_utf8_lossy(&buffer).contains("INTEGRATION_OK"),
        "expected integration marker, got: {}",
        String::from_utf8_lossy(&buffer)
    );

    // close cleanly — if the shell already exited, close() will report
    // SessionNotFound or succeed depending on timing; accept either.
    let _ = manager.close(&id);
}

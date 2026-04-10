use std::io::{Read, Write};
use std::thread::{self, JoinHandle};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

use super::error::{PtyError, PtyResult};
use super::shell::ShellSpec;

pub struct PtySession {
    pub id: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    reader_thread: Mutex<Option<JoinHandle<()>>>,
}

impl PtySession {
    /// Spawn a new shell. `on_data` receives every chunk read from the PTY on a background thread.
    pub fn spawn<F>(shell: ShellSpec, size: PtySize, mut on_data: F) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell.program);
        for arg in shell.args {
            cmd.arg(arg);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;
        // Drop slave so that the child sees EOF on exit (avoid leaks on Windows).
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Internal(format!("take_writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Internal(format!("try_clone_reader: {e}")))?;

        let id = Uuid::new_v4().to_string();

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break, // EOF: shell exited
                        Ok(n) => on_data(buf[..n].to_vec()),
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
        })
    }

    /// Like `spawn`, but uses a caller-supplied id. Useful when the id must be
    /// known before the `on_data` closure is constructed (e.g. Tauri event names).
    pub fn spawn_with_id<F>(
        shell: ShellSpec,
        size: PtySize,
        id: String,
        mut on_data: F,
    ) -> PtyResult<Self>
    where
        F: FnMut(Vec<u8>) + Send + 'static,
    {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell.program);
        for arg in shell.args {
            cmd.arg(arg);
        }
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::SpawnFailed(e.to_string()))?;
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Internal(format!("take_writer: {e}")))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Internal(format!("try_clone_reader: {e}")))?;

        let reader_thread = thread::Builder::new()
            .name(format!("pty-reader-{}", id))
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => on_data(buf[..n].to_vec()),
                        Err(e) => {
                            eprintln!("pty reader error: {e}");
                            break;
                        }
                    }
                }
            })
            .map_err(|e| PtyError::Internal(format!("spawn reader thread: {e}")))?;

        Ok(Self {
            id,
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            reader_thread: Mutex::new(Some(reader_thread)),
        })
    }

    pub fn write(&self, data: &[u8]) -> PtyResult<()> {
        let mut writer = self.writer.lock();
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, size: PtySize) -> PtyResult<()> {
        let master = self.master.lock();
        master
            .resize(size)
            .map_err(|e| PtyError::Internal(format!("resize: {e}")))
    }

    pub fn kill(&self) -> PtyResult<()> {
        let mut child = self.child.lock();
        child
            .kill()
            .map_err(|e| PtyError::Internal(format!("kill: {e}")))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // Best-effort: kill child so the reader thread eventually sees EOF/error.
        let _ = self.child.lock().kill();
        // Do NOT join the reader thread here: on Windows conpty the reader can
        // block indefinitely even after the child exits.  The thread is
        // detached and will exit on its own once the OS cleans up the handle.
        let _ = self.reader_thread.lock().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn test_shell() -> ShellSpec {
        #[cfg(windows)]
        {
            ShellSpec {
                program: "cmd.exe".into(),
                args: vec!["/Q".into()], // no banner
            }
        }
        #[cfg(not(windows))]
        {
            ShellSpec {
                program: "/bin/sh".into(),
                args: vec![],
            }
        }
    }

    #[test]
    fn session_echoes_written_bytes() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        // Give the shell a moment to be ready, then send an echo and exit.
        #[cfg(windows)]
        session.write(b"echo HELLO_AITERM\r\nexit\r\n").unwrap();
        #[cfg(not(windows))]
        session.write(b"echo HELLO_AITERM\nexit\n").unwrap();

        // Collect until EOF or 5s timeout.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut buffer = Vec::new();
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if String::from_utf8_lossy(&buffer).contains("HELLO_AITERM") {
                break;
            }
        }

        let output = String::from_utf8_lossy(&buffer);
        assert!(
            output.contains("HELLO_AITERM"),
            "expected shell to echo marker, got: {output}"
        );

        drop(session);
    }

    #[test]
    fn session_resize_does_not_error() {
        let (tx, _rx) = mpsc::channel::<Vec<u8>>();
        let session = PtySession::spawn(
            test_shell(),
            PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 },
            move |chunk| {
                let _ = tx.send(chunk);
            },
        )
        .expect("spawn pty");

        session
            .resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .expect("resize ok");

        drop(session);
    }
}

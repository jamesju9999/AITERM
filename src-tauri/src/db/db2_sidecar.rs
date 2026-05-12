//! Manages the db2-sidecar child process and provides JSON line I/O.

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

struct SidecarIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct Db2SidecarClient {
    io: Mutex<SidecarIo>,
    _child: Mutex<Child>,
}

impl Db2SidecarClient {
    /// Spawn the sidecar. `sidecar_dir` must contain `db2sidecar.jar` and `jre/bin/java[.exe]`.
    /// Returns Err with "db2_sidecar_not_found:" prefix if the jar is missing.
    pub fn spawn(sidecar_dir: PathBuf) -> Result<Self> {
        #[cfg(target_os = "windows")]
        let java_bin = sidecar_dir.join("jre").join("bin").join("java.exe");
        #[cfg(not(target_os = "windows"))]
        let java_bin = sidecar_dir.join("jre").join("bin").join("java");

        let jar = sidecar_dir.join("db2sidecar.jar");

        if !jar.exists() {
            return Err(anyhow::anyhow!(
                "db2_sidecar_not_found: {}",
                jar.display()
            ));
        }

        let mut cmd = tokio::process::Command::new(&java_bin);
        cmd.arg("-jar")
            .arg(&jar)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(&sidecar_dir);

        #[cfg(target_os = "windows")]
        {
            #[allow(unused_imports)]
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "db2_sidecar_not_found: java not found at {}",
                    java_bin.display()
                )
            } else {
                anyhow::anyhow!("failed to spawn db2-sidecar: {e}")
            }
        })?;

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));

        Ok(Self {
            io: Mutex::new(SidecarIo { stdin, stdout }),
            _child: Mutex::new(child),
        })
    }

    /// Send one JSON request, receive one JSON response.
    /// The Mutex ensures serial (one-at-a-time) request/response pairs.
    pub async fn send(&self, req: Value) -> Result<Value> {
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let mut io = self.io.lock().await;
        io.stdin.write_all(line.as_bytes()).await?;
        io.stdin.flush().await?;

        for _ in 0..8 {
            let mut resp_bytes = Vec::new();
            timeout(
                Duration::from_secs(20),
                io.stdout.read_until(b'\n', &mut resp_bytes),
            )
            .await
            .map_err(|_| anyhow::anyhow!("db2_sidecar_timeout: no response in 20s"))??;

            if resp_bytes.is_empty() {
                return Err(anyhow::anyhow!("db2_sidecar_died: EOF on stdout"));
            }

            let resp_line = match String::from_utf8(resp_bytes) {
                Ok(s) => s,
                Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
            };
            let cleaned = resp_line.trim().trim_start_matches('\u{feff}');
            if cleaned.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(cleaned) {
                return Ok(v);
            }
        }

        Err(anyhow::anyhow!(
            "db2_sidecar_invalid_json: did not receive parseable JSON line"
        ))
    }
}

pub struct Db2SidecarState {
    client: Mutex<Option<Arc<Db2SidecarClient>>>,
    sidecar_dir: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_dir: PathBuf) -> Self {
        Self {
            client: Mutex::new(None),
            sidecar_dir,
        }
    }

    /// Clears the cached client so the next `get_client` call re-spawns the sidecar.
    pub async fn reset(&self) {
        *self.client.lock().await = None;
    }

    /// Returns the shared sidecar client, spawning it on first call.
    pub async fn get_client(&self) -> Result<Arc<Db2SidecarClient>> {
        let mut guard = self.client.lock().await;
        if let Some(ref c) = *guard {
            return Ok(c.clone());
        }
        let c = Arc::new(Db2SidecarClient::spawn(self.sidecar_dir.clone())?);
        *guard = Some(c.clone());
        Ok(c)
    }
}

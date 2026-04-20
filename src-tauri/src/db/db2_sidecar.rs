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
    /// Spawn the sidecar process. Returns Err with "db2_sidecar_not_found:" prefix
    /// if the binary does not exist, so the frontend can show install guidance.
    pub fn spawn(path: PathBuf) -> Result<Self> {
        let sidecar_dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("db2_sidecar_invalid_path"))?;
        let mut cmd = tokio::process::Command::new(&path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(sidecar_dir);

        #[cfg(target_os = "windows")]
        {
            let clidriver = sidecar_dir.join("clidriver");
            if clidriver.exists() {
                cmd.env("DB2_CLI_DRIVER_INSTALL_PATH", &clidriver);
                let clidriver_bin = clidriver.join("bin");
                if clidriver_bin.exists() {
                    let old_path = std::env::var_os("PATH").unwrap_or_default();
                    let mut new_path = std::ffi::OsString::new();
                    new_path.push(clidriver_bin);
                    new_path.push(";");
                    new_path.push(old_path);
                    cmd.env("PATH", new_path);
                }
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("db2_sidecar_not_found: {}", path.display())
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
    sidecar_path: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_path: PathBuf) -> Self {
        Self {
            client: Mutex::new(None),
            sidecar_path,
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
        let c = Arc::new(Db2SidecarClient::spawn(self.sidecar_path.clone())?);
        *guard = Some(c.clone());
        Ok(c)
    }
}

//! Manages the db2-sidecar child process and provides JSON line I/O.

use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

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
        let mut child = tokio::process::Command::new(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
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

        let mut resp_line = String::new();
        io.stdout.read_line(&mut resp_line).await?;

        Ok(serde_json::from_str(resp_line.trim())?)
    }
}

pub struct Db2SidecarState {
    client: Mutex<Option<Arc<Db2SidecarClient>>>,
    sidecar_path: PathBuf,
}

impl Db2SidecarState {
    pub fn new(sidecar_path: PathBuf) -> Self {
        Self { client: Mutex::new(None), sidecar_path }
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

// src-tauri/src/mcp/transport.rs

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Server returned JSON-RPC error {code}: {message}")]
    JsonRpc { code: i64, message: String },
    #[error("Process exited unexpectedly")]
    ProcessExited,
}

// ── stdio transport ────────────────────────────────────────────────────────────

pub struct StdioTransport {
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    child: Mutex<Child>,
    next_id: AtomicU64,
}

impl StdioTransport {
    /// Spawn a subprocess and establish a stdio transport.
    /// On Windows, wraps the command in `cmd /C` to handle `.cmd` scripts (npx, etc.).
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Self, TransportError> {
        let mut cmd = build_command(command, args, env);
        cmd.stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::null())
           .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");

        Ok(Self {
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            child: Mutex::new(child),
            next_id: AtomicU64::new(1),
        })
    }

    /// Send a JSON-RPC request and wait for the matching response.
    pub async fn send_request(
        &self,
        mut req: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, TransportError> {
        req.id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let line = serde_json::to_string(&req)? + "\n";

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin);

        let mut stdout = self.stdout.lock().await;
        let mut buf = String::new();
        let n = stdout.read_line(&mut buf).await?;
        if n == 0 {
            return Err(TransportError::ProcessExited);
        }
        let resp: JsonRpcResponse = serde_json::from_str(buf.trim())?;
        Ok(resp)
    }

    pub async fn kill(&self) {
        let _ = self.child.lock().await.kill().await;
    }
}

#[cfg(target_os = "windows")]
fn build_command(command: &str, args: &[String], env: &HashMap<String, String>) -> tokio::process::Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd.args(args);
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

#[cfg(target_os = "macos")]
fn build_command(command: &str, args: &[String], env: &HashMap<String, String>) -> tokio::process::Command {
    // macOS GUI apps don't inherit the user's shell PATH (e.g. Homebrew, nvm).
    // Wrap in a zsh login shell so /etc/paths and ~/.zprofile are sourced.
    let mut parts = vec![command.to_string()];
    parts.extend_from_slice(args);
    let shell_cmd = shell_words::join(&parts);

    let mut cmd = tokio::process::Command::new("/bin/zsh");
    cmd.args(["-l", "-c", &shell_cmd]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn build_command(command: &str, args: &[String], env: &HashMap<String, String>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args);
    // Only on this arm because AppImage is Linux-only. AppRun exports
    // PYTHONHOME into every child, which breaks any Python-based MCP server
    // (uvx, `python -m …`). Before the configured env, so an explicit setting
    // still wins.
    crate::appimage_env::strip_appimage_env(cmd.as_std_mut());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

// ── HTTP transport ─────────────────────────────────────────────────────────────

pub struct HttpTransport {
    base_url: String,
    client: reqwest::Client,
    next_id: AtomicU64,
}

impl HttpTransport {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn send_request(
        &self,
        mut req: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, TransportError> {
        req.id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let resp = self.client
            .post(&self.base_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| TransportError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Try to extract a human-readable message from JSON error bodies
            let detail = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v["error_description"].as_str()
                        .or(v["error"].as_str())
                        .or(v["message"].as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(TransportError::Http(if detail.is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}: {detail}")
            }));
        }

        let body: JsonRpcResponse = resp.json()
            .await
            .map_err(|e| TransportError::Http(e.to_string()))?;
        Ok(body)
    }
}

// ── Unified enum ──────────────────────────────────────────────────────────────

pub enum McpTransport {
    Stdio(Arc<StdioTransport>),
    Http(HttpTransport),
}

impl McpTransport {
    pub fn new_http(url: String) -> Self {
        Self::Http(HttpTransport::new(url))
    }

    pub async fn send_request(
        &mut self,
        req: JsonRpcRequest,
    ) -> Result<JsonRpcResponse, TransportError> {
        match self {
            McpTransport::Stdio(t) => t.send_request(req).await,
            McpTransport::Http(t) => t.send_request(req).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::JsonRpcRequest;

    #[tokio::test]
    async fn http_transport_sends_jsonrpc_request() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": { "tools": [] }
                }))
            )
            .mount(&server)
            .await;

        let mut transport = McpTransport::new_http(server.uri());
        let resp = transport.send_request(
            JsonRpcRequest::new(1, "tools/list", serde_json::json!({}))
        ).await.unwrap();
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }
}

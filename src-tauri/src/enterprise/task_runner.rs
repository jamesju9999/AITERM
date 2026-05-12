//! Task Runner: executes Task Packets received from the Management Server.
//! Flow: receive TaskPacket → store VCS token → clone repo → read spec → emit to frontend.

use chrono::{DateTime, Utc};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

use crate::enterprise::types::TaskPacket;

/// Stored short-lived VCS token for a task.
#[derive(Debug, Clone)]
pub struct VcsCredential {
    pub task_id: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl VcsCredential {
    /// Returns true if the token has less than 20% of its lifetime remaining.
    pub fn needs_refresh(&self, issued_at: DateTime<Utc>) -> bool {
        let total = (self.expires_at - issued_at).num_seconds();
        let remaining = (self.expires_at - Utc::now()).num_seconds();
        if total <= 0 {
            return true;
        }
        (remaining as f64 / total as f64) < 0.20
    }
}

pub struct VcsCredentialManager {
    credentials: Vec<VcsCredential>,
}

impl VcsCredentialManager {
    pub fn new() -> Self {
        Self { credentials: Vec::new() }
    }

    pub fn store(&mut self, task_id: String, token: String, expires_at: DateTime<Utc>) {
        self.credentials.retain(|c| c.task_id != task_id);
        self.credentials.push(VcsCredential { task_id, token, expires_at });
    }

    pub fn get(&self, task_id: &str) -> Option<&VcsCredential> {
        self.credentials.iter().find(|c| c.task_id == task_id)
    }

    pub fn remove(&mut self, task_id: &str) {
        self.credentials.retain(|c| c.task_id != task_id);
    }
}

/// Executes a task packet: clones the repo, reads the spec, emits to frontend for Agent Loop.
/// The actual AI agent execution is handled by useAgentMission.ts on the frontend.
pub async fn execute_task(app: &AppHandle, packet: TaskPacket, vcs_mgr: Arc<Mutex<VcsCredentialManager>>) {
    let task_id = packet.task_id.clone();
    log::info!("task_runner: starting task {}: {}", task_id, packet.title);

    // Store VCS token.
    {
        let mut mgr = vcs_mgr.lock().await;
        mgr.store(
            task_id.clone(),
            packet.vcs_token.clone(),
            packet.vcs_token_expires_at,
        );
    }

    // Clone the repo and checkout the work branch.
    let repo_dir = match clone_repo(&packet).await {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("task_runner: clone failed: {}", e);
            app.emit("enterprise:task-failed", serde_json::json!({
                "task_id": task_id,
                "reason": format!("repo clone failed: {}", e),
            })).ok();
            return;
        }
    };

    // Read the OpenSpec tasks.md if a spec_path is given.
    let spec_content = if let Some(spec_path) = &packet.spec_path {
        let full_path = repo_dir.join(spec_path);
        std::fs::read_to_string(&full_path).ok()
    } else {
        None
    };

    // Emit to frontend: this triggers the Agent Loop (useAgentMission.ts).
    app.emit("enterprise:task-ready", serde_json::json!({
        "task_id": task_id,
        "mission_id": packet.mission_id,
        "title": packet.title,
        "description": packet.description,
        "spec_content": spec_content,
        "repo_dir": repo_dir.to_string_lossy(),
        "work_branch": packet.vcs.work_branch,
        "ai_provider_id": packet.ai_provider_id,
        "execution_mode": packet.execution_mode,
        "max_steps": packet.max_steps,
        "on_complete": packet.on_complete,
    })).ok();
}

async fn clone_repo(packet: &TaskPacket) -> anyhow::Result<std::path::PathBuf> {
    let work_dir = std::env::temp_dir()
        .join("aiterm-tasks")
        .join(&packet.task_id);

    std::fs::create_dir_all(&work_dir)?;

    // Build authenticated repo URL.
    let auth_url = inject_token_into_url(&packet.vcs.repo_url, &packet.vcs_token)?;

    // git clone --branch <base> --depth 1 <url> <dir>
    let status = tokio::process::Command::new("git")
        .args([
            "clone",
            "--branch", &packet.vcs.base_branch,
            "--depth", "1",
            &auth_url,
            work_dir.to_str().unwrap_or("."),
        ])
        .status()
        .await?;

    if !status.success() {
        anyhow::bail!("git clone exited with {}", status);
    }

    // Create and checkout work branch.
    tokio::process::Command::new("git")
        .args(["checkout", "-b", &packet.vcs.work_branch])
        .current_dir(&work_dir)
        .status()
        .await?;

    Ok(work_dir)
}

/// Injects a token into a GitHub/GitLab HTTPS URL.
/// `https://github.com/org/repo` → `https://x-access-token:<token>@github.com/org/repo`
fn inject_token_into_url(url: &str, token: &str) -> anyhow::Result<String> {
    let mut parsed = url::Url::parse(url)?;
    parsed.set_username("x-access-token").map_err(|_| anyhow::anyhow!("cannot set username"))?;
    parsed.set_password(Some(token)).map_err(|_| anyhow::anyhow!("cannot set password"))?;
    Ok(parsed.to_string())
}

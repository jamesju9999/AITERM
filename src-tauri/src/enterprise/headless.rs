//! Headless Worker mode: runs without Tauri GUI.
//! Started when AITERM is launched with `--headless`.
//!
//! Tasks 11.1, 11.3, 11.4, 11.5, 11.6 implementation.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};
use chrono;

use crate::config::ConfigStore;
use crate::enterprise::types::{
    DeviceStatus, HeartbeatRequest, HeartbeatResponse, OnCompleteAction, TaskPacket,
};
use crate::secret::SecretStore;

const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const MAX_CONCURRENT_TASKS: usize = 3;

struct HeadlessState {
    /// task_id → join handle
    active: HashMap<String, tokio::task::JoinHandle<()>>,
    /// task_id → (steps_done, steps_total)
    progress: HashMap<String, (u32, u32)>,
}

impl HeadlessState {
    fn new() -> Self {
        Self { active: HashMap::new(), progress: HashMap::new() }
    }

    fn count(&self) -> usize {
        self.active.len()
    }

    fn reap_finished(&mut self) {
        let done: Vec<_> = self.active.iter()
            .filter(|(_, h)| h.is_finished())
            .map(|(k, _)| k.clone())
            .collect();
        for k in done {
            self.active.remove(&k);
            self.progress.remove(&k);
        }
    }

    fn first_task_id(&self) -> Option<String> {
        self.active.keys().next().cloned()
    }
}

/// Entry point for headless mode. Blocks until the process is killed.
pub async fn run_headless(config: Arc<ConfigStore>, secrets: Arc<SecretStore>) {
    let cfg = config.get();

    let server_url = match cfg.enterprise_server_url {
        Some(ref url) => url.clone(),
        None => {
            eprintln!("[headless] enterprise_server_url not set — exiting");
            return;
        }
    };

    let device_id = match cfg.enterprise_device_id {
        Some(ref id) => id.clone(),
        None => {
            eprintln!("[headless] enterprise_device_id not set — exiting");
            return;
        }
    };

    let device_token = match secrets.get(&format!("enterprise_device_{}", device_id)) {
        Ok(Some(t)) => t,
        _ => {
            eprintln!("[headless] device token not found in keychain — exiting");
            return;
        }
    };

    println!("[headless] started — server={} device={}", server_url, device_id);

    let client = reqwest::Client::new();
    let state = Arc::new(Mutex::new(HeadlessState::new()));
    let mut ticker = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));

    loop {
        ticker.tick().await;

        state.lock().await.reap_finished();

        // Build heartbeat request
        let (status, current_task_id, task_progress) = {
            let s = state.lock().await;
            let status = if s.count() > 0 { DeviceStatus::Busy } else { DeviceStatus::Idle };
            let first_id = s.first_task_id();
            let progress = first_id.as_ref().and_then(|tid| {
                s.progress.get(tid).map(|(done, total)| crate::enterprise::types::TaskProgress {
                    task_id: tid.clone(),
                    steps_done: *done,
                    steps_total: *total,
                })
            });
            (status, first_id, progress)
        };

        let req = HeartbeatRequest { status, current_task_id, task_progress };
        let url = format!("{}/api/devices/{}/heartbeat", server_url, device_id);

        match client.post(&url).bearer_auth(&device_token).json(&req).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.json::<HeartbeatResponse>().await {
                    handle_response(body, &server_url, &device_token, &state, &config).await;
                }
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                eprintln!("[headless] device token rejected (401) — exiting");
                return;
            }
            Ok(resp) => eprintln!("[headless] heartbeat HTTP {}", resp.status()),
            Err(e) => eprintln!("[headless] heartbeat failed: {}", e),
        }
    }
}

async fn handle_response(
    response: HeartbeatResponse,
    server_url: &str,
    device_token: &str,
    state: &Arc<Mutex<HeadlessState>>,
    config: &Arc<ConfigStore>,
) {
    // Apply policy if updated
    if let Some(policy) = response.policy_update {
        let _ = config.update(|cfg| {
            use crate::config::ExecutionMode;
            let ep = cfg.enterprise_policy.get_or_insert_with(Default::default);
            ep.version = policy.version;
            if let Some(id) = &policy.ai_provider_id { ep.ai_provider_id = Some(id.clone()); }
            if let Some(m) = &policy.execution_mode {
                ep.execution_mode = match m.as_str() {
                    "graded" => Some(ExecutionMode::Graded),
                    "full_auto" => Some(ExecutionMode::FullAuto),
                    _ => Some(ExecutionMode::AlwaysConfirm),
                };
            }
            if let Some(s) = policy.max_agent_steps { ep.max_agent_steps = Some(s); }
        });
    }

    // 11.3: Auto-accept pending tasks up to concurrency limit
    let slots = MAX_CONCURRENT_TASKS.saturating_sub(state.lock().await.count());
    for task in response.pending_tasks.into_iter().take(slots) {
        let task_id = task.task_id.clone();
        println!("[headless] auto-accepting task {}: {}", task_id, task.title);

        let srv = server_url.to_string();
        let tok = device_token.to_string();
        let cfg = config.clone();
        let state_clone = state.clone();

        let handle = tokio::spawn(async move {
            execute_task(task, &srv, &tok, &cfg, &state_clone).await;
        });

        state.lock().await.active.insert(task_id, handle);
    }
}

async fn execute_task(
    packet: TaskPacket,
    server_url: &str,
    device_token: &str,
    config: &Arc<ConfigStore>,
    state: &Arc<Mutex<HeadlessState>>,
) {
    let task_id = packet.task_id.clone();
    let client = reqwest::Client::new();

    let repo_dir = match clone_repo(&packet).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[headless] task {} clone failed: {}", task_id, e);
            fail_task(server_url, device_token, &task_id, &e.to_string()).await;
            return;
        }
    };

    let goal = {
        let mut g = packet.description.clone();
        if let Some(spec_path) = &packet.spec_path {
            let spec = std::fs::read_to_string(repo_dir.join(spec_path)).unwrap_or_default();
            if !spec.is_empty() {
                g.push_str("\n\nSpec:\n");
                g.push_str(&spec);
            }
        }
        g
    };

    println!("[headless] task {}: {}", task_id, goal.chars().take(80).collect::<String>());

    let max_steps = packet.max_steps;
    let mut history: Vec<(String, i32, String)> = Vec::new(); // (cmd, exit, output)

    for step in 0..max_steps {
        // Update progress
        state.lock().await.progress.insert(task_id.clone(), (step, max_steps));

        let query = build_query(&goal, &history, step, max_steps);
        let command = match call_ai(&client, &query, config).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[headless] AI error step {}: {}", step, e);
                fail_task(server_url, device_token, &task_id, &e.to_string()).await;
                return;
            }
        };

        let command = command.trim().to_string();
        if command == "DONE" {
            println!("[headless] task {} DONE at step {}", task_id, step);
            break;
        }

        // 11.5: Detect dangerous commands and request remote approval before executing.
        if is_dangerous_command(&command) {
            println!("[headless] task {} step {}: dangerous command detected, requesting approval", task_id, step);
            let approved = request_dangerous_command_approval(
                server_url, device_token, &task_id, &command, &client
            ).await;
            if !approved {
                eprintln!("[headless] task {} dangerous command rejected by admin", task_id);
                fail_task(server_url, device_token, &task_id, "dangerous command rejected by admin").await;
                return;
            }
        }

        let (exit_code, output) = run_cmd(&command, &repo_dir).await
            .unwrap_or_else(|e| (1, e.to_string()));

        println!("[headless] step {}/{}: $ {} → {}", step + 1, max_steps, command.trim(), exit_code);

        let log = format!("[{}/{}] $ {}\nexit {}\n{}\n", step + 1, max_steps, command.trim(), exit_code, output.chars().take(500).collect::<String>());
        // 11.6: Write to local rotating log file
        write_log_file(&task_id, &log).await;
        // SSE log upload to server
        let _ = client.post(format!("{server_url}/api/tasks/{task_id}/log"))
            .bearer_auth(device_token)
            .json(&serde_json::json!({ "line": log }))
            .send()
            .await;

        history.push((command, exit_code, output));
    }

    // on_complete: push branch
    let _ = tokio::process::Command::new("git")
        .args(["push", "origin", &packet.vcs.work_branch])
        .current_dir(&repo_dir)
        .status()
        .await;

    // Optional: open PR
    if let OnCompleteAction::OpenPr { title, base_branch } = &packet.on_complete {
        if let Ok(github_token) = std::env::var("GITHUB_TOKEN") {
            if let Ok(url) = url::Url::parse(&packet.vcs.repo_url) {
                let path = url.path().trim_matches('/').trim_end_matches(".git");
                let _ = client
                    .post(format!("https://api.github.com/repos/{path}/pulls"))
                    .bearer_auth(&github_token)
                    .header("Accept", "application/vnd.github+json")
                    .json(&serde_json::json!({
                        "title": title,
                        "head": packet.vcs.work_branch,
                        "base": base_branch,
                    }))
                    .send()
                    .await;
            }
        }
    }

    // Report complete
    let _ = client.post(format!("{server_url}/api/tasks/{task_id}/complete"))
        .bearer_auth(device_token)
        .json(&serde_json::json!({}))
        .send()
        .await;

    let _ = tokio::fs::remove_dir_all(&repo_dir).await;
    state.lock().await.progress.remove(&task_id);
}

/// Patterns that indicate a potentially destructive command requiring admin approval.
fn is_dangerous_command(cmd: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "rm -rf", "rm -fr", "dd if=", "mkfs", "fdisk", "format ",
        "DROP TABLE", "DROP DATABASE", "TRUNCATE", "DELETE FROM",
        "chmod 777", "chmod -R 777", "shutdown", "reboot", "halt",
        "git push --force", "git push -f",
    ];
    let lower = cmd.to_lowercase();
    PATTERNS.iter().any(|p| lower.contains(&p.to_lowercase()))
}

/// Report a dangerous command to the server, set task status to `awaiting_approval`,
/// then poll until the admin approves or rejects (max 10 minutes).
/// Returns true if approved, false if rejected or timed out.
async fn request_dangerous_command_approval(
    server_url: &str,
    token: &str,
    task_id: &str,
    command: &str,
    client: &reqwest::Client,
) -> bool {
    // Notify server: task is awaiting approval
    let _ = client
        .post(format!("{server_url}/api/tasks/{task_id}/dangerous-command"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "command": command,
            "status": "pending",
        }))
        .send()
        .await;

    // Poll for the admin's decision (check task status every 5 seconds, up to 10 min)
    for _ in 0..120 {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let resp = client
            .get(format!("{server_url}/api/tasks/{task_id}"))
            .bearer_auth(token)
            .send()
            .await;

        if let Ok(r) = resp {
            if let Ok(json) = r.json::<serde_json::Value>().await {
                let status = json["status"].as_str().unwrap_or("");
                match status {
                    "running" => return true,   // admin approved
                    "failed" => return false,   // admin rejected
                    _ => {}                     // still awaiting_approval, keep polling
                }
            }
        }
    }

    // Timed out
    false
}

fn build_query(goal: &str, history: &[(String, i32, String)], step: u32, max: u32) -> String {
    if history.is_empty() {
        return format!("{goal}\n\nRespond with a single shell command, or 'DONE' if complete. No markdown.");
    }
    let hist: String = history.iter().enumerate()
        .map(|(i, (cmd, exit, out))| format!("Step {}:\n$ {}\nexit: {}\n{}", i + 1, cmd.trim(), exit, out.chars().take(300).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("Goal: {goal}\n\nHistory:\n{hist}\n\nStep {}/{}: next command or 'DONE'.", step + 1, max)
}

async fn call_ai(client: &reqwest::Client, prompt: &str, config: &Arc<ConfigStore>) -> anyhow::Result<String> {
    let cfg = config.get();
    let provider = cfg.default_provider.as_deref()
        .and_then(|id| cfg.providers.iter().find(|p| p.id == id))
        .or_else(|| cfg.providers.first())
        .ok_or_else(|| anyhow::anyhow!("no AI provider configured"))?;
    let base = provider.base_url.clone().unwrap_or_else(|| "https://api.openai.com".into());
    let model = provider.model.clone();
    let api_key = std::env::var("OPENAI_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
        .unwrap_or_default();

    let resp = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&api_key)
        .json(&serde_json::json!({
            "model": model,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 256,
            "temperature": 0.1,
        }))
        .send()
        .await?;

    anyhow::ensure!(resp.status().is_success(), "AI API {}", resp.status());
    let json: serde_json::Value = resp.json().await?;
    Ok(json["choices"][0]["message"]["content"].as_str().unwrap_or("DONE").to_string())
}

async fn run_cmd(cmd: &str, cwd: &std::path::Path) -> anyhow::Result<(i32, String)> {
    let out = tokio::process::Command::new("sh").arg("-c").arg(cmd).current_dir(cwd).output().await?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    Ok((out.status.code().unwrap_or(1), stdout))
}

async fn clone_repo(packet: &TaskPacket) -> anyhow::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("aiterm-headless").join(&packet.task_id);
    std::fs::create_dir_all(&dir)?;

    let mut url = url::Url::parse(&packet.vcs.repo_url)?;
    url.set_username("x-access-token").map_err(|_| anyhow::anyhow!("url err"))?;
    url.set_password(Some(&packet.vcs_token)).map_err(|_| anyhow::anyhow!("url err"))?;

    let status = tokio::process::Command::new("git")
        .args(["clone", "--branch", &packet.vcs.base_branch, "--depth", "1", url.as_str(), dir.to_str().unwrap()])
        .status().await?;
    anyhow::ensure!(status.success(), "git clone failed");

    tokio::process::Command::new("git")
        .args(["checkout", "-b", &packet.vcs.work_branch])
        .current_dir(&dir).status().await?;

    Ok(dir)
}

/// Write a log line to a per-task rotating log file and purge files older than 7 days (11.6).
async fn write_log_file(task_id: &str, line: &str) {
    let log_dir = std::env::temp_dir().join("aiterm-headless-logs");
    let _ = tokio::fs::create_dir_all(&log_dir).await;

    // Purge log files older than 7 days
    if let Ok(mut entries) = tokio::fs::read_dir(&log_dir).await {
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(7 * 24 * 3600))
            .unwrap_or(std::time::UNIX_EPOCH);
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = tokio::fs::remove_file(entry.path()).await;
                    }
                }
            }
        }
    }

    let log_file = log_dir.join(format!("{task_id}.log"));
    use tokio::io::AsyncWriteExt;
    if let Ok(mut f) = tokio::fs::OpenOptions::new()
        .create(true).append(true).open(&log_file).await
    {
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
        let _ = f.write_all(format!("[{ts}] {line}\n").as_bytes()).await;
    }
}

async fn fail_task(server_url: &str, token: &str, task_id: &str, reason: &str) {
    let client = reqwest::Client::new();
    let _ = client.post(format!("{server_url}/api/tasks/{task_id}/fail"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "reason": reason }))
        .send().await;
}

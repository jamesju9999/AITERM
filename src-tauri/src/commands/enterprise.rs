use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;

use crate::enterprise::{
    agent::EnterpriseTaskState,
    task_runner::{VcsCredentialManager, execute_task},
    types::TaskPacket,
};
use crate::config::ConfigStore;
use crate::secret::SecretStore;

/// Accept a task packet and start execution (clone repo, read spec, emit task-ready).
/// Called by the Interactive frontend when the user accepts a task.
#[tauri::command]
pub async fn enterprise_accept_task(
    app: AppHandle,
    vcs_mgr: State<'_, Arc<Mutex<VcsCredentialManager>>>,
    task_state: State<'_, Arc<Mutex<EnterpriseTaskState>>>,
    packet: TaskPacket,
) -> Result<(), String> {
    // Mark device as busy.
    {
        let mut state = task_state.lock().await;
        state.current_task_id = Some(packet.task_id.clone());
        state.task_progress = None;
    }

    let mgr = vcs_mgr.inner().clone();
    execute_task(&app, packet, mgr).await;
    Ok(())
}

/// Reject a task — device stays idle; server re-queues or cancels.
#[tauri::command]
pub async fn enterprise_reject_task(
    _task_id: String,
) -> Result<(), String> {
    Ok(())
}

/// Update task progress so the heartbeat can report it to the server.
#[tauri::command]
pub async fn enterprise_update_task_progress(
    task_id: String,
    steps_done: u32,
    steps_total: u32,
    task_state: State<'_, Arc<Mutex<EnterpriseTaskState>>>,
) -> Result<(), String> {
    let mut state = task_state.lock().await;
    state.current_task_id = Some(task_id.clone());
    state.task_progress = Some(crate::enterprise::types::TaskProgress {
        task_id,
        steps_done,
        steps_total,
    });
    Ok(())
}

/// Mark a task as complete and clear the busy state.
#[tauri::command]
pub async fn enterprise_complete_task(
    _task_id: String,
    task_state: State<'_, Arc<Mutex<EnterpriseTaskState>>>,
) -> Result<(), String> {
    let mut state = task_state.lock().await;
    state.current_task_id = None;
    state.task_progress = None;
    Ok(())
}

/// Execute the on_complete action after a task's agent loop finishes.
/// Currently supports: open_pr (GitHub) and push_only.
#[tauri::command]
pub async fn enterprise_on_complete(
    task_id: String,
    repo_dir: String,
    work_branch: String,
    on_complete: serde_json::Value,
    vcs_mgr: State<'_, Arc<Mutex<VcsCredentialManager>>>,
) -> Result<String, String> {
    // Push the work branch first.
    let push_status = tokio::process::Command::new("git")
        .args(["push", "--set-upstream", "origin", &work_branch])
        .current_dir(&repo_dir)
        .status()
        .await
        .map_err(|e| e.to_string())?;

    if !push_status.success() {
        return Err(format!("git push failed with {}", push_status));
    }

    // If on_complete is open_pr, create the PR via GitHub API.
    let action_type = on_complete.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if action_type == "open_pr" {
        let pr_title = on_complete.get("title").and_then(|v| v.as_str()).unwrap_or("Enterprise Task PR");
        let base_branch = on_complete.get("base_branch").and_then(|v| v.as_str()).unwrap_or("main");

        // Get the authenticated remote URL to extract owner/repo.
        let remote_url_out = tokio::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(&repo_dir)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        let remote_url = String::from_utf8_lossy(&remote_url_out.stdout).trim().to_string();

        // Extract owner/repo from github URL.
        // Pattern: https://x-access-token:TOKEN@github.com/owner/repo.git
        if let Some(path) = remote_url.split("github.com/").nth(1) {
            let repo_path = path.trim_end_matches(".git");
            let token = {
                let mgr = vcs_mgr.lock().await;
                mgr.get(&task_id).map(|c| c.token.clone())
            };
            if let Some(token) = token {
                let client = reqwest::Client::new();
                let resp = client
                    .post(format!("https://api.github.com/repos/{}/pulls", repo_path))
                    .header("Authorization", format!("token {}", token))
                    .header("User-Agent", "aiterm-enterprise")
                    .json(&serde_json::json!({
                        "title": pr_title,
                        "head": work_branch,
                        "base": base_branch,
                        "body": format!("Automated PR created by AITerm Enterprise (task {})", task_id),
                    }))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;

                if resp.status().is_success() {
                    let pr: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                    let pr_url = pr.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
                    return Ok(format!("PR created: {}", pr_url));
                } else {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(format!("PR creation failed {}: {}", status, text));
                }
            }
        }
    }

    Ok("push complete".to_string())
}

/// Generate and optionally install a system service for headless worker mode (11.2).
/// Returns the generated service file content so the UI can show it.
#[tauri::command]
pub async fn enterprise_install_service(
    install: bool,
) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_path = exe.to_string_lossy();

    #[cfg(target_os = "linux")]
    {
        let unit = format!(
            r#"[Unit]
Description=AITerm Headless Worker
After=network.target

[Service]
ExecStart={exe_path} --headless
Restart=always
RestartSec=10
Environment=AITERM_LOG=info

[Install]
WantedBy=multi-user.target
"#
        );
        if install {
            let dest = "/etc/systemd/system/aiterm-headless.service";
            std::fs::write(dest, &unit).map_err(|e| format!("write failed: {e}"))?;
            tokio::process::Command::new("systemctl")
                .args(["daemon-reload"])
                .status()
                .await
                .map_err(|e| e.to_string())?;
            tokio::process::Command::new("systemctl")
                .args(["enable", "--now", "aiterm-headless.service"])
                .status()
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(unit);
    }

    #[cfg(target_os = "macos")]
    {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.aiterm.headless</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_path}</string>
        <string>--headless</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>/tmp/aiterm-headless.log</string>
    <key>StandardOutPath</key>
    <string>/tmp/aiterm-headless.log</string>
</dict>
</plist>
"#
        );
        if install {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            let dest = format!("{home}/Library/LaunchAgents/com.aiterm.headless.plist");
            std::fs::write(&dest, &plist).map_err(|e| format!("write failed: {e}"))?;
            tokio::process::Command::new("launchctl")
                .args(["load", &dest])
                .status()
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(plist);
    }

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            r#"# Run as Administrator
New-Service -Name "AITermHeadless" `
  -BinaryPathName '"{exe_path}" --headless' `
  -DisplayName "AITerm Headless Worker" `
  -StartupType Automatic `
  -Description "AITerm Enterprise headless worker service"

Start-Service -Name "AITermHeadless"
"#
        );
        if install {
            // On Windows, we just write the PowerShell script — admin elevation needed
            let script_path = std::env::temp_dir().join("aiterm-install-service.ps1");
            std::fs::write(&script_path, &script).map_err(|e| format!("write failed: {e}"))?;
            tokio::process::Command::new("powershell")
                .args(["-ExecutionPolicy", "Bypass", "-File", script_path.to_str().unwrap_or("")])
                .status()
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(script);
    }

    #[allow(unreachable_code)]
    Err("unsupported platform".into())
}

/// Register this device with an Enterprise Management Server.
/// Stores the returned device_token in the OS keychain.
#[tauri::command]
pub async fn enterprise_register_device(
    server_url: String,
    device_name: String,
    device_type: String,
    role: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let platform = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "name": device_name,
        "device_type": device_type,
        "role": role,
        "platform": platform,
    });
    let resp = client
        .post(format!("{}/api/devices/register", server_url))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("Server error {}: {}", status, text));
    }

    #[derive(serde::Deserialize)]
    struct RegisterResponse {
        device_id: String,
        device_token: String,
    }
    let data: RegisterResponse = resp.json().await.map_err(|e| e.to_string())?;

    // Store device_token in keychain.
    let key = format!("enterprise_device_{}", data.device_id);
    secrets
        .set(&key, &data.device_token)
        .map_err(|e| e.to_string())?;

    // Save device_id and server_url in config.
    config.update(|cfg| {
        cfg.enterprise_server_url = Some(server_url.clone());
        cfg.enterprise_device_id = Some(data.device_id.clone());
    }).map_err(|e| e.to_string())?;

    Ok(data.device_id)
}

//! Enterprise Agent: heartbeat polling, policy sync, skill sync, task dispatch.

use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::{interval, Duration};
use tokio::sync::Mutex;

use crate::config::{ConfigStore, ExecutionMode};
use crate::enterprise::{
    skill_sync::SkillSyncer,
    types::{DeviceStatus, HeartbeatRequest, HeartbeatResponse, TaskProgress},
};

const HEARTBEAT_INTERVAL_SECS: u64 = 30;
const MAX_RETRIES_BEFORE_OFFLINE: u32 = 3;

/// Globally managed enterprise task state — updated by commands, read by heartbeat.
pub struct EnterpriseTaskState {
    pub current_task_id: Option<String>,
    pub task_progress: Option<TaskProgress>,
    pub consecutive_failures: u32,
}

impl EnterpriseTaskState {
    pub fn new() -> Self {
        Self {
            current_task_id: None,
            task_progress: None,
            consecutive_failures: 0,
        }
    }
}

/// Initialize and start the Enterprise Agent if enterprise mode is configured.
pub fn init(app: &AppHandle) {
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        run_agent(&app_handle).await;
    });
}

async fn run_agent(app: &AppHandle) {
    let config_store = app.state::<Arc<ConfigStore>>();
    let config = config_store.get();

    let server_url = match &config.enterprise_server_url {
        Some(url) => url.clone(),
        None => return, // Not in enterprise mode.
    };

    let device_id = match &config.enterprise_device_id {
        Some(id) => id.clone(),
        None => {
            log::warn!("enterprise_server_url set but no enterprise_device_id — skipping agent");
            return;
        }
    };

    // Retrieve device token from Keychain.
    let secret_store = app.state::<Arc<crate::secret::SecretStore>>();
    let device_token = match secret_store.get(&format!("enterprise_device_{}", device_id)) {
        Ok(Some(t)) => t,
        _ => {
            log::warn!("enterprise device token not found in keychain");
            return;
        }
    };

    log::info!("Enterprise Agent started for server {}", server_url);

    let client = reqwest::Client::new();
    // Use globally managed state so commands can update current_task_id / progress.
    let task_state = app.state::<Arc<Mutex<EnterpriseTaskState>>>();

    let mut ticker = interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    loop {
        ticker.tick().await;

        let (status, current_task_id, task_progress) = {
            let state = task_state.lock().await;
            let status = if state.current_task_id.is_some() {
                DeviceStatus::Busy
            } else {
                DeviceStatus::Idle
            };
            (status, state.current_task_id.clone(), state.task_progress.clone())
        };

        let req_body = HeartbeatRequest {
            status,
            current_task_id,
            task_progress,
        };

        let url = format!("{}/api/devices/{}/heartbeat", server_url, device_id);
        match client
            .post(&url)
            .bearer_auth(&device_token)
            .json(&req_body)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                {
                    let mut state = task_state.lock().await;
                    state.consecutive_failures = 0;
                }

                if let Ok(body) = resp.json::<HeartbeatResponse>().await {
                    handle_heartbeat_response(app, body, &task_state).await;
                }
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                log::error!("Enterprise Agent: device token rejected (401) — stopping");
                app.emit("enterprise:auth-error", ()).ok();
                break;
            }
            Err(e) => {
                let mut state = task_state.lock().await;
                state.consecutive_failures += 1;
                if state.consecutive_failures >= MAX_RETRIES_BEFORE_OFFLINE {
                    app.emit("enterprise:offline", ()).ok();
                    log::warn!("Enterprise Agent: {} consecutive failures", state.consecutive_failures);
                }
                log::warn!("heartbeat failed: {}", e);
            }
            _ => {}
        }
    }
}

async fn handle_heartbeat_response(
    app: &AppHandle,
    response: HeartbeatResponse,
    task_state: &Arc<Mutex<EnterpriseTaskState>>,
) {
    // 1. Apply policy update.
    if let Some(policy) = response.policy_update {
        apply_policy_update(app, policy).await;
    }

    // 2. Sync skills.
    if !response.skill_updates.is_empty() {
        let app_clone = app.clone();
        let updates = response.skill_updates;
        tauri::async_runtime::spawn(async move {
            SkillSyncer::apply_updates(&app_clone, updates).await;
        });
    }

    // 3. Dispatch pending tasks.
    let is_busy = task_state.lock().await.current_task_id.is_some();

    if !is_busy && !response.pending_tasks.is_empty() {
        // Take the first pending task.
        let task = response.pending_tasks.into_iter().next().unwrap();
        // Emit to frontend so user can accept/defer (Interactive mode).
        app.emit("enterprise:task-received", &task).ok();
    }
}

async fn apply_policy_update(app: &AppHandle, policy: super::types::PolicyUpdate) {
    let config_store = app.state::<Arc<ConfigStore>>();

    let current_version = config_store.get().enterprise_policy
        .as_ref().map(|p| p.version).unwrap_or(0);

    if policy.version <= current_version {
        return;
    }

    let result = config_store.update(|cfg| {
        let ep = cfg.enterprise_policy.get_or_insert_with(Default::default);
        ep.version = policy.version;
        if let Some(p) = &policy.ai_provider_id {
            ep.ai_provider_id = Some(p.clone());
        }
        if let Some(m) = &policy.execution_mode {
            ep.execution_mode = match m.as_str() {
                "graded" => Some(ExecutionMode::Graded),
                "full_auto" => Some(ExecutionMode::FullAuto),
                _ => Some(ExecutionMode::AlwaysConfirm),
            };
        }
        if let Some(s) = policy.max_agent_steps {
            ep.max_agent_steps = Some(s);
        }
        ep.vcs_push_pattern = policy.vcs_push_pattern.clone();
    });

    if let Err(e) = result {
        log::error!("Failed to save policy update: {}", e);
        return;
    }

    let updated_policy = config_store.get().enterprise_policy;
    app.emit("enterprise:policy-updated", &updated_policy).ok();
    log::info!("Enterprise policy updated to version {}", policy.version);
}

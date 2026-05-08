use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Task Packet received from Management Server via heartbeat response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPacket {
    pub task_id: String,
    pub mission_id: String,
    pub title: String,
    pub description: String,
    pub spec_path: Option<String>,
    pub vcs: VcsTaskConfig,
    pub ai_provider_id: String,
    pub execution_mode: String,
    pub max_steps: u32,
    pub on_complete: OnCompleteAction,
    pub vcs_token: String,
    pub vcs_token_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsTaskConfig {
    pub repo_url: String,
    pub base_branch: String,
    pub work_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OnCompleteAction {
    OpenPr { title: String, base_branch: String },
    PushOnly,
    Notify { message: String },
}

/// Policy update from Management Server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyUpdate {
    pub version: i64,
    pub ai_provider_id: Option<String>,
    pub execution_mode: Option<String>,
    pub max_agent_steps: Option<u32>,
    pub vcs_push_pattern: Option<String>,
}

/// Skill update from Management Server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUpdate {
    pub skill_id: String,
    pub version: String,
    pub content: Option<String>,
    pub action: SkillUpdateAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillUpdateAction {
    Install,
    Remove,
}

/// Heartbeat request body.
#[derive(Debug, Serialize)]
pub struct HeartbeatRequest {
    pub status: DeviceStatus,
    pub current_task_id: Option<String>,
    pub task_progress: Option<TaskProgress>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatus {
    Idle,
    Busy,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskProgress {
    pub task_id: String,
    pub steps_done: u32,
    pub steps_total: u32,
}

/// Heartbeat response body.
#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    pub pending_tasks: Vec<TaskPacket>,
    pub policy_update: Option<PolicyUpdate>,
    pub skill_updates: Vec<SkillUpdate>,
}

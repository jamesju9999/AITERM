//! Tauri commands for VCS connection management and natural-language queries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::ai::{router::AiRouter, ChatMessage, GenerateChunk, GenerateRequest, QueryMode};
use crate::config::{
    types::{VcsConnection, VcsType, VcsWriteMode},
    ConfigStore,
};
use crate::secret::SecretStore;
use crate::vcs::{
    git::GitClient,
    svn::SvnClient,
    VcsAgentDecision, VcsAgentHistoryEntry, VcsIntent, VcsManager, VcsRepoInfo, VcsResult,
};

/// Tracks the in-flight AI call behind each `vcs_agent_step`, keyed by the
/// VCS agent loop's session id, so a stuck step can actually be cancelled
/// from the frontend's Stop button instead of just being ignored client-side.
#[derive(Default)]
pub struct VcsAgentStepRegistry(Mutex<HashMap<String, AbortHandle>>);

impl VcsAgentStepRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Steps rarely need more than a few seconds; if the provider stalls this
/// keeps the loop from hanging forever with the Stop button unable to help.
const VCS_AGENT_STEP_TIMEOUT: Duration = Duration::from_secs(60);

fn vcs_secret_key(id: &str) -> String {
    format!("vcs:{id}")
}

/// 把功能名稱轉成適合當 git 分支名稱片段的字串：只留英數字元，
/// 其他字元（含中文、空白、標點）都轉成 `-`，並收斂連續的 `-`。
fn slugify(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn resolve_vcs_token(repo_info: &VcsRepoInfo, secrets: &SecretStore) -> Option<String> {
    repo_info
        .connection_id
        .as_deref()
        .and_then(|id| secrets.get(&vcs_secret_key(id)).ok().flatten())
}

fn resolve_write_mode(repo_info: &VcsRepoInfo, config: &ConfigStore) -> VcsWriteMode {
    repo_info
        .connection_id
        .as_deref()
        .and_then(|id| {
            config
                .get()
                .vcs_connections
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.write_mode)
        })
        .unwrap_or(VcsWriteMode::Guarded)
}

#[cfg(test)]
mod resolve_write_mode_tests {
    use super::*;
    use crate::config::types::{AppConfig, VcsConnection, VcsType};

    fn repo_info(connection_id: Option<&str>) -> VcsRepoInfo {
        VcsRepoInfo {
            vcs_type: VcsType::Git,
            root: "/tmp/repo".to_string(),
            remote_url: None,
            connection_id: connection_id.map(str::to_string),
        }
    }

    #[test]
    fn no_connection_id_defaults_to_guarded() {
        let config = ConfigStore::from_config(AppConfig::default());
        assert_eq!(resolve_write_mode(&repo_info(None), &config), VcsWriteMode::Guarded);
    }

    #[test]
    fn connection_id_not_found_in_config_defaults_to_guarded() {
        let config = ConfigStore::from_config(AppConfig::default());
        assert_eq!(
            resolve_write_mode(&repo_info(Some("missing")), &config),
            VcsWriteMode::Guarded
        );
    }

    #[test]
    fn connection_id_found_returns_its_write_mode() {
        let mut cfg = AppConfig::default();
        cfg.vcs_connections.push(VcsConnection {
            id: "conn-1".to_string(),
            name: "Test Conn".to_string(),
            vcs_type: VcsType::Git,
            url: None,
            username: None,
            write_mode: VcsWriteMode::ReadOnly,
        });
        let config = ConfigStore::from_config(cfg);
        assert_eq!(
            resolve_write_mode(&repo_info(Some("conn-1")), &config),
            VcsWriteMode::ReadOnly
        );
    }
}

/// Input for add/update/test — includes the secret (token/password).
#[derive(Debug, Deserialize)]
pub struct VcsConnectionInput {
    pub id: Option<String>,
    pub name: String,
    pub vcs_type: VcsType,
    pub url: Option<String>,
    pub username: Option<String>,
    /// GitHub token or SVN password.
    pub secret: Option<String>,
    pub write_mode: VcsWriteMode,
}

/// Safe info returned to the frontend (no secret).
#[derive(Debug, Serialize)]
pub struct VcsConnectionInfo {
    pub id: String,
    pub name: String,
    pub vcs_type: VcsType,
    pub url: Option<String>,
    pub username: Option<String>,
    pub write_mode: VcsWriteMode,
    pub has_secret: bool,
}

#[tauri::command]
pub async fn vcs_list_connections(
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<VcsConnectionInfo>, String> {
    let conns = config.get().vcs_connections;
    let result = conns
        .into_iter()
        .map(|c| {
            let has_secret = secrets.has(&vcs_secret_key(&c.id));
            VcsConnectionInfo {
                id: c.id,
                name: c.name,
                vcs_type: c.vcs_type,
                url: c.url,
                username: c.username,
                write_mode: c.write_mode,
                has_secret,
            }
        })
        .collect();
    Ok(result)
}

#[tauri::command]
pub async fn vcs_add_connection(
    input: VcsConnectionInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let conn = VcsConnection {
        id: id.clone(),
        name: input.name,
        vcs_type: input.vcs_type,
        url: normalize_optional_string(input.url),
        username: normalize_optional_string(input.username),
        write_mode: input.write_mode,
    };
    config
        .add_vcs_connection(conn)
        .map_err(|e| e.to_string())?;
    if let Some(secret) = &input.secret {
        if !secret.is_empty() {
            secrets
                .set(&vcs_secret_key(&id), secret)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(id)
}

#[tauri::command]
pub async fn vcs_update_connection(
    input: VcsConnectionInput,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let id = input.id.clone().ok_or("missing id")?;
    let conn = VcsConnection {
        id: id.clone(),
        name: input.name,
        vcs_type: input.vcs_type,
        url: normalize_optional_string(input.url),
        username: normalize_optional_string(input.username),
        write_mode: input.write_mode,
    };
    config
        .update_vcs_connection(conn)
        .map_err(|e| e.to_string())?;
    if let Some(secret) = &input.secret {
        if !secret.is_empty() {
            secrets
                .set(&vcs_secret_key(&id), secret)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn vcs_remove_connection(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    config
        .remove_vcs_connection(&id)
        .map_err(|e| e.to_string())?;
    // Best-effort: ignore Keychain errors if secret was never stored.
    let _ = secrets.delete(&vcs_secret_key(&id));
    Ok(())
}

#[tauri::command]
pub async fn vcs_test_connection(
    input: VcsConnectionInput,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    // The edit form leaves the secret field blank ("leave blank to keep
    // unchanged") when editing an existing connection, so an empty
    // `input.secret` doesn't mean "no secret" — fall back to whatever is
    // already stored for this connection id.
    let stored_secret = input
        .id
        .as_deref()
        .and_then(|id| secrets.get(&vcs_secret_key(id)).ok().flatten());
    let effective_secret = input
        .secret
        .clone()
        .filter(|s| !s.is_empty())
        .or(stored_secret);

    match input.vcs_type {
        VcsType::Git => {
            let secret = effective_secret.as_deref().unwrap_or("");
            if secret.is_empty() {
                return Ok("Local Git mode — no test needed".into());
            }
            // Verify token by calling GitHub /user endpoint
            let client = reqwest::Client::new();
            let resp = client
                .get("https://api.github.com/user")
                .header("Authorization", format!("Bearer {secret}"))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header("User-Agent", "AITerm")
                .send()
                .await
                .map_err(|e| e.to_string())?;

            if resp.status().is_success() {
                let json: serde_json::Value =
                    resp.json().await.map_err(|e| e.to_string())?;
                let login = json["login"].as_str().unwrap_or("unknown");
                Ok(format!("GitHub token valid — authenticated as @{login}"))
            } else {
                let status = resp.status();
                Err(format!("GitHub token rejected (HTTP {status})"))
            }
        }
        VcsType::Svn => {
            let url = input.url.as_deref().unwrap_or_default();
            if url.is_empty() {
                return Err("SVN URL is required for testing".into());
            }

            let mut args = vec![
                "info".to_string(),
                "--non-interactive".to_string(),
                // Trust unknown-CA server certs (e.g. internal/self-signed)
                // since --non-interactive can't show the accept-cert prompt.
                "--trust-server-cert-failures=unknown-ca".to_string(),
                url.to_string(),
            ];
            if let Some(u) = &input.username {
                if !u.is_empty() {
                    args.push("--username".to_string());
                    args.push(u.clone());
                }
            }
            if let Some(p) = &effective_secret {
                if !p.is_empty() {
                    args.push("--password".to_string());
                    args.push(p.clone());
                }
            }

            let mut cmd = std::process::Command::new(crate::vcs::svn::svn_program());
            cmd.args(&args);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            let out = cmd.output()
                .map_err(|_| "svn command not found — please install SVN".to_string())?;

            if out.status.success() {
                Ok("SVN connection successful".into())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).to_string())
            }
        }
    }
}

#[tauri::command]
pub async fn vcs_detect_repo(
    path: String,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<VcsRepoInfo, String> {
    let mut info = VcsManager::detect_repo(&path).await?;

    // Try to match the remote URL against configured connections.
    if let Some(remote_url) = &info.remote_url {
        let connections = config.get().vcs_connections;
        for conn in &connections {
            if let Some(conn_url) = &conn.url {
                if urls_match(remote_url, conn_url) {
                    info.connection_id = Some(conn.id.clone());
                    break;
                }
            }
        }
    }

    Ok(info)
}

#[tauri::command]
pub async fn vcs_get_block_info(cwd: String) -> Option<crate::vcs::types::GitBlockInfo> {
    let client = crate::vcs::git::GitClient::new(cwd, None);
    client.quick_block_info().await
}

#[tauri::command]
pub async fn vcs_query(
    query: String,
    repo_info: VcsRepoInfo,
    session_id: String,
    config: State<'_, Arc<ConfigStore>>,
    secrets: State<'_, Arc<SecretStore>>,
    router: State<'_, AiRouter>,
) -> Result<serde_json::Value, String> {
    // 1. Use AI to parse the natural-language query into a VcsIntent.
    let intent = parse_vcs_intent(&query, &router).await?;

    // 2. Resolve token from keychain if a connection_id is set.
    let token = match &repo_info.connection_id {
        Some(id) => secrets
            .get(&vcs_secret_key(id))
            .ok()
            .flatten(),
        None => None,
    };

    // 3. Look up write_mode for this connection.
    let write_mode = repo_info
        .connection_id
        .as_deref()
        .and_then(|id| {
            config
                .get()
                .vcs_connections
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.write_mode)
        })
        .unwrap_or(VcsWriteMode::Guarded);

    // 4. Check write-mode gating for write operations.
    if is_write_intent(&intent) {
        match write_mode {
            VcsWriteMode::ReadOnly => {
                let result = VcsResult::Error {
                    message: "Write operations are disabled (ReadOnly mode)".into(),
                };
                return serde_json::to_value(&result).map_err(|e| e.to_string());
            }
            VcsWriteMode::Guarded => {
                // Return a WriteConfirm — let the frontend confirm before executing.
                let preview = describe_intent(&intent);
                let result = VcsResult::WriteConfirm {
                    operation: intent_operation_name(&intent).to_string(),
                    preview,
                    intent,
                };
                return serde_json::to_value(&result).map_err(|e| e.to_string());
            }
            VcsWriteMode::FullAuto => {
                // Fall through and execute immediately.
            }
        }
    }

    // 5. Dispatch to the appropriate backend.
    let vcs_result = match repo_info.vcs_type {
        VcsType::Git => {
            dispatch_git(intent, &repo_info.root, token, session_id).await
        }
        VcsType::Svn => {
            let (url, username, password) = resolve_svn_credentials(&repo_info, &config, &secrets);
            dispatch_svn(intent, &repo_info.root, url, username, password).await
        }
    };

    match vcs_result {
        Ok(r) => serde_json::to_value(&r).map_err(|e| e.to_string()),
        Err(msg) => {
            // Translate no_token sentinel into the proper NoToken variant.
            if let Some(level) = msg.strip_prefix("no_token:") {
                let lvl: u8 = level.parse().unwrap_or(2);
                let r = VcsResult::NoToken { required_level: lvl };
                return serde_json::to_value(&r).map_err(|e| e.to_string());
            }
            let r = VcsResult::Error { message: msg };
            serde_json::to_value(&r).map_err(|e| e.to_string())
        }
    }
}

/// Parse AI response into a VcsAgentDecision (reuses strip_json_fences logic).
/// If full parse fails (e.g. AI invented an unknown intent kind), attempt graceful recovery:
/// treat the response as done=true using the summary as the final answer.
fn parse_vcs_agent_decision(response: &str) -> Result<VcsAgentDecision, String> {
    let cleaned = strip_json_fences(response);
    if let Ok(decision) = serde_json::from_str::<VcsAgentDecision>(&cleaned) {
        return Ok(decision);
    }
    // Graceful recovery: extract summary/final_answer from the raw JSON and
    // return done=true so the loop terminates cleanly instead of erroring.
    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&cleaned) {
        let summary = raw["summary"].as_str().unwrap_or("（AI 回應無法解析）").to_string();
        let final_answer = raw["final_answer"].as_str().map(|s| s.to_string())
            .or_else(|| Some(summary.clone()));
        return Ok(VcsAgentDecision {
            done: true,
            intent: None,
            summary,
            final_answer,
        });
    }
    Err(format!("Failed to parse VcsAgentDecision. Raw: {cleaned}"))
}

#[tauri::command]
pub async fn vcs_agent_step(
    goal: String,
    history: Vec<VcsAgentHistoryEntry>,
    repo_info: VcsRepoInfo,
    session_id: String,
    provider_id: Option<String>,
    router: State<'_, AiRouter>,
    step_registry: State<'_, VcsAgentStepRegistry>,
) -> Result<serde_json::Value, String> {
    // Resolve the provider (optional override)
    let provider = match provider_id.as_deref() {
        Some(id) => router.resolve_by_id(id).await.map_err(|e| e.to_string())?,
        None => router.resolve().await.map_err(|e| e.to_string())?,
    };

    // Build history context for the prompt.
    // result_json can be very large (full diffs, many commits) — truncate per entry
    // to avoid exceeding the model's context window.
    const MAX_RESULT_JSON_CHARS: usize = 3000;
    let history_text = history
        .iter()
        .enumerate()
        .map(|(i, entry)| match entry {
            VcsAgentHistoryEntry::User { text } => {
                format!("[Message {}] User: {}", i + 1, text)
            }
            VcsAgentHistoryEntry::Step { step_num, operation, result_json, summary } => {
                let truncated = if result_json.len() > MAX_RESULT_JSON_CHARS {
                    format!(
                        "{}... [truncated — {} chars total]",
                        &result_json[..MAX_RESULT_JSON_CHARS],
                        result_json.len()
                    )
                } else {
                    result_json.clone()
                };
                format!(
                    "[Step {}] Operation: {}\nResult (JSON): {}\nAI summary: {}",
                    step_num, operation, truncated, summary
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let system_prompt = format!(
        r#"You are a VCS agent that plans step-by-step operations to achieve the user's goal.

Goal: {goal}

History so far:
{history_context}

Available VCS operations — intent.kind MUST be EXACTLY one of these values (never invent new kinds):
  log_query        {{ path?, author?, since?, max_count (u32, default 20) }}
  diff_view        {{ revision }}
  blame            {{ path }}
  branch_list      {{}}
  pr_list          {{ state? ("open"|"closed"|"all") }}
  issue_list       {{ state? ("open"|"closed"|"all") }}
  actions_list     {{}}
  revert_commit    {{ revision }}
  cherry_pick      {{ revision }}
  create_pr        {{ title, head, base, body? }}
  merge_pr         {{ pr_number (u64) }}
  create_issue     {{ title, body? }}
  trigger_workflow {{ workflow_id, ref }}
  create_branch    {{ name, from? }}
  delete_branch    {{ name }}
  checkout_branch  {{ name }}
  svn_commit       {{ message, paths: [] }}
  svn_revert       {{ paths: [] }}
  svn_update       {{ path? }}

Repo type: {vcs_type}

RULES:
1. intent.kind MUST be one of the listed values above. NEVER use "summarize", "analyze", "report", or any other kind not in the list.
2. If you have enough information from the history to answer the goal — including when you want to summarize or synthesize what you found — return done=true with final_answer. Do NOT invent a fake intent to summarize.
3. Only return done=false when you genuinely need to execute another VCS operation to gather more data.

Decide what to do next. Output ONLY a JSON object with this schema:
{{
  "done": false,
  "intent": {{ "kind": "<one of the listed kinds>", ...fields }},
  "summary": "one sentence describing what you're doing",
  "final_answer": null
}}

OR if the goal is achieved (including when you want to summarize what you found):
{{
  "done": true,
  "intent": null,
  "summary": "one sentence summary",
  "final_answer": "complete answer to the user's goal"
}}

Output ONLY the JSON object. No prose, no markdown fences."#,
        goal = goal,
        history_context = if history_text.is_empty() { "(no history yet — this is the first step)".to_string() } else { history_text },
        vcs_type = format!("{:?}", repo_info.vcs_type).to_lowercase(),
    );

    let req = GenerateRequest {
        system_prompt,
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!(format!("Plan the next step to achieve: {goal}")),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: crate::ai::EnvSnapshot::default(),
        mode: QueryMode::SingleCommand,
        max_tokens: Some(1024),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_clone = provider.clone();
    let join = tokio::spawn(async move { provider_clone.generate(req, tx).await });

    step_registry
        .0
        .lock()
        .unwrap()
        .insert(session_id.clone(), join.abort_handle());

    let recv_outcome = tokio::time::timeout(VCS_AGENT_STEP_TIMEOUT, async {
        let mut buf = String::new();
        while let Some(chunk) = rx.recv().await {
            buf.push_str(&chunk.delta);
            if chunk.done {
                break;
            }
        }
        buf
    })
    .await;

    step_registry.0.lock().unwrap().remove(&session_id);

    let buf = match recv_outcome {
        Ok(buf) => buf,
        Err(_) => {
            join.abort();
            return Err("AI 回應逾時（60 秒），已中止此步驟".to_string());
        }
    };

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.to_string()),
        Err(e) if e.is_cancelled() => return Err("已停止".to_string()),
        Err(e) => return Err(e.to_string()),
    }

    let decision = parse_vcs_agent_decision(&buf)?;
    serde_json::to_value(&decision).map_err(|e| e.to_string())
}

/// Cancels the in-flight AI call for a `vcs_agent_step` invocation, if any is
/// still running for this session. No-op if the step already finished.
#[tauri::command]
pub async fn vcs_agent_abort_step(
    session_id: String,
    step_registry: State<'_, VcsAgentStepRegistry>,
) -> Result<(), String> {
    if let Some(handle) = step_registry.0.lock().unwrap().remove(&session_id) {
        handle.abort();
    }
    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/// Parse a natural-language VCS query into a `VcsIntent` using the AI router.
async fn parse_vcs_intent(query: &str, router: &AiRouter) -> Result<VcsIntent, String> {
    let provider = router.resolve().await.map_err(|e| e.to_string())?;

    let system_prompt = r#"You are a VCS intent parser. Translate the user's natural-language request into a JSON object matching exactly one of these intent variants. Output ONLY a JSON object, no prose, no markdown fences.

Available intents (use the exact "kind" value):
  log_query        { path?, author?, since?, max_count (u32, default 20) }
  diff_view        { revision }
  blame            { path }
  branch_list      {}
  pr_list          { state? ("open"|"closed"|"all") }
  issue_list       { state? ("open"|"closed"|"all") }
  actions_list     {}
  revert_commit    { revision }
  cherry_pick      { revision }
  create_pr        { title, head, base, body? }
  merge_pr         { pr_number (u64) }
  create_issue     { title, body? }
  trigger_workflow { workflow_id, ref }
  create_branch    { name, from? }
  delete_branch    { name }
  checkout_branch  { name }
  svn_commit       { message, paths: [] }
  svn_revert       { paths: [] }
  svn_update       { path? }

Example output: {"kind":"log_query","path":null,"author":null,"since":null,"max_count":20}
"#;

    let req = GenerateRequest {
        system_prompt: system_prompt.to_string(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: serde_json::json!(query),
            tool_call_id: None,
            tool_calls: None,
        }],
        context: crate::ai::EnvSnapshot::default(),
        mode: QueryMode::SingleCommand,
        max_tokens: Some(512),
    };

    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_clone = provider.clone();
    let join = tokio::spawn(async move { provider_clone.generate(req, tx).await });

    let mut buf = String::new();
    while let Some(chunk) = rx.recv().await {
        buf.push_str(&chunk.delta);
        if chunk.done {
            break;
        }
    }

    match join.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e.to_string()),
        Err(e) => return Err(e.to_string()),
    }

    // Strip markdown fences if present
    let cleaned = strip_json_fences(&buf);

    serde_json::from_str::<VcsIntent>(&cleaned)
        .map_err(|e| format!("Failed to parse VCS intent: {e}. Raw: {cleaned}"))
}

fn strip_json_fences(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        if let Some(newline) = trimmed.find('\n') {
            let inner = &trimmed[newline + 1..];
            if let Some(close) = inner.rfind("```") {
                return inner[..close].trim().to_string();
            }
            return inner.trim().to_string();
        }
    }
    // Find first '{' to skip any preamble
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            return trimmed[start..=end].to_string();
        }
    }
    trimmed.to_string()
}

fn is_write_intent(intent: &VcsIntent) -> bool {
    matches!(
        intent,
        VcsIntent::RevertCommit { .. }
            | VcsIntent::CherryPick { .. }
            | VcsIntent::CreatePr { .. }
            | VcsIntent::MergePr { .. }
            | VcsIntent::CreateIssue { .. }
            | VcsIntent::TriggerWorkflow { .. }
            | VcsIntent::CreateBranch { .. }
            | VcsIntent::DeleteBranch { .. }
            | VcsIntent::CheckoutBranch { .. }
            | VcsIntent::SvnCommit { .. }
            | VcsIntent::SvnRevert { .. }
            | VcsIntent::SvnUpdate { .. }
    )
}

fn intent_operation_name(intent: &VcsIntent) -> &'static str {
    match intent {
        VcsIntent::LogQuery { .. } => "log_query",
        VcsIntent::DiffView { .. } => "diff_view",
        VcsIntent::Blame { .. } => "blame",
        VcsIntent::BranchList => "branch_list",
        VcsIntent::PrList { .. } => "pr_list",
        VcsIntent::IssueList { .. } => "issue_list",
        VcsIntent::ActionsList => "actions_list",
        VcsIntent::RevertCommit { .. } => "revert_commit",
        VcsIntent::CherryPick { .. } => "cherry_pick",
        VcsIntent::CreatePr { .. } => "create_pr",
        VcsIntent::MergePr { .. } => "merge_pr",
        VcsIntent::CreateIssue { .. } => "create_issue",
        VcsIntent::TriggerWorkflow { .. } => "trigger_workflow",
        VcsIntent::CreateBranch { .. } => "create_branch",
        VcsIntent::DeleteBranch { .. } => "delete_branch",
        VcsIntent::CheckoutBranch { .. } => "checkout_branch",
        VcsIntent::SvnCommit { .. } => "svn_commit",
        VcsIntent::SvnRevert { .. } => "svn_revert",
        VcsIntent::SvnUpdate { .. } => "svn_update",
    }
}

fn describe_intent(intent: &VcsIntent) -> String {
    match intent {
        VcsIntent::RevertCommit { revision } => format!("Revert commit {revision}"),
        VcsIntent::CherryPick { revision } => format!("Cherry-pick commit {revision}"),
        VcsIntent::CreatePr { title, head, base, .. } => {
            format!("Create PR: '{title}' ({head} → {base})")
        }
        VcsIntent::MergePr { pr_number } => format!("Merge PR #{pr_number}"),
        VcsIntent::CreateIssue { title, .. } => format!("Create issue: '{title}'"),
        VcsIntent::TriggerWorkflow { workflow_id, r#ref } => {
            format!("Trigger workflow '{workflow_id}' on '{ref}'")
        }
        VcsIntent::CreateBranch { name, from } => {
            if let Some(f) = from {
                format!("Create branch '{name}' from '{f}'")
            } else {
                format!("Create branch '{name}'")
            }
        }
        VcsIntent::DeleteBranch { name } => format!("Delete branch '{name}'"),
        VcsIntent::CheckoutBranch { name } => format!("Checkout branch '{name}'"),
        VcsIntent::SvnCommit { message, paths } => {
            format!("SVN commit '{}' ({} path(s))", message, paths.len())
        }
        VcsIntent::SvnRevert { paths } => format!("SVN revert {} path(s)", paths.len()),
        VcsIntent::SvnUpdate { path } => {
            format!("SVN update {}", path.as_deref().unwrap_or("."))
        }
        other => format!("{:?}", other),
    }
}

async fn dispatch_git(
    intent: VcsIntent,
    repo_root: &str,
    token: Option<String>,
    _session_id: String,
) -> Result<VcsResult, String> {
    let client = GitClient::new(repo_root.to_string(), token);

    match intent {
        VcsIntent::LogQuery { path, author, since, max_count } => {
            client
                .log(path.as_deref(), author.as_deref(), since.as_deref(), max_count)
                .await
        }
        VcsIntent::DiffView { revision } => client.show(&revision).await,
        VcsIntent::Blame { path } => client.blame(&path).await,
        VcsIntent::BranchList => client.branch_list().await,
        VcsIntent::PrList { state } => client.pr_list(state.as_deref()).await,
        VcsIntent::IssueList { state } => client.issue_list(state.as_deref()).await,
        VcsIntent::ActionsList => client.actions_list().await,
        VcsIntent::RevertCommit { revision } => client.revert(&revision).await,
        VcsIntent::CherryPick { revision } => client.cherry_pick(&revision).await,
        VcsIntent::CreatePr { title, head, base, body } => {
            client
                .create_pr(&title, &head, &base, body.as_deref(), false)
                .await
                .map(|(number, pr_url)| VcsResult::WriteSuccess {
                    operation: "create_pr".to_string(),
                    detail: format!("Created PR #{number}: {pr_url}"),
                })
        }
        VcsIntent::MergePr { pr_number } => client.merge_pr(pr_number).await,
        VcsIntent::CreateIssue { title, body } => {
            client.create_issue(&title, body.as_deref()).await
        }
        VcsIntent::TriggerWorkflow { workflow_id, r#ref } => {
            client.trigger_workflow(&workflow_id, &r#ref).await
        }
        VcsIntent::CreateBranch { name, from } => {
            client.create_branch(&name, from.as_deref()).await
        }
        VcsIntent::DeleteBranch { name } => client.delete_branch(&name).await,
        VcsIntent::CheckoutBranch { name } => client.checkout_branch(&name).await,
        VcsIntent::SvnCommit { .. } | VcsIntent::SvnRevert { .. } | VcsIntent::SvnUpdate { .. } => {
            Ok(VcsResult::Error {
                message: "SVN operations are not supported on a Git repository".into(),
            })
        }
    }
}

async fn dispatch_svn(
    intent: VcsIntent,
    working_copy_root: &str,
    url: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<VcsResult, String> {
    let client = SvnClient::new(working_copy_root.to_string(), url, username, password);

    match intent {
        VcsIntent::LogQuery { path, max_count, .. } => {
            client.log(path.as_deref(), max_count).await
        }
        VcsIntent::DiffView { revision } => client.diff(&revision).await,
        VcsIntent::Blame { path } => client.blame(&path).await,
        VcsIntent::SvnCommit { message, paths } => client.commit(&message, &paths).await,
        VcsIntent::SvnRevert { paths } => client.revert(&paths).await,
        VcsIntent::SvnUpdate { path } => client.update(path.as_deref()).await,
        _ => Ok(VcsResult::Error {
            message: "This operation is not supported for SVN repositories".into(),
        }),
    }
}

fn resolve_svn_credentials(
    repo_info: &VcsRepoInfo,
    config: &ConfigStore,
    secrets: &SecretStore,
) -> (String, Option<String>, Option<String>) {
    let (url, username) = repo_info
        .connection_id
        .as_deref()
        .and_then(|id| {
            config
                .get()
                .vcs_connections
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| (c.url.unwrap_or_default(), c.username))
        })
        .unwrap_or_else(|| {
            (
                repo_info.remote_url.clone().unwrap_or_default(),
                None,
            )
        });

    let password = repo_info
        .connection_id
        .as_deref()
        .and_then(|id| secrets.get(&vcs_secret_key(id)).ok().flatten());

    (url, username, password)
}

/// Normalise two VCS URLs for comparison: strip trailing slashes and `.git` suffix.
fn urls_match(a: &str, b: &str) -> bool {
    normalise_url(a) == normalise_url(b)
}

fn normalise_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_lowercase()
}

/// Opens a native OS folder picker and returns the selected path, or None if cancelled.
#[tauri::command]
pub async fn pick_folder() -> Option<String> {
    rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|handle| handle.path().to_string_lossy().to_string())
}

#[derive(Debug, Serialize)]
pub struct StartFeatureOutcome {
    pub branch_name: String,
    pub pr_number: u64,
    pub pr_url: String,
}

#[tauri::command]
pub async fn vcs_list_active_features(
    repo_info: VcsRepoInfo,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<crate::vcs::ActiveFeature>, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.list_active_features().await
}

#[tauri::command]
pub async fn vcs_check_overlap(
    repo_info: VcsRepoInfo,
    files: Vec<String>,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<Vec<crate::vcs::ActiveFeature>, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    let features = client.list_active_features().await?;
    Ok(crate::vcs::overlap::find_overlaps(&files, &features))
}

/// 把使用者宣告會動到的檔案清單格式化成 PR body 的一段文字；沒有宣告任何檔案就回傳空字串。
fn format_declared_files_body(declared_files: &[String]) -> String {
    if declared_files.is_empty() {
        String::new()
    } else {
        format!(
            "預計會動到的檔案：\n{}",
            declared_files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n")
        )
    }
}

/// 產生一段以目前時間為基礎的簡短十六進位字尾，用來讓不同人各自建立的分支名稱不會撞名
/// （刻意不查詢 GitHub 使用者身分——那是本功能明確排除的能力）。
fn unique_branch_suffix() -> String {
    format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            % 0xFFFFFF
    )
}

#[tauri::command]
pub async fn vcs_start_feature(
    repo_info: VcsRepoInfo,
    feature_name: String,
    base_branch: String,
    declared_files: Vec<String>,
    secrets: State<'_, Arc<SecretStore>>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<StartFeatureOutcome, String> {
    if resolve_write_mode(&repo_info, &config) == VcsWriteMode::ReadOnly {
        return Err("此連線為唯讀模式，無法開始新功能".to_string());
    }

    let slug = slugify(&feature_name);
    if slug.is_empty() {
        return Err("功能名稱需要至少包含一個英文字母或數字".to_string());
    }
    // A short suffix derived from the current time (not a GitHub username —
    // fetching "who am I on GitHub" is a separate capability this feature
    // deliberately doesn't have) guarantees two teammates starting
    // similarly-named features never collide on the same branch name.
    let branch_name = format!("feature/{slug}-{}", unique_branch_suffix());

    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);

    client.create_branch(&branch_name, Some(&base_branch)).await?;
    // A freshly-created branch has zero commits ahead of base and doesn't
    // exist on the remote yet — GitHub's create-PR API rejects both of
    // those. An empty commit plus a push satisfies it before we call
    // create_pr below.
    client.commit_empty(&format!("Start feature: {feature_name}")).await?;
    if let Err(e) = client.push_branch(&branch_name).await {
        // Best-effort cleanup so a failed push doesn't leave the user stuck on
        // an orphaned local branch with an unpushed commit — mirrors the same
        // best-effort-cleanup pattern already used in vcs_merge_feature.
        let _ = client.checkout_branch(&base_branch).await;
        let _ = client.delete_branch_force(&branch_name).await;
        return Err(e);
    }

    let body = format_declared_files_body(&declared_files);
    let (pr_number, pr_url) = client
        .create_pr(&feature_name, &branch_name, &base_branch, Some(&body), true)
        .await?;

    Ok(StartFeatureOutcome { branch_name, pr_number, pr_url })
}

#[tauri::command]
pub async fn vcs_finish_feature(
    repo_info: VcsRepoInfo,
    pr_number: u64,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<(), String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.mark_pr_ready(pr_number).await?;
    Ok(())
}

#[tauri::command]
pub async fn vcs_get_feature_diff(
    repo_info: VcsRepoInfo,
    base: String,
    head: String,
    secrets: State<'_, Arc<SecretStore>>,
) -> Result<String, String> {
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    match client.pr_diff(&base, &head).await? {
        VcsResult::Diff { content, .. } => Ok(content),
        _ => Err("pr_diff 回傳了非預期的結果型別".to_string()),
    }
}

#[tauri::command]
pub async fn vcs_merge_feature(
    repo_info: VcsRepoInfo,
    pr_number: u64,
    branch_to_delete: Option<String>,
    secrets: State<'_, Arc<SecretStore>>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<(), String> {
    if resolve_write_mode(&repo_info, &config) == VcsWriteMode::ReadOnly {
        return Err("此連線為唯讀模式，無法合併".to_string());
    }
    let token = resolve_vcs_token(&repo_info, &secrets);
    let client = GitClient::new(repo_info.root, token);
    client.merge_pr(pr_number).await?;
    if let Some(branch) = branch_to_delete {
        // 合併已經成功了；刪分支失敗不該讓整個操作回報失敗，忽略即可。
        let _ = client.delete_branch(&branch).await;
    }
    Ok(())
}

#[cfg(test)]
mod slugify_tests {
    use super::slugify;

    #[test]
    fn ascii_words_become_hyphenated_lowercase() {
        assert_eq!(slugify("Login Fix"), "login-fix");
    }

    #[test]
    fn chinese_characters_are_dropped_not_kept() {
        assert_eq!(slugify("登入頁優化"), "");
    }

    #[test]
    fn mixed_chinese_and_ascii_keeps_only_ascii() {
        assert_eq!(slugify("登入頁 Login 優化"), "login");
    }

    #[test]
    fn consecutive_separators_collapse_to_one_hyphen() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }
}

#[cfg(test)]
mod format_declared_files_body_tests {
    use super::format_declared_files_body;

    #[test]
    fn empty_list_returns_empty_string() {
        assert_eq!(format_declared_files_body(&[]), "");
    }

    #[test]
    fn single_file_is_a_single_bullet_line_under_the_header() {
        let files = vec!["src/foo.rs".to_string()];
        assert_eq!(format_declared_files_body(&files), "預計會動到的檔案：\n- src/foo.rs");
    }

    #[test]
    fn multiple_files_are_each_on_their_own_line_in_order() {
        let files = vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()];
        assert_eq!(
            format_declared_files_body(&files),
            "預計會動到的檔案：\n- src/foo.rs\n- src/bar.rs"
        );
    }
}

#[cfg(test)]
mod unique_branch_suffix_tests {
    use super::unique_branch_suffix;

    #[test]
    fn suffix_is_non_empty_lowercase_hex() {
        let suffix = unique_branch_suffix();
        assert!(!suffix.is_empty());
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn branch_name_includes_slug_and_a_suffix_beyond_it() {
        let slug = "login-fix";
        let branch_name = format!("feature/{slug}-{}", unique_branch_suffix());
        let suffix_part = branch_name
            .strip_prefix("feature/login-fix-")
            .expect("branch name should start with feature/{slug}-");
        assert!(!suffix_part.is_empty(), "expected a non-empty suffix after the slug");
        assert!(suffix_part.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

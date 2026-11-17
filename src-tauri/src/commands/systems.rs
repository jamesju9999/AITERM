//! 系統連線（外部網頁系統）的設定指令。見
//! docs/superpowers/specs/2026-11-17-system-connections-design.md。

use std::sync::Arc;

use tauri::State;

use crate::config::types::{McpServerConfig, McpTransport};
use crate::config::ConfigStore;
use crate::secret::SecretStore;
use crate::systems::approvals::PendingView;
use crate::systems::audit;
use crate::systems::endpoint::{EndpointState, DEFAULT_PORT, ENDPOINT_TOKEN_KEY, MCP_SERVER_ID};
use crate::systems::gate::host_of;
use crate::systems::state::{SystemStatus, SystemsState};
use crate::systems::types::SystemConnection;

/// 儲存前的檢查：網址要是 http(s)；名稱不可空白；允許的網域必須包含起始網址的網域。
pub fn validate(conn: &SystemConnection) -> Result<(), String> {
    if conn.name.trim().is_empty() {
        return Err("名稱不可空白".into());
    }
    if !(conn.start_url.starts_with("http://") || conn.start_url.starts_with("https://")) {
        return Err("起始網址必須是 http 或 https".into());
    }
    let host = host_of(&conn.start_url).ok_or("起始網址不正確")?;
    let covered = conn.allowed_hosts.iter().any(|a| {
        let a = a.trim().trim_start_matches("*.").to_ascii_lowercase();
        !a.is_empty() && (host == a || host.ends_with(&format!(".{a}")))
    });
    if !covered {
        return Err("允許的網域必須包含起始網址的網域".into());
    }
    if let Some(m) = conn.model.as_deref().filter(|m| !m.trim().is_empty()) {
        if crate::systems::jobs::clean_model(Some(m)).is_none() {
            return Err("模型名稱只能用英數與 . _ - [ ] :".into());
        }
    }
    Ok(())
}

type McpState = crate::commands::mcp::McpManagerState;

/// 啟動端點、確保 token，並在 `mcp_servers` 維護一筆自動管理的項目（讓 Ask AI 迴圈看得到 `sys_*` 工具）。
pub async fn ensure_endpoint(
    endpoint: &EndpointState,
    systems: &Arc<SystemsState>,
    secrets: &Arc<SecretStore>,
    config: &Arc<ConfigStore>,
    mcp: &McpState,
) -> Result<(), String> {
    // 鑰匙圈不可用時（例如沒有預設鑰匙圈的環境）退回只在記憶體裡的 token：它只需要在
    // 端點與自動維護的 MCP 設定之間一致，每次啟動換一個也不影響使用。
    let token = match endpoint.token() {
        Some(t) => t, // 端點已在跑：沿用它的 token，不可另產生（否則與已寫進設定的對不上）
        None => match secrets.get(ENDPOINT_TOKEN_KEY) {
            Ok(Some(t)) if !t.is_empty() => t,
            _ => {
                let t = crate::bridge::auth::generate_token();
                if let Err(e) = secrets.set(ENDPOINT_TOKEN_KEY, &t) {
                    log::warn!("systems 端點 token 無法存進鑰匙圈，只在這次執行有效：{e}");
                }
                t
            }
        },
    };
    endpoint.start(systems.clone(), token.clone(), DEFAULT_PORT).await?;
    let desired = McpServerConfig {
        id: MCP_SERVER_ID.into(),
        name: "AITerm Systems".into(),
        enabled: true,
        transport: McpTransport::Http,
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(format!("http://127.0.0.1:{DEFAULT_PORT}/systems/mcp")),
        headers: [("Authorization".to_string(), format!("Bearer {token}"))].into_iter().collect(),
    };
    let current = config.get().mcp_servers.into_iter().find(|s| s.id == MCP_SERVER_ID);
    let same = current.as_ref().is_some_and(|c| c.url == desired.url && c.headers == desired.headers && c.enabled);
    if !same {
        config
            .update(|cfg| match cfg.mcp_servers.iter().position(|s| s.id == MCP_SERVER_ID) {
                Some(i) => cfg.mcp_servers[i] = desired.clone(),
                None => cfg.mcp_servers.push(desired.clone()),
            })
            .map_err(|e| e.to_string())?;
        mcp.lock().await.reconnect(&desired).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn systems_list(config: State<'_, Arc<ConfigStore>>) -> Result<Vec<SystemConnection>, String> {
    Ok(config.get().system_connections)
}

/// 新增或取代（同 id）。存好後確保端點在跑；系統被停用時立刻關掉它的瀏覽器。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn systems_save(
    conn: SystemConnection,
    config: State<'_, Arc<ConfigStore>>,
    systems: State<'_, Arc<SystemsState>>,
    endpoint: State<'_, Arc<EndpointState>>,
    secrets: State<'_, Arc<SecretStore>>,
    mcp: State<'_, McpState>,
) -> Result<(), String> {
    validate(&conn)?;
    config
        .update(|cfg| match cfg.system_connections.iter().position(|c| c.id == conn.id) {
            Some(i) => cfg.system_connections[i] = conn.clone(),
            None => cfg.system_connections.push(conn.clone()),
        })
        .map_err(|e| e.to_string())?;
    if !conn.enabled {
        systems.close(&conn.id).await;
    }
    ensure_endpoint(&endpoint, &systems, &secrets, &config, &mcp).await
}

#[tauri::command]
pub async fn systems_remove(
    id: String,
    config: State<'_, Arc<ConfigStore>>,
    systems: State<'_, Arc<SystemsState>>,
) -> Result<(), String> {
    systems.close(&id).await;
    config.update(|cfg| cfg.system_connections.retain(|c| c.id != id)).map_err(|e| e.to_string())
}

/// 開啟這個系統的瀏覽器視窗（沒開就開），讓使用者手動登入。
#[tauri::command]
pub async fn systems_open_login(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    systems.inner().clone().ensure_running(&id).await.map(|_| ())
}

#[tauri::command]
pub async fn systems_status(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<SystemStatus, String> {
    Ok(systems.status(&id).await)
}

#[tauri::command]
pub async fn systems_pending(systems: State<'_, Arc<SystemsState>>) -> Result<Vec<PendingView>, String> {
    Ok(systems.pending())
}

#[tauri::command]
pub async fn systems_decide(approval_id: String, approve: bool, systems: State<'_, Arc<SystemsState>>) -> Result<bool, String> {
    Ok(systems.approvals.decide(&approval_id, approve))
}

#[tauri::command]
pub async fn systems_stop(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<usize, String> {
    Ok(systems.stop(&id).await)
}

#[tauri::command]
pub async fn systems_takeover(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    systems.set_manual(&id, true).await
}

#[tauri::command]
pub async fn systems_handback(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    systems.set_manual(&id, false).await
}

#[tauri::command]
pub async fn systems_audit(id: String, limit: Option<i64>, systems: State<'_, Arc<SystemsState>>) -> Result<Vec<audit::Row>, String> {
    let Some(pool) = systems.audit_pool().await else { return Ok(vec![]) };
    audit::list(&pool, &id, limit.unwrap_or(100).clamp(1, 500)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn systems_recipes(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<Vec<crate::systems::recipes::Recipe>, String> {
    systems.recipe_list(&id).await
}

#[tauri::command]
pub async fn systems_recipe_delete(recipe_id: i64, systems: State<'_, Arc<SystemsState>>) -> Result<bool, String> {
    systems.recipe_delete(recipe_id).await
}

// ── 定時任務 ────────────────────────────────────────────────────────────────

use crate::systems::jobs::{self, Job, JobRun, JobsCtx, JobsRuntime, NewJob};

#[tauri::command]
pub async fn systems_jobs(system_id: Option<String>, systems: State<'_, Arc<SystemsState>>) -> Result<Vec<Job>, String> {
    let pool = systems.audit_pool().await.ok_or("systems.db 不可用")?;
    jobs::list(&pool, system_id.as_deref()).await.map_err(|e| e.to_string())
}

/// 新增（`id` 為空）或更新。回傳任務 id。
#[tauri::command]
pub async fn systems_job_save(id: Option<String>, job: NewJob, systems: State<'_, Arc<SystemsState>>) -> Result<String, String> {
    systems.connection(&job.system_id)?;
    let pool = systems.audit_pool().await.ok_or("systems.db 不可用")?;
    match id {
        Some(id) => jobs::update(&pool, &id, &job).await.map(|_| id),
        None => jobs::create(&pool, &job).await,
    }
}

#[tauri::command]
pub async fn systems_job_delete(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    let pool = systems.audit_pool().await.ok_or("systems.db 不可用")?;
    jobs::delete(&pool, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn systems_job_set_enabled(id: String, enabled: bool, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    let pool = systems.audit_pool().await.ok_or("systems.db 不可用")?;
    jobs::set_enabled(&pool, &id, enabled).await.map_err(|e| e.to_string())
}

/// 現在執行：丟到背景，完成時畫面會收到 `systems://job-finished`。
#[tauri::command]
pub async fn systems_job_run_now(
    id: String,
    rt: State<'_, Arc<JobsRuntime>>,
    ctx: State<'_, Arc<JobsCtx>>,
) -> Result<(), String> {
    if rt.is_running(&id) {
        return Err("這個任務正在執行".into());
    }
    let (rt, ctx) = (rt.inner().clone(), ctx.inner().clone());
    tokio::spawn(async move {
        if let Err(e) = rt.run_job(&ctx, &id).await {
            log::warn!("systems job {id}: {e}");
        }
    });
    Ok(())
}

/// 「請 AI 操作」：跑一次，回傳結果與錄到的步驟（可存成配方時日誌會保留）。
#[tauri::command]
pub async fn systems_run_goal(
    system_id: String,
    goal: String,
    model: Option<String>,
    rt: State<'_, Arc<JobsRuntime>>,
    ctx: State<'_, Arc<JobsCtx>>,
) -> Result<jobs::AdhocReport, String> {
    let (rt, ctx) = (rt.inner().clone(), ctx.inner().clone());
    rt.run_adhoc(&ctx, &system_id, &goal, model.as_deref()).await
}

/// 把剛剛一次性執行錄到的步驟存成配方（使用者確認過名稱與參數）。
#[tauri::command]
pub async fn systems_recipe_save_from_journal(
    system_id: String,
    name: String,
    description: String,
    params: Vec<crate::systems::recipes::Param>,
    systems: State<'_, Arc<SystemsState>>,
) -> Result<String, String> {
    let params: Vec<_> = params.into_iter().filter(|p| !p.name.trim().is_empty() && !p.example.is_empty()).collect();
    systems.recipe_save(&system_id, &name, &description, params, None).await
}

/// 不存配方：清掉錄到的步驟。
#[tauri::command]
pub async fn systems_journal_discard(system_id: String, systems: State<'_, Arc<SystemsState>>) -> Result<(), String> {
    systems.journal_clear(&system_id).await;
    Ok(())
}

/// 配方的「執行」：帶入參數值直接重播。
#[tauri::command]
pub async fn systems_recipe_run(
    system_id: String,
    name: String,
    params: std::collections::HashMap<String, String>,
    rt: State<'_, Arc<JobsRuntime>>,
    ctx: State<'_, Arc<JobsCtx>>,
) -> Result<String, String> {
    let (rt, ctx) = (rt.inner().clone(), ctx.inner().clone());
    rt.run_recipe_locked(&ctx, &system_id, &name, params).await
}

#[tauri::command]
pub async fn systems_job_runs(id: String, systems: State<'_, Arc<SystemsState>>) -> Result<Vec<JobRun>, String> {
    let pool = systems.audit_pool().await.ok_or("systems.db 不可用")?;
    jobs::list_runs(&pool, &id).await.map_err(|e| e.to_string())
}

/// 給 Claude Code 貼的 MCP 設定。
#[tauri::command]
pub async fn systems_mcp_config(secrets: State<'_, Arc<SecretStore>>) -> Result<String, String> {
    let token = secrets.get(ENDPOINT_TOKEN_KEY).ok().flatten().unwrap_or_default();
    // 端點還沒啟動（沒有任何系統連線）或鑰匙圈不可用時沒有 token：不給一份不能用的設定。
    if token.is_empty() {
        return Ok(String::new());
    }
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { MCP_SERVER_ID: {
            "type": "http",
            "url": format!("http://127.0.0.1:{DEFAULT_PORT}/systems/mcp"),
            "headers": { "Authorization": format!("Bearer {token}") }
        } }
    }))
    .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems::types::Mode;

    fn ok() -> SystemConnection {
        SystemConnection {
            id: "s".into(),
            name: "訂單系統".into(),
            start_url: "https://app.example.com/login".into(),
            allowed_hosts: vec!["example.com".into()],
            mode: Mode::ReadOnly,
            rules: vec![],
            enabled: true, model: None,
        }
    }

    #[test]
    fn a_proper_connection_passes() {
        assert!(validate(&ok()).is_ok());
    }

    #[test]
    fn rejects_blank_name_non_http_and_uncovered_host() {
        let mut c = ok();
        c.name = "  ".into();
        assert!(validate(&c).unwrap_err().contains("名稱"));

        let mut c = ok();
        c.start_url = "ftp://app.example.com/".into();
        assert!(validate(&c).unwrap_err().contains("http"));

        let mut c = ok();
        c.allowed_hosts = vec!["other.test".into()];
        assert!(validate(&c).unwrap_err().contains("網域"));

        let mut c = ok();
        c.allowed_hosts = vec![];
        assert!(validate(&c).is_err(), "白名單空的不能存");

        let mut c = ok();
        c.model = Some("sonnet".into());
        assert!(validate(&c).is_ok());
        c.model = Some("a b; rm".into());
        assert!(validate(&c).unwrap_err().contains("模型"));
    }
}

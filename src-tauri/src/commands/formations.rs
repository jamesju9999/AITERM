//! 團隊編制的 Tauri 指令。薄薄一層委派給 `tasks::formations`，每次變動後送
//! `tasks-updated` 並叫醒排程器（等待中的切換可能可以完成了）。
//! 見 `docs/superpowers/specs/2026-10-20-agent-canvas-team-formations-design.md`。

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::projects::ProjectRegistry;
use crate::tasks::formations::{self, FormationAgent, FormationsState, RetiredAgent, SwitchOutcome};
use crate::tasks::repo_check::{check_repo, RepoCheck};
use crate::tasks::scheduler::SchedulerHandle;

use super::tasks::{emit_updated, project};
use crate::intake::board_remote as remote;

/// 共用專案：取得連線；`Some((api, server 專案 id))`。
fn shared(rs: &crate::intake::remote::RemoteState, project_id: &str) -> Result<Option<(aiterm_intake_client::api::Api, String)>, String> {
    match remote::parse(project_id) {
        Some(pid) => Ok(Some((remote::require(rs)?, pid.to_string()))),
        None => Ok(None),
    }
}

fn s(e: sqlx::Error) -> String {
    e.to_string()
}

/// 編制狀態。舊專案（桌布上有 Agent、還沒有任何編制）第一次打開時先建立 `default_name`。
#[tauri::command]
pub async fn formations_state(project_id: String, default_name: String, reg: State<'_, ProjectRegistry>, rs: State<'_, Arc<crate::intake::remote::RemoteState>>) -> Result<serde_json::Value, String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        return remote::formations_state(&api, &pid, &default_name).await.map_err(|e| remote::fail(&rs, e));
    }
    let p = project(&reg, &project_id)?;
    formations::ensure_default(&p.pool, &default_name).await.map_err(s)?;
    let st: FormationsState = formations::state(&p.pool).await.map_err(s)?;
    serde_json::to_value(st).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn formations_create(
    project_id: String,
    name: String,
    agents: Vec<FormationAgent>,
    reg: State<'_, ProjectRegistry>,
    rs: State<'_, Arc<crate::intake::remote::RemoteState>>,
    app: AppHandle,
) -> Result<String, String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        let id = remote::formations_create(&api, &pid, &name, &serde_json::to_value(&agents).map_err(|e| e.to_string())?).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(id);
    }
    let p = project(&reg, &project_id)?;
    let id = formations::create(&p.pool, &name, &agents).await.map_err(s)?;
    emit_updated(&app);
    Ok(id)
}

#[tauri::command]
pub async fn formations_save_as(project_id: String, name: String, reg: State<'_, ProjectRegistry>, rs: State<'_, Arc<crate::intake::remote::RemoteState>>, app: AppHandle) -> Result<String, String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        let id = remote::formations_save_as(&api, &pid, &name).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(id);
    }
    let p = project(&reg, &project_id)?;
    let id = formations::save_as(&p.pool, &name).await.map_err(s)?;
    emit_updated(&app);
    Ok(id)
}

#[tauri::command]
pub async fn formations_rename(project_id: String, id: String, name: String, reg: State<'_, ProjectRegistry>, rs: State<'_, Arc<crate::intake::remote::RemoteState>>, app: AppHandle) -> Result<(), String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        remote::formations_rename(&api, &pid, &id, &name).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(());
    }
    let p = project(&reg, &project_id)?;
    formations::rename(&p.pool, &id, &name).await.map_err(s)?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn formations_delete(project_id: String, id: String, reg: State<'_, ProjectRegistry>, rs: State<'_, Arc<crate::intake::remote::RemoteState>>, app: AppHandle) -> Result<(), String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        remote::formations_delete(&api, &pid, &id).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(());
    }
    let p = project(&reg, &project_id)?;
    formations::delete(&p.pool, &id).await?;
    emit_updated(&app);
    Ok(())
}

/// 切換。目標有 Agent 時專案的程式庫要是可用的 git repo（跟新增 Agent 同一個條件）。
#[tauri::command]
pub async fn formations_switch(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    scheduler: State<'_, SchedulerHandle>,
    rs: State<'_, Arc<crate::intake::remote::RemoteState>>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    // 共用專案：規則（含只有管理員、進行中的卡擋住）都在 server。
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        let o = remote::formations_switch(&api, &pid, &id).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(o);
    }
    let p = project(&reg, &project_id)?;
    let repo = formations::repo_path(&p.pool).await.map_err(s)?;
    let needs_repo = formations::agents_of(&p.pool, &id).await.map_err(s)?.is_some_and(|a| !a.is_empty());
    if needs_repo {
        match &repo {
            None => return Err("請先在「團隊範本」設定專案資料夾".into()),
            Some(r) => {
                if !matches!(check_repo(r).await, RepoCheck::Ok { .. }) {
                    return Err(format!("專案資料夾不是可用的 git repo：{r}"));
                }
            }
        }
    }
    let o: SwitchOutcome = formations::switch_to(&p.pool, &id, repo.as_deref()).await?;
    emit_updated(&app);
    scheduler.poke();
    serde_json::to_value(o).map_err(|e| e.to_string())
}

/// 做完後切換：記下目標；現在就沒人在工作的話立刻換。
#[tauri::command]
pub async fn formations_request_switch(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    scheduler: State<'_, SchedulerHandle>,
    rs: State<'_, Arc<crate::intake::remote::RemoteState>>,
    app: AppHandle,
) -> Result<(), String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        remote::formations_request_switch(&api, &pid, &id).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(());
    }
    let p = project(&reg, &project_id)?;
    formations::set_pending(&p.pool, Some(&id)).await.map_err(s)?;
    formations::try_complete_pending(&p.pool).await?;
    emit_updated(&app);
    scheduler.poke();
    Ok(())
}

#[tauri::command]
pub async fn formations_cancel_switch(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
    scheduler: State<'_, SchedulerHandle>,
    rs: State<'_, Arc<crate::intake::remote::RemoteState>>,
    app: AppHandle,
) -> Result<(), String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        remote::formations_cancel_switch(&api, &pid).await.map_err(|e| remote::fail(&rs, e))?;
        emit_updated(&app);
        return Ok(());
    }
    let p = project(&reg, &project_id)?;
    formations::set_pending(&p.pool, None).await.map_err(s)?;
    emit_updated(&app);
    // 暫停開工的新模組可以繼續派了。
    scheduler.poke();
    Ok(())
}

/// 已經從桌布移除的 Agent（已完成卡片的歷史顯示名稱用）。
#[tauri::command]
pub async fn agents_retired(project_id: String, reg: State<'_, ProjectRegistry>, rs: State<'_, Arc<crate::intake::remote::RemoteState>>) -> Result<serde_json::Value, String> {
    if let Some((api, pid)) = shared(&rs, &project_id)? {
        return remote::formations_retired(&api, &pid).await.map_err(|e| remote::fail(&rs, e));
    }
    let p = project(&reg, &project_id)?;
    let v: Vec<RetiredAgent> = formations::retired(&p.pool).await.map_err(s)?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

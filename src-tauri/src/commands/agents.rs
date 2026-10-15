//! Agent 協作（Agent Canvas）的 Tauri 指令。跟 `commands/tasks.rs` 同一種
//! 形狀：薄薄一層委派給 `tasks::agents`／`tasks::store`，每次變動後送
//! `tasks-updated`，前端是被動的渲染器。
//! 見 `docs/superpowers/specs/2026-10-15-agent-canvas-design.md`。

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::config::types::VcsType;
use crate::projects::ProjectRegistry;
use crate::tasks::agents::{self, AgentRow, NewAgent};
use crate::tasks::scheduler::SchedulerHandle;
use crate::tasks::store;
use crate::vcs::git::GitClient;
use crate::vcs::VcsManager;

use super::tasks::{clone_with_attachments, emit_updated, project, BlockedReason, MergeOutcome};

fn agent_or_err(row: Option<AgentRow>) -> Result<AgentRow, String> {
    row.ok_or_else(|| "找不到這個 Agent".to_string())
}

#[tauri::command]
pub async fn agents_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<AgentRow>, String> {
    let p = project(&reg, &project_id)?;
    agents::list_agents(&p.pool).await.map_err(|e| e.to_string())
}

#[derive(Deserialize)]
pub struct CreateAgentArgs {
    pub kind: String,
    pub name: String,
    pub role: String,
    pub prompt_prefix: String,
    pub project_dir: String,
    pub canvas_x: f64,
    pub canvas_y: f64,
}

#[tauri::command]
pub async fn agents_create(
    project_id: String,
    args: CreateAgentArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    if args.kind != "coder" {
        return Err("目前只能建立 Coder Agent".to_string());
    }
    // 不是 git repo 就擋下：Agent 的隔離、依序累積與合併都建立在分支上。
    // 一般卡片「不是 repo 就退回原資料夾」的做法在這裡會讓兩個 Coder 同時
    // 改同一個資料夾。
    match VcsManager::detect_repo(&args.project_dir).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ => return Err(format!("Agent 的程式庫必須是 git repo：{}", args.project_dir)),
    }
    let id = agents::create_agent(
        &p.pool,
        &NewAgent {
            kind: &args.kind,
            name: &args.name,
            role: &args.role,
            prompt_prefix: &args.prompt_prefix,
            project_dir: &args.project_dir,
            canvas_x: args.canvas_x,
            canvas_y: args.canvas_y,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(id)
}

#[derive(Deserialize)]
pub struct UpdateAgentArgs {
    pub id: String,
    pub name: String,
    pub role: String,
    pub prompt_prefix: String,
    pub canvas_x: f64,
    pub canvas_y: f64,
}

#[tauri::command]
pub async fn agents_update(
    project_id: String,
    args: UpdateAgentArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    agents::update_agent(&p.pool, &args.id, &args.name, &args.role, &args.prompt_prefix, args.canvas_x, args.canvas_y)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn agents_delete(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    agents::delete_agent(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn tasks_assign_agent(
    project_id: String,
    task_id: String,
    agent_id: Option<String>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    agents::assign_task(&p.pool, &task_id, agent_id.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    scheduler.poke();
    Ok(())
}

/// 處理暫停中的 Agent：`retry` 或 `skip`。
///
/// - `retry`：複製失敗的卡片（含附件）、指派給同一個 Agent、插到它佇列
///   最前面。worktree 原樣保留，新的那次在半成品上繼續。
/// - `skip`：worktree 有未提交改動就 `git stash`（訊息標記卡片 id，不會
///   遺失），從乾淨狀態接著做下一張。
#[tauri::command]
pub async fn agents_resolve(
    project_id: String,
    id: String,
    action: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let agent = agent_or_err(agents::get_agent(&p.pool, &id).await.map_err(|e| e.to_string())?)?;
    let failed_id = agent.paused_task_id.clone().ok_or_else(|| "這個 Agent 沒有暫停".to_string())?;
    match action.as_str() {
        "retry" => {
            let new_id = clone_with_attachments(&p, &failed_id).await?;
            agents::assign_task(&p.pool, &new_id, Some(&agent.id)).await.map_err(|e| e.to_string())?;
            if let Some(first) = agents::first_queued_order(&p.pool, &agent.id).await.map_err(|e| e.to_string())? {
                // assign_task 把它排在最後；改成插到這個 Agent 的佇列最前面。
                store::move_task(&p.pool, &new_id, store::STATUS_QUEUED, first - 1.0)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        "skip" => {
            if let Some(wt) = &agent.worktree_path {
                if std::path::Path::new(wt).is_dir() {
                    let client = GitClient::new(wt.clone(), None);
                    if client.has_uncommitted_changes().await? {
                        client.stash_push(&format!("aiterm skipped {failed_id}")).await?;
                    }
                }
            }
        }
        other => return Err(format!("未知的動作：{other}")),
    }
    agents::set_paused(&p.pool, &agent.id, None).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    scheduler.poke();
    Ok(())
}

/// 把 Agent 的分支合併回 `agent.project_dir` 目前的分支。
///
/// 跟每卡的「合併回原分支」不同：**不刪 worktree、不刪分支**——Agent 之後
/// 還要繼續在上面工作。前置檢查與衝突處理沿用同一套 `MergeOutcome`。
#[tauri::command]
pub async fn agents_merge(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<MergeOutcome, String> {
    let p = project(&reg, &project_id)?;
    let agent = agent_or_err(agents::get_agent(&p.pool, &id).await.map_err(|e| e.to_string())?)?;
    if agent.paused_task_id.is_some() {
        return Err("Agent 暫停中，請先重試或跳過".to_string());
    }
    let running = store::list_by_status(&p.pool, store::STATUS_RUNNING)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .any(|t| t.agent_id.as_deref() == Some(agent.id.as_str()));
    if running {
        return Err("Agent 還有工作在執行，等它做完再合併".to_string());
    }
    let branch = agent.branch.clone().ok_or_else(|| "這個 Agent 還沒有分支（尚未派過工）".to_string())?;

    let base = GitClient::new(agent.project_dir.clone(), None);
    if base.is_merge_in_progress().await {
        return Ok(MergeOutcome::Blocked {
            reason: BlockedReason::MergeInProgress,
            files: base.conflicted_files().await.unwrap_or_default(),
        });
    }
    if base.has_uncommitted_changes().await? {
        return Ok(MergeOutcome::Blocked {
            reason: BlockedReason::DirtyBase,
            files: base.dirty_files().await.unwrap_or_default(),
        });
    }
    // 正常情況每張成功的卡片都已經自動 commit，這裡只是保險。
    if let Some(wt) = &agent.worktree_path {
        if std::path::Path::new(wt).is_dir() {
            let client = GitClient::new(wt.clone(), None);
            if client.has_uncommitted_changes().await? {
                client.commit_all(&format!("Agent: {}", agent.name)).await?;
            }
        }
    }
    if let Err(merge_err) = base.merge_branch(&branch).await {
        let files = base.conflicted_files().await.unwrap_or_default();
        if !files.is_empty() {
            return Ok(MergeOutcome::Conflict { files });
        }
        if base.is_merge_in_progress().await {
            let _ = base.merge_abort().await;
        }
        return Err(merge_err);
    }
    emit_updated(&app);
    Ok(MergeOutcome::Merged)
}

#[tauri::command]
pub async fn agents_abort_merge(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let agent = agent_or_err(agents::get_agent(&p.pool, &id).await.map_err(|e| e.to_string())?)?;
    GitClient::new(agent.project_dir, None).merge_abort().await?;
    Ok(())
}

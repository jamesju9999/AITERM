//! 定時器的 Tauri 指令：薄薄一層委派給 `tasks::schedules`。
//! 見 `docs/superpowers/specs/2026-11-16-agent-canvas-scheduler-design.md`。

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::projects::ProjectRegistry;
use crate::tasks::scheduler::SchedulerHandle;
use crate::tasks::schedules::{self, NewSchedule, RunRow, ScheduleRow};

use super::tasks::{emit_updated, project};

#[derive(Serialize)]
pub struct ScheduleView {
    #[serde(flatten)]
    pub row: ScheduleRow,
    pub runs: Vec<RunRow>,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[tauri::command]
pub async fn schedules_list(project_id: String, reg: State<'_, ProjectRegistry>) -> Result<Vec<ScheduleView>, String> {
    let p = project(&reg, &project_id)?;
    let mut out = Vec::new();
    for row in schedules::list(&p.pool).await.map_err(|e| e.to_string())? {
        let runs = schedules::list_runs(&p.pool, &row.id).await.map_err(|e| e.to_string())?;
        out.push(ScheduleView { row, runs });
    }
    Ok(out)
}

#[tauri::command]
pub async fn schedules_save(
    project_id: String,
    id: Option<String>,
    args: NewSchedule,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let agent = crate::tasks::agents::get_agent(&p.pool, &args.agent_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到這個角色".to_string())?;
    if agent.kind != "coder" && agent.kind != "verifier" {
        return Err("定時器只能叫起工程師或驗收者".into());
    }
    let id = match id {
        Some(id) => {
            schedules::update(&p.pool, &id, &args).await?;
            id
        }
        None => schedules::create(&p.pool, &args, now_secs()).await?,
    };
    emit_updated(&app);
    Ok(id)
}

#[tauri::command]
pub async fn schedules_delete(project_id: String, id: String, reg: State<'_, ProjectRegistry>, app: AppHandle) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    schedules::delete(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn schedules_set_enabled(
    project_id: String,
    id: String,
    enabled: bool,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    schedules::set_enabled(&p.pool, &id, enabled, now_secs()).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 立即執行一次：走 `fire` 的同一條路（上一輪未結案會被擋下並記為跳過），
/// 但不吃掉下一個預定時間點——`last_fired_at` 還原成執行前的值。回傳新卡 id，被擋下時是空字串。
#[tauri::command]
pub async fn schedules_run_now(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    scheduler: State<'_, SchedulerHandle>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let row = schedules::get(&p.pool, &id).await.map_err(|e| e.to_string())?.ok_or_else(|| "找不到這個定時器".to_string())?;
    let agent = crate::tasks::agents::get_agent(&p.pool, &row.agent_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "目標角色已不存在".to_string())?;
    let head = if agent.kind == "verifier" {
        Some(
            crate::vcs::git::GitClient::new(agent.project_dir.clone(), None)
                .head_sha()
                .await
                .map_err(|e| format!("讀不到目前版本：{e}"))?,
        )
    } else {
        None
    };
    let before = row.last_fired_at;
    let fired = schedules::fire(&p.pool, &p.path, &row, now_secs(), head.as_deref()).await?;
    sqlx::query("UPDATE schedules SET last_fired_at = ? WHERE id = ?")
        .bind(before)
        .bind(&id)
        .execute(&p.pool)
        .await
        .map_err(|e| e.to_string())?;
    scheduler.poke();
    emit_updated(&app);
    Ok(match fired {
        schedules::Fired::Created { card_id } => card_id,
        schedules::Fired::Skipped => String::new(),
    })
}

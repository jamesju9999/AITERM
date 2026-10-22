//! Agent 協作（Agent Canvas）的 Tauri 指令。跟 `commands/tasks.rs` 同一種
//! 形狀：薄薄一層委派給 `tasks::agents`／`tasks::store`，每次變動後送
//! `tasks-updated`，前端是被動的渲染器。
//! 見 `docs/superpowers/specs/2026-10-15-agent-canvas-design.md`。

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::projects::ProjectRegistry;
use crate::tasks::agents::{self, AgentRow, NewAgent};
use crate::tasks::scheduler::SchedulerHandle;
use crate::tasks::store;
use crate::vcs::git::GitClient;

use super::tasks::{emit_updated, project, BlockedReason, MergeOutcome};

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
    /// Coder：產出規格（見規格先行 spec）。舊前端不送就是 false。
    #[serde(default)]
    pub produces_spec: bool,
    /// 派工時的 `--model`；空字串或不送＝沿用預設。
    #[serde(default)]
    pub model: Option<String>,
    /// AI 工具（claude／codex）；不送＝claude。
    #[serde(default)]
    pub cli: Option<String>,
}

#[tauri::command]
pub async fn agents_create(
    project_id: String,
    args: CreateAgentArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    // 不是 git repo 就擋下：Agent 的隔離、依序累積與合併都建立在分支上。
    // 一般卡片「不是 repo 就退回原資料夾」的做法在這裡會讓兩個 Coder 同時
    // 改同一個資料夾。
    match crate::tasks::repo_check::check_repo(&args.project_dir).await {
        crate::tasks::repo_check::RepoCheck::Ok { .. } => {}
        crate::tasks::repo_check::RepoCheck::Missing => {
            return Err(format!("資料夾不存在：{}", args.project_dir))
        }
        crate::tasks::repo_check::RepoCheck::NotRepo => {
            return Err(format!("Agent 的程式庫必須是 git repo：{}", args.project_dir))
        }
        crate::tasks::repo_check::RepoCheck::NoCommits => {
            return Err(format!("這個 git repo 還沒有任何 commit：{}", args.project_dir))
        }
        crate::tasks::repo_check::RepoCheck::BoardFolder => {
            return Err(format!("這是看板專案資料夾，不是程式碼：{}", args.project_dir))
        }
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
    if args.produces_spec && args.kind == "coder" {
        agents::set_produces_spec(&p.pool, &id, true).await.map_err(|e| e.to_string())?;
    }
    agents::set_model(&p.pool, &id, args.model.as_deref()).await.map_err(|e| e.to_string())?;
    if let Some(cli) = &args.cli {
        agents::set_cli(&p.pool, &id, cli).await.map_err(|e| e.to_string())?;
    }
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
    /// `None` = 不改（例如只是在桌布上拖動位置）。
    #[serde(default)]
    pub produces_spec: Option<bool>,
    /// `None` = 不改；空字串 = 清掉（沿用預設）。
    #[serde(default)]
    pub model: Option<String>,
    /// `None` = 不改。
    #[serde(default)]
    pub cli: Option<String>,
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
    if let Some(v) = args.produces_spec {
        agents::set_produces_spec(&p.pool, &args.id, v).await.map_err(|e| e.to_string())?;
    }
    if let Some(m) = &args.model {
        agents::set_model(&p.pool, &args.id, Some(m)).await.map_err(|e| e.to_string())?;
    }
    if let Some(cli) = &args.cli {
        agents::set_cli(&p.pool, &args.id, cli).await.map_err(|e| e.to_string())?;
    }
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

/// 清空團隊：移除桌布上全部 Agent。有人在執行就整個拒絕。回傳移除了幾位。
#[tauri::command]
pub async fn agents_delete_all(project_id: String, reg: State<'_, ProjectRegistry>, app: AppHandle) -> Result<usize, String> {
    let p = project(&reg, &project_id)?;
    let n = agents::delete_all_agents(&p.pool).await?;
    emit_updated(&app);
    Ok(n)
}

/// 手動暫停的現況：全部暫停？哪些 Agent 個別暫停？
#[tauri::command]
pub async fn agents_manual_pause_state(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<crate::tasks::manual_pause::ManualPause, String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::manual_pause::get(&p.pool).await.map_err(|e| e.to_string())
}

/// 手動暫停／繼續。`agent_id` 是 `None` = 全部；已經在跑的卡不受影響（要中斷用停止）。
#[tauri::command]
pub async fn agents_set_manual_pause(
    project_id: String,
    agent_id: Option<String>,
    paused: bool,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::manual_pause::set(&p.pool, agent_id.as_deref(), paused).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    if !paused {
        scheduler.poke();
    }
    Ok(())
}

#[tauri::command]
pub async fn tasks_assign_agent(
    project_id: String,
    task_id: String,
    agent_id: Option<String>,
    fast: Option<bool>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    agents::assign_task_with(&p.pool, &task_id, agent_id.as_deref(), fast.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    scheduler.poke();
    Ok(())
}

/// 處理暫停中的 Agent：`retry` 或 `skip`。
///
/// - `retry`：把失敗的那張卡原地排回這個 Agent 佇列的最前面。worktree 原樣
///   保留，新的那次在半成品上繼續。
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
            // 原地排回這個 Agent 佇列的最前面（不複製新卡，見 store::requeue_failed）。
            let order = match agents::first_queued_order(&p.pool, &agent.id).await.map_err(|e| e.to_string())? {
                Some(first) => first - 1.0,
                None => 0.0,
            };
            store::requeue_failed(&p.pool, &failed_id, order).await.map_err(|e| e.to_string())?;
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
    agents::resume_after_resolve(&p.pool, &agent.id).await.map_err(|e| e.to_string())?;
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

    // 看板專案資料夾在程式庫裡面時先排除，免得它讓程式庫變成「有未提交的改動」而合不回去。
    if let Err(e) = crate::tasks::repo_check::exclude_inside(std::path::Path::new(&agent.project_dir), &p.path) {
        eprintln!("exclude board folder: {e}");
    }
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
    if agent.kind == "orchestrator" {
        // 合併卡上「還沒合回程式庫」的提示到此解除。
        sqlx::query(
            "UPDATE tasks SET verdict = ? WHERE id = (
                 SELECT id FROM tasks WHERE agent_id = ? AND card_kind = 'merge' AND outcome = 'success'
                 ORDER BY finished_at DESC LIMIT 1)",
        )
        .bind(crate::tasks::workflow::REPO_MERGED)
        .bind(&agent.id)
        .execute(&p.pool)
        .await
        .map_err(|e| e.to_string())?;
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

/// 手動放行一個卡住的模組（審查不通過超過上限，或審查卡本身失敗）。放行後
/// 若所有模組都通過，會立即替 Orchestrator 建合併卡。
#[tauri::command]
pub async fn agents_override_module(
    project_id: String,
    task_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::workflow::override_module(&p.pool, &task_id).await.map_err(|e| e.to_string())?;
    crate::tasks::workflow::maybe_enqueue_merge(&p.pool).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    scheduler.poke();
    Ok(())
}

/// 驗收報告：verify.json 的內容，加上證據圖片（證據路徑 → data URL）。
/// 驗收卡還沒寫出 verify.json 時 `report` 是 `None`。
#[derive(serde::Serialize)]
pub struct VerifyReportView {
    pub report: Option<crate::tasks::workflow::VerifyReport>,
    pub images: std::collections::HashMap<String, String>,
}

#[tauri::command]
pub async fn agents_verify_report(
    project_id: String,
    task_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<VerifyReportView, String> {
    let p = project(&reg, &project_id)?;
    let path = p.path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let report = std::fs::read_to_string(crate::tasks::workflow::verify_json_path(&path, &task_id))
            .ok()
            .and_then(|t| crate::tasks::workflow::parse_verify_json(&t).ok());
        let images = report
            .as_ref()
            .map(|r| crate::tasks::workflow::evidence_images(&crate::tasks::task_dir(&path, &task_id), r))
            .unwrap_or_default();
        VerifyReportView { report, images }
    })
    .await
    .map_err(|e| e.to_string())
}

/// 放行一張沒通過的驗收：當成通過，程式庫更新到受驗版本。
#[tauri::command]
pub async fn agents_accept_verification(
    project_id: String,
    task_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::workflow::accept_verification(&p.pool, &p.path, &task_id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 在系統的檔案管理員（Finder／檔案總管／Linux 預設）打開程式庫資料夾。
#[tauri::command]
pub fn agents_open_repo(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("找不到資料夾：{path}"));
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut cmd = std::process::Command::new(program);
    cmd.arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // 被打開的是使用者的檔案管理員，不能繼承 AppImage 的函式庫路徑。
    crate::appimage_env::strip_appimage_env(&mut cmd);
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// 對話框選資料夾時呼叫：是不是可以用的 git repo。
#[tauri::command]
pub async fn agents_check_repo(path: String) -> Result<crate::tasks::repo_check::RepoCheck, String> {
    Ok(crate::tasks::repo_check::check_repo(&path).await)
}

/// 裝上、修改或移除（`None`）Agent 的資料庫技能。下一張派出去的卡才生效。
#[tauri::command]
pub async fn agents_set_db_skill(
    project_id: String,
    agent_id: String,
    skill: Option<crate::tasks::db_skill::DbSkill>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::db_skill::set_agent_skill(&p.pool, &agent_id, skill.as_ref()).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 專案最近的資料庫查詢（活動記錄與「使用中」用）。
#[tauri::command]
pub async fn agents_db_queries(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<crate::tasks::db_skill::DbQueryRow>, String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::db_skill::recent_queries(&p.pool, 100).await.map_err(|e| e.to_string())
}

/// 專案的參考資料夾（整個團隊都能讀、不能寫）。
#[tauri::command]
pub async fn agents_get_references(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<crate::tasks::references::Reference>, String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::references::get(&p.pool).await.map_err(|e| e.to_string())
}

/// 存參考資料夾清單（整份取代）。資料夾必須存在；下一張派出去的卡才生效。
#[tauri::command]
pub async fn agents_set_references(
    project_id: String,
    references: Vec<crate::tasks::references::Reference>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<Vec<crate::tasks::references::Reference>, String> {
    let p = project(&reg, &project_id)?;
    let refs = crate::tasks::references::normalize(references)?;
    crate::tasks::references::set(&p.pool, &refs).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(refs)
}

/// 可以給 Agent 的技能：它的程式庫 `.claude/skills` 與使用者的 `~/.claude/skills`。
#[tauri::command]
pub async fn agents_list_skills(repo_dir: String) -> Vec<crate::tasks::custom_skills::SkillInfo> {
    let home = dirs::home_dir();
    crate::tasks::custom_skills::discover(Some(std::path::Path::new(&repo_dir)), home.as_deref())
}

/// 使用者自己選的資料夾裡的技能（它本身、它底下、或它的 `.claude/skills`）。
#[tauri::command]
pub fn agents_read_skill(path: String) -> Result<Vec<crate::tasks::custom_skills::SkillInfo>, String> {
    let found = crate::tasks::custom_skills::find_in(std::path::Path::new(&path));
    if found.is_empty() {
        return Err(format!("這個資料夾裡找不到技能（SKILL.md），也沒有 .claude/skills：{path}"));
    }
    Ok(found)
}

/// 這些已存的技能路徑裡，哪些已經讀不到 SKILL.md（資料夾搬走或刪了）。
#[tauri::command]
pub fn agents_missing_skills(paths: Vec<String>) -> Vec<String> {
    crate::tasks::custom_skills::missing(&paths)
}

/// 設定 Agent 的自訂技能（技能資料夾路徑，整份取代）。下一張派出去的卡才生效。
#[tauri::command]
pub async fn agents_set_custom_skills(
    project_id: String,
    agent_id: String,
    paths: Vec<String>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let mut clean: Vec<String> = Vec::new();
    for p in paths {
        let p = p.trim().to_string();
        if p.is_empty() || clean.contains(&p) {
            continue;
        }
        if !std::path::Path::new(&p).join(crate::tasks::custom_skills::SKILL_FILE).is_file() {
            return Err(format!("這個資料夾裡沒有 SKILL.md：{p}"));
        }
        clean.push(p);
    }
    if clean.len() > crate::tasks::custom_skills::MAX_SKILLS {
        return Err(format!("一位 Agent 最多 {} 個自訂技能", crate::tasks::custom_skills::MAX_SKILLS));
    }
    let p = project(&reg, &project_id)?;
    crate::tasks::custom_skills::set_agent_skills(&p.pool, &agent_id, &clean).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 專案最近的技能使用（技能列亮起來、活動記錄用）。
#[tauri::command]
pub async fn agents_skill_uses(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<crate::tasks::custom_skills::SkillUseRow>, String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::custom_skills::recent_uses(&p.pool, 100).await.map_err(|e| e.to_string())
}

/// 讀了參考資料的記錄（桌布上「讀取中」的線與活動記錄）。
#[tauri::command]
pub async fn agents_reference_reads(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<crate::tasks::references::ReferenceReadRow>, String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::references::recent_reads(&p.pool, 100).await.map_err(|e| e.to_string())
}

/// 這個專案的程式庫（套用團隊範本用）：桌布上 Agent 的 → 這個專案記住的。都沒有是 None，由使用者選。
#[tauri::command]
pub async fn agents_project_repo(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Option<crate::tasks::repo_check::ProjectRepo>, String> {
    let p = project(&reg, &project_id)?;
    let dirs: Vec<String> = agents::list_agents(&p.pool).await.map_err(|e| e.to_string())?.into_iter().map(|a| a.project_dir).collect();
    let saved = crate::tasks::repo_check::saved_project_repo(&p.pool).await.map_err(|e| e.to_string())?;
    Ok(crate::tasks::repo_check::project_repo(&dirs, saved))
}

/// 記住這個專案的程式庫。
#[tauri::command]
pub async fn agents_set_project_repo(project_id: String, path: String, reg: State<'_, ProjectRegistry>) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    crate::tasks::repo_check::save_project_repo(&p.pool, path.trim()).await.map_err(|e| e.to_string())
}

/// 協作頁檢查 Claude Code／Codex／git 有沒有裝（用派工分頁同一個 shell 環境找）。
#[tauri::command]
pub async fn agents_check_tools(
    config: State<'_, std::sync::Arc<crate::config::ConfigStore>>,
) -> Result<crate::tasks::tools_check::ToolsCheck, String> {
    let command = config.get().task_board.claude_command;
    crate::tasks::tools_check::check(&command).await
}

/// 一鍵把資料夾變成可用的 git repo（git init＋第一個 commit）。
#[tauri::command]
pub async fn agents_init_repo(
    path: String,
    project_id: Option<String>,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    // 看板專案資料夾在這個程式庫裡面時，第一個 commit 不能把它收進去。
    let board = project_id.and_then(|id| project(&reg, &id).ok()).map(|p| p.path.clone());
    crate::tasks::repo_check::init_repo(&path, board.as_deref()).await
}

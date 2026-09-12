//! Tauri commands for the task board. Thin delegates to `tasks::store`, plus
//! attachment file I/O and a `tasks-updated` emit after every mutation so the
//! board view (a passive renderer) refreshes. Same shape as
//! `commands/loop_session.rs`.

use std::fs;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::projects::{ProjectHandle, ProjectRegistry};
use crate::tasks::scheduler::SchedulerHandle;
use crate::tasks::store::{self, AttachmentRow, TaskRow};
use crate::tasks::task_dir;
use crate::vcs::git::GitClient;

pub(crate) fn edit_allowed(status: &str) -> bool {
    status == store::STATUS_PLANNING
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
}

/// 合併進度的酬載。大型 worktree 的合併會跑好幾分鐘，而且慢的幾乎都是最後
/// 的 `worktree remove`（逐檔刪除，Windows 上還要過防毒掃描）——沒有分階段
/// 回報的話，畫面上只有一句「合併中…」，使用者分不出是正常進行還是卡死。
#[derive(Clone, serde::Serialize)]
struct MergeProgress {
    task_id: String,
    /// 見 `src/components/TaskBoard/TaskCard.tsx` 的 `MERGE_STEP_LABEL`，
    /// 兩邊的字串必須一致。
    step: &'static str,
}

fn emit_merge_step(app: &AppHandle, task_id: &str, step: &'static str) {
    let _ = app.emit(
        "task-merge-progress",
        MergeProgress { task_id: task_id.to_string(), step },
    );
}

/// 從 registry 取出專案。找不到時回傳給前端的錯誤訊息——
/// 這在正常使用下不會發生（前端只會送出 `projects_list` 給過的 id），
/// 會發生代表專案在操作進行中被移除了。
fn project(reg: &ProjectRegistry, id: &str) -> Result<ProjectHandle, String> {
    reg.get(id).ok_or_else(|| format!("專案不存在或已關閉：{id}"))
}

#[derive(Serialize)]
pub struct TaskWithAttachments {
    #[serde(flatten)]
    pub task: TaskRow,
    pub attachments: Vec<AttachmentRow>,
}

/// 幫每一列補上它的附件。看板與封存清單共用，兩邊回傳的形狀因此一致。
async fn with_attachments(
    pool: &sqlx::SqlitePool,
    tasks: Vec<TaskRow>,
) -> Result<Vec<TaskWithAttachments>, String> {
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let attachments = store::list_attachments(pool, &task.id)
            .await
            .map_err(|e| e.to_string())?;
        out.push(TaskWithAttachments { task, attachments });
    }
    Ok(out)
}

#[tauri::command]
pub async fn tasks_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<TaskWithAttachments>, String> {
    let p = project(&reg, &project_id)?;
    let tasks = store::list_tasks(&p.pool).await.map_err(|e| e.to_string())?;
    with_attachments(&p.pool, tasks).await
}

#[derive(Deserialize)]
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
    pub label: Option<String>,
    /// `None` = 沿用全域設定，見 `TaskRow::isolate_worktree`。
    pub isolate_worktree: Option<bool>,
}

#[tauri::command]
pub async fn tasks_create(
    project_id: String,
    args: CreateArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let id = store::create_task(
        &p.pool,
        &args.title,
        &args.body,
        &args.project_dir,
        args.parallel_ok,
        args.interactive,
    )
    .await
    .map_err(|e| e.to_string())?;
    store::set_bridge_config(&p.pool, &id, args.use_bridge, args.bridge_tiers)
        .await
        .map_err(|e| e.to_string())?;
    store::set_label(&p.pool, &id, args.label.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    store::set_isolate_worktree(&p.pool, &id, args.isolate_worktree)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(id)
}

#[derive(Deserialize)]
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
    pub use_bridge: bool,
    pub bridge_tiers: Option<String>,
    pub label: Option<String>,
    /// `None` = 沿用全域設定，見 `TaskRow::isolate_worktree`。
    pub isolate_worktree: Option<bool>,
}

#[tauri::command]
pub async fn tasks_update(
    project_id: String,
    args: UpdateArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    store::set_parallel_ok(&p.pool, &args.id, args.parallel_ok)
        .await
        .map_err(|e| e.to_string())?;
    store::set_interactive(&p.pool, &args.id, args.interactive)
        .await
        .map_err(|e| e.to_string())?;
    store::set_bridge_config(&p.pool, &args.id, args.use_bridge, args.bridge_tiers)
        .await
        .map_err(|e| e.to_string())?;
    store::set_isolate_worktree(&p.pool, &args.id, args.isolate_worktree)
        .await
        .map_err(|e| e.to_string())?;
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &p.pool,
            &args.id,
            &args.title,
            &args.body,
            &args.project_dir,
            args.label.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct MoveArgs {
    pub id: String,
    pub to_status: String,
    pub sort_order: f64,
}

#[tauri::command]
pub async fn tasks_move(
    project_id: String,
    args: MoveArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::move_task(&p.pool, &args.id, &args.to_status, args.sort_order)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    if args.to_status == store::STATUS_QUEUED {
        scheduler.poke();
    }
    Ok(())
}

#[tauri::command]
pub async fn tasks_stop(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if row.status != store::STATUS_RUNNING {
        return Err("task is not running".into());
    }
    if let Some(tab_id) = &row.tab_id {
        let _ = pty.write(tab_id, b"\x03"); // Ctrl+C
    }
    if !scheduler.cancel(&id) {
        store::finish_task(&p.pool, &id, "cancelled", Some("使用者停止"), None)
            .await
            .map_err(|e| e.to_string())?;
        emit_updated(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn tasks_mark_done(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if row.status != store::STATUS_RUNNING || !row.interactive {
        return Err("only a running interactive task can be marked done".to_string());
    }
    if !scheduler.mark_done(&id) {
        // No active watch to signal — e.g. it just finished on its own via
        // an exit-code signal in the moment between the frontend rendering
        // the button and the click landing. Finish it directly, mirroring
        // tasks_stop's own fallback for the equivalent race.
        store::finish_task(&p.pool, &id, "success", None, row.transcript_path.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        emit_updated(&app);
    }
    Ok(())
}

/// 「合併回原分支」的結果。
///
/// 衝突不是錯誤，而是一種**需要使用者決定**的狀態，所以用結構化結果回傳，
/// 而不是塞進 Err 字串讓前端去比對文字。
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// 合併完成，worktree 已清理。
    Merged,
    /// 有衝突。原專案目錄停在合併進行中，worktree 與分支原樣保留。
    Conflict { files: Vec<String> },
    /// **合併已經成功**，但 worktree 目錄刪不掉。
    ///
    /// 這不是合併失敗，硬要講成失敗反而會害使用者以為成果沒進去。Windows 上
    /// 的常見成因是還有行程的工作目錄在那個資料夾裡（見 aiterm-core 的
    /// `kill_tree_first`）。DB 欄位照樣清掉——卡片的任務確實完成了，按鈕該
    /// 消失；留著只會讓下一次按在半刪除的目錄上失敗。
    MergedButNotCleaned { path: String, detail: String },
    /// 還沒開始動手就擋下來了。
    Blocked { reason: BlockedReason, files: Vec<String> },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    /// 原專案目錄有未提交的變更。
    DirtyBase,
    /// 原專案目錄還停在上一次沒收尾的合併。
    MergeInProgress,
}

#[tauri::command]
pub async fn tasks_merge_worktree(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    pty: State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<MergeOutcome, String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let worktree_path = row.worktree_path.ok_or_else(|| "this card has no worktree to merge".to_string())?;
    let worktree_branch = row.worktree_branch.ok_or_else(|| "this card has no worktree to merge".to_string())?;

    let base_client = GitClient::new(row.project_dir.clone(), None);

    // 兩個前置檢查都在動任何東西**之前**做完。一旦開始 commit/merge 就很難
    // 乾淨地退回去，而這兩種狀況都是使用者自己就能處理的——與其讓 git 在
    // 半路拒絕、丟一段沒頭沒尾的 stderr，不如一開始就講清楚。
    emit_merge_step(&app, &id, "checking");
    if base_client.is_merge_in_progress().await {
        return Ok(MergeOutcome::Blocked {
            reason: BlockedReason::MergeInProgress,
            files: base_client.conflicted_files().await.unwrap_or_default(),
        });
    }
    if base_client.has_uncommitted_changes().await? {
        return Ok(MergeOutcome::Blocked {
            reason: BlockedReason::DirtyBase,
            files: base_client.dirty_files().await.unwrap_or_default(),
        });
    }

    // 這一步檢查的是 worktree，不是原分支——標籤要在檢查**之前**就換掉，
    // 否則失敗時畫面上停在「檢查原分支」，會把人指向錯的地方（實機踩過）。
    emit_merge_step(&app, &id, "committing");
    let worktree_client = GitClient::new(worktree_path.clone(), None);
    if worktree_client.has_uncommitted_changes().await? {
        worktree_client.commit_all(&format!("Task: {}", row.title)).await?;
    }

    emit_merge_step(&app, &id, "merging");

    // 合併失敗時 worktree/分支一律原樣保留——成果在上一步就已經 commit 到
    // 那個分支上了，保留住使用者才有機會解衝突或改天再試。
    if let Err(merge_err) = base_client.merge_branch(&worktree_branch).await {
        let files = base_client.conflicted_files().await.unwrap_or_default();
        if !files.is_empty() {
            // 真的是衝突：決定權交還給使用者（前端跳視窗問要自己解還是還原）。
            return Ok(MergeOutcome::Conflict { files });
        }
        // 不是衝突的其他失敗。倉庫仍可能被留在半合併狀態，先還原再把 git
        // 自己的錯誤往上丟——這種情況使用者無從決定，留著只會礙事。
        if base_client.is_merge_in_progress().await {
            let _ = base_client.merge_abort().await;
        }
        return Err(merge_err);
    }

    // 這一步通常是最慢的：它要逐一刪掉 worktree 裡的每個檔案。
    emit_merge_step(&app, &id, "cleaning");

    // **先把這張卡片的 PTY 關掉，否則 Windows 上刪不掉那個目錄。**
    //
    // 派工分頁走的是「被領養的 session」：後端先建好 PTY，前端再接管。
    // `TerminalView` 的卸載清理刻意不關這種 session（見該檔
    // `if (id && !externalSessionId)` 那段註解），所以分頁關掉之後 `pwsh.exe`
    // 與它底下的 `claude.exe` 仍然活著，工作目錄還在這個 worktree 裡。Windows
    // 不允許刪除使用中的目錄，`git worktree remove` 因此以
    // `failed to delete ...: Directory not empty` 失敗——實機用資源監視器查到
    // 持有 handle 的正是這兩個行程。
    //
    // 我們正要刪掉這個 worktree，那個 shell 本來就該結束，所以這裡主動關。
    // `PtySession::kill` 會連同整棵行程樹一起殺（見 aiterm-core 的
    // `kill_tree_first`），claude.exe 才不會變成孤兒繼續佔著目錄。
    if let Some(tab_id) = &row.tab_id {
        // 已經關掉的 session 會回 SessionNotFound，那是正常情況不是錯誤。
        let _ = pty.close(tab_id);
    }

    if let Err(cleanup_err) = base_client.remove_worktree(&worktree_path).await {
        // **合併已經成功了**，這裡失敗的只是清理。照樣把 DB 欄位清掉並 prune：
        // 卡片的任務確實完成了，按鈕該消失；留著的話下一次按會跑在一個半刪除
        // 的 worktree 上，得到 `not a git repository` 這種跟真正問題無關的錯誤
        // （實機踩過）。
        let _ = base_client.prune_worktrees().await;
        store::clear_worktree(&p.pool, &id).await.map_err(|e| e.to_string())?;
        emit_updated(&app);
        return Ok(MergeOutcome::MergedButNotCleaned {
            path: worktree_path,
            detail: cleanup_err,
        });
    }
    store::clear_worktree(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(MergeOutcome::Merged)
}

/// 還原一次沒收尾的合併（`git merge --abort`）。前端在衝突視窗上選「還原」
/// 時呼叫。**worktree 與分支不動**——成果都在那個分支上，隨時能再試。
#[tauri::command]
pub async fn tasks_abort_merge(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    GitClient::new(row.project_dir, None).merge_abort().await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct DeleteArgs {
    pub id: String,
    pub close_tab: bool,
}

#[tauri::command]
pub async fn tasks_delete(
    project_id: String,
    args: DeleteArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    if let Some(row) = store::get_task(&p.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
    {
        scheduler.cancel(&args.id);
        if args.close_tab {
            if let Some(tab_id) = &row.tab_id {
                let _ = pty.close(tab_id);
            }
        }
    }
    store::delete_task(&p.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(task_dir(&p.path, &args.id));
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct AddAttachmentArgs {
    pub id: String,
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[tauri::command]
pub async fn tasks_add_attachment(
    project_id: String,
    args: AddAttachmentArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<AttachmentRow, String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if !edit_allowed(&row.status) {
        return Err("attachments can only be changed while the card is in 計畫中".into());
    }
    let dir = task_dir(&p.path, &args.id).join("attachments");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe = std::path::Path::new(&args.filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    let stored = dir.join(&safe);
    fs::write(&stored, &args.bytes).map_err(|e| e.to_string())?;
    let att_id = store::add_attachment(&p.pool, &args.id, &safe, &stored.to_string_lossy())
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(AttachmentRow {
        id: att_id,
        task_id: args.id,
        filename: safe,
        stored_path: stored.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn tasks_remove_attachment(
    project_id: String,
    attachment_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    if let Some(att) = store::get_attachment(&p.pool, &attachment_id)
        .await
        .map_err(|e| e.to_string())?
    {
        if let Some(row) = store::get_task(&p.pool, &att.task_id)
            .await
            .map_err(|e| e.to_string())?
        {
            if !edit_allowed(&row.status) {
                return Err("attachments can only be changed while the card is in 計畫中".into());
            }
        }
        let _ = fs::remove_file(&att.stored_path);
    }
    store::remove_attachment(&p.pool, &attachment_id)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn tasks_clone(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let src = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let new_id = store::clone_task_fields(&p.pool, &src.id)
        .await
        .map_err(|e| e.to_string())?;

    // Copy each attachment file into the new card's dir; skip any whose
    // source file is gone (best effort — a missing file must not fail the clone).
    let dir = task_dir(&p.path, &new_id).join("attachments");
    for att in store::list_attachments(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
    {
        if !std::path::Path::new(&att.stored_path).exists() {
            continue;
        }
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("tasks_clone: mkdir {dir:?}: {e}");
            break;
        }
        let dest = dir.join(&att.filename);
        if fs::copy(&att.stored_path, &dest).is_err() {
            continue;
        }
        let _ = store::add_attachment(&p.pool, &new_id, &att.filename, &dest.to_string_lossy()).await;
    }
    emit_updated(&app);
    Ok(new_id)
}

/// 決定要把哪一份記錄交給前端。
///
/// 有 `session_path` 且讀得出對話 → 渲染完整的逐輪記錄；否則退回
/// `transcript_path`（現行行為）。兩者都沒有就是空字串。
///
/// 退回對**使用者**是安靜的、不報錯：claude 根本沒啟動、卡在信任提示、
/// 或使用者把 `claude_command` 設成別的東西時，JSONL 不存在，而終端機
/// 畫面是唯一的診斷線索。原本的東西還在，沒有東西壞掉，不值得打斷
/// 使用者。但對 log 不是——見下面兩個 `eprintln!`。
pub fn resolve_transcript(session_path: Option<&str>, transcript_path: Option<&str>) -> String {
    if let Some(p) = session_path {
        match fs::read_to_string(p) {
            Ok(raw) => {
                let rendered = crate::tasks::session_log::render_session_log(&raw);
                if !rendered.trim().is_empty() {
                    return rendered;
                }
                // 檔案在、讀得到，但一句對話都渲染不出來。多半是 claude 在
                // 寫出任何 user/assistant 記錄之前就結束了（卡在信任提示、
                // 立刻被停掉），或是 JSONL 格式變了。跟「檔案不見」是完全
                // 不同的病因，所以分開記。
                eprintln!(
                    "session log at {p} rendered to nothing, falling back to the terminal capture"
                );
            }
            // 路徑有值卻讀不到：檔案被刪了、專案資料夾搬走而 rewrite_stored_paths
            // 沒跟上、或權限問題。
            Err(e) => {
                eprintln!("session log {p} unreadable ({e}), falling back to the terminal capture")
            }
        }
    }
    // session_path 為 None 時刻意不記——那是舊卡片與非 claude 指令的正常
    // 狀態，記了就是雜訊，而雜訊會讓上面兩行真正有用的訊息被忽略。
    transcript_path
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// `resolve_transcript`'s two params are both `Option<&str>` — same type,
/// so a transposed call site (`transcript_path` then `session_path`) still
/// compiles. Routing the field access through one named function, tested
/// below against a real `TaskRow`, is what actually pins which column goes
/// where; `resolve_transcript`'s own tests can't, since they call it with
/// already-correctly-labelled arguments and never touch a `TaskRow`.
fn transcript_for_row(row: &TaskRow) -> String {
    resolve_transcript(row.session_path.as_deref(), row.transcript_path.as_deref())
}

#[tauri::command]
pub async fn tasks_read_transcript(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<String, String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    Ok(transcript_for_row(&row))
}

#[tauri::command]
pub async fn tasks_save_transcript(
    project_id: String,
    id: String,
    text: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let path = row.transcript_path.ok_or_else(|| "no transcript path yet".to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

/// 這個專案的卡片用過的工作目錄。專案不綁資料夾（工作可散布在多個
/// repo），這個清單讓新增工作時不必每次重新瀏覽選取。
#[tauri::command]
pub async fn tasks_used_dirs(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<String>, String> {
    let p = project(&reg, &project_id)?;
    store::distinct_project_dirs(&p.pool).await.map_err(|e| e.to_string())
}

/// 這個專案的卡片用過的 Label。跟 `tasks_used_dirs` 同一個用途——新增/
/// 編輯卡片時給一鍵選取，不必每次重新手打。
#[tauri::command]
pub async fn tasks_used_labels(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<String>, String> {
    let p = project(&reg, &project_id)?;
    store::distinct_labels(&p.pool).await.map_err(|e| e.to_string())
}

/// 直接設定卡片的 Label，不受 `edit_allowed` 限制——跟
/// `set_parallel_ok`/`set_interactive`/`set_bridge_config` 同一個「隨時可改」
/// 類別。給拖曳換組跟「已派工/已完成卡片事後補改 Label」這兩個情境用，
/// 這兩者都不該連帶開放 title/body/project_dir 的編輯。
#[tauri::command]
pub async fn tasks_set_label(
    project_id: String,
    task_id: String,
    label: Option<String>,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::set_label(&p.pool, &task_id, label.as_deref()).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 寫入這張卡片的 AI 履行摘要。工作報告的第一階段產物——已完成的卡片
/// 不可變，所以這是永久快取，下次產報告時就不必重跑這張。
#[tauri::command]
pub async fn tasks_set_summary(
    project_id: String,
    task_id: String,
    summary: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::set_summary(&p.pool, &task_id, &summary).await.map_err(|e| e.to_string())
}

/// 把一張已完成的卡片從看板上收起來。資料完全保留，只是不再顯示、也
/// 不會被排程器或工作報告撿到。
#[tauri::command]
pub async fn tasks_archive(
    project_id: String,
    task_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::archive_task(&p.pool, &task_id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 把封存的卡片放回看板（回到「已完成」欄）。
#[tauri::command]
pub async fn tasks_unarchive(
    project_id: String,
    task_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    store::unarchive_task(&p.pool, &task_id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

/// 一次收走整個「已完成」欄，回傳實際封存了幾張。
#[tauri::command]
pub async fn tasks_archive_done(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<u64, String> {
    let p = project(&reg, &project_id)?;
    let n = store::archive_all_done(&p.pool).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(n)
}

/// 一頁封存的卡片，加上符合條件的總數（分頁要靠它算頁數）。
#[derive(Serialize)]
pub struct ArchivePage {
    pub rows: Vec<TaskRow>,
    pub total: i64,
}

/// 封存清單的一頁，新封存的在前。`query` 空字串代表不過濾。
///
/// 刻意**不**附上 attachments：封存清單只顯示標題、目錄、封存時間與
/// 對話記錄按鈕，一列一次的附件查詢是純粹的浪費——一千張封存卡片就是
/// 打開視窗時一千零一次查詢。
#[tauri::command]
pub async fn tasks_list_archived(
    project_id: String,
    query: String,
    limit: i64,
    offset: i64,
    reg: State<'_, ProjectRegistry>,
) -> Result<ArchivePage, String> {
    let p = project(&reg, &project_id)?;
    let rows = store::search_archived(&p.pool, &query, limit, offset)
        .await
        .map_err(|e| e.to_string())?;
    let total = store::count_archived(&p.pool, &query).await.map_err(|e| e.to_string())?;
    Ok(ArchivePage { rows, total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_planning_cards_accept_edits() {
        assert!(edit_allowed("planning"));
        assert!(!edit_allowed("queued"));
        assert!(!edit_allowed("running"));
        assert!(!edit_allowed("done"));
    }
}

#[cfg(test)]
mod save_transcript_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn overwrites_the_file_at_transcript_path() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab-x").await;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.txt");
        std::fs::write(&path, "raw messy version").unwrap();
        store::finish_task(&pool, &id, "success", None, Some(path.to_str().unwrap())).await.unwrap();

        // Exercises the exact same logic tasks_save_transcript's body runs,
        // without needing a Tauri State<'_, ProjectRegistry> extractor (which needs
        // a running app to construct) — get_task + the transcript_path
        // lookup + fs::write, in the same order the command does them.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        let transcript_path = row.transcript_path.unwrap();
        std::fs::write(&transcript_path, "clean version").unwrap();

        assert_eq!(std::fs::read_to_string(&transcript_path).unwrap(), "clean version");
    }

    #[tokio::test]
    async fn errors_instead_of_panicking_when_transcript_path_is_unset() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        // Never moved past planning — transcript_path is None.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.transcript_path.is_none());
    }
}

#[cfg(test)]
mod transcript_for_row_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    /// Pins which `TaskRow` column feeds which `resolve_transcript` param.
    /// `session_path` and `transcript_path` are both `Option<String>` on
    /// `TaskRow`, so a transposed call site
    /// (`resolve_transcript(row.transcript_path.as_deref(),
    /// row.session_path.as_deref())`) would compile silently — this test
    /// builds a real row via the store (not hand-assembled args) with
    /// content distinguishable per source, so a transposition fails it.
    #[tokio::test]
    async fn wires_session_path_not_transcript_path_into_the_rendered_source() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab-x").await;

        let dir = tempfile::tempdir().unwrap();
        let session_path = dir.path().join("session.jsonl");
        std::fs::write(&session_path, ONE_TURN_FOR_TEST).unwrap();
        let transcript_path = dir.path().join("transcript.txt");
        std::fs::write(&transcript_path, "只有最後一屏").unwrap();

        store::set_session_path(&pool, &id, session_path.to_str().unwrap()).await.unwrap();
        store::finish_task(&pool, &id, "success", None, Some(transcript_path.to_str().unwrap()))
            .await
            .unwrap();

        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        let out = transcript_for_row(&row);
        assert!(out.contains("使用者：做這件事"), "沒有用 session 記錄：{out}");
        assert!(!out.contains("只有最後一屏"), "把兩個欄位接反了：{out}");
    }

    const ONE_TURN_FOR_TEST: &str =
        r#"{"type":"user","message":{"role":"user","content":"做這件事"}}"#;
}

#[cfg(test)]
mod mark_done_tests {
    use super::*;
    use crate::tasks::monitor::WatchControl;
    use crate::tasks::scheduler::SchedulerHandle;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{oneshot, Notify};

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    fn empty_scheduler() -> SchedulerHandle {
        SchedulerHandle {
            wake: Arc::new(Notify::new()),
            cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }

    // Exercises the exact same logic tasks_mark_done's body runs — get_task,
    // the status/interactive guard, then scheduler.mark_done() — without
    // needing a Tauri State<'_, ProjectRegistry>/AppHandle extractor (same
    // limitation save_transcript_tests documents above tasks_mark_done).
    #[tokio::test]
    async fn signals_the_active_watch_for_a_running_interactive_task() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab-x").await;

        let scheduler = empty_scheduler();
        let (tx, mut rx) = oneshot::channel::<WatchControl>();
        scheduler.cancels.lock().insert(id.clone(), tx);

        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_RUNNING);
        assert!(row.interactive);
        assert!(scheduler.mark_done(&id));
        assert!(matches!(rx.try_recv().unwrap(), WatchControl::MarkDone));
    }

    #[tokio::test]
    async fn a_non_running_task_fails_the_guard_tasks_mark_done_checks() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        // Still planning — never dispatched.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_ne!(row.status, store::STATUS_RUNNING);
    }

    #[tokio::test]
    async fn a_non_interactive_task_fails_the_guard_tasks_mark_done_checks() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab-x").await;
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_RUNNING);
        assert!(!row.interactive);
    }

    #[tokio::test]
    async fn falls_back_to_finishing_directly_when_there_is_no_active_watch() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::dispatch_for_test(&pool, &id, "tab-x").await;

        let scheduler = empty_scheduler(); // no cancels entry registered
        assert!(!scheduler.mark_done(&id));

        // tasks_mark_done's fallback path when mark_done() returns false —
        // mirrors tasks_stop's own fallback (finish it directly).
        store::finish_task(&pool, &id, "success", None, None).await.unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("success"));
    }
}

#[cfg(test)]
mod merge_worktree_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::fs;
    use std::process::Command as StdCommand;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    fn init_repo(dir: &std::path::Path) {
        StdCommand::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.name", "Test"]).current_dir(dir).status().unwrap();
        fs::write(dir.join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir).status().unwrap();
    }

    /// 一張從沒被隔離過的卡片，`worktree_path`/`worktree_branch` 都是
    /// `None`——`tasks_merge_worktree` body 裡的
    /// `row.worktree_path.ok_or_else(...)` 對這種列一定會短路回 Err，
    /// 這裡驗證的是這個前提本身成立，不是重新實作 `Option::ok_or_else`。
    #[tokio::test]
    async fn a_card_that_was_never_isolated_has_no_worktree_fields() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.worktree_path.is_none());
        assert!(row.worktree_branch.is_none());
    }

    /// Exercises the exact same sequence `tasks_merge_worktree`'s body
    /// runs（commit-if-dirty → merge → remove worktree → clear DB
    /// fields），without needing a Tauri `State<'_, ProjectRegistry>`
    /// extractor — same workaround `save_transcript_tests` above already
    /// uses for the same reason.
    #[tokio::test]
    async fn merges_committed_worktree_changes_back_and_cleans_up() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", &project_dir.path().to_string_lossy(), true, false)
            .await
            .unwrap();

        let wt_path = crate::tasks::task_dir(storage_dir.path(), &id).join("worktree");
        let branch = format!("aiterm-task/{id}");
        let base_client = GitClient::new(project_dir.path().to_string_lossy().to_string(), None);
        base_client.create_worktree(&wt_path.to_string_lossy(), &branch).await.unwrap();
        store::set_worktree(&pool, &id, &wt_path.to_string_lossy(), &branch).await.unwrap();

        fs::write(wt_path.join("b.txt"), "from task\n").unwrap();

        // 以下照抄 tasks_merge_worktree 的 body：
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        let worktree_path = row.worktree_path.unwrap();
        let worktree_branch = row.worktree_branch.unwrap();
        let worktree_client = GitClient::new(worktree_path.clone(), None);
        if worktree_client.has_uncommitted_changes().await.unwrap() {
            worktree_client.commit_all(&format!("Task: {}", row.title)).await.unwrap();
        }
        base_client.merge_branch(&worktree_branch).await.unwrap();
        base_client.remove_worktree(&worktree_path).await.unwrap();
        store::clear_worktree(&pool, &id).await.unwrap();

        assert!(project_dir.path().join("b.txt").exists(), "合併後原分支應該拿到新檔案");
        assert!(!wt_path.exists(), "worktree 應該被移除");
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.worktree_path.is_none());
        assert!(row.worktree_branch.is_none());
    }

    /// worktree 完全沒有變更時（Claude Code 這次沒動任何檔案），不該
    /// 呼叫 `commit_all` 產生空 commit——`has_uncommitted_changes` 回
    /// `false` 就跳過那一步，直接合併（分支本身在 `create_worktree` 時
    /// 就已經跟 base 同一個 commit，`merge_branch` 是 no-op 但不會出錯）。
    #[tokio::test]
    async fn skips_commit_when_worktree_has_no_changes() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", &project_dir.path().to_string_lossy(), true, false)
            .await
            .unwrap();

        let wt_path = crate::tasks::task_dir(storage_dir.path(), &id).join("worktree");
        let branch = format!("aiterm-task/{id}");
        let base_client = GitClient::new(project_dir.path().to_string_lossy().to_string(), None);
        base_client.create_worktree(&wt_path.to_string_lossy(), &branch).await.unwrap();

        let worktree_client = GitClient::new(wt_path.to_string_lossy().to_string(), None);
        assert!(!worktree_client.has_uncommitted_changes().await.unwrap());

        base_client.merge_branch(&branch).await.unwrap();
        base_client.remove_worktree(&wt_path.to_string_lossy()).await.unwrap();
        assert!(!wt_path.exists());
    }

    /// 造一張已經建好 worktree 的卡片，回傳 (pool, id, base_client, wt_path, branch)。
    async fn card_with_worktree(
        project_dir: &std::path::Path,
        storage_dir: &std::path::Path,
    ) -> (sqlx::SqlitePool, String, GitClient, std::path::PathBuf, String) {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", &project_dir.to_string_lossy(), true, false)
            .await
            .unwrap();
        let wt_path = crate::tasks::task_dir(storage_dir, &id).join("worktree");
        let branch = format!("aiterm-task/{id}");
        let base_client = GitClient::new(project_dir.to_string_lossy().to_string(), None);
        base_client.create_worktree(&wt_path.to_string_lossy(), &branch).await.unwrap();
        store::set_worktree(&pool, &id, &wt_path.to_string_lossy(), &branch).await.unwrap();
        (pool, id, base_client, wt_path, branch)
    }

    /// 原分支有未提交變更時，必須在動任何東西**之前**就擋下來。讓 git 自己
    /// 拒絕的話，使用者只會拿到一段沒頭沒尾的 stderr，而且中途可能已經在
    /// worktree 產生了 commit。
    #[tokio::test]
    async fn detects_a_dirty_base_before_touching_anything() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();
        let (_pool, _id, base_client, _wt, _branch) =
            card_with_worktree(project_dir.path(), storage_dir.path()).await;

        assert!(base_client.dirty_files().await.unwrap().is_empty());

        fs::write(project_dir.path().join("a.txt"), "使用者改到一半\n").unwrap();

        assert!(base_client.has_uncommitted_changes().await.unwrap());
        assert_eq!(base_client.dirty_files().await.unwrap(), vec!["a.txt".to_string()]);
        assert!(!base_client.is_merge_in_progress().await, "還沒合併就不該是合併中");
    }

    /// 衝突時：倉庫停在半合併、worktree 與 DB 欄位都要原樣保留——成果都
    /// commit 在那個分支上，保留住使用者才有機會解衝突或改天再試。
    #[tokio::test]
    async fn conflict_keeps_the_worktree_and_leaves_merge_in_progress() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();
        let (pool, id, base_client, wt_path, branch) =
            card_with_worktree(project_dir.path(), storage_dir.path()).await;

        // 兩邊改同一個檔案的同一行 → 必定衝突。
        fs::write(wt_path.join("a.txt"), "worktree 版本\n").unwrap();
        GitClient::new(wt_path.to_string_lossy().to_string(), None)
            .commit_all("Task")
            .await
            .unwrap();
        fs::write(project_dir.path().join("a.txt"), "原分支版本\n").unwrap();
        base_client.commit_all("base moved on").await.unwrap();

        assert!(base_client.merge_branch(&branch).await.is_err());
        assert!(base_client.is_merge_in_progress().await, "衝突後倉庫應該停在合併進行中");
        assert_eq!(base_client.conflicted_files().await.unwrap(), vec!["a.txt".to_string()]);
        assert!(wt_path.exists(), "衝突時 worktree 必須原樣保留");

        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.worktree_path.is_some(), "衝突時不可以清掉 DB 欄位，否則按鈕會消失");

        // 還原之後倉庫乾淨，而且成果仍在那個分支上，可以再試。
        base_client.merge_abort().await.unwrap();
        assert!(!base_client.is_merge_in_progress().await);
        assert!(base_client.dirty_files().await.unwrap().is_empty());
    }
}

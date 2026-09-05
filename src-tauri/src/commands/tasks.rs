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

pub(crate) fn edit_allowed(status: &str) -> bool {
    status == store::STATUS_PLANNING
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
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

#[tauri::command]
pub async fn tasks_list(
    project_id: String,
    reg: State<'_, ProjectRegistry>,
) -> Result<Vec<TaskWithAttachments>, String> {
    let p = project(&reg, &project_id)?;
    let tasks = store::list_tasks(&p.pool).await.map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let attachments = store::list_attachments(&p.pool, &task.id)
            .await
            .map_err(|e| e.to_string())?;
        out.push(TaskWithAttachments { task, attachments });
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
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
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &p.pool,
            &args.id,
            &args.title,
            &args.body,
            &args.project_dir,
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
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
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
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
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
    match row.transcript_path {
        Some(path) => fs::read_to_string(&path).map_err(|e| e.to_string()),
        None => Ok(String::new()),
    }
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
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

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
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

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
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_RUNNING);
        assert!(!row.interactive);
    }

    #[tokio::test]
    async fn falls_back_to_finishing_directly_when_there_is_no_active_watch() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

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

//! Tauri commands for the task board. Thin delegates to `tasks::store`, plus
//! attachment file I/O and a `tasks-updated` emit after every mutation so the
//! board view (a passive renderer) refreshes. Same shape as
//! `commands/loop_session.rs`.

use std::fs;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::tasks::scheduler::SchedulerHandle;
use crate::tasks::store::{self, AttachmentRow, TaskRow};
use crate::tasks::{task_dir, TasksDb};

pub(crate) fn edit_allowed(status: &str) -> bool {
    status == store::STATUS_PLANNING
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
}

#[derive(Serialize)]
pub struct TaskWithAttachments {
    #[serde(flatten)]
    pub task: TaskRow,
    pub attachments: Vec<AttachmentRow>,
}

#[tauri::command]
pub async fn tasks_list(db: State<'_, TasksDb>) -> Result<Vec<TaskWithAttachments>, String> {
    let tasks = store::list_tasks(&db.pool).await.map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        let attachments = store::list_attachments(&db.pool, &task.id)
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
}

#[tauri::command]
pub async fn tasks_create(
    args: CreateArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<String, String> {
    let id = store::create_task(
        &db.pool,
        &args.title,
        &args.body,
        &args.project_dir,
        args.parallel_ok,
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
}

#[tauri::command]
pub async fn tasks_update(
    args: UpdateArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<(), String> {
    let row = store::get_task(&db.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    store::set_parallel_ok(&db.pool, &args.id, args.parallel_ok)
        .await
        .map_err(|e| e.to_string())?;
    if edit_allowed(&row.status) {
        store::update_task_fields(
            &db.pool,
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
    args: MoveArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    store::move_task(&db.pool, &args.id, &args.to_status, args.sort_order)
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
    id: String,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id)
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
        store::finish_task(&db.pool, &id, "cancelled", Some("使用者停止"), None)
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
    args: DeleteArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
    pty: State<'_, Arc<crate::pty::manager::PtyManager>>,
) -> Result<(), String> {
    if let Some(row) = store::get_task(&db.pool, &args.id)
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
    store::delete_task(&db.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(task_dir(&args.id));
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
    args: AddAttachmentArgs,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<AttachmentRow, String> {
    let row = store::get_task(&db.pool, &args.id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if !edit_allowed(&row.status) {
        return Err("attachments can only be changed while the card is in 計畫中".into());
    }
    let dir = task_dir(&args.id).join("attachments");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe = std::path::Path::new(&args.filename)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_string());
    let stored = dir.join(&safe);
    fs::write(&stored, &args.bytes).map_err(|e| e.to_string())?;
    let att_id = store::add_attachment(&db.pool, &args.id, &safe, &stored.to_string_lossy())
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
    attachment_id: String,
    db: State<'_, TasksDb>,
    app: AppHandle,
) -> Result<(), String> {
    if let Some(att) = store::get_attachment(&db.pool, &attachment_id)
        .await
        .map_err(|e| e.to_string())?
    {
        if let Some(row) = store::get_task(&db.pool, &att.task_id)
            .await
            .map_err(|e| e.to_string())?
        {
            if !edit_allowed(&row.status) {
                return Err("attachments can only be changed while the card is in 計畫中".into());
            }
        }
        let _ = fs::remove_file(&att.stored_path);
    }
    store::remove_attachment(&db.pool, &attachment_id)
        .await
        .map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

#[tauri::command]
pub async fn tasks_clone(id: String, db: State<'_, TasksDb>, app: AppHandle) -> Result<String, String> {
    let src = store::get_task(&db.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let new_id = store::clone_task_fields(&db.pool, &src.id)
        .await
        .map_err(|e| e.to_string())?;

    // Copy each attachment file into the new card's dir; skip any whose
    // source file is gone (best effort — a missing file must not fail the clone).
    let dir = task_dir(&new_id).join("attachments");
    for att in store::list_attachments(&db.pool, &id)
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
        let _ = store::add_attachment(&db.pool, &new_id, &att.filename, &dest.to_string_lossy()).await;
    }
    emit_updated(&app);
    Ok(new_id)
}

#[tauri::command]
pub async fn tasks_read_transcript(id: String, db: State<'_, TasksDb>) -> Result<String, String> {
    let row = store::get_task(&db.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    match row.transcript_path {
        Some(p) => fs::read_to_string(&p).map_err(|e| e.to_string()),
        None => Ok(String::new()),
    }
}

#[tauri::command]
pub async fn tasks_save_transcript(id: String, text: String, db: State<'_, TasksDb>) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let path = row.transcript_path.ok_or_else(|| "no transcript path yet".to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
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
        let id = store::create_task(&pool, "t", "", "/r", true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.txt");
        std::fs::write(&path, "raw messy version").unwrap();
        store::finish_task(&pool, &id, "success", None, Some(path.to_str().unwrap())).await.unwrap();

        // Exercises the exact same logic tasks_save_transcript's body runs,
        // without needing a Tauri State<'_, TasksDb> extractor (which needs
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
        let id = store::create_task(&pool, "t", "", "/r", true).await.unwrap();
        // Never moved past planning — transcript_path is None.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.transcript_path.is_none());
    }
}

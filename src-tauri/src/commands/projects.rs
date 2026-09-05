//! 專案的 Tauri 指令。`projects_list` 每次都重新掃描設定裡的路徑清單，
//! 這樣使用者在 Finder 裡刪掉或改動專案資料夾，重新整理就會反映出來。

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::config::ConfigStore;
use crate::projects::{find_aitprj, read_meta, ProjectHandle, ProjectRegistry};
use crate::tasks::store;

#[derive(Serialize)]
pub struct ProjectCounts {
    pub planning: i64,
    pub queued: i64,
    pub running: i64,
    pub done: i64,
}

#[derive(Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    /// `ok` | `missing` | `invalid` | `incompatible`
    pub status: String,
    pub counts: ProjectCounts,
    pub error: Option<String>,
}

fn zero_counts() -> ProjectCounts {
    ProjectCounts { planning: 0, queued: 0, running: 0, done: 0 }
}

async fn counts_of(handle: &ProjectHandle) -> ProjectCounts {
    ProjectCounts {
        planning: store::count_by_status(&handle.pool, store::STATUS_PLANNING).await.unwrap_or(0),
        queued: store::count_by_status(&handle.pool, store::STATUS_QUEUED).await.unwrap_or(0),
        running: store::count_by_status(&handle.pool, store::STATUS_RUNNING).await.unwrap_or(0),
        done: store::count_by_status(&handle.pool, store::STATUS_DONE).await.unwrap_or(0),
    }
}

fn emit_updated(app: &AppHandle) {
    let _ = app.emit("tasks-updated", ());
}

fn paths(config: &ConfigStore) -> Vec<String> {
    config.get().task_board.project_paths
}

/// 只改動 `project_paths` 這一個欄位——`ConfigStore` 沒有整份覆寫的 API，
/// 也不該有：整份覆寫會把同時間別處寫進去的設定蓋掉。
fn set_paths(config: &ConfigStore, next: Vec<String>) -> Result<(), String> {
    config
        .update(|cfg| cfg.task_board.project_paths = next)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn projects_list(
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
) -> Result<Vec<ProjectInfo>, String> {
    let mut out = Vec::new();
    for path in paths(&config) {
        let folder = PathBuf::from(&path);

        // `open_folder` 對已開啟的專案是冪等的（它先讀 `.aitprj` 拿 id，
        // 再看 registry 有沒有），所以這裡不必也不該用路徑字串去比對
        // registry —— symlink 與尾斜線都會讓那種比對失效。
        // 反過來說，資料夾在 Finder 裡被刪掉時這裡就會失敗，正是我們要的。
        match reg.open_folder(&folder).await {
            Ok(handle) => {
                // 名稱與描述一律從磁碟上的 `.aitprj` 重讀，不用 registry
                // 裡那份快取——使用者可能剛改名，或直接編輯了專案檔。
                let meta = find_aitprj(&folder).and_then(|p| read_meta(&p)).ok();
                let (name, description) = match meta {
                    Some(m) => (m.name, m.description),
                    None => (handle.name.clone(), String::new()),
                };
                let counts = counts_of(&handle).await;
                out.push(ProjectInfo {
                    id: handle.id,
                    name,
                    description,
                    path,
                    status: "ok".to_string(),
                    counts,
                    error: None,
                });
            }
            Err(e) => {
                // 這個專案先前可能是開著的（資料夾剛剛才被 Finder 刪掉/
                // 搬走）。registry 裡那個 handle 必須跟著拿掉，否則排程器
                // 每輪都還會看到它。`.aitprj` 已經讀不到、拿不到 id，
                // 所以只能用路徑比對。
                for stale in reg.close_by_path(&folder) {
                    stale.pool.close().await;
                }
                let name = folder
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                out.push(ProjectInfo {
                    // 開不起來就沒有真正的 id，用路徑當暫時識別，
                    // 前端只會拿它來呼叫 projects_remove。
                    id: format!("unopened:{path}"),
                    name,
                    description: String::new(),
                    path,
                    status: e.status_str().to_string(),
                    counts: zero_counts(),
                    error: Some(e.to_string()),
                });
            }
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct CreateProjectArgs {
    pub parent_dir: String,
    pub name: String,
    pub description: String,
}

#[tauri::command]
pub async fn projects_create(
    args: CreateProjectArgs,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<String, String> {
    let handle = reg
        .create(std::path::Path::new(&args.parent_dir), &args.name, &args.description)
        .await
        .map_err(|e| e.to_string())?;
    let mut next = paths(&config);
    let path_str = handle.path.to_string_lossy().into_owned();
    if !next.contains(&path_str) {
        next.push(path_str);
    }
    set_paths(&config, next)?;
    emit_updated(&app);
    Ok(handle.id)
}

/// 開啟既有專案 —— 這同時就是「匯入」：別台機器複製過來的資料夾，
/// 挑它的 `.aitprj` 就進來了。
#[tauri::command]
pub async fn projects_open(
    aitprj_path: String,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<String, String> {
    let folder = PathBuf::from(&aitprj_path)
        .parent()
        .ok_or_else(|| "無效的專案檔路徑".to_string())?
        .to_path_buf();

    // 先只讀專案檔：這既驗證了資料夾，又讓重複檢查發生在開啟之前，
    // 不會留下一個「被拒絕但已經開起來」的 registry 項目。
    let meta = read_meta(&find_aitprj(&folder).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    let mut next = paths(&config);
    let path_str = folder.to_string_lossy().into_owned();
    if next.contains(&path_str) || reg.get(&meta.id).is_some() {
        return Err(format!("專案「{}」已在清單中", meta.name));
    }

    let handle = reg.open_folder(&folder).await.map_err(|e| e.to_string())?;
    next.push(path_str);
    set_paths(&config, next)?;
    emit_updated(&app);
    Ok(handle.id)
}

#[derive(Deserialize)]
pub struct RemoveProjectArgs {
    pub id: String,
    pub delete_folder: bool,
}

#[tauri::command]
pub async fn projects_remove(
    args: RemoveProjectArgs,
    reg: State<'_, ProjectRegistry>,
    config: State<'_, Arc<ConfigStore>>,
    app: AppHandle,
) -> Result<(), String> {
    // 未能開啟的專案，前端送來的 id 是 "unopened:<path>"。
    let folder: PathBuf = if let Some(p) = args.id.strip_prefix("unopened:") {
        PathBuf::from(p)
    } else {
        let handle = reg
            .get(&args.id)
            .ok_or_else(|| format!("專案不存在：{}", args.id))?;
        let running = store::count_by_status(&handle.pool, store::STATUS_RUNNING)
            .await
            .map_err(|e| e.to_string())?;
        if running > 0 {
            return Err("這個專案還有工作在執行中，請先停止它們".to_string());
        }
        reg.close(&args.id);
        // Windows 不讓你刪掉還被開著的檔案，所以 `delete_folder` 之前
        // 一定要先把 `tasks.db` 的連線收乾淨，不能只靠 Drop。
        handle.pool.close().await;
        handle.path
    };

    let path_str = folder.to_string_lossy().into_owned();
    let next: Vec<String> = paths(&config).into_iter().filter(|p| p != &path_str).collect();
    set_paths(&config, next)?;

    if args.delete_folder {
        std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())?;
    }
    emit_updated(&app);
    Ok(())
}

#[derive(Deserialize)]
pub struct RenameProjectArgs {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 只改 `.aitprj` 裡的顯示名稱與描述，**不動磁碟路徑**。
///
/// 刻意不把專案從 registry 關掉：使用者很可能正開著這個專案的看板，
/// 關掉會讓後續每個 `tasks_*` 指令都失敗。`projects_list` 本來就每次
/// 從磁碟重讀名稱，所以下一次重新整理就會顯示新名字。
#[tauri::command]
pub async fn projects_rename(
    args: RenameProjectArgs,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let handle = reg.get(&args.id).ok_or_else(|| format!("專案不存在：{}", args.id))?;
    let meta_path = find_aitprj(&handle.path).map_err(|e| e.to_string())?;
    let mut meta = read_meta(&meta_path).map_err(|e| e.to_string())?;
    meta.name = args.name;
    meta.description = args.description;
    crate::projects::write_meta(&meta_path, &meta).map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}

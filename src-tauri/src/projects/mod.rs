//! 專案 = 磁碟上一個自成一體的資料夾：
//!   <folder>/<name>.aitprj    專案清單檔（本模組負責）
//!   <folder>/tasks.db         這個專案的卡片（schema 同 tasks::init_schema）
//!   <folder>/tasks/<id>/      附件與對話記錄
//!
//! 見 docs/superpowers/specs/2026-09-04-task-board-projects-design.md

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod migrate;
pub mod naming;

/// 本程式認得的 `.aitprj` 格式版本。讀到比這個大的版本一律拒絕，
/// 不猜測、不嘗試向前相容。
pub const SCHEMA_VERSION: u32 = 1;

/// `.aitprj` 的內容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub schema: u32,
    /// UUID，不是路徑——資料夾改名或搬家後仍是同一個專案。
    pub id: String,
    /// 顯示名稱，與資料夾名稱獨立。重新命名專案只改這裡。
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub created_at: String,
}

impl ProjectMeta {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Debug)]
pub enum ProjectError {
    /// 資料夾或 `.aitprj` 不存在。
    Missing,
    /// `.aitprj` 存在但無法解析。
    Invalid(String),
    /// `.aitprj` 的 schema 版本高於本程式支援。
    Incompatible(u32),
    /// `tasks.db` 開啟或建立失敗。
    Db(String),
    /// 檔案系統操作失敗。
    Io(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Missing => write!(f, "專案資料夾或專案檔不存在"),
            ProjectError::Invalid(m) => write!(f, "專案檔無法讀取：{m}"),
            ProjectError::Incompatible(v) => {
                write!(f, "專案檔格式版本 {v} 高於本版本支援的 {SCHEMA_VERSION}，請更新 AITerm")
            }
            ProjectError::Db(m) => write!(f, "專案資料庫錯誤：{m}"),
            ProjectError::Io(m) => write!(f, "檔案操作失敗：{m}"),
        }
    }
}

/// 供 `projects_list` 回報用的機器可讀狀態字串。
impl ProjectError {
    pub fn status_str(&self) -> &'static str {
        match self {
            ProjectError::Missing => "missing",
            ProjectError::Invalid(_) => "invalid",
            ProjectError::Incompatible(_) => "incompatible",
            ProjectError::Db(_) | ProjectError::Io(_) => "invalid",
        }
    }
}

pub fn read_meta(path: &Path) -> Result<ProjectMeta, ProjectError> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ProjectError::Missing),
        Err(e) => return Err(ProjectError::Io(e.to_string())),
    };
    // 先只取 schema 欄位判斷版本：未來版本可能加了本版反序列化不了的欄位，
    // 那種情況要回報 Incompatible（可修正：叫使用者更新），
    // 不能回報 Invalid（看起來像檔案壞掉，會誤導使用者去刪檔）。
    let probe: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ProjectError::Invalid(e.to_string()))?;
    let schema = probe.get("schema").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    if schema > SCHEMA_VERSION {
        return Err(ProjectError::Incompatible(schema));
    }
    serde_json::from_str(&text).map_err(|e| ProjectError::Invalid(e.to_string()))
}

pub fn write_meta(path: &Path, meta: &ProjectMeta) -> Result<(), ProjectError> {
    let text = serde_json::to_string_pretty(meta).map_err(|e| ProjectError::Invalid(e.to_string()))?;
    std::fs::write(path, text).map_err(|e| ProjectError::Io(e.to_string()))
}

/// 找出資料夾裡的 `.aitprj`。多於一個時取檔名排序的第一個——
/// 這種情況只會發生在使用者手動複製檔案，取一個穩定的結果比報錯有用。
pub fn find_aitprj(folder: &Path) -> Result<PathBuf, ProjectError> {
    let entries = match std::fs::read_dir(folder) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ProjectError::Missing),
        Err(e) => return Err(ProjectError::Io(e.to_string())),
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("aitprj"))
        .collect();
    found.sort();
    found.into_iter().next().ok_or(ProjectError::Missing)
}

use std::collections::HashMap;

use parking_lot::RwLock;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

/// 一個已開啟的專案。`pool` 與 `path` 都便宜可複製，
/// 所以這個型別直接 `Clone` 出去給呼叫端持有，
/// 不需要在每個使用點回頭查 registry。
#[derive(Debug, Clone)]
pub struct ProjectHandle {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub pool: SqlitePool,
}

/// 目前開啟中的專案。由 Tauri `.manage()` 持有，取代舊的 `TasksDb` 單例。
#[derive(Default)]
pub struct ProjectRegistry {
    open: RwLock<HashMap<String, ProjectHandle>>,
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &str) -> Option<ProjectHandle> {
        self.open.read().get(id).cloned()
    }

    pub fn all(&self) -> Vec<ProjectHandle> {
        self.open.read().values().cloned().collect()
    }

    /// 從 registry 移除。**不刪磁碟上的任何東西。**
    pub fn close(&self, id: &str) {
        self.open.write().remove(id);
    }

    /// 移除所有指向 `folder` 的專案，回傳被移除的 handle（呼叫端負責
    /// 關掉它們的連線池）。
    ///
    /// 資料夾在 App 執行中被外部刪掉/搬走時用。這時 `.aitprj` 已經讀不到、
    /// 拿不到 id，所以只能用路徑比對——`open_folder` 存進來的 path 就是
    /// 設定裡那個字串轉成的 `PathBuf`，而檢查的人走的是同一份設定，
    /// 兩邊一致。
    pub fn close_by_path(&self, folder: &Path) -> Vec<ProjectHandle> {
        let mut open = self.open.write();
        let ids: Vec<String> =
            open.values().filter(|h| h.path == folder).map(|h| h.id.clone()).collect();
        ids.iter().filter_map(|id| open.remove(id)).collect()
    }

    /// 建立新專案資料夾並開啟。`parent` 必須已存在。
    pub async fn create(
        &self,
        parent: &Path,
        name: &str,
        description: &str,
    ) -> Result<ProjectHandle, ProjectError> {
        let folder_name = naming::safe_folder_name(name);
        let folder = parent.join(&folder_name);

        // 已存在且非空 → 拒絕。空資料夾則可以直接用（使用者可能先建好了）。
        if folder.exists() {
            let empty = std::fs::read_dir(&folder)
                .map_err(|e| ProjectError::Io(e.to_string()))?
                .next()
                .is_none();
            if !empty {
                return Err(ProjectError::Io(format!("資料夾已存在且不是空的：{}", folder.display())));
            }
        }
        std::fs::create_dir_all(&folder).map_err(|e| ProjectError::Io(e.to_string()))?;

        let meta = ProjectMeta::new(name, description);
        write_meta(&folder.join(format!("{folder_name}.aitprj")), &meta)?;

        let pool = open_pool(&folder).await?;
        let handle = ProjectHandle { id: meta.id.clone(), name: meta.name, path: folder, pool };
        self.open.write().insert(handle.id.clone(), handle.clone());
        Ok(handle)
    }

    /// 開啟既有專案資料夾。已開啟時直接回傳現有的 handle。
    pub async fn open_folder(&self, folder: &Path) -> Result<ProjectHandle, ProjectError> {
        let meta_path = find_aitprj(folder)?;
        let meta = read_meta(&meta_path)?;

        if let Some(existing) = self.get(&meta.id) {
            return Ok(existing);
        }

        let pool = open_pool(folder).await?;
        let handle = ProjectHandle {
            id: meta.id.clone(),
            name: meta.name,
            path: folder.to_path_buf(),
            pool,
        };
        self.open.write().insert(handle.id.clone(), handle.clone());
        Ok(handle)
    }
}

/// 開啟（必要時建立）專案資料夾裡的 `tasks.db` 並確保 schema 存在。
///
/// 刻意**不**沿用舊 `TasksDb::new()` 那個「開啟失敗就換成
/// `sqlite::memory:`」的回退——那會靜默吃掉使用者的卡片。
/// 這裡失敗就是失敗，交給呼叫端顯示錯誤。
async fn open_pool(folder: &Path) -> Result<SqlitePool, ProjectError> {
    let options = SqliteConnectOptions::new()
        .filename(folder.join("tasks.db"))
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
    crate::tasks::init_schema(&pool)
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
    Ok(pool)
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let meta = ProjectMeta::new("makemoney", "賺錢用");
        let path = dir.path().join("makemoney.aitprj");
        write_meta(&path, &meta).unwrap();

        let back = read_meta(&path).unwrap();
        assert_eq!(back.id, meta.id);
        assert_eq!(back.name, "makemoney");
        assert_eq!(back.description, "賺錢用");
        assert_eq!(back.schema, SCHEMA_VERSION);
    }

    #[test]
    fn a_missing_file_is_missing_not_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_meta(&dir.path().join("nope.aitprj")).unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }

    #[test]
    fn malformed_json_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.aitprj");
        std::fs::write(&path, "{ not json").unwrap();
        let err = read_meta(&path).unwrap_err();
        assert!(matches!(err, ProjectError::Invalid(_)), "{err:?}");
    }

    #[test]
    fn a_newer_schema_is_incompatible_not_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("future.aitprj");
        std::fs::write(
            &path,
            r#"{"schema":999,"id":"x","name":"n","description":"","created_at":"2026-09-04T00:00:00Z"}"#,
        )
        .unwrap();
        let err = read_meta(&path).unwrap_err();
        assert!(matches!(err, ProjectError::Incompatible(999)), "{err:?}");
    }

    #[test]
    fn finds_the_aitprj_inside_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.aitprj");
        write_meta(&path, &ProjectMeta::new("hello", "")).unwrap();
        assert_eq!(find_aitprj(dir.path()).unwrap(), path);
    }

    #[test]
    fn a_folder_with_no_aitprj_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = find_aitprj(dir.path()).unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[tokio::test]
    async fn create_then_get_returns_the_same_project() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();

        let handle = reg.create(parent.path(), "makemoney", "賺錢").await.unwrap();
        assert_eq!(handle.name, "makemoney");
        assert!(handle.path.join("makemoney.aitprj").is_file());
        assert!(handle.path.join("tasks.db").is_file());

        let again = reg.get(&handle.id).unwrap();
        assert_eq!(again.id, handle.id);
        assert_eq!(again.path, handle.path);
    }

    #[tokio::test]
    async fn the_new_projects_db_has_the_tasks_schema() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "p", "").await.unwrap();

        // 能建立卡片就證明 init_schema 跑過了
        let id = crate::tasks::store::create_task(&handle.pool, "t", "b", "/r", true, false)
            .await
            .unwrap();
        let rows = crate::tasks::store::list_tasks(&handle.pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
    }

    #[tokio::test]
    async fn creating_into_an_existing_non_empty_folder_is_rejected() {
        let parent = tempfile::tempdir().unwrap();
        std::fs::create_dir(parent.path().join("taken")).unwrap();
        std::fs::write(parent.path().join("taken").join("something.txt"), "x").unwrap();

        let reg = ProjectRegistry::new();
        let err = reg.create(parent.path(), "taken", "").await.unwrap_err();
        assert!(matches!(err, ProjectError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn open_folder_loads_an_existing_project() {
        let parent = tempfile::tempdir().unwrap();
        let created = {
            let reg = ProjectRegistry::new();
            reg.create(parent.path(), "reopen", "").await.unwrap()
        };

        // 全新的 registry（模擬重新啟動）
        let reg2 = ProjectRegistry::new();
        let opened = reg2.open_folder(&created.path).await.unwrap();
        assert_eq!(opened.id, created.id);
        assert_eq!(opened.name, "reopen");
    }

    #[tokio::test]
    async fn open_folder_on_a_missing_folder_is_missing() {
        let reg = ProjectRegistry::new();
        let err = reg
            .open_folder(std::path::Path::new("/definitely/not/here"))
            .await
            .unwrap_err();
        assert!(matches!(err, ProjectError::Missing), "{err:?}");
    }

    #[tokio::test]
    async fn close_removes_it_from_the_registry_but_leaves_the_folder() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let handle = reg.create(parent.path(), "keepme", "").await.unwrap();

        reg.close(&handle.id);
        assert!(reg.get(&handle.id).is_none());
        assert!(handle.path.join("keepme.aitprj").is_file(), "關閉不可刪檔");
    }

    #[tokio::test]
    async fn all_lists_every_open_project() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        reg.create(parent.path(), "a", "").await.unwrap();
        reg.create(parent.path(), "b", "").await.unwrap();
        let mut names: Vec<String> = reg.all().into_iter().map(|h| h.name).collect();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    /// 資料夾在 App 執行中被 Finder 刪掉時，`.aitprj` 已經讀不到，
    /// 拿不到 id——只能用路徑比對把 registry 裡那個 handle 清掉。
    #[tokio::test]
    async fn close_by_path_evicts_only_the_matching_project() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let gone = reg.create(parent.path(), "gone", "").await.unwrap();
        let kept = reg.create(parent.path(), "kept", "").await.unwrap();

        // 先關池子再刪：Windows 不允許刪除還有檔案被開著的目錄，
        // 而 `tasks.db` 正被這個池子開著。
        gone.pool.close().await;
        std::fs::remove_dir_all(&gone.path).unwrap();
        let evicted = reg.close_by_path(&gone.path);

        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].id, gone.id);
        assert!(reg.get(&gone.id).is_none(), "消失的專案必須被驅逐");
        assert!(reg.get(&kept.id).is_some(), "其他專案不可受影響");
    }

    #[tokio::test]
    async fn close_by_path_on_an_unknown_path_is_a_no_op() {
        let parent = tempfile::tempdir().unwrap();
        let reg = ProjectRegistry::new();
        let kept = reg.create(parent.path(), "kept", "").await.unwrap();

        assert!(reg.close_by_path(std::path::Path::new("/nope")).is_empty());
        assert!(reg.get(&kept.id).is_some());
    }
}

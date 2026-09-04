//! 專案 = 磁碟上一個自成一體的資料夾：
//!   <folder>/<name>.aitprj    專案清單檔（本模組負責）
//!   <folder>/tasks.db         這個專案的卡片（schema 同 tasks::init_schema）
//!   <folder>/tasks/<id>/      附件與對話記錄
//!
//! 見 docs/superpowers/specs/2026-09-04-task-board-projects-design.md

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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

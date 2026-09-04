//! 一次性搬遷：把專案化之前那份全域 `tasks.db` 的卡片，
//! 連同附件與對話記錄，複製成一個名為「預設專案」的專案資料夾。
//!
//! 刻意用**複製**而非搬移：搬遷若有 bug，使用者的原始資料仍在原地。

use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

use super::{write_meta, ProjectError, ProjectMeta};
use crate::tasks::store;

/// 搬遷完成標記。放在磁碟上而非設定檔裡——設定檔被重置時
/// 不該導致資料被重複搬遷一次。
pub const MARKER: &str = ".projects_migrated";

const DEFAULT_PROJECT_NAME: &str = "預設專案";

/// 若 `root`（＝ `tasks::app_data_dir()`）底下有尚未搬遷的舊資料，
/// 搬成一個專案資料夾並回傳它的路徑。已搬過或沒有舊資料時回傳 `None`。
pub async fn migrate_legacy(root: &Path) -> Result<Option<PathBuf>, ProjectError> {
    let marker = root.join(MARKER);
    if marker.exists() {
        return Ok(None);
    }

    let legacy_db = root.join("tasks.db");
    if !legacy_db.is_file() {
        touch_marker(&marker)?;
        return Ok(None);
    }

    // 舊 db 有卡片嗎？沒有的話不值得建一個空專案。
    let count = {
        let options = SqliteConnectOptions::new().filename(&legacy_db);
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| ProjectError::Db(e.to_string()))?;
        let n = store::count_all(&pool).await.map_err(|e| ProjectError::Db(e.to_string()))?;
        pool.close().await;
        n
    };
    if count == 0 {
        touch_marker(&marker)?;
        return Ok(None);
    }

    let folder_name = super::naming::safe_folder_name(DEFAULT_PROJECT_NAME);
    let dest = root.join("projects").join(&folder_name);
    std::fs::create_dir_all(&dest).map_err(|e| ProjectError::Io(e.to_string()))?;

    std::fs::copy(&legacy_db, dest.join("tasks.db")).map_err(|e| ProjectError::Io(e.to_string()))?;
    let legacy_tasks = root.join("tasks");
    if legacy_tasks.is_dir() {
        copy_dir_all(&legacy_tasks, &dest.join("tasks")).map_err(|e| ProjectError::Io(e.to_string()))?;
    }

    // 資料庫裡存的是絕對路徑，指向舊位置。不改寫的話這個專案資料夾
    // 就不是自成一體的——複製到別台機器會掉附件與對話記錄。
    {
        let options = SqliteConnectOptions::new().filename(dest.join("tasks.db"));
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|e| ProjectError::Db(e.to_string()))?;
        store::rewrite_stored_paths(
            &pool,
            &root.to_string_lossy(),
            &dest.to_string_lossy(),
        )
        .await
        .map_err(|e| ProjectError::Db(e.to_string()))?;
        pool.close().await;
    }

    let meta = ProjectMeta::new(DEFAULT_PROJECT_NAME, "專案功能上線前既有的工作");
    write_meta(&dest.join(format!("{folder_name}.aitprj")), &meta)?;

    touch_marker(&marker)?;
    Ok(Some(dest))
}

fn touch_marker(marker: &Path) -> Result<(), ProjectError> {
    std::fs::write(marker, "").map_err(|e| ProjectError::Io(e.to_string()))
}

/// std 沒有遞迴複製目錄的函式，自己寫一個。
fn copy_dir_all(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::SqlitePool;

    /// 造一份「舊版」資料區：tasks.db 裡有一張卡片，
    /// 附件與對話記錄放在 <root>/tasks/<id>/ 底下，
    /// 而且資料庫裡存的是它們的絕對路徑（就跟真實情況一樣）。
    async fn legacy_root() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let db_path = root.path().join("tasks.db");
        let options = SqliteConnectOptions::new().filename(&db_path).create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();

        let id = store::create_task(&pool, "舊卡片", "body", "/repo", true, false).await.unwrap();
        let task_dir = root.path().join("tasks").join(&id);
        std::fs::create_dir_all(task_dir.join("attachments")).unwrap();
        let att = task_dir.join("attachments").join("f.txt");
        std::fs::write(&att, "attachment content").unwrap();
        store::add_attachment(&pool, &id, "f.txt", att.to_str().unwrap()).await.unwrap();

        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab").await.unwrap();
        let transcript = task_dir.join("transcript.txt");
        std::fs::write(&transcript, "transcript content").unwrap();
        store::finish_task(&pool, &id, "success", None, transcript.to_str().unwrap().into())
            .await
            .unwrap();

        pool.close().await;
        root
    }

    #[tokio::test]
    async fn copies_cards_files_and_repoints_paths() {
        let root = legacy_root().await;
        let dest = migrate_legacy(root.path()).await.unwrap().expect("應該有搬遷");

        // 卡片進來了
        let options = SqliteConnectOptions::new().filename(dest.join("tasks.db"));
        let pool = SqlitePool::connect_with(options).await.unwrap();
        let rows = store::list_tasks(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "舊卡片");

        // 路徑改寫成新家，而且檔案真的在那裡（＝資料夾是自成一體的）
        let transcript = rows[0].transcript_path.clone().unwrap();
        assert!(transcript.starts_with(dest.to_str().unwrap()), "{transcript}");
        assert_eq!(std::fs::read_to_string(&transcript).unwrap(), "transcript content");

        let atts = store::list_attachments(&pool, &rows[0].id).await.unwrap();
        assert!(atts[0].stored_path.starts_with(dest.to_str().unwrap()));
        assert_eq!(std::fs::read_to_string(&atts[0].stored_path).unwrap(), "attachment content");
    }

    #[tokio::test]
    async fn leaves_the_original_files_untouched() {
        let root = legacy_root().await;
        migrate_legacy(root.path()).await.unwrap().unwrap();
        assert!(root.path().join("tasks.db").is_file(), "舊 db 必須保留作為備份");
        assert!(root.path().join("tasks").is_dir(), "舊 tasks/ 必須保留");
    }

    #[tokio::test]
    async fn writes_a_marker_and_does_not_run_twice() {
        let root = legacy_root().await;
        assert!(migrate_legacy(root.path()).await.unwrap().is_some());
        assert!(root.path().join(MARKER).is_file());
        assert!(
            migrate_legacy(root.path()).await.unwrap().is_none(),
            "第二次呼叫不可再搬一次"
        );
    }

    #[tokio::test]
    async fn does_nothing_when_there_is_no_legacy_db() {
        let root = tempfile::tempdir().unwrap();
        assert!(migrate_legacy(root.path()).await.unwrap().is_none());
        assert!(root.path().join(MARKER).is_file(), "沒有舊資料也要留標記，避免每次啟動重掃");
    }

    #[tokio::test]
    async fn does_nothing_when_the_legacy_db_is_empty() {
        let root = tempfile::tempdir().unwrap();
        let options = SqliteConnectOptions::new()
            .filename(root.path().join("tasks.db"))
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool.close().await;

        assert!(migrate_legacy(root.path()).await.unwrap().is_none());
    }
}

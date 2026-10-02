//! Task board: a queued work list that dispatches cards to `claude`.
//! See `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`.
//!
//! - `store`     — `tasks.db` schema + CRUD (sqlx free functions over a pool)
//! - `session_log` — Claude Code 自己寫的 session JSONL：定位、複製、渲染
//! - `dispatch`  — compose the prompt, spawn a visible PTY tab, type it in
//! - `monitor`   — watch one running task's session to a terminal outcome
//! - `scheduler` — pick the next runnable card; the long-lived dispatch loop

pub mod store;
pub mod session_log;
pub mod dispatch;
pub mod monitor;
pub mod scheduler;

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

/// `<data-dir>/AITERM` — the same app data directory every other
/// dedicated-SQLite module in this codebase uses.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("AITERM")
}

/// `<project-folder>/tasks/<task_id>` — 這張卡片的附件與對話記錄。
/// 由 dispatch/store 在需要時建立。
///
/// 根目錄是**專案資料夾**而非全域資料區——專案資料夾必須自成一體，
/// 這樣複製走就等於匯出。
pub fn task_dir(project_path: &Path, task_id: &str) -> PathBuf {
    project_path.join("tasks").join(task_id)
}

/// 資料庫裡記「專案內的檔案」用的路徑：**相對於專案資料夾**、一律用 `/` 分隔。
///
/// 專案資料夾設計成自成一體、可以整個搬走（見 [`task_dir`]），存絕對路徑等於
/// 搬完之後全部指向舊位置。用 `/` 而不是平台分隔符，是為了同一份資料夾在
/// macOS 與 Windows 之間複製後仍然讀得到（Windows 的 `join` 也接受 `/`）。
///
/// `abs` 不在 `project_path` 底下（理論上不會發生）時退回原本的絕對路徑，
/// 讀取端仍然讀得到，只是不具可搬遷性。
pub fn to_stored_path(project_path: &Path, abs: &Path) -> String {
    match abs.strip_prefix(project_path) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => abs.to_string_lossy().into_owned(),
    }
}

/// [`to_stored_path`] 的反向。**絕對路徑原樣回傳**：這個版本之前寫入的舊卡片
/// 存的是絕對路徑，它們照舊能讀（搬遷時仍靠 `store::rewrite_stored_paths`）。
///
/// 另一種作業系統的絕對路徑（Windows 的 `C:\...` 複製到 macOS，或反過來）在本機
/// `is_absolute()` 為 false，原樣接在專案資料夾後面一定指向不存在的檔案。
/// 卡片檔案永遠在 `<專案>/tasks/<id>/` 底下，所以取最後一個 `tasks` 段之後的
/// 部分接到本機專案資料夾；找不到 `tasks` 段就維持原行為。
pub fn resolve_stored_path(project_path: &Path, stored: &str) -> PathBuf {
    let p = Path::new(stored);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    if looks_absolute_on_another_platform(stored) {
        let unified = stored.replace('\\', "/");
        if let Some(i) = unified.rfind("/tasks/") {
            return project_path.join(&unified[i + 1..]);
        }
    }
    project_path.join(p)
}

/// 本機判為相對、但長得像絕對路徑：磁碟機代號（`C:\`、`C:/`）、UNC（`\\`）、
/// 或開頭 `/`（Windows 上判為相對）。
fn looks_absolute_on_another_platform(stored: &str) -> bool {
    let b = stored.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    drive || stored.starts_with("\\\\") || stored.starts_with('/')
}

pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tasks (
            id              TEXT PRIMARY KEY NOT NULL,
            title           TEXT NOT NULL,
            body            TEXT NOT NULL DEFAULT '',
            project_dir     TEXT NOT NULL,
            status          TEXT NOT NULL DEFAULT 'planning',
            parallel_ok     INTEGER NOT NULL DEFAULT 1,
            interactive     INTEGER NOT NULL DEFAULT 0,
            sort_order      REAL NOT NULL DEFAULT 0,
            outcome         TEXT,
            tab_id          TEXT,
            transcript_path TEXT,
            error_message   TEXT,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            dispatched_at   INTEGER,
            finished_at     INTEGER,
            ai_summary      TEXT,
            archived_at     INTEGER,
            session_id      TEXT,
            session_path    TEXT,
            use_bridge      INTEGER NOT NULL DEFAULT 0,
            bridge_tiers    TEXT,
            label           TEXT,
            worktree_path   TEXT,
            worktree_branch TEXT,
            isolate_worktree INTEGER
        )",
    )
    .execute(pool)
    .await?;
    // Migration: existing databases created before `interactive` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    // Migration: existing databases created before `ai_summary` existed.
    // 跟上面的 `interactive` 同一個寫法——欄位已存在時 ALTER TABLE 會
    // 失敗，那是正常的，所以刻意丟掉錯誤。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN ai_summary TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `archived_at` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN archived_at INTEGER")
        .execute(pool)
        .await;
    // Migration: existing databases created before the session-log columns
    // existed. 跟上面幾個同一個寫法——欄位已存在時 ALTER TABLE 會失敗，
    // 那是正常的，所以刻意丟掉錯誤。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN session_id TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN session_path TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `use_bridge`/`bridge_tiers`
    // existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN use_bridge INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN bridge_tiers TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `label` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN label TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `worktree_path`/
    // `worktree_branch` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_path TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_branch TEXT")
        .execute(pool)
        .await;
    // Migration: existing databases created before `isolate_worktree` existed.
    // 刻意不給 DEFAULT——NULL 代表「沿用全域設定」，跟 false 是不同的意思。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN isolate_worktree INTEGER")
        .execute(pool)
        .await;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status, sort_order)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS task_attachments (
            id          TEXT PRIMARY KEY NOT NULL,
            task_id     TEXT NOT NULL,
            filename    TEXT NOT NULL,
            stored_path TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_task_attachments_task ON task_attachments(task_id)")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod task_dir_tests {
    use super::*;

    #[test]
    fn is_rooted_at_the_project_folder() {
        let project = std::path::Path::new("/projects/makemoney");
        assert_eq!(task_dir(project, "abc"), project.join("tasks").join("abc"));
    }
}

#[cfg(test)]
mod stored_path_tests {
    use super::*;

    #[test]
    fn a_path_inside_the_project_is_stored_relative_with_forward_slashes() {
        let project = Path::new("/projects/makemoney");
        let abs = task_dir(project, "abc").join("session.jsonl");
        assert_eq!(to_stored_path(project, &abs), "tasks/abc/session.jsonl");
    }

    #[test]
    fn a_path_outside_the_project_falls_back_to_the_absolute_path() {
        let stored = to_stored_path(Path::new("/projects/a"), Path::new("/elsewhere/s.jsonl"));
        assert_eq!(stored, "/elsewhere/s.jsonl");
    }

    #[test]
    fn a_relative_path_resolves_against_the_project_folder() {
        let project = Path::new("/projects/makemoney");
        assert_eq!(
            resolve_stored_path(project, "tasks/abc/session.jsonl"),
            project.join("tasks").join("abc").join("session.jsonl")
        );
    }

    /// 舊卡片存的是絕對路徑，不能因為新的解析規則就讀不到。
    #[test]
    fn an_absolute_legacy_path_is_returned_untouched() {
        let project = Path::new("/projects/new-home");
        let legacy = if cfg!(windows) { r"C:\old\tasks\x\session.jsonl" } else { "/old/tasks/x/session.jsonl" };
        assert_eq!(resolve_stored_path(project, legacy), PathBuf::from(legacy));
    }

    /// 從另一種作業系統複製過來的專案：舊卡片存的是對方平台的絕對路徑
    /// （例如 Windows 的 `C:\...\tasks\<id>\session.jsonl`），在這台機器上
    /// `is_absolute()` 為 false，若原樣接在專案資料夾後面就是不存在的路徑，
    /// 對話記錄會安靜地退回空白。卡片資料夾永遠是 `<專案>/tasks/<id>/`，
    /// 所以取 `tasks/` 之後的部分重新接到本機的專案資料夾即可。
    #[test]
    fn a_foreign_platform_absolute_path_maps_into_the_local_project_folder() {
        let (project, foreign) = if cfg!(windows) {
            (Path::new(r"C:\Users\me\ARESGUI"), "/Users/old/AITERMProjects/ARESGUI/tasks/abc/session.jsonl")
        } else {
            (Path::new("/Users/me/ARESGUI"), r"C:\AITERMProjects\ARESGUI\tasks\abc\session.jsonl")
        };
        assert_eq!(
            resolve_stored_path(project, foreign),
            project.join("tasks").join("abc").join("session.jsonl")
        );
    }

    #[test]
    fn a_foreign_attachment_path_keeps_its_subfolder_and_filename() {
        if cfg!(windows) {
            return;
        }
        let project = Path::new("/Users/me/ARESGUI");
        assert_eq!(
            resolve_stored_path(project, r"D:\tasks\ARESGUI\tasks\abc\attachments\shot 1.png"),
            project.join("tasks/abc/attachments/shot 1.png"),
            "專案資料夾本身叫 tasks 時要取最後一個 tasks 段"
        );
    }

    #[test]
    fn a_foreign_path_without_a_tasks_segment_is_left_alone() {
        if cfg!(windows) {
            return;
        }
        let project = Path::new("/Users/me/ARESGUI");
        assert_eq!(
            resolve_stored_path(project, r"C:\somewhere\else.png"),
            project.join(r"C:\somewhere\else.png")
        );
    }

    #[test]
    fn store_then_resolve_round_trips_and_survives_moving_the_project() {
        let old_home = Path::new("/projects/old");
        let stored = to_stored_path(old_home, &task_dir(old_home, "abc").join("session.jsonl"));
        let new_home = Path::new("/somewhere/else/entirely");
        assert_eq!(
            resolve_stored_path(new_home, &stored),
            task_dir(new_home, "abc").join("session.jsonl"),
            "搬遷後要指向新位置，不是舊位置"
        );
    }
}

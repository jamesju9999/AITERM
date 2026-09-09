# 工作看板派工 Git Worktree 隔離（第 1 階段）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 工作看板派工時，若 `task.project_dir` 是 git repo，自動在獨立的 git worktree + 新分支裡跑 Claude Code，避免 `parallel_ok=true` 的多張卡片同時改動同一份 working tree；卡片跑完後看板上多一顆「合併回原分支」按鈕，把 worktree 的變更合併回去並清理。非 git 專案完全不受影響。

**Architecture:** 後端新增一個純本地（不需要 GitHub/token）的 `prepare_worktree` 自由函式，在 `RealDispatcher::dispatch` 呼叫 `spawn_and_run` 之前決定實際要用的 cwd；`GitClient`（`vcs/git.rs`）新增 5 個本地 git 操作；`tasks` 表新增 `worktree_path`/`worktree_branch` 兩個欄位；新指令 `tasks_merge_worktree` 做「commit 未提交的變更 → merge 回原分支 → 清理 worktree」。前端只多一顆條件顯示的按鈕。

**Tech Stack:** Rust（`git` subprocess、sqlx/SQLite）、TypeScript/React、Vitest + React Testing Library。

**Spec:** `docs/superpowers/specs/2026-09-09-task-board-worktree-isolation-design.md`

---

## Task 1: 後端 — `tasks` 表新增欄位、`TaskRow`、`set_worktree`/`clear_worktree`

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs`（`init_schema`）
- Modify: `src-tauri/src/tasks/store.rs`（`TaskRow`、新函式、migration 測試）
- Modify: `src-tauri/src/tasks/scheduler.rs`（`mod tests` 裡的 `row()`/`queued_row()` 兩個 `TaskRow` fixture——新增必要欄位後這兩處會編譯失敗，必須跟著補）

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/tasks/store.rs` 的 `mod tests`（`init_schema_migrates_a_database_that_predates_the_label_column` 後面，`store.rs:1408-1452` 那個測試函式結束的 `}` 之後）加：

```rust
/// 舊資料庫沒有 worktree_path/worktree_branch 時，`init_schema` 要能
/// 補上這兩個欄位而不報錯，且既有資料的這兩欄應該是 NULL。
#[tokio::test]
async fn init_schema_migrates_a_database_that_predates_the_worktree_columns() {
    let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();

    // 完整但沒有 worktree_path/worktree_branch 欄位的舊 schema。
    sqlx::query(
        "CREATE TABLE tasks (
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
            label           TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO tasks (id, title, body, project_dir) VALUES ('old1', 't', 'b', '/work/repo')")
        .execute(&pool)
        .await
        .unwrap();

    crate::tasks::init_schema(&pool).await.unwrap();

    let row = get_task(&pool, "old1").await.unwrap().unwrap();
    assert_eq!(row.worktree_path, None, "舊資料遷移後應該是 NULL，不是欄位缺席");
    assert_eq!(row.worktree_branch, None);

    set_worktree(&pool, "old1", "/tmp/wt", "aiterm-task/old1").await.unwrap();
    let row = get_task(&pool, "old1").await.unwrap().unwrap();
    assert_eq!(row.worktree_path.as_deref(), Some("/tmp/wt"));
    assert_eq!(row.worktree_branch.as_deref(), Some("aiterm-task/old1"));

    clear_worktree(&pool, "old1").await.unwrap();
    let row = get_task(&pool, "old1").await.unwrap().unwrap();
    assert_eq!(row.worktree_path, None);
    assert_eq!(row.worktree_branch, None);
}
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test init_schema_migrates_a_database_that_predates_the_worktree_columns`
Expected: 編譯失敗——`worktree_path`/`worktree_branch` 欄位、`set_worktree`/`clear_worktree` 函式都還不存在

- [ ] **Step 3: 加欄位、函式、migration**

`src-tauri/src/tasks/mod.rs` 的 `init_schema`（`CREATE TABLE` 裡 `label TEXT` 後面）：

```rust
            label           TEXT,
            worktree_path   TEXT,
            worktree_branch TEXT
        )",
```

同一個函式，既有的 `ALTER TABLE tasks ADD COLUMN label TEXT` 那行後面加：

```rust
    // Migration: existing databases created before `worktree_path`/
    // `worktree_branch` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_path TEXT")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_branch TEXT")
        .execute(pool)
        .await;
```

`src-tauri/src/tasks/store.rs` 的 `TaskRow` struct（`label: Option<String>,` 欄位後面）：

```rust
    /// 這張卡片專屬 git worktree 的路徑。有值代表派工時偵測到
    /// `project_dir` 是 git repo，Claude Code 實際是在這個路徑跑的，
    /// 不是 `project_dir` 本身。`None` 代表沒有隔離（非 git 專案，或
    /// worktree 建立失敗退回原本行為），或已經合併回原分支並清理過。
    pub worktree_path: Option<String>,
    /// 上面那個 worktree 所在的分支名稱（`aiterm-task/<task_id>`）。
    /// 跟 `worktree_path` 同進退——一個有值另一個必然也有值。
    pub worktree_branch: Option<String>,
```

`store.rs` 新增兩個函式，緊接在 `set_session_path`（`store.rs:469-476`）後面：

```rust
/// 記下派工時建立的 worktree 路徑與分支。在 `spawn_and_run` 之後、
/// 與 `set_tab_id` 同一個時機呼叫，只有偵測到隔離成功時才呼叫。
pub async fn set_worktree(pool: &SqlitePool, id: &str, path: &str, branch: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET worktree_path = ?, worktree_branch = ? WHERE id = ?")
        .bind(path)
        .bind(branch)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 合併成功、worktree 已經被 `git worktree remove` 之後呼叫，把這兩個
/// 欄位清空，讓「合併回原分支」按鈕在前端自然消失。
pub async fn clear_worktree(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET worktree_path = NULL, worktree_branch = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

`src-tauri/src/tasks/scheduler.rs` 的兩個測試 fixture（`mod tests` 裡的 `row()`，`scheduler.rs:485-505` 附近，以及 `queued_row()`，`scheduler.rs:573-596` 附近）各自的 `TaskRow { ... }` 字面量，`label: None,` 後面加：

```rust
            label: None,
            worktree_path: None,
            worktree_branch: None,
```

- [ ] **Step 4: 執行測試確認會過**

Run: `cd src-tauri && cargo test --lib tasks::`
Expected: PASS，含新測試

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs src-tauri/src/tasks/scheduler.rs
git commit -m "feat(taskboard): add worktree_path/worktree_branch columns and store helpers

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 2: 後端 — `GitClient` 新增 5 個本地 git 操作

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/vcs/git.rs` 的 `mod block_info_tests`（`git.rs:1029` 起，已經有 `init_repo` 這個 helper 可以直接重用）最後面加：

```rust
    #[tokio::test]
    async fn create_worktree_checks_out_a_new_branch_at_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("worktree");
        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        client.create_worktree(&wt_path.to_string_lossy(), "aiterm-task/t1").await.unwrap();

        assert!(wt_path.join("a.txt").exists(), "worktree 應該看得到來源分支的檔案");
        let wt_client = GitClient::new(wt_path.to_string_lossy().to_string(), None);
        assert_eq!(wt_client.current_branch().await.unwrap(), "aiterm-task/t1");
    }

    #[tokio::test]
    async fn remove_worktree_removes_a_clean_worktree() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("worktree");
        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        client.create_worktree(&wt_path.to_string_lossy(), "aiterm-task/t2").await.unwrap();

        client.remove_worktree(&wt_path.to_string_lossy()).await.unwrap();
        assert!(!wt_path.exists());
    }

    #[tokio::test]
    async fn has_uncommitted_changes_detects_new_untracked_files() {
        // `quick_block_info` 用的 `diff --shortstat` 抓不到這個——這條測試
        // 就是要證明新方法用的是 `status --porcelain`，兩者不是同一套邏輯。
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        assert!(!client.has_uncommitted_changes().await.unwrap());

        fs::write(dir.path().join("new-untracked.txt"), "new\n").unwrap();
        assert!(client.has_uncommitted_changes().await.unwrap());
    }

    #[tokio::test]
    async fn commit_all_stages_and_commits_untracked_and_modified_files() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        fs::write(dir.path().join("b.txt"), "new file\n").unwrap();

        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        client.commit_all("Task: do something").await.unwrap();

        assert!(!client.has_uncommitted_changes().await.unwrap());
        let log = StdCommand::new("git").args(["log", "-1", "--format=%s"]).current_dir(dir.path()).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "Task: do something");
    }

    #[tokio::test]
    async fn merge_branch_brings_in_the_other_branchs_commits() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("worktree");
        client.create_worktree(&wt_path.to_string_lossy(), "aiterm-task/t3").await.unwrap();
        fs::write(wt_path.join("b.txt"), "from worktree\n").unwrap();
        let wt_client = GitClient::new(wt_path.to_string_lossy().to_string(), None);
        wt_client.commit_all("add b.txt").await.unwrap();

        client.merge_branch("aiterm-task/t3").await.unwrap();
        assert!(dir.path().join("b.txt").exists(), "合併後原分支應該拿到 worktree 分支的檔案");
    }

    #[tokio::test]
    async fn merge_branch_returns_err_on_conflict() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "line1\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir.path()).status().unwrap();

        let client = GitClient::new(dir.path().to_string_lossy().to_string(), None);
        let wt_dir = tempfile::tempdir().unwrap();
        let wt_path = wt_dir.path().join("worktree");
        client.create_worktree(&wt_path.to_string_lossy(), "aiterm-task/t4").await.unwrap();

        // 兩邊都改同一行，製造衝突。
        fs::write(wt_path.join("a.txt"), "line1-from-worktree\n").unwrap();
        let wt_client = GitClient::new(wt_path.to_string_lossy().to_string(), None);
        wt_client.commit_all("conflicting change").await.unwrap();
        fs::write(dir.path().join("a.txt"), "line1-from-base\n").unwrap();
        StdCommand::new("git").args(["commit", "-aqm", "base change"]).current_dir(dir.path()).status().unwrap();

        // 不驗證清理（那是 Task 4 tasks_merge_worktree 的責任，衝突時刻意
        // 不清理 worktree，讓使用者自己處理）——這裡只驗證呼叫端拿到 Err。
        assert!(client.merge_branch("aiterm-task/t4").await.is_err());
    }
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test --lib vcs::git`
Expected: 編譯失敗——`create_worktree`/`remove_worktree`/`has_uncommitted_changes`/`commit_all`/`merge_branch` 都還不存在

- [ ] **Step 3: 寫實作**

在 `src-tauri/src/vcs/git.rs` 的 `push_branch`（`git.rs:253-266`）後面、`// ── GitHub API operations ──` 分隔線（`git.rs:266`）之前插入：

```rust
    /// `git worktree add -b <branch> <path>`——從目前 HEAD 分支出一個新
    /// 分支，同時建立一個獨立的 working directory。不 fetch，完全本地
    /// 操作，不需要遠端或 token。
    pub async fn create_worktree(&self, path: &str, branch_name: &str) -> Result<VcsResult, String> {
        self.git(&[
            "worktree".to_string(), "add".to_string(),
            "-b".to_string(), branch_name.to_string(),
            path.to_string(),
        ])?;
        Ok(VcsResult::WriteSuccess {
            operation: "create_worktree".to_string(),
            detail: format!("Created worktree at '{path}' on branch '{branch_name}'"),
        })
    }

    /// `git worktree remove <path>`——只有在該 worktree 乾淨（沒有未提交
    /// 變更）時才會成功；呼叫端應該在確定所有變更都已經 commit 之後才
    /// 呼叫這個方法。
    pub async fn remove_worktree(&self, path: &str) -> Result<VcsResult, String> {
        self.git(&["worktree".to_string(), "remove".to_string(), path.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "remove_worktree".to_string(),
            detail: format!("Removed worktree at '{path}'"),
        })
    }

    /// `git status --porcelain` 是否非空。跟 `quick_block_info` 用的
    /// `diff --shortstat` 不同——這裡也會抓到新增的未追蹤檔案，判斷
    /// 「這個 worktree 有沒有東西需要 commit」才會準。
    pub async fn has_uncommitted_changes(&self) -> Result<bool, String> {
        let out = self.git(&["status".to_string(), "--porcelain".to_string()])?;
        Ok(!out.trim().is_empty())
    }

    /// `git add -A && git commit -m <message>`——呼叫前應該先用
    /// `has_uncommitted_changes` 確認真的有東西要 commit，避免產生空
    /// commit 噪音（跟 `commit_empty` 刻意允許空 commit 的語意不同，
    /// 這裡不允許——沒有變更時 `git commit` 本身就會失敗，直接回傳 Err）。
    pub async fn commit_all(&self, message: &str) -> Result<VcsResult, String> {
        self.git(&["add".to_string(), "-A".to_string()])?;
        self.git(&["commit".to_string(), "-m".to_string(), message.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "commit_all".to_string(),
            detail: format!("Committed all changes: {message}"),
        })
    }

    /// `git merge <branch>`——在 `self.repo_root` 執行（呼叫端應該傳原本
    /// 的 `project_dir`，不是 worktree 路徑）。衝突或該路徑本身有未提交
    /// 變更擋著都會讓這裡回傳 Err，錯誤訊息直接是 git 自己的輸出。
    pub async fn merge_branch(&self, branch_name: &str) -> Result<VcsResult, String> {
        self.git(&["merge".to_string(), branch_name.to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "merge_branch".to_string(),
            detail: format!("Merged branch '{branch_name}'"),
        })
    }

```

- [ ] **Step 4: 執行測試確認會過**

Run: `cd src-tauri && cargo test --lib vcs::git`
Expected: PASS，含 6 個新測試

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/git.rs
git commit -m "feat(vcs): add worktree create/remove and local commit/merge to GitClient

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 3: 後端 — 派工時建立 worktree（`scheduler.rs`）

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs`

- [ ] **Step 1: 寫失敗的測試**

在 `scheduler.rs` 的 `mod tests`（`scheduler.rs:479` 起）最後面加一個新的測試模組（獨立於既有的 `pick_next`/`order_heads` 測試，因為需要真的 git repo）：

```rust
#[cfg(test)]
mod prepare_worktree_tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn init_repo(dir: &std::path::Path) {
        StdCommand::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["config", "user.name", "Test"]).current_dir(dir).status().unwrap();
        fs::write(dir.join("a.txt"), "hello\n").unwrap();
        StdCommand::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
        StdCommand::new("git").args(["commit", "-q", "-m", "init"]).current_dir(dir).status().unwrap();
    }

    #[tokio::test]
    async fn non_git_project_dir_is_not_isolated() {
        let project_dir = tempfile::tempdir().unwrap();
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t1",
            &project_dir.path().to_string_lossy(),
        ).await;

        assert_eq!(effective_dir, project_dir.path());
        assert!(info.is_none());
    }

    #[tokio::test]
    async fn git_project_dir_gets_an_isolated_worktree() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t2",
            &project_dir.path().to_string_lossy(),
        ).await;

        let (wt_path, wt_branch) = info.expect("git repo 應該要被隔離");
        assert_eq!(effective_dir.to_string_lossy(), wt_path);
        assert_eq!(wt_branch, "aiterm-task/t2");
        assert!(effective_dir.join("a.txt").exists());
        assert_ne!(effective_dir, project_dir.path(), "隔離後不該還是原本的資料夾");
    }

}
```

**已知的測試覆蓋缺口（不在這個 Task 補，理由如下）**：`VcsManager::detect_repo` 對 SVN repo 回傳 `Ok`、不是 `Err`，所以 `prepare_worktree` 的實作必須額外檢查 `info.vcs_type == VcsType::Git`，不能只看「`detect_repo` 有沒有成功」——這條防線本身沒有專屬的自動化測試，因為整個 `vcs/svn.rs` 現有的測試（`svn.rs:372-400` 附近）都只測純字串解析函式（`extract_svn_wc_root`/`extract_svn_url`），從來沒有真的起一個 SVN repo 去跑，這個專案的測試環境不保證裝了 `svn`/`svnadmin`。新增這條防線的測試需要引入這個專案目前完全沒有的「真的建一個 SVN repo」測試基礎設施，超出這個 Task 的範圍——寫實作時只要確實檢查 `vcs_type`（見下面 Step 3 的程式碼），不要因為沒測試就漏掉這個檢查。

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test --lib tasks::scheduler::prepare_worktree_tests`
Expected: 編譯失敗——`prepare_worktree` 還不存在

- [ ] **Step 3: 寫實作**

`scheduler.rs` 檔案開頭的 `use` 區塊（`scheduler.rs:5-16`）加兩行：

```rust
use crate::config::types::VcsType;
use crate::vcs::VcsManager;
```

在 `TaskFinishedEvent` struct（`scheduler.rs:60-68`）前面新增這個自由函式（不是 `RealDispatcher` 的方法——不需要 `AppHandle`/`PtyManager` 等任何欄位，只需要純資料，這樣才能直接用 tempfile git repo 測試，不用組一個完整的 `RealDispatcher`）：

```rust
/// 決定這張卡片實際要用哪個工作目錄，以及要不要建立隔離用的 worktree。
///
/// `project_dir` 是 git repo 就在 `<project_storage_path>/tasks/<task_id>/worktree`
/// 建一個新 worktree + 分支（`aiterm-task/<task_id>`），從目前 HEAD 分支
/// 出去，不 fetch、不需要遠端。回傳的第二個值非 `None` 時，呼叫端要把
/// 這組 (path, branch) 寫進 `store::set_worktree`。
///
/// 不是 git repo（`VcsManager::detect_repo` 回 Err，或偵測到是 SVN——
/// `detect_repo` 對 SVN repo 回傳 `Ok`，不是 Err，所以要額外檢查
/// `vcs_type`），或 `git worktree add` 本身失敗（磁碟空間不足、舊版 git
/// 沒有 worktree 支援等），一律退回直接用 `project_dir`，不擋派工。
async fn prepare_worktree(
    project_storage_path: &std::path::Path,
    task_id: &str,
    task_project_dir: &str,
) -> (std::path::PathBuf, Option<(String, String)>) {
    let fallback = std::path::PathBuf::from(task_project_dir);

    match VcsManager::detect_repo(task_project_dir).await {
        Ok(info) if info.vcs_type == VcsType::Git => {}
        _ => return (fallback, None),
    }

    let worktree_path = crate::tasks::task_dir(project_storage_path, task_id).join("worktree");
    let branch_name = format!("aiterm-task/{task_id}");
    let client = crate::vcs::git::GitClient::new(task_project_dir.to_string(), None);
    match client.create_worktree(&worktree_path.to_string_lossy(), &branch_name).await {
        Ok(_) => {
            let path_str = worktree_path.to_string_lossy().into_owned();
            (worktree_path, Some((path_str, branch_name)))
        }
        Err(e) => {
            eprintln!("worktree create failed for task {task_id}: {e}");
            (fallback, None)
        }
    }
}
```

`RealDispatcher::dispatch`（`scheduler.rs:92-101` 附近）在 `bridge_env` 算完之後、呼叫 `dispatch::spawn_and_run` 之前插入：

```rust
        let (effective_dir, worktree_info) =
            prepare_worktree(&project.path, &task.id, &task.project_dir).await;
        let effective_dir_str = effective_dir.to_string_lossy().into_owned();
```

`spawn_and_run` 呼叫（`scheduler.rs:102-110` 附近）的 `&task.project_dir` 參數改成 `&effective_dir_str`：

```rust
        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &effective_dir_str,
            &claude_cmd,
            session_id.as_deref(),
            &prompt,
            !task.interactive,
            bridge_env,
        )
        .await?;
```

`store::set_tab_id`（`scheduler.rs:126-128`）後面、既有的 `if let Some(sid) = session_id.as_deref() { ... }` 區塊（`scheduler.rs:129-134`）旁邊加：

```rust
        if let Some((wt_path, wt_branch)) = &worktree_info {
            if let Err(e) = store::set_worktree(&project.pool, &task.id, wt_path, wt_branch).await {
                // 記不起來只代表「合併回原分支」按鈕不會出現，worktree
                // 本身已經建好、Claude Code 照樣在裡面跑——不值得讓派工失敗。
                eprintln!("set_worktree {}: {e}", task.id);
            }
        }
```

原本 `let work_dir = std::path::PathBuf::from(&task.project_dir);`（`scheduler.rs:150` 附近，在 `task_title`/`project_name` clone 那幾行旁邊）改成：

```rust
        let work_dir = effective_dir;
```

（`effective_dir` 這個 `PathBuf` 在它算出來、被用來組 `effective_dir_str` 之後就沒有再被借用過，這裡直接 move 進 `work_dir`，不需要 `.clone()`。）

- [ ] **Step 4: 執行測試確認會過**

Run: `cd src-tauri && cargo test --lib tasks::scheduler`
Expected: PASS，含 2 個新測試，既有的 `pick_next`/`order_heads` 測試不受影響

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/scheduler.rs
git commit -m "feat(taskboard): isolate dispatch into a git worktree when project_dir is a git repo

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 4: 後端 — `tasks_merge_worktree` 指令

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/src/lib.rs`（註冊新指令）

**重要前提**：這個專案的 `tauri` 依賴沒有開 `test` feature（`Cargo.toml` 是 `features = []`），`tauri::test::mock_app`／`State<'_, T>` 相關的測試工具**不能用**——`src-tauri/src/tasks/dispatch.rs:947-957` 那段既有註解記錄了確切的編譯錯誤（`error[E0432]: ... the item is gated here`）。這個檔案自己既有的 `save_transcript_tests`（`commands/tasks.rs:598-637`）已經示範了繞過方式：**不呼叫 `#[tauri::command]` 包出來的函式本身，直接照抄它 body 裡那串呼叫順序去測**。新測試照同一個模式寫，不嘗試組 `State<'_, ProjectRegistry>` 或 `AppHandle`。

- [ ] **Step 1: 寫失敗的測試**

在 `src-tauri/src/commands/tasks.rs` 檔案最後加一個新的測試模組：

```rust
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
}
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `cd src-tauri && cargo test --lib commands::tasks::merge_worktree_tests`
Expected: 編譯失敗——`GitClient`（還沒 import）、`store::set_worktree`/`clear_worktree`（Task 1 應該已經存在）——若 Task 1/2 已經做完，這裡唯一缺的是 `commands/tasks.rs` 還沒 `use crate::vcs::git::GitClient;`

- [ ] **Step 3: 寫實作**

`src-tauri/src/commands/tasks.rs` 檔案開頭加一行 import（跟既有的 `use crate::tasks::store::{self, AttachmentRow, TaskRow};` 放在一起）：

```rust
use crate::vcs::git::GitClient;
```

在 `tasks_mark_done`（`commands/tasks.rs:210-233` 附近）後面加：

```rust
#[tauri::command]
pub async fn tasks_merge_worktree(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<(), String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let worktree_path = row.worktree_path.ok_or_else(|| "this card has no worktree to merge".to_string())?;
    let worktree_branch = row.worktree_branch.ok_or_else(|| "this card has no worktree to merge".to_string())?;

    let worktree_client = GitClient::new(worktree_path.clone(), None);
    if worktree_client.has_uncommitted_changes().await? {
        worktree_client.commit_all(&format!("Task: {}", row.title)).await?;
    }

    let base_client = GitClient::new(row.project_dir.clone(), None);
    base_client.merge_branch(&worktree_branch).await?;

    // 合併成功才清理——失敗的話（通常是衝突）worktree/分支原樣保留，
    // 讓使用者自己進那個分頁處理再重按一次。
    base_client.remove_worktree(&worktree_path).await?;
    store::clear_worktree(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(())
}
```

- [ ] **Step 4: 註冊指令**

`src-tauri/src/lib.rs` 的 `tasks::{...}` import 區塊（`lib.rs:112-117`）加一個名字：

```rust
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_clone, tasks_read_transcript,
        tasks_save_transcript, tasks_mark_done, tasks_used_dirs, tasks_used_labels,
        tasks_set_summary, tasks_set_label, tasks_archive, tasks_unarchive,
        tasks_archive_done, tasks_list_archived, tasks_merge_worktree,
    },
```

`invoke_handler![...]` 列表（`lib.rs:590` 起，`tasks_mark_done,` 那行後面）加：

```rust
            tasks_mark_done,
            tasks_merge_worktree,
```

- [ ] **Step 5: 執行測試確認會過**

Run: `cd src-tauri && cargo test --lib commands::tasks`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs
git commit -m "feat(taskboard): add tasks_merge_worktree command

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 5: 後端完整驗證

**Files:** 無新增/修改——純驗證步驟，在繼續前端之前先確認整個後端沒問題。

- [ ] **Step 1: 完整 `cargo test`（不是 `--lib`）**

Run: `cd src-tauri && cargo test`
Expected: 全綠，包含既有的所有測試（`vcs::git`、`tasks::*`、`commands::tasks`，以及跟這次改動完全無關的其他模組）

- [ ] **Step 2: 若失敗，先修好再繼續**

不往下做前端任務，直到這一步全綠。

---

## Task 6: 前端 — `TaskRow` 型別、`mergeTaskWorktree`、既有測試 fixture 修好

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/components/TaskBoard/ReportDialog.test.tsx`
- Modify: `src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx`
- Modify: `src/components/TaskBoard/groupByLabel.test.ts`
- Modify: `src/components/TaskBoard/reportPrompts.test.ts`

**注意**：`src/components/TaskBoard/index.test.tsx` 的 `card()` fixture 跟 `mergeTaskWorktree` mock 留到 Task 7（那裡要測按鈕行為，跟這個 fixture 綁在一起改比較不會漏東西）。

- [ ] **Step 1: 補 TS 型別與新 IPC 函式**

`src/ipc/tasks.ts` 的 `TaskRow` interface（`label: string | null;` 後面）：

```ts
  label: string | null;
  /** 這張卡片專屬 git worktree 的路徑；`null` 代表沒有隔離或已經合併清理過。 */
  worktree_path: string | null;
  /** 上面那個 worktree 所在的分支名稱，跟 `worktree_path` 同進退。 */
  worktree_branch: string | null;
```

在 `markTaskDone`（`tasks.ts:111-112`）後面加：

```ts
export const mergeTaskWorktree = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_merge_worktree", { projectId, id });
```

- [ ] **Step 2: 執行型別檢查確認會紅**

Run: `npx tsc -b`
Expected: 錯誤——四個測試檔案的 `TaskWithAttachments` fixture 缺 `worktree_path`/`worktree_branch`

- [ ] **Step 3: 補齊四個測試檔案的 fixture**

`src/components/TaskBoard/ReportDialog.test.tsx`（`taskCard` 裡 `use_bridge: false, bridge_tiers: null, label: null,` 那行）：

```ts
    use_bridge: false, bridge_tiers: null, label: null,
    worktree_path: null, worktree_branch: null,
```

`src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx`（`BASE_CARD` 裡 `label: null,` 後面）：

```ts
  label: null,
  worktree_path: null,
  worktree_branch: null,
```

`src/components/TaskBoard/groupByLabel.test.ts`（`card()` 裡 `label: null, attachments: [],` 那行）：

```ts
  label: null, worktree_path: null, worktree_branch: null, attachments: [],
```

`src/components/TaskBoard/reportPrompts.test.ts`（`card()` 裡 `label: null,` 後面）：

```ts
  label: null,
  worktree_path: null,
  worktree_branch: null,
```

- [ ] **Step 4: 執行型別檢查與既有測試確認會過**

Run: `npx tsc -b && npm run test -- --run ReportDialog TaskEditorDialog.bridge groupByLabel reportPrompts`
Expected: 型別檢查通過；四個測試檔案全綠（跟改動前行為完全相同，這步只是修型別）

- [ ] **Step 5: Commit**

```bash
git add src/ipc/tasks.ts src/components/TaskBoard/ReportDialog.test.tsx src/components/TaskBoard/TaskEditorDialog.bridge.test.tsx src/components/TaskBoard/groupByLabel.test.ts src/components/TaskBoard/reportPrompts.test.ts
git commit -m "chore(taskboard): add worktree fields to TaskRow TS type and test fixtures

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 7: 前端 — 「合併回原分支」按鈕

**Files:**
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Modify: `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

`src/components/TaskBoard/index.test.tsx` 的 `vi.mock("../../ipc/tasks", ...)`（`index.test.tsx:11-30`）加一行：

```ts
  markTaskDone: vi.fn().mockResolvedValue(undefined),
  mergeTaskWorktree: vi.fn().mockResolvedValue(undefined),
```

import 那行（`index.test.tsx:48`）加 `mergeTaskWorktree`：

```ts
import { listTasks, onTasksUpdated, moveTask, archiveTask, archiveDoneTasks, listArchivedTasks, unarchiveTask, setTaskLabel, mergeTaskWorktree } from "../../ipc/tasks";
```

`card()` fixture（`index.test.tsx:59-67`）的 `label: null, attachments: [],` 那行：

```ts
  label: null, worktree_path: null, worktree_branch: null, attachments: [],
```

在 `describe("ProjectBoard", ...)` 區塊裡、既有的 "done card re-dispatch calls cloneTask"（`index.test.tsx:662-669` 附近）旁邊加兩個新測試：

```ts
  it("done card with a worktree shows a merge button", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "DoneCard", status: "done", outcome: "success", worktree_branch: "aiterm-task/d" }),
    ]);
    view();
    const done = await screen.findByTestId("column-done");
    expect(within(done).getByText(/合併回原分支|Merge back/)).toBeInTheDocument();
  });

  it("done card without a worktree does not show a merge button", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "DoneCard", status: "done", outcome: "success", worktree_branch: null }),
    ]);
    view();
    const done = await screen.findByTestId("column-done");
    expect(within(done).queryByText(/合併回原分支|Merge back/)).not.toBeInTheDocument();
  });

  it("clicking the merge button calls mergeTaskWorktree", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "d", title: "DoneCard", status: "done", outcome: "success", worktree_branch: "aiterm-task/d" }),
    ]);
    view();
    const done = await screen.findByTestId("column-done");
    await userEvent.click(within(done).getByText(/合併回原分支|Merge back/));
    await waitFor(() => expect(mergeTaskWorktree).toHaveBeenCalledWith(PROJECT_ID, "d"));
  });
```

- [ ] **Step 2: 執行測試確認會紅**

Run: `npm run test -- --run TaskBoard/index`
Expected: FAIL——三個新測試找不到「合併回原分支」文字（按鈕還不存在）

- [ ] **Step 3: 寫實作**

`src/components/TaskBoard/TaskCard.tsx` 的 import（`TaskCard.tsx:5`）加 `mergeTaskWorktree`：

```tsx
import { archiveTask, cloneTask, deleteTask, markTaskDone, mergeTaskWorktree, stopTask, type TaskWithAttachments } from "../../ipc/tasks";
```

`status === "done"` 動作列（`TaskCard.tsx:138-150`）的 `requeue` 按鈕後面加：

```tsx
            <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={() => void run(() => cloneTask(projectId, card.id))}>
              {t.board_action_requeue}
            </button>
            {card.worktree_branch && (
              <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void run(() => mergeTaskWorktree(projectId, card.id))}>
                {t.board_action_merge_worktree}
              </button>
            )}
```

- [ ] **Step 4: 執行測試確認會過**

Run: `npx tsc -b && npm run test -- --run TaskBoard/index`
Expected: 型別檢查通過；但**測試會紅**——`t.board_action_merge_worktree` 這個 i18n key 還不存在，按鈕文字是 `undefined`，`getByText(/合併回原分支|Merge back/)` 找不到匹配。這是預期中的紅燈，Task 8 補上 i18n 後才會轉綠，先繼續往下不要在這一步卡住。

- [ ] **Step 5: 先不 commit**——這個 Task 的測試要等 Task 8 補齊 i18n key 才會真的綠燈，兩個 Task 的變動一起在 Task 8 最後 commit。

---

## Task 8: i18n — 一個新 key（en / zh-TW）

**Files:**
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: zh-TW 區塊加 key**

`src/lib/i18n.ts:64`（`board_action_requeue: "重新派工",` 後面）加：

```ts
    board_action_requeue: "重新派工",
    board_action_merge_worktree: "合併回原分支",
```

- [ ] **Step 2: en 區塊加對應 key**

`src/lib/i18n.ts:1576`（`board_action_requeue: "Re-dispatch",` 後面）加：

```ts
    board_action_requeue: "Re-dispatch",
    board_action_merge_worktree: "Merge back to original branch",
```

- [ ] **Step 3: 執行測試確認會過**

Run: `npx tsc -b && npm run test -- --run TaskBoard/index`
Expected: PASS——Task 7 用 regex `/合併回原分支|Merge back/` 匹配文字的三個測試，現在兩個語系都有真正的翻譯文字可以匹配

- [ ] **Step 4: Commit（涵蓋 Task 7 + 8）**

```bash
git add src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/index.test.tsx src/lib/i18n.ts
git commit -m "feat(taskboard): add merge-back-to-original-branch button for isolated cards

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012Cbm9KPo2DAenqgoCk4hYH"
```

---

## Task 9: 全專案驗證

**Files:** 無新增/修改——純驗證步驟。

- [ ] **Step 1: 後端完整測試**

Run: `cd src-tauri && cargo test`
Expected: 全綠

- [ ] **Step 2: 前端完整測試**

Run: `npm run test`
Expected: 全綠

- [ ] **Step 3: 型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤

- [ ] **Step 4: Lint**

Run: `npm run lint`
Expected: 這次改動的檔案無新增錯誤（比照本次分支基準：改動前就存在的、跟本次改動無關的既有 lint 錯誤不算數，只要確認新增/修改的檔案本身乾淨）

- [ ] **Step 5: 手動驗證的已知限制**

跟後端 git 操作有關的功能，單元測試已經覆蓋「worktree 建立/移除/commit/merge」的核心邏輯，但沒有覆蓋「真的透過工作看板 UI 派一張卡、看到它在隔離的 worktree 裡跑、按下合併按鈕」這條完整路徑。這步驟建議：
  1. 在一個測試用的本機 git repo 裡建一張工作看板卡片並派工，確認開出來的終端機分頁 cwd 是 `<AITerm 專案資料夾>/tasks/<task_id>/worktree`，且 `git branch` 顯示在 `aiterm-task/<task_id>` 上。
  2. 卡片跑完後，看板「已完成」欄該卡片上出現「合併回原分支」；按下去後回到原本的 `project_dir` 用 `git log` 確認變更真的進來了，且 worktree 目錄消失。
  這一步不阻塞完成，但强烈建議在合併進 master 前至少手動跑過一次，因為這是這次改動裡沒有自動化測試覆蓋、涉及真實檔案系統 + 真實終端機分頁互動的部分。

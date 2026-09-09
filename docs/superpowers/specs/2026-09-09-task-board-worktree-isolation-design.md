# 工作看板派工 Git Worktree 隔離（第 1 階段）— 設計

日期：2026-09-09
狀態：待使用者複審

## 問題

工作看板派工卡片跑完全直接在 `task.project_dir` 這個資料夾裡執行（`src-tauri/src/tasks/dispatch.rs:340-360` 的 `spawn_and_run`）——沒有任何隔離。`pick_next`/`drain_once`（`scheduler.rs`）允許 `parallel_ok=true` 的多張卡片同時進入 `running`，代表兩個同時在跑的 Claude Code Agent 可能同時改動同一份 working tree，除非使用者自己手動把會衝突的卡片設成 `parallel_ok=false`。這不是功能缺口，是併發安全性的洞。

## 現況調查（讀過程式碼確認，非推測）

- `dispatch.rs:340-360`：`spawn_and_run` 的 cwd 直接是傳進來的 `project_dir` 字串，`scheduler.rs` 裡 `RealDispatcher::dispatch` 呼叫時傳的就是 `&task.project_dir`，原封不動。
- 整個 `src-tauri/` 沒有任何地方用到 `git worktree`（`grep -rn "worktree" src-tauri/src src/` 完全沒有非測試的命中）。
- AITerm 已經有一套完整的 VCS 團隊協作功能（`src/components/VcsView/`、`src-tauri/src/vcs/git.rs` 的 `GitClient`），但那套是**原地換分支**（`create_branch`/`checkout_branch` 都在 `self.repo_root` 本身操作，`git.rs:154-224`），而且整個建立在「這個資料夾有設定過 GitHub connection」之上（`vcs_start_feature` 會 `fetch_ref`、`push_branch`、`create_pr`，都要 token）。這套機制不能直接重用在工作看板上——工作看板的專案資料夾完全不要求是 git repo，更不要求有 GitHub connection。
- `vcs_detect_repo`（`commands/vcs.rs:338-357`）已經把「是不是 git repo」跟「比對 GitHub connection」分成兩步——`VcsManager::detect_repo(path)`（`vcs/mod.rs:19-52`）只做 `git rev-parse --show-toplevel`，完全不需要 remote、不需要 token，這一步可以直接被工作看板排程器呼叫。
- `task_dir(project_path, task_id)`（`tasks/mod.rs:31-33`）= `<AITerm 專案資料夾>/tasks/<task_id>`——已經是每張卡片專屬、保證不在 `task.project_dir`（使用者實際要動的 code repo）裡面的資料夾，目前用來放 `transcript.txt`。
- `scheduler.rs:149-186`：`work_dir`（目前 = `task.project_dir`）會被傳進 `persist_outcome`，用來定位 Claude Code 自己寫的 session log（`session_log::claude_projects_root()` 靠 encode 這個 cwd 路徑去找 `~/.claude/projects/<encoded>/`）。這代表**實際 spawn 的 cwd 換到哪裡，`work_dir` 就必須跟著換到哪裡**，兩邊沒對齊，完整對話記錄那個功能就會找錯資料夾（會 fallback 回原始 PTY 擷取，不會整個掛掉，但會退化）。
- `GitClient`（`vcs/git.rs:13-25`）的所有本地操作（`create_branch`/`commit_empty`/`push_branch` 等）都用 `Command::new("git").current_dir(&self.repo_root)`——`self.repo_root` 可以是任何路徑，不限定是 repo 的頂層目錄（worktree 目錄本身也是一個完整可用的 working directory）。
- `quick_block_info`（`git.rs:602-617`）只用 `git diff --shortstat`，只涵蓋已追蹤檔案的**未 staged** 改動，抓不到新增的未追蹤檔案——不能拿來判斷「這個 worktree 有沒有東西需要 commit」。
- `TaskRow`（`store.rs:15-51`）跟 `tasks` 表的 schema/migration 都在 `tasks/mod.rs:35-98`：新欄位的標準做法是 `CREATE TABLE` 裡加、再补一行 `ALTER TABLE tasks ADD COLUMN ...`（`let _ =` 吞掉「欄位已存在」的失敗，注解都寫得很清楚，照抄即可）。
- `TaskCard.tsx:138-150`：`status === "done"` 的卡片動作列已經有 `requeue`/`archive`/`transcript` 幾顆按鈕，統一用 `run(() => someIpcCall(...))` 包 busy 狀態，**沒有**任何 inline 錯誤顯示（`run` 本身不 catch，錯誤會變成 unhandled rejection）——這是既有模式，新按鈕原樣沿用，不新增錯誤 UI。

## 範圍（brainstorming 已確認的決定）

1. **只做第 1 階段**：純本地 git worktree 隔離 + 手動「合併回原分支」按鈕。不做 GitHub/PR，那是留給下一輪的第 2 階段（接 `FinishFeatureReview` 的 diff/PR review）。
2. **啟動時機**：`task.project_dir` 是 git repo 就自動開 worktree；不是 git repo（或 `git worktree add` 本身失敗）就完全比照現行行為，直接在 `project_dir` 跑，不擋派工、不需要任何新設定開關。
3. **完成後不自動合併**：卡片跑完後改動停留在獨立分支/worktree 裡，看板上多一顆「合併回原分支」按鈕，使用者自己按。
4. **不做捨棄／自動清理**：卡片刪除、封存都不會動到 worktree；不合併就一直留著，使用者自己用終端機清。

## 架構

### 派工時建立 worktree（`scheduler.rs` + `dispatch.rs`）

`RealDispatcher::dispatch` 在組好 `prompt` 之後、呼叫 `dispatch::spawn_and_run`之前，新增一段：

```rust
let effective_dir = self.prepare_worktree(project, task).await;
// effective_dir: PathBuf，隔離成功就是 worktree 路徑，
// 不是 git repo 或建立失敗就原樣是 task.project_dir。
```

`prepare_worktree` 的邏輯（新的私有方法，放在 `scheduler.rs`）：

```rust
async fn prepare_worktree(&self, project: &ProjectHandle, task: &TaskRow) -> (PathBuf, Option<(String, String)>) {
    // 回傳 (實際要用的 cwd, Option<(worktree_path, branch_name)>)——
    // 第二個值 None 代表沒有隔離，Some 才要寫進 DB。
    if crate::vcs::VcsManager::detect_repo(&task.project_dir).await.is_err() {
        return (PathBuf::from(&task.project_dir), None);
    }
    let worktree_path = crate::tasks::task_dir(&project.path, &task.id).join("worktree");
    let branch_name = format!("aiterm-task/{}", task.id);
    let client = crate::vcs::git::GitClient::new(task.project_dir.clone(), None);
    match client.create_worktree(&worktree_path.to_string_lossy(), &branch_name).await {
        Ok(_) => (worktree_path.clone(), Some((worktree_path.to_string_lossy().into_owned(), branch_name))),
        Err(e) => {
            // 建立失敗（舊版 git 沒有 worktree 支援、磁碟空間不足等）——
            // 不擋派工，退回原本直接在 project_dir 跑的行為。
            eprintln!("worktree create failed for task {}: {e}", task.id);
            (PathBuf::from(&task.project_dir), None)
        }
    }
}
```

`detect_repo` 是既有的 `VcsManager::detect_repo`（`vcs/mod.rs`），不需要 connection、不需要 token，純本地 `git rev-parse`。

`create_worktree`/`create_branch` 沒有 `fetch_ref`——直接從目前 `project_dir` 的 HEAD 分支出去，完全離線可用（跟 `vcs_start_feature` 那套需要先 `fetch origin/<base>` 不一樣，那是為了跟遠端同步，這裡沒有遠端依賴）。

`dispatch` 呼叫 `spawn_and_run` 時，cwd 參數改傳 `effective_dir`；後面 spawn 到 async block 裡的 `work_dir`（目前是 `std::path::PathBuf::from(&task.project_dir)`，`scheduler.rs:150`）也要改成同一個 `effective_dir`，維持「spawn 用哪個路徑、transcript 查找就用哪個路徑」的一致性。

`Some((worktree_path, branch_name))` 的話，在 `set_tab_id`（`scheduler.rs:126-128` 附近同一個位置）一起呼叫新的 `store::set_worktree(&project.pool, &task.id, &worktree_path, &branch_name)`。

### 資料庫（`tasks/mod.rs` + `store.rs`）

`tasks` 表新增兩個欄位，比照現有欄位的 `CREATE TABLE` + `ALTER TABLE` 雙寫模式：

```rust
// CREATE TABLE 裡加在 label 後面：
worktree_path   TEXT,
worktree_branch TEXT

// migration 區塊加在既有幾行 ALTER TABLE 後面：
let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_path TEXT")
    .execute(pool)
    .await;
let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN worktree_branch TEXT")
    .execute(pool)
    .await;
```

`TaskRow`（`store.rs:15-51`）加：

```rust
pub worktree_path: Option<String>,
pub worktree_branch: Option<String>,
```

`store.rs` 新增兩個函式，比照 `set_session_id`/`set_session_path`（`store.rs:458-472`）的寫法：

```rust
pub async fn set_worktree(pool: &SqlitePool, id: &str, path: &str, branch: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET worktree_path = ?, worktree_branch = ? WHERE id = ?")
        .bind(path).bind(branch).bind(id)
        .execute(pool).await?;
    Ok(())
}

pub async fn clear_worktree(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET worktree_path = NULL, worktree_branch = NULL WHERE id = ?")
        .bind(id)
        .execute(pool).await?;
    Ok(())
}
```

`TaskWithAttachments`／前端 `TaskRow`（`src/ipc/tasks.ts:16-51`）比照後端加 `worktree_path: string | null`、`worktree_branch: string | null`。

### `GitClient` 新增 5 個純本地操作（`vcs/git.rs`）

放在既有 `push_branch`（`git.rs:253-266`）跟「── GitHub API operations ──」分隔線之間，全部不需要 `token`，風格比照現有方法：

```rust
/// `git worktree add -b <branch> <path>`——從目前 HEAD 分支出一個新分支，
/// 同時建立一個獨立的 working directory。不 fetch，完全本地操作。
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

/// `git worktree remove <path>`——只有在該 worktree 乾淨（沒有未提交變更）
/// 時才會成功；呼叫端應該在確定所有變更都已經 commit 之後才呼叫。
pub async fn remove_worktree(&self, path: &str) -> Result<VcsResult, String> {
    self.git(&["worktree".to_string(), "remove".to_string(), path.to_string()])?;
    Ok(VcsResult::WriteSuccess {
        operation: "remove_worktree".to_string(),
        detail: format!("Removed worktree at '{path}'"),
    })
}

/// `git status --porcelain` 是否非空。跟 `quick_block_info` 用的
/// `diff --shortstat` 不同——這裡也會抓到新增的未追蹤檔案，
/// 判斷「這個 worktree 有沒有東西需要 commit」才會準。
pub async fn has_uncommitted_changes(&self) -> Result<bool, String> {
    let out = self.git(&["status".to_string(), "--porcelain".to_string()])?;
    Ok(!out.trim().is_empty())
}

/// `git add -A && git commit -m <message>`——呼叫前應該先用
/// `has_uncommitted_changes` 確認真的有東西要 commit，避免產生空 commit
/// 噪音（跟 `commit_empty` 刻意允許空 commit 的語意不同，這裡不允許）。
pub async fn commit_all(&self, message: &str) -> Result<VcsResult, String> {
    self.git(&["add".to_string(), "-A".to_string()])?;
    self.git(&["commit".to_string(), "-m".to_string(), message.to_string()])?;
    Ok(VcsResult::WriteSuccess {
        operation: "commit_all".to_string(),
        detail: format!("Committed all changes: {message}"),
    })
}

/// `git merge <branch>`——在 `self.repo_root`（呼叫端應該傳原本的
/// `project_dir`，不是 worktree 路徑）執行。衝突或該路徑本身有未提交
/// 變更擋著都會讓這裡回傳 Err，錯誤訊息直接是 git 自己的輸出。
pub async fn merge_branch(&self, branch_name: &str) -> Result<VcsResult, String> {
    self.git(&["merge".to_string(), branch_name.to_string()])?;
    Ok(VcsResult::WriteSuccess {
        operation: "merge_branch".to_string(),
        detail: format!("Merged branch '{branch_name}'"),
    })
}
```

### 「合併回原分支」指令（`commands/tasks.rs`）

新增 `tasks_merge_worktree`，跟既有的 `tasks_stop`/`tasks_mark_done`（`commands/tasks.rs:181-230`）放在一起，拿 `project`/`task` 的方式完全比照那兩個既有指令（`project(&reg, &project_id)` 這個既有 helper + `store::get_task`）：

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

    // 合併成功才清理——失敗的話 worktree/分支原樣保留，讓使用者自己
    // 進那個分頁處理（通常是衝突，或 project_dir 本身有未提交變更擋著）。
    base_client.remove_worktree(&worktree_path).await?;
    store::clear_worktree(&p.pool, &id).await.map_err(|e| e.to_string())?;
    let _ = app.emit("tasks-updated", ());
    Ok(())
}
```

### 前端

`src/ipc/tasks.ts` 加，比照緊鄰的 `stopTask`/`markTaskDone`（`tasks.ts:108-112`）的參數命名：

```ts
export const mergeTaskWorktree = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_merge_worktree", { projectId, id });
```

`TaskCard.tsx` 在 `status === "done"` 的動作列（`TaskCard.tsx:138-150`）比照 `requeue`/`archive` 的既有寫法加一顆：

```tsx
{card.worktree_branch && (
  <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void run(() => mergeTaskWorktree(projectId, card.id))}>
    {t.board_action_merge_worktree}
  </button>
)}
```

沒有額外的錯誤顯示——沿用 `run()` 的既有行為（失敗變成 unhandled rejection，跟 `stop`/`archive`/`requeue` 目前一致），不為這顆按鈕新增錯誤 UI。

`i18n.ts` 加一個 key（en/zh-TW 各一份）：`board_action_merge_worktree`（「合併回原分支」／"Merge back to original branch"）。

## 明確不做的部分

- 不建立 PR、不需要 GitHub connection、不 push 到遠端——整個功能純本地 git 操作。
- 不做「捨棄變更」按鈕——不合併的話 worktree 就留著，使用者自己用終端機或 `git worktree remove --force` 清；這是留給未來視情況再加的東西，不是這次的一部分。
- 卡片刪除／封存都**不**觸碰 worktree——避免在使用者還沒看過改動前就把東西弄丟。
- 不處理合併衝突——衝突就是 `merge_branch` 回傳 Err，直接顯示 git 的錯誤訊息（透過既有 `run()` 的 unhandled-rejection 行為），worktree 原封不動留著讓使用者自己去那個分頁解決。
- 不新增設定開關——行為完全由 `task.project_dir` 是不是 git repo 決定，沒有「要不要隔離」這個選項。
- 不影響非 git 專案或現有的直接派工行為。
- 不做第 2 階段（`FinishFeatureReview` 的 diff view / PR review）——那是下一輪 spec。

## 測試

- **Rust**：
  - `GitClient` 5 個新方法各自的單元測試，比照 `vcs/git.rs` 既有測試的模式（用 `tempfile` 建一個真的 git repo）：`create_worktree` 建出來的路徑真的是一個可用的 working directory 且在正確的分支上；`remove_worktree` 對乾淨的 worktree 成功、對有未提交變更的 worktree 失敗；`has_uncommitted_changes` 對新增的未追蹤檔案回 `true`（用來驗證不是重用 `quick_block_info` 那套只看 tracked diff 的邏輯）；`commit_all` 真的把新增與修改的檔案都收進同一個 commit；`merge_branch` 成功合併時目標分支真的拿到來源分支的內容，衝突時回傳 `Err` 且不留下半套合併狀態（`git merge --abort` 或確認 `MERGE_HEAD` 不存在）。
  - `scheduler.rs` 的 `prepare_worktree`：對非 git repo 的 `project_dir` 回傳 `(project_dir, None)`；對 git repo 成功建立時回傳 worktree 路徑跟正確命名的分支（`aiterm-task/<task_id>`）；`create_worktree` 失敗時（例如目標路徑已存在非空目錄）優雅退回 `(project_dir, None)`，不 panic、不讓整個 dispatch 失敗。
  - `store.rs` 的 `set_worktree`/`clear_worktree`：round-trip 測試，比照既有 `set_session_id`/`set_session_path` 的測試模式；`clear_worktree` 只清 `worktree_path`/`worktree_branch`，不動其他欄位。
  - `tasks/mod.rs` 的 migration：比照既有 `init_schema_migrates_a_database_that_predates_the_label_column` 的模式，補一個「舊資料庫沒有 `worktree_path`/`worktree_branch` 時 `init_schema` 不報錯、補上這兩個欄位」的測試。
  - `task_merge_worktree`：沒有 worktree 的卡片呼叫回傳 `Err`；worktree 有未提交變更時，合併前會先 commit（用 `git log` 驗證合併後的 base 分支真的拿到那些檔案）；合併成功後 worktree 目錄真的被移除、DB 裡的兩個欄位被清空；合併衝突時 worktree 不被移除、DB 欄位保留原值。
- **前端**：`TaskCard.tsx` 新增測試，比照既有 `requeue`/`archive` 按鈕的測試模式：`worktree_branch` 有值時顯示合併按鈕、按下去呼叫 `mergeTaskWorktree(projectId, card.id)`；`worktree_branch` 是 `null` 時不顯示這顆按鈕。

# 合併回原分支：衝突處理與前置檢查 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 「合併回原分支」不再靜靜失敗或把原專案目錄留在半合併狀態；衝突時由使用者當下決定要就地解還是還原。

**Architecture:** 後端 `tasks_merge_worktree` 改成回傳結構化結果（`merged` /
`conflict` / `blocked`），前端依結果決定要不要跳選擇視窗；選「還原」時呼叫新的
`tasks_abort_merge`。同步的 `git()` 移進 `spawn_blocking`。

**Tech Stack:** Rust（Tauri command + GitClient）、React + TypeScript、
`@tauri-apps/plugin-dialog` 的 `confirm`/`message`、Vitest。

**使用者已定的決策：**

- 衝突時**當下問**：跳確認視窗讓使用者選「自己解」或「還原」。
- 這一輪四項全做：衝突處理、合併前檢查原分支乾淨、按鈕進度回饋、`spawn_blocking`。

**已驗證的前提：**

- `@tauri-apps/plugin-dialog` 的 `ConfirmDialogOptions` 支援 `okLabel` /
  `cancelLabel` / `kind`（`dist-js/index.d.ts:231-239`）。
- 全庫沒有任何 `merge --abort` 或 `MERGE_HEAD` 偵測。
- `git()` 是同步的 `std::process::Command`（`vcs/git.rs:3,684`），
  有 22 個 `self.git(` 呼叫點，另有 `diff_tree_files` / `parse_remote` /
  `require_github` 三個同步輔助函式會連帶變成 async。漏掉 `.await` 一律是
  編譯錯誤，所以這個重構由編譯器全程把關。
- 錯誤顯示（`TaskCard` 的 `run()`）已經在 commit `5b11d4da` 修好了，這份
  計畫建立在那個基礎上。

---

## File Structure

| 檔案 | 這輪的責任 |
|------|-----------|
| `src-tauri/src/vcs/git.rs` | 新增 `is_merge_in_progress` / `conflicted_files` / `merge_abort`；`git()` 改 async + spawn_blocking |
| `src-tauri/src/commands/tasks.rs` | `tasks_merge_worktree` 改回傳 `MergeOutcome`；新增 `tasks_abort_merge` |
| `src-tauri/src/lib.rs` | 註冊 `tasks_abort_merge` |
| `src/ipc/tasks.ts` | `MergeOutcome` 型別、`abortMerge` |
| `src/components/TaskBoard/TaskCard.tsx` | 依結果跳視窗、按鈕進度文字 |
| `src/lib/i18n.ts` | 中英文案 |

---

### Task 1: GitClient 的三個衝突相關查詢

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`（接在 `merge_branch` 之後，第 322 行）

- [ ] **Step 1: 寫會紅的測試**

加在 `git.rs` 既有測試模組（若該模組不存在就建一個 `#[cfg(test)] mod merge_state_tests`）：

```rust
#[cfg(test)]
mod merge_state_tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn run(dir: &std::path::Path, args: &[&str]) {
        StdCommand::new("git").args(args).current_dir(dir).status().unwrap();
    }

    /// 造一個必定衝突的情境：兩個分支改同一行。
    fn repo_with_conflict() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        run(p, &["init", "-q", "-b", "main"]);
        run(p, &["config", "user.email", "t@t.com"]);
        run(p, &["config", "user.name", "T"]);
        std::fs::write(p.join("a.txt"), "base\n").unwrap();
        run(p, &["add", "."]);
        run(p, &["commit", "-q", "-m", "init"]);

        run(p, &["checkout", "-q", "-b", "side"]);
        std::fs::write(p.join("a.txt"), "side\n").unwrap();
        run(p, &["commit", "-qam", "side"]);

        run(p, &["checkout", "-q", "main"]);
        std::fs::write(p.join("a.txt"), "main\n").unwrap();
        run(p, &["commit", "-qam", "main"]);
        d
    }

    #[tokio::test]
    async fn reports_merge_state_and_conflicted_files_then_aborts() {
        let d = repo_with_conflict();
        let c = GitClient::new(d.path().to_string_lossy().to_string(), None);

        // 合併之前：乾淨。
        assert!(!c.is_merge_in_progress().await, "合併前不該說正在合併中");
        assert!(c.conflicted_files().await.unwrap().is_empty());

        // 這次合併一定失敗。
        assert!(c.merge_branch("side").await.is_err(), "同一行的兩邊修改必須衝突");

        // 失敗之後，倉庫確實停在半合併狀態——這正是目前沒人處理的狀態。
        assert!(c.is_merge_in_progress().await, "衝突後應該偵測得到 MERGE_HEAD");
        assert_eq!(c.conflicted_files().await.unwrap(), vec!["a.txt".to_string()]);

        // 還原之後回到乾淨。
        c.merge_abort().await.unwrap();
        assert!(!c.is_merge_in_progress().await, "abort 之後不該還在合併中");
        assert!(c.conflicted_files().await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
cd src-tauri && cargo test --workspace reports_merge_state 2>&1 | grep -E "^error|cannot find" | head -5
```

Expected：`cannot find method is_merge_in_progress`（編譯失敗）。

- [ ] **Step 3: 實作三個方法**

加在 `merge_branch`（`git.rs:322`）之後：

```rust
    /// 這個倉庫是不是停在「合併進行中」（`.git/MERGE_HEAD` 還在）。
    ///
    /// 衝突之後 `git merge` 會把倉庫留在這個狀態直到有人收尾。在這種狀態上
    /// 再跑一次 merge 只會得到 "You have not concluded your merge" 這種
    /// 看不懂的錯誤，所以要先擋下來。
    pub async fn is_merge_in_progress(&self) -> bool {
        self.git(&[
            "rev-parse".to_string(),
            "--verify".to_string(),
            "--quiet".to_string(),
            "MERGE_HEAD".to_string(),
        ])
        .is_ok()
    }

    /// 目前處於未解衝突狀態的檔案（`git diff --diff-filter=U`）。
    pub async fn conflicted_files(&self) -> Result<Vec<String>, String> {
        let out = self.git(&[
            "diff".to_string(),
            "--name-only".to_string(),
            "--diff-filter=U".to_string(),
        ])?;
        Ok(out.lines().filter(|l| !l.is_empty()).map(str::to_string).collect())
    }

    /// `git merge --abort`——把倉庫還原到嘗試合併之前的樣子。
    pub async fn merge_abort(&self) -> Result<VcsResult, String> {
        self.git(&["merge".to_string(), "--abort".to_string()])?;
        Ok(VcsResult::WriteSuccess {
            operation: "merge_abort".to_string(),
            detail: "Aborted the in-progress merge".to_string(),
        })
    }
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
cd src-tauri && cargo test --workspace reports_merge_state 2>&1 | grep -E "^test |test result" | head -3
```

Expected：`1 passed`。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/vcs/git.rs
git commit -m "feat(vcs): GitClient 加上合併狀態查詢與 merge --abort"
```

---

### Task 2: `tasks_merge_worktree` 改回傳結構化結果 + 前置檢查

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs:239-268`
- Modify: `src-tauri/src/lib.rs`（註冊 `tasks_abort_merge`）

- [ ] **Step 1: 寫會紅的測試**

加進既有的 `merge_worktree_tests` 模組（`tasks.rs:806`）。這裡沿用該模組
既有的作法：照抄 command body 的邏輯，避開 Tauri 的 `State` extractor。

```rust
    /// 原分支有未提交變更時，必須在動手之前就擋下來並講出是哪些檔案——
    /// 讓 git 自己拒絕的話，使用者只會看到一段沒頭沒尾的 stderr。
    #[tokio::test]
    async fn refuses_when_the_base_branch_is_dirty() {
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        fs::write(project_dir.path().join("a.txt"), "使用者改到一半\n").unwrap();

        let base_client = GitClient::new(project_dir.path().to_string_lossy().to_string(), None);
        assert!(base_client.has_uncommitted_changes().await.unwrap());
        assert!(!base_client.is_merge_in_progress().await);
    }

    /// 衝突時：worktree 與分支都要原樣保留（成果還在那個分支上），
    /// 而且倉庫確實停在半合併狀態等使用者決定。
    #[tokio::test]
    async fn conflict_keeps_the_worktree_and_leaves_merge_in_progress() {
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

        // 兩邊改同一個檔案的同一行 → 必定衝突。
        fs::write(wt_path.join("a.txt"), "worktree 版本\n").unwrap();
        let worktree_client = GitClient::new(wt_path.to_string_lossy().to_string(), None);
        worktree_client.commit_all("Task").await.unwrap();

        fs::write(project_dir.path().join("a.txt"), "原分支版本\n").unwrap();
        base_client.commit_all("base moved on").await.unwrap();

        assert!(base_client.merge_branch(&branch).await.is_err());
        assert!(base_client.is_merge_in_progress().await, "衝突後倉庫應該停在合併進行中");
        assert_eq!(base_client.conflicted_files().await.unwrap(), vec!["a.txt".to_string()]);
        assert!(wt_path.exists(), "衝突時 worktree 必須原樣保留——成果都在那個分支上");

        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.worktree_path.is_some(), "衝突時不可以清掉 DB 欄位，否則按鈕會消失");
    }
```

- [ ] **Step 2: 跑測試確認它是紅的**

```bash
cd src-tauri && cargo test --workspace conflict_keeps_the_worktree 2>&1 | grep -E "^error|cannot find" | head -3
```

Expected：編譯失敗（`is_merge_in_progress` 若 Task 1 已完成則這兩條可能直接綠——
**那代表它們只是在描述既有行為，不是在保護新行為**。若如此，把它們留著當回歸
測試即可，真正的紅燈在 Step 3 的 `MergeOutcome` 上）。

- [ ] **Step 3: 改寫 command**

把 `tasks.rs:239-268` 整段換成：

```rust
/// 「合併回原分支」的結果。衝突不是錯誤而是一種需要使用者決定的狀態，
/// 所以用結構化結果回傳，而不是塞進 Err 字串讓前端去比對文字。
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MergeOutcome {
    /// 合併完成，worktree 已清理。
    Merged,
    /// 有衝突。原專案目錄停在合併進行中，worktree 與分支原樣保留。
    Conflict { files: Vec<String> },
    /// 還沒開始合併就擋下來了。
    Blocked { reason: BlockedReason, files: Vec<String> },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    /// 原專案目錄有未提交的變更。
    DirtyBase,
    /// 原專案目錄已經停在上一次沒收尾的合併。
    MergeInProgress,
}

#[tauri::command]
pub async fn tasks_merge_worktree(
    project_id: String,
    id: String,
    reg: State<'_, ProjectRegistry>,
    app: AppHandle,
) -> Result<MergeOutcome, String> {
    let p = project(&reg, &project_id)?;
    let row = store::get_task(&p.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let worktree_path = row.worktree_path.ok_or_else(|| "this card has no worktree to merge".to_string())?;
    let worktree_branch = row.worktree_branch.ok_or_else(|| "this card has no worktree to merge".to_string())?;

    let base_client = GitClient::new(row.project_dir.clone(), None);

    // 前置檢查都在動任何東西之前做完——一旦開始 commit/merge 就很難乾淨地
    // 退回去，而這兩種狀況都是使用者自己就能處理的。
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

    let worktree_client = GitClient::new(worktree_path.clone(), None);
    if worktree_client.has_uncommitted_changes().await? {
        worktree_client.commit_all(&format!("Task: {}", row.title)).await?;
    }

    // 合併失敗時 worktree/分支一律原樣保留——成果都已經 commit 在那個分支
    // 上，保留住使用者才有機會解衝突或改天再試。
    if base_client.merge_branch(&worktree_branch).await.is_err() {
        let files = base_client.conflicted_files().await.unwrap_or_default();
        if !files.is_empty() {
            // 真的是衝突：把決定權交還給使用者（前端跳視窗問要自己解還是還原）。
            return Ok(MergeOutcome::Conflict { files });
        }
        // 不是衝突的其他失敗：倉庫可能已經停在半合併，先還原再把錯誤往上丟。
        if base_client.is_merge_in_progress().await {
            let _ = base_client.merge_abort().await;
        }
        return Err(base_client.merge_branch(&worktree_branch).await.unwrap_err());
    }

    base_client.remove_worktree(&worktree_path).await?;
    store::clear_worktree(&p.pool, &id).await.map_err(|e| e.to_string())?;
    emit_updated(&app);
    Ok(MergeOutcome::Merged)
}

/// 還原一次沒收尾的合併（`git merge --abort`）。前端在衝突視窗上選「還原」
/// 時呼叫。worktree 與分支不動——成果都在那個分支上，隨時能再試。
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
```

**注意**：上面用到 `dirty_files()`，`GitClient` 目前沒有這個方法。在
`git.rs` 的 `has_uncommitted_changes`（第 295 行）旁邊補上：

```rust
    /// `git status --porcelain` 列出的檔案名，給「原分支不乾淨」的提示用。
    pub async fn dirty_files(&self) -> Result<Vec<String>, String> {
        let out = self.git(&["status".to_string(), "--porcelain".to_string()])?;
        Ok(out
            .lines()
            .filter_map(|l| l.get(3..).map(str::to_string))
            .filter(|s| !s.is_empty())
            .collect())
    }
```

- [ ] **Step 4: 註冊新指令**

`src-tauri/src/lib.rs`：`tasks_merge_worktree` 出現在 use 清單（約第 119 行）
與 `invoke_handler` 清單（約第 597 行）的地方，各自在旁邊加上
`tasks_abort_merge,`。

- [ ] **Step 5: 編譯 + 跑測試**

```bash
cd src-tauri && cargo test --workspace merge_worktree_tests 2>&1 | grep -E "^test |test result|^error" | head -8
```

Expected：全部 pass，沒有 error。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/vcs/git.rs src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs
git commit -m "feat(board): 合併前先檢查原分支，衝突改成結構化結果交還給使用者"
```

---

### Task 3: i18n 文案

**Files:**
- Modify: `src/lib/i18n.ts`（`zhTW` 與 `enRaw` 兩個區塊的結尾）

- [ ] **Step 1: 加中文**

```ts
    // 合併回原分支
    board_merge_running: "合併中…",
    board_merge_conflict_title: "合併有衝突",
    board_merge_conflict_body: (files: string) =>
      `以下檔案有衝突，尚未合併進原分支：\n\n${files}\n\n原專案目錄目前停在「合併進行中」。\n選「我自己解」會保持這個狀態，您可以直接在原專案目錄解衝突後 git commit，再回來按一次合併。\n選「還原」會執行 git merge --abort，把原專案目錄回復乾淨；工作成果仍然完整保留在這張卡片的分支上，隨時可以再試。`,
    board_merge_conflict_keep: "我自己解",
    board_merge_conflict_abort: "還原",
    board_merge_blocked_title: "還不能合併",
    board_merge_blocked_dirty: (files: string) =>
      `原專案目錄有未提交的變更，git 不會讓合併進行。請先提交或 stash 這些檔案：\n\n${files}`,
    board_merge_blocked_in_progress: (files: string) =>
      `原專案目錄還停在上一次沒有收尾的合併。請先在原專案目錄把它處理完（解衝突後 git commit，或 git merge --abort），再回來合併。\n\n未解的檔案：\n${files}`,
```

- [ ] **Step 2: 加英文**

```ts
    // Merge worktree back
    board_merge_running: "Merging…",
    board_merge_conflict_title: "Merge conflict",
    board_merge_conflict_body: (files: string) =>
      `These files conflict and were not merged into the base branch:\n\n${files}\n\nThe project directory is now mid-merge.\nChoose "Let me resolve it" to keep that state — resolve the conflicts in the project directory, git commit, then press merge again.\nChoose "Undo" to run git merge --abort and restore the project directory; your work stays committed on this card's branch and can be merged later.`,
    board_merge_conflict_keep: "Let me resolve it",
    board_merge_conflict_abort: "Undo",
    board_merge_blocked_title: "Cannot merge yet",
    board_merge_blocked_dirty: (files: string) =>
      `The project directory has uncommitted changes, so git will not merge. Commit or stash these first:\n\n${files}`,
    board_merge_blocked_in_progress: (files: string) =>
      `The project directory is still mid-merge from a previous attempt. Finish it there (resolve and git commit, or git merge --abort), then merge again.\n\nUnresolved files:\n${files}`,
```

- [ ] **Step 3: 型別檢查 + Commit**

```bash
npx tsc -b && echo TSC_OK
git add src/lib/i18n.ts && git commit -m "feat(i18n): 合併衝突與前置檢查的中英文案"
```

---

### Task 4: 前端接上結果與進度回饋

**Files:**
- Modify: `src/ipc/tasks.ts:120`
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Test: `src/components/TaskBoard/TaskCard.test.tsx`（已存在）

- [ ] **Step 1: IPC 型別與新指令**

`src/ipc/tasks.ts` 把第 120 行的 `mergeTaskWorktree` 換成：

```ts
export type MergeOutcome =
  | { status: "merged" }
  | { status: "conflict"; files: string[] }
  | { status: "blocked"; reason: "dirty_base" | "merge_in_progress"; files: string[] };

export const mergeTaskWorktree = (projectId: string, id: string): Promise<MergeOutcome> =>
  invoke("tasks_merge_worktree", { projectId, id });

export const abortMerge = (projectId: string, id: string): Promise<void> =>
  invoke("tasks_abort_merge", { projectId, id });
```

- [ ] **Step 2: 寫會紅的測試**

加進 `TaskCard.test.tsx`（mock 清單要補 `abortMerge`）：

```tsx
  it("衝突時問使用者，選「還原」才呼叫 abort", async () => {
    mergeTaskWorktree.mockResolvedValue({ status: "conflict", files: ["a.txt", "b.txt"] });
    confirmDialog.mockResolvedValue(true); // true = okLabel = 還原
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(abortMerge).toHaveBeenCalledWith("p1", "t1"));
    expect(String(confirmDialog.mock.calls[0][0])).toContain("a.txt");
  });

  it("衝突時選「我自己解」就不還原，保持半合併狀態", async () => {
    mergeTaskWorktree.mockResolvedValue({ status: "conflict", files: ["a.txt"] });
    confirmDialog.mockResolvedValue(false);
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(confirmDialog).toHaveBeenCalled());
    expect(abortMerge).not.toHaveBeenCalled();
  });

  it("原分支不乾淨時只提示，不會動到任何東西", async () => {
    mergeTaskWorktree.mockResolvedValue({
      status: "blocked",
      reason: "dirty_base",
      files: ["config.yml"],
    });
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));

    await waitFor(() => expect(messageDialog).toHaveBeenCalled());
    expect(String(messageDialog.mock.calls[0][0])).toContain("config.yml");
    expect(abortMerge).not.toHaveBeenCalled();
  });

  it("合併期間按鈕顯示進度文字", async () => {
    let release: (v: unknown) => void = () => {};
    mergeTaskWorktree.mockReturnValue(new Promise((r) => { release = r; }));
    mount();

    await userEvent.click(screen.getByText("合併回原分支"));
    await screen.findByText("合併中…");

    release({ status: "merged" });
    await waitFor(() => expect(screen.queryByText("合併中…")).toBeNull());
  });
```

- [ ] **Step 3: 跑測試確認是紅的**

```bash
npx vitest run src/components/TaskBoard/TaskCard.test.tsx 2>&1 | grep -E "×|Tests " | head -6
```

Expected：四條新測試全紅。

- [ ] **Step 4: 實作**

`TaskCard.tsx`：import 補 `abortMerge` 與 `type MergeOutcome`，加一個
`merging` 狀態，並把合併按鈕改成專屬的 handler（不再走通用的 `run`，因為
它要處理三種結果）：

```tsx
  const [merging, setMerging] = useState(false);

  const mergeWorktree = async () => {
    setMerging(true);
    setBusy(true);
    try {
      const outcome: MergeOutcome = await mergeTaskWorktree(projectId, card.id);
      if (outcome.status === "merged") {
        onChanged();
        return;
      }
      if (outcome.status === "blocked") {
        const body =
          outcome.reason === "dirty_base"
            ? t.board_merge_blocked_dirty(outcome.files.join("\n"))
            : t.board_merge_blocked_in_progress(outcome.files.join("\n"));
        await message(body, { title: t.board_merge_blocked_title, kind: "warning" });
        return;
      }
      // conflict：原專案目錄現在停在合併進行中，決定權交給使用者。
      // 取消（含直接關掉視窗）＝維持現狀，這跟「什麼都不做」一致。
      const undo = await confirm(t.board_merge_conflict_body(outcome.files.join("\n")), {
        title: t.board_merge_conflict_title,
        kind: "warning",
        okLabel: t.board_merge_conflict_abort,
        cancelLabel: t.board_merge_conflict_keep,
      });
      if (undo) await abortMerge(projectId, card.id);
    } catch (e) {
      await message(errorText(e), { title: card.title, kind: "error" });
    } finally {
      setMerging(false);
      setBusy(false);
    }
  };
```

按鈕（`TaskCard.tsx:151-155`）改成：

```tsx
            {card.worktree_branch && (
              <button className="tb-btn tb-btn--primary" disabled={busy} onClick={() => void mergeWorktree()}>
                {merging ? t.board_merge_running : t.board_action_merge_worktree}
              </button>
            )}
```

- [ ] **Step 5: 跑測試確認全綠**

```bash
npx vitest run src/components/TaskBoard/TaskCard.test.tsx 2>&1 | grep -E "×|Tests " | head -4
npx tsc -b && echo TSC_OK
```

- [ ] **Step 6: Commit**

```bash
git add src/ipc/tasks.ts src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/TaskCard.test.tsx
git commit -m "feat(board): 衝突時讓使用者選自己解或還原，合併中顯示進度"
```

---

### Task 5: `git()` 移進 spawn_blocking

排在最後，因為它動的範圍最大而且沒有使用者可見的效果——前面四個任務先落地，
這個出問題也不影響它們。

**Files:**
- Modify: `src-tauri/src/vcs/git.rs`

- [ ] **Step 1: 把 `git()` 改成 async**

```rust
    /// 跑一個 git 指令。
    ///
    /// **在 `spawn_blocking` 裡跑**：`std::process::Command::output()` 是阻塞
    /// 的，而這個型別的方法全部從 async 的 Tauri 指令裡呼叫。直接阻塞會佔住
    /// 一條 tokio 工作執行緒直到 git 跑完——大型 worktree 的
    /// `status`/`add -A`/`worktree remove` 動輒數十秒（Windows 上因為防毒
    /// 逐檔掃描更久），期間背景排程等工作都會被拖住。
    async fn git(&self, args: &[String]) -> Result<String, String> {
        let args: Vec<String> = args.to_vec();
        let repo_root = self.repo_root.clone();
        tokio::task::spawn_blocking(move || {
            let mut cmd = Command::new("git");
            cmd.args(&args).current_dir(&repo_root);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            let out = cmd.output().map_err(|e| format!("git exec error: {e}"))?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).to_string())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).to_string())
            }
        })
        .await
        .map_err(|e| format!("git task join error: {e}"))?
    }
```

- [ ] **Step 2: 讓編譯器列出所有要補 `.await` 的地方**

```bash
cd src-tauri && cargo check 2>&1 | grep -E "^error" | head -40
```

把 `diff_tree_files` / `parse_remote` / `require_github` 一併改成 `async fn`，
並在每個 `self.git(...)`、`self.diff_tree_files(...)`、`self.parse_remote()`、
`self.require_github(...)` 後面補 `.await`。**漏掉任何一個都是編譯錯誤**，
所以照著編譯器的清單改到沒有 error 為止即可，不需要自己清點 22 個呼叫點。

- [ ] **Step 3: 全套 Rust 測試**

```bash
cd src-tauri && cargo test --workspace 2>&1 | grep -E "FAILED|error\[|test result: FAILED" | head -5
echo "(沒有輸出 = 全綠)"
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/vcs/git.rs
git commit -m "perf(vcs): git 指令移進 spawn_blocking，不再佔住 async 執行緒"
```

---

### Task 6: 完整驗證

- [ ] **Step 1: 四道關卡全跑**

```bash
npx tsc -b && echo TSC_OK
npx vitest run 2>&1 | grep -E "Test Files|Tests |FAIL"
npx eslint src/components/TaskBoard/TaskCard.tsx src/ipc/tasks.ts && echo LINT_OK
cd src-tauri && cargo test --workspace 2>&1 | grep -E "FAILED|error\[" | head -5
```

Expected：`TSC_OK`、測試全綠、`LINT_OK`、cargo 沒有 FAILED。

**注意**：`TerminalView.tsx` 有 19 條**既有的** `react-hooks` lint 問題，
不是這輪造成的，不要順手修。

---

## 驗收（需要 Windows 實機）

本機（macOS）能驗到 Task 6。以下要 Windows 實機，做法是推
`v<版本>-<主題><n>` 形式的 pre-release tag，**推 tag 前要先問過使用者**：

1. 原專案目錄留一個未提交的變更 → 按合併 → 應該跳「還不能合併」並列出該檔案，
   而且**原專案目錄完全沒被動過**（`git status` 前後一致）。
2. 製造真衝突 → 按合併 → 跳衝突視窗列出衝突檔案。
   - 選「我自己解」→ 原專案目錄停在合併進行中，worktree 還在，卡片按鈕還在。
   - 選「還原」→ `git status` 乾淨，worktree 還在，可以再按一次。
3. 大型 worktree 按合併 → 按鈕文字變成「合併中…」，期間工作看板其他操作
   不會卡住。
4. 正常情況合併成功 → worktree 消失、卡片上的合併按鈕消失。

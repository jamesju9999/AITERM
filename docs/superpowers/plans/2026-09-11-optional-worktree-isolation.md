# worktree 隔離可關閉 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者能關掉「派工時為每張卡片建立獨立 git worktree」這個行為，全域可設、單張卡片可覆寫。

**Architecture:** 全域布林存在 `TaskBoardConfig`；每張卡片存一個 `Option<bool>`（`NULL` = 沿用全域）。派工當下由 `RealDispatcher::dispatch` 解析成一個 `bool` 傳進 `prepare_worktree`；為 `false` 時走既有的「非 git repo」回傳路徑，不是新邏輯。

**Tech Stack:** Rust（sqlx / Tauri command / serde）、React + TypeScript、Vitest。

**Spec:** `docs/superpowers/specs/2026-09-11-optional-worktree-isolation-design.md`

**已探勘確認的既有結構**（實作時不需重查）：

- `TaskRow` 用 `#[derive(FromRow)]` 配 `SELECT *`（`store.rs:14`），所以加欄位
  只要改 struct + 在 `init_schema` 補 `ALTER TABLE`。
- `init_schema`（`tasks/mod.rs:35`）的遷移寫法是 `let _ = sqlx::query("ALTER
  TABLE tasks ADD COLUMN ...")`——欄位已存在時會失敗，那是正常的，刻意丟掉。
- `tasks_create` 的做法是先 `create_task` 再呼叫 `set_bridge_config` /
  `set_label` 這種獨立 setter（`commands/tasks.rs:101-117`），新欄位照做。
- 排程器讀全域設定的寫法是 `self.config.get().task_board.<欄位>`
  （`scheduler.rs:142`）。
- 設定頁的核取方塊樣式見 `TaskBoardPage.tsx:99-111`。
- 編輯對話框的多態 `select` 樣式見 `TaskEditorDialog.tsx:62`（`bridgeChoice`）。
- `prepare_worktree` 既有測試在 `scheduler.rs:1181`，已涵蓋「非 git repo」與
  「git repo」兩種情況。

---

## File Structure

| 檔案 | 這輪的責任 |
|------|-----------|
| `src-tauri/src/config/types.rs` | 全域 `isolate_with_worktree` |
| `src-tauri/src/tasks/mod.rs` | `isolate_worktree` 欄位的 schema 與遷移 |
| `src-tauri/src/tasks/store.rs` | `TaskRow` 欄位 + `set_isolate_worktree` |
| `src-tauri/src/tasks/scheduler.rs` | `prepare_worktree` 收 `isolate` 參數；派工當下解析 |
| `src-tauri/src/commands/tasks.rs` | `CreateArgs`/`UpdateArgs` 新欄位 |
| `src/ipc/tasks.ts` | 型別 |
| `src/lib/i18n.ts` | 中英文案 |
| `src/components/Settings/TaskBoardPage.tsx` | 全域開關與併發提醒 |
| `src/components/TaskBoard/TaskEditorDialog.tsx` | 三態 select |

---

### Task 1: 全域設定欄位

**Files:**
- Modify: `src-tauri/src/config/types.rs:199-247`

- [ ] **Step 1: 寫會紅的測試**

`config/types.rs` 測試模組（`:917` 附近已有 `TaskBoardConfig::default()` 的測試）加：

```rust
    #[test]
    fn worktree_isolation_defaults_to_on_and_survives_old_config_files() {
        // 預設必須是 true——這個功能只是多一個選項，不是改變既有行為。
        assert!(TaskBoardConfig::default().isolate_with_worktree);

        // 舊的 config.json 沒有這個欄位，反序列化後也要是 true，
        // 否則升級 App 會讓所有既有使用者突然失去隔離。
        let c: TaskBoardConfig = serde_json::from_str(r#"{"max_concurrent":3}"#).unwrap();
        assert!(c.isolate_with_worktree, "舊設定檔缺欄位時必須落到 true");
    }
```

- [ ] **Step 2: 跑測試確認是紅的**

```bash
cd src-tauri && cargo test --workspace worktree_isolation_defaults 2>&1 | grep -E "^error|no field" | head -3
```

Expected：`no field \`isolate_with_worktree\``。

- [ ] **Step 3: 實作**

在 `TaskBoardConfig`（`:199`）的 `stuck_timeout_secs` 之後加：

```rust
    /// 派工時是否為每張卡片建立獨立的 git worktree。關掉的話 Agent 直接在
    /// 專案目錄工作——適合信任 Agent 自己操作 git 版控的使用者。單張卡片
    /// 可以用 `TaskRow::isolate_worktree` 覆寫這個預設值。
    #[serde(default = "default_true")]
    pub isolate_with_worktree: bool,
```

並在 `impl Default`（`:235`）補 `isolate_with_worktree: true,`。

- [ ] **Step 4: 跑測試確認變綠**

```bash
cd src-tauri && cargo test --workspace worktree_isolation_defaults 2>&1 | grep -E "^test config|test result: ok. [1-9]" | head -3
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/config/types.rs
git commit -m "feat(config): 新增 isolate_with_worktree 全域開關"
```

---

### Task 2: 卡片層欄位與資料庫遷移

**Files:**
- Modify: `src-tauri/src/tasks/store.rs`（`TaskRow` 與 setter）
- Modify: `src-tauri/src/tasks/mod.rs:35-108`（schema 與遷移）

- [ ] **Step 1: 寫會紅的測試**

`store.rs` 測試模組加（照抄 `:1434` 既有遷移測試的寫法）：

```rust
    /// 舊資料庫沒有 `isolate_worktree` 欄位時，`init_schema` 要能補上，
    /// 而且舊資料要讀成 `None`（＝沿用全域），不是欄位缺席讓 `SELECT *`
    /// 配 `FromRow` 直接爆掉。
    #[tokio::test]
    async fn migrates_old_db_without_isolate_worktree_column() {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        // 刻意建一個沒有該欄位的舊 schema。
        sqlx::query(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL DEFAULT '',
                project_dir TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'planning',
                parallel_ok INTEGER NOT NULL DEFAULT 1,
                sort_order REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();

        crate::tasks::init_schema(&pool).await.unwrap();

        let id = create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        let row = get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.isolate_worktree, None, "新卡片預設沿用全域，不是寫死 true/false");

        set_isolate_worktree(&pool, &id, Some(false)).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().isolate_worktree, Some(false));

        set_isolate_worktree(&pool, &id, Some(true)).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().isolate_worktree, Some(true));

        // 回到「沿用全域」也要存得回去，不能只能單向設定。
        set_isolate_worktree(&pool, &id, None).await.unwrap();
        assert_eq!(get_task(&pool, &id).await.unwrap().unwrap().isolate_worktree, None);
    }
```

- [ ] **Step 2: 跑測試確認是紅的**

```bash
cd src-tauri && cargo test --workspace migrates_old_db_without_isolate 2>&1 | grep -E "^error|cannot find|no field" | head -3
```

Expected：`cannot find function \`set_isolate_worktree\``。

- [ ] **Step 3: 實作**

`store.rs` 的 `TaskRow` 在 `worktree_branch`（`:53` 附近）之後加：

```rust
    /// 這張卡片要不要用獨立的 git worktree。`None` 代表沿用
    /// `TaskBoardConfig::isolate_with_worktree`；派工當下才解析，所以改動
    /// 全域設定會影響所有還在等待、又沒有個別覆寫的卡片。
    pub isolate_worktree: Option<bool>,
```

同檔案加 setter（放在 `clear_worktree`（`:501`）旁邊）：

```rust
/// 設定單張卡片的 worktree 隔離覆寫值。`None` 寫回 NULL＝沿用全域。
pub async fn set_isolate_worktree(
    pool: &SqlitePool,
    id: &str,
    value: Option<bool>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET isolate_worktree = ? WHERE id = ?")
        .bind(value)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

`tasks/mod.rs` 的 `CREATE TABLE`（`:60` 附近，`worktree_branch TEXT` 之後）加
`isolate_worktree INTEGER`，並在遷移區塊末尾（`worktree_branch` 那兩行之後）加：

```rust
    // Migration: existing databases created before `isolate_worktree` existed.
    // NULL＝沿用全域設定，所以不給 DEFAULT。
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN isolate_worktree INTEGER")
        .execute(pool)
        .await;
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
cd src-tauri && cargo test --workspace migrates_old_db_without_isolate 2>&1 | grep -E "^test tasks|test result: ok. [1-9]" | head -3
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/store.rs src-tauri/src/tasks/mod.rs
git commit -m "feat(tasks): 卡片層 isolate_worktree 欄位與遷移"
```

---

### Task 3: `prepare_worktree` 收 `isolate` 參數

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs:70-95`（函式）、`:155`（呼叫端）、`:1181`（測試）

- [ ] **Step 1: 寫會紅的測試**

`scheduler.rs` 的 `prepare_worktree_tests` 模組加：

```rust
    #[tokio::test]
    async fn isolation_turned_off_runs_directly_in_the_project_dir() {
        // 即使是 git repo，關掉隔離就該走跟「非 git repo」完全同一條路徑。
        let project_dir = tempfile::tempdir().unwrap();
        init_repo(project_dir.path());
        let storage_dir = tempfile::tempdir().unwrap();

        let (effective_dir, info) = prepare_worktree(
            storage_dir.path(),
            "t3",
            &project_dir.path().to_string_lossy(),
            false,
        ).await;

        assert_eq!(effective_dir, project_dir.path(), "關掉隔離就該直接在專案目錄跑");
        assert!(info.is_none(), "沒有 worktree 就不該回報 path/branch，否則合併按鈕會冒出來");
        assert!(!storage_dir.path().join("t3").join("worktree").exists(), "不該留下任何 worktree 目錄");
    }
```

同時把該模組既有的兩條測試的呼叫補上第四個參數 `true`。

- [ ] **Step 2: 跑測試確認是紅的**

```bash
cd src-tauri && cargo test --workspace prepare_worktree_tests 2>&1 | grep -E "^error|takes 3 arguments" | head -3
```

Expected：`this function takes 3 arguments but 4 arguments were supplied`。

- [ ] **Step 3: 實作**

`prepare_worktree`（`:70`）改簽名並在最前面短路：

```rust
async fn prepare_worktree(
    project_storage_path: &std::path::Path,
    task_id: &str,
    task_project_dir: &str,
    isolate: bool,
) -> (std::path::PathBuf, Option<(String, String)>) {
    let fallback = std::path::PathBuf::from(task_project_dir);

    // 關掉隔離時直接走跟「這個目錄不是 git repo」完全一樣的回傳值——
    // 那條路徑本來就存在而且驗證過，不需要新的邏輯。
    if !isolate {
        return (fallback, None);
    }

    match VcsManager::detect_repo(task_project_dir).await {
```

（其餘不動。）

呼叫端（`:155`）改成：

```rust
        let isolate = task
            .isolate_worktree
            .unwrap_or(self.config.get().task_board.isolate_with_worktree);
        let (effective_dir, worktree_info) =
            prepare_worktree(&project.path, &task.id, &task.project_dir, isolate).await;
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
cd src-tauri && cargo test --workspace prepare_worktree_tests 2>&1 | grep -E "^test tasks|test result: ok. [1-9]" | head -5
```

Expected：3 條全過。

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): 派工當下解析是否隔離，關掉時直接在專案目錄跑"
```

---

### Task 4: 建立／編輯卡片時帶上這個欄位

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs:84-93`（CreateArgs）、`:95-120`（tasks_create）、`:124-134`（UpdateArgs）、`tasks_update` body

- [ ] **Step 1: 兩個 args struct 各加一個欄位**

`CreateArgs` 與 `UpdateArgs` 都在 `label` 之後加：

```rust
    /// `None` = 沿用全域設定，見 `TaskRow::isolate_worktree`。
    pub isolate_worktree: Option<bool>,
```

- [ ] **Step 2: 兩個 command 各多呼叫一次 setter**

`tasks_create`（`:117` 的 `set_label` 之後）與 `tasks_update` 的對應位置各加：

```rust
    store::set_isolate_worktree(&p.pool, &id, args.isolate_worktree)
        .await
        .map_err(|e| e.to_string())?;
```

（`tasks_update` 裡的 id 變數是 `args.id`，依該函式現有寫法調整。）

- [ ] **Step 3: 編譯**

```bash
cd src-tauri && cargo check 2>&1 | grep -E "^error" -A 3 | head -10
```

Expected：沒有 error。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/tasks.rs
git commit -m "feat(tasks): 建立與編輯卡片時帶上 isolate_worktree"
```

---

### Task 5: 前端型別與文案

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/lib/i18n.ts`

- [ ] **Step 1: 型別**

`src/ipc/tasks.ts` 的 `TaskRow` 介面在 `worktree_branch` 之後加：

```ts
  /** `null` 代表沿用全域的 isolate_with_worktree 設定。 */
  isolate_worktree: boolean | null;
```

`TaskBoardConfig` 介面加：

```ts
  isolate_with_worktree: boolean;
```

`createTask` 的 args 型別與 `updateTask` 的 args 型別各加
`isolate_worktree: boolean | null;`。

- [ ] **Step 2: 中文文案**

`zhTW` 區塊加：

```ts
    // worktree 隔離
    board_settings_isolate: "派工時建立獨立的 git worktree",
    board_settings_isolate_hint:
      "開啟時，每張卡片會在自己的 worktree 與 aiterm-task/<id> 分支上執行，完成後用「合併回原分支」合併。關閉時 Agent 直接在專案目錄工作——適合讓 Agent 自己操作 git 版控。",
    board_settings_isolate_concurrent_warning:
      "⚠️ 同時執行數大於 1：關閉隔離後，多張卡片會在同一個工作目錄互相覆蓋。",
    board_card_isolate: "git worktree 隔離",
    board_card_isolate_inherit: "沿用全域設定",
    board_card_isolate_on: "建立獨立的 worktree",
    board_card_isolate_off: "直接在專案目錄執行",
```

- [ ] **Step 3: 英文文案**

`enRaw` 區塊加：

```ts
    // Worktree isolation
    board_settings_isolate: "Create an isolated git worktree for each task",
    board_settings_isolate_hint:
      "When on, each card runs in its own worktree on an aiterm-task/<id> branch and is merged back with \"Merge into base branch\". When off, the agent works directly in the project directory — suitable if you let the agent manage git itself.",
    board_settings_isolate_concurrent_warning:
      "⚠️ Concurrency is above 1: with isolation off, multiple cards will overwrite each other in the same working directory.",
    board_card_isolate: "Git worktree isolation",
    board_card_isolate_inherit: "Use the global setting",
    board_card_isolate_on: "Create an isolated worktree",
    board_card_isolate_off: "Work directly in the project directory",
```

- [ ] **Step 4: 型別檢查 + Commit**

```bash
npx tsc -b && echo TSC_OK
git add src/ipc/tasks.ts src/lib/i18n.ts
git commit -m "feat(i18n): worktree 隔離設定的型別與中英文案"
```

---

### Task 6: 設定頁開關

**Files:**
- Modify: `src/components/Settings/TaskBoardPage.tsx`
- Test: `src/components/Settings/TaskBoardPage.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

`TaskBoardPage.test.tsx` 的 fixture 補 `isolate_with_worktree: true`，並加：

```tsx
  it("可以關掉 worktree 隔離並存檔", async () => {
    renderPage();
    const box = await screen.findByLabelText("派工時建立獨立的 git worktree");
    await userEvent.click(box);
    await userEvent.click(screen.getByText(/儲存|Save/));

    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ isolate_with_worktree: false }),
      ),
    );
  });

  it("關掉隔離且同時執行數大於 1 時顯示提醒", async () => {
    renderPage({ isolate_with_worktree: false, max_concurrent: 3 });
    expect(await screen.findByText(/多張卡片會在同一個工作目錄互相覆蓋/)).toBeTruthy();
  });

  it("隔離開著時不顯示那個提醒", async () => {
    renderPage({ isolate_with_worktree: true, max_concurrent: 3 });
    await screen.findByText("派工時建立獨立的 git worktree");
    expect(screen.queryByText(/互相覆蓋/)).toBeNull();
  });
```

**注意**：`renderPage` 若目前不接參數，先把它改成接一個 `Partial<TaskBoardConfig>`
覆寫 fixture；既有測試呼叫 `renderPage()` 不受影響。`findByLabelText` 需要
`<label>` 包住 `<input>`，該頁既有的核取方塊寫法已經是這樣。

- [ ] **Step 2: 跑測試確認是紅的**

```bash
npx vitest run src/components/Settings/TaskBoardPage.test.tsx 2>&1 | grep -E "×|Tests " | head -5
```

- [ ] **Step 3: 實作**

在 `notify_telegram_on_finish` 那個區塊之後（`TaskBoardPage.tsx:127` 附近）加：

```tsx
        <label className="task-board-field task-board-field--checkbox">
          <input
            type="checkbox"
            className="task-board-checkbox"
            checked={cfg.isolate_with_worktree}
            onChange={(e) => {
              setSaved(false);
              setCfg({ ...cfg, isolate_with_worktree: e.target.checked });
            }}
          />
          <span>{t.board_settings_isolate}</span>
          <span className="task-board-hint">{t.board_settings_isolate_hint}</span>
        </label>

        {!cfg.isolate_with_worktree && cfg.max_concurrent > 1 && (
          <p className="task-board-hint">{t.board_settings_isolate_concurrent_warning}</p>
        )}
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
npx vitest run src/components/Settings/TaskBoardPage.test.tsx 2>&1 | grep -E "×|Tests " | head -4
```

- [ ] **Step 5: Commit**

```bash
git add src/components/Settings/TaskBoardPage.tsx src/components/Settings/TaskBoardPage.test.tsx
git commit -m "feat(settings): 工作看板加上 worktree 隔離開關"
```

---

### Task 7: 編輯對話框的三態 select

**Files:**
- Modify: `src/components/TaskBoard/TaskEditorDialog.tsx`
- Test: `src/components/TaskBoard/TaskEditorDialog.isolate.test.tsx`（新建）

- [ ] **Step 1: 寫會紅的測試**

新建 `TaskEditorDialog.isolate.test.tsx`。**mock 與掛載方式照抄同資料夾既有的
`TaskEditorDialog.usedLabels.test.tsx`**（同一個元件、同一套 mock 需求），只把
斷言換成下面這些：

```tsx
  it("預設是沿用全域設定，存出 null", async () => {
    renderDialog();
    await userEvent.type(screen.getByLabelText(/標題|Title/), "t");
    await userEvent.click(screen.getByText(/建立|Create/));

    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ isolate_worktree: null }),
      ),
    );
  });

  it("選「直接在專案目錄執行」存出 false", async () => {
    renderDialog();
    await userEvent.type(screen.getByLabelText(/標題|Title/), "t");
    await userEvent.selectOptions(
      screen.getByLabelText("git worktree 隔離"),
      "off",
    );
    await userEvent.click(screen.getByText(/建立|Create/));

    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({ isolate_worktree: false }),
      ),
    );
  });

  it("開啟既有卡片時回填它自己的設定", async () => {
    renderDialog({ card: { isolate_worktree: true } });
    const sel = await screen.findByLabelText<HTMLSelectElement>("git worktree 隔離");
    expect(sel.value).toBe("on");
  });
```

- [ ] **Step 2: 跑測試確認是紅的**

```bash
npx vitest run src/components/TaskBoard/TaskEditorDialog.isolate.test.tsx 2>&1 | grep -E "×|Tests " | head -5
```

- [ ] **Step 3: 實作**

`TaskEditorDialog.tsx` 加狀態（放在 `parallelOk`/`interactive` 旁邊，`:46`）：

```tsx
  const [isolateChoice, setIsolateChoice] = useState<"inherit" | "on" | "off">(() => {
    if (card?.isolate_worktree === true) return "on";
    if (card?.isolate_worktree === false) return "off";
    return "inherit";
  });
```

送出時（`:205` 與 `:215` 兩處 payload）各加：

```tsx
          isolate_worktree: isolateChoice === "inherit" ? null : isolateChoice === "on",
```

UI 放在 `parallel_ok` 那個區塊之後（`:415` 附近）：

```tsx
        <label className="task-field">
          <span>{t.board_card_isolate}</span>
          <select value={isolateChoice} onChange={(e) => setIsolateChoice(e.target.value as "inherit" | "on" | "off")}>
            <option value="inherit">{t.board_card_isolate_inherit}</option>
            <option value="on">{t.board_card_isolate_on}</option>
            <option value="off">{t.board_card_isolate_off}</option>
          </select>
        </label>
```

- [ ] **Step 4: 跑測試確認變綠**

```bash
npx vitest run src/components/TaskBoard/TaskEditorDialog 2>&1 | grep -E "×|Tests " | head -5
```

Expected：新檔案 3 條全過，且同資料夾既有的 TaskEditorDialog 測試都沒被弄壞。

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx src/components/TaskBoard/TaskEditorDialog.isolate.test.tsx
git commit -m "feat(board): 卡片可個別覆寫 worktree 隔離設定"
```

---

### Task 8: 完整驗證

- [ ] **Step 1: 四道關卡全跑**

```bash
npx tsc -b && echo TSC_OK
npx vitest run 2>&1 | grep -E "Test Files|Tests |FAIL"
npx eslint src/components/Settings/TaskBoardPage.tsx src/components/TaskBoard/TaskEditorDialog.tsx src/ipc/tasks.ts && echo LINT_OK
cd src-tauri && cargo test --workspace 2>&1 | grep -E "failed; " | grep -v " 0 failed" | head -3
```

Expected：`TSC_OK`、測試全綠、`LINT_OK`、cargo 那行沒有輸出。

**注意**：`TerminalView.tsx` 有 19 條**既有的** `react-hooks` lint 問題，不是這輪
造成的，不要順手修。

---

## 驗收（需要實機）

本機（macOS）能驗到 Task 8，而且這個功能**不是 Windows 專屬**，macOS 上就能
完整驗證。建議先在本機用 `npm run tauri:dev` 跑過以下情境，確認無誤再考慮推
測試版：

1. 設定維持預設（隔離開著）→ 派一張卡 → 卡片完成後有「合併回原分支」按鈕，
   且 `git worktree list` 看得到那個 worktree。
2. 設定關掉隔離 → 派一張卡 → 卡片完成後**沒有**合併按鈕，`git worktree list`
   沒有新增項目，Agent 的變更直接出現在專案目錄。
3. 全域關掉、但某張卡片選「建立獨立的 worktree」→ 那張卡仍然被隔離。
4. 全域開著、但某張卡片選「直接在專案目錄執行」→ 那張卡不被隔離。
5. 全域關掉且同時執行數設成 3 → 設定頁出現互相覆蓋的提醒。
6. 用舊版建立的卡片（資料庫沒有新欄位）→ App 啟動不報錯，那些卡片視為「沿用
   全域」。

# 工作看板「互動模式」 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓工作看板卡片可以在建立當下選擇「互動模式」——這種任務會自動送出初始 prompt，但不受 bell/完成標記/120 秒無輸出這些自動完成訊號約束、不占併發上限，改由使用者親自對話完，用按鈕或拖曳手動標記完成。

**Architecture:** `tasks` 表新增 `interactive` 欄位；派工時依這個欄位決定要不要附加完成標記指示、要用哪種 `monitor::WatchMode`；排程器讓互動任務完全繞過併發上限的挑選邏輯；`monitor::watch` 的外部控制頻道從單純 cancel 擴充成 `WatchControl::{Cancel, MarkDone}`，手動完成直接送 `MarkDone` 讓既有的收尾流程（含對話記錄自動升級）原樣接手。

**Tech Stack:** Tauri 2、Rust（sqlx/SQLite、tokio）、React 19/TypeScript。

---

## 背景（每個任務都可能用到）

現況調查已完整記錄在 spec：`docs/superpowers/specs/2026-09-03-task-board-interactive-mode-design.md`。以下是這份計畫每個任務都會引用的既有事實，先列出來避免每個任務重複貼一次：

- `tasks` 表 schema 定義在 `src-tauri/src/tasks/mod.rs::init_schema`，用 `CREATE TABLE IF NOT EXISTS`（對已存在的資料庫是 no-op，不會補欄位）。這個 repo 對「既有資料庫要多一個欄位」的既有做法是額外加一行 `let _ = sqlx::query("ALTER TABLE ... ADD COLUMN ...").execute(pool).await;`（忽略錯誤——欄位已存在時會出錯，這是預期的）,範例在 `src-tauri/src/db/design.rs:75-77`。
- `TaskRow`（`src-tauri/src/tasks/store.rs:14-30`）用 `#[derive(FromRow)]` 直接對應 SELECT * 的結果，新增欄位只需要加一個 struct 欄位 + 讓 schema 有這個欄位，不需要手動映射。
- `store::create_task`（`src-tauri/src/tasks/store.rs:49-79`）目前簽章是 `(pool, title, body, project_dir, parallel_ok) -> Result<String, sqlx::Error>`。這個函式全 repo 有 18 個呼叫點（見任務 1 的完整清單），簽章一改全部要同步更新，否則編譯不過。
- `SchedulerHandle`/`RealDispatcher` 的 `cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<()>>>>`（`src-tauri/src/tasks/scheduler.rs:61,167`）是唯一一條「外部介入正在跑的 watch」的管道，`cancel(task_id)` 方法送出訊號。

---

### Task 1: 資料模型——`interactive` 欄位

**Files:**
- Modify: `src-tauri/src/tasks/mod.rs`
- Modify: `src-tauri/src/tasks/store.rs`
- Modify: `src-tauri/src/tasks/scheduler.rs`
- Modify: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/tests/task_board.rs`

這個任務只處理資料層——`interactive` 欄位存在、可以寫入/讀出/複製。派工邏輯（怎麼用這個欄位）留給後面的任務。

- [ ] **Step 1: 寫一個會失敗的測試，鎖住「建立時可以指定 interactive、預設 false、clone 會複製」這個行為**

在 `src-tauri/src/tasks/store.rs` 的 `#[cfg(test)] mod tests`（檔案結尾附近，`clone_task_fields_errors_when_source_is_missing` 之後）加：

```rust
    #[tokio::test]
    async fn interactive_flag_defaults_false_and_is_persisted_when_true() {
        let pool = mem_pool().await;
        let auto_id = create_task(&pool, "auto one", "", "/r", true, false).await.unwrap();
        let chat_id = create_task(&pool, "chat one", "", "/r", true, true).await.unwrap();
        assert!(!get_task(&pool, &auto_id).await.unwrap().unwrap().interactive);
        assert!(get_task(&pool, &chat_id).await.unwrap().unwrap().interactive);
    }

    #[tokio::test]
    async fn clone_task_fields_copies_the_interactive_flag() {
        let pool = mem_pool().await;
        let src = create_task(&pool, "chat one", "", "/r", true, true).await.unwrap();
        let new_id = clone_task_fields(&pool, &src).await.unwrap();
        assert!(get_task(&pool, &new_id).await.unwrap().unwrap().interactive);
    }
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib tasks::store::tests::interactive_flag 2>&1 | tail -20`
Expected: 編譯錯誤——`create_task` 目前只接受 4 個參數（`pool, title, body, project_dir, parallel_ok`），這裡傳了 5 個。

- [ ] **Step 3: schema 加欄位**

`src-tauri/src/tasks/mod.rs`，`init_schema` 裡的 `CREATE TABLE IF NOT EXISTS tasks (...)` 那段，在 `parallel_ok INTEGER NOT NULL DEFAULT 1,` 後面加一行：

```rust
            interactive     INTEGER NOT NULL DEFAULT 0,
```

在那個 `CREATE TABLE` 的 `.execute(pool).await?;` 之後、`CREATE INDEX idx_tasks_status` 之前，加一行既有資料庫的遷移（忽略錯誤——欄位已存在時的錯誤是預期的，跟 `db/design.rs:75-77` 同款寫法）：

```rust
    // Migration: existing databases created before `interactive` existed.
    let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await;
```

- [ ] **Step 4: `TaskRow` 加欄位**

`src-tauri/src/tasks/store.rs`，`TaskRow` struct（14-30 行）裡 `parallel_ok` 那行之後加：

```rust
    // Set at creation time, editable only while `planning` (same rule as
    // `parallel_ok`) — see docs/superpowers/specs/2026-09-03-task-board-interactive-mode-design.md.
    pub interactive: bool,
```

- [ ] **Step 5: `create_task`/`clone_task_fields`/新增 `set_interactive`**

`src-tauri/src/tasks/store.rs`，`create_task` 函式（49-79 行）簽章加一個參數、INSERT 語句加一欄：

```rust
pub async fn create_task(
    pool: &SqlitePool,
    title: &str,
    body: &str,
    project_dir: &str,
    parallel_ok: bool,
    interactive: bool,
) -> Result<String, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let next_order: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(MAX(sort_order), 0) + 1 AS REAL) FROM tasks WHERE status = ?",
    )
    .bind(STATUS_PLANNING)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO tasks (id, title, body, project_dir, status, parallel_ok, interactive, sort_order)
         VALUES (?, ?, ?, ?, 'planning', ?, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(body)
    .bind(project_dir)
    .bind(parallel_ok as i64)
    .bind(interactive as i64)
    .bind(next_order)
    .execute(pool)
    .await?;
    Ok(id)
}
```

`clone_task_fields`（84-88 行附近）改成也帶上 `interactive`：

```rust
pub async fn clone_task_fields(pool: &SqlitePool, src_id: &str) -> Result<String, sqlx::Error> {
    let src = get_task(pool, src_id).await?.ok_or(sqlx::Error::RowNotFound)?;
    create_task(pool, &src.title, &src.body, &src.project_dir, src.parallel_ok, src.interactive).await
}
```

在 `set_parallel_ok`（198-206 行）後面加一個對稱的新函式：

```rust
pub async fn set_interactive(
    pool: &SqlitePool,
    id: &str,
    interactive: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tasks SET interactive = ? WHERE id = ?")
        .bind(interactive as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```

- [ ] **Step 6: 修正 `store.rs` 自己既有的呼叫點（9 處）**

`create_task` 簽章多了一個參數，`store.rs` 檔案內所有既有呼叫都要補上 `, false`（這些測試不關心 interactive，補 `false`保持原行為）。逐一修正：

| 行號附近 | 改法 |
|---|---|
| `create_then_list_roundtrips_a_planning_card` 內 | `create_task(&pool, "Fix the flaky test", "make it deterministic", "/repo", true)` → 加 `, false` |
| `move_planning_to_queued_is_allowed_but_done_to_running_is_not` 內 | `create_task(&pool, "t", "", "/r", true)` → 加 `, false` |
| `midpoint_sort_order_between_two_cards` 內（3 處：`a`/`b`/`c`） | 各自加 `, false` |
| `mark_dispatched_and_finish_set_the_right_columns` 內 | 加 `, false` |
| `recover_orphaned_running_marks_them_cancelled` 內 | 加 `, false` |
| `add_and_remove_attachment_rows` 內 | 加 `, false` |
| `clone_task_fields_copies_the_core_fields_into_a_new_planning_card` 內 | `create_task(&pool, "Ship it", "the body", "/repo/x", false)` → 加 `, false`（第二個 false 是新的 interactive 參數，第一個 false 是既有的 parallel_ok，不要搞混） |

- [ ] **Step 7: 修正其他檔案的呼叫點（讓整個 crate 繼續編譯）**

`create_task` 是 pub 函式，`store.rs` 以外還有 6 處呼叫，都要加 `, false`（這個任務只顧「能編譯、行為不變」，互動模式真正的建立流程留給任務 5）：

`src-tauri/src/tasks/scheduler.rs`（`loop_tests` 模組內，兩處，`store::create_task(&db.pool, t, "", "/r", par)`）：
```rust
            let id = store::create_task(&db.pool, t, "", "/r", par, false).await.unwrap();
```
兩處都這樣改（第 333、355 行附近，一模一樣的寫法，各自照樣加 `, false`）。

`src-tauri/src/tasks/scheduler.rs` 的 `row()` 測試 helper（`pick_next` 純函式的測試用，239-256 行）——這個不是呼叫 `create_task`，是手動建構 `TaskRow` 字面值，一樣要補欄位，直接寫死 `false`（`pick_next` 完全不看這個欄位，不需要當參數開放）：

```rust
    fn row(id: &str, parallel_ok: bool, sort_order: f64) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: id.into(),
            body: String::new(),
            project_dir: "/r".into(),
            status: "queued".into(),
            parallel_ok,
            interactive: false,
            sort_order,
            outcome: None,
            tab_id: None,
            transcript_path: None,
            error_message: None,
            created_at: String::new(),
            dispatched_at: None,
            finished_at: None,
        }
    }
```

`src-tauri/src/commands/tasks.rs`：
- 第 58 行附近，`tasks_create` 指令本體的 `store::create_task(&db.pool, &args.title, &args.body, &args.project_dir, args.parallel_ok)` → 加 `, false`（暫時寫死；任務 5 會把它換成 `args.interactive`，那時 `CreateArgs` 才會有這個欄位）。
- `save_transcript_tests` 模組內兩處 `store::create_task(&pool, "t", "", "/r", true)`（342、365 行附近）→ 各自加 `, false`。

`src-tauri/tests/task_board.rs`：三處 `store::create_task(&db.pool, "...", "", "/...", true)`（88、106、120 行附近）→ 各自加 `, false`。

- [ ] **Step 8: 執行測試，確認通過**

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
Expected: 乾淨編譯，沒有新警告——這一步先確認上面所有呼叫點都補齊了，crate 能編譯。

Run: `cd src-tauri && cargo test --lib tasks:: 2>&1 | tail -40`
Expected: 全部通過，包含 Step 1 新增的兩個測試。

Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`（全套測試，包含 `tests/task_board.rs`、`commands::tasks` 的既有測試）。

- [ ] **Step 9: clippy**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "tasks/mod.rs|tasks/store.rs|tasks/scheduler.rs|commands/tasks.rs|tests/task_board.rs"`
Expected: 空。

- [ ] **Step 10: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/tasks/mod.rs src-tauri/src/tasks/store.rs src-tauri/src/tasks/scheduler.rs src-tauri/src/commands/tasks.rs src-tauri/tests/task_board.rs
git commit -m "feat(tasks): interactive column on the tasks table + store plumbing"
```

---

### Task 2: `dispatch.rs` — 派工時可以選擇不要求完成標記

**Files:**
- Modify: `src-tauri/src/tasks/dispatch.rs`
- Modify: `src-tauri/src/tasks/scheduler.rs`

`run_on_session(pty, tab_id, prompt, request_done_marker)` 已經是參數化的（`src-tauri/src/tasks/dispatch.rs:98-138`）；外層的 `spawn_and_run` 目前呼叫它時寫死 `true`（167 行）。這個任務把這個布林值往外開放到 `spawn_and_run` 的簽章。

- [ ] **Step 1: 寫一個會失敗的測試**

在 `src-tauri/src/tasks/dispatch.rs` 的 `#[cfg(test)] mod tests`（`run_on_session_types_the_prompt_into_an_existing_session` 之後）加：

```rust
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn spawn_and_run_forwards_request_done_marker_false_to_run_on_session() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let pty = PtyManager::new();
        let dir = std::env::temp_dir();
        let (tab_id, _res) = spawn_and_run(
            app.handle(),
            &pty,
            dir.to_str().unwrap(),
            "sh",
            "echo hi",
            false, // request_done_marker
        )
        .await
        .unwrap();

        // The done-marker instruction always mentions the tab_id — its
        // absence after settling is what proves request_done_marker=false
        // actually reached run_on_session instead of being silently dropped.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
        assert!(!out.contains(&tab_id), "done-marker instruction was sent despite request_done_marker=false: {out}");
    }
```

**在寫這個測試之前，先確認 `tauri::test::mock_builder`/`mock_context`/`noop_assets` 這幾個 helper 在這個專案的 `tauri` crate 版本裡確實存在、簽章相符**——執行 `cd src-tauri && cargo doc --no-deps -p tauri --open 2>&1 | head -5` 只是確認能不能生成文件不必真的打開；更直接的做法是 `grep -rn "mock_builder\|mock_context" ~/.cargo/registry/src/*/tauri-*/src/test/mod.rs 2>/dev/null | head -20`，或直接嘗試編譯這個測試看錯誤訊息。如果這個 mock 機制在目前的 tauri 版本用不了，改用下面這個不需要真的 `AppHandle` 的替代寫法（跳過 `spawn_and_run` 本身，直接證明 `run_on_session` 收到 `false` 時的行為——這其實跟既有的 `run_on_session_sends_the_done_marker_instruction_even_when_the_target_never_bells` 測試互為鏡像，一個測 `true`、一個測 `false`）：

```rust
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn run_on_session_does_not_send_the_done_marker_instruction_when_not_requested() {
        let pty = PtyManager::new();
        let tab_id = pty
            .create_with_callback(
                portable_pty::PtySize { rows: 24, cols: 300, pixel_width: 0, pixel_height: 0 },
                |_| {},
            )
            .unwrap();

        run_on_session(&pty, &tab_id, "echo hi", false).await.unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;
        let out = pty.get_recent_output(&tab_id, 8192).unwrap_or_default();
        assert!(!out.contains(&tab_id), "done-marker instruction was sent despite request_done_marker=false: {out}");
    }
```

這個替代版本已經測過 `run_on_session` 本身（既有行為，這個任務不用重新驗證它）——這個任務真正要新增測試覆蓋的是「`spawn_and_run` 有沒有把它收到的參數原封不動往下傳」，不是「`run_on_session` 收到 false 會不會送」（那個已經有既有測試對稱覆蓋 true 的情況）。**判斷用哪個版本、或兩個都留、由實作者依前面那段調查結果決定**，但至少要有一個測試證明 `spawn_and_run` 這一層新增的參數真的有被使用（不是加了參數卻沒接上）。

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib tasks::dispatch 2>&1 | tail -30`
Expected: 編譯錯誤——`spawn_and_run` 目前只接受 5 個參數。

- [ ] **Step 3: 實作**

`src-tauri/src/tasks/dispatch.rs`，`spawn_and_run`（143-169 行）簽章加一個參數，往下傳給 `run_on_session`：

```rust
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    prompt: &str,
    request_done_marker: bool,
) -> Result<(String, DispatchResult), String> {
    let size = PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 };
    let tab_id = pty
        .create_with_app(app.clone(), size, Some(std::path::PathBuf::from(project_dir)), None)
        .map_err(|e| e.to_string())?;

    if let Err(e) = pty.write(&tab_id, format!("{claude_command}\r").as_bytes()) {
        let _ = pty.close(&tab_id);
        return Err(e.to_string());
    }
    if let Err(e) = app.emit(
        "mcp-coordination-tab-spawned",
        TabSpawnedEvent { session_id: tab_id.clone(), command: Some(claude_command.to_string()) },
    ) {
        eprintln!("emit mcp-coordination-tab-spawned failed: {e}");
    }

    wait_until_settled(pty, &tab_id).await;
    let result = run_on_session(pty, &tab_id, prompt, request_done_marker).await?;
    Ok((tab_id, result))
}
```

`src-tauri/src/tasks/scheduler.rs`，`RealDispatcher::dispatch`（第 76-77 行附近）的呼叫點加上第 6 個參數：

```rust
        let (tab_id, disp) = dispatch::spawn_and_run(
            &self.app,
            &self.pty,
            &task.project_dir,
            &claude_cmd,
            &prompt,
            !task.interactive,
        )
        .await?;
```

（`!task.interactive`：自動任務要完成標記，互動任務不要——這就是這個任務的重點行為。）

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib tasks::dispatch 2>&1 | tail -30`
Expected: 通過。

Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`。

- [ ] **Step 5: clippy**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "tasks/dispatch.rs|tasks/scheduler.rs"`
Expected: 空。

- [ ] **Step 6: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/tasks/dispatch.rs src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): spawn_and_run takes request_done_marker, interactive tasks skip it"
```

---

### Task 3: `monitor.rs` — `WatchMode` + `WatchControl`，`scheduler.rs` 對應更新

**Files:**
- Modify: `src-tauri/src/tasks/monitor.rs`
- Modify: `src-tauri/src/tasks/scheduler.rs`

這是這份計畫裡最大的一個任務，因為 `monitor::watch` 的簽章改變會牽動 `scheduler.rs` 好幾個地方（`SchedulerHandle`/`RealDispatcher` 的 `cancels` 欄位型別、`cancel()`/新 `mark_done()` 方法、`RealDispatcher::dispatch` 呼叫 `watch()` 的地方），這些改動彼此依賴、要一起改才能編譯，所以放同一個任務。**`drain_once` 的兩階段派工邏輯不在這個任務——那是任務 4，這個任務只確保現有的派工/監看路徑接上新型別，行為不變（除了新增的 `WatchMode`/手動完成能力本身）。**

- [ ] **Step 1: 寫會失敗的測試——先鎖住新型別的行為**

`src-tauri/src/tasks/monitor.rs`，整個 `#[cfg(test)] mod tests` 區塊（119-221 行）換成下面這版——既有 5 個測試的 channel 型別跟著新簽章調整（`oneshot::channel()` 型別推導原本是 `()`，現在要明確是 `WatchControl`；`cancel_signal_yields_cancelled` 的 `tx.send(())` 要換成 `tx.send(WatchControl::Cancel)`；所有 `watch(...)` 呼叫加一個 `WatchMode::Auto` 參數，保持現有行為完全不變），並新增 5 個測試覆蓋 `WatchMode::Interactive`/`WatchControl::MarkDone`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::manager::PtyManager;
    use crate::pty::session::done_marker;
    use portable_pty::PtySize;
    use std::time::Duration;

    fn size() -> PtySize {
        PtySize { rows: 24, cols: 200, pixel_width: 0, pixel_height: 0 }
    }

    fn test_thresholds() -> Thresholds {
        Thresholds { quiet_stuck_ms: 60_000, poll_ms: 50, min_run_ms: 0 }
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn marker_in_output_yields_success() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let marker = done_marker(&tab);
        pty.write(&tab, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn nonzero_exit_yields_failed() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();

        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains('3'), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "PowerShell shell-integration writes OSC133 D;0 on its very first \
                  prompt unconditionally (no __aiterm_cmd_running guard, unlike the \
                  zsh/bash hooks), so last_exit_code is Some(0) at startup and watch \
                  returns Success before the stuck threshold — real-ConPTY quirk, \
                  tracked separately"
    )]
    async fn silence_past_the_threshold_yields_failed_stuck() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("expected Failed(stuck), got {other:?}"),
        }
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn a_stale_exit_code_from_before_watch_does_not_count_as_completion() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        pty.write(&tab, b"true\n").unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if pty.last_exit_code(&tab) == Some(0) { break; }
            assert!(tokio::time::Instant::now() < deadline, "shell never emitted D;0");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let thresholds = Thresholds { quiet_stuck_ms: 300, poll_ms: 50, min_run_ms: 200 };
        let outcome = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Auto).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains("卡住"), "{msg}"),
            other => panic!("stale Some(0) must not yield {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_signal_yields_cancelled() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }

    #[tokio::test]
    async fn mark_done_control_signal_yields_success() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        tx.send(WatchControl::MarkDone).unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Auto).await;
        assert!(matches!(outcome, TaskOutcome::Success), "{outcome:?}");
    }

    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn interactive_mode_still_fails_on_nonzero_exit() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (_tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        pty.write(&tab, b"sh -c 'exit 3'\n").unwrap();
        let outcome = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Interactive).await;
        match outcome {
            TaskOutcome::Failed(msg) => assert!(msg.contains('3'), "{msg}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // Proves the marker/bell signal is genuinely ignored in Interactive
    // mode — not by waiting a fixed duration and assuming (a coincidence-
    // based test would pass just as well against broken code that merely
    // runs slowly), but by racing watch() against a real timeout: if it's
    // STILL RUNNING after a window well past what Auto mode would need to
    // return, that's a real, observed fact about the future's state, not a
    // guess. Then proves the loop is genuinely alive (not just slow) by
    // cancelling it and checking it reacts.
    #[tokio::test]
    #[cfg_attr(windows, ignore = "real-ConPTY test, broken on Windows CI — tracked separately")]
    async fn interactive_mode_does_not_treat_a_marker_as_completion() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        let marker = done_marker(&tab);
        pty.write(&tab, format!("printf '%s\\n' '{marker}'\n").as_bytes()).unwrap();

        let watch_fut = watch(&pty, &tab, rx, Baselines::default(), test_thresholds(), WatchMode::Interactive);
        tokio::pin!(watch_fut);

        let still_running = tokio::time::timeout(Duration::from_millis(500), &mut watch_fut).await.is_err();
        assert!(still_running, "watch() returned early in Interactive mode — marker signal was not ignored");

        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch_fut.await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }

    // Same non-coincidental-timeout technique as the marker test above, for
    // the 120s-stuck signal.
    #[tokio::test]
    async fn interactive_mode_does_not_time_out_on_silence() {
        let pty = PtyManager::new();
        let tab = pty.create_with_callback(size(), |_| {}).unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<WatchControl>();
        // Never write anything — session silent from the start.
        let thresholds = Thresholds { quiet_stuck_ms: 200, poll_ms: 50, min_run_ms: 100 };

        let watch_fut = watch(&pty, &tab, rx, Baselines::default(), thresholds, WatchMode::Interactive);
        tokio::pin!(watch_fut);

        let still_running = tokio::time::timeout(Duration::from_millis(600), &mut watch_fut).await.is_err();
        assert!(still_running, "watch() returned early in Interactive mode — stuck-timeout signal was not ignored");

        tx.send(WatchControl::Cancel).unwrap();
        let outcome = watch_fut.await;
        assert!(matches!(outcome, TaskOutcome::Cancelled), "{outcome:?}");
    }
}
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib tasks::monitor 2>&1 | tail -40`
Expected: 編譯錯誤——`WatchControl`/`WatchMode` 還不存在，`watch()` 簽章不符。

- [ ] **Step 3: 實作 `monitor.rs`**

`src-tauri/src/tasks/monitor.rs`，在 `TaskOutcome` 定義（17-39 行）之後、`Baselines`（41-48 行）之前，加：

```rust
/// External control signal for a running `watch()`. Sent through the same
/// oneshot channel that used to only carry cancellation — `SchedulerHandle`
/// exposes `cancel()`/`mark_done()` as two thin wrappers over one `send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchControl {
    Cancel,
    MarkDone,
}

/// Which soft completion signals `watch()` trusts. Both modes still trust
/// the hard signal — the Claude Code process actually exiting (③④) — since
/// that can't be a false positive from mid-conversation chatter the way
/// bell/marker/silence can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// bell/marker → success, 120s silence → stuck-failed. Current
    /// behavior for tasks dispatched to run unattended.
    Auto,
    /// A human is expected to be chatting in this tab — bell fires on
    /// every reply, and thinking for well over 120s is normal, so both of
    /// those signals would misfire constantly. Completion is signalled
    /// externally instead (`WatchControl::MarkDone`).
    Interactive,
}
```

`watch` 函式（67-117 行）簽章跟訊號②⑤的判斷式改成：

```rust
pub async fn watch(
    pty: &PtyManager,
    tab_id: &str,
    mut control: oneshot::Receiver<WatchControl>,
    baselines: Baselines,
    thresholds: Thresholds,
    mode: WatchMode,
) -> TaskOutcome {
    let start = tokio::time::Instant::now();
    let exit_baseline = pty.last_exit_code(tab_id);
    loop {
        // 1. external control signal
        match control.try_recv() {
            Ok(WatchControl::Cancel) => return TaskOutcome::Cancelled,
            Ok(WatchControl::MarkDone) => return TaskOutcome::Success,
            Err(oneshot::error::TryRecvError::Closed) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Empty) => {}
        }

        // Session gone (tab closed out from under us) — treat as cancelled.
        let Some(bell) = pty.bell_count(tab_id) else {
            return TaskOutcome::Cancelled;
        };
        let marker = pty.marker_count(tab_id).unwrap_or(0);

        // 2. reply signal — Auto mode only (see WatchMode::Interactive's doc).
        if mode == WatchMode::Auto && (marker > baselines.marker || bell > baselines.bell) {
            return TaskOutcome::Success;
        }

        // 3/4. process exited *since we started watching* — both modes, this
        // is a hard signal either way.
        let current_exit = pty.last_exit_code(tab_id);
        if current_exit != exit_baseline {
            if let Some(code) = current_exit {
                if code != 0 {
                    return TaskOutcome::Failed(format!("claude 以 exit code {code} 結束"));
                }
                return TaskOutcome::Success;
            }
        }

        // 5. stuck — Auto mode only.
        let ran_ms = start.elapsed().as_millis() as u64;
        let quiet_ms = pty.ms_since_output(tab_id).unwrap_or(0);
        if mode == WatchMode::Auto && ran_ms >= thresholds.min_run_ms && quiet_ms >= thresholds.quiet_stuck_ms {
            return TaskOutcome::Failed("疑似卡住（120 秒無輸出）".to_string());
        }

        tokio::time::sleep(Duration::from_millis(thresholds.poll_ms)).await;
    }
}
```

也更新這個檔案最上方的模組文件註解（1-9 行），第 1 行改成也提到手動完成：

```rust
//! Watches one dispatched task's PTY session until it reaches a terminal
//! outcome. Signals, in priority order:
//!   1. control channel fires Cancel or MarkDone → Cancelled / Success
//!   2. fresh bell or done-marker observed (Auto mode only) → Success
//!   3. OSC133 non-zero exit code          → Failed("claude 以 exit code N 結束")
//!   4. OSC133 exit code 0                 → Success (claude exited cleanly)
//!   5. no output for `quiet_stuck_ms` (Auto mode only) → Failed("疑似卡住（120 秒無輸出）")
//!
//! Interactive mode (`WatchMode::Interactive`) skips signals ②⑤ — see that
//! variant's doc comment for why. Reuses `PtyManager`'s existing per-session
//! counters — no new protocol.
```

- [ ] **Step 4: 更新 `scheduler.rs` 讓整個 crate 繼續編譯**

`src-tauri/src/tasks/scheduler.rs`：

`RealDispatcher` 的 `cancels` 欄位（61 行）改型別：
```rust
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
```

`RealDispatcher::dispatch`（65-111 行）：`oneshot::channel()`（83 行）改成明確型別，`monitor::watch(...)` 呼叫（95 行）加 `watch_mode` 參數：

```rust
        let (cancel_tx, cancel_rx) = oneshot::channel::<monitor::WatchControl>();
        self.cancels.lock().insert(task.id.clone(), cancel_tx);

        let pool = db.pool.clone();
        let pty = self.pty.clone();
        let app = self.app.clone();
        let wake = self.wake.clone();
        let cancels = self.cancels.clone();
        let task_id = task.id.clone();
        let baselines = monitor::Baselines { bell: disp.bell_baseline, marker: disp.marker_baseline };
        let watch_mode = if task.interactive { monitor::WatchMode::Interactive } else { monitor::WatchMode::Auto };
        tauri::async_runtime::spawn(async move {
            let outcome = monitor::watch(
                &pty, &tab_id, cancel_rx, baselines, monitor::Thresholds::default(), watch_mode,
            ).await;
            let transcript = write_transcript(&pty, &task_id, &tab_id);
            let _ = store::finish_task(
                &pool, &task_id, outcome.as_str(), outcome.error_message(), transcript.as_deref(),
            ).await;
            cancels.lock().remove(&task_id);
            let _ = app.emit("tasks-updated", ());
            wake.notify_one();
        });
        Ok(())
```

`SchedulerHandle`（164-184 行）——`cancels` 欄位改型別，`cancel()` 改成走共用的 `send`，新增 `mark_done()`：

```rust
#[derive(Clone)]
pub struct SchedulerHandle {
    pub wake: Arc<Notify>,
    pub cancels: Arc<parking_lot::Mutex<HashMap<String, oneshot::Sender<monitor::WatchControl>>>>,
}

impl SchedulerHandle {
    pub fn poke(&self) {
        self.wake.notify_one();
    }
    /// Fire the cancel signal for a running task, if present. Returns true
    /// if a watch was signalled.
    pub fn cancel(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::Cancel)
    }
    /// Fire the manual-completion signal for a running task, if present.
    /// Returns true if a watch was signalled — see `commands::tasks::tasks_mark_done`
    /// for the fallback when this returns false (no active watch to signal).
    pub fn mark_done(&self, task_id: &str) -> bool {
        self.send(task_id, monitor::WatchControl::MarkDone)
    }
    fn send(&self, task_id: &str, msg: monitor::WatchControl) -> bool {
        if let Some(tx) = self.cancels.lock().remove(task_id) {
            let _ = tx.send(msg);
            true
        } else {
            false
        }
    }
}
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib tasks::monitor 2>&1 | tail -50`
Expected: 全部通過（既有 5 個改寫過的 + 新增 5 個 = 10 個，Windows CI 上有幾個因為既有理由被 ignore，不影響本機）。

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
Expected: 乾淨編譯。

Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`。

- [ ] **Step 6: 幫 `SchedulerHandle` 補兩個直接單元測試**

`src-tauri/src/tasks/scheduler.rs` 的 `#[cfg(test)] mod tests`（234-299 行那個，`pick_next` 的測試群組）結尾加：

```rust
    #[test]
    fn mark_done_sends_the_control_signal_and_returns_true_when_a_watch_is_registered() {
        let cancels = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let handle = SchedulerHandle { wake: Arc::new(Notify::new()), cancels: cancels.clone() };
        let (tx, mut rx) = oneshot::channel::<monitor::WatchControl>();
        cancels.lock().insert("t1".to_string(), tx);
        assert!(handle.mark_done("t1"));
        assert!(matches!(rx.try_recv().unwrap(), monitor::WatchControl::MarkDone));
    }

    #[test]
    fn mark_done_returns_false_when_nothing_is_registered() {
        let handle = SchedulerHandle {
            wake: Arc::new(Notify::new()),
            cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        };
        assert!(!handle.mark_done("nope"));
    }
```

Run: `cd src-tauri && cargo test --lib tasks::scheduler::tests 2>&1 | tail -30`
Expected: 通過（既有 6 個 + 新增 2 個）。

- [ ] **Step 7: clippy**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "tasks/monitor.rs|tasks/scheduler.rs"`
Expected: 空。

- [ ] **Step 8: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/tasks/monitor.rs src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): WatchMode/WatchControl — interactive tasks skip bell/marker/stuck signals, gain manual mark-done"
```

---

### Task 4: `scheduler.rs` — 互動任務繞過併發上限

**Files:**
- Modify: `src-tauri/src/tasks/scheduler.rs`

`pick_next` 這個純函式本體、簽章、既有 6 個測試完全不動。這個任務只改 `drain_once`。

- [ ] **Step 1: 寫會失敗的測試**

在 `src-tauri/src/tasks/scheduler.rs` 的 `#[cfg(test)] mod loop_tests`（`drain_once_stops_at_a_solo_head_while_something_runs` 之後）加：

```rust
    #[tokio::test]
    async fn drain_once_dispatches_interactive_cards_even_when_the_auto_lane_is_full() {
        let db = mem_db().await;
        // Seed one auto card already running — fills a cap of 1.
        let running_id = store::create_task(&db.pool, "already-running", "", "/r", true, false).await.unwrap();
        store::move_task(&db.pool, &running_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&db.pool, &running_id, "tab-already").await.unwrap();

        // A second auto card, queued — cap=1 means this must stay queued.
        let blocked_id = store::create_task(&db.pool, "blocked-auto", "", "/r", true, false).await.unwrap();
        store::move_task(&db.pool, &blocked_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        // An interactive card, queued — must dispatch anyway, cap or no cap.
        let interactive_id = store::create_task(&db.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&db.pool, &interactive_id, store::STATUS_QUEUED, 3.0).await.unwrap();

        struct RecordingDispatcher {
            dispatched: std::sync::Mutex<Vec<String>>,
        }
        #[async_trait::async_trait]
        impl Dispatcher for RecordingDispatcher {
            async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
                self.dispatched.lock().unwrap().push(task.title.clone());
                store::mark_dispatched(&db.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        let dispatcher = RecordingDispatcher { dispatched: std::sync::Mutex::new(vec![]) };
        drain_once(&db, &dispatcher, 1).await;

        assert_eq!(*dispatcher.dispatched.lock().unwrap(), vec!["chat".to_string()]);

        let all = store::list_tasks(&db.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("already-running"), "running");
        assert_eq!(status_of("blocked-auto"), "queued"); // cap=1 respected for the auto lane
        assert_eq!(status_of("chat"), "running"); // interactive bypassed the cap entirely
    }

    #[tokio::test]
    async fn drain_once_does_not_let_a_running_interactive_card_count_against_the_auto_cap() {
        let db = mem_db().await;
        // An interactive card already running should NOT occupy the auto
        // lane's capacity slot — an auto card queued behind it must still
        // start immediately even at cap=1.
        let chat_id = store::create_task(&db.pool, "chat", "", "/r", true, true).await.unwrap();
        store::move_task(&db.pool, &chat_id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&db.pool, &chat_id, "tab-chat").await.unwrap();

        let auto_id = store::create_task(&db.pool, "auto", "", "/r", true, false).await.unwrap();
        store::move_task(&db.pool, &auto_id, store::STATUS_QUEUED, 2.0).await.unwrap();

        struct FakeDispatcher;
        #[async_trait::async_trait]
        impl Dispatcher for FakeDispatcher {
            async fn dispatch(&self, db: &TasksDb, task: &TaskRow) -> Result<(), String> {
                store::mark_dispatched(&db.pool, &task.id, &format!("fake-{}", task.id))
                    .await
                    .map_err(|e| e.to_string())
            }
        }
        drain_once(&db, &FakeDispatcher, 1).await;

        let all = store::list_tasks(&db.pool).await.unwrap();
        let status_of = |t: &str| all.iter().find(|r| r.title == t).unwrap().status.clone();
        assert_eq!(status_of("chat"), "running");
        assert_eq!(status_of("auto"), "running"); // not blocked by the already-running interactive card
    }
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib tasks::scheduler::loop_tests::drain_once_dispatches_interactive 2>&1 | tail -30`
Run: `cd src-tauri && cargo test --lib tasks::scheduler::loop_tests::drain_once_does_not_let_a_running_interactive 2>&1 | tail -30`
Expected: 兩個都 FAIL——目前 `drain_once` 沒有任何互動卡片特殊處理，`chat` 會跟 `blocked-auto` 一樣卡在併發上限後面。

- [ ] **Step 3: 實作**

`src-tauri/src/tasks/scheduler.rs`，`drain_once`（134-160 行）整段換成：

```rust
/// Promote as many queued cards as the rules allow, right now. Shared by the
/// loop and by tests.
pub async fn drain_once(db: &TasksDb, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    // Interactive cards bypass the concurrency cap and the solo-blocking
    // rule entirely — dispatch every queued one, unconditionally, first.
    loop {
        let queued = match store::list_by_status(&db.pool, store::STATUS_QUEUED).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("scheduler list queued (interactive pass): {e}");
                return;
            }
        };
        let Some(next) = queued.into_iter().find(|t| t.interactive) else {
            break;
        };
        if let Err(e) = dispatcher.dispatch(db, &next).await {
            eprintln!("dispatch {} failed: {e}", next.id);
            let _ = store::mark_dispatched(&db.pool, &next.id, "").await;
            let _ = store::finish_task(&db.pool, &next.id, "failed", Some(&e), None).await;
        }
    }

    // Existing logic, unchanged — but `running`/`queued` here only ever see
    // non-interactive cards, so they neither count against max_concurrent
    // nor get blocked by (or block) anything in the interactive lane above.
    loop {
        let running = match store::list_by_status(&db.pool, store::STATUS_RUNNING).await {
            Ok(r) => r.into_iter().filter(|t| !t.interactive).collect::<Vec<_>>(),
            Err(e) => {
                eprintln!("scheduler list running: {e}");
                return;
            }
        };
        let queued = match store::list_by_status(&db.pool, store::STATUS_QUEUED).await {
            Ok(q) => q.into_iter().filter(|t| !t.interactive).collect::<Vec<_>>(),
            Err(e) => {
                eprintln!("scheduler list queued: {e}");
                return;
            }
        };
        let Some(next) = pick_next(&running, &queued, max_concurrent) else {
            return;
        };
        let next = next.clone();
        if let Err(e) = dispatcher.dispatch(db, &next).await {
            eprintln!("dispatch {} failed: {e}", next.id);
            let _ = store::mark_dispatched(&db.pool, &next.id, "").await;
            let _ = store::finish_task(&db.pool, &next.id, "failed", Some(&e), None).await;
        }
    }
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib tasks::scheduler 2>&1 | tail -50`
Expected: 全部通過（既有的 `pick_next`/`loop_tests` 全部 + 新增 2 個）。

Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`。

- [ ] **Step 5: clippy**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep "tasks/scheduler.rs"`
Expected: 空。

- [ ] **Step 6: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/tasks/scheduler.rs
git commit -m "feat(tasks): interactive cards bypass the concurrency cap entirely"
```

---

### Task 5: `commands/tasks.rs` — 建卡片帶 `interactive`、新指令 `tasks_mark_done`

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/task_board.rs`

- [ ] **Step 1: 寫會失敗的測試**

在 `src-tauri/src/commands/tasks.rs` 檔案結尾（`save_transcript_tests` 模組之後）加一個新模組：

```rust
#[cfg(test)]
mod mark_done_tests {
    use super::*;
    use crate::tasks::monitor::WatchControl;
    use crate::tasks::scheduler::SchedulerHandle;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{oneshot, Notify};

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    fn empty_scheduler() -> SchedulerHandle {
        SchedulerHandle {
            wake: Arc::new(Notify::new()),
            cancels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }

    // Exercises the exact same logic tasks_mark_done's body runs — get_task,
    // the status/interactive guard, then scheduler.mark_done() — without
    // needing a Tauri State<'_, TasksDb>/AppHandle extractor (same
    // limitation save_transcript_tests documents above tasks_mark_done).
    #[tokio::test]
    async fn signals_the_active_watch_for_a_running_interactive_task() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

        let scheduler = empty_scheduler();
        let (tx, mut rx) = oneshot::channel::<WatchControl>();
        scheduler.cancels.lock().insert(id.clone(), tx);

        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_RUNNING);
        assert!(row.interactive);
        assert!(scheduler.mark_done(&id));
        assert!(matches!(rx.try_recv().unwrap(), WatchControl::MarkDone));
    }

    #[tokio::test]
    async fn a_non_running_task_fails_the_guard_tasks_mark_done_checks() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        // Still planning — never dispatched.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_ne!(row.status, store::STATUS_RUNNING);
    }

    #[tokio::test]
    async fn a_non_interactive_task_fails_the_guard_tasks_mark_done_checks() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, false).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, store::STATUS_RUNNING);
        assert!(!row.interactive);
    }

    #[tokio::test]
    async fn falls_back_to_finishing_directly_when_there_is_no_active_watch() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true, true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

        let scheduler = empty_scheduler(); // no cancels entry registered
        assert!(!scheduler.mark_done(&id));

        // tasks_mark_done's fallback path when mark_done() returns false —
        // mirrors tasks_stop's own fallback (finish it directly).
        store::finish_task(&pool, &id, "success", None, None).await.unwrap();
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert_eq!(row.status, "done");
        assert_eq!(row.outcome.as_deref(), Some("success"));
    }
}
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `cd src-tauri && cargo test --lib commands::tasks::mark_done_tests 2>&1 | tail -40`
Expected: 編譯錯誤——`store::create_task` 現在需要 5 個參數（這個檔案裡的呼叫還是 4 個），`SchedulerHandle`/`WatchControl` 匯入路徑目前正確但這個模組本身還沒建過。先確認錯誤是「還沒加 `, true`/`, false`」這類，不是別的。

- [ ] **Step 3: `CreateArgs`/`UpdateArgs` 加欄位，`tasks_create`/`tasks_update` 接上**

`src-tauri/src/commands/tasks.rs`，`CreateArgs`（45-50 行）加欄位：

```rust
#[derive(Deserialize)]
pub struct CreateArgs {
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
}
```

`tasks_create` 指令本體（52-69 行附近）的 `store::create_task` 呼叫，把任務 1 暫時寫死的 `false` 換成真正的欄位：

```rust
    let id = store::create_task(
        &db.pool,
        &args.title,
        &args.body,
        &args.project_dir,
        args.parallel_ok,
        args.interactive,
    )
    .await
    .map_err(|e| e.to_string())?;
```

`UpdateArgs`（71-78 行）加欄位：

```rust
#[derive(Deserialize)]
pub struct UpdateArgs {
    pub id: String,
    pub title: String,
    pub body: String,
    pub project_dir: String,
    pub parallel_ok: bool,
    pub interactive: bool,
}
```

`tasks_update`（80-一百多行）在 `store::set_parallel_ok(...)` 呼叫旁邊加一行：

```rust
    store::set_parallel_ok(&db.pool, &args.id, args.parallel_ok)
        .await
        .map_err(|e| e.to_string())?;
    store::set_interactive(&db.pool, &args.id, args.interactive)
        .await
        .map_err(|e| e.to_string())?;
```

（這兩行都在 `edit_allowed(&row.status)` 檢查之前——沿用 `parallel_ok` 既有的行為，不是這次改動範圍。實務上 `TaskEditorDialog` 只在卡片是 `planning` 狀態時才會被打開，所以這個欄位實際上永遠只在可編輯期間被寫入。）

- [ ] **Step 4: 新增 `tasks_mark_done` 指令**

在 `tasks_stop`（132-157 行）之後加：

```rust
#[tauri::command]
pub async fn tasks_mark_done(
    id: String,
    db: State<'_, TasksDb>,
    app: AppHandle,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if row.status != store::STATUS_RUNNING || !row.interactive {
        return Err("only a running interactive task can be marked done".to_string());
    }
    if !scheduler.mark_done(&id) {
        // No active watch to signal — e.g. it just finished on its own via
        // an exit-code signal in the moment between the frontend rendering
        // the button and the click landing. Finish it directly, mirroring
        // tasks_stop's own fallback for the equivalent race.
        store::finish_task(&db.pool, &id, "success", None, row.transcript_path.as_deref())
            .await
            .map_err(|e| e.to_string())?;
        emit_updated(&app);
    }
    Ok(())
}
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib commands::tasks 2>&1 | tail -50`
Expected: 全部通過（既有 + 新增 4 個 `mark_done_tests`）。

- [ ] **Step 6: 補一個 `tests/task_board.rs` 整合測試**

在 `src-tauri/tests/task_board.rs` 結尾加：

```rust
#[tokio::test]
async fn interactive_task_created_via_create_task_round_trips_through_list() {
    let db = mem_db().await;
    let id = store::create_task(&db.pool, "chat with claude", "", "/r", true, true)
        .await
        .unwrap();
    let row = store::get_task(&db.pool, &id).await.unwrap().unwrap();
    assert!(row.interactive);
}
```

（在寫這個之前，先確認 `mem_db()` 這個 helper 在這個檔案裡的真實名稱/引入方式——跟前面 task-board 相關計畫遇到的情況一樣，用檔案裡實際的慣例，不要照抄這裡的名稱。）

Run: `cd src-tauri && cargo test --test task_board interactive_task 2>&1 | tail -20`
Expected: 通過。

- [ ] **Step 7: 註冊 `tasks_mark_done` 到 `lib.rs`**

`src-tauri/src/lib.rs`，`tasks::{...}` 匯入清單（107-110 行附近），加 `tasks_mark_done`：

```rust
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_clone, tasks_read_transcript,
        tasks_save_transcript, tasks_mark_done,
    },
```

`generate_handler!` 巨集裡，`tasks_stop,`（約 554 行）之後加一行：

```rust
            tasks_mark_done,
```

- [ ] **Step 8: 建置確認 + 全套測試**

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
Expected: 乾淨編譯。

Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`。

- [ ] **Step 9: clippy**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "commands/tasks.rs|tests/task_board.rs"`
Expected: 空。

- [ ] **Step 10: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs src-tauri/tests/task_board.rs
git commit -m "feat(tasks): interactive flag on create/update, tasks_mark_done command"
```

---

### Task 6: 前端 `src/ipc/tasks.ts`

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/ipc/tasks.test.ts`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/ipc/tasks.test.ts`，於現有測試風格旁邊加：

```ts
it("markTaskDone forwards the id as a bare param", async () => {
  vi.mocked(invoke).mockResolvedValue(undefined);
  await markTaskDone("id1");
  expect(invoke).toHaveBeenCalledWith("tasks_mark_done", { id: "id1" });
});

it("createTask forwards the interactive flag", async () => {
  vi.mocked(invoke).mockResolvedValue("new-id");
  await createTask({ title: "t", body: "b", project_dir: "/r", parallel_ok: true, interactive: true });
  expect(invoke).toHaveBeenCalledWith(
    "tasks_create",
    { args: { title: "t", body: "b", project_dir: "/r", parallel_ok: true, interactive: true } },
  );
});
```

（先讀這個檔案目前實際的 import 清單/既有測試寫法，把 `markTaskDone`/`createTask` 加進最上方的 import；如果 `createTask` 已經有等價測試，改成擴充既有那個測試的斷言即可，不用重複建一個。）

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: FAIL——`markTaskDone` 不存在；`createTask` 的斷言少了 `interactive` 欄位。

- [ ] **Step 3: 實作**

`src/ipc/tasks.ts`：

`TaskRow` interface（16-31 行）加欄位（跟 `parallel_ok` 相鄰）：

```ts
export interface TaskRow {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  status: TaskStatus;
  parallel_ok: boolean;
  interactive: boolean;
  sort_order: number;
  outcome: TaskOutcome | null;
  tab_id: string | null;
  transcript_path: string | null;
  error_message: string | null;
  created_at: string;
  dispatched_at: number | null;
  finished_at: number | null;
}
```

`createTask`（46-51 行）跟 `updateTask`（55-61 行）的參數型別各加一個欄位：

```ts
export const createTask = (args: {
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
  interactive: boolean;
}): Promise<string> => invoke("tasks_create", { args });

export const updateTask = (args: {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
  interactive: boolean;
}): Promise<void> => invoke("tasks_update", { args });
```

在 `stopTask`（66 行）之後加：

```ts
export const markTaskDone = (id: string): Promise<void> => invoke("tasks_mark_done", { id });
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: 通過。

- [ ] **Step 5: tsc**

Run: `npx tsc -b`
Expected: 乾淨——**這一步會先炸開**，因為 `TaskEditorDialog.tsx`/`TaskCard.tsx`/`TaskBoard/index.tsx`/`TaskBoard/index.test.tsx` 這幾個檔案目前建構 `TaskRow`/呼叫 `createTask`/`updateTask` 的地方都還沒加 `interactive`，型別會對不上。**這是預期的**——這幾個檔案是任務 8/9/10 的範圍，這個任務先讓 `ipc/tasks.ts` 本身跟它自己的測試過關即可，不要在這個任務裡順手改別的檔案。`tsc -b` 這一步只需要確認 `src/ipc/tasks.ts`/`src/ipc/tasks.test.ts` 這兩個檔案自己沒有型別錯誤：

Run: `npx tsc --noEmit src/ipc/tasks.ts src/ipc/tasks.test.ts 2>&1 | grep -v "TaskEditorDialog\|TaskCard\|TaskBoard/index"`
Expected: 空（這個指令會列出很多下游檔案的型別錯誤，用 grep 濾掉那些——它們屬於後面的任務）。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/tasks.ts src/ipc/tasks.test.ts
git commit -m "feat(tasks): interactive field + markTaskDone ipc wrapper"
```

---

### Task 7: 前端 `src/lib/i18n.ts` — 新增字串

**Files:**
- Modify: `src/lib/i18n.ts`

這個任務單純加字串，沒有邏輯，不需要 TDD 循環——但要在兩個語系區塊都加，位置對齊既有的 `board_card_*`/`board_action_*` 分組。

- [ ] **Step 1: 找到現有位置**

Run: `grep -n "board_card_parallel:\|board_card_solo_hint:\|board_action_stop:" src/lib/i18n.ts`

會看到 zh-TW 區塊（約 41-58 行）跟 en 區塊（約 1435-1452 行）各自的對應位置。

- [ ] **Step 2: zh-TW 區塊加字串**

在 `board_card_parallel: "可與其他任務並行",` 之前加：

```ts
    board_card_interactive: "互動模式",
    board_card_interactive_hint: "任務會需要你親自跟 Claude Code 對話，系統不會自動判定完成，也不受併發上限限制",
```

在 `board_action_stop: "停止",` 之後加：

```ts
    board_action_mark_done: "標記完成",
```

找到 `board_running_hint`（約 58 行）附近，加：

```ts
    board_badge_interactive: "互動",
```

- [ ] **Step 3: en 區塊加對應字串**

在 `board_card_parallel: "Can run alongside other tasks",` 之前加：

```ts
    board_card_interactive: "Interactive mode",
    board_card_interactive_hint: "This task needs you to chat with Claude Code directly — the system won't auto-detect completion or count it against the concurrency cap",
```

在 `board_action_stop: "Stop",` 之後加：

```ts
    board_action_mark_done: "Mark Done",
```

找到 `board_running_hint`（約 1452 行）附近，加：

```ts
    board_badge_interactive: "Interactive",
```

- [ ] **Step 4: 確認格式一致、tsc 過**

Run: `npx tsc -b 2>&1 | grep "i18n.ts"`
Expected: 空（這個檔案本身的型別是 `Record<string, string>` 之類的鬆散結構，多加 key 不會壞——如果專案的 i18n 型別是嚴格的兩語系欄位對齊檢查，這一步會抓到漏加哪一邊）。

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(tasks): i18n strings for interactive mode"
```

---

### Task 8: `TaskEditorDialog.tsx` — 互動模式開關

**Files:**
- Modify: `src/components/TaskBoard/TaskEditorDialog.tsx`
- Modify: `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/components/TaskBoard/index.test.tsx`：

在 `vi.mock("../../ipc/tasks", ...)` 區塊裡確認/補上 `markTaskDone: vi.fn().mockResolvedValue(undefined),`（若已存在跳過——這個任務不用它，但下一個任務會，先加起來避免遺漏）。

`card()` helper（35-40 行）的預設物件加一個欄位：

```ts
const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "", dispatched_at: null,
  finished_at: null, attachments: [],
  ...over,
});
```

在 `"new-card dialog creates a task with the typed fields"`（233-248 行）測試之後加兩個新測試：

```ts
  it("new-card dialog: checking interactive mode hides the parallel toggle and is sent to createTask", async () => {
    const { createTask } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Chat task");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");

    expect(screen.getByText(/可與其他任務並行|Can run alongside other tasks/)).toBeInTheDocument();
    await user.click(screen.getByLabelText(/互動模式|Interactive mode/));
    expect(screen.queryByText(/可與其他任務並行|Can run alongside other tasks/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Chat task", project_dir: "/repo", interactive: true }),
      ),
    );
  });

  it("new-card dialog defaults interactive to false when left unchecked", async () => {
    const { createTask } = await import("../../ipc/tasks");
    vi.mocked(createTask).mockResolvedValue("id-new");
    view();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
    await user.type(screen.getByLabelText(/標題|Title/), "Auto task");
    await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");
    await user.click(screen.getByRole("button", { name: /^儲存$|^Save$/ }));
    await waitFor(() =>
      expect(createTask).toHaveBeenCalledWith(expect.objectContaining({ interactive: false })),
    );
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -60`
Expected: 新增的 2 個測試 FAIL（`screen.getByLabelText(/互動模式|Interactive mode/)` 找不到元素）；既有測試維持 PASS（`card()` 多了一個欄位不影響既有斷言）。

- [ ] **Step 3: 實作**

`src/components/TaskBoard/TaskEditorDialog.tsx`：

在 `const [parallelOk, setParallelOk] = useState(card?.parallel_ok ?? true);`（30 行）之後加：

```tsx
  const [interactive, setInteractive] = useState(card?.interactive ?? false);
```

`save()`（59-76 行）的兩個 ipc 呼叫都加上這個欄位：

```tsx
      if (isEdit) {
        await updateTask({ id: card.id, title, body, project_dir: dir, parallel_ok: parallelOk, interactive });
      } else {
        const newId = await createTask({ title, body, project_dir: dir, parallel_ok: parallelOk, interactive });
```

「並行/單獨執行」那組 checkbox+hint（120-129 行）之前加一個新 checkbox，並把並行那組包進條件渲染：

```tsx
        <label className="task-field task-field--checkbox">
          <input
            type="checkbox"
            className="task-checkbox"
            checked={interactive}
            onChange={(e) => setInteractive(e.target.checked)}
          />
          <span>{t.board_card_interactive}</span>
        </label>
        <p className="task-field-hint">{t.board_card_interactive_hint}</p>

        {!interactive && (
          <>
            <label className="task-field task-field--checkbox">
              <input
                type="checkbox"
                className="task-checkbox"
                checked={parallelOk}
                onChange={(e) => setParallelOk(e.target.checked)}
              />
              <span>{t.board_card_parallel}</span>
            </label>
            <p className="task-field-hint">{t.board_card_solo_hint}</p>
          </>
        )}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -60`
Expected: 全部通過。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b 2>&1 | grep -E "TaskEditorDialog|index.test.tsx"`
Expected: 空（`TaskCard.tsx`/`TaskBoard/index.tsx` 這時候還沒加 `interactive`，會有型別錯誤——那是任務 9/10 的範圍，這裡只確認這個任務自己改的檔案沒問題）。

Run: `npx eslint src/components/TaskBoard/TaskEditorDialog.tsx`
Expected: 空。

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(tasks): interactive mode toggle in the create/edit dialog"
```

---

### Task 9: `TaskCard.tsx` — badge + 標記完成按鈕

**Files:**
- Modify: `src/components/TaskBoard/TaskCard.tsx`
- Modify: `src/components/TaskBoard/index.css`
- Modify: `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/components/TaskBoard/index.test.tsx`，於 `"running card shows Stop, and Stop calls stopTask"`（183-191 行）之後加：

```ts
  it("interactive running card shows the interactive badge and a Mark Done button that calls markTaskDone", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    const user = userEvent.setup();
    await screen.findByText("Chatting");
    expect(screen.getByText(/互動|Interactive/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /標記完成|Mark Done/ }));
    expect(markTaskDone).toHaveBeenCalledWith("r");
  });

  it("non-interactive running card has no Mark Done button", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Auto running", status: "running", tab_id: "tab-1", interactive: false }),
    ]);
    view();
    await screen.findByText("Auto running");
    expect(screen.queryByRole("button", { name: /標記完成|Mark Done/ })).not.toBeInTheDocument();
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -40`
Expected: 第一個測試 FAIL（badge/按鈕都不存在）；第二個測試會 PASS（因為按鈕本來就不存在）——這是預期的，第二個測試是防止未來誤把按鈕加到所有卡片上的回歸測試，不是這次要修的紅燈。

- [ ] **Step 3: 實作**

`src/components/TaskBoard/TaskCard.tsx`：

`import` 清單加 `markTaskDone`：

```tsx
import { cloneTask, deleteTask, markTaskDone, stopTask, type TaskWithAttachments } from "../../ipc/tasks";
```

在 `{!card.parallel_ok && <div className="task-card-meta">⚑ {t.board_card_solo_hint}</div>}`（59 行）之後加 badge：

```tsx
      {card.interactive && (
        <div className="task-badge task-badge--interactive">👤 {t.board_badge_interactive}</div>
      )}
```

`running` 狀態的動作區（77-84 行）多一個按鈕，只在 `card.interactive` 時出現：

```tsx
        {card.status === "running" && (
          <>
            <button disabled={busy} onClick={() => void run(() => stopTask(card.id))}>
              {t.board_action_stop}
            </button>
            {card.interactive && (
              <button disabled={busy} onClick={() => void run(() => markTaskDone(card.id))}>
                {t.board_action_mark_done}
              </button>
            )}
            {card.tab_id && <button onClick={openTab}>{t.board_action_open_tab}</button>}
          </>
        )}
```

`src/components/TaskBoard/index.css`，在既有的 `.task-badge--cancelled` 那行（約 48 行）之後加一個新樣式（用中性色，不是成功/失敗語意色）：

```css
.task-badge--interactive { background: #1f2a3a; color: #b6d4f0; }
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -60`
Expected: 全部通過。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b 2>&1 | grep -E "TaskCard.tsx|index.test.tsx"`
Expected: 空（`TaskBoard/index.tsx` 還沒加 `interactive` 相關欄位到它自己組的物件——那是任務 10）。

Run: `npx eslint src/components/TaskBoard/TaskCard.tsx`
Expected: 空。

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/TaskCard.tsx src/components/TaskBoard/index.css src/components/TaskBoard/index.test.tsx
git commit -m "feat(tasks): interactive badge + Mark Done button on TaskCard"
```

---

### Task 10: `TaskBoard/index.tsx` — 拖曳「執行中→已完成」

**Files:**
- Modify: `src/components/TaskBoard/index.tsx`
- Modify: `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/components/TaskBoard/index.test.tsx`，於 `"dropping a planning card on the queued column calls moveTask"`（96-115 行）之後加：

```ts
  it("dropping a running interactive card on the done column calls markTaskDone, not moveTask", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Chatting", status: "running", tab_id: "tab-1", interactive: true }),
    ]);
    view();
    const cardEl = await screen.findByText("Chatting");
    const doneCol = screen.getByTestId("column-done");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;
    expect(dragWrap).toBeTruthy();

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(doneCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      await waitFor(() => expect(markTaskDone).toHaveBeenCalledWith("r"));
      expect(moveTask).not.toHaveBeenCalled();
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });

  it("dropping a running NON-interactive card on the done column does nothing", async () => {
    const { markTaskDone } = await import("../../ipc/tasks");
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "r", title: "Auto running", status: "running", tab_id: "tab-1", interactive: false }),
    ]);
    view();
    const cardEl = await screen.findByText("Auto running");
    const doneCol = screen.getByTestId("column-done");
    const dragWrap = cardEl.closest("[data-task-drag-id]") as HTMLElement;

    const originalElementFromPoint = document.elementFromPoint;
    document.elementFromPoint = vi.fn().mockReturnValue(doneCol);
    try {
      const { fireEvent } = await import("@testing-library/react");
      fireEvent.mouseDown(dragWrap, { clientX: 100, clientY: 100, button: 0 });
      fireEvent.mouseMove(window, { clientX: 100, clientY: 120 });
      fireEvent.mouseUp(window, { clientX: 100, clientY: 120 });
      // Not draggable at all — mousedown shouldn't even arm a drag for a
      // non-interactive running card, so neither call should ever fire.
      await new Promise((r) => setTimeout(r, 50));
      expect(markTaskDone).not.toHaveBeenCalled();
      expect(moveTask).not.toHaveBeenCalled();
    } finally {
      document.elementFromPoint = originalElementFromPoint;
    }
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -40`
Expected: 第一個測試 FAIL（`running`+`interactive` 卡片目前完全不可拖曳，`markTaskDone` 不會被呼叫）。第二個測試本來就會 PASS（現況本來就不可拖曳）——留著是為了鎖住「這個行為不能被下面的改動意外打開」。

- [ ] **Step 3: 實作**

`src/components/TaskBoard/index.tsx`：

`import` 清單加 `markTaskDone`：

```tsx
import {
  listTasks,
  markTaskDone,
  moveTask,
  onTasksUpdated,
  type TaskStatus,
  type TaskWithAttachments,
} from "../../ipc/tasks";
```

`handleDrop`（81-99 行）加一個新分支，放在既有 `legal` 判斷之前：

```tsx
  const handleDrop = useCallback(
    async (id: string, to: TaskStatus) => {
      const cardRow = tasks.find((x) => x.id === id);
      if (!cardRow || cardRow.status === to) return;

      if (cardRow.status === "running" && to === "done" && cardRow.interactive) {
        await markTaskDone(id);
        return; // finish flow (incl. the outcome badge) arrives via the
                 // existing tasks-updated listener, same as auto-completion.
      }

      const legal =
        (cardRow.status === "planning" && to === "queued") ||
        (cardRow.status === "queued" && to === "planning");
      if (!legal) return;
      const dest = tasks
        .filter((x) => x.status === to)
        .sort((a, b) => a.sort_order - b.sort_order);
      const sortOrder = dest.length ? dest[dest.length - 1].sort_order + 1 : 1;
      await moveTask(id, to, sortOrder);
      setTasks((prev) =>
        prev.map((x) => (x.id === id ? { ...x, status: to, sort_order: sortOrder } : x)),
      );
    },
    [tasks],
  );
```

`handleCardMouseDown`（148-152 行）的狀態守衛加一個例外：

```tsx
  const handleCardMouseDown = (e: ReactMouseEvent<HTMLDivElement>, cardRow: TaskWithAttachments) => {
    if (e.button !== 0) return;
    const draggable =
      cardRow.status === "planning" ||
      cardRow.status === "queued" ||
      (cardRow.status === "running" && cardRow.interactive);
    if (!draggable) return;
    dragRef.current = { id: cardRow.id, startX: e.clientX, startY: e.clientY, started: false };
  };
```

卡片渲染那段（164-185 行附近）的 `draggableCard` 判斷同步更新：

```tsx
            {byStatus(s).map((cardRow) => {
              const draggableCard =
                cardRow.status === "planning" ||
                cardRow.status === "queued" ||
                (cardRow.status === "running" && cardRow.interactive);
              const isDragging = draggingCardId === cardRow.id;
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx 2>&1 | tail -60`
Expected: 全部通過（既有全部 + 這個任務新增 2 個 + 前面兩個任務新增的 4 個 = 全綠）。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b`
Expected: 乾淨——這一步整條 `TaskBoard` 相關的型別鏈路應該全部接上了（任務 6-10 累積下來）。

Run: `npx eslint src/components/TaskBoard/`
Expected: 空。

- [ ] **Step 6: 廣泛回歸測試**

Run: `npm run test -- src/components/TaskBoard src/ipc/tasks.test.ts src/lib`
Expected: 全部通過。

- [ ] **Step 7: Commit**

```bash
git add src/components/TaskBoard/index.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(tasks): drag a running interactive card to done calls markTaskDone"
```

---

### Task 11: 驗證整輪

- [ ] **Step 1:** `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"` → `0`
- [ ] **Step 2:** `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "tasks/|commands/tasks.rs|tests/task_board.rs"` → 空
- [ ] **Step 3:** `npm run test 2>&1 | tail -8` → 全部通過
- [ ] **Step 4:** `npx tsc -b` → 乾淨
- [ ] **Step 5:** `npm run lint 2>&1 | grep -iE "taskboard|tasks/|i18n"` → 空（跟工作看板/新檔案相關的部分零錯誤；repo 既有的其他無關 lint 錯誤不算）
- [ ] **Step 6（手動驗證，需要真的裝 `claude`）：** `npm run tauri:dev`：
  1. 建立一張卡片，勾選「互動模式」，確認「並行/單獨執行」開關消失，存檔。
  2. 拖到「待執行」，確認**立刻**（不等併發名額）跳到「執行中」，分頁自動送出標題+內文當第一句話。
  3. 在那個分頁裡跟 Claude Code 來回聊個幾輪，確認：思考超過 120 秒卡片不會被判定失敗；每次它回覆卡片也不會自動跳到「已完成」。
  4. 點卡片上的「標記完成」按鈕，確認卡片變成「已完成」、標成功。
  5. 建第二張互動卡片，重複到「執行中」，這次改成用滑鼠把卡片拖到「已完成」欄位，確認一樣正常轉移。
  6. 同時建一張一般（非互動）卡片跟一張互動卡片都丟到「待執行」，把全域併發上限設成 1（設定頁），確認一般卡片乖乖排隊、互動卡片完全不受影響立刻開始。
  7. 確認「已完成」的互動任務打開「對話記錄」，內容也是乾淨版本（沿用今天稍早完成的對話記錄升級機制，這裡只是確認手動完成一樣有觸發它）。
- [ ] **Step 7:** 如果 Step 6 發現任何問題，修正後回到 Step 1 重跑整輪驗證，再 commit 修正。

---

## Self-Review

**Spec 覆蓋：**
- 建卡片時選、只能 planning 編輯 → Task 1（欄位）+ Task 8（UI，沿用既有「只有 planning 有編輯鈕」規則，沒有另外做開關）。✅
- 互動模式繞過併發上限 → Task 4（`drain_once` 兩階段）+ 對應測試明確驗證「auto 卡片被卡住時互動卡片照樣派工」跟「互動卡片在跑時不占用 auto 上限」兩個方向。✅
- 自動送出初始 prompt、不附加完成標記指示 → Task 2（`request_done_marker` 參數）。✅
- bell/marker/120 秒無輸出三個訊號在互動模式關掉、exit code 保留 → Task 3（`WatchMode`），測試用「race against timeout」技巧證明訊號真的被跳過，不是巧合。✅
- 按鈕 + 拖曳都能手動標記完成 → Task 5（`tasks_mark_done`）+ Task 9（按鈕）+ Task 10（拖曳）。✅
- 手動完成一律成功 → `tasks_mark_done`/`SchedulerHandle::mark_done` 走 `WatchControl::MarkDone` → `TaskOutcome::Success`，沒有讓使用者選的入口。✅
- 對話記錄升級機制不用改 → 整份計畫沒有任何任務碰 `transcriptUpgrade.ts`/`TaskBoard/index.tsx` 的 `justFinished` 判斷式本體，手動完成一樣會經過 `tasks-updated` → `refresh()` → 偵測到狀態變成 done → 自動觸發。✅
- 執行中卡片視覺 badge → Task 9。✅

**Placeholder 掃描：** 無 TBD/TODO，每個程式碼步驟都是完整可執行的內容（Task 2 的替代測試方案是唯一一處要求實作者做一個小判斷，但兩個選項都給了完整程式碼，不是空白待補）。

**型別一致性檢查：**
- `interactive: bool` 在 `TaskRow`（Task 1）→ `create_task`/`clone_task_fields`/`set_interactive`（Task 1）→ `CreateArgs`/`UpdateArgs`（Task 5）→ `tasks_mark_done` 的守衛條件（Task 5）→ 前端 `TaskRow`/`createTask`/`updateTask`（Task 6）→ `TaskEditorDialog` 的 `interactive`/`setInteractive`（Task 8）→ `TaskCard`/`TaskBoard/index.tsx` 讀 `card.interactive`（Task 9/10），全程欄位名稱一致。
- `WatchMode`/`WatchControl`（Task 3 定義）在 `RealDispatcher::dispatch`（Task 3 更新）、`SchedulerHandle::cancel`/`mark_done`（Task 3 定義）、`tasks_mark_done` 的測試（Task 5，import `crate::tasks::monitor::WatchControl`）之間用法一致。
- `spawn_and_run` 的 `request_done_marker: bool` 參數（Task 2 定義）跟 `RealDispatcher::dispatch` 的呼叫點（Task 2 更新為 `!task.interactive`）簽章相符。
- `markTaskDone(id: string): Promise<void>`（Task 6 定義）在 `TaskCard.tsx`（Task 9）、`TaskBoard/index.tsx`（Task 10）的用法一致。

## 相關

`docs/superpowers/specs/2026-09-03-task-board-interactive-mode-design.md`（這份計畫的設計依據）
`docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`（工作看板原始設計）
`docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md`（對話記錄升級機制，這次手動完成沿用不改）

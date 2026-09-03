# 工作看板「互動模式」— 設計

日期：2026-09-03
狀態：待使用者複審

## 問題

工作看板現有的自動完成偵測（`monitor::watch`）只認三種硬訊號：bell/完成標記 → 成功、120 秒無輸出 → 判定卡住失敗、Claude Code 進程 exit code → 成功/失敗。這套機制假設任務是「丟一句 prompt、等它自己做完」的單輪自動化工作。

有些任務天生需要多輪來回、或使用者一開始就知道要親自跟 Claude Code CLI 對話（不是丟著讓它自動跑）。這種任務套用現有偵測機制會被誤判：使用者思考超過 120 秒會被判定「卡住失敗」，Claude Code 每次回覆觸發的 bell 也會被誤判成「完成」而提早把卡片搬到已完成。

使用者確認：**這種情況在建立卡片的當下就會知道**（不是任務跑到一半才臨時決定要介入），所以可以在建卡片時就選好模式，走不同的流程。

## 現況調查

以下是這次設計會動到、或緊鄰的現有機制，都已實際讀過程式碼確認：

| 事實 | 位置 |
|---|---|
| `pick_next(running, queued, max_concurrent)` 是純函式，決定下一張要派工的卡片：solo 卡片跑起來會擋住所有後續、遵守全域併發上限、嚴格照 `sort_order` 排隊優先權 | `src-tauri/src/tasks/scheduler.rs:27-43`，已有 6 個單元測試 |
| `RealDispatcher::dispatch` 派工時：建 tab → 打字送出 prompt（`run_on_session(..., request_done_marker: true)` 目前寫死 `true`）→ `mark_dispatched` → spawn 一個背景 `monitor::watch` task，watch 結束後寫 transcript、`store::finish_task`、emit `tasks-updated`、清掉 cancel handle | `src-tauri/src/tasks/scheduler.rs:65-111` |
| `run_on_session(pty, tab_id, prompt, request_done_marker)` 已經是參數化的——`request_done_marker=false` 時完全跳過「等 bell → 附加完成標記指示」那段，只送 prompt 本身 | `src-tauri/src/tasks/dispatch.rs:98-138` |
| `spawn_and_run` 是 `run_on_session` 外面包一層「開 tab + emit `mcp-coordination-tab-spawned` + 等 settle」，目前呼叫 `run_on_session(..., true)` 寫死 `true`，沒有把這個參數往外開放 | `src-tauri/src/tasks/dispatch.rs:143-169`（呼叫點在第 167 行） |
| `monitor::watch` 的訊號判斷順序：① cancel channel（`oneshot::Receiver<()>`）② bell/marker → Success ③④ exit code 非零/零 → Failed/Success ⑤ 120 秒無輸出 → Failed(卡住)。`Baselines`/`Thresholds` 都是純資料，沒有「這次要不要判斷某個訊號」的開關 | `src-tauri/src/tasks/monitor.rs:67-117` |
| `SchedulerHandle.cancels: HashMap<task_id, oneshot::Sender<()>>`，`cancel(task_id)` 送出訊號、`tasks_stop` 指令呼叫它。這是唯一一條「外部介入正在跑的 watch」的管道 | `src-tauri/src/tasks/scheduler.rs:164-184` |
| `TaskRow`/`store::create_task`/schema 目前有 `parallel_ok: bool`，是這個功能最相近的既有前例——建卡片時選、只能在 `planning` 狀態編輯（一旦轉成 `queued` 之後 `TaskCard` 就不再顯示編輯按鈕）、`set_parallel_ok` 只在 planning 階段被呼叫 | `src-tauri/src/tasks/store.rs:23,56,68-75,198-204`；`src/components/TaskBoard/TaskCard.tsx:71-76`（只有 `status === "planning"` 才有編輯鈕） |
| `TaskEditorDialog` 的「並行/單獨執行」是一個 checkbox + hint 文字，`TaskCard` 用 `!card.parallel_ok` 顯示 `⚑` 提示 | `src/components/TaskBoard/TaskEditorDialog.tsx:120-129`；`src/components/TaskBoard/TaskCard.tsx:59` |
| `TaskBoard/index.tsx` 的 `handleDrop` 目前只認 `planning↔queued` 兩個方向合法，其餘一律忽略；拖曳呼叫的是 `moveTask(id, to, sortOrder)`，單純更新 `status`/`sort_order`，沒有任何「完成」相關的副作用 | `src/components/TaskBoard/index.tsx`（本次對話稍早已讀過完整檔案） |
| 卡片轉成 `done` 之後的對話記錄升級（`tryUpgradeTranscript`）完全是靠「偵測到狀態從非 `done` 變成 `done`」觸發，不管這個轉變是怎麼發生的——這代表手動完成不需要額外碰這段邏輯就能自動受惠 | `src/components/TaskBoard/index.tsx` 的 `refresh()`（`lastStatusRef`/`justFinished` 判斷），今天稍早完成，已合併進 master |

## 範圍

**含：**

- `tasks` 表新增 `interactive` 欄位（bool，跟 `parallel_ok` 同款式），建卡片時選、只能在 `planning` 狀態編輯（沿用 `parallel_ok` 既有規則，不另外做「中途切換模式」的機制）
- 建卡片/編輯對話框新增「互動模式」開關；勾選後隱藏「並行/單獨執行」開關（對互動任務沒有意義）
- 互動模式卡片移到「待執行」後：跳過併發上限，立即派工；自動送出初始 prompt；**不**附加「請印出完成標記」指示
- 背景監看：互動模式下關閉「bell/完成標記 → 成功」跟「120 秒無輸出 → 卡住失敗」這兩個訊號；保留「Claude Code 進程 exit code → 成功/失敗」
- 新增「手動標記完成」動作，兩種觸發方式都支援：卡片上的按鈕、把卡片從「執行中」拖到「已完成」——都只對 `running` 狀態的互動卡片開放，走同一套後端邏輯，讓現有的 watch 收尾流程（含 transcript 升級）原樣接手
- 「執行中」卡片視覺上加 `👤 互動` badge，跟自動任務區分

**不含：**

- 互動任務的獨立/額外併發上限——完全不算進任何上限（使用者明確選擇，不做防護上限）
- 任務跑到一半才臨時切換模式——模式在建卡片當下決定，之後（含 `queued`/`running`/`done`）不能再改，沿用 `parallel_ok` 既有的「只能在 planning 編輯」規則，不另外做特殊 UI
- 手動標記完成時選擇成功/失敗——一律視為成功（想中止用既有「停止」按鈕，會標成「已中斷」）
- 修改 `pick_next` 這個純函式本體或它的既有測試——互動卡片完全繞過它，走一條獨立的派工路徑

## 架構

### 資料模型

`tasks` 表新增一欄，比照 `parallel_ok` 的既有寫法：

```sql
ALTER TABLE tasks ADD COLUMN interactive INTEGER NOT NULL DEFAULT 0;
```

`TaskRow` 加 `pub interactive: bool`；`store::create_task` 簽章加一個 `interactive: bool` 參數（`INSERT` 語句同步加欄位）；`store::clone_task`（重新派工用）原樣把來源卡片的 `interactive` 一併複製過去，跟 `parallel_ok` 現在的做法一致。

`commands/tasks.rs::tasks_update` 目前的寫法是：`CreateArgs`/`UpdateArgs` 各自帶一個 `parallel_ok: bool` 欄位，`tasks_update` 呼叫獨立的 `store::set_parallel_ok(pool, id, parallel_ok)`（這一步在 `edit_allowed(&row.status)` 檢查**之前**，不受它閘控——沿用既有行為，不在這次改動範圍內）。`interactive` 要照同一個模式：`CreateArgs`/`UpdateArgs` 加 `interactive: bool` 欄位，新增 `store::set_interactive(pool, id, interactive)`，`tasks_update` 裡在 `set_parallel_ok` 呼叫旁邊加一行呼叫它。

### 排程器：互動卡片繞過併發上限

`pick_next` 本體、簽章、既有 6 個測試全部不動。改在 `drain_once` 裡面，在原本那個呼叫 `pick_next` 的迴圈**之前**，新增一段「無條件派工每一張 `queued` 且 `interactive` 的卡片」的迴圈：

```rust
pub async fn drain_once(db: &TasksDb, dispatcher: &dyn Dispatcher, max_concurrent: u32) {
    // 互動卡片不受併發上限/solo 阻擋規則約束——每一輪先把它們全部派掉。
    loop {
        let queued = match store::list_by_status(&db.pool, store::STATUS_QUEUED).await {
            Ok(q) => q,
            Err(e) => { eprintln!("scheduler list queued (interactive pass): {e}"); return; }
        };
        let Some(next) = queued.into_iter().find(|t| t.interactive) else { break; };
        if let Err(e) = dispatcher.dispatch(db, &next).await {
            eprintln!("dispatch {} failed: {e}", next.id);
            let _ = store::mark_dispatched(&db.pool, &next.id, "").await;
            let _ = store::finish_task(&db.pool, &next.id, "failed", Some(&e), None).await;
        }
    }

    // 既有邏輯不變，但 running/queued 只看非互動卡片——互動卡片對這條路徑
    // 完全隱形，既不算進 running.len()（併發上限），也不會被 solo 卡片
    // 擋住或去擋別人。
    loop {
        let running = match store::list_by_status(&db.pool, store::STATUS_RUNNING).await {
            Ok(r) => r.into_iter().filter(|t| !t.interactive).collect::<Vec<_>>(),
            Err(e) => { eprintln!("scheduler list running: {e}"); return; }
        };
        let queued = match store::list_by_status(&db.pool, store::STATUS_QUEUED).await {
            Ok(q) => q.into_iter().filter(|t| !t.interactive).collect::<Vec<_>>(),
            Err(e) => { eprintln!("scheduler list queued: {e}"); return; }
        };
        let Some(next) = pick_next(&running, &queued, max_concurrent) else { return; };
        let next = next.clone();
        if let Err(e) = dispatcher.dispatch(db, &next).await {
            eprintln!("dispatch {} failed: {e}", next.id);
            let _ = store::mark_dispatched(&db.pool, &next.id, "").await;
            let _ = store::finish_task(&db.pool, &next.id, "failed", Some(&e), None).await;
        }
    }
}
```

### 派工：不要求完成標記

`spawn_and_run` 新增一個參數，往下原封不動傳給 `run_on_session`：

```rust
pub async fn spawn_and_run(
    app: &AppHandle,
    pty: &PtyManager,
    project_dir: &str,
    claude_command: &str,
    prompt: &str,
    request_done_marker: bool,   // 新增
) -> Result<(String, DispatchResult), String> {
    ...
    let result = run_on_session(pty, &tab_id, prompt, request_done_marker).await?;
    ...
}
```

`RealDispatcher::dispatch` 呼叫時傳 `!task.interactive`（自動任務原行為不變，繼續是 `true`）。

### 監看：互動模式關掉會誤判的兩個訊號

`monitor::watch` 新增一個模式參數，只影響訊號②跟⑤：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// 現行行為：bell/完成標記→成功、120 秒無輸出→卡住失敗，兩者都判斷。
    Auto,
    /// 互動任務：bell 在正常對話裡本來就一直響、使用者思考超過 120 秒也很
    /// 正常，這兩個訊號在互動模式下只會誤判，整個跳過。exit code 訊號
    /// （③④）維持不變——進程真的終止是硬訊號，不管是不是互動模式都不會
    /// 誤判，值得繼續自動抓。
    Interactive,
}

pub async fn watch(
    pty: &PtyManager,
    tab_id: &str,
    mut control: oneshot::Receiver<WatchControl>,   // 型別見下
    baselines: Baselines,
    thresholds: Thresholds,
    mode: WatchMode,                                 // 新增
) -> TaskOutcome {
    ...
    loop {
        // 1. 外部控制訊號（cancel 或手動標記完成）
        match control.try_recv() {
            Ok(WatchControl::Cancel) => return TaskOutcome::Cancelled,
            Ok(WatchControl::MarkDone) => return TaskOutcome::Success,
            Err(oneshot::error::TryRecvError::Closed) => return TaskOutcome::Cancelled,
            Err(oneshot::error::TryRecvError::Empty) => {}
        }
        ...
        // 2. reply signal — 只有 Auto 模式判斷
        if mode == WatchMode::Auto && (marker > baselines.marker || bell > baselines.bell) {
            return TaskOutcome::Success;
        }
        // 3/4. exit code — 兩種模式都判斷，不變
        ...
        // 5. stuck — 只有 Auto 模式判斷
        if mode == WatchMode::Auto && ran_ms >= thresholds.min_run_ms && quiet_ms >= thresholds.quiet_stuck_ms {
            return TaskOutcome::Failed("疑似卡住（120 秒無輸出）".to_string());
        }
        ...
    }
}
```

`RealDispatcher::dispatch` 依 `task.interactive` 決定傳 `WatchMode::Auto` 或 `WatchMode::Interactive`。

### 手動標記完成：重用 cancel channel 的既有管道

現有 `cancels: HashMap<task_id, oneshot::Sender<()>>` 只能送一種訊號（cancel）。把 payload 從 `()` 換成一個小 enum，`SchedulerHandle` 對稱地多一個方法：

```rust
pub enum WatchControl { Cancel, MarkDone }

impl SchedulerHandle {
    pub fn cancel(&self, task_id: &str) -> bool {
        self.send(task_id, WatchControl::Cancel)
    }
    pub fn mark_done(&self, task_id: &str) -> bool {
        self.send(task_id, WatchControl::MarkDone)
    }
    fn send(&self, task_id: &str, msg: WatchControl) -> bool {
        if let Some(tx) = self.cancels.lock().remove(task_id) {
            let _ = tx.send(msg);
            true
        } else {
            false
        }
    }
}
```

`watch()` 收到 `MarkDone` 直接回傳 `TaskOutcome::Success`，之後完全走現有收尾流程（`write_transcript` → `store::finish_task(outcome="success")` → emit `tasks-updated`）——包含今天才做完的對話記錄自動升級，因為那段邏輯只看「狀態變成 done」，不管是誰觸發的，不需要為這個功能另外碰 `TaskBoard/index.tsx` 的升級判斷。

### 新後端指令 `tasks_mark_done`

```rust
#[tauri::command]
pub async fn tasks_mark_done(
    id: String,
    db: State<'_, TasksDb>,
    scheduler: State<'_, SchedulerHandle>,
) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    if row.status != store::STATUS_RUNNING || !row.interactive {
        return Err("only a running interactive task can be marked done".to_string());
    }
    if scheduler.mark_done(&id) { Ok(()) } else { Err("task has no active watch".to_string()) }
}
```

前端合法性檢查（按鈕/拖曳只在 `running && interactive` 才會出現/允許）只是 UX 層——後端一樣要擋，避免對非互動或非執行中的卡片誤呼叫。

`src/ipc/tasks.ts` 新增：

```ts
export const markTaskDone = (id: string): Promise<void> => invoke("tasks_mark_done", { id });
```

### 前端 UI

**`TaskEditorDialog`**：在「並行/單獨執行」那個 checkbox 前面加一個「互動模式」checkbox（`interactive`/`setInteractive`，跟 `parallelOk` 同款寫法）；`interactive === true` 時，整段「並行/單獨執行」checkbox + hint 不渲染（不是 disabled，是直接不顯示——對互動任務來說那個設定沒有意義）。`createTask`/`updateTask` 呼叫加上 `interactive: interactive` 欄位。

**`TaskCard`**：
- 卡片上（不分欄位）：`card.interactive && <div className="task-badge task-badge--interactive">👤 {t.board_badge_interactive}</div>`
- `status === "running" && card.interactive` 時，動作區多一個「標記完成」按鈕，呼叫 `markTaskDone(card.id)`（跟現有 `run()` 包裝一致，成功後 `onChanged()` 觸發 `refresh()`）

**`TaskBoard/index.tsx` 的 `handleDrop`**：合法轉換規則加一條——`cardRow.status === "running" && to === "done" && cardRow.interactive`。這個分支不呼叫 `moveTask`（那個指令只是單純狀態搬移，沒有「完成」該有的副作用），改呼叫 `markTaskDone(id)`，跟卡片按鈕走同一個後端指令。非互動卡片拖到「已完成」維持現在的行為——`legal` 判斷式為 false，直接忽略。

## 已知限制

- 互動任務完全不受併發上限約束，代表使用者可以同時開任意多個——這是刻意的設計（Q3 選 B），不做額外的資源防護；如果之後發現真的有人開太多導致系統吃不消，屬於另一個獨立問題，這次不處理。
- exit code 0（Claude Code CLI 自己乾淨結束）在互動模式下依然會自動把卡片判定成功、搬到已完成，即使使用者可能還沒打完——這跟「互動模式不自動完成」的精神看似有點矛盾，但這是刻意保留的：終端機真的關閉代表確實沒辦法再輸入，是硬訊號，跟 bell 這種每次對話都會觸發的軟訊號性質不同。
- 卡片一旦離開 `planning` 狀態，互動模式的開關就不能再改——沿用 `parallel_ok` 現有的規則，不是這次新增的限制。

## 測試

- **後端單元測試（`scheduler.rs`）**：新增案例——互動卡片即使 `running` 已達併發上限、或有 solo 卡片正在跑，一樣立刻被 `drain_once` 派工；互動卡片不會被算進 `pick_next` 用來判斷併發上限的 `running` 集合裡（非互動卡片的派工節奏不受互動卡片影響）。
- **後端單元測試（`monitor.rs`）**：`WatchMode::Interactive` 下，bell/marker 超過 baseline 不會觸發 `Success`（要能一路撐到別的訊號才結束）；`WatchMode::Interactive` 下，超過 `quiet_stuck_ms` 不會觸發 `Failed(卡住)`；兩種模式下 exit code 訊號行為不變（沿用/複製既有測試）；收到 `WatchControl::MarkDone` 回傳 `Success`；收到 `WatchControl::Cancel` 回傳 `Cancelled`（既有測試的 payload 型別改一下,行為不變）。
- **後端整合測試（`commands/tasks.rs` 或 `tests/task_board.rs`）**：`tasks_mark_done` 對 `running` 且 `interactive` 的任務成功、觸發 `finish_task(outcome="success")`；對非 `running` 或非 `interactive` 的任務回錯誤，不 panic。
- **前端單元測試（`TaskEditorDialog.test.tsx`）**：勾選「互動模式」後「並行/單獨執行」整段消失；建立/更新呼叫帶上正確的 `interactive` 值。
- **前端單元測試（`TaskCard.test.tsx`）**：`interactive` 卡片顯示 badge；`running && interactive` 顯示「標記完成」按鈕且點擊呼叫 `markTaskDone`；非互動或非 running 狀態不顯示這顆按鈕。
- **前端單元測試（`TaskBoard/index.test.tsx`）**：拖曳 `running→done` 對互動卡片呼叫 `markTaskDone`（不是 `moveTask`）；對非互動卡片維持忽略（既有行為不變的回歸測試）。

## 相關

`docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`（工作看板原始設計，`parallel_ok`/`pick_next`/`monitor::watch` 機制的出處）
`docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md`（對話記錄升級機制，這次手動完成會自動沿用，不需要修改）

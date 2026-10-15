# Agent 協作桌布（Agent Canvas）— 設計

日期：2026-10-15
狀態：使用者已逐節確認（§1–§3），授權直接進入實作
原型：`.superpowers/brainstorm/agent-canvas/index.html`

## 問題與目標

使用者要把一個 C/S 架構系統（HTBKTW）的 A–F 六個模組轉成網頁系統，想要：

- A、B、C 派給 Coder Agent1，D、E、F 派給 Coder Agent2，兩者平行開發互不干擾；
- 一個 Code Reviewer **每個模組做完就審**，不通過自動退回原 Coder；
- 一個 Orchestrator 在**所有模組都通過後**把兩個 Coder 的程式碼合併；
- 一個獨立分頁用「桌布＋拖曳＋流程光棒」把上面這一切視覺化。

工作看板（`src-tauri/src/tasks/`、`src/components/TaskBoard/`）已經能「一張卡片＝一次派工給 claude」，
但沒有「Agent」概念：沒有依序執行、每張卡各開一個 worktree 分支、卡片之間沒有交接。

## 分期

| 期 | 內容 | 本文件 |
|---|---|---|
| 第一期 | 新分頁「Agent 協作」、桌布、常駐 Coder Agent、角色、模組拖給 Agent 即派工、每個 Agent 一個分支且依序執行、狀態光環、失敗暫停、手動合併 Agent 分支 | **完整設計（§1–§3）** |
| 第二期 | Reviewer／Orchestrator 實際運作、自動連線與交接 | 已拍板的決定記在 §4，另開 spec |

## 現況（讀過程式碼確認）

- 排程：`scheduler.rs::drain_once` 每個專案只交出**一張**非互動隊首（`list_by_status(queued)` 依 `sort_order`），
  跨專案依 `created_at` 排序後交給 `pick_next(running, [head], max_concurrent)`。互動卡片另走第一輪、不受上限。
- 派工：`RealDispatcher::dispatch` → `prepare_worktree`（每張卡 `<專案>/tasks/<id>/worktree`、分支 `aiterm-task/<id>`）
  → `dispatch::spawn_and_run` → `monitor::watch` → `persist_outcome` → `finish_task`。
- `store.rs`：`TaskRow` 用 `SELECT *` + `FromRow`，新欄位照 `init_schema` 的 `ALTER TABLE ... ADD COLUMN`（吞錯）模式。
- 合併：`commands/tasks.rs::tasks_merge_worktree` 有完整的前置檢查（`is_merge_in_progress`、`has_uncommitted_changes`）
  與 `MergeOutcome`（`merged | conflict | merged_but_not_cleaned | blocked`）。
- 前端：`TabType` 定義在 `src/components/TabBar/index.tsx`，要改的地方有 `TabBar`（圖示）、`TerminalApp`（標題鏈＋render 分支）、
  `NewTabPicker/tabCatalog.tsx`（`_exhaustive` 窮舉檢查）。拖曳一律用 mouse/pointer 事件（Tauri `dragDropEnabled` 會吃掉 HTML5 DnD），
  拖曳影子用 portal 掛到 `document.body`。資料刷新靠 `onTasksUpdated`（`tasks-updated` 事件）。

## §1 資料模型與排程

### `agents` 表（每個看板專案的 `tasks.db`）

```sql
CREATE TABLE IF NOT EXISTS agents (
    id              TEXT PRIMARY KEY NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'coder',   -- coder | reviewer | orchestrator（第一期只建立 coder）
    name            TEXT NOT NULL,
    role            TEXT NOT NULL,                   -- 角色預設名稱（Senior Coder…）
    prompt_prefix   TEXT NOT NULL DEFAULT '',
    project_dir     TEXT NOT NULL,                   -- 這個 Agent 負責的 git repo
    canvas_x        REAL NOT NULL DEFAULT 0,
    canvas_y        REAL NOT NULL DEFAULT 0,
    worktree_path   TEXT,                            -- 第一次派工時建立
    branch          TEXT,                            -- aiterm-agent/<id>
    paused_task_id  TEXT,                            -- 非 NULL = 暫停中，指向造成暫停的那張卡
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### `tasks` 新欄位（migration：`ALTER TABLE ... ADD COLUMN`，吞錯）

- `agent_id TEXT` — 指派給哪個 Agent。`NULL` = 一般卡片，行為完全不變。
- `start_sha TEXT`、`end_sha TEXT` — 這張卡在 Agent 分支上的範圍；第二期審查用 `start..end` 取 diff。

### 指派

- `tasks_assign_agent(project_id, task_id, agent_id | null)`：
  - 指派：卡片必須是 `planning` 或 `queued`；寫入 `agent_id`、`project_dir = agent.project_dir`，
    狀態改 `queued`，`sort_order` 放到 queued 欄最後。
  - 取消指派：卡片必須是 `queued`；清掉 `agent_id`，狀態改回 `planning`。
- 看板上把有 Agent 的卡片拖回「計畫中」等同取消指派（`tasks_move` 到 `planning` 時清 `agent_id`）。

### 派工（`RealDispatcher::dispatch`，只在 `task.agent_id` 有值時走新路徑）

1. 讀 Agent；`prepare_agent_worktree`：已有 `worktree_path` 且資料夾存在 → 沿用；否則在
   `<專案>/agents/<agent_id>/worktree` 以 `aiterm-agent/<agent_id>` 建立（從 `agent.project_dir` 目前 HEAD 分出）並寫回 agents 表。
   **失敗不退回原資料夾**，派工回傳 Err（卡片 failed、Agent 暫停，見 §3）。
2. `start_sha = git rev-parse HEAD`（在 worktree 內）寫入卡片。
3. 提示詞 = `agent.prompt_prefix` + 空行 + `build_prompt(body, attachments)`（前綴空白就只有後者）。
4. cwd = Agent 的 worktree。卡片的 `worktree_path/worktree_branch` **不寫**（那是每卡隔離用的欄位，
   有值會讓看板顯示「合併回原分支」）。
5. 結束後（watch closure 內、`finish_task` 之前）：outcome 為 success →
   有未提交改動就 `commit_all("Module: <title>")`；`end_sha = HEAD`。commit 失敗 → outcome 改 failed、
   error_message 附 git 輸出。outcome 非 success → `paused_task_id = 這張卡`。

### 排程規則

`pick_next` 不動（既有測試全保留）。新增純函式：

```rust
/// 這一輪可以考慮的隊首卡片。
/// - 未指派卡片：維持原規則，只交出 sort_order 最小的一張
/// - 每個 Agent：自己 queued 卡片中 sort_order 最小的一張，但
///   Agent 有卡片正在 running、或 paused_task_id 非 NULL 時不交出
/// 回傳依 sort_order 排序。
pub fn candidate_heads(queued: &[TaskRow], running: &[TaskRow], paused_agents: &HashSet<String>) -> Vec<TaskRow>
```

`drain_once` 每個專案改用 `candidate_heads` 的結果（取代單一 head），每一張再照舊套 `pick_next`
（全域上限與 `parallel_ok`）。互動卡片的第一輪**排除有 `agent_id` 的卡片**——Agent 卡片一律受依序執行與全域上限約束。

## §2 新分頁與桌布 UI

- 新 `TabType` = `"agent-canvas"`，標題 i18n `agent_canvas_tab`（「Agent 協作」/「Agent Canvas」），
  放進 `tabCatalog`，色 `#e879f9`（目前未被其他分頁使用）。
- 版面（對照原型）：頂列（看板專案下拉、同時上限顯示）、Agent 池（第一期只有 Coder）、左側模組庫、桌布、底部活動記錄。
- **模組＝看板卡片**：模組庫列出該專案 `planning` 卡片；「＋新增模組」開既有 `TaskEditorDialog`。已指派的卡片顯示在 Agent 卡上。
- 拖曳：pointer 事件 + 4px 門檻 + portal 影子（同看板）。
  - Agent 池 → 桌布：開「指派角色」對話框（名稱、角色預設 chips、可編輯前綴、程式庫資料夾）。確定才呼叫 `agents_create`；取消什麼都不建立。
  - 模組 → Coder：`tasks_assign_agent`。未設定完成的 Agent 不存在（建立即完成設定），所以只需檢查 kind。
  - Agent 卡 chip（queued）→ 另一個 Coder 或模組庫：改派／取消指派。running／done 的 chip 不能拖。
  - Agent 卡表頭 → 移動：放開時 `agents_update` 寫 `canvas_x/y`。
- 狀態（`deriveAgentStatus` 純函式，由卡片推導）：
  `attention`（paused_task_id 非 NULL，紅）> `coding`（有 running 卡，藍色脈動）> `idle`（有 queued 卡、等名額）>
  `done`（有卡片且全部 done/success、無 queued）> `idle`（沒有卡片）。
- 進度：**沒有百分比**。coding 時顯示不定長度流動條＋已執行時間（`dispatched_at` 起算，每秒更新）。點 Agent 的執行中模組跳到其分頁（`aiterm:focus-tab`）。
- 失敗（attention）：Agent 卡顯示失敗卡標題與錯誤訊息，三個動作：重試、跳過、看對話記錄（`TranscriptDialog`）。
- 合併：Agent 閒置、未暫停、分支存在時顯示「合併分支」→ `agents_merge`；衝突時列出檔案＋「還原合併」（`agents_abort_merge`）。
- 資料同步：協作頁不保存狀態，`agents_list` + `tasks_list`，監聽 `tasks-updated` 重載（agents 變動也發同一個事件）。

### IPC

| 指令 | 說明 |
|---|---|
| `agents_list(project_id)` | 全部 Agent |
| `agents_create(project_id, args{kind,name,role,prompt_prefix,project_dir,canvas_x,canvas_y})` | 檢查 `project_dir` 是 git repo，否則 Err |
| `agents_update(project_id, args{id,name,role,prompt_prefix,canvas_x,canvas_y})` | `project_dir` 建立後不可改（分支已綁定） |
| `agents_delete(project_id, id)` | 有 running 卡片 → Err；名下 queued 卡片退回 planning 並清 agent_id；done 卡片保留 agent_id；worktree/分支不刪 |
| `agents_resolve(project_id, id, action: "retry" \| "skip")` | 見 §3 |
| `agents_merge(project_id, id)` → `MergeOutcome` | 見 §3 |
| `agents_abort_merge(project_id, id)` | `git merge --abort` 於 `agent.project_dir` |
| `tasks_assign_agent(project_id, task_id, agent_id?)` | 見 §1 |

## §3 錯誤處理與測試

- 建立 Agent 時 `project_dir` 不是 git repo（`VcsManager::detect_repo` 失敗或為 SVN）→ 拒絕並說明。
- worktree 建立失敗 → 派工失敗、Agent 暫停，不退回原資料夾。
- 自動 commit：沒改動 → success、`end_sha = start_sha`；commit 失敗（hook）→ failed＋git 輸出、Agent 暫停。
- 暫停：卡片 failed／cancelled（含卡住、手動停止、app 重啟被 `recover_orphaned_running` 收掉的）→ `paused_task_id`。
  `recover_orphaned_running` 之後補一步：被收掉的卡片若有 `agent_id`，對應 Agent 設暫停。
- `agents_resolve`：
  - `retry`：`clone_task_fields` 複製失敗卡（含附件，沿用 `tasks_clone` 的檔案複製），新卡 `agent_id` 同、狀態 queued、
    `sort_order` = 該 Agent queued 最小值 − 1；清 `paused_task_id`；worktree 原樣保留（在半成品上繼續）。
  - `skip`：worktree 有未提交改動 → `git stash push -u -m "aiterm skipped <task_id>"`；清 `paused_task_id`。
- 執行中卡片不可改派／取消指派（指令層檢查狀態）。
- 看板：有 `agent_id` 的卡片顯示 Agent 名稱徽章；本來就不會有 `worktree_branch`，所以「合併回原分支」按鈕自然不出現。
- `agents_merge`：Agent 有 running 卡或暫停中 → Err；基底 `is_merge_in_progress`／`has_uncommitted_changes` → `Blocked`；
  worktree 若仍有未提交改動 → `commit_all`；`merge_branch(branch)`，衝突 → `Conflict{files}`；**不刪 worktree、不刪分支**（Agent 還要繼續用）。

### 測試

- Rust 純函式：`candidate_heads`（Agent 忙碌只擋自己、暫停跳過、未指派維持單一隊首、排序）；既有 `pick_next` 測試不動。
- Rust store：agents CRUD、舊 schema migration（沒有 agents 表與三個新欄位）、`assign` 狀態限制、`recover_orphaned_running` 後暫停 Agent。
- Rust git（tempdir repo）：`prepare_agent_worktree` 第一次建立、第二次沿用；`head_sha`；`stash_push`；自動 commit 的有改動／沒改動。
- Rust 排程整合（既有 `drain_once` 測試 harness + 假 Dispatcher）：兩個 Agent 各兩張卡、上限 3 → 同時只會各跑一張。
- 前端 Vitest：`deriveAgentStatus`、拖曳指派呼叫 `assignAgent`、`tasks-updated` 觸發重載、tabTypeCoverage 加 `agent-canvas`。
- 實機：真 repo、兩個 Coder 各三張小卡，確認分支、commit、光環、失敗暫停、重試、跳過、合併。

## §4 第二期（已拍板，另開 spec）

- 連線自動建立：每張桌布最多 1 個 Reviewer、1 個 Orchestrator；Coder→Reviewer→Orchestrator。
- **每個模組做完就審**：Coder 卡 success 後自動產生一張 Reviewer 卡（`start_sha..end_sha` 的 diff）。
- 判定：Reviewer 把結果寫進任務資料夾的 `review.json`（`{"verdict":"pass"|"fail","reasons":[...]}`），不從終端機文字猜。
  只有正確性／需求不符才 fail，風格寫進報告。
- 不通過 → 自動退回：在原 Coder 佇列最前面插「修正 <模組>」卡（附審查意見），修完再審；
  **同一模組最多退回 2 輪**，第 3 次不通過 → 停下、標「需處理」、通知使用者（等同只產報告）。
- Orchestrator：所有指派模組都通過後，依序把各 Coder 分支合進整合分支，衝突交給 claude 解決。
- 光棒：交接時一顆光點沿線移動（藍＝交件、綠＝通過、橘＝退回）；下游處理上游的東西時線段持續流動。

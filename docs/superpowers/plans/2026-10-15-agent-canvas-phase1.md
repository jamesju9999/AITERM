# Agent Canvas 第一期 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新分頁「Agent 協作」：常駐 Coder Agent（角色＋提示詞前綴）、把看板卡片拖給 Agent 即派工、每個 Agent 一個 git 分支並依序執行、狀態光環、失敗暫停／重試／跳過、手動合併 Agent 分支。

**Architecture:** Agent 存在各看板專案的 `tasks.db`（新 `agents` 表＋`tasks.agent_id/start_sha/end_sha`）。排程器新增純函式 `candidate_heads` 讓每個 Agent 有自己的佇列隊首；`RealDispatcher` 對有 `agent_id` 的卡片改用 Agent 的 worktree、加角色前綴、結束時自動 commit 或暫停 Agent。前端是被動渲染器：`agents_list`＋`tasks_list`，監聽 `tasks-updated`。

**Tech Stack:** Rust（tauri 2、sqlx sqlite、tokio）、React 19 + TypeScript、Vitest + RTL。

Spec：`docs/superpowers/specs/2026-10-15-agent-canvas-design.md`

---

## File Structure

| 檔案 | 責任 |
|---|---|
| `src-tauri/src/tasks/agents.rs`（新） | `AgentRow`、agents CRUD、assign、pause/unpause、`agent_dir` |
| `src-tauri/src/tasks/mod.rs` | schema：agents 表＋tasks 三欄 migration；`pub mod agents` |
| `src-tauri/src/tasks/store.rs` | `TaskRow` 加 `agent_id/start_sha/end_sha`；`set_start_sha/set_end_sha`；`recover_orphaned_running` 回傳被收掉的 agent ids |
| `src-tauri/src/vcs/git.rs` | `head_sha`、`stash_push` |
| `src-tauri/src/tasks/scheduler.rs` | `candidate_heads`；`drain_once` 改用它；`RealDispatcher` 的 Agent 路徑；`finalize_agent_card` |
| `src-tauri/src/commands/agents.rs`（新） | `agents_*` 與 `tasks_assign_agent` 指令 |
| `src-tauri/src/commands/tasks.rs` | `tasks_move` 到 planning 時清 `agent_id` |
| `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs` | 註冊 |
| `src/ipc/agents.ts`（新）、`src/ipc/tasks.ts` | 型別與 invoke 包裝 |
| `src/components/AgentCanvas/*`（新） | `index.tsx`（AgentCanvasView）、`AgentNode.tsx`、`AgentConfigDialog.tsx`、`EdgeLayer.tsx`（第一期無邊，保留給第二期——**不建立**，YAGNI）、`deriveAgentStatus.ts`、`roles.ts`、`index.css` |
| `src/components/TabBar/index.tsx`、`TerminalApp.tsx`、`NewTabPicker/tabCatalog.tsx` | 新 TabType 接線 |
| `src/components/TaskBoard/TaskCard.tsx` | Agent 名稱徽章 |
| `src/lib/i18n.ts` | zh-TW／en 字串 |

---

### Task 1: schema 與 TaskRow 新欄位

**Files:** Modify `src-tauri/src/tasks/mod.rs`, `src-tauri/src/tasks/store.rs`, `src-tauri/src/tasks/scheduler.rs`（兩個測試 helper 的 TaskRow literal）

- [ ] Step 1：在 `store.rs` 測試模組寫失敗測試 `old_schema_gains_agent_columns_and_agents_table`：建只有舊欄位的 tasks 表 → `init_schema` → `create_task` 後 `get_task` 的 `agent_id/start_sha/end_sha` 都是 None，且 `SELECT COUNT(*) FROM agents` 成功。
- [ ] Step 2：`cargo test -p app --lib tasks::store` 確認失敗（欄位不存在）。
- [ ] Step 3：`init_schema` 的 CREATE TABLE 加三欄＋三行 `ALTER TABLE tasks ADD COLUMN ...`（吞錯）；新增 `CREATE TABLE IF NOT EXISTS agents (...)`（欄位照 spec §1）。`TaskRow` 加：
  ```rust
  /// 指派給哪個 Agent（`agents.id`）。`None` = 一般卡片，排程與派工完全照舊。
  pub agent_id: Option<String>,
  /// Agent 卡片派工當下 worktree 的 HEAD；與 `end_sha` 構成這個模組的 commit 範圍。
  pub start_sha: Option<String>,
  pub end_sha: Option<String>,
  ```
  更新 `scheduler.rs` 測試的 `row()`／`queued_row()` literal（加 `agent_id: None, start_sha: None, end_sha: None`）。
- [ ] Step 4：測試通過。
- [ ] Step 5：commit `feat(tasks): agents table and agent columns on tasks`

### Task 2: `tasks/agents.rs` CRUD＋assign＋pause

**Files:** Create `src-tauri/src/tasks/agents.rs`; Modify `src-tauri/src/tasks/mod.rs`

API：
```rust
pub struct AgentRow { id, kind, name, role, prompt_prefix, project_dir, canvas_x: f64, canvas_y: f64,
                      worktree_path: Option<String>, branch: Option<String>, paused_task_id: Option<String>, created_at }
pub struct NewAgent<'a> { kind, name, role, prompt_prefix, project_dir, canvas_x, canvas_y }
pub async fn create_agent(pool, &NewAgent) -> Result<String>
pub async fn list_agents(pool) -> Result<Vec<AgentRow>>            // ORDER BY created_at, id
pub async fn get_agent(pool, id) -> Result<Option<AgentRow>>
pub async fn update_agent(pool, id, name, role, prompt_prefix, x, y) -> Result<()>
pub async fn set_agent_worktree(pool, id, path, branch) -> Result<()>
pub async fn set_paused(pool, id, task_id: Option<&str>) -> Result<()>
pub async fn delete_agent(pool, id) -> Result<()>                   // queued 卡退回 planning＋清 agent_id；有 running → Err
pub async fn assign_task(pool, task_id, agent_id: Option<&str>) -> Result<()>
pub fn agent_dir(project_path, agent_id) -> PathBuf                  // <專案>/agents/<id>
```
`assign_task`：Some → 卡必須 planning|queued，否則 `Protocol("只有計畫中或待執行的卡片可以指派")`；寫 agent_id、project_dir=agent.project_dir、status=queued、sort_order=queued 最大+1。None → 卡必須 queued，清 agent_id、status=planning、sort_order=planning 最大+1。

- [ ] Step 1：測試（同檔 `#[cfg(test)]`，mem_pool＋init_schema）：
  `create_then_list_round_trips`、`assign_moves_planning_card_to_queued_and_copies_project_dir`、`assign_rejects_running_card`、`unassign_returns_card_to_planning`、`delete_agent_returns_queued_cards_and_refuses_while_running`、`set_paused_round_trips`。
- [ ] Step 2：確認失敗（模組不存在→編譯錯誤）。
- [ ] Step 3：實作。
- [ ] Step 4：通過。
- [ ] Step 5：commit `feat(tasks): agent store`

### Task 3: git `head_sha`、`stash_push`

**Files:** Modify `src-tauri/src/vcs/git.rs`

```rust
/// `git rev-parse HEAD`
pub async fn head_sha(&self) -> Result<String, String>
/// `git stash push -u -m <message>`（含未追蹤檔）
pub async fn stash_push(&self, message: &str) -> Result<VcsResult, String>
```
- [ ] 測試（沿用該檔 `init_repo` helper）：`head_sha_matches_rev_parse`、`stash_push_cleans_untracked_and_tracked_changes`（之後 `has_uncommitted_changes` 為 false、`git stash list` 含訊息）。
- [ ] 失敗 → 實作 → 通過 → commit `feat(vcs): head_sha and stash_push`

### Task 4: `candidate_heads` 與 `drain_once`

**Files:** Modify `src-tauri/src/tasks/scheduler.rs`, `src-tauri/src/tasks/store.rs`（`list_paused_agent_ids` 放 agents.rs）

```rust
pub fn candidate_heads(queued: &[TaskRow], running: &[TaskRow], paused: &HashSet<String>) -> Vec<TaskRow>
```
- 未指派（且非 interactive）：只取 sort_order 最小一張。
- 每個 agent_id：若 running 中有同 agent_id 或 paused 含之 → 跳過；否則取該 agent sort_order 最小一張（含 interactive）。
- 結果依 sort_order 排序。

`drain_once`：互動第一輪 `find(|t| t.interactive && t.agent_id.is_none())`。一般輪：每個專案 `candidate_heads(queued, running_of_this_project, paused)` 的每一張都推進 `heads`；`running` 收集時**包含** interactive 的 Agent 卡（`filter(|t| !t.interactive || t.agent_id.is_some())`）——Agent 卡一律占名額。

- [ ] 純函式測試：`agent_busy_blocks_only_its_own_queue`、`paused_agent_is_skipped`、`unassigned_cards_keep_a_single_head`、`heads_are_sorted_by_sort_order`。
- [ ] `drain_once` 測試（StuckDispatcher）：`two_agents_run_one_card_each_even_with_spare_capacity`（兩 Agent 各兩張、cap 5 → running 恰為各自第一張）、`an_interactive_agent_card_still_waits_for_its_agent`。
- [ ] 失敗 → 實作 → 通過（含既有 scheduler 測試全綠）→ commit `feat(scheduler): per-agent queues`

### Task 5: Agent 派工路徑與結束處理

**Files:** Modify `src-tauri/src/tasks/scheduler.rs`, `src-tauri/src/tasks/store.rs`

純／可測函式：
```rust
/// 角色前綴 + 卡片提示詞。前綴全空白就原樣回傳。
pub fn with_role_prefix(prefix: &str, prompt: &str) -> String
/// 沿用或建立 Agent worktree。回傳 worktree 路徑。失敗回 Err（不退回原資料夾）。
async fn prepare_agent_worktree(pool, project_path, agent: &AgentRow) -> Result<PathBuf, String>
/// watch 結束、finish_task 之前：success → commit 剩餘改動並記 end_sha（commit 失敗回傳 Failed outcome）；
/// 非 success → 暫停 Agent。回傳最終 outcome。
async fn finalize_agent_card(pool, agent_id, task_id, title, worktree: &Path, outcome: TaskOutcome) -> TaskOutcome
```
store：`set_start_sha`、`set_end_sha`。`RealDispatcher::dispatch`：`task.agent_id` 有值時 → get_agent（不存在 → Err）→ `prepare_agent_worktree`（Err → 先 `set_paused` 再回 Err）→ `start_sha` → prompt 加前綴 → cwd=worktree；**不**呼叫 `set_worktree`。watch closure：`let outcome = if let Some(aid) = agent { finalize_agent_card(...).await } else { outcome };` 再 `persist_outcome`。dispatch Err 的路徑（`drain_once` 裡 `finish_task(failed)`）也要暫停 Agent → 在 `drain_once` 的 Err 分支：`if let Some(aid) = &head.agent_id { agents::set_paused(pool, aid, Some(&head.id)) }`。
`monitor::TaskOutcome` 需要 `Clone`／可建構 `Failed(String)`（已有）。

- [ ] 測試：`with_role_prefix_*`（空前綴、一般）；tempdir repo：`prepare_agent_worktree_creates_then_reuses`（第二次路徑相同、DB 有 branch）、`prepare_agent_worktree_fails_for_non_git_dir`；`finalize_success_commits_changes_and_records_end_sha`、`finalize_success_without_changes_keeps_end_equal_start`、`finalize_failure_pauses_agent`。
- [ ] 失敗 → 實作 → 通過 → commit `feat(scheduler): dispatch agent cards into the agent worktree`

### Task 6: 重啟收尾暫停 Agent

**Files:** Modify `src-tauri/src/tasks/store.rs`, `scheduler.rs::spawn`

`recover_orphaned_running` 前先 `SELECT id, agent_id FROM tasks WHERE status='running' AND agent_id IS NOT NULL`，更新後對每筆 `agents::set_paused(agent_id, task_id)`。放在 `agents::recover_paused_after_restart(pool)`，`spawn` 啟動恢復時於 `recover_orphaned_running` **之前**呼叫。
- [ ] 測試 `restart_recovery_pauses_agents_of_orphaned_cards` → 實作 → commit。

### Task 7: 指令層

**Files:** Create `src-tauri/src/commands/agents.rs`; Modify `commands/mod.rs`, `lib.rs`, `commands/tasks.rs`

指令：`agents_list`、`agents_create`（`VcsManager::detect_repo` 必須 Ok 且 `vcs_type == Git`，否則 Err「Agent 的程式庫必須是 git repo」）、`agents_update`、`agents_delete`、`agents_resolve(action)`、`agents_merge`、`agents_abort_merge`、`tasks_assign_agent`。全部變動後 `emit("tasks-updated")`，assign/resolve 後 `scheduler.poke()`。
- `agents_resolve("retry")`：paused_task_id 必須有值；沿用 `tasks_clone` 的複製（抽出 `clone_with_attachments(p, src_id) -> Result<String,String>` 給兩者共用），新卡 `assign_task(new, Some(agent))` 後 `move_task` 到 queued 最前（`midpoint_between(queued, None, first_of_agent)`）；`set_paused(None)`。
- `agents_resolve("skip")`：worktree 存在且 dirty → `stash_push("aiterm skipped <task_id>")`；`set_paused(None)`。
- `agents_merge`：見 spec §3；回傳 `commands::tasks::MergeOutcome`（`pub` 已是）。
- `tasks_move`：`to_status == planning` 時 `agents::assign_task` 不適用（狀態可能已是 planning），直接 `UPDATE tasks SET agent_id=NULL` → 新增 `agents::clear_agent(pool, task_id)`。
- [ ] 測試：`clone_with_attachments` 抽出後既有 tasks_clone 行為不變（跑既有測試）；新增 `agents::clear_agent` 單元測試。指令本身靠前面的 store 測試＋實機。
- [ ] `cd src-tauri && cargo test --workspace --no-fail-fast` 全綠 → commit `feat(commands): agent canvas commands`

### Task 8: 前端 IPC

**Files:** Create `src/ipc/agents.ts`; Modify `src/ipc/tasks.ts`（TaskRow 加三欄）
```ts
export type AgentKind = "coder" | "reviewer" | "orchestrator";
export interface AgentRow { id; kind: AgentKind; name; role; prompt_prefix; project_dir; canvas_x: number; canvas_y: number;
  worktree_path: string|null; branch: string|null; paused_task_id: string|null; created_at: string }
listAgents(projectId) / createAgent(projectId, args) / updateAgent(projectId, args) / deleteAgent(projectId, id)
resolveAgent(projectId, id, action: "retry"|"skip") / mergeAgent(projectId, id): Promise<MergeOutcome> / abortAgentMerge / assignAgent(projectId, taskId, agentId|null)
```
- [ ] 測試 `src/ipc/agents.test.ts`：mock `invoke`，斷言指令名與參數形狀（`args` 用 snake_case，與 Rust Deserialize 對齊）。→ commit。

### Task 9: `deriveAgentStatus`＋`roles.ts`

**Files:** Create `src/components/AgentCanvas/deriveAgentStatus.ts`, `roles.ts`, tests
```ts
export type AgentStatus = "attention" | "coding" | "waiting" | "done" | "idle";
export function deriveAgentStatus(agent: AgentRow, cards: TaskRow[]): AgentStatus
```
順序：paused_task_id → attention；任何 running → coding；任何 queued → waiting；有卡且全部 done 且 outcome success → done；否則 idle（含：有失敗卡但已跳過）。
`roles.ts`：`ROLE_PRESETS.coder = [{name, prefix}...]`（Migration Engineer、Senior Coder、Bug Fixer、Frontend Specialist；前綴文字照原型）。
- [ ] 測試五種狀態＋優先序 → commit。

### Task 10: TabType 接線＋i18n

**Files:** `TabBar/index.tsx`（union＋`getTabIcon` case 用 `RobotIcon`）、`TerminalApp.tsx`（title 鏈＋render 分支 `<AgentCanvasView isActive={isActive} />`）、`tabCatalog.tsx`（entry 色 `#e879f9`）、`src/lib/i18n.ts`（zh-TW 與 en 各一份：`agent_canvas_tab`、`new_agent_canvas_desc` 及 Task 11 用到的所有字串）、`tabTypeCoverage.test.ts`（加 agent-canvas 一組 describe）。
- [ ] coverage 測試先紅 → 接線 → 綠 → `npx tsc -b` → commit。

### Task 11: AgentCanvasView

**Files:** Create `src/components/AgentCanvas/index.tsx`, `AgentNode.tsx`, `AgentConfigDialog.tsx`, `index.css`; test `index.test.tsx`

- 專案下拉：`listProjects()`，記住上次選擇（localStorage `aiterm.agentCanvas.project`，try/catch）。
- 資料：`refresh = Promise.all([listAgents, listTasks])`；`onTasksUpdated` 重載；`mounted` ref 防卸載後 setState。
- 模組庫：`tasks.filter(planning && !agent_id)`；「＋新增模組」→ `TaskEditorDialog`（`card=null`）。
- Agent 池（只有 Coder）→ 拖到桌布 → `AgentConfigDialog`（名稱預設 `Coder Agent{n}`、角色 chips、前綴 textarea、程式庫資料夾＝`usedDirs` 下拉＋可輸入）→ `createAgent`，錯誤顯示在對話框內。
- `AgentNode`：表頭（圖示、名稱、角色、狀態徽章、⚙ 編輯、× 刪除）、目前模組＋經過時間＋不定長度流動條、模組 chips（queued 可拖）、分支、attention 區（失敗卡標題、error_message、重試／跳過／對話記錄）、合併按鈕（status idle|done 且 branch 有值）。
- 拖曳：一個 `useCanvasDrag` 邏輯寫在 index.tsx（mousedown/move/up、4px 門檻、portal 影子、`elementFromPoint` 找 `[data-agent-id]`／`[data-drop="library"]`／`[data-drop="canvas"]`）。
- 刪除確認用 `@tauri-apps/plugin-dialog` 的 `confirm`（`window.confirm` 在 Tauri 無效）。
- 測試（mock `@tauri-apps/api/core` invoke、`@tauri-apps/api/event` listen）：載入後顯示 Agent 與模組；模擬 mousedown/move/up 把模組拖到 Agent → `tasks_assign_agent` 被呼叫；`tasks-updated` 觸發第二次 `agents_list`；attention 時按「重試」呼叫 `agents_resolve` with `retry`。
- [ ] 先寫測試 → 實作 → 綠 → commit。

### Task 12: 看板徽章

**Files:** `src/components/TaskBoard/TaskCard.tsx`、`ProjectBoard.tsx`
ProjectBoard 多抓 `listAgents` 建 `Map<id,name>`，TaskCard 新 prop `agentName?: string`，有值時在標題列顯示 `🤖 {agentName}` 徽章。
- [ ] TaskCard 測試：有 agentName 顯示徽章 → commit。

### Task 13: 全量驗證

- [ ] `npm run test`、`npx tsc -b`、`npm run lint`、`cd src-tauri && cargo test --workspace --no-fail-fast`（逐行看 `test result:`）。
- [ ] `npm run tauri:dev` 實機：真 repo、兩個 Coder 各兩張小卡，檢查分支、commit、光環、失敗暫停、重試、跳過、合併。

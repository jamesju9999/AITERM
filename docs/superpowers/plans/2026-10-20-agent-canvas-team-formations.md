# 團隊編制切換 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一個專案可以存好幾組團隊編制並依階段切換；切換時保留卡片、記錄與專案設定，有人在工作時可以「做完後切換」。

**Architecture:** 新的後端模組 `src-tauri/src/tasks/formations.rs` 持有三張表：`formations`、`formation_log`、`retired_agents`，以及切換邏輯。切換沿用 `delete_all_agents` 和 `create_agent`。排程器在等待切換期間不開始新的模組卡，並在每一輪完成待切換。前端新增 `FormationMenu` 元件和 `src/ipc/formations.ts`，團隊範本對話框加上「套用成新編制」。

**Tech Stack:** Rust（sqlx、SQLite、Tauri 2 指令）、React 19、Vitest＋RTL。

規格：`docs/superpowers/specs/2026-10-20-agent-canvas-team-formations-design.md`

---

## 檔案

| 檔案 | 責任 |
|---|---|
| Create `src-tauri/src/tasks/formations.rs` | 資料表、編制 CRUD、快照、忙碌判定、切換、等待切換、退役 Agent 名稱 |
| Modify `src-tauri/src/tasks/mod.rs` | `pub mod formations;`、在 `init_schema` 呼叫 `formations::init_schema` |
| Modify `src-tauri/src/tasks/agents.rs` | `delete_agent` 在同一個交易裡記下退役 Agent 的名稱與種類 |
| Modify `src-tauri/src/tasks/scheduler.rs` | 等待切換時 `drain_once` 不派 Agent 的新模組卡；主迴圈每輪呼叫 `formations::try_complete_pending` |
| Create `src-tauri/src/commands/formations.rs` | Tauri 指令 |
| Modify `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs` | 註冊指令 |
| Create `src/ipc/formations.ts` | 型別與 invoke 包裝 |
| Create `src/components/AgentCanvas/FormationMenu.tsx` (+ test) | 編制選單：切換、新增、另存、改名、刪除；等待切換的提示 |
| Modify `src/components/AgentCanvas/index.tsx` (+ test) | 載入編制狀態、放選單、活動記錄加切換紀錄、退役 Agent 名稱 |
| Modify `src/components/AgentCanvas/TeamTemplateDialog.tsx` (+ test) | 「套用成新編制」 |
| Modify `src/components/AgentCanvas/teamTemplates.ts` (+ test) | `toFormationAgents` |
| Modify `src/lib/i18n.ts` | zh-TW／en 字串 |

## 資料形狀

Rust：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormationAgent {
    pub kind: String,
    pub name: String,
    pub role: String,
    #[serde(default)] pub prompt_prefix: String,
    #[serde(default)] pub model: String,
    #[serde(default = "default_cli")] pub cli: String,
    #[serde(default)] pub produces_spec: bool,
    /// 桌布上的絕對位置。
    pub dx: f64,
    pub dy: f64,
    #[serde(default)] pub db_skill: Option<String>,
    #[serde(default)] pub custom_skills: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormationSummary { pub id: String, pub name: String, pub agent_count: usize }

#[derive(Debug, Clone, Serialize)]
pub struct FormationLogRow { pub at: i64, pub from_name: Option<String>, pub to_name: String }

#[derive(Debug, Clone, Serialize)]
pub struct FormationsState {
    pub formations: Vec<FormationSummary>,
    pub active: Option<String>,
    pub pending: Option<String>,
    /// 還在工作、擋住切換的 Agent 名稱（畫面上顯示等誰）。
    pub busy: Vec<String>,
    pub log: Vec<FormationLogRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SwitchOutcome { Switched, Busy { names: Vec<String> } }

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct RetiredAgent { pub id: String, pub name: String, pub kind: String }
```

TS 對應寫在 `src/ipc/formations.ts`。

---

### Task 1：資料表與編制 CRUD

**Files:** Create `src-tauri/src/tasks/formations.rs`; Modify `src-tauri/src/tasks/mod.rs`

- [ ] 寫測試（`formations.rs` 的 `#[cfg(test)] mod tests`，用 `sqlite::memory:` 加 `crate::tasks::init_schema`）：
  - `create_list_rename_delete`：建兩組並列出（依 `sort_order`），改名，刪除非目前的那組。
  - `cannot_delete_active`：`set_active(a)` 之後 `delete(a)` 回 Err。
  - `agents_json_round_trips`：含 `db_skill`／`custom_skills` 的 `FormationAgent` 存進去再讀出來相等。
- [ ] 跑 `cd src-tauri && cargo test -p app formations` → 編譯失敗（模組不存在）。
- [ ] 實作：

```rust
pub async fn init_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("CREATE TABLE IF NOT EXISTS formations (
        id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, agents TEXT NOT NULL,
        sort_order REAL NOT NULL, updated_at INTEGER NOT NULL)").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS formation_log (
        at INTEGER NOT NULL, from_name TEXT, to_name TEXT NOT NULL)").execute(pool).await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS retired_agents (
        id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, retired_at INTEGER NOT NULL)").execute(pool).await?;
    Ok(())
}
pub async fn create(pool, name: &str, agents: &[FormationAgent]) -> Result<String, sqlx::Error>   // sort_order = MAX+1
pub async fn list(pool) -> Result<Vec<FormationSummary>, sqlx::Error>
pub async fn agents_of(pool, id) -> Result<Option<Vec<FormationAgent>>, sqlx::Error>
pub async fn rename(pool, id, name) -> Result<(), sqlx::Error>
pub async fn delete(pool, id) -> Result<(), String>   // 目前的編制 → Err("不能刪除目前的編制")
pub async fn active(pool) / set_active(pool, id) / pending(pool) / set_pending(pool, Option<&str>)
// active/pending 存 project_settings 的 active_formation／pending_formation
```

- [ ] 跑測試 → 通過。Commit `feat(formations): formation storage`.

### Task 2：快照、忙碌判定、退役名稱

**Files:** Modify `formations.rs`, `agents.rs`

- [ ] 寫測試：
  - `snapshot_captures_skills_and_positions`：建 Agent，用 `db_skill::`／`custom_skills::` 的 UPDATE 寫入技能、設定 model／cli／produces_spec，`snapshot(pool)` 的欄位全部對得上。
  - `busy_counts_running_and_queued_pipeline_but_not_paused_or_queued_work`：running 的 work 算忙；queued 的 review／fix／merge／verify／plan 算忙；queued 的 work 不算；Agent 被暫停時它排隊中的 review 不算。
  - `delete_agent_remembers_name`（放在 agents.rs）：刪掉後 `formations::retired(pool)` 有 `{id, name, kind}`。
- [ ] 跑 → 失敗。
- [ ] 實作：

```rust
pub async fn snapshot(pool) -> Result<Vec<FormationAgent>, sqlx::Error>  // 依 list_agents，dx/dy = canvas_x/canvas_y
pub async fn busy_agents(pool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT DISTINCT a.name FROM tasks t JOIN agents a ON a.id = t.agent_id
        WHERE t.archived_at IS NULL AND (t.status = 'running'
          OR (t.status = 'queued' AND t.card_kind IN ('review','fix','merge','verify','plan') AND a.paused_task_id IS NULL))
        ORDER BY a.created_at").fetch_all(pool).await
}
pub async fn retired(pool) -> Result<Vec<RetiredAgent>, sqlx::Error>
```

  `agents::delete_agent` 在 `DELETE FROM agents` 之前執行 `INSERT OR REPLACE INTO retired_agents SELECT id, name, kind, strftime('%s','now') FROM agents WHERE id = ?`（同一個 tx）。
- [ ] 跑 → 通過。Commit `feat(formations): snapshot, busy check, retired agent names`.

### Task 3：切換與等待切換

**Files:** Modify `formations.rs`

- [ ] 寫測試：
  - `switch_saves_current_then_builds_target`：目前編制 A 有兩位 Agent（含技能），切到 B（一位審核者）：A 的 agents JSON 更新成切換前的桌布；桌布上只剩 B 的審核者；`active` = B；`formation_log` 一筆 A→B。
  - `switch_back_restores_skills`：A→B→A，回來的 Agent 技能、模型、位置跟離開前一樣（id 不同）。
  - `switch_refuses_when_busy`：有 running 卡 → `SwitchOutcome::Busy{names}`，桌布不變。
  - `switch_needs_repo_for_non_empty_target`：`repo = None` 且目標有 Agent → Err，桌布不變。
  - `queued_work_returns_to_library`：切換後原本排隊中的 work 卡 `agent_id IS NULL`、`status = planning`。
  - `try_complete_pending`：設 pending；忙碌時回 `Ok(false)` 且沒切換；不忙時切換、清掉 pending、回 `Ok(true)`。
  - `ensure_default_only_when_agents_and_no_formations`：沒有 Agent 不建；有 Agent 時建一組並設為目前；再呼叫一次不重複建立。
- [ ] 跑 → 失敗。
- [ ] 實作：

```rust
/// 目前的程式庫：桌布上 Agent 的 → 專案記住的；要是可用的 git repo。
pub async fn resolve_repo(pool) -> Result<Option<String>, String>

pub async fn switch_to(pool, target: &str, repo: Option<&str>) -> Result<SwitchOutcome, String> {
    let busy = busy_agents(pool).await?;  if !busy.is_empty() { return Ok(Busy{names: busy}) }
    let target_agents = agents_of(pool, target)?.ok_or("找不到這組編制")?;
    if !target_agents.is_empty() && repo.is_none() { return Err("這組編制要先設定專案資料夾") }
    let from = active(pool)?;
    if from.as_deref() == Some(target) { return Ok(Switched) }
    if let Some(f) = &from { save_agents(pool, f, &snapshot(pool)?) }
    agents::delete_all_agents(pool)?;
    for a in &target_agents { build(pool, a, repo.unwrap()) }   // create_agent＋set_produces_spec/model/cli＋技能 UPDATE
    set_active(pool, target); set_pending(pool, None); 寫 formation_log(from 的名稱, target 的名稱)
    Ok(Switched)
}

/// 把目前桌布另存成新編制並設為目前（舊的那組先存下最新的桌布）。
pub async fn save_as(pool, name) -> Result<String, sqlx::Error>

/// 排程器每輪呼叫：有 pending 且不忙就切換。回傳有沒有切換。
pub async fn try_complete_pending(pool) -> Result<bool, String>

/// 舊專案：沒有任何編制但桌布上有 Agent → 用目前桌布建立 `name` 並設為目前。
pub async fn ensure_default(pool, name) -> Result<(), sqlx::Error>

pub async fn state(pool) -> Result<FormationsState, String>   // log 取最近 20 筆
```

- [ ] 跑 → 通過。Commit `feat(formations): switch, pending switch, default formation`.

### Task 4：排程器

**Files:** Modify `scheduler.rs`

- [ ] 寫測試（scheduler 的測試模組）：`holds_new_agent_work_while_switch_pending`。用純函式 `hold_for_pending(heads, pending: bool)` 測：pending 時保留 review／fix／merge／verify／plan 和未指派的看板卡，濾掉 Agent 的 work 卡；不 pending 時原樣。
- [ ] 跑 → 失敗。
- [ ] 實作 `pub fn hold_for_pending(mut heads: Vec<TaskRow>, pending: bool) -> Vec<TaskRow>`。在 `drain_once` 的 `heads.retain(... quota_cooling ...)` 後面：

```rust
let pending = matches!(crate::tasks::formations::pending(&project.pool).await, Ok(Some(_)));
heads = hold_for_pending(heads, pending);
```

  主迴圈在 `drain_once` 之前：

```rust
for project in reg.all() {
    match crate::tasks::formations::try_complete_pending(&project.pool).await {
        Ok(true) => { let _ = app.emit("tasks-updated", ()); }
        Ok(false) => {}
        Err(e) => eprintln!("formation pending switch ({}): {e}", project.name),
    }
}
```

- [ ] 跑 `cargo test -p app scheduler` → 通過。Commit `feat(formations): scheduler holds new work and completes pending switch`.

### Task 5：指令與 IPC

**Files:** Create `src-tauri/src/commands/formations.rs`、`src/ipc/formations.ts`；Modify `commands/mod.rs`、`lib.rs`

指令（每個變動後 `emit_updated(&app)`，並 `SchedulerHandle::poke()` 讓排程器接手）：

- `formations_state(project_id, default_name) -> FormationsState`：先 `ensure_default`。
- `formations_create(project_id, name, agents: Vec<FormationAgent>) -> String`
- `formations_save_as(project_id, name) -> String`
- `formations_rename(project_id, id, name)`、`formations_delete(project_id, id)`
- `formations_switch(project_id, id) -> SwitchOutcome`：`resolve_repo` 後 `switch_to`。
- `formations_request_switch(project_id, id)`：設 pending，並立刻試 `try_complete_pending`。
- `formations_cancel_switch(project_id)`
- `agents_retired(project_id) -> Vec<RetiredAgent>`

TS：

```ts
export interface FormationAgent { kind: AgentKind; name: string; role: string; prompt_prefix: string; model: string; cli: AgentCli;
  produces_spec: boolean; dx: number; dy: number; db_skill?: string | null; custom_skills?: string | null }
export interface FormationsState { formations: { id: string; name: string; agent_count: number }[]; active: string | null;
  pending: string | null; busy: string[]; log: { at: number; from_name: string | null; to_name: string }[] }
export type SwitchOutcome = { status: "switched" } | { status: "busy"; names: string[] };
export interface RetiredAgent { id: string; name: string; kind: AgentKind }
```

- [ ] `cd src-tauri && cargo check` 與 `npx tsc -b` 通過。Commit `feat(formations): commands and IPC`.

### Task 6：前端編制選單

**Files:** Create `FormationMenu.tsx`、`FormationMenu.test.tsx`；Modify `index.tsx`、`index.test.tsx`、`i18n.ts`

`FormationMenu` props：`{ state: FormationsState; onSwitch(id); onCreate(name); onSaveAs(name); onRename(id, name); onDelete(id); onCancelPending() }`。

- 按鈕文字 `t.ac_form_button(目前名稱 ?? t.ac_form_none)`，點開下拉清單。
- 清單的每組顯示名稱與人數，目前的那組有 ✓。
- 新增、另存、改名用清單內的輸入框（Tauri 沒有 `window.prompt`），Enter 送出、Esc 取消。
- 刪除：目前的那組不顯示刪除鈕。
- `pending` 時按鈕旁顯示 `t.ac_form_pending(目標名稱, busy.join("、"))` 和「取消切換」。

測試（FormationMenu.test.tsx）：

- 顯示目前名稱與 ✓。
- 點另一組會呼叫 `onSwitch(id)`。
- 新增：輸入名稱後按 Enter，呼叫 `onCreate`；空白名稱不送。
- 目前那組沒有刪除鈕。
- pending 時顯示等誰，按取消會呼叫 `onCancelPending`。

`index.tsx`：

- 每次 `refresh` 一併載入 `formationsState(projectId, t.ac_form_default)` 與 `listRetired`。
- 切換流程：

```ts
const switchFormation = (id: string) => void run(async () => {
  const o = await switchFormation(projectId, id);
  if (o.status === "busy") {
    const ok = await confirm(t.ac_form_busy_confirm(o.names.join("、")), { kind: "warning" });
    if (ok) await requestFormationSwitch(projectId, id);
  }
});
```

  新增空白編制 = `createFormation(name, [])` 後切換。
- `agentName`／role 找不到 Agent 時退回 `retired`。
- 活動記錄加入 `log`：`t.ac_log_formation(from, to)`，kind 是 `"dispatch"`。

`index.test.tsx`：mock `../../ipc/formations`，加兩條測試：

- 切換時遇到 busy，確認後呼叫 `requestFormationSwitch`。
- 已刪除 Agent 完成的卡在活動記錄顯示退役名稱，而不是「?」。

- [ ] 先寫測試看它紅，再實作，跑 `npx vitest run src/components/AgentCanvas` 通過。Commit `feat(formations): formation menu on the canvas`.

### Task 7：範本「套用成新編制」

**Files:** Modify `teamTemplates.ts`(+test)、`TeamTemplateDialog.tsx`(+test)、`index.tsx`

- `toFormationAgents(tpl, x, y): FormationAgent[]`：位置是 `x + dx`、`y + dy`，`cli ?? "claude"`。測試位置、cli 預設值，以及不跳過單一角色（新編制是空桌布）。
- 對話框每個範本多一個按鈕 `t.ac_tpl_apply_new`，呼叫 `onApplyAsFormation(tpl, repo)`，跟「套用」用同一個 `repoOk` 條件。測試按下後會呼叫回呼，並帶程式庫路徑。
- `index.tsx`：

```ts
onApplyAsFormation={async (tpl, repoDir) => {
  await setProjectRepo(projectId, repoDir);
  const id = await createFormation(projectId, uniqueFormationName(tpl.name), toFormationAgents(tpl, 40, 40));
  await doSwitchFormation(id);
}}
```

- [ ] 紅 → 綠。Commit `feat(formations): apply a template as a new formation`.

### Task 8：驗證

- [ ] `npx tsc -b`、`npm run lint`（比對基準 137 個問題，改到的檔案要乾淨）、`npm run test`。
- [ ] `cd src-tauri && cargo test --workspace --no-fail-fast`，逐行看每個 `test result:`。
- [ ] 更新規格狀態為「已實作」；兩項待驗證寫上結果：退役名稱已處理，磁碟用量維持不處理。
- [ ] Commit。

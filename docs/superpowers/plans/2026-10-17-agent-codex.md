# Agent 協作支援 Codex — 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 每位 Agent 可選 Claude Code 或 Codex；Codex 卡片用 `codex exec` 非互動執行，ask_human、token、額度用完、App 重開接著做都能用。

**Architecture:** 新增純函式模組 `tasks/codex.rs` 組指令、解析 rollout；`RealDispatcher::dispatch` 依 `agent.cli` 分流，Codex 走新的 `dispatch::spawn_command`（只寫指令、不打提示詞）；`QuotaState` 加「SessionStart 回報」與「會等答案的 ask_human」；結案時讀 rollout 判斷額度用完 → 回佇列並冷卻 10 分鐘。

**Tech Stack:** Rust（tauri、sqlx、axum、tokio）、React＋TypeScript、Vitest。

設計：`docs/superpowers/specs/2026-10-17-agent-codex-design.md`。探勘結果：記憶 `project_codex_dispatch_probe`。

驗證指令：
- Rust：`cd src-tauri && CARGO_TARGET_DIR=/Users/jamesju/Documents/GitHub/AITERM-wt-quota/src-tauri/target cargo test --workspace --no-fail-fast`（看每一行 `test result:`）
- 前端：`npx vitest run`、`npx tsc -b`、`npx eslint <檔案>`
- 每個新測試都做突變檢查（備份用 `mktemp`）。

---

### Task 1：agents 表的 `cli` 欄位

**Files:** `src-tauri/src/tasks/mod.rs`（migration）、`src-tauri/src/tasks/agents.rs`、`src-tauri/src/commands/agents.rs`、`src/ipc/agents.ts`

- [ ] 測試（agents.rs tests）：新 Agent 的 `cli` 是 `"claude"`；`set_cli(&pool, &id, "codex")` 後讀回 `"codex"`；`set_cli(..., "bogus")` 回 Err。
- [ ] migration：`ALTER TABLE agents ADD COLUMN cli TEXT NOT NULL DEFAULT 'claude'`（跟 `model` 一樣忽略已存在錯誤）。
- [ ] `AgentRow` 加 `pub cli: String`；`pub async fn set_cli(pool, id, cli) -> Result<(), sqlx::Error>`，只接受 `claude`／`codex`。
- [ ] `CreateAgentArgs`、`UpdateAgentArgs` 加 `#[serde(default)] pub cli: Option<String>`；有值就 `set_cli`。
- [ ] 前端 `AgentRow.cli?: "claude" | "codex"`、`NewAgentArgs.cli?`、`UpdateAgentArgs.cli?`。
- [ ] 跑測試、突變（拿掉白名單檢查）、commit。

### Task 2：`tasks/codex.rs` 純函式

**Files:** Create `src-tauri/src/tasks/codex.rs`；`src-tauri/src/tasks/mod.rs` 加 `pub mod codex;`

```rust
pub struct CodexLaunch<'a> {
    pub model: Option<&'a str>,
    pub prompt_file: &'a Path,
    pub resume_session: Option<&'a str>,
    pub hook_command: Option<&'a str>,
    pub ask_url: Option<&'a str>,
    pub playwright_args: Option<&'a [String]>,
    pub disable_mcp: &'a [String],
}
pub fn toml_str(s: &str) -> String            // "\"...\""，跳脫 \ 與 "
pub fn launch_command(o: &CodexLaunch) -> Option<String>   // 路徑有特殊字元就 None
pub fn hook_command(base: &str) -> String     // curl[.exe] -s -m 5 -X POST -H 'Content-Type: application/json' --data-binary @- <base>/codex-session
pub fn parse_usage(text: &str) -> Option<TokenUsage>
pub enum CodexError { UsageLimit(String), Other(String) }
pub fn last_error(text: &str) -> Option<CodexError>
pub fn user_mcp_servers(config_toml: &str) -> Vec<String>
pub fn is_rollout_path(p: &str) -> bool
```

指令形狀（一行，shell 參數用單引號包 `-c` 的值；Windows PowerShell 單引號同樣是字面字串）：

```
codex exec [resume <sid>] --sandbox workspace-write -c sandbox_workspace_write.network_access=true
  [-m <model>] [--dangerously-bypass-hook-trust -c 'hooks.SessionStart=[{hooks=[{type="command",command="<hook>"}]}]']
  [-c 'mcp_servers.aiterm.url="<url>"' -c 'mcp_servers.aiterm.default_tools_approval_mode="approve"' -c mcp_servers.aiterm.tool_timeout_sec=86400]
  [-c 'mcp_servers.playwright.command="npx"' -c 'mcp_servers.playwright.args=[...]' -c 'mcp_servers.playwright.default_tools_approval_mode="approve"']
  [-c mcp_servers.<user>.enabled=false ...]
  '請讀 <prompt_file> 並照裡面的指示完成工作。'
```

（`codex exec resume` 的參數順序：選項放在 `resume <sid>` 之後、提示詞之前，實作時用 `codex exec resume --help` 確認。）

- [ ] 測試：一般、resume、ask＋approve＋逾時、playwright、關使用者 MCP、`toml_str` 跳脫、模型名不合法不加 `-m`、提示路徑含單引號回 None。
- [ ] 測試 `parse_usage`／`last_error`：fixture 用探勘的真實行（成功：兩筆 token_count，最後 total input 42121、cached 21011、output 29；額度：`task_complete.error.codex_error_info=usage_limit_exceeded`）。
- [ ] 測試 `user_mcp_servers`：`[mcp_servers.pencil]`、`[mcp_servers.node_repl.env]`（子表不算新名字）→ `["pencil","node_repl"]`。用 `toml` crate（已在依賴就用，否則手寫逐行解析 `[mcp_servers.<name>]`）。
- [ ] 實作、突變、commit。

### Task 3：QuotaState — SessionStart 回報與會等答案的 ask_human

**Files:** `src-tauri/src/tasks/quota.rs`、`src-tauri/src/tasks/store.rs`（已有 set_session_id／set_session_path）、`src-tauri/src/commands/questions.rs`、`src-tauri/src/tasks/questions.rs`

- [ ] `register(task_id, tab_id, pool, label, blocking_ask: bool)`；Entry 加 `blocking_ask` 與 `waiters: HashMap<i64, oneshot::Sender<String>>`（解除登記時一起丟掉 → 等待端收到取消）。更新所有呼叫端。
- [ ] hook 路徑 `codex-session`：body 取 `session_id`、`transcript_path`，寫進卡片（`store::set_session_id`、`store::set_session_path`）。
- [ ] `ask()`：`blocking_ask` 時建立問題、`set_asking(true)`、通知，然後 await waiter；收到答案回 `questions::answer_text(&row_with_answer)`，取消回 `("問題已取消（卡片已結束）", true)`。
- [ ] `pub fn give_answer(&self, task_id, question_id, text) -> bool`：有 waiter 就送出並回 true。
- [ ] `questions::answer_text(q)`：「{label} 的回答：{answer}\n（請依這個答案繼續。）」。
- [ ] `deliver_answer`：先 `quota.give_answer(...)`；true 就不寫 PTY；false 且卡片 Agent 是 codex → Err「Agent 目前沒有在等這個答案」。
- [ ] 測試：codex-session 寫入兩欄；blocking ask 在 give_answer 後回答案文字；unregister 後回取消；非 blocking 行為不變。
- [ ] 突變、commit。

### Task 4：PTY 額外環境變數與 `spawn_command`

**Files:** `src-tauri/src/pty/manager.rs`、`src-tauri/src/tasks/dispatch.rs`

- [ ] `create_with_app_env(..., bridge_env, extra_env: Vec<(String,String)>)`，`create_with_app` 呼叫它並傳空 Vec。
- [ ] `parse_env_lines(text) -> Vec<(String,String)>`（dispatch.rs）：`KEY=VALUE` 每行一組，忽略空行與 `#`，KEY 只收 `[A-Za-z_][A-Za-z0-9_]*`。測試＋突變。
- [ ] `spawn_command(app, pty, dir, line, extra_env, on_spawned) -> Result<(String, DispatchResult), String>`：建分頁（帶 extra_env）、`on_spawned`、寫 `line\r`、送 `mcp-coordination-tab-spawned`；baseline 取當下的 bell／marker 計數。
- [ ] commit。

### Task 5：派工分流（scheduler.rs）

**Files:** `src-tauri/src/tasks/scheduler.rs`、`src-tauri/src/tasks/dispatch.rs`（RESUME 提示）

- [ ] `AUTONOMY_NOTE_ASK_WAIT`（Codex 版自主說明）＋ `agent_prompt` 多一個參數 `ask_waits: bool`。測試。
- [ ] `RESUME_PROMPT_CODEX`：「AITerm 剛才重新啟動，你的工作被中斷了。先檢查目前進度（看檔案、git status）再把這張卡做完；之前問的問題已取消，需要的話用 ask_human 重新提問。」
- [ ] `dispatch()`：在取得 `agent` 之後判斷 `codex = agent.cli == "codex" && !task.interactive`。Codex 路徑：
  - session id：resume 時用 `task.session_id`（且 `task.resume_session`），否則 None。
  - ask_url 照舊取得；驗收卡 playwright 參數由 `ask::playwright_args(evidence, browser)`（從 `mcp_config_json_for` 抽出共用）。
  - hook：`quota.hook_base(&task.id)` → `codex::hook_command`。
  - 提示寫 `tasks/<id>/prompt.md`；`launch_command` 回 None 時回 Err「路徑含特殊字元，無法派給 Codex」。
  - 精簡啟動：讀 `~/.codex/config.toml` → `user_mcp_servers`。
  - extra_env：`parse_env_lines(secrets.get(CODEX_ENV_KEY))`。
  - register 帶 `blocking_ask = true`；`spawn_command` 取代 `spawn_and_run`。
  - 不呼叫 `store::set_session_id`（SessionStart 回報會寫）。
- [ ] watch 結束後、`finalize_*` 之前：Codex 卡片且 outcome 是 Failed → 讀 `session_path` → `codex::last_error`：
  - `UsageLimit` → `store::requeue_for_quota(pool, task_id, now)`（status queued、resume_session=1、quota_wait_at=now、tab_id NULL、排到該 Agent 最前面），**跳過** finalize／persist_outcome／after_card_finished，`wake`。
  - `Other(m)` → `TaskOutcome::Failed(m)`。
- [ ] `drain_once`：`heads.retain(|h| !quota_cooling(h, now))`，`quota_cooling` = `quota_wait_at` 在 `RETRY_AFTER` 內。測試：冷卻中不派、滿 10 分鐘才派。
- [ ] `recover_after_restart`：Codex Agent 的卡 → resume 判斷看 `session_path` 檔案存在；並取消它沒答的問題。測試。
- [ ] commit。

### Task 6：token 統計讀 Codex rollout

**Files:** `src-tauri/src/tasks/usage.rs`

- [ ] `task_usage`：Claude 路徑找不到時，`session_path` 是 rollout（`codex::is_rollout_path`）→ 讀檔 `codex::parse_usage`。結案時 `persist_outcome` 照樣存 usage.json（確認 Codex 卡也走得到；若 persist 只在 Claude 時存，補上）。測試、commit。

### Task 7：設定頁「Codex 環境變數」

**Files:** `src-tauri/src/commands/tasks.rs`（或新 commands）、`src-tauri/src/lib.rs`（註冊）、`src/ipc/tasks.ts`、`src/components/Settings/TaskBoardPage.tsx`、`src/lib/i18n.ts`

- [ ] 指令 `codex_env_status() -> bool`、`codex_env_set(text: String)`（空字串＝刪除）。key 常數 `CODEX_ENV_KEY = "task_board.codex_env"`。
- [ ] 設定頁「執行環境」區加一個多行輸入＋「儲存」；已設定時顯示「已設定（內容不顯示）」與「清除」。測試、commit。

### Task 8：前端 Agent 設定、卡片、範本

**Files:** `src/components/AgentCanvas/AgentConfigDialog.tsx`、`AgentNode.tsx`、`teamTemplates.ts`、`index.tsx`（建立/套用範本時帶 cli）、`src/lib/i18n.ts`

- [ ] 對話框「AI 工具」兩顆選項；選 Codex 時模型必填（送出鈕 disabled＋提示）、模型建議清單換成空（不給 Claude 的 opus/sonnet）。
- [ ] 卡片副標題：Codex 時模型小標籤顯示 `Codex · <model>`。
- [ ] 範本 snapshot／apply 帶 `cli`（舊範本無 cli → claude）。
- [ ] 測試：對話框選 Codex 未填模型不能送出、送出時帶 `cli: "codex"`；卡片顯示 Codex；範本 roundtrip。突變、commit。

### Task 9：完整驗證與實機

- [ ] Rust 全部測試、前端全部測試、tsc、eslint。
- [ ] dev App 實機（Ask Test）：Codex 工程師（oMLX、Qwen3.6-35B-A3B-4bit，設定頁填 OMLX_API_KEY）＋既有 Claude 審核者／協調者／驗收者，派一個小模組，確認派工、session 回報、commit、審查、合併、token 顯示。
- [ ] 額度用完：使用者的 Codex 供應商是 oMLX（設計 A：沿用使用者 config，不在 AITerm 切供應商），App 內無法讓卡片改走 ChatGPT 帳號。這條路徑用「真實 rollout 片段」的單元測試＋結案分類測試覆蓋，並在回報中註明未在 App 內實測。
- [ ] App 重開時 Codex 卡接著做。
- [ ] 更新記憶、回報。

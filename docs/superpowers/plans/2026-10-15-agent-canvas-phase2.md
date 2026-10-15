# Agent Canvas 第二期 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reviewer 每模組自動審查、不通過自動退回（最多 2 輪）、全部通過後 Orchestrator 自動合併；桌布連線與光棒。

**Architecture:** 新模組 `src-tauri/src/tasks/workflow.rs` 在每張 Agent 卡片結案後建立後續卡片（review／fix／merge）；
派工與 finalize 依 Agent kind 分流（reviewer 用 detached worktree、讀 review.json）。前端加 `deriveModuleState`、`EdgeLayer`。

Spec：`docs/superpowers/specs/2026-10-15-agent-canvas-phase2-design.md`

每個任務：先寫會紅的測試 → 實作 → 綠 → commit。

### Task 1: schema
`tasks` 加 `card_kind/module_id/review_of/verdict/review_reasons/module_override`、`agents` 加 `last_merge_signature`；
`TaskRow`／`AgentRow` 加欄位；更新 scheduler 測試 literal。測試：舊 schema migration 讀寫新欄位。

### Task 2: workflow — module_state
`pub enum ModuleState`、`pub fn module_state(module: &TaskRow, chain: &[TaskRow]) -> ModuleState`（純函式），各分支測試。

### Task 3: workflow — 建卡函式與 after_card_finished
`create_review_card`、`create_fix_card`、`maybe_enqueue_merge`、`after_card_finished`；提示詞模板函式。
測試：spec「測試」一節的 Rust 項目（mem pool，不碰 git）。

### Task 4: review worktree 與 reviewer finalize
`git.rs`：`add_detached_worktree(path, sha)`、`remove_worktree_force(path)`；
scheduler：派工依 kind 決定 cwd（reviewer → detached worktree）、finalize 依 kind 分流（`parse_review_json`）；
watch closure 呼叫 `workflow::after_card_finished`，Attention 事件發 `task-finished`。
測試：tempdir repo 建立／清除 detached worktree；`parse_review_json`；reviewer finalize 寫 verdict、缺檔暫停。

### Task 5: 指令
`agents_create` 開放 reviewer/orchestrator（每種一個，store 函式 `count_kind` 測試）；`agents_override_module`；
`tasks_assign_agent` 只能給 coder（store 層檢查＋測試）。

### Task 6: 前端 IPC 與 deriveModuleState
`ipc/tasks.ts` TaskRow 新欄位、fixtures 補欄位；`ipc/agents.ts` 加 `overrideModule`；
`AgentCanvas/deriveModuleState.ts`（與 Rust 同規則）＋測試；`deriveAgentStatus` 依 kind 給 reviewing/merging。

### Task 7: 前端 UI
Agent 池三種、對話框角色預設依 kind、Coder chip 改每模組一顆＋放行、Reviewer／Orchestrator 卡內容、
`EdgeLayer.tsx`（邊、流動、一次性光點）＋測試；i18n。

### Task 8: 驗證
全部測試、tsc、lint（新檔）；假 claude 擴充後實機跑完整流程。

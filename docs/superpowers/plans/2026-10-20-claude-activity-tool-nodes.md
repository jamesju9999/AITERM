# 工具呼叫節點 Implementation Plan

> 依 spec `docs/superpowers/specs/2026-10-20-claude-activity-tool-nodes-design.md`；TDD，逐任務提交。

**Goal:** 每次工具呼叫在協作圖上是一個節點。
**Architecture:** Rust `Tree` 記 `ToolCall`；前端 `toolNodes.ts` 把工具轉成偽節點餵給既有 `layoutGraph`；GraphCanvas/DetailPane 依 id 前綴換卡片樣式。
**Tech Stack:** Rust, React 19, Vitest。

### Task 1: Rust 資料與分類（紅燈先行）
`category`/`tool_hint` 單元測試；Pre/Post/PostFail 狀態機測試；Prompt 中斷、SessionEnd 清空、上限 300。
### Task 2: Hook 接線
settings 加 `PostToolUseFailure`→`post-fail`；`Kind::parse`；更新 settings 測試；`real_claude_pipeline` 驗證真 claude 的事件。
### Task 3: 前端型別與 `toolNodes.ts`
視窗化（最近 6）與「更早 N 個」群組、父節點被隱藏時丟棄。
### Task 4: UI
卡片、詳情、工具開關、展開全部、i18n、CSS。
### Task 5: 全套驗證與提交

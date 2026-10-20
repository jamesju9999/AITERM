# Claude 協作圖實用化 Implementation Plan

> 依 spec `docs/superpowers/specs/2026-10-20-claude-activity-actionable-design.md`，TDD，逐任務提交。

**Goal:** 通知（失敗／全完成／卡住）、token 排行、主 Claude 活動。
**Architecture:** Rust `Tree` 新增 `last_event_ms`、`main_tool_count`、`main_recent`；前端純函式 `tokenStats`、`detectAlerts` + hook `useClaudeActivityAlerts`；面板/詳情/根卡片顯示。
**Tech Stack:** Rust (serde), React 19, Vitest.

### Task 1: Rust — last_event_ms、主活動計數
測試先紅：節點事件更新 last_event_ms；主工具 pre 增加計數並記 recent（上限 10）；Prompt/SessionEnd 重置；Agent 工具算一次。
### Task 2: 前端型別與 token 統計
`ActivityNode.last_event_ms`、`ActivitySnapshot.main_tool_count/main_recent`；`tokens.ts` 的 `tokenStats` 測試。
### Task 3: 警示偵測純函式與 hook
`alerts.ts`（`detectAlerts(prev, next, now, stuckNotified)`）；`useClaudeActivityAlerts` 在 TerminalView 掛載；設定開關。
### Task 4: UI
標頭合計、最耗徽章、詳情百分比、根卡片次數、點根看最近工具。i18n 兩語系。
### Task 5: 全套驗證與提交
vitest、tsc、cargo test --workspace --no-fail-fast、突變檢查。

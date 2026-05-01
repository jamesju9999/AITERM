## 1. Rust 型別（Backend）

- [x] 1.1 在 `src-tauri/src/vcs/types.rs` 新增 `VcsAgentHistoryEntry` enum（User { text } / Step { step_num, operation, result_json, summary }）
- [x] 1.2 新增 `VcsAgentDecision` struct（done: bool, intent: Option<VcsIntent>, summary: String, final_answer: Option<String>）
- [x] 1.3 為 `VcsAgentHistoryEntry` 和 `VcsAgentDecision` 加入 TOML/JSON roundtrip 測試

## 2. vcs_agent_step Command（Backend）

- [x] 2.1 在 `src-tauri/src/commands/vcs.rs` 新增 `parse_vcs_agent_decision(response: &str) -> Result<VcsAgentDecision, String>`（複用現有 fence-stripping 邏輯）
- [x] 2.2 新增 `vcs_agent_step(goal, history, repo_info, session_id, provider_id?)` command：建構 AI system prompt（含 goal、history、VcsIntent schema），呼叫 AI，解析回傳的 `VcsAgentDecision`
- [x] 2.3 在 `src-tauri/src/lib.rs` 加入 `vcs_agent_step` 至 invoke_handler

## 3. IPC Wrapper（Frontend）

- [x] 3.1 在 `src/ipc/vcs.ts` 新增 `VcsAgentHistoryEntry`、`VcsAgentDecision` TypeScript 型別
- [x] 3.2 新增 `vcsAgentStep(goal, history, repoInfo, sessionId, providerId?)` IPC wrapper function

## 4. useVcsAgentLoop Hook（Frontend）

- [x] 4.1 建立 `src/hooks/useVcsAgentLoop.ts`：管理 `goal`、`messages`（`VcsChatMessage[]`）、`isRunning`、`stepCount`、`maxSteps`（從 config 讀取）
- [x] 4.2 實作 `send(text)` 邏輯：若 isRunning=false 開始新 loop；若 isRunning=true 注入 user message 並於當前步驟後繼續
- [x] 4.3 實作 loop 核心：呼叫 `vcsAgentStep` → 若 done=false 執行 intent（透過 `vcsQuery`）→ append step to history → 繼續；若 done=true 顯示 final answer
- [x] 4.4 實作步驟上限檢查：stepCount >= maxSteps 時終止 loop 並附上上限提示
- [x] 4.5 實作 `stop()` 函式：設定停止 flag，在當前步驟完成後不再呼叫下一步
- [x] 4.6 加入 `mountedRef` guard（比照 `useAiChat.ts`）

## 5. VcsMessageBubble 更新（Frontend）

- [x] 5.1 在 `src/components/VcsView/VcsMessageBubble.tsx` 新增 `step` message variant 的渲染：顯示步驟編號（Step N/M）、⚙️ commandDisplay、💬 aiSummary、VcsResult 卡片
- [x] 5.2 新增 `final-answer` variant：✅ 綠色卡片顯示 final_answer 文字
- [x] 5.3 新增 `step-limit-reached` variant：⚠️ 黃色卡片顯示步驟上限提示
- [x] 5.4 新增 `stopped` variant：灰色「已停止」提示

## 6. VcsView 更新（Frontend）

- [x] 6.1 在 `src/components/VcsView/VcsView.tsx` 替換 `useVcsChat` 為 `useVcsAgentLoop`
- [x] 6.2 新增 provider 選擇器（比照 `DatabaseAiChat.tsx` 的 `selectedProviderId` + `<select>` 下拉 pattern）：讀取 `listProviders()`，渲染於 header 區域
- [x] 6.3 新增「停止」按鈕：isRunning=true 時顯示，點擊呼叫 `stop()`
- [x] 6.4 將 `selectedProviderId` 傳入 `send()` → 最終傳給 `vcsAgentStep`

## 7. i18n

- [x] 7.1 在 `src/lib/i18n.ts` 新增 en / zh-TW 字串：`vcs_step_limit_reached`、`vcs_goal_achieved`、`vcs_loop_stopped`、`vcs_step_running`

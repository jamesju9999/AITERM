## Why

現有的 VCS Panel 每次只能執行一個操作，無法自主規劃多步驟來達成複雜目標（例如「找出 issue #142 改了哪些檔案」需要先搜 log、再看 diff）。升級為 agent loop 後，AI 可以迭代執行 VCS 操作直到目標達成，同時讓用戶隨時打字調整方向，真正實現對話式版控操作。

## What Changes

- 新增 `vcs_agent_step` IPC command：接受目標 + 歷史紀錄，AI 規劃下一個 VcsIntent 或宣告完成
- **BREAKING**：`useVcsChat` 替換為 `useVcsAgentLoop`，對話模型從單發改為迭代 loop
- VCS Panel 每步同時顯示兩層資訊：⚙️ 實際操作（指令層）與 💬 AI 摘要（語意層）
- 用戶可在 loop 執行中途輸入，AI 將新訊息納入歷史並調整方向繼續
- VCS Panel 加入模型選擇器（比照 DatabaseAiChat 的 `selectedProviderId` 模式）
- 最大步驟數沿用現有 `max_agent_steps` 設定，不新增設定項

## Capabilities

### New Capabilities

- `vcs-agent-step`: AI 規劃下一步 VCS 操作的決策引擎——接受目標與歷史，回傳 next intent 或 final answer
- `vcs-agent-loop-ui`: 迭代式 VCS 對話 UI——每步卡片展示指令層 + 語意層，支援中途打字重導方向
- `vcs-chat-model-selection`: VCS Panel 的模型選擇器，讓用戶選擇執行 VCS AI 推理的 provider

### Modified Capabilities

## Impact

- **Backend 新增**：`src-tauri/src/commands/vcs.rs` 新增 `vcs_agent_step` command；`src-tauri/src/vcs/types.rs` 新增 `VcsAgentHistoryEntry`、`VcsAgentDecision` 型別
- **Frontend 替換**：`src/hooks/useVcsChat.ts` → `src/hooks/useVcsAgentLoop.ts`
- **Frontend 更新**：`src/components/VcsView/VcsMessageBubble.tsx` 新增 step 卡片 variant；`src/components/VcsView/VcsView.tsx` 加入 provider 選擇器
- **IPC 新增**：`src/ipc/vcs.ts` 新增 `vcsAgentStep` wrapper 與相關型別
- **lib.rs**：新 command 加入 invoke_handler

# Agent 找人決策（收件匣＋Telegram）— 實作計畫

> 規格：docs/superpowers/specs/2026-10-16-agent-ask-human-design.md
> 每一步：先寫會失敗的測試 → 實作 → 通過 → commit。

## Task 1：問題資料（`src-tauri/src/tasks/questions.rs`）
- `questions` 表（專案 tasks.db）：id INTEGER PK、task_id、agent_id、question、options（JSON）、recommended、context、
  status（open/answered/cancelled）、answer、answered_via、telegram_message_id、reminders、last_reminded_at、created_at、answered_at。
- 函式：`create`、`get`、`list_open`（只含卡片仍在執行中的）、`answer`（只接受 open）、`cancel_open_for_task`、
  `set_telegram_message`、`due_reminders(now)`／`mark_reminded`、`answer_message(q, answer, tab_id)`（送回分頁的文字，含完成標記提醒）。

## Task 2：`ask_human` MCP 端點（`tasks/ask.rs`＋`quota.rs` 路由）
- 純函式 `handle_rpc(msg, call) -> Option<Value>`：`server/discover`→空結果、`initialize`、`tools/list`、`tools/call`、通知回 None。
- 路由 `/mcp/{secret}/{task_id}`：POST 交給 `handle_rpc`；GET 405；密鑰不對回 404。
- `ask_human` 建立問題、標記卡片「等決策」（暫停卡住判定、問題未答時不把完成標記當結案）、送 `agent-question` 事件。

## Task 3：派工
- `mcp.json`（`{"mcpServers":{"aiterm":{"type":"http","url":...}}}`）＋`--mcp-config "<path>" --allowedTools mcp__aiterm__ask_human`。
- 提示：有 `ask_human` 時的自主說明版本。
- `monitor::watch_with_quota` 多一個 `holding_marker`：等決策時完成標記不算結案。
- 卡片結束：`cancel_open_for_task`。

## Task 4：回答（App）
- 指令 `questions_list`（所有專案的未答問題）、`questions_answer(project_id, id, answer)`：寫入、對分頁送答案、解除等決策、
  更新 Telegram 原訊息、送 `questions-updated`。

## Task 5：Telegram
- 新問題：送出問題＋inline keyboard（`callback_data` = `q|<專案>|<id>|<選項>`），記下 message_id。
- 輪詢：處理 `callback_query`（回答、`answerCallbackQuery`）與「回覆」問題訊息；其餘照舊。
- 已回答：`editMessageText` 標示。

## Task 6：提醒
- 每分鐘檢查：未答滿 30 分鐘（距上次提醒）且提醒少於 3 次 → 送 `agent-question` 提醒事件與 Telegram 提醒。

## Task 7：前端
- `ipc/questions.ts`；頂列「❓ 待決策 N」＋收件匣面板（選項按鈕、建議標示、自由輸入）；Agent 卡「❓ 等決策」；
  通知 hook 處理 `agent-question`；i18n。

## Task 8：驗證
- cargo test --workspace、vitest、tsc、eslint。
- 實機：dev App＋真 claude：提問 → App 回答 → 接著做；再一次從 Telegram 回答。

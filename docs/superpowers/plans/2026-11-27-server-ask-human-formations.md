# Server 中途問人（M5）＋編制切換（M6）實作計畫

> 對應 spec：`docs/superpowers/specs/2026-11-27-server-ask-human-formations-design.md`。每個任務先寫會紅的測試，再實作。

**Goal:** server 上的 Agent 能用 `ask_human` 掛起等人回答（App／Telegram／Email 通知）；共用專案支援多組編制與切換。
**Architecture:** core 新表與函式（`board_questions`、`board_formations`）；server 新增第三個 MCP 端點 `/mcp/{token}/ask`（掛起輪詢，同核准機制）、`routes_ask.rs`、`routes_formations.rs`、Telegram 通知與回呼；桌面新增 Tauri 指令與「待回答」條、編制下拉。

## M5
1. **core：`board_questions`**（建表、`NewQuestion`、`ask_question`（同卡只能有一個 open）、`get`、`answer`（只有 open 可答）、`cancel`、`open_questions(project)`、`recent_answered`、`close_dangling_questions`、`close_card_questions`、提醒欄位更新）。測試：同卡唯一、重答 409 語意、重啟收尾。
2. **server：MCP `ask` 端點**（`dbmcp.rs`）：`SERVER_NAME_ASK = "aiterm-ask"`、`/mcp/{token}/ask`、`ask_human` 工具（驗證、插入、通知、掛起輪詢、取消／token 作廢／24h 逾時）；`McpGuard` drop 關閉該卡 open 問題；`SkillCtx::prepare` 一律註冊並允許 `mcp__aiterm-ask`、附提示詞附註。
3. **server：`routes_ask.rs`**：GET 清單、POST answer（成員）、POST cancel（管理員）、稽核、`events.changed()`。
4. **server：通知**（`asknotify.rs`）：Telegram 發送（附 inline 按鈕）、`callback_query` 與「回覆」答案處理（`telegram.rs` 輪詢加 `callback_query`）、Email 通知（設定 `ask_notify_email`，API `GET/PUT /v1/ask/settings` 管理員）、提醒 ticker（30 分鐘×3）。
5. **桌面 Rust**：`board_remote.rs`／`commands/agents.rs`／`lib.rs`：`server_questions_list／answer／cancel`、`server_ask_settings_get／set`。
6. **前端**：`ipc/serverQuestions.ts`、`ServerQuestionsBar.tsx`（待回答條）、`index.tsx` 掛載、共用 server 設定頁 Email 欄位、i18n zh／en、測試。
7. **驗證**：全套測試；真實 Claude 呼叫 `ask_human`（`#[ignore]`）；真機畫面回答；Telegram 假 API 流程。

## M6
8. **core：編制**（`board_formations`、`board_formation_log`、設定鍵 `active_formation／pending_formation／retired_agents`；`save_formation`、`switch_formation`（擋進行中 work 卡、重建 Agent、記退役）、改名、刪除、記錄）。測試：切換保留卡片、擋進行中、退役。
9. **server：`routes_formations.rs`** 與編排器套用 `pending`（無進行中 work 卡時）。
10. **桌面 Rust＋前端**：`server_formations_*`、共用專案的編制下拉改走 server、測試。
11. **驗證與文件**：真機切換、`docs/intake-server.md`、CHANGELOG（v1.70.0 段落補一條）、server 版本 0.6.0。

# Ask AI：⚡ 與 MCP 開關合併成一套工具迴圈

日期：2026-10-14（整合 MCP 與 Agent 的第 2、3 步；第 1 步見 `2026-10-13-chat-terminal-tool-design.md`）

## 問題

Ask AI 有兩套迴圈：⚡（Agent）走 `<cmd>` 文字迴圈、不經確認就執行、不能用 MCP；⚡ 關時走 `useMcpChat` 的工具迴圈，能用 MCP 與電腦操作，第 1 步又加了 `run_command`。兩個開關的意思互相打架（⚡ 開時 MCP 按鈕反而變灰），安全規則也有兩套。

## 行為（使用者決定）

| ⚡ | MCP 按鈕 | 效果 |
|---|---|---|
| 關 | 灰色不能按 | 純聊天：不給任何工具；回覆裡的 `<cmd>` 照樣有 ▶ 讓使用者手動點 |
| 開 | 關 | 只給 `aiterm_terminal__run_command` |
| 開 | 開 | 終端機工具＋MCP 伺服器工具＋內建電腦操作工具 |

- MCP 按鈕的開關狀態（localStorage `aiterm-use-mcp`）在 ⚡ 關時保留不改，⚡ 再開時恢復。
- 全域 `mcp_enabled = false` 時：⚡ 照樣可用，只給終端機工具；MCP 按鈕照現狀不顯示。
- MCP 沒有任何工具（`mcpToolCount === 0`）時，⚡＋MCP 等同只有 ⚡。

## 安全：完全照設定的「執行模式」

⚡ 模式的每個 `run_command` 都過現有的 `terminal_command_gate`，它讀設定的 `execution_mode`、套用 `terminal_tool::decide`，不加任何例外（使用者決定：設定畫面說什麼就做什麼）：

| 風險 | 一律確認 | 分級自動 | 全自動 Agent |
|---|---|---|---|
| Safe | 問 | 直接跑 | 直接跑 |
| NeedsConfirm | 問 | 問 | 直接跑 |
| Dangerous | 問 | 問 | 問 |
| Blocked | 拒絕 | 拒絕 | 拒絕 |

預設值是「一律確認」，所以沒改過設定的人在 ⚡ 模式下每個指令都會跳授權框。

與現況的差異（要寫進 CHANGELOG，並提示可到設定改成「分級自動」或「全自動 Agent」）：舊 ⚡ 什麼都不問，改完後依執行模式跳授權框，60 秒沒回應視為拒絕（沿用第 1 步）。

⚡ 關時沒有終端機工具，所以閘門不會被呼叫；▶ 手動執行維持既有的 `risk_level × execution_mode` 規則。

## 後端（Rust）

- `ai_chat` 多一個選用參數 `tool_scope: Option<ToolScope>`（`none | terminal | all`，serde 小寫）。沒給時沿用 `use_mcp`（true → `all`、false → `none`），其他十幾個呼叫端不用改。
  - `all`：維持現狀（需要 `cfg.mcp_enabled`），但終端機工具改成不論 `mcp_enabled` 都附上——全域 MCP 關、⚡＋MCP 開時，等同 `terminal`。
  - `terminal`：只附 `terminal_tool::tool_infos()`，不看 `mcp_enabled`；提示詞用 `with_tool_use_rule(build_chat_prompt(..))` ＋ `terminal_tool::PROMPT_HINT`，不加電腦操作提示。
  - `none`：走既有 `run_chat`。
  - `last_role == "tool"` 的放行條件改成「scope 不是 none」。
- `terminal_command_gate` 不改。

## 前端

### `AiPanel/index.tsx`
- 刪除舊 `<cmd>` Agent 迴圈：`runAgentLoop`、`runAgentLoopRef`、`submitAgent`、`buildAgentSystemPrompt`。
- 送出：⚡ 開 → `chat.send(text, scope)`，`scope = mcpActive ? "all" : "terminal"`；⚡ 關 → `"none"`。附件在 ⚡ 模式下也照常送（舊迴圈會丟掉附件，順手修正）。
- `mcpActive` 改成 `agentMode && useMcp && mcpEnabled && mcpToolCount > 0`；`PanelMode` 改成 `suggest | agent | agent-mcp`，`ModeHint` 對應更新文字（三種都要中英文）。
- MCP 按鈕：`disabled = !agentMode || mcpToolCount === 0 || isDisabled`；亮燈條件同 `mcpActive`；⚡ 關時的 title 說明「開啟 ⚡ 後才能使用 MCP 工具」。

### `useMcpChat.ts`
- `send(text, useMcp: boolean, …)` 改為接受 `scope: "none" | "terminal" | "all"`（`resend` 的 `lastSendRef` 一起改），呼叫 `aiChat(..., scope)`；催促動手（intent nudge）條件從 `useMcp` 改成 `scope !== "none"`。
- 從舊迴圈搬過來的東西：
  - **步數與階段**：hook 對外多給 `step`（目前第幾輪）與 `phase`（`thinking` 等 AI／`running` 等工具），AiPanel 的狀態列與 `thinkingLabel` 改讀它們；上限沿用 `maxIterationsRef`（已經是 `max_agent_steps`）。到上限時加一則「已達最大步驟數 N」訊息（現在工具迴圈到上限是靜默結束）。
  - **卡住提示**：`run_command` 執行期間啟動既有的 `getIdleMs` 檢查（`STUCK_IDLE_MS`／`STUCK_CHECK_INTERVAL_MS`）；做法是把 interval 放在 AiPanel 的 `runTerminalCommand` 裡（它本來就包著 `onExecuteCommand`），結束時清掉。
  - **停止**：沿用現有 `onStop`（`chat.stop()` ＋ 有指令在跑就 `onInterruptCommand`）；`onAbortAgent` 合併進同一個停止路徑，`onAgentAborted` 照樣通知外層。
- 系統提示詞補充：放在前端。`send` 多一個選用參數 `extraSystemPrompt`，非空時在送給模型的 `iterHistory` 最前面加一則 `{role:"system"}`（不顯示在對話裡、不存進 session）。舊 Agent 迴圈就是這樣注入 system 訊息，各供應商都已處理（antigravity 會轉成 user）。AiPanel 在 ⚡ 模式下組出這段：
  - Windows 提權規則（exit 740 不要用 `Start-Process -Verb RunAs`／`runas` 繞過，改告訴使用者；把原文的 `<cmd>` 字樣改成 `run_command`）。
  - `remoteSessionPromptNote(getRemoteSession?.())`。
  - 舊提示詞的 cwd ＋ 目錄前 60 項由後端 `context::snapshot` 已涵蓋，不重複加；實作時確認 snapshot 內容確實包含 cwd。

### 不動
- 遠端觀看端的 Ask AI（`src/lib/agentLoop.ts`）與其他用 `invokeAiChat` 的畫面。
- `<cmd>` ▶ 手動執行流程。

## 測試

- Rust：`ToolScope` 對工具清單的影響（none／terminal／all，含 `mcp_enabled=false` 時 all 退成 terminal）；`last_role == "tool"` 在 terminal scope 被放行。
- 前端：
  - ⚡ 關：MCP 按鈕 disabled、送出 scope 為 `none`。
  - ⚡ 開＋MCP 關／開：送出 scope 為 `terminal`／`all`；MCP 狀態在切 ⚡ 後保留。
  - ⚡ 模式的 `run_command` 會呼叫閘門；閘門拒絕時工具結果是 `is_error` 且指令沒執行。
  - 指令執行中沒輸出超過門檻 → 卡住提示出現；指令完成 → 消失。
  - 到步數上限出現提示訊息；停止鍵中斷正在跑的指令。
  - ModeHint 三種模式文字（中英文都有，避免 i18n 漂移）。
- 每個新測試都先證明會紅（對應的實作拿掉時失敗）。
- 真機：三種組合各跑一次（純聊天問問題；⚡ 只開讓它列檔案；⚡＋MCP 讓它用電腦操作開一個 App），以及一個 NeedsConfirm 指令確認會跳框。

## 不做

- 遠端觀看端 Agent 迴圈的同步改造。
- MCP 工具個別啟用／停用。

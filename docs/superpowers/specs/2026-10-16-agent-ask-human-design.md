# Agent 需要決策時找人回答：收件匣＋Telegram — 設計

日期：2026-10-16
狀態：草稿，待使用者確認

## 目標

AI 是執行者；遇到需要人決定的事，用結構化的方式提問，人在 AITerm 的「待決策」收件匣或 Telegram 回答，
答案送回原本的 Agent，它接著做。第一版：回答者只有使用者本人；平台是 App 內收件匣＋Telegram。

## 現況與限制

- 派出去的 claude 跑在 AITerm 的終端機分頁裡；停下來問問題時，只能到那個分頁打字回答（「等你回覆」偵測已做）。
- AITerm 已有的 MCP 伺服器要使用者手動開啟，而且帶著電腦操作、任意 SQL 等工具，不適合交給每個派出去的 Agent。
- 已有的 Telegram 整合：一個 bot、一個聊天室；收到的訊息一律送到「目前作用中的分頁」，對不到是哪一個問題。

## 設計

### 1. 提問工具 `ask_human`（每張卡專用的 MCP 端點）

- 沿用額度 hook 那台本機接收器（127.0.0.1 隨機埠＋密鑰），多開一個極小的 MCP（streamable HTTP）端點，
  只有一個工具 `ask_human`。網址帶卡片 id 與密鑰，所以每張卡只能問自己的問題，也拿不到其他工具。
- 派工時加 `--mcp-config <task_dir>/mcp.json`（路徑規則同 `--settings`，有特殊字元就不加）。只對非互動 Agent 卡。
- 參數：`question`（必填）、`options`（可選，最多 6 個選項，每個可附說明）、`recommended`（建議選項）、`context`（為什麼要問）。
- **工具立刻回傳**：「已送出問題 Q-12，請結束這一輪回覆並等待，答案會以新訊息送來；不要自己先做決定。」
  - 理由：不讓工具呼叫一直卡著等（Claude Code 對 MCP 工具呼叫有逾時，等人可能要幾小時），也不怕 App 重開。
- 答案回來時，AITerm 對那個分頁打字送出：「Q-12 的回答：B（不算次數）。附註：……」，Claude 接著做。

### 2. 問題的資料與狀態

新資料表 `questions`（存在專案的 tasks.db）：id、task_id、agent_id、question、options（JSON）、recommended、context、
status（open／answered／cancelled）、answer、answered_via（app／telegram）、telegram_message_id、created_at、answered_at。

- 卡片有未回答的問題 → 卡片顯示「❓ 等決策」，不做卡住判定（同等額度、等回覆）。
- 卡片結案或被停止 → 它的未回答問題改成 cancelled。

### 3. App 內「待決策」收件匣

- Agent 協作頂列：「❓ 待決策 N」按鈕，打開側邊面板（所有專案的未回答問題，新的在上）。
- 每題顯示：哪個 Agent／哪張卡、問題、背景、選項按鈕（建議的那個標示出來）、自由輸入欄（可以補充或不選選項直接回答）。
- Agent 卡上：有未回答問題時顯示「❓ 等決策：<問題摘要>」，點一下打開那一題。
- 新問題發系統通知（沿用「派工完成時通知」的設定）。

### 4. Telegram

- 新問題送到設定好的聊天室：問題＋背景＋選項按鈕（inline keyboard）。
- 回答方式：按選項按鈕，或「回覆」那則訊息打字。bot 依按鈕資料／被回覆的訊息 id 找到問題。
- 回答後 bot 編輯原訊息標示「已回答：B（從 Telegram）」；在 App 回答的也同步標示。
- 不是回答問題的訊息照舊送到作用中的分頁（既有的遠端控制功能不變）。
- 需要處理 Telegram 的 `callback_query`（按鈕）與 `reply_to_message`；目前的輪詢只處理一般文字訊息。

### 5. 沒人回答

第一版：一直等，不判卡住；每 30 分鐘在 App 與 Telegram 各提醒一次（同一題最多提醒 3 次）。
之後若需要再加「逾時就採建議答案」的選項。

### 6. 提示調整

非互動 Agent 卡的自主說明改成：「需要人做決定（需求不清、取捨會影響成果）時用 `ask_human` 提問，送出後結束這一輪等答案；
小細節自己採合理做法，記在『待確認事項』。」沒有掛上 `ask_human`（指令不是 claude、路徑有特殊字元）時維持現在的說明。

## 動手前要先實測（不寫進實作前）

1. Claude Code 的 `--mcp-config` 指向 HTTP 端點、工具立刻回傳的流程在真的 claude 上是否可行（用假的接收器打一次）。
2. 「工具回傳後要它結束這一輪」是否照做；不照做時，答案送來前它會不會自己先決定。
3. Telegram inline keyboard 與 callback_query 在現有 bot 上的實際格式。

## 不做（第一版）

- 多位回答者、依問題類型分派。
- Slack／Teams／LINE／Email。
- 逾時自動採用建議答案。

## 測試

- Rust：MCP 端點只認自己的密鑰與卡片；`ask_human` 建立問題並立刻回傳；回答後對分頁送出的文字；
  卡片結案時問題改成 cancelled；Telegram 按鈕資料與被回覆訊息對應到問題；不是回答的訊息照舊路由；提醒的次數與間隔。
- 前端：收件匣列出、選項回答、自由輸入回答；Agent 卡的等決策提示；頂列計數。
- 實機：真的 claude 呼叫 `ask_human` → App 收件匣回答 → 它接著做；同一流程改從 Telegram 回答。

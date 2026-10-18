# Agent 共同的專案背景（專案簡介＋團隊分工）設計

日期：2026-10-18　狀態：使用者核可（做 1 專案簡介、2 團隊地圖；3 決策紀錄、4 寫進程式庫先不做）

## 問題

單一 Claude Code session 有整個專案的全貌；Agent 協作裡每張卡都是新的 session，只看得到自己那張卡（從規格拆出來的卡另外附規格全文）。工程師不知道別人負責什麼，容易重複實作或做出互相衝突的介面。

## 做法

每張 **Agent 卡**（agent_id 不是 null；一般看板卡不受影響）派工時，在角色前綴之後、卡片內容之前放一段「專案背景」：

```
## 專案背景
（整個專案的背景，給你參考。只做你這張卡負責的部分，別人負責的模組不要動；…）

### 專案簡介
<使用者寫的內容>

### 團隊分工
- 「計時器」：工程師1，已通過
- 「統計頁」：工程師2，實作中；相依 「計時器」　← 你這張卡
---
```

簡介與地圖都沒有時整段省略。

### 1. 專案簡介

- 存在專案自己的 tasks.db `project_settings`（key `project_brief`），跟 token 上限同一處。空白＝刪除。
- 頂列「專案簡介」鈕打開對話框編輯；已寫時按鈕標 ✓。只影響之後派工的卡。
- 長度：畫面超過 2000 字提醒（每張卡都帶著這段）；放進提示時硬上限 6000 字，超過截斷並註明。

### 2. 團隊分工（自動）

- 列出所有「已指派給工程師的 work 卡」（模組庫裡還沒指派的不列），依 sort_order 排序。
- 每行：標題、負責的 Agent 名稱、模組狀態（沿用後端 `module_state`，中文標籤）、相依（前置的標題）。
- 「你這張卡」：work 卡是自己；fix／review 卡是它的 `module_id`；merge／plan／verify 卡沒有。
- 模組超過 15 個時：只完整列出自己的模組、它的前置、以它為前置的模組；其他合成一行「其他模組：「A」、「B」…」。

### 3. 交件摘要（2026-10-18 追加）

團隊分工只說「誰負責、進度到哪」，後面的工程師還是要自己翻程式碼才知道前面的模組能怎麼用。

- 審核者通過模組時，在 review.json 多填 `provides`：一兩句、100 字左右，寫這個模組對外提供了什麼（函式、元件、API、資料結構、檔案位置），只寫怎麼用不寫過程。審查卡的指示把它列為「判定 pass 時必填」。
- `set_verdict` 在判定 pass 且 `provides` 不是空白時，把它存到模組卡的 `provides` 欄（上限 200 字；以最後一次通過為準）。不通過、空白、驗收卡（也走 set_verdict）都不動。
- 團隊分工在那個模組那一行下面多一行「提供：…」。桌布上模組 chip 的滑鼠提示也顯示同一句。
- 來源選審核者而不是工程師自己：審核者本來就讀完整 diff，寫的是看過程式碼確認過的內容，而且不用多開一個 session。

## Token 成本

簡介數百到一千多 tokens＋地圖每模組約 30–50 tokens，相對一張卡數十萬 tokens 約 1% 以下；內容只在開頭讀一次，之後的輪次走提示快取。

## 檔案

- `src-tauri/src/tasks/context.rs`：`get_brief`／`set_brief`、`team_map`、`context_section`、`project_context`
- `src-tauri/src/tasks/scheduler.rs`：Agent 卡派工時把 `project_context` 接在卡片內容前面
- `src-tauri/src/commands/tasks.rs`：`tasks_get_project_brief`／`tasks_set_project_brief`
- `src/components/AgentCanvas/ProjectBriefDialog.tsx`、`index.tsx`（按鈕、導覽多一步、說明面板 FAQ 一條）

## 不做

- 共享決策紀錄（ask_human 回答、審核重點累積）：等實際看到重複詢問或決策衝突再說（交件摘要先補上「別人做好了什麼可以用」這一塊）。
- 寫進程式庫的 CLAUDE.md／AGENTS.md：會改動使用者的程式庫。

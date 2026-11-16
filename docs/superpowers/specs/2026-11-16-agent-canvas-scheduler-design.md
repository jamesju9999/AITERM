# Agent 協作定時器 — 設計規格

**日期：** 2026-11-16
**狀態：** 使用者已逐段確認設計，待過目後寫實作計畫
**前置：** `2026-10-15-agent-canvas-design.md`（卡片／模組／審查鏈）、`2026-10-17-agent-verifier-design.md`（驗收者）。本期只新增「定時建卡」，不改審查鏈的狀態機。

## 1. 目標與已拍板的決定

在 Agent 協作桌布上加一個「定時器」節點：到時間就自動建一輪工作，交給指定角色，之後沿用現有流程（工程師 → 審核者 → 驗收者）。

| # | 決定 |
|---|---|
| D1 | 定時器是**排程設定**，不是會跑的 Agent。到時間建卡，之後完全重用現有交接 |
| D2 | **方案 A**：每次觸發建一個**新模組＋一張工作卡**，指派給 `agent_id`。不重用同一模組、不走不屬於模組的提問卡 |
| D3 | 目標角色可以是**工程師或驗收者**。驗收者當起點時改建驗收卡，只跑它自己那一關，不再接審核者 |
| D4 | 工作內容是定時器上的一段**固定指令**（不複製既有卡片、不讓角色自行決定） |
| D5 | 週期有三種模式：每天、每週、固定間隔（見 §3）。**不支援 cron 運算式** |
| D6 | 上一輪沒結案就**跳過**這次，不堆卡 |
| D7 | 連續失敗 3 次自動停用並通知 |
| D8 | 一輪成功結案後，其模組與底下所有卡片**自動歸檔**；失敗的保留在看板 |
| D9 | App 沒開不補跑；錯過多個時間點也只建一輪 |
| D10 | 驗收有失敗項目預設算「成功執行」，可用開關改成算失敗 |

不做（YAGNI）：cron 運算式、每月、跨專案共用的定時器、補跑關閉期間錯過的時間。

## 2. 資料

新增表 `schedules`（放各專案自己的 `tasks.db`，同 `agents`）：

| 欄位 | 說明 |
|---|---|
| `id`、`name` | 排程名稱 |
| `agent_id` | 被叫起的角色（工程師或驗收者） |
| `instruction` | 固定指令文字 |
| `schedule_kind` | `daily`／`weekly`／`interval` |
| `schedule_json` | 該模式的參數（見 §3） |
| `enabled` | 是否啟用 |
| `failures_count_as_failed` | 驗收有失敗項目是否算失敗，預設 0 |
| `last_fired_at` | 上次觸發的時間戳 |
| `last_module_id` | 上一輪建出的模組，用來判斷「還沒結案」 |
| `fail_streak` | 連續失敗次數 |
| `skip_streak` | 連續跳過次數 |
| `disabled_reason` | 自動停用的原因（連續失敗／角色消失），手動停用為空 |

另有 `schedule_runs`（`schedule_id`、`fired_at`、`module_id`、`result`：success／failed／skipped），保留每個排程最近 5 筆供介面顯示。

遷移要寫明確的 `ALTER TABLE`／`CREATE TABLE IF NOT EXISTS`，並補「舊資料庫升級」測試（`init_schema` 的 `IF NOT EXISTS` 會讓表已存在時整段跳過，且錯誤被 `let _ =` 吞掉）。

## 3. 週期模式

時間一律用**電腦本地時區**。

| 模式 | `schedule_json` | 例 |
|---|---|---|
| `daily` | `{ "times": ["08:00", "18:30"] }` | 每天 08:00 與 18:30（8 點只是範例，時間可自訂） |
| `weekly` | `{ "days": [1,2,3,4,5], "times": ["09:00"] }`（1=週一 … 7=週日） | 平日 09:00 |
| `interval` | `{ "every_minutes": 120, "window": { "from": "09:00", "to": "18:00" } }`，`window` 可省略 | 每 2 小時，只在 09:00–18:00 |

限制：`times` 至少一個、最多 12 個；`interval` 的 `every_minutes` 最小 5；`window` 的 `from` 必須早於 `to`（不支援跨午夜）。

## 4. 觸發

排程迴圈每分鐘檢查一次，掛在 `scheduler::run` 旁邊，**不改 `pick_next`**。

純函式 `due(now, schedule) -> bool`（新檔 `tasks/schedules.rs`，方便測試）：
- `enabled` 且距離上次觸發之後，**至少有一個預定時間點已經過了**才為真。
- `daily`／`weekly`：預定時間點 = 當天（或符合星期）的每個 `times`。
- `interval`：預定時間點 = 從 `last_fired_at` 起每 `every_minutes` 一個，且落在 `window` 內；從未觸發過則啟用後的第一個時間點。
- 錯過的時間點不補：一次檢查就算過了好幾個，也只觸發一次，並把 `last_fired_at` 設為 `now`。

觸發步驟（同一個交易）：
1. 上一輪（`last_module_id`）還沒結案 → 跳過：寫一筆 `skipped`、`skip_streak` +1，結束。連續跳過 3 次，介面顯示「上一輪卡住了」。
2. 否則建一個新模組＋一張卡指派給 `agent_id`（工程師 → work 卡；驗收者 → verify 卡），更新 `last_fired_at`、`last_module_id`、`skip_streak = 0`。
3. 任何一步失敗整個交易回滾，不留孤兒卡，`last_fired_at` 不更新。
4. 建卡後 poke 排程器並通知畫面重讀（`tasks-updated`），不等輪詢。

額度冷卻中時間到了照樣建卡，由現有派工等額度恢復。

「立即執行一次」走同一個建卡路徑，但**不更新 `last_fired_at`**，所以不影響下一個預定時間點；若上一輪未結案同樣擋下。

## 5. 失敗判定、停用與歸檔

這一輪的模組走到終點時判定一次，沿用現有 `module_state`，排程只讀狀態，不另開偵測（`tick` 每分鐘先結算再觸發）：
- 成功：**工程師起點＝模組審查通過**（沒有審核者時工作卡成功結案即算通過）。合併與驗收是專案層級、要等所有模組都過，不屬於單一輪，所以不等它；驗收者起點＝那張驗收卡自己通過。`fail_streak` 歸零，**自動歸檔**該模組與底下所有卡片（`archived_at`）。
- 失敗：工作卡出錯、審核退回 2 輪仍未過、驗收不通過。`fail_streak` +1，**不歸檔**。
- 驗收報告有失敗項目：依 `failures_count_as_failed`（預設算成功執行），看板上另外標記。

`fail_streak` 達 3 → `enabled = false`、`disabled_reason = "連續失敗"`、發通知（用現有通知機制，不新增管道）。使用者重新啟用時 `fail_streak`、`skip_streak` 歸零。

**角色被暫停（實機驗收發現）**：工程師的卡片失敗後，既有機制會把這位 Agent 暫停（`agents.paused_task_id`，要人決定重試或跳過）。它不會再做事，之後每輪的卡只會排著、被跳過。所以 `tick` 在**到點時**若發現目標 Agent 已暫停，就不建卡，直接 `enabled = false`、`disabled_reason = "角色已暫停"` 並發通知（失敗的那一輪照常先結算、記一次失敗）。使用者處理好卡片後手動重新啟用。使用者自己按的「暫停」（`manual_pause`）是另一回事，不影響定時器。

## 6. 介面

- 畫布新增「⏰ 定時器」節點（`ScheduleNode.tsx`，仿 `AgentNode.tsx`），顯示名稱、週期摘要（「平日 09:00」「每 2 小時」）、啟用狀態、上次結果。狀態顏色：啟用中、已停用、因連續失敗停用（紅）、上一輪卡住（黃）。
- 用 `EdgeLayer` 畫箭頭連到目標角色，畫面即「定時器 → 角色 → 審核者 → 驗收者」。
- 設定對話框（`ScheduleDialog.tsx`，仿 `AgentConfigDialog.tsx`）：名稱、目標角色（只列工程師與驗收者）、固定指令、**先選模式再顯示該模式的欄位**、「驗收有失敗項目算失敗」開關、「立即執行一次」按鈕、最近 5 輪結果清單（點下去跳到該模組）。
- 入口：`TopMoreMenu.tsx` 加「新增定時器」。一個專案可有多個。
- 編制（formation）切換時，指向已不存在角色的定時器自動停用，`disabled_reason = "角色已不存在"`，不建孤兒卡。
- IPC（`src/ipc/schedules.ts` ＋ `#[tauri::command]`）：`list_schedules`、`save_schedule`、`delete_schedule`、`set_schedule_enabled`、`run_schedule_now`。
- 所有字串進 `i18n.ts`，zh-TW 與 en 同時補（key 對齊測試擋得住漏補，型別系統擋不住）。

## 7. 測試與驗收

Rust（記憶體資料庫，不依賴 `AppHandle`）：
- `due`：三種模式各自的邊界（時間點前一分鐘不觸發、後一分鐘觸發）、同一時間點不重複、停用不觸發、跨日跨月、錯過多點只觸發一次、`interval` 的 `window` 邊界與最小 5 分鐘。測試資料要能分辨對錯，不用離界線很遠的值。
- 建卡：模組與卡屬於指定角色；驗收者建 verify 卡；中途失敗不留孤兒卡且 `last_fired_at` 不變。
- 跳過、連續跳過標記、失敗累計 3 次停用、成功歸零、驗收失敗項目依開關、自動歸檔只歸成功的、編制切換停用。
- 舊資料庫升級路徑。

前端（Vitest）：`ScheduleDialog` 必填驗證與各模式欄位切換、角色下拉只列工程師與驗收者、「立即執行」呼叫正確 IPC；`ScheduleNode` 各狀態；i18n 對齊。

紅燈規則：每個新測試都要先證明會紅（拿掉實作或改壞判斷確認失敗）。

實機驗收（不碰使用者那份正式 App）：假 `claude` ＋ 隔離 HOME，定時器設成「現在＋2 分鐘」，驗證到點建模組 → 假 claude 完成 → 審核 → 驗收 → 自動歸檔，以及失敗 3 次停用。節點與箭頭的視覺需使用者看一眼。

## 8. 規劃階段查證結果（原待確認項）

- **模組**＝一張 `card_kind = 'work'`、`module_id` 為空的卡，用 `workflow::create_scheduled_work_card` 建立；審查鏈靠 `module_id` 串。
- **驗收者起點**：既有驗收卡掛在合併卡之後（`review_of` ＝合併卡、`end_sha` ＝合併結果）。定時器直接建驗收卡時沒有合併卡，所以 `fire` 把 `end_sha` 設成該角色 repo 目前的 `HEAD`（`GitClient::head_sha`），否則 `prepare_review_worktree` 會回「沒有受驗版本」。驗收失敗時既有的 `on_verify_failed`（開修正卡給模組工程師或協調者）照常運作，這是「套用後面流程」的一部分。
- **介面**：節點預設放在目標角色左側（不超出畫布左緣），連線是 SVG 虛線直線，不走 `EdgeLayer` 的 DOM 量測。節點不可拖曳（`canvas_x/y` 欄位已留，之後要做再加）。
- 「立即執行一次」不會吃掉下一個預定時間點（執行後還原 `last_fired_at`）；上一輪未結案同樣被擋下。

# Ask AI 的 Agent 協作工具 — 設計規格

**日期：** 2026-11-18
**狀態：** 方向已獲使用者同意，待過目後寫實作計畫
**前置：** `2026-10-15-agent-canvas-design.md`（模組／卡片／審查鏈）、`2026-11-16-agent-canvas-scheduler-design.md`（定時器：定時建卡的現成做法）。本期不改 Agent 協作的狀態機，只新增「從 Ask AI 派工、查結果」的入口。

## 1. 目標與已拍板的決定

使用者在 Ask AI（⚡＋MCP）用一句話交代，Ask AI 不自己做完，而是把工作派給 Agent 協作的角色，之後再查結果。長任務、背景執行、多角色交接與審核都沿用 Agent 協作現有的機制。

| # | 決定 |
|---|---|
| D1 | 做成 **MCP 工具**，加進現有的 AITerm MCP 工具伺服器（`mcp_server/`），由 Ask AI 的工具迴圈取用，不另開伺服器 |
| D2 | 受設定裡 MCP 工具伺服器的新開關 **`agent_canvas_enabled`** 控制，**預設關**（同 `coordination_enabled` 的做法） |
| D3 | **建卡片一定要使用者確認**（第一版不可關）。確認框顯示：專案、指派的角色、標題、完整內容 |
| D4 | 派工**重用定時器的建卡函式**（`workflow::create_scheduled_work_card` / `create_scheduled_verify_card`），不另寫一套建卡邏輯 |
| D5 | 權限不放寬：派出去的卡片照 Agent 協作自己的規則走（角色勾選的系統連線與其寫入權限、命令閘門都不變）。Ask AI **不能**修改角色設定、不能刪卡、不能合併或覆寫審查結果 |
| D6 | 只能派給**工程師**與**驗收者**（同定時器）；審核者、總管由流程自動帶入，不可直接指定 |
| D7 | 只讀工具（列專案、列角色、查看板、讀結果）**不需要確認** |

不做（YAGNI）：Ask AI 自己建立或修改角色、刪除或移動卡片、改定時器、跨專案派工、郵件與檔案工具（另一期）。

## 2. 工具（共 5 個）

所有工具回傳純文字或 JSON 字串，錯誤用 MCP 的 tool error 回傳說明原因。

| 工具 | 參數 | 說明 | 需確認 |
|---|---|---|---|
| `canvas_list_projects` | — | 列出目前開啟的專案（id、名稱、資料夾） | 否 |
| `canvas_list_agents` | `project` | 列出該專案的角色：名稱、類型（coder/verifier/…）、技能（勾選的系統連線名稱、資料庫等）、目前是否忙碌 | 否 |
| `canvas_dispatch` | `project`、`agent`、`title`、`instruction` | 建一個新模組＋一張工作卡派給該角色，回傳 `card_id`。驗收者改建驗收卡 | **是** |
| `canvas_list_cards` | `project`、`status`（選填） | 列看板上的卡片：id、標題、狀態、指派角色、最後更新時間 | 否 |
| `canvas_read_card` | `project`、`card_id` | 回傳卡片狀態與 AI 摘要（`ai_summary`）；進行中的卡片說明「尚未完成」 | 否 |

`project` 與 `agent` 接受名稱或 id（名稱不分大小寫；多個符合時回錯誤並列出候選，不猜）。

`canvas_dispatch` 的限制：
- `title` 必填，上限 80 字；`instruction` 必填，上限 4000 字。
- 該角色已有進行中的輪（同定時器的 `round_open` 概念，這裡改為「該角色現在有未結案的卡」）時，**回錯誤說明忙碌**，不排隊、不堆卡。
- 角色不存在、不是工程師或驗收者、專案未開啟，都回明確錯誤。

## 3. 架構

```
Ask AI 工具迴圈 ──MCP──> mcp_server/tools.rs（5 個 canvas_* 工具，薄薄一層）
                              │  開關檢查 → agent_canvas_enabled
                              ▼
                    mcp_server/canvas_ops.rs（新，純邏輯，可單元測試）
                              │  解析 project / agent、組結果文字
                              ▼
        ProjectRegistry（取專案連線池）  +  tasks::workflow / tasks::store（既有）
                              ▲
                    確認框：mcp_server 發 Tauri 事件 → 前端 → 回傳決定
```

- `canvas_ops.rs` 只依賴連線池與既有的 tasks 函式，不碰 UI；工具函式負責開關檢查與把結果轉成 MCP 回傳。
- 取得 `ProjectRegistry`：`AiTermTools` 已持有 `app: Option<AppHandle>`，用 `app.state::<ProjectRegistry>()` 取得；沒有 `app`（測試環境）時回錯誤。
- 建卡函式需要的 `head_sha`（驗收卡）沿用定時器取得方式。

### 確認機制

建卡前由 `canvas_ops` 發出 `canvas://confirm-dispatch` 事件（含請求 id、專案、角色、標題、內容），然後等待前端回覆。前端顯示確認框，使用者按「派工」或「取消」，呼叫 `canvas_confirm_decide(id, approve)` 命令回覆。

- **逾時 5 分鐘視為拒絕**（無人在場時不會悄悄建卡）。
- 拒絕時工具回傳「使用者拒絕了這次派工」，Ask AI 要告知使用者，不重試。
- 同一時間最多一個待確認請求；其餘直接回「有另一個派工等待確認」。
- 沿用系統連線核准卡片的作法（`systems://need-approval`）：待確認清單在畫面最上層顯示，切換分頁不會漏掉。

## 4. 資料與設定

- 設定：`mcp_tool_server.agent_canvas_enabled: bool`，預設 `false`；設定頁的 MCP 工具伺服器區新增一個開關與一句說明（i18n 中英）。
- 不新增資料表。派出去的卡片就是一般卡片，在 Agent 協作看板上看得到。
- 卡片來源標記：建卡時在內容最前面加一行 `【由 Ask AI 派工】`，讓角色與你在看板上看得出來源（不改資料結構）。

## 5. 錯誤與邊界

| 情況 | 行為 |
|---|---|
| 開關關閉 | 每個 canvas 工具回「Agent 協作工具已停用，請到設定 → MCP 工具伺服器開啟」 |
| 沒有開啟的專案 | `canvas_list_projects` 回空清單與說明；其他工具回「找不到專案」 |
| 角色忙碌 | `canvas_dispatch` 回錯誤，建議稍後或改派別人 |
| 使用者拒絕或逾時 | 回拒絕說明，不建卡 |
| 卡片不存在 | `canvas_read_card` 回找不到 |
| `instruction` 含密碼等敏感字 | 不特別處理（確認框已讓使用者看到完整內容） |

## 6. 提示（給 Ask AI 的使用規則）

在 Ask AI 的系統提示附一小段（僅在開關開啟且 scope 為 all 時）：先用 `canvas_list_agents` 看誰有需要的技能；派工前把要做的事寫清楚，包含輸出要求；派工後告訴使用者「已派出，卡片 id」，不要等結果；使用者之後問結果再用 `canvas_read_card`；被拒絕不重試。

## 7. 測試

- **單元測試（`canvas_ops`）：** 名稱／id 解析（不分大小寫、多重符合回候選、找不到）；忙碌角色拒絕；只能派工程師與驗收者；title／instruction 長度限制；`list_cards` 狀態過濾；`read_card` 進行中與完成的文字。
- **整合測試（記憶體 `tasks.db`）：** 派工後卡片與模組確實存在、指派正確、內容有來源標記；驗收者建驗收卡。
- **確認機制：** 同意、拒絕、逾時三條路徑；逾時不建卡；同時兩個請求第二個被擋。
- **開關：** 關閉時每個工具都被擋；`resolve_tool_scope` 既有行為不變。
- **前端：** 確認框顯示完整資訊；按派工／取消送出對應決定；設定頁開關存取。
- **實機驗收（使用者）：** 在 Ask AI 開 ⚡＋MCP，說「請角色 X 查 …」，看到確認框、按派工、看板出現卡片、角色開始執行；之後問結果能讀到摘要。

## 8. 範圍外與後續

- 郵件工具、本機檔案工具另開一期。
- 讓 Ask AI 建立定時任務（把 `schedules` 也開成工具）是自然的下一步，但本期不做。
- 確認框之後可考慮「本次對話內自動同意同一角色」，需要使用者實際用過再決定。

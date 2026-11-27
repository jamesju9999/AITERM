# Server 功能落差補齊：Agent 中途問人（M5）＋編制切換（M6）

日期：2026-11-27　延續共用看板 server 化（M0～M4 已完成）。

使用者已拍板：
- 中途問人：**所有成員都能回答**；管理員可取消。通知管道**再加 Telegram／Email**。
- 編制切換：**照桌面版**——只有管理員切換，做完後只擋進行中的卡；被換掉的 Agent 以「退役」顯示。

## M5　Agent 中途問人（`ask_human`）

### 機制
沿用每卡 MCP 端點（`/mcp/{token}`，已有 `db_*`）新增工具 `ask_human { question, options?, recommended?, context? }`：
工具呼叫**掛起**，直到有人回答（同 DB／系統核准：500ms 輪詢、上限 24h，`MCP_TOOL_TIMEOUT` 已設）。回答文字就是工具結果，Agent 接著做。
- 只在該卡的 Agent 有任何 server 技能或一律提供？→ **一律提供**（問人與技能無關）；MCP 設定改成每張卡都掛端點（原本只有有技能的卡才掛）。
- 取消／卡被撤銷／server 重啟：掛起的問題標成 `cancelled`，工具回「這個問題被取消了，請自行判斷或回報無法繼續」；重啟時殘留的 `open` 一律關成 `cancelled`（同 `close_dangling_db_queries`）。
- 選項最多 6 個、問題 ≤ 2000 字、同一張卡同時只能有 1 個未答問題。

### 資料
intake.db 新表 `board_questions(id INTEGER PK, project_id, card_id, agent_id, question, options JSON, recommended, context, status open|answered|cancelled, answer, answered_by, answered_via app|telegram, telegram_message_id, reminders, last_reminded_at, created_at, answered_at)`。

### API（成員可答）
| 路徑 | 說明 |
|---|---|
| `GET /v1/board/projects/{pid}/questions` | 未答＋最近 20 筆已答 |
| `POST /v1/board/questions/{id}/answer` | `{answer}`（任何成員；已答／取消回 409）；稽核 `board.question.answer` |
| `POST /v1/board/questions/{id}/cancel` | 管理員 |
變更時 `events.changed()`，桌面版即時更新。

### 通知（server 端）
- **Telegram**：用 server 既有機器人，傳給主人聊天室（`owner_chat_id`）；有選項就附 inline 按鈕（`callback_data = q|<id>|<序號>`），沒有選項請對該則訊息「回覆」答案。輪詢 `allowed_updates` 加上 `callback_query`；回答者以聊天室視為「Telegram（主人）」。回答後把原訊息改成已答。
- **Email**：設定 `ask_notify_email`（管理員在 server 設定）；有人提問時寄純文字通知，內容寫「請到 AITerm 的共用看板回答」。**Email 只通知不能回覆。**
- 無人回答：每 30 分鐘提醒一次、最多 3 次（同桌面版常數），提醒走同樣管道。
- 沒設 Telegram／Email 就只在畫布顯示。

### 桌面版
- 畫布頂端新增「待回答」條（比照 `ServerDbApprovalsBar`）：顯示問題、Agent、卡、選項按鈕＋自由輸入；成員／管理員都可答，管理員多一個「取消」。
- Tauri 指令：`server_questions_list／answer／cancel`；i18n zh／en。
- 設定頁（共用 server）加 Email 通知欄位。

## M6　編制切換

### 資料
- `board_formations(id, project_id, name, agents JSON, created_by, created_at)`，agents 的欄位同桌面版 `FormationAgent`（kind、name、role、prompt_prefix、model、cli、produces_spec、相對位置、db_skill、custom_skills、**systems**）。
- 專案設定（`board_settings` 或既有 kv）存 `active_formation`、`pending_formation`、`retired_agents`（退役 Agent 的 id→名稱，歷史顯示不出現「?」）。
- 切換記錄表 `board_formation_log(at, from_name, to_name, by)`。

### 規則（同桌面版）
- 只有管理員：存目前編制、切換、改名、刪除。
- 切換時：先把目前桌布存回目前編制；刪掉現有 Agent、依目標編制重建（新 id）；卡片、活動記錄、專案設定保留；被換掉的 Agent 記入 `retired_agents`。
- 有進行中的 work 卡（`working` 狀態）時**拒絕切換**（回 409 並說明）；切換後不再擋其他卡。若桌面版是「掛起 pending_formation 等做完再換」，server 同樣支援：`pending` 在無進行中 work 卡時由編排器自動套用。
- 編制內有 DB／系統／自訂技能時：切換重建後 id 對應保持（技能 id 是 server 全域的，不需重新授權）。

### API
`GET/POST /v1/board/projects/{pid}/formations`、`POST …/formations/{id}/switch`、`PUT …/formations/{id}`（改名）、`DELETE`、`GET …/formations/log`。

### 桌面版
- 共用專案的畫布標題列「編制：xxx」下拉（目前桌面版已有元件，改走 server 指令）；切換確認、被擋時顯示原因。
- Tauri 指令：`server_formations_*`；沿用現有 UI 元件。

## 錯誤處理
- 掛起中的 `ask_human` 遇到 Claude 行程被終止：`McpGuard` drop → 該卡所有 open 問題關成 cancelled。
- Telegram 發送失敗只記錄，不影響提問（畫布仍看得到）。
- 兩人同時回答：第一個成功，第二個得到 409「已有人回答」。

## 測試
- core：問題 CRUD、同卡唯一未答、重啟清殘留；編制存取、切換重建、擋進行中。
- server 整合（假 claude／假 MCP 客戶端）：`ask_human` 掛起→成員回答→工具回傳；取消；重啟；權限（成員可答、不可取消／切換）；Telegram 假 API（發送、按鈕回呼、回覆文字）；Email 假 SMTP 層（沿用 mail 測試做法）；提醒計時。
- 桌面：待回答條、編制下拉的前端測試與 board_remote 指令測試。
- 真實驗證：真的 Claude Code 在卡裡呼叫 `ask_human`，從畫面回答後繼續；Telegram 用假 API 實測按鈕流程（真機器人使用者自測）；編制切換用真機畫面操作。

## 不在這版
- Email 回覆答案、LINE／網頁通知、問題的歷史搜尋。
- 單卡多個同時問題。

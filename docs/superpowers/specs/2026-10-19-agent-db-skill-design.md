# Agent 協作：可拖曳的「資料庫」技能

日期：2026-10-19　分支：feat/agent-canvas

## 目標

使用者把「🗄 資料庫」技能拖到 Agent 卡上，選要開放的連線與權限，那位 Agent 派工時就能查 AITerm 已設定的
資料庫；查詢時畫面上看得到誰在查哪個連線、跑什麼 SQL。

使用者已拍板：
1. 第一版只做資料庫技能。
2. 權限三選一：**唯讀**／**每次寫入要核准**／**可寫入直接放行**。
3. 卡片上的標籤只顯示 SQL 開頭約 40 字，活動記錄顯示完整 SQL。

## 設計

### 資料

- `agents.db_skill TEXT`（JSON，NULL = 沒裝）：`{"connections": ["<連線 id>", …], "mode": "read" | "approve" | "write"}`。
  連線 id 對到 `config.db_connections`。任何種類的 Agent 都可以裝（驗收者查資料庫確認資料有存進去也有用）。
- 新表 `db_queries`（專案的 tasks.db）：`id, task_id, agent_id, connection_id, connection_name, sql, started_at,
  finished_at, rows, ms, error, decision`。`decision`：`allowed`／`approved`／`denied`（唯讀擋下或使用者拒絕）。
  活動記錄與「使用中」都從這張表算，不靠事件：事件在畫面訂閱前送出就遺失（這個 repo 踩過兩次）。

### 工具：掛在每張卡自己的 MCP 端點

每張卡已經有專屬的 MCP 端點（`/mcp/<密鑰>/<task_id>`，目前只有 `ask_human`）。卡片登記時記下那位 Agent 的
`db_skill`，有裝才在 `tools/list` 多列：

| 工具 | 用途 |
|---|---|
| `db_list_connections` | 只列這位 Agent 被允許的連線（名稱、類型、資料庫、權限）。 |
| `db_list_schemas` / `db_list_tables` / `db_table_schema` | 結構查詢，一律允許（不改資料）。 |
| `db_query` | 執行 SQL。權限見下。 |

連線不在允許清單 → 錯誤。帳密不經過 Agent，只用連線 id；實際執行沿用 `mcp_server::db_ops`。
路徑上有 task_id，所以 AITerm 知道是哪張卡、哪位 Agent 在查。Claude 與 Codex 卡片用同一個端點，兩種都適用。

### 權限（在 AITerm 這邊擋，不靠提示詞）

`sql_guard::is_read_only(sql)`：去掉註解與字串常值後，
- 只能有一個敘述（結尾的 `;` 可以）；
- 第一個關鍵字是 `SELECT`、`WITH`、`SHOW`、`DESCRIBE`/`DESC`、`EXPLAIN`、`VALUES`、`TABLE`；
- 整句不得出現寫入或副作用關鍵字（`INSERT UPDATE DELETE MERGE UPSERT REPLACE DROP ALTER CREATE TRUNCATE
  RENAME GRANT REVOKE EXEC EXECUTE CALL DO COPY LOAD ATTACH DETACH VACUUM REINDEX LOCK SET PRAGMA INTO`
  等；`INTO` 擋 `SELECT … INTO`）。
寧可錯擋：被擋的唯讀查詢只會讓 Agent 收到錯誤訊息改寫，錯放一句寫入的代價大得多。

| 模式 | 唯讀查詢 | 其他 |
|---|---|---|
| 唯讀 | 執行 | 拒絕，告訴 Agent「這個連線只開放唯讀」 |
| 每次核准 | 執行 | 建一個問題到收件匣（＋Telegram）：選項「核准執行／拒絕」，內容附完整 SQL；工具呼叫掛著等答案 |
| 直接放行 | 執行 | 執行 |

核准沿用 ask_human 的問題與等待機制（Codex 的阻塞式提問已經用這套）。等待期間卡片是「等決策」，不判卡住。
Claude 卡片把 `MCP_TOOL_TIMEOUT` 設成 24 小時（寫在派工的 settings 檔 `env`），等人核准時工具呼叫才不會逾時。

### 畫面

- **技能區**：Agent 池那一列後面加「技能」標籤與「🗄 資料庫」，拖到 Agent 卡上放開 → 開設定框。拖到別處沒反應。
- **設定框**：勾選連線（沒有任何連線時提示先到資料庫頁新增）、三選一權限、儲存／移除技能。
- **Agent 卡**：裝了技能就在標題下顯示標籤「🗄 連線名稱（唯讀／需核准／可寫入）」，多個連線用「、」接；點標籤開設定框。
- **使用中**：這位 Agent 有還沒結束的查詢 → 標籤變亮，後面接 SQL 開頭 40 字（多的用「…」）。
- **活動記錄**：每筆查詢一行「工程師1 查詢 HTBKTW-DEV：<完整 SQL>（12 列，0.3 秒）」；被擋或被拒絕的也記一行。
- 查詢開始與結束時後端送 `tasks-updated`（現有事件），畫面重抓 `db_queries`。

## 不做

- 其他技能（瀏覽器、自訂 SKILL.md）。
- 資料庫層級的唯讀交易（各資料庫語法不同；分類器已經擋在前面）。
- 查詢結果存檔（只存列數與耗時）。

## 測試

- `is_read_only`：唯讀各種寫法通過；寫入、DDL、多敘述、`SELECT … INTO`、`WITH … DELETE`、註解或字串裡藏關鍵字的情況。
- 權限決策：連線不在清單、三種模式 × 讀／寫。
- MCP：沒裝技能不列 db 工具；有裝才列；未知工具錯誤。
- 接收器：唯讀擋下記 `denied`；核准→執行；拒絕→不執行；每筆都寫進 `db_queries`。
- 前端：拖曳開設定框、存檔呼叫 IPC、卡片標籤、使用中顯示 40 字、活動記錄完整 SQL。

## 實作順序

1. `sql_guard`（純函式＋測試）。
2. `agents.db_skill` 欄位、`db_queries` 表與存取函式、`agents_set_db_skill`／`agents_db_queries` 指令。
3. `ask.rs` 依卡片是否有技能列出 db 工具；`QuotaState` 處理 db 呼叫（權限、核准、執行、記錄）。
4. 排程器：派工時登記技能、放行 db 工具、settings 加 `MCP_TOOL_TIMEOUT`。
5. 前端：IPC、技能區與拖曳、設定框、卡片標籤與使用中、活動記錄、i18n。
6. 實測：真 claude 透過卡片端點查 SQLite（唯讀被擋、核准流程）。

# Server 資料庫技能（M2）

日期：2026-11-26　接續 `2026-11-26-server-skills-custom-design.md`（總覽與決策在那份）。

## 目標

共用看板專案的 Agent（工程師、審查者、驗收者…）在 server 上也能查資料庫：連線由 AITerm 分享到 server，帳密加密存在 server；
權限沿用桌面版三種（唯讀／每次核准／直接放行），**每次核准**由管理員在共用畫布上看完整 SQL 後按核准或拒絕。

已拍板：連線由 AITerm 設定並分享到 server；包含「每次核准」。範圍（本版）：PostgreSQL、MySQL、SQLite。
SQL Server、ODBC、DB2 只在桌面版（server 沒有那些驅動與 sidecar），分享時會明講不支援。

## 設計

### 共用程式（`aiterm-db` crate，桌面版與 server 同一份）

- 已抽出的適配器（`DbAdapter`＋PG／MySQL／SQLite）。
- 新增：`sql_guard`（`is_read_only`，從桌面版搬來）、`skill`（`DbMode`、`DbSkill`、`parse`、`Decision`、`authorize`、`prompt_note`，從桌面版搬來；Agent 的 `db_skill` JSON 格式不變：`{"connections":[…],"mode":"read|approve|write"}`，server 端的連線 id 是 server 自己的）、
  `connect(db_type, host, port, database, username, password)`（組連線字串並建適配器，含百分比編碼）、`format_result`（截斷到 100 KB）。
- 桌面版 `tasks/sql_guard`、`tasks/db_skill` 改為 re-export，行為與測試不變。

### 資料（server 的 intake.db）

- `board_db_connections(id, name, db_type, host, port, database, username, default_schema, created_by, created_at, updated_at)`；**密碼不進資料表**，存 `FileSecrets`（鍵 `dbconn:<id>`，與 AI 金鑰同一套加密）。
- `board_agents.db_skill TEXT`（JSON，NULL＝沒裝）。
- `board_db_queries(id, project_id, card_id, agent_id, connection_id, connection_name, sql, decision, started_at, finished_at, rows, ms, error, decided_by)`；
  `decision`：`allowed`｜`pending`｜`approved`｜`denied`。待核准清單就是 `decision='pending'` 的列。啟動時把 `pending` 且卡已不在跑的列標成 `denied`（server 重啟後 Agent 已被殺，不會再有人等）。

### API

| 路徑 | 說明 |
|---|---|
| `GET /v1/db/connections` | 清單。成員只看到 `id、name、db_type、database`；管理員另有 host、port、username |
| `POST /v1/db/connections`（管理員） | `{name, db_type, host, port, database, username, password, default_schema}`；db_type 限 `postgresql｜mysql｜sqlite`；同名取代（保留 id） |
| `DELETE /v1/db/connections/{id}`（管理員） | 刪除（連同密碼、連線池）；Agent 的設定在讀取時自然濾掉 |
| `POST /v1/db/connections/{id}/test`（管理員） | 實際連一次，回成功或原因 |
| `PUT /v1/board/agents/{id}/db-skill`（管理員） | `{skill: null｜{connections, mode}}`；連線必須存在 |
| `GET /v1/board/projects/{pid}/db/queries?limit=` | 近期查詢記錄（成員可讀） |
| `GET /v1/board/projects/{pid}/db/approvals` | 待核准的寫入（成員可讀，只有管理員能決定） |
| `POST /v1/board/db/approvals/{id}/decide`（管理員） | `{approve: bool}`；已決定的不能再改 |

每次異動、每個決定寫稽核紀錄。

### 給 Claude 的工具：server 內建的 MCP 端點

- server 另開一個**只聽 127.0.0.1 的純 HTTP 埠**（不走 TLS，因為 Claude 是本機子行程）；每次執行發一個**隨機 token**，對應 `{卡片、Agent、專案、DbSkill}`，卡結束就作廢。
- 端點 `POST /mcp/<token>`，JSON-RPC：`initialize`、`tools/list`、`tools/call`。工具同桌面版：
  `db_list_connections`、`db_list_schemas`、`db_list_tables`、`db_table_schema`、`db_query`。連線不在 Agent 被允許的清單 → 錯誤。
- 權限判定用 `authorize`（唯讀擋寫入；`approve` 寫入 → 建 `pending` 列並**掛著等**，每 500ms 查決定，最久 24 小時；卡被中斷或 token 作廢就視為拒絕）。
- 給 Claude 的設定檔：`--mcp-config`（`{"mcpServers":{"aiterm-db":{"type":"http","url":"http://127.0.0.1:<埠>/mcp/<token>"}}}`）＋`--strict-mcp-config`，
  `--allowedTools` 加 `mcp__aiterm-db`；環境變數 `MCP_TOOL_TIMEOUT=86400000`（等核准時工具呼叫才不會逾時）。
- 提示詞附 `prompt_note`（連線名稱與 id、權限說明）。
- 連線池：每個連線一份、第一次用才建、連線被修改或刪除就丟掉。單次查詢逾時 60 秒；結果最多 100 KB。

### 桌面版（AITerm）

- 分享、測試、刪除連線都在 Agent 資料庫技能的對話框裡（管理員；PG／MySQL／SQLite 才能分享）。AITerm 讀本機連線與鑰匙圈密碼，送 server。
  （原先想放在設定的資料庫連線頁，實作時為了少一個入口改放在這裡。）
- 共用專案畫布：技能區顯示「資料庫」；拖到 Agent 上開對話框，勾選 server 連線、選模式。
- 頂列在有待核准時出現「🗄 N 筆待核准」（管理員可按進去看完整 SQL、核准或拒絕；成員看得到數量）。

### 錯誤與邊界

- 唯讀判斷寧可錯擋（同桌面版）；DB 錯誤原樣回給 Agent。
- 分享時 SQLite 的 host 是檔案路徑：那是 **server 主機上的路徑**，對話框要明講。
- 刪除連線時還有 `pending` 的列：標成 `denied`。
- Agent 同時有多個連線、卡被中斷、token 重複使用：token 作廢後的呼叫一律 401。

### 測試與驗證

- 共用程式：`authorize`、`is_read_only` 沿用既有測試（搬過去後仍通過）；`connect` 對 SQLite 與壞參數。
- server：連線 CRUD 與權限、密碼不出現在 API 與資料表、MCP 端點（token、工具清單、唯讀擋寫入、直接放行、核准流程的掛起／核准／拒絕／逾時作廢）、
  runner 參數與環境（`--mcp-config`、`mcp__aiterm-db`、`MCP_TOOL_TIMEOUT`）、重啟後 pending 清掉。
- 真實驗證：真的 Claude Code 對一個 SQLite 檔查資料；唯讀擋 INSERT；核准模式下 INSERT 掛起、管理員核准後完成、拒絕後 Agent 收到拒絕。

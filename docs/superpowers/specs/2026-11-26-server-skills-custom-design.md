# Server 支援 Agent 技能 — 總覽與 M0＋M1（自訂技能）

日期：2026-11-26　分支：master（延續共用看板 server 化）

## 背景與目標

共用看板開了「由 server 執行」後，Agent 的卡片由 server 上的 Claude Code 處理，但桌面版的三種技能（資料庫、自訂、系統連線）
在 server 上用不到。使用者要求 server 也支援，並已拍板：

1. 三種都要做，拆成里程碑，各自 spec → 計畫 → 實作 → 驗證。
2. **資料庫連線**由 AITerm 設定並分享到 server，帳密加密存在 server；寫入模式要包含「每次核准」。
3. **自訂技能**從桌面版上傳到 server。
4. **系統連線**連同即時預覽與人工接手一起做；M0～M3 連續做，M4（預覽與接手）到時再確認設計。

| # | 內容 |
|---|---|
| M0＋M1 | 本文件：Agent 技能欄位與畫布接線、自訂技能（上傳、管理、派工附帶） |
| M2 | 資料庫技能：連線分享、server 內建查詢引擎、每卡 MCP 端點、唯讀／直接放行／每次核准（含 server 端「掛起等人」機制與畫布的待核准清單） |
| M3 | 系統連線技能：server 上跑 Playwright、帳密存 server、寫入閘門、審計 |
| M4 | 即時預覽與人工接手（另寫 spec，屆時再確認） |

原先想放進 M0 的「每卡 MCP 端點」只有 M2 才用到，移到 M2（避免過早建造）。

## M0＋M1 設計

### 資料

- `board_agents.custom_skills TEXT`：JSON 陣列，裝的是 **server 技能 id**（NULL＝沒裝）。以 `ALTER TABLE ADD COLUMN` 加入（已存在就略過）。
  單一 Agent 最多 12 個（同桌面版 `MAX_SKILLS`）。
- 新表 `board_skills(id, name UNIQUE, description, files, bytes, uploaded_by, created_at, updated_at)`（intake.db）。
- 檔案存 `<data>/skills/<id>/…`，原樣保留資料夾結構。技能庫是**整台 server 共用**的，不分專案。

### 上傳規則（server 端檢查，不信任桌面版）

- 必須有根目錄的 `SKILL.md`；`name`、`description` 讀 frontmatter（沒有就用資料夾名與空說明；同桌面版 `parse_frontmatter`）。
- 路徑必須是相對路徑、不含 `..`、不含空段、不含反斜線；略過 `.git`、`node_modules`、`.DS_Store`。
- 上限：300 個檔、總計 10 MB、單檔 2 MB。超過整份拒絕並說明哪一項。
- 同名技能再上傳＝**取代內容、保留 id**（已裝這個技能的 Agent 不會掉）；整份先寫到暫存資料夾再換，失敗不會留下半份。
- 只有管理員能上傳、刪除、設定 Agent 的技能；所有成員可讀清單與內容的檔案列表。每次異動寫稽核紀錄。
- 刪除技能：從所有 Agent 的清單移除（讀取時就過濾不存在的 id，所以不需要逐個改寫）。

### API

| 路徑 | 說明 |
|---|---|
| `GET /v1/skills` | 技能清單 `[{id,name,description,files,bytes,uploaded_by,updated_at}]` |
| `POST /v1/skills`（管理員） | `{folder_name, files:[{path, data}]}`（`data` 為 base64）；路由放寬到 16 MB 主體 |
| `DELETE /v1/skills/{id}`（管理員） | 刪除 |
| `PUT /v1/board/agents/{id}/skills`（管理員） | `{ids:[…]}` 整份取代；不存在的 id 拒絕；超過 12 個拒絕 |

Agent 的 JSON 多一個 `custom_skills`（id 陣列），變更會 bump 版本並通知前端。

### 派工（server）

任何由 server 處理的卡（work／fix／review／plan／verify）：Agent 有技能時，

1. 把每個技能複製到 `<data>/run/<card id>/skills/<name>/`（一次性副本，結束就刪）。**不直接把技能庫路徑交給 Claude**：
   work 卡有 Edit 權限，直接給技能庫路徑等於讓 Agent 能改共用技能。
2. 每個副本資料夾用 `--add-dir` 交給 Claude。
3. 提示詞附一段清單（名稱、一句用途、`SKILL.md` 路徑），跟桌面版一樣只給清單、需要時才讀全文。

限制（要寫進說明文件）：work／review 卡沒有 Bash（只有專案驗證指令），所以技能裡的**腳本不能執行**，技能以「指引文字」為主；驗收者有 Bash，可以。

### 桌面版（AITerm）

- 新的 Tauri 指令：`server_skills_list`、`server_skills_upload(folder_path)`（遞迴讀本機資料夾、套用同一組上限、base64 後上傳）、
  `server_skills_delete(id)`、`agents_set_server_skills(project_id, agent_id, ids)`。
- 共用專案的畫布：技能區只顯示「自訂技能」（資料庫、系統連線在 server 還沒支援，先隱藏，M2／M3 再開）。
  拖到 Agent 後開「Server 技能庫」對話框：列出 server 上的技能並勾選；管理員有「從本機加入…」（選含 `SKILL.md` 的資料夾，或程式庫／使用者的技能位置）與「刪除」；成員只能看。
- 共用專案的 Agent 的 `custom_skills` 對應成 id 陣列；畫布上跳過本機路徑檢查（`missingSkills`），名稱用 server 技能清單對照。

### 錯誤處理

- 技能檔案被手動刪掉／複製失敗：該張卡照常處理但提示詞明講「技能 X 讀不到」，不讓整張卡失敗。
- 上傳中斷：暫存資料夾不會取代正式內容，下次啟動清掉殘留暫存。
- 非管理員呼叫管理 API：403 並說明。

### 測試與驗證

- server 單元：路徑檢查（`..`、絕對路徑、反斜線）、上限、frontmatter、取代保留 id、刪除後 Agent 清單過濾。
- server 整合（假 claude）：有技能的 Agent 派工 → 參數含 `--add-dir`、提示詞含清單、副本存在、結束後清掉；work 卡改副本不影響技能庫。
- API：管理員／成員權限、超限、壞路徑。
- 桌面：board_remote 的上傳（讀資料夾、上限、略過 `.git`）；前端對話框的勾選、管理員／成員差異。
- 真實驗證：真的 Claude Code 讀到技能裡的暗號並照做；再用畫面操作上傳與勾選。

## 不在這一版

- 技能腳本在 work／review 卡的執行（沒有 Bash）。
- 技能版本歷史、每專案各自的技能庫。
- 讀技能的使用紀錄（桌面版靠 PostToolUse hook，`claude -p` 沒有）。

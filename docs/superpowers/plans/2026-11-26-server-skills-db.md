# Server 資料庫技能（M2）實作計畫

> spec：`docs/superpowers/specs/2026-11-26-server-skills-db-design.md`。每個任務先寫會紅的測試。

1. **aiterm-db 共用程式**：搬 `sql_guard`、`skill`（DbMode／DbSkill／authorize／prompt_note）；新增 `connect`、`format_result`；桌面版 re-export；既有測試仍過。
2. **core**：`board_db_connections`、`board_agents.db_skill`、`board_db_queries` 與 store 函式（連線 upsert 保留 id、記錄／待核准／決定／啟動清理、設定 Agent 技能驗證）。
3. **server：連線管理與 API**：密碼走 `FileSecrets`；CRUD／test／agent db-skill／queries／approvals／decide；權限、稽核；連線池（AppState）。
4. **server：MCP 端點**：127.0.0.1 純 HTTP、token 註冊、JSON-RPC、五個工具、授權與核准掛起；啟動時開、關閉時收。
5. **runner：附掛（Attach）**：`--mcp-config`（可多個）＋`--strict-mcp-config`、`--allowedTools` 追加、環境變數；`SkillCtx` 一併準備 db 的 MCP 設定檔與 token、結束作廢。
6. **桌面版 Rust**：分享連線、列出／刪除／測試、設定 Agent db 技能、待核准清單與決定；`to_agent` 帶 `db_skill`。
7. **前端**：設定頁分享、`ServerDbSkillDialog`、畫布技能區與節點顯示、待核准面板（頂列徽章）、i18n、測試。
8. **驗證與文件**：全套測試、真實 Claude（SQLite：查詢／唯讀擋寫入／核准與拒絕）、docs 與 CHANGELOG。

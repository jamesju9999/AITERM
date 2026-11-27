# Server 系統連線技能（M3）實作計畫

> spec：`docs/superpowers/specs/2026-11-27-server-skills-systems-design.md`。每個任務先寫會紅的測試。

1. **抽出 `aiterm-systems` crate**：types／gate／approvals／audit／cdp／browser／proxy 搬過去；切掉 App 相依；桌面版 re-export；桌面版 systems 測試全過。
2. **core 資料層**：`board_systems`、`board_agents.systems`、`board_system_sessions`、`board_sys_events` 與 store 函式。
3. **server：系統與 session 管理 API**：CRUD、匯入 session（加密、只收白名單網域的 cookie）、權限、稽核。
4. **server：瀏覽器執行階段**：每卡隔離 Chrome＋cookie 注入＋CDP 閘門＋Playwright 代理；失效偵測；卡結束與重啟清理。
5. **server：`sys_*` 工具接進 MCP 端點**：token 環境、工具目錄與 `intent`、`--allowedTools`、提示詞；待核准與審計寫入 `board_sys_events`。
6. **桌面版 Rust**：分享系統設定、分享 session（CDP 取 cookie）、勾選、待核准泛化。
7. **前端**：系統技能對話框、分享按鈕、待核准面板泛化、i18n、測試。
8. **驗證與文件**：全套測試、真實 Claude＋測試站、docs 與 CHANGELOG。

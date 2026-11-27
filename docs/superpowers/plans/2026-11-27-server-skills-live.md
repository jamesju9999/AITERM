# M4 即時預覽、接手與遠端登入 實作計畫

> spec：`docs/superpowers/specs/2026-11-27-server-skills-live-design.md`。

1. **`aiterm-systems::live`**：CDP 串流與輸入驅動（純邏輯可測：座標換算、鍵盤對應、協定型別）；真 Chrome 的串流與輸入測試（ignored）。
2. **server：Agent 執行中的觀看與接手**：`SysRun` 暴露 CDP 位址與 `manual`；觀看者註冊（懶啟動、最多 3 位）、列表端點、WebSocket 端點（成員唯讀、管理員可輸入與接手）、離線自動交還、稽核。
3. **server：遠端登入**：start／live／finish／cancel、閒置逾時、`store_session` 沿用、單一登入限制、收乾淨。
4. **client 與桌面 Rust**：`Api` 開通用 WebSocket；`live_open/send/close` 指令與事件。
5. **前端**：`LiveViewDialog`（畫面、輸入轉送、唯讀標示、按鈕）、系統對話框入口、Agent 節點「👁 查看」、i18n、測試。
6. **驗證與文件**：全套測試、真 Chrome＋真 Claude（接手流程）、實機視窗驗證、docs 與 CHANGELOG。

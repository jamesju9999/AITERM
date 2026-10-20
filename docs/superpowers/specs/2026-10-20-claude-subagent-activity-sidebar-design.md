# 終端機分頁 Claude subagent 協作側欄 — 設計

日期：2026-10-20
狀態：待使用者複審

## 問題

在 AITerm 的一般終端機分頁手動執行 `claude` 時，Claude 自己用 Agent 工具開出的
subagent 目前沒有任何視覺呈現（Agent 協作桌布只涵蓋由桌布／看板派工的卡片）。
使用者想在該分頁旁看到 subagent 的協作情形。

## 決策（已與使用者確認）

| 項目 | 決定 |
|---|---|
| 呈現位置 | 該終端機分頁內的可收合側欄，不離開當前分頁 |
| 事件來源 | 啟動時注入 Claude Code hook（`--settings`），不解析畫面、不輪詢 transcript |
| 注入方式 | PATH shim：AITerm 開的 PTY 把包裝過的 `claude` 放在 PATH 最前面 |
| 顯示內容 | subagent 樹狀與狀態、目前使用的工具、任務描述與結果摘要、token／費用 |

## 範圍

**含：** shim、hooks 設定、後端事件端點與狀態機、`claude-activity://{sessionId}` 事件、
前端側欄、跨平台（macOS／Linux 用 sh，Windows 用 `.cmd`＋PowerShell helper）。

**不含：** 併入 Agent 協作桌布；修改 `~/.claude/settings.json`；攔截以絕對路徑執行的
claude（已知限制，說明文字註明）；Codex 等其他 CLI。

## 架構與資料流

1. **shim**：`pty_create` 時把 `<appdata>/shims/` 放到子行程 `PATH` 最前面，並設
   `AITERM_SESSION_ID`。shim 的 `claude` 於 PATH 其餘部分尋找真正的 `claude`，補上
   `--settings <hooks.json>` 後轉交；找不到時明確報錯。若參數已含 AITerm 的
   `--settings`（派工路徑），不重複注入。環境必須在 `pty_create` 當下設定
   （Windows 的 PATH 在行程啟動時即固定）。
2. **hooks.json**：訂閱 `SubagentStart`、`SubagentStop`、`PreToolUse`、`PostToolUse`。
   每個 hook 由小型 helper 將 stdin JSON 附上 `AITERM_SESSION_ID`，POST 到 AITerm
   本機 MCP 伺服器的新端點。因為走 `--settings`，與使用者既有 hooks 合併而非覆蓋。
3. **後端**（新模組 `src-tauri/src/claude_activity/`）：每個 session 維護 subagent 樹
   （id、父節點、描述、狀態、起訖時間、目前工具、結果摘要）。token／費用於
   `SubagentStop` 時依 transcript 路徑彙總。狀態變更 emit `claude-activity://{sessionId}`，
   並提供 `claude_activity_snapshot(sessionId)` 指令。
4. **前端**：終端機分頁右側可收合側欄。掛載時先取快照再訂閱事件（避免事件在訂閱前送出
   而遺失）。沒有任何事件的 session 不顯示側欄。

## 錯誤處理與邊界

- **hook 不可影響 Claude**：helper 逾時 1 秒、吞掉所有錯誤、永遠 exit 0。AITerm 未開或
  端點不通時，Claude 照常運作，僅無畫面。
- **事件不可信**：端點只收 localhost，驗證 `AITERM_SESSION_ID` 對應真實 PTY session，
  未知 session 的事件丟棄。
- **亂序／遺失**：`Stop` 先於 `Start` 時補建節點再標完成；重複事件冪等。Claude 結束
  或 PTY 關閉時，未結束節點一律標「已中斷」。
- **shim 範圍**：只對 AITerm 開的 PTY、以 `claude` 名稱啟動者生效。

## 測試

- Rust：事件→樹的狀態機（亂序、重複、缺 Start、PTY 關閉收尾）、端點 session 驗證；
  每個新測試先證明會紅。
- shim：以假 `claude` 記錄收到的參數，驗證補上 `--settings`、不重複注入、找不到真
  claude 時報錯（sh 與 Windows 各一）。
- 前端：側欄於快照與即時事件下的渲染；無事件時不出現；收合狀態。
- 實機：先用假 claude 送事件驗收畫面，再用真 claude 跑含 subagent 的任務。

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
| 注入方式 | AITerm 的 shell 整合腳本（在使用者 rc 之後載入）定義 `claude` 函式，自動補 `--settings`；不用 PATH shim（使用者 rc 常把原生安裝的 claude 目錄前置，shim 會被蓋過） |
| 顯示內容 | subagent 樹狀與狀態、目前使用的工具、任務描述與結果摘要、token 用量（Agent 工具回應帶 totalTokens，無金額） |

## 範圍

**含：** shell 函式、hooks 設定、後端事件端點與狀態機、`claude-activity://{sessionId}` 事件、
前端側欄、跨平台（macOS／Linux 用 zsh／bash 函式，Windows 用 PowerShell 函式）。

**不含：** 併入 Agent 協作桌布；修改 `~/.claude/settings.json`；攔截以絕對路徑執行的
claude（已知限制，說明文字註明）；Codex 等其他 CLI。

## 架構與資料流

1. **注入**：`pty_create` 時為該 session 寫一份 `<data_dir>/AITERM/claude-activity/<sessionId>.json`
   （hooks 設定），路徑放進子行程環境變數 `AITERM_CLAUDE_SETTINGS`。zsh／bash／PowerShell 的整合腳本
   各定義 `claude` 函式：參數已含 `--settings` 就原樣轉交（派工路徑），否則補
   `--settings "$AITERM_CLAUDE_SETTINGS"`；使用者自己已定義 `claude` 別名／函式時不覆蓋（該情況不會有
   側欄）。cmd.exe 不支援（Windows 預設 shell 是 PowerShell）。
2. **hooks**：Claude Code 的 **HTTP hook**（`"type":"http"`，不經 shell），訂閱 `PreToolUse`、
   `PostToolUse`、`SubagentStart`、`SubagentStop`、`SessionEnd`，POST 到
   `http://127.0.0.1:<port>/activity/<secret>/<sessionId>/<kind>`。不需要 curl／helper。
   Token 用量不讀 transcript：Agent 工具的 `PostToolUse` 回應已帶 `totalTokens`／`totalToolUseCount`。
3. **後端**（新模組 `src-tauri/src/claude_activity/`）：每個 session 維護 subagent 樹
   （id、父節點、描述、狀態、起訖時間、目前工具、結果摘要）。token／費用於
   `SubagentStop` 時依 transcript 路徑彙總。狀態變更 emit `claude-activity://{sessionId}`，
   並提供 `claude_activity_snapshot(sessionId)` 指令。
4. **前端**：終端機分頁右側可收合側欄。掛載時先取快照再訂閱事件（避免事件在訂閱前送出
   而遺失）。沒有任何事件的 session 不顯示側欄。

## 錯誤處理與邊界

- **hook 不可影響 Claude**：接收器收到事件立即回 `{}`；AITerm 未開或端點不通時 HTTP hook
  連線失敗不影響 Claude，僅無畫面。
- **事件不可信**：端點只收 localhost，驗證 `AITERM_SESSION_ID` 對應真實 PTY session，
  未知 session 的事件丟棄。
- **亂序／遺失**：`Stop` 先於 `Start` 時補建節點再標完成；重複事件冪等。Claude 結束
  （`SessionEnd`：/exit、Ctrl+C、/clear）時整張圖清空、側欄消失；PTY 關閉時丟棄該 session 狀態。
- **範圍**：只對 AITerm 開的 PTY、以 `claude` 名稱啟動者生效。

## 測試

- Rust：事件→樹的狀態機（亂序、重複、缺 Start、PTY 關閉收尾）、端點 session 驗證；
  每個新測試先證明會紅。
- shell 函式：以假 `claude` 記錄收到的參數（真的啟動 bash／zsh），驗證補上 `--settings`、
  不重複注入、不蓋使用者別名；PowerShell 驗腳本內容。
- 前端：側欄於快照與即時事件下的渲染；無事件時不出現；收合狀態。
- 實機：先用假 claude 送事件驗收畫面，再用真 claude 跑含 subagent 的任務。

# 派工卡片：額度用完後自動續跑 — 設計

日期：2026-10-16
狀態：使用者同意方向（「好」），授權直接實作

## 目標

派出去的 claude 碰到訂閱額度上限時，卡片不要被判「卡住」失敗、也不要被誤判完成；
使用者換帳號（任何地方 `/login`）或過一段時間後，AITerm 自動在那個終端機送「繼續」，不需要使用者下指令。

## 實測事實（2026-10-16，claude 2.1.300，假 API 伺服器回 429）

- Claude Code 有 `StopFailure` hook；`type: "http"` 的 hook 會把 JSON POST 到指定網址，
  額度類錯誤帶 `"error":"rate_limit"`（其他值：`authentication_failed`、`billing_error`、`overloaded`…）。
  HTTP hook 不經過 shell，三平台一致。
- 互動模式碰到 429 時沒有裸 BEL（只有 OSC 結尾的 BEL）。
- 目前登入的帳號記在 `~/.claude.json`（或 `$CLAUDE_CONFIG_DIR/.claude.json`）的 `oauthAccount.accountUuid`，`/login` 會更新它。
- Claude Code 自己有 `autoContinueAtUsageLimit`（等到重置自動繼續，或改跳 `/rate-limit-options` 選單）。
  它只等重置時間、不管換帳號。

**未驗證**：已在跑的 claude 在別處 `/login` 換帳號後，下一個請求是否改用新帳號。要靠真機驗收確認。

## 設計

1. **訊號**：非互動的 claude 派工一律加 `--settings "<task_dir>/claude-settings.json"`，檔案內容是三個 HTTP hook，
   都指向 AITerm 在 `127.0.0.1` 隨機埠開的接收器（網址含隨機密鑰與 task id）：
   - `StopFailure`：`error == "rate_limit"` → 卡片進入「等待額度」。
   - `PostToolUse`、`Stop`：代表它又在工作（例如 Claude 自己等到重置後續跑了）→ 解除「等待額度」。
   路徑含 `"`、`$`、`` ` ``、`%` 時不加旗標（退回原行為），避免各 shell 的引號差異。
2. **狀態**：記憶體裡的等待表（task → 分頁、時間、當時的帳號）＋資料表欄位 `tasks.quota_wait_at`（給畫面用）。
3. **監看**：等待額度期間，`monitor::watch` 不採計 bell（防誤判完成）、不做卡住判定；完成標記照常有效。
4. **續跑**：每 15 秒檢查一次。帳號跟進入等待時不同，或已經等了 10 分鐘 → 對該分頁送 Esc（關掉額度選單／取消 Claude 自己的倒數），
   0.6 秒後送「繼續」並提交，然後解除等待。又撞上限的話 hook 會再把它標回等待。
5. **畫面**：Agent 卡與看板卡片顯示「⏸ 額度用完：換帳號後自動繼續（每 10 分鐘重試）」。

## 測試

- Rust：hook 設定檔內容；`rate_limit` 判斷；帳號解析；何時該續跑（換帳號／逾時／都不是）；引號安全判斷；
  watch 在等待期間忽略 bell 與卡住；接收器收到 StopFailure 會標記、收到 PostToolUse 會解除；資料表欄位。
- 前端：Agent 卡與看板卡片的等待提示。
- 實機：換帳號續跑要使用者配合一次（或等真的撞到上限）。

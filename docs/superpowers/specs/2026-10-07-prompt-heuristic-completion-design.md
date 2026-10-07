# 沒有 OSC 133 時的後備指令完成判斷

## 問題
AI agent 靠 shell 送的 OSC 133 `D;<code>` 得知指令結束。ssh 進 QNAP（busybox/bash 3.2，沒有注入）
或遠端 host 開的是裸 `sh` 時沒有這個訊號，agent 等不到完成，只能由使用者手動點。

## 設計
- `lib/promptDetect.ts`：`looksLikePrompt(line)`（偏保守：符號在行尾，前面要有空白或 user@host 形式；
  裸 `>` 是 PS2 不算）與 `lastNonEmptyLine`。
- `useTerminalBlocks`：只對「有人等完成」的區塊（登記了 onComplete）啟用。`appendOutput` 每次重設
  `QUIET_PROMPT_MS`（1500ms）計時器；到期時若游標所在行（退而求其次 rawOutput 最後一行）像提示字元，
  以 `finalizeBlock(id, 0, { exitUnknown: true })` 結案。有 OSC 133 的 shell 先送 D，不受影響。
- 結案區塊 `exitUnknown: true`、`exitCode: undefined`；agent 回報文字用 `describeExit`，
  告訴 AI「結束狀態未知，請從輸出判斷」，不謊報 0。

## 與既有機制的關係
卡住偵測（120s 安靜→使用者選擇）仍在，負責「安靜但不像提示字元」的情況（密碼提示、heredoc）。

## 已知限制
- 提示字元樣式很特殊（無結尾符號）的 shell 認不出，會退回卡住偵測。
- 拿不到 exit code。
- 根治遠端 host 的情形另見 shell.rs `prefer_bash_over_sh`。

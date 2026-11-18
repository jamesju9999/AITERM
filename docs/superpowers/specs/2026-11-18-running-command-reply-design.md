# 指令執行中的回覆輸入

日期：2026-11-18
狀態：兩期都已實作（尚未經 Windows 實機驗收）

## 問題

輸入框送出的任何文字都會被當成「新指令」：`submitCommand` 會把還在跑的卡片強制結案（exit -1），再開一張新卡片。
指令執行中向使用者提問（winget 的 `[Y] 是 [N] 否:`、`Continue? [y/N]`、密碼）時，使用者在輸入框打 `Y`，
結果是舊卡片被截斷、多出一張叫 `Y` 的卡片，內容亂（實機畫面：空白行、重複的進度條）。
空輸入框時方向鍵／Enter／Esc／Ctrl+C 已會轉給 PTY，但文字沒有。

## 第一期：回覆模式（B 方案）

- 有指令執行中（最後一張卡片 `running`）時，輸入框進入**回覆模式**：
  - 輸入框上方顯示一條提示列：「↳ 回覆執行中的指令：`<指令>`」，右邊兩顆按鈕：**停止**（送 Ctrl+C）、**結束這張卡片**（強制結案、回到正常模式）。
  - 邊框換色；placeholder 改成「輸入的文字會送給執行中的指令（Enter 送出）」。
  - Enter 只把「文字＋\r」寫進 PTY，**不開新卡片、不結案舊卡片**；回覆不存進指令歷史（可能是密碼）。
  - 空輸入框的 Enter／方向鍵／Esc／Ctrl+C 行為不變（已是 raw 轉發）。
- 只改輸入框這條路徑。Agent 迴圈、AI 預覽、`cd` 選單等自己呼叫 `submitCommand` 的地方不變。
- **唯一風險與出口**：卡片卡在 running（shell 沒送結束訊號）時會吞掉輸入 → 「結束這張卡片」＋「停止」永遠可用；既有的安靜後備結案（`QUIET_PROMPT_MS`）也照常運作。

## 第二期：看出「正在等你輸入」

`src/lib/inputPrompt.ts` 的 `detectInputPrompt(最後一列文字)`：
- **yesno**：`[Y/n]`、`[y/N]`、`(yes/no)`、`[Y] 是 [N] 否:`（winget 中文）→ 選項 `["Y","N"]`（yes/no 則 `["yes","no"]`）。
- **anykey**：`Press any key`、`按任意鍵` → 選項 `[""]`（只送 Enter）。
- **password**：`password`、`passphrase`、`密碼` 結尾冒號 → 遮罩。
- 其他 → `null`（不猜）。
只有在「最後一張卡片 running，且它最後一列有內容」時才偵測；輸出繼續增加、最後一列變了，提示自然消失。

- 卡片標頭：偵測到時，「執行中」改顯示「⏳ 等待你的輸入」。
- 回覆列：yesno／anykey 顯示快速按鈕（`Y` `N` `Enter`），一鍵送出。
- 密碼：輸入框遮罩（`-webkit-text-security: disc`），內容不進歷史、不顯示在卡片。

## 不做

- 不做「2 秒內有輸出」之類的自動判斷（會誤判）。
- 不改 Agent 迴圈；不改全螢幕程式／Kitty 協定時由終端機本身收鍵的既有路徑。
- 遠端觀看端（RemoteTerminalView）本期不動。

## 測試

- `inputPrompt.test.ts`：各樣式（含 winget 中文原文）與不該誤判的句子。
- `WarpInput`：回覆模式不呼叫 `onSubmit`、呼叫 `onReply`；停止／結束按鈕；遮罩；快速按鈕。
- `TerminalBlockCard`：等待輸入徽章。

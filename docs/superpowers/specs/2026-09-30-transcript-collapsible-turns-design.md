# 對話記錄：依使用者提示折疊執行記錄

日期：2026-09-30
範圍：僅前端（`src/components/TaskBoard/`、`src/lib/i18n.ts`），不動儲存格式與後端。

## 目的

任務看板「對話記錄」視窗的「執行記錄」目前是一整塊 `<pre>`。使用者想一眼看出自己對 Claude Code 下過哪些命令（提示），其餘輸出預設折疊。

## 「命令」的定義

使用者輸入給 Claude Code 的提示。實機驗證發現真實 transcript 有兩種格式：

1. **session-log 格式（主要）**：`src-tauri/src/tasks/session_log.rs` 產生，每則提示是行首的 `使用者：文字`，多行內容第二行起縮排兩格，其餘為 `Claude：`、`〔工具〕`、`〔結果〕`。
2. **終端機序列化格式（備用）**：以 `❯ ` 開頭、後面有文字的行。

只要有任何一行符合格式 1，就整份用格式 1 解析；否則用格式 2。Agent 自己執行的 shell 指令不在此範圍。

## 設計

### 1. 解析：`parseTranscriptTurns(text)`（`transcriptUtils.ts`）

回傳 `{ preamble: string; turns: { prompt: string; output: string }[] }`。

- 提示行：符合 `^\s*❯ (\S.*)$`。單獨的 `❯`（Claude Code 底部空輸入框）不算。
- 輸入框防護：輸入框的 `❯` 行夾在兩條 `─` 分隔線之間，即使後面有文字（使用者正在打字、尚未送出）也不算提示。
- 每輪 `output` 是該提示行之後、下一則提示行之前的所有文字（不含提示行本身）。
- 第一則提示之前的文字放進 `preamble`。
- 續行：提示若折成多行，本輪不特別處理，多出的行會落在 `output` 開頭；標題只取提示第一行。
- 沒有解析到任何提示時，回傳 `turns: []`。

前置處理仍沿用 `collapseConsecutiveDuplicateLines`，在解析之前套用。

### 2. 顯示：`TranscriptDialog.tsx`

- 「執行記錄」區塊改為清單：每輪一個可點擊的標題列 `▸ #N 提示第一行…`（單行截斷），點擊展開／折疊該輪 `output`。預設全部折疊。
- `preamble` 若非空白，顯示為一個預設折疊的「開頭輸出」項目。
- 區塊標題旁放三個控制：「全部展開」「全部收合」「原始文字」切換。
- 「原始文字」開啟時，顯示現在的 `<pre data-testid="task-transcript-raw">`，行為與現況完全相同。
- `turns` 為空（沒解析到提示）或文字為空時，自動使用原始文字顯示，不顯示折疊控制。
- 展開狀態存在元件內 `useState`（以輪次索引為 key），關閉對話框即重置，不寫 localStorage。

### 3. i18n

`zh-TW` 與 `en` 各補：`board_transcript_expand_all`、`board_transcript_collapse_all`、`board_transcript_show_raw`、`board_transcript_show_turns`、`board_transcript_preamble`。`en` 是 `{...zhTW,...enRaw}` 合併，tsc 抓不到漏字串，兩邊都要實際寫。

### 4. 樣式

`index.css` 新增 `.task-transcript-turn`、`.task-transcript-turn-head`、`.task-transcript-turn-body`。輸出沿用 `.task-transcript-raw` 的字型與 `pre-wrap`。整個清單維持 `.task-field--grow` 的 flex 分配與 `overflow` 捲動，不可讓最大化模式失效。

## 已知風險

- `❯` 格式是依 Claude Code TUI 推測、未經真實樣本驗證；實機證實主要格式是 `使用者：`，`❯` 僅作備用。
- transcript 只取最後 300,000 字元，尾段截斷可能讓第一輪的提示行被切掉，該段會落入 `preamble`，屬預期行為。
- 不同語系或版本的 Claude Code 若改用 `>` 作提示符，則會退回原始文字顯示，不會壞掉。

## 測試

`transcriptUtils.test.ts`（先寫、先確認會紅）：
- 多則提示：切成正確輪數，`prompt` 與 `output` 正確。
- 只有輸入框的空 `❯`：`turns` 為空。
- 輸入框中未送出的 `❯ 文字`（夾在 `─` 線之間）：不算提示。
- 提示之前有文字：進入 `preamble`。
- 完全沒有提示：`turns` 為空。

`TranscriptDialog` 元件測試：
- 預設折疊，點標題後才看得到輸出。
- 「全部展開／收合」生效。
- 「原始文字」切換後出現 `task-transcript-raw` 且內容與原文一致。
- 沒有提示時直接顯示原始文字、無折疊控制。

## 不做

- 不列出 Agent 執行過的 Bash 指令。
- 不做提示搜尋、不記住展開狀態、不改儲存格式或後端。

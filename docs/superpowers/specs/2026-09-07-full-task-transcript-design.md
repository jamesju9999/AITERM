# 工作看板：完整對話記錄

**日期**：2026-09-07
**狀態**：已實作（分支 `feat/full-task-transcript`，待真機驗證）

## 問題

工作看板保存的 `transcript.txt` **只有終端機的最後一個畫面**——實機十份記錄全部落在 2.5–7 KB、31–45 行，正好是一個終端機高度。

成因在 `transcriptUpgrade.ts`：

```ts
serializeAddon.serialize({ scrollback: 0 })
```

而且「把 scrollback 調大」**行不通**。Claude Code 是全螢幕 TUI（進 `\x1b[?1049h` 替代畫面），替代畫面依規格沒有捲動緩衝區——早期的對話輪次不是捲出去了，是被原地覆蓋，從來不曾存在於任何緩衝區。從終端機側救不回來。

連帶影響：工作報告第一階段的摘要就是讀這份記錄，所以**報告看到的也只有最後一個畫面**。

## 已驗證的事實

以下每一條都經過實測，不是推論：

1. **Claude Code 自己會寫完整記錄**，位置是
   `~/.claude/projects/<cwd 的每個 / 換成 ->/<session-id>.jsonl`，
   內容是結構化的每一輪 user / assistant / 工具呼叫。
2. **`claude --session-id <uuid>` 可以指定檔名**，所以不必事後猜哪個檔案對應哪張卡片。
3. **邊跑邊寫**，不是結束才落地——卡片一跑完檔案就已經完整。
4. **實測體積**：一次派工任務 48 KB。（跑滿一整天的互動對話是 64 MB，但那不是派工任務的量級。）
5. **體積組成**：73% 是 `user` 記錄，因為 Claude Code 把**工具回傳結果**也記成 user 訊息。真正的人類輸入佔比很小。
6. **資料夾信任狀態**存在 `~/.claude.json` 的 `projects["<絕對路徑>"].hasTrustDialogAccepted`。

### 調查過程中被推翻的假設（避免後人重走）

實測初期發現派工的 session 完全沒有落地，一度以為是產品缺陷。真正原因是**開發時從 Claude Code 對話內啟動 dev server**，App 繼承了 `CLAUDE_CODE_CHILD_SESSION=1` 這組子會話標記，互動模式的記錄因而被抑制。

A/B 對照確認：汙染環境下手動跑 `claude` 完整一輪 → 無檔案；乾淨環境（從 Dock 啟動）同樣操作 → 有檔案。

**這個假設一度被錯誤地「排除」過**——因為那次 A/B 用的是 `-p` 模式，而 `-p` 不受影響，只有互動模式會被抑制。教訓：對照組必須跟真實情境用同一種執行模式。

### 寫計畫與實作期間補測到的事實

以下是這份規格寫完之後才量到的，每一條都改變了實作：

7. **信任畫面的 PTY 輸出裡一個空白位元組都沒有。** 實跑 `claude` 進未信任目錄、擷取原始 bytes，`' ' in raw == False`。TUI 用游標移動排版。所以第 6 節設想的「比對特徵字串」若寫成 `contains("Yes, I trust this folder")` **永遠不會命中**——比對前必須濾掉所有空白。
8. **預設選中的是 `No, exit`，而且選項順序不保證。** 只送 `\r` 會直接退出；但寫死「往下一次」在上游對調選項那天會反過來按到 `No, exit`，把派工殺掉。改成同時定位游標與 `Yes` 行、算相對位移，任一定位不到就什麼都不送。
9. **`claude_command` 可能不是 claude。** `dispatch.rs` 的 `NO_TUI_QUIET_MS` 是刻意為非 TUI 指令留的路，無條件接 `--session-id` 會讓那些指令直接啟動失敗。加了 `looks_like_claude` 閘門。
10. **`isSidechain` 在主 session 檔裡永遠是 false**（掃過六份真實檔案）——子代理的對話不寫進主檔，不需要任何過濾。
11. **`user` 記錄的 content 陣列會有 2–3 個 block**（六份檔案裡共 60 筆），`assistant` 固定 1 個。渲染必須走完整個陣列。
12. **`printf '%b'` 對參數的八進位解碼是殼相依的。** zsh 的內建 printf 不解碼，而 `pty/shell.rs` 讀 `$SHELL`（macOS 預設 zsh）。測試裡要印出非 ASCII 字元請直接送字面 UTF-8 位元組配 `%s`。

## 設計

### 1. 資料流與儲存

`RealDispatcher::dispatch` 在 spawn 前產生一個 UUID，送進終端機的指令變成：

```
<claude_command> --session-id <uuid>
```

`claude_command` 仍取自設定（預設 `claude`），只是後面接上旗標。UUID 寫進卡片的新欄位 `session_id`。

任務完成時，`write_transcript` 之外多做一步：把

```
~/.claude/projects/<project_dir 編碼後>/<session_id>.jsonl
```

**原封不動**複製到

```
<專案>/tasks/<卡片 id>/session.jsonl
```

並把路徑寫進新欄位 `session_path`。

**為什麼原封不動而不精簡**：一次派工只有幾十 KB 到數 MB，磁碟成本遠低於提早丟資料的風險。要精簡永遠來得及，丟掉的救不回來。

**路徑編碼規則**：把絕對路徑的每一個 `/` 換成 `-`。路徑本身既有的 `-` 保留不動（所以 `/private/tmp/-Users-x` 會變成 `-private-tmp--Users-x`）。這是整份設計最容易寫錯的一行，必須有獨立測試。

### 2. 保留既有的終端機擷取

`transcript.txt` 與 `transcript_path` **完全不動**。

理由：`claude` 根本沒啟動、卡在信任提示、或使用者把 `claude_command` 設成別的東西時，終端機畫面是**唯一的診斷線索**。JSONL 在那些情況下不存在。

### 3. 讀取與呈現

`tasks_read_transcript` 的簽章不變（仍回傳 `String`），內部改成：

- 有 `session_path` 且讀得到 → 渲染 JSONL 成逐輪對話回傳
- 否則 → 退回讀 `transcript_path`（現行行為）

這樣 `TranscriptDialog` 與 `useWorkReport` 兩個消費端**都不必改介面**。

渲染放在新模組 `src-tauri/src/tasks/session_log.rs`，是純函式（吃 `&str`、吐 `String`），與檔案系統和 Tauri state 無關，好測。

### 4. 渲染規則

逐行解析 JSONL，只處理 `type` 為 `user` / `assistant` 的記錄，其餘（`mode`、`attachment`、`file-history-snapshot`、`cost-state`、`system` 等）一律跳過。

`message.content` 有兩種形態：

| 形態 | 處理 |
|---|---|
| 字串（user） | 輸出 `使用者：<內容>` |
| `[{type:"text"}]` | 輸出 `Claude：<text>` |
| `[{type:"thinking"}]` | **跳過**（思考過程不是工作記錄） |
| `[{type:"tool_use"}]` | 輸出 `〔工具〕<name> <第一個參數的簡短摘要>` |
| `[{type:"tool_result"}]` | **跳過**（見下） |

**為什麼保留工具呼叫、省略工具回傳**：呼叫回答「它做了什麼」，那是工作記錄的核心；回傳是雜訊，而且佔了整個檔案 73% 的體積。摘要要的是「做了什麼、結果如何」，不是每個檔案的完整內容。

單行壞掉（JSON 解析失敗）就跳過那一行，不中止整份渲染——一行壞掉不該讓整份記錄消失。

### 5. 工作報告

`buildSummaryPrompt` **不改**。它拿到的字串會自動變成完整對話，因為來源在後端就換掉了。

這是刻意的設計選擇：把變更收斂在 `tasks_read_transcript` 這一個接縫，前端兩個消費端都不動。

### 6. 資料夾信任提示

沒信任過的目錄會讓 `claude` 停在「Do you trust this folder?」畫面，直到卡住偵測介入才收掉——期間卡片看起來像在執行，實際上什麼都沒發生。

**做法：偵測 PTY 輸出中的信任畫面，送出接受的按鍵。**

明確**不採用**改寫 `~/.claude.json` 的做法。那是 Claude Code 與 AITerm 共用的設定檔（本機已有 50 個專案的資料在裡面），兩邊同時寫有損毀整份設定的風險；這個 repo 過去就因為盲目寫入使用者正在編輯的設定檔，造成整份設定解析失敗。

送按鍵的缺點是 Claude Code 改 UI 文字就會失效，但**失效的後果只是退回現行行為**（卡住、被偵測收掉），不會弄壞任何東西。風險從「可能損毀全域設定」降到「可能不生效」。

偵測邏輯放在 `dispatch.rs`，與現有的 `wait_until_settled` 同一層：在等待 TUI 就緒的迴圈裡順便比對信任畫面的特徵字串，命中就送出選擇。

### 7. 失敗與退回

以下情況一律**安靜退回**既有的 `transcript.txt`，不對使用者報錯：

- session 檔不存在（claude 沒啟動、或使用者換了 `claude_command`）
- 複製失敗（權限、磁碟）
- JSONL 完全解析不出任何對話

理由與現行的 `tryUpgradeTranscript` 一致：原本的東西還在，沒有東西壞掉，不值得打斷使用者。失敗寫進 `eprintln!` 供診斷。

### 8. 重新派工

重新派工會產生新的 UUID 與新的 session 檔，`session.jsonl` 直接覆蓋——與 `transcript.txt` 現行行為一致。舊的一次執行的記錄不保留（要保留舊版是另一個題目）。

## 資料庫變更

`tasks` 表新增兩欄，沿用既有的 ALTER TABLE 慣例（欄位已存在時失敗屬正常，刻意丟掉錯誤）：

```sql
ALTER TABLE tasks ADD COLUMN session_id TEXT
ALTER TABLE tasks ADD COLUMN session_path TEXT
```

`TaskRow` 對應加上 `session_id: Option<String>` 與 `session_path: Option<String>`；前端 `TaskRow` 型別同步，所有測試 fixture 要補這兩欄（`tsc -b` 會抓出漏的）。

## 測試策略

| 對象 | 方式 |
|---|---|
| 路徑編碼 | 純函式測試，含路徑本身帶 `-` 的案例 |
| JSONL 渲染 | 純函式測試，fixture 取自真實 session 檔的片段（含 text / thinking / tool_use / tool_result 四種形態，以及一行壞掉的 JSON） |
| 複製邏輯 | tempfile，含「來源不存在」的退回路徑 |
| 讀取優先序 | 有 session_path 用它、沒有退回 transcript_path、兩者都沒有回空字串 |
| 信任畫面偵測 | 純函式比對特徵字串，含不該誤判的近似輸出 |

**fixture 必須取自真實檔案**，不可手寫——手寫的乾淨樣本會替錯誤的假設背書。

每個關鍵測試都要做突變驗證：測試資料必須讓正確與錯誤的實作產生**不同**結果，否則測試是空的。

## 不在這次範圍

- 舊卡片的回溯補齊（沒有 session_id，無從對應）
- 保留多次執行的歷史記錄
- 對話記錄視窗的排版重做（仍是純文字 pre 區塊）
- 大檔案的分頁載入（實測量級用不到，真的遇到再說）

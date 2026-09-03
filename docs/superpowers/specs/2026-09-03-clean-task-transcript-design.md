# 工作看板「對話記錄」乾淨化 — 設計

日期：2026-09-03
狀態：待使用者複審

## 問題

工作看板卡片的「對話記錄」對話框，目前顯示的是任務完成當下後端存下的原始終端機輸出（`transcript.txt`）。這份存檔完全沒有解讀游標移動、`\r` 重繪、畫面覆寫等終端機控制語意——只是把 ANSI escape 碼移除後的逐位元組原始輸出，所以 Claude Code TUI 的思考動畫、逐字重繪畫面，全部被當成獨立文字疊加上去，變成一堆重複亂碼，很難閱讀。

使用者要求：**即使分頁後來被關閉、或 app 重啟過，看到的「對話記錄」內容仍然要是乾淨的**——排除了「打開對話框當下才即時處理」這種做法（那種做法在分頁已關閉時沒有任何補救辦法，等於沒解決問題）。

## 現況調查（設計的事實基礎）

以下都是實際讀過程式碼確認的，不是推測：

| 事實 | 位置 |
|---|---|
| 任務完成當下，後端把 `pty.get_recent_output(tab_id, 200_000)`（ANSI 碼已濾除，但沒有解讀重繪語意的原始文字）寫進 `<app-data>/AITERM/tasks/<task_id>/transcript.txt`，並把路徑記進 `tasks` 資料表的 `transcript_path` 欄位 | `src-tauri/src/tasks/scheduler.rs::write_transcript`，呼叫點在 `RealDispatcher::dispatch` 內、`monitor::watch` 回傳之後 |
| 對話框讀取用 `tasks_read_transcript` 指令，單純 `fs::read_to_string(transcript_path)` | `src-tauri/src/commands/tasks.rs::tasks_read_transcript` |
| `pty.get_recent_output` 底層的 `strip_ansi` 只是逐位元組移除 ANSI escape 序列，**不解讀游標移動/覆寫**——這是亂碼的根本原因 | `src-tauri/src/pty/ansi.rs::strip_ansi`、`src-tauri/src/pty/session.rs::get_recent_output` |
| 前端每個分頁已經在用 `@xterm/xterm` 的 `Terminal` 物件即時渲染畫面——這正確處理了游標移動/重繪，你在畫面上看到的乾淨結果，就是 xterm.js 即時重建出來的。這份重建結果**只存在於該 `Terminal` 物件的記憶體裡，從未落地存過** | `src/components/TerminalView.tsx:1064`（`new Terminal(...)`），無任何序列化/持久化邏輯 |
| `Terminal` 物件建立在一個不依賴 `sessionId` 的掛載 effect（約 1064 行起），`fit`/`search` 兩個官方 addon 在同一處用 `term.loadAddon(...)` 掛上；`sessionId`（PTY session id，即工作看板的 `tab_id`）要等 PTY 建立後才有值，另一個依賴 `sessionId` 的 effect（約 996 行起）處理 PTY 相關邏輯；`term.dispose()` 在該掛載 effect 的清理函式裡（約 1751 行） | `src/components/TerminalView.tsx` |
| `@xterm/addon-serialize` 是 xterm.js 官方套件（目前不是本專案依賴），提供 `.serialize()` 把目前畫面內容序列化回字串，會保留樣式用的 ANSI 碼（不是純文字），沒有內建「輸出純文字」選項 | xterm.js 官方套件文件 |
| 前端目前沒有任何 ANSI 濾除工具；也沒有任何「跨元件依 tab id 拿到對應 xterm 實例」的登記簿——每個 `TerminalView` 的 `term` 只活在自己元件內部 | 全庫搜尋 `stripAnsi`／`Map<string, Terminal`／`terminalsRef` 均無結果 |
| 這個 app 已有「小型 module-level 工具檔持有跨元件共用邏輯」的既有慣例（例如 `src/lib/tabAgentProgress.ts`），以及「子元件用 register/unregister 回呼把自己的控制代碼登記到父層」的既有慣例（`registerCloseGuard`/`unregisterCloseGuard`，`src/components/TerminalApp.tsx`） | `src/lib/tabAgentProgress.ts`、`src/components/TerminalApp.tsx` |
| `TaskBoardView` 已經透過 `onTasksUpdated` 事件驅動的 `refresh()` 持續重抓 `tasks_list`，所以能自然偵測到「某張卡片剛從別的狀態變成 `done`」——不需要新的事件機制 | `src/components/TaskBoard/index.tsx` |
| `transcriptUtils.ts` 已經有 `collapseConsecutiveDuplicateLines`（收斂連續重複行），是先前對這個亂碼問題的治標處理，獨立成檔是因為 `react-refresh/only-export-components` 這條 eslint 規則不允許元件檔案混雜一般函式匯出 | `src/components/TaskBoard/transcriptUtils.ts` |

**結論**：要讓「分頁關閉後仍然乾淨」，必須在**任務完成當下、分頁仍然活著的那個時間點**就把 xterm.js 重建好的畫面序列化、覆寫進存檔——而不是等使用者之後打開對話框才處理（那時分頁可能已經不在了）。

## 範圍

**含：**

- 新增一個 module-level 登記簿，讓每個分頁的 xterm `Terminal`（+ 掛載好的 `SerializeAddon`）能被跨元件用 `tab_id` 查到
- `TerminalView` 幫自己的 xterm 掛上 `@xterm/addon-serialize`，並在該分頁的 PTY session 建立後登記進登記簿、分頁卸載時取消登記
- 工作看板偵測到「某張卡片剛變成 `done`」時，查登記簿看該分頁還在不在；還在的話：序列化畫面 → 濾除 ANSI 碼 → 呼叫新指令覆寫存檔
- 新增後端指令 `tasks_save_transcript(id, text)`：把給定文字覆寫進該卡片既有的 `transcript_path` 檔案
- 前端新增一個純文字 ANSI 濾除工具（放進既有的 `transcriptUtils.ts`）
- `TranscriptDialog` 本身不需要任何改動——它一直都只是單純讀檔案，讀到的內容自然就是（盡力而為下）乾淨版本

**不含：**

- 後端終端機模擬器（例如導入 `vt100` 之類的 Rust crate，在後端重建畫面狀態）——不需要，前端已經有 xterm.js 在做這件事
- 保證 100% 情境都乾淨——如果任務完成的瞬間分頁剛好已經被手動關掉，那次錯過就錯過，存檔停在原始版本（見「已知限制」）
- 修改任務完成當下後端原有的 `write_transcript` 存檔時機或機制——那個「不管分頁在不在都先存一份」的保底行為完全不動
- 針對「已中斷」（cancelled）的任務做特別處理——這類任務一樣嘗試升級存檔，失敗或跳過都無妨，不特別排除

## 架構

### 為什麼是「完成當下升級存檔」而不是「打開對話框時即時轉換」

打開對話框時才轉換的做法，乾淨版本的來源（xterm.js 記憶體）如果那時分頁已經關閉就不存在了——完全沒辦法補救。改成在任務**剛完成、分頁幾乎一定還開著**的那個時間點就把乾淨版本存下來、覆寫掉原始檔案，之後不管分頁在不在，讀到的都已經是存好的乾淨版本。這是使用者明確要求的行為，也是唯一能滿足這個要求的做法（除非做後端終端機模擬器）。

### 為什麼登記簿用 module-level 單例、不用 React context 或 props 一路往下傳

`TerminalView` 跟 `TaskBoardView` 是 `TerminalApp`底下的兩個獨立分支（分頁陣列 vs. 看板覆蓋畫面），彼此不是父子關係。如果要用 props 傳遞，得從 `TerminalApp` 同時往下傳給兩邊，牽動這個已經很大的元件的 prop 介面。這個登記簿的操作是純粹指令式的（「登記」「查詢」「取消登記」），不需要觸發任何 React 重繪，跟現有 `registerCloseGuard` 那種「子元件把控制代碼交給父層」的模式性質不同、更接近純粹的旁路資料存取——比照 `tabAgentProgress.ts` 這種既有的小型 module-level 工具檔慣例，用一個獨立檔案裡的 plain `Map` 就夠，不需要途經 `TerminalApp`。

### 模組佈局

```
src/lib/terminalInstanceRegistry.ts   新檔。registerTerminal(tabId, term, serializeAddon)
                                       unregisterTerminal(tabId)
                                       serializeTerminal(tabId): string | null
                                         內部呼叫 serializeAddon.serialize()，
                                         找不到就回 null（呼叫端當作「分頁不在」處理）
```

`TerminalView.tsx`：
- 掛上 xterm 時（約 1064 行那個掛載 effect），額外 `const serializeAddon = new SerializeAddon(); term.loadAddon(serializeAddon);`，比照 `fitAddonRef`/`searchAddonRef` 的既有模式存進 `serializeAddonRef`。
- 在依賴 `sessionId` 的那個 effect（約 996 行起，PTY 相關邏輯所在）裡，`sessionId` 一旦有值就呼叫 `registerTerminal(sessionId, termRef.current, serializeAddonRef.current)`；該 effect 的清理函式（或 `sessionId` 變成別的值時）呼叫 `unregisterTerminal(sessionId)`。
- 分頁整個卸載時（掛載 effect 的清理函式、`term.dispose()` 前後皆可）保險起見再呼叫一次 `unregisterTerminal(sessionId)`，避免任何路徑遺漏。

`src/components/TaskBoard/index.tsx`：
- `refresh()` 目前只是 `setTasks(rows)`。改成：先讀出「舊的 id→status 對照表」，`setTasks(rows)` 之後，找出「舊狀態不是 `done`、新狀態是 `done`」的卡片，逐一呼叫一個新函式 `tryUpgradeTranscript(taskId, tabId)`（見下）。這個判斷只需要一個 `useRef<Map<string, TaskStatus>>` 記住上一次的狀態快照，不需要新的 state。**第一次載入**（這個 ref 還是空 Map 的時候）只負責把快照填好，不對任何卡片觸發升級——避免把「app 剛啟動、讀到一張早就已完成的舊卡片」誤判成「剛剛才完成」。只有這個 ref 裡「先前確實記過某個非 done 狀態、這次變成 done」的才算數。

`src/components/TaskBoard/transcriptUpgrade.ts`（新檔，純函式，跟 `transcriptUtils.ts` 分開，因為這個檔案需要 import ipc 而 `transcriptUtils.ts` 目前是零依賴的純文字處理）：
```ts
export async function tryUpgradeTranscript(taskId: string, tabId: string | null): Promise<void> {
  if (!tabId) return;
  const raw = serializeTerminal(tabId);   // from terminalInstanceRegistry
  if (raw === null) return;               // 分頁不在，放棄，維持原始存檔
  const clean = stripAnsiCodes(raw);      // from transcriptUtils
  try {
    await saveTranscript(taskId, clean);  // 新 ipc 函式
  } catch {
    // 盡力而為：存檔失敗不影響任何使用者可見的流程，靜默放棄即可，
    // 原始存檔還在，對話框仍然讀得到東西。
  }
}
```

`src/components/TaskBoard/transcriptUtils.ts`：新增 `stripAnsiCodes(text: string): string`，用正則移除 CSI/SGR 這類 ANSI 序列（`xterm.js` 的 `serialize()` 輸出是規範良好的標準序列，不像後端要防禦真正任意的原始 PTY 二進位輸入，用簡單正則即可，不需要後端 `strip_ansi` 那種逐位元組 UTF-8 邊界檢查）。

### 後端

不新增 `store.rs` 函式——`store::get_task` 已經存在，直接在 command 層重用，比照 `tasks_read_transcript` 現有的做法（該指令也是直接在 command 函式裡 `store::get_task` 再 `fs::read_to_string`，檔案 I/O 沒有下放進 `store.rs`；這個 repo 的既有慣例是 `store.rs` 只碰資料庫，檔案系統操作留在 command 層）：

`src-tauri/src/commands/tasks.rs` 新增：
```rust
#[tauri::command]
pub async fn tasks_save_transcript(id: String, text: String, db: State<'_, TasksDb>) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id).await.map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let path = row.transcript_path.ok_or_else(|| "no transcript path yet".to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}
```
`lib.rs` 註冊這個新指令（沿用既有 `tasks_*` 指令的登記方式）。**不 emit `tasks-updated`**——這只是把同一個檔案路徑底下的內容換成更乾淨的版本，卡片本身的任何欄位都沒變，沒有必要觸發整個看板重抓。

`src/ipc/tasks.ts` 新增：
```ts
export const saveTranscript = (id: string, text: string): Promise<void> =>
  invoke("tasks_save_transcript", { id, text });
```

### 資料流時序

```
任務完成 → 後端 finish_task + write_transcript（原始版本存檔，跟現在完全一樣，不變）
         → 前端下一次 tasks_list 重抓，偵測到這張卡片狀態變成 done
         → 查登記簿：這個 tab_id 的 xterm 還在嗎？
              在  → serialize() → 濾除 ANSI → tasks_save_transcript 覆寫存檔
              不在 → 什麼都不做，存檔維持原始版本
```

## 已知限制

- **這是「盡力而為」的升級，不是保證**。如果任務完成的瞬間，該分頁剛好已經被手動關閉（機率很低——分頁預設不會自動關），這次升級就補不上，存檔永遠停在原始雜亂版本。使用者若真的很在意這極少數情境，唯一的根治辦法是後端也做一個真正的終端機模擬器，重建畫面狀態後再存檔——這是完全不同量級的工程，這次不做。
- `serialize()` 的輸出仍然可能受限於 xterm.js 本身的 scrollback 設定（畫面外太久以前的內容可能已經被 xterm.js 自己捲出緩衝區），跟後端原始擷取的 20 萬字元上限是兩套獨立的容量限制，這次不特別對齊。
- 覆寫是「整份取代」，不是「附加」——如果同一個分頁後續又被拿去執行下一個任務（理論上 v1 不會發生，一個分頁只對應一次派工），沒有累加的問題需要處理。

## 測試

- **前端單元測試（`transcriptUtils.test.ts` 或併入既有測試檔）**：`stripAnsiCodes` 對含 SGR 顏色碼的字串輸出純文字；對已經是純文字的輸入原樣不動。
- **前端單元測試（`terminalInstanceRegistry.test.ts`）**：`registerTerminal`/`unregisterTerminal`/`serializeTerminal` 的基本讀寫；查詢不存在的 `tabId` 回 `null`。
- **前端單元測試（`TaskBoard/index.test.tsx` 新增案例）**：mock `terminalInstanceRegistry` 讓 `serializeTerminal` 回傳一段固定文字，讓某張卡片的 `listTasks` 回傳從非 `done` 變成 `done`，斷言 `saveTranscript` 被正確呼叫、內容是濾除 ANSI 後的版本；再測一次 `serializeTerminal` 回 `null`（分頁不在）的情境，斷言完全不呼叫 `saveTranscript`。
- **前端單元測試**：卡片狀態維持不變（例如本來就是 `done`，重抓後還是 `done`）不會重複觸發升級。
- **後端整合測試（`commands/tasks.rs` 或既有的 `tests/task_board.rs`）**：`tasks_save_transcript` 正確覆寫既有檔案內容；`transcript_path` 為 `None`（任務還沒完成過）或任務不存在時回錯誤而非 panic。
- **手動驗證**：真的派工一個任務、等它完成、確認分頁還開著時，`~/Library/Application Support/AITERM/tasks/<id>/transcript.txt` 的內容變乾淨（比對關閉分頁前後這個檔案的差異，看到覆寫確實發生）；再測一次「完成後立刻手動關閉分頁」的情境，確認存檔維持原始版本（沒有壞掉，只是沒升級）；打開對話框確認兩種情境下都讀得到東西、不會空白或報錯。

## 相關

`docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md`（工作看板原始設計，`transcript_path`/`write_transcript` 機制的出處）

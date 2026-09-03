# 工作看板 — 自動派工給 Claude Code — 設計

日期：2026-09-03
狀態：待使用者複審

## 問題

使用者想要一個工作管理清單，分成四欄：**計畫中 / 待執行 / 執行中 / 已完成**。
把「計畫中」的項目轉到「待執行」後，系統就自動把那項工作交給 Claude Code CLI
（`claude`）去執行，執行完自動落到「已完成」。看板本身要能離開視圖後工作照跑、
app 重開後卡片還在。

這不是要重新發明一套 agent 執行框架，而是在既有的 **agent 協調機制**上加一層
「佇列 + 排程 + 看板 UI」。派工、監看、idle 偵測全部重用既有實作。

## 現況調查（設計的事實基礎）

以下都是實際讀過程式碼確認的，不是推測：

| 事實 | 位置 |
|---|---|
| agent 協調已實作 `spawn_tab`：用 `PtyManager::create_with_app(app, size, cwd_path, None)` spawn 一個 PTY session，記進 `CoordinationRegistry`，可選擇性 `write` 一段啟動指令，然後 `app.emit("mcp-coordination-tab-spawned", { session_id, command })` | `src-tauri/src/mcp_server/coordination_ops.rs` |
| 前端 `TerminalApp.tsx` / `TerminalView.tsx` 已經會聽 `mcp-coordination-tab-spawned`，把後端已存在的 session **adopt** 成一個看得見的分頁（含補畫錯過的輸出、標示「由 agent 協調工具開啟」） | `src/components/TerminalView.tsx:88,1228,1739`、`src/ipc/mcpToolServer.ts:38-42` |
| `wait_for_idle` 已實作：250ms 輪詢，判斷 `idle = bell_count > baseline || marker_count > baseline`。`PtySession` 讀取迴圈對每個 chunk 掃 `contains_bare_bell` 與自己 `id` 專屬的 `<<AITERM_DONE:{tab_id}>>` 標記（含跨 chunk 邊界處理），各自遞增 `AtomicU64` | `src-tauri/src/mcp_server/coordination_ops.rs`、`src-tauri/src/pty/session.rs` |
| `send_input` 已實作：對 session `write` 一段文字 + `\r`（**用 `\r` 不用 `\n`**，raw-mode TUI 只驗證過 `\r` 能穩定觸發 Enter）。有 `request_done_marker: bool` 選項，會在任務文字後、等目標端因任務文字觸發新 bell（上限 `DONE_MARKER_WAIT_SECONDS = 15`）後，再分開寫入一段固定措辭的 done-marker 指示文字 | `src-tauri/src/mcp_server/coordination_ops.rs` |
| Rust 後端已有 OSC133 解析器，每個 PTY 輸出 chunk 即時掃「指令結束 + exit code」 | `src-tauri/src/pty/cd_parser.rs::find_exit_codes`，呼叫點 `src-tauri/src/pty/session.rs` |
| 「卡住」偵測既有做法：`TerminalView` 在 `onPtyData` 記 `lastPtyOutputAtRef`，門檻 `STUCK_IDLE_MS = 120_000`（安靜多久，不是跑多久——長建置/弱掃會持續吐輸出，固定逾時會誤殺） | `src/components/TerminalView.tsx`、[[project-agent-command-deadlock]] |
| 專用 SQLite 的既有模式：`src-tauri/src/db/<name>.rs` 各自持有一個檔案（`dirs::data_dir().join("AITERM").join("<name>.db")`），`SqliteConnectOptions::new().filename(..).create_if_missing(true).busy_timeout(5s)`，schema 用 `init()` 裡一串 `CREATE TABLE IF NOT EXISTS`，在 `lib.rs` 用 `.manage(...)` 掛上 | `src-tauri/src/db/knowledge_base.rs`、`loop_sessions.rs`、`usage/store.rs` |
| 首頁入口是**側邊欄固定按鈕**（`homeActive` 狀態，非分頁類型），內容區用 `{homeActive && <HomeView .../>}` 切換，終端機分頁維持掛載但隱藏 | `src/components/TerminalApp.tsx:69,613`、`src/components/HomeView`、[[project-entry-page-daily-work]] |

**結論**：派工 = spawn PTY session + emit 既有事件 + `write` 提示詞；監看 = 重用
`wait_for_idle` 的 bell/marker 邏輯 + OSC133 exit code + 120 秒安靜偵測。新東西只有
「一個 `tasks.db`」、「一個長駐排程器」、「一個看板視圖 + 側邊欄按鈕」。

## 範圍

**含：**

- 側邊欄新增固定按鈕「工作看板」，內容區顯示佔滿寬度的四欄看板
- 新 Rust 模組 `src-tauri/src/tasks/`：`tasks.db`（比照 `db/knowledge_base.rs` 模式）+ 長駐排程器
- `tasks_*` Tauri 指令 + `tasks://updated` 事件
- 卡片：標題、內文、專案資料夾、附件（拖檔案進來）
- 每張卡片一個 `parallel_ok` 開關 + 全域同時執行上限 N（設定，預設 2）
- 派工：在卡片專案資料夾 spawn session、adopt 成分頁、跑 `claude`、寫入組好的提示詞
- 完成偵測：idle → 自動移到「已完成」`success`；非零 exit / 卡住 120 秒 / 手動停止 → 「已完成」標 `失敗` / `已中斷`
- 逆紀錄：完成時把分頁捲動內容快照存檔，卡片可「查看逆紀錄」「開啟分頁」
- 「已完成」卡片「重新派工」（複製回「計畫中」）

**不含：**

- 看板驅動的多輪對話——派工是一次性提示詞；要對話就跳進那個分頁手動打字
- `claude` 以外 agent 的專屬整合——CLI 指令是個設定字串（預設 `claude`），Codex 等照理能跑但不特別做成功能、不寫死也不測其他家
- app 執行中重啟後讓 `running` 任務存活——`PtyManager` 的 session 隨 app process 一起死（[[project-agent-command-deadlock]] 提過這個既有壽命邊界），重啟後仍是 `running` 的卡片一律標成「已中斷」
- 排程／週期性任務、卡片相依、指派人、多人協作
- 分頁裡實際跑的指令做二次確認 UI——比照 `send_input` 既有風險等級（允許任意內容），不做同步確認流程
- 重構 `spawn()`/`spawn_with_id()` 既有的重複程式碼

## 架構

### 為什麼排程器放在 Rust 後端、不放前端

看板視圖會被關掉（使用者切回終端機），app 也可能整個失焦到背景，但工作要照跑、
下一張卡要照樣被拉起來執行。如果排程迴圈活在 React 元件裡，視圖一 unmount 迴圈就
停了。所以排程器是 app 啟動時就拉起的長駐 `tokio` task，狀態的唯一真實來源是
`tasks.db`，前端只是渲染 + 發指令。這跟協調機制刻意把 idle 偵測留在 Rust 讀取
迴圈、不跨行程問前端是同一個理由。

### 模組佈局

```
src-tauri/src/tasks/
  mod.rs        TasksManager：持有 SqlitePool，init() 建 schema（比照 db/knowledge_base.rs）
  store.rs      CRUD + 欄位間移動 + sort_order 重排（純 DB 操作，可單元測試）
  scheduler.rs  長駐 tokio task：佇列變動 / 任務完成時挑下一張卡起跑；併發與 solo 規則
  dispatch.rs   把一張卡變成一次派工：複製附件、組提示詞、spawn session、emit 事件、寫入
  monitor.rs    對一個 running 卡的 session 等 idle / 逾時 / 非零 exit，回報結果給 scheduler
```

`commands/tasks.rs` 放 `#[tauri::command]` 入口，delegate 到上面各模組（比照
`db_ops.rs`/`kb_ops.rs` 的邏輯分離）。`lib.rs` 用 `.manage(TasksManager::new().await)`
掛上，並在 setup 裡 `scheduler::spawn(app_handle, tasks_manager, pty_manager)`。

### `PtyManager` 的共用

`spawn_tab` 已經需要 `PtyManager` 跟主 app 共用同一份（協調設計已把它改成 `Arc`
共用、12 個 `State<'_, PtyManager>` 呼叫點改成 `State<'_, Arc<PtyManager>>`）。排程器
的 `dispatch.rs` 直接拿這個 `Arc<PtyManager>` 呼叫 `create_with_app` / `write` /
`bell_count` / `marker_count` / `get_recent_output` / `close`，**不透過 MCP HTTP**——
直接呼叫 Rust 函式。若 `coordination_ops.rs` 裡的 `spawn_tab` / `wait_for_idle`
函式主體可重用（例如抽成 `pub(crate)` 的純函式版本），就重用；否則在 `dispatch.rs`
/ `monitor.rs` 照同樣的步驟重寫一份薄的（步驟本身很短：spawn → emit → write；
輪詢 `bell_count`/`marker_count` 比對 baseline）。實作時以「重用既有函式」為優先，
只有在耦合了 `CoordinationRegistry` 之類無關狀態時才另寫。

### 資料模型 — `tasks.db`

`SqliteConnectOptions::new().filename(app_data_dir.join("tasks.db")).create_if_missing(true).busy_timeout(Duration::from_secs(5))`，
`app_data_dir = dirs::data_dir().unwrap_or(".").join("AITERM")`（跟既有模組完全一致）。

```sql
CREATE TABLE IF NOT EXISTS tasks (
    id             TEXT PRIMARY KEY NOT NULL,
    title          TEXT NOT NULL,
    body           TEXT NOT NULL DEFAULT '',
    project_dir    TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'planning',  -- planning | queued | running | done
    parallel_ok    INTEGER NOT NULL DEFAULT 1,        -- 1 = 可並行, 0 = 必須單獨執行
    sort_order     REAL NOT NULL DEFAULT 0,           -- 同欄內拖曳排序（越小越前）
    outcome        TEXT,                              -- NULL 直到 done；success | failed | cancelled
    tab_id         TEXT,                              -- 派工後的 session id
    transcript_path TEXT,                             -- 完成時的捲動內容快照檔
    error_message  TEXT,                              -- outcome != success 時的簡短原因
    created_at     DATETIME DEFAULT CURRENT_TIMESTAMP,
    dispatched_at  INTEGER,
    finished_at    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status, sort_order);

CREATE TABLE IF NOT EXISTS task_attachments (
    id          TEXT PRIMARY KEY NOT NULL,
    task_id     TEXT NOT NULL,
    filename    TEXT NOT NULL,
    stored_path TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_attachments_task ON task_attachments(task_id);
```

附件檔案放在 `app_data_dir.join("tasks").join(<task-id>).join("attachments")`。
逆紀錄快照放在 `app_data_dir.join("tasks").join(<task-id>).join("transcript.txt")`。
`sort_order` 用 REAL：拖到兩張卡中間 = 取兩者平均，不必整欄重排。

### `tasks_*` 指令與事件

| 指令 | 參數 | 說明 |
|---|---|---|
| `tasks_list` | — | 回傳全部卡片（含附件），前端自行分欄 |
| `tasks_create` | `title, body, project_dir, parallel_ok` | 建一張 `planning` 卡，`sort_order` 放該欄尾 |
| `tasks_update` | `id, {title?, body?, project_dir?, parallel_ok?}` | 只在 `planning` 允許改內容；其他欄只允許改 `parallel_ok`（下次執行才生效） |
| `tasks_move` | `id, to_status, new_sort_order` | 欄位間移動 + 排序。`planning → queued` 會 `notify` 排程器。非法轉移（例如 `done → running`）回錯誤 |
| `tasks_stop` | `id` | 對 `running` 卡送 Ctrl+C（`\x03`）、強制結案，卡片落 `done` / `cancelled` |
| `tasks_delete` | `id, close_tab: bool` | 刪卡 + 附件/逆紀錄檔；`close_tab` 為真且有 `tab_id` 時一併 `PtyManager::close` |
| `tasks_add_attachment` | `id, filename, bytes` | 寫進該卡 attachments 目錄，插一筆 `task_attachments`。僅 `planning` |
| `tasks_remove_attachment` | `attachment_id` | 刪檔 + 刪列。僅 `planning` |
| `tasks_open_tab` | `id` | 要前端切到該卡的 `tab_id` 分頁（若還在） |
| `tasks_read_transcript` | `id` | 回傳 `transcript_path` 檔案內容 |

事件 `tasks://updated`：排程器 / 指令在任何狀態變動後 emit（無 payload 或帶
變動的卡 id 陣列）。前端收到就重新 `tasks_list`。看板視圖不自己維護樂觀狀態，
一切以後端為準。

### 排程器規則（`scheduler.rs`）

排程器持有一個 `tokio::sync::Notify`（或 `mpsc`），下列事件會敲它：
`tasks_move` 把卡移進 `queued`、`monitor` 回報某卡完成、app 啟動後的第一次掃描。

每次被敲醒：

1. 讀 `running` 卡集合。若有任一張是 `parallel_ok = 0`（solo 執行中）→ 這輪不派任何新卡，等它結束。
2. 否則計算可用名額 = `N - running.len()`（`N` = 設定的全域上限，預設 2）。名額 ≤ 0 → 結束。
3. 依 `sort_order` 取最舊的 `queued` 卡當候選：
   - 候選是 `parallel_ok = 0`（solo）→ 只有在 `running` 為空時才起跑，否則**停在這裡**（不跳過它去派後面可並行的卡——維持嚴格優先序，solo 卡不會被無限插隊餓死）。
   - 候選是 `parallel_ok = 1` → 直接起跑，扣一個名額，回到步驟 3 看下一張。
4. 起跑一張卡 = 呼叫 `dispatch::run(app, pty, task)`，把卡 `status` 設 `running`、記 `tab_id` / `dispatched_at`，emit `tasks://updated`，並 spawn 一個 `monitor::watch(...)` 的 tokio task。

app 啟動時排程器先跑一次「復原掃描」：任何 `status = 'running'` 的卡（上次 app
沒好好關）→ 設 `status = 'done'`、`outcome = 'cancelled'`、
`error_message = "app 重啟，工作已中斷"`，再進正常排程迴圈。

### 派工（`dispatch.rs`）

給定一張 `queued` 卡：

1. 把 `task_attachments` 的檔案（已經在該卡 attachments 目錄裡）路徑收集起來。
2. 組提示詞（單一字串）：
   ```
   {body}

   （相關附件：{path1}、{path2}、…）
   ```
   附件那行只在有附件時加。**不**在這裡自己拼 done-marker——沿用 `send_input` 既有的
   `request_done_marker` 路徑產生固定措辭，避免本地 echo 自我觸發（協調設計踩過且修過的坑）。
3. `pty_manager.create_with_app(app.clone(), PtySize { rows: 24, cols: 80, .. }, Some(project_dir), None)` → `tab_id`。
4. `app.emit("mcp-coordination-tab-spawned", { session_id: tab_id, command: Some("claude") })`——重用既有事件，前端 adopt 成看得見的分頁。
5. 等分頁 adopt + `claude` 冷啟動。**不寫死延遲**：`dispatch` 送出 `claude\r` 後，
   輪詢等 session 輸出安靜一小段（沿用既有輸出時間戳機制的後端版；協調設計實測
   `claude` 冷啟動約 3.7 秒起跳），或退回一個上限逾時（例如 30 秒）。
6. 用等同 `send_input(text, request_done_marker: true)` 的路徑寫入提示詞：先寫任務文字 + `\r`，
   等目標端因任務文字觸發新 bell（上限 `DONE_MARKER_WAIT_SECONDS = 15`）後再分開寫入
   done-marker 指示文字；逾時就只送任務文字。
7. 回傳 `tab_id` 給排程器。

### 監看（`monitor.rs`）

對一張 `running` 卡的 `tab_id`，一個 tokio task 每 250ms 輪詢，直到命中其一：

| 命中 | outcome | 說明 |
|---|---|---|
| `marker_count > baseline` 或 `bell_count > baseline` | `success` | idle 訊號，比照 `wait_for_idle` |
| OSC133 掃到非零 exit code | `failed` | 重用 `cd_parser::find_exit_codes` 的結果（need：`PtySession` 曝露「最近一次 exit code」，若尚未曝露則加一個 `AtomicI64` / `Mutex<Option<i32>>`，比照 `bell_count` 的做法） |
| session 輸出安靜 ≥ 120 秒（`STUCK_IDLE_MS`）且期間完全沒有新輸出 | `failed`，`error_message = "疑似卡住（120 秒無輸出）"` | 沿用既有「安靜多久」而非「跑多久」的判準 |
| 收到 `tasks_stop`（透過一個 per-task 的 cancel channel） | `cancelled` | `tasks_stop` 先送 `\x03`，monitor 收到 cancel 後結案 |

命中後：`pty_manager.get_recent_output(tab_id, <大一點的上限>)`（或既有的完整
scrollback 取得路徑）寫進 `transcript.txt`；更新卡片 `status = 'done'`、`outcome`、
`error_message`、`finished_at`、`transcript_path`；emit `tasks://updated`；敲排程器
的 `Notify` 讓它挑下一張。**分頁不自動關**（比照協調），使用者可自己關或用
`tasks_delete(close_tab: true)`。

> 已知模糊性（沿用協調設計的既有共識）：bell / marker 都不是強制訊號。若 `claude`
> 這輪就是不敲 bell、也沒照指示印 marker（協調實測發生過），這張卡會靠「安靜 120 秒」
> 落到 `failed`「疑似卡住」，即使它其實正常跑完了。v1 接受這個代價；使用者點開逆紀錄
> 就能看到實際結果並「重新派工」或手動處理。

### 前端

- `TaskBoardView`（比照 `HomeView` 的側邊欄按鈕切換）：`TerminalApp.tsx` 加一個
  `boardActive` 狀態（跟 `homeActive` 併排），側邊欄多一顆按鈕，內容區
  `{boardActive && <TaskBoardView />}`，終端機分頁維持掛載隱藏。
- `TaskBoardView` 掛載時 `tasks_list` + `listen("tasks://updated")` 重抓（用
  `mountedRef` 防 unmount 後 setState，比照 `useAiChat.ts` 的既有做法）。
- 四欄用 CSS grid；卡片拖曳用專案既有的拖曳做法（若無則用原生 HTML5 drag events，
  放下時算 `new_sort_order` = 目標位置前後卡的 `sort_order` 平均）。
- 卡片編輯彈窗：標題 / 內文 / 專案資料夾（`@tauri-apps/plugin-dialog` 的 `open({ directory: true })`，
  預設帶上次用過的目錄——存在 `localStorage` 的 `aiterm_last_task_dir`）/ 附件拖放區 / `parallel_ok` 開關。
- 「執行中」卡片顯示：spinner、`開啟分頁`、`停止`。
- 「已完成」卡片顯示：`success`/`failed`/`cancelled` badge、`查看逆紀錄`（開一個唯讀彈窗）、`開啟分頁`（若還在）、`重新派工`。
- i18n：新字串加進 `src/lib/i18n.ts` 的 `en` / `zh-TW` 兩份。

### 設定

MCP 工具伺服器設定頁（或一般設定頁）新增「工作看板」區塊：

- **全域同時執行上限 N**（數字，預設 2，下限 1）
- **Claude Code CLI 指令**（文字，預設 `claude`）——派工時 spawn 的啟動指令

存在既有 config store（JSON），比照 `mcp_tool_server` 設定的讀寫方式。

## 已知限制

- idle 偵測沿用 bell / marker，兩者都非絕對可靠（見上「監看」的已知模糊性）。不聽話或非 MCP-aware 的情境會落到「疑似卡住」。
- `running` 任務不跨 app 重啟存活——重啟一律標「已中斷」。
- 派工是一次性提示詞，看板不做多輪對話。
- 併發上限是全域單一數字，不分專案 / 不依機器資源自動調整。
- `spawn_tab` 那條路徑「寫入前不等 shell 完全就緒」的既有風險類別仍在——`dispatch` 用「等輸出安靜 + 上限逾時」緩解，但不是保證。
- 附件是「複製進任務目錄 + 提示詞附路徑」，靠 `claude` 自己去讀；不做內容內嵌、不驗證 `claude` 真的讀了。

## 測試

- **Rust 單元（`store.rs`）**：CRUD；`tasks_move` 的合法／非法狀態轉移；`sort_order` 取中點插入；刪卡連帶刪附件列。
- **Rust 單元（`scheduler.rs`）**：
  - N 上限：3 張可並行卡、N=2 → 只有 2 張進 `running`。
  - solo 卡等到獨佔：1 張 solo 卡排在佇列最前、已有 1 張在跑 → solo 卡不起跑，直到那張跑完。
  - solo 卡擋住其他：1 張 solo 卡在 `running` → 後面可並行卡不起跑。
  - 嚴格優先序：佇列 `[solo, parallel]`、有東西在跑 → 停在 solo，不跳過它去派 parallel。
  - 最舊優先：依 `sort_order` 取。
  - 復原掃描：啟動時 `running` 卡 → `status = 'done'` / `outcome = 'cancelled'` / `error_message` 正確。
- **Rust 單元（`monitor.rs`）**：完成狀態對應——marker/bell → `success`；非零 exit → `failed`；120 秒安靜 → `failed`「疑似卡住」（測試用可注入的短門檻）；cancel → `cancelled`。
- **Rust 整合**（比照 `tests/mcp_tool_server.rs` 手法）：`tasks_create` → `tasks_move` 進 `queued` → 排程器 spawn 一個 session 跑假腳本（印幾行輸出、再印 `<<AITERM_DONE:{tab_id}>>`）→ 卡片結束為 `done` / `success`、`transcript_path` 檔案內容含預期輸出。不需要真的裝 `claude`。
- **Rust 整合**：假腳本印完輸出後 `exit 1` → 卡片 `failed`、`error_message` 有值。
- **前端**（用既有大元件測試掛載法，mock Tauri `invoke` / `listen`）：
  - 四欄從 `tasks_list` 結果正確分欄渲染。
  - 拖卡片從「計畫中」到「待執行」→ 呼叫 `tasks_move` 帶 `to_status: "queued"` 與算出的 `new_sort_order`。
  - 「執行中」卡片按「停止」→ 呼叫 `tasks_stop`。
  - 「計畫中」卡片新增／移除附件 → 呼叫 `tasks_add_attachment` / `tasks_remove_attachment`；非 `planning` 卡片看不到附件編輯 UI。
  - 收到 `tasks://updated` 事件 → 重新 `tasks_list`（用 fake timers + `act()` 包非同步，比照既有做法）。
  - i18n：`zh-TW` / `en` 兩份都有新 key（比照既有 i18n 測試）。
- **手動驗證**：真的建一張卡、填標題內文、選一個真 git repo 當專案資料夾、加一個附件、拖到「待執行」；確認自動開出一個跑 `claude` 的分頁、提示詞（含附件路徑）被送進去、`claude` 跑完後卡片落到「已完成」`success`、逆紀錄可看；再測一次 N=1 時第二張卡在「待執行」等、第一張完成後才起跑；再測 solo 卡與並行卡混在佇列裡的順序；再測「停止」與「重新派工」。

## 相關

[[project-agent-command-deadlock]]、[[project-entry-page-daily-work]]、[[project-claude-code-bridge]]、
`docs/superpowers/specs/2026-08-20-agent-coordination-design.md`、
`docs/superpowers/specs/2026-08-21-coordination-done-marker-design.md`、
`docs/superpowers/specs/2026-08-16-home-entry-design.md`

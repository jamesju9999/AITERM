# Agent 對 Agent 協調 — 設計

日期：2026-08-20
狀態：待使用者複審

## 問題

使用者想在 AITerm 裡跑多個獨立的 coding agent（例如一個負責實作、一個負責寫測試）互相協作，而不是單一 agent 自己跑一條自主迴圈。概念上取材自 [herdr](https://github.com/herdrdev/herdr)（一個 Rust 終端機 multiplexer daemon，標語「the runtime your coding agents live on」）——但範圍刻意縮小，只做「agent 對 agent 協調」這一塊，不碰 herdr 的常駐背景 daemon、session 跨 app 重啟存活、跨網路重新連接這些概念（那部分是完全不同量級的架構改動；AITerm 目前 `PtyManager` 的 session 活在 Tauri app process 自己的記憶體裡，app 關掉 session 就死，這次不打算改）。

## 現況調查（設計的事實基礎）

以下都是實際讀過程式碼／查過上游文件確認的，不是推測：

| 事實 | 位置 |
|---|---|
| `PtyManager` 已有完整的低階 API：`create_with_app`（spawn）、`write`、`close`、`get_recent_output`（回傳最近 ANSI-stripped 輸出），純 Rust 後端、不依賴前端 React state | `src-tauri/src/pty/manager.rs` |
| `PtyManager` 目前用 `.manage(PtyManager::new())` 裸管理（不是 `Arc`），有 12 個 `State<'_, PtyManager>` 呼叫點分佈在 3 個檔案 | `src-tauri/src/pty/commands.rs`, `src-tauri/src/commands/design.rs`, `src-tauri/src/commands/ai.rs` |
| Rust 後端已經有 OSC133 解析器，每個 PTY 輸出 chunk 即時掃描「指令結束＋exit code」，目前只拿來輔助 cwd 追蹤，沒有存成通用的「這個分頁是不是在跑東西」狀態 | `src-tauri/src/pty/cd_parser.rs::find_exit_codes`，呼叫點在 `src-tauri/src/pty/session.rs:190` |
| AITerm 已經有機制引導使用者把 Claude Code 設成用 terminal bell（`\x07`）發通知（`preferredNotifChannel: terminal_bell`），寫在 `~/.claude/settings.json` | `src-tauri/src/commands/claude_notif.rs` |
| **Codex CLI（OpenAI）也有一模一樣的機制**：`notification_method` 預設 `auto`，AITerm 的 xterm.js 終端機不會被它偵測成 Ghostty/iTerm2/Kitty/WarpTerminal/WezTerm 這幾款已知支援 OSC9 的終端，所以會自動退回發純 BEL（`\x07`）——跟 Claude Code 送出來的位元組完全相同。查證來源：`codex-rs/tui/src/notifications/{bel.rs,osc9.rs,mod.rs}`、`codex-rs/config/src/types.rs` 的 `NotificationMethod`/`Notifications` 定義 | `github.com/openai/codex` |
| Codex CLI 的 `notification_condition` 預設 `unfocused`——**只有終端機沒有 focus 時才發通知**。Claude Code 是否有類似限制尚未查證 | `codex-rs/config/src/types.rs:633-639` |
| herdr 官方文件對狀態的定義：`blocked`＝需要輸入/核准/決定、`working`＝執行中、`done`＝跑完但沒看過、`idle`＝跑完或等待且已看過。偵測方式是「辨識前景行程＋畫面內容規則＋可選的 agent 主動回報」的混合做法，不是單一通用機制 | `herdr.dev/docs/concepts/` |
| AITerm 已有 MCP tool server（`src-tauri/src/mcp_server/`，用 `rmcp` crate），DB／知識庫兩組工具已經照 `AiTermTools` struct + `#[tool_router]`／`#[tool]` 的模式、業務邏輯分離在 `db_ops.rs`/`kb_ops.rs` 的方式實作完成 | `src-tauri/src/mcp_server/{tools.rs,db_ops.rs,kb_ops.rs}` |

**結論**：這不是要重新發明一套 agent 協調協定，而是在既有 MCP tool server 上加第三組工具，重用已經存在的三塊底子（`PtyManager` 低階 API、OSC133 解析模式、bell 通知管道），只是這三塊底子目前彼此沒有串起來。

## 範圍

**含：**

- 在 `mcp_server/` 新增第三組工具群組（`coordination_ops.rs`），掛在既有 `AiTermTools` 上
- 4 個工具：`spawn_tab`、`send_input`、`get_tab_status`、`wait_for_idle`
- `PtyManager` 改成 `Arc` 共用（MCP server 與主 app 的 Tauri commands 用同一份，spawn 出來的分頁使用者在 AITerm 視窗裡看得到）
- `PtySession` 讀取迴圈新增 bell 位元組（`\x07`）偵測，作為「idle」訊號
- 一份「由 `spawn_tab` 開出來的分頁」名單，`send_input`/`get_tab_status`/`wait_for_idle` 只認名單內的 tab_id
- 獨立的設定頁開關（跟 DB/知識庫工具分開），預設關閉

**不含：**

- herdr 的常駐背景 daemon、session 跨 app 重啟存活、跨網路重新連接
- `close_tab`（分頁清理）——先手動關閉，之後有需要再加
- 針對特定 agent（Claude Code / Codex）的專屬整合邏輯——偵測機制是「bell 位元組」層級，agent 無關，不寫死任何一家
- 分頁裡跑的指令做二次確認 UI（風險等級比照 `execute_query` 允許任意 SQL，這次不做同步確認流程）
- OSC133 層級的「shell 指令跑完」偵測（一次性任務型的等待，例如「等某分頁跑完 `npm test`」）——v1 只做 bell 訊號；OSC133 那條路徑既有解析器已經可重用，但這次的動機是多輪對話式協作，不是一次性任務，先不做

## 架構

### 為什麼 `PtyManager` 一定要改成 `Arc`，不能像 DB/知識庫那樣自己開一份獨立的

DB/知識庫工具刻意讓 MCP server 建立自己獨立的 `DbManager`/`Db2SidecarState`/知識庫 `SqlitePool`，理由是「同一份磁碟資料、各自獨立的連線狀態」可以接受。這次不行：協調工具的整個價值就是操作使用者在 AITerm 視窗裡看得到的真實分頁，如果 MCP server 自己 spawn 一份使用者看不到的 PTY，整個功能沒有意義。因此 `PtyManager` 必須改成 `Arc` 包裝、MCP server 與主 app 共用同一份——這牽涉到把 12 個 `State<'_, PtyManager>` 呼叫點（`pty/commands.rs`、`commands/design.rs`、`commands/ai.rs`）改成 `State<'_, Arc<PtyManager>>`，靠 deref coercion 應該不需要動呼叫端的邏輯，只是簽名調整。

### 狀態偵測留在 Rust 後端，不丟回前端

考慮過另一個做法：MCP 工具呼叫時透過 Tauri event 問前端「這個分頁狀態如何」，重用前端既有的 `terminalAttention.ts` 邏輯。放棄的理由：MCP 的 HTTP 請求要跨行程等前端事件迴圈回應，複雜度與脆弱度都更高（前端卡住，MCP 呼叫就跟著卡住）。改成在 `PtySession` 的讀取迴圈裡直接偵測 bell 位元組，跟現有 `find_exit_codes` 掃 OSC133 的模式一致，不新開一條路徑，也不跨行程往返。

### 模組佈局

```
src-tauri/src/mcp_server/
  coordination_ops.rs   協調工具的業務邏輯：spawn_tab/send_input/get_tab_status/wait_for_idle
  tools.rs               新增 4 個 #[tool] 方法，delegate 到 coordination_ops.rs（既有檔案，擴充不新建）
```

`PtySession`（`src-tauri/src/pty/session.rs`）新增一個 idle 旗標（例如 `AtomicBool` 或 `Mutex<Instant>` 記最後一次看到 bell 的時間），在讀取迴圈裡跟 `find_exit_codes` 同一個位置多掃一次原始 chunk 找 `\x07`。

`McpToolServerState`（或掛在 `AiTermTools` 上的一個新欄位）新增一個「MCP 自己 spawn 出來的 tab_id 名單」（`HashSet<String>`，記憶體內即可，不需要跨 server 重啟存活）。

### 工具清單

| 工具 | 參數 | 說明 |
|---|---|---|
| `spawn_tab` | `cwd?: string`, `command?: string` | 開一個新分頁（使用者在 UI 上看得到）。若給了 `command`，分頁開好後自動 `send_input` 那段文字（例如 `"claude"` 或 `"codex"`）。回傳 `tab_id`，並記進「MCP 自己開的分頁」名單 |
| `send_input` | `tab_id: string`, `text: string` | 對分頁送一段文字＋Enter，就像使用者打字（重用 `PtyManager::write`）。只認名單裡的 tab_id，回傳錯誤若目標不在名單內（絕不能碰使用者手動開的分頁） |
| `get_tab_status` | `tab_id: string` | 回傳 `{ idle: bool, recent_output: string }`——`idle` 來自 bell 旗標，`recent_output` 重用既有的 `PtyManager::get_recent_output` |
| `wait_for_idle` | `tab_id: string`, `timeout_seconds?: number`（預設 300，上限 1800，超過上限視為 1800） | 後端非同步等到 bell 訊號或逾時，回傳跟 `get_tab_status` 一樣的結構，外加 `timed_out: bool` |

### 安全設計

- 協調工具是獨立的設定頁開關（跟 DB/知識庫工具分開），**預設關閉**
- `send_input` 硬性限制只能對「由 `spawn_tab` 開出來」的分頁下指令；使用者自己手動開的分頁不在名單內，MCP 永遠碰不到
- 分頁裡實際跑的指令內容仍是任意的——跟既有 `execute_query` 允許任意 SQL 同一個風險等級，這次同樣不做同步確認 UI（理由：MCP 工具呼叫是同步 HTTP 請求—回應，沒有現成管道跳出視窗等使用者按確認；要做的話是完全不同量級的工作）

### 已知限制

- **這是「bell 位元組」偵測，不是特定 agent 專屬**。Claude Code 與 Codex CLI 都預設（或容易設成）在無法辨識終端機支援更豐富通知格式時退回發純 BEL，同一套後端邏輯直接通用，不需要為個別 agent 寫專屬規則
- **Focus 限制待實作時驗證**：Codex CLI 的 `notification_condition` 預設 `unfocused`——只有終端機沒有 focus 時才發通知。如果協作分頁剛好是使用者正在看的那個，可能收不到 bell。Claude Code 是否有類似限制尚未查證，這是實作階段要實測的風險，不是現在能保證的事
- bell 位元組本身不是絕對可靠的信號（理論上其他情境也可能觸發），這是重用既有通知管道的固有代價，v1 不特別處理
- 不含分頁清理（`close_tab`）

## 測試

- Rust 單元測試：`PtySession` 的 bell 偵測（送 `\x07` 進讀取迴圈，確認 idle 旗標翻轉；確認一般輸出不會誤觸發）
- `coordination_ops.rs` 的單元測試：`send_input` 對不在名單內的 tab_id 回錯誤而非靜默失敗；`spawn_tab` 正確把新 tab_id 記進名單
- 整合測試：仿照既有 `tests/mcp_tool_server.rs` 的 `tower::ServiceExt::oneshot` 手法，跑一次 `spawn_tab` → `send_input`（送一個會印出東西再印 bell 的假指令，測試環境不需要真的裝 Claude Code/Codex）→ `wait_for_idle` → 確認回傳的 `recent_output` 含預期內容
- 手動驗證：真的 spawn 一個跑 Claude Code 的分頁，透過另一個分頁的 Claude Code session 呼叫這組工具，確認協作流程（下指令→等待→拿到回應→繼續對話）真的通

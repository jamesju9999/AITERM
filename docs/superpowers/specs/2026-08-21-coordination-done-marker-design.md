# Agent 協調工具 — 選用完成加速訊號設計

日期：2026-08-21
狀態：待使用者複審

## 問題

`docs/superpowers/specs/2026-08-20-agent-coordination-design.md` 已實作的 `wait_for_idle` 只靠 terminal bell（`\x07`）判斷目標分頁是否 idle，250ms 輪詢一次。實測發現：真正拖慢協調流程的是每個目標分頁的冷啟動成本（shell 啟動 + `claude`/`codex` CLI 冷啟動 + hooks/LSP/plugin sync/CLAUDE.md 掃描，實測約 3.7 秒起跳）與「bell 是全有全無訊號，中途完全沒有進度可見」這兩點，而不是 250ms 這個輪詢間隔本身。

即便如此，如果目標分頁裡跑的是 MCP-aware 的 agent（例如另一個 Claude Code），讓它在完成當下**主動**留下一個訊號，理論上還是能比等 bell 更快浮現 idle 狀態一點點（bell 依賴 `preferredNotifChannel: terminal_bell` 設定正確、依賴該次任務確實觸發終端機通知邏輯；主動印出的標記則不依賴那層設定）。這份設計要在不破壞既有 bell 機制、不新增 MCP 工具介面、不引入跨行程等待對方回應的前提下，加一個**選用**的加速路徑。

2026-08-20 那份設計文件已經明確評估並放棄過「MCP 工具呼叫時跨行程問前端」這類依賴對方回應的做法（理由：對方卡住，呼叫端就跟著卡住）。這次的加速訊號必須避開同一個陷阱——它不能變成新的單點故障，沒觸發就必須完全等效於這個功能不存在，照舊靠 bell。

## 現況調查（設計的事實基礎）

| 事實 | 位置 |
|---|---|
| `PtySession` 用 `bell_count: Arc<AtomicU64>`，讀取迴圈裡對每個新進 chunk 呼叫 `cd_parser::contains_bare_bell(&chunk)`，命中就 `fetch_add(1)`；對外只曝露單調遞增的 `pub fn bell_count(&self) -> u64` | `src-tauri/src/pty/session.rs:71,276-341,632-634` |
| 這段讀取迴圈邏輯在 `spawn()`（第 215 行）與 `spawn_with_id()`（第 347 行）兩個建構進入點各自重複一份（`bell_count`/`bell_count_for_thread` 那段程式碼出現兩次） | `src-tauri/src/pty/session.rs:215,276-341,347,411-476` |
| 讀取迴圈已有一個持續累積的 `output_ring`（`VecDeque<u8>`，容量 8KB）供 `get_recent_output` 使用，但 bell 偵測本身是**對每個新進 chunk 各自獨立**呼叫 `contains_bare_bell(&chunk)`，不看 chunk 之間的邊界銜接——因為 bell 是單一位元組，不會被切在兩個 chunk 之間；多位元組的字串樣式沒有這個保證 | `src-tauri/src/pty/session.rs:284-292,310-315` |
| `CoordinationRegistry`（`tabs: Mutex<HashMap<String, u64>>`）只記單一 bell baseline；`record_baseline`/`baseline`/`is_known` 三個方法都是單一 `u64` 語意 | `src-tauri/src/mcp_server/coordination_ops.rs:40-61` |
| `status_for` 判斷 `idle = current > baseline`，`current` 來自 `pty_manager.bell_count(tab_id)` | `src-tauri/src/mcp_server/coordination_ops.rs:144-158` |
| `spawn_tab` 呼叫 `registry.record_baseline(&tab_id, 0)`；`send_input` 每次呼叫後把 baseline 重設成當下的 `bell_count()`，讓「這次送出後才出現的 bell」才算數 | `src-tauri/src/mcp_server/coordination_ops.rs:100,138-139` |
| `wait_for_idle` 輪詢間隔 `POLL_INTERVAL_MS = 250`，逾時預設 `DEFAULT_WAIT_SECONDS = 300`，上限 `MAX_WAIT_SECONDS = 1800` | `src-tauri/src/mcp_server/coordination_ops.rs:22-24` |
| `SpawnTabArgs`/`SendInputArgs` 是 `#[tool]` 方法的參數結構，doc comment 上的文字會直接變成 MCP 工具給呼叫端 LLM 看的 schema 說明（`cwd` 欄位已經示範過這種「引導呼叫端怎麼用」的寫法） | `src-tauri/src/mcp_server/tools.rs:85-118` |

**結論**：加速訊號的偵測邏輯，應該完全比照 `bell_count` 現有的「讀取迴圈裡掃 chunk → 遞增 `AtomicU64` → 曝露單調計數器」模式，讓 `CoordinationRegistry` 的 baseline 比較邏輯自然地從「一個訊號」擴充成「兩個訊號，任一個先超過 baseline 就算 idle」，不需要新的跨行程或跨 session 通道。

## 範圍

**含：**

- `PtySession` 新增 `marker_count: Arc<AtomicU64>` 與 `marker_tail: Mutex<Vec<u8>>`，讀取迴圈在掃 `contains_bare_bell` 的同一個位置，額外用「上一輪尾巴＋這次 chunk」掃描自己 `id` 專屬的完成標記字串（見下方「標記格式」與「跨 chunk 邊界的偵測正確性」），命中就 `fetch_add(1)`；曝露 `pub fn marker_count(&self) -> u64`，`spawn()`/`spawn_with_id()` 兩處都要加
- `CoordinationRegistry` 的 baseline 從單一 `u64` 改成 `(bell: u64, marker: u64)` 一對；`record_baseline`/`baseline` 對應改成記兩個值
- `status_for` 的 idle 判斷改成 `bell_count > bell_baseline || marker_count > marker_baseline`
- `SpawnTabArgs`/`SendInputArgs` 各加一個 `request_done_marker: bool`（`#[serde(default)]`，預設 `false`）
- 設為 `true` 時，`spawn_tab`/`send_input` 把要寫進 PTY 的文字，在原文字後面加上一段固定措辭、含 `tab_id` 的提示（見下方「指示文字」），一次寫入，呼叫端不必自己組措辭
- `TabStatus`/`WaitResult` 加一個 `signal: Option<&'static str>` 欄位（`"bell"` 或 `"marker"`），標出這次 idle 是哪個訊號先觸發的

**不含：**

- 新的 MCP 工具——完全靠既有 `spawn_tab`/`send_input` 的新參數承載
- 改變 bell 機制本身的行為、預設值或優先順序
- 針對特定 agent 的專屬邏輯——標記格式本身是 AITerm 自訂的固定字串，任何會照著提示文字印出來的 agent 都適用，不限 Claude Code
- 分頁清理、逾時數值調整、標記失敗時的重試或錯誤提示——`request_done_marker: true` 但 worker 從未印出標記，就是純粹落回等 bell／逾時，跟功能沒開一樣
- 重構 `spawn()`/`spawn_with_id()` 之間既有的重複程式碼——這次新增偵測邏輯會維持現狀，同步複製到兩處，不在本次範圍內合併

## 架構

### 標記格式

`<<AITERM_DONE:{tab_id}>>`，要求 worker 把它單獨印在一行。`tab_id` 是 `spawn_tab` 產生的隨機 UUID（見 `PtyManager::create_with_app` 用 `uuid::Uuid::new_v4()`，字串固定 36 字元），所以整個標記是**固定長度**：`<<AITERM_DONE:`（14 字元）+ UUID（36 字元）+ `>>`（2 字元）＝ 52 字元。天然跟其他分頁的標記不會混淆，也讓「worker 剛好在正常對話中提到這串文字」的誤判機率趨近於零。偵測範圍限定在該分頁自己的 `PtySession` 讀取迴圈裡找自己的 `id`——不需要額外做 tab_id 比對，因為每個 `PtySession` 只看得到自己的輸出流。

### 跨 chunk 邊界的偵測正確性

自我複審時發現的正確性缺口：標記是 52 字元的固定長度字串，而 PTY 讀取迴圈是任意大小的 `buf[..n]` chunk（見上表）。如果照搬 bell 那樣「對每個新進 chunk 各自獨立掃描」，標記剛好被切在兩個 chunk 交界處就會被漏掉——這是 bell（單一位元組，不可能被切開）完全不會遇到的風險等級，不能含糊放過。

修法：比照現有 `output_ring` 的做法但獨立開一個小緩衝——`PtySession` 新增 `marker_tail: Mutex<Vec<u8>>`，只保留「上一個 chunk 結尾的最後 51 個位元組」（`MARKER_LEN - 1`）。每次新 chunk 進來時：

1. 把 `marker_tail` 的內容跟這次的 `chunk`接起來（`[marker_tail, chunk].concat()`），只在這段接起來的位元組裡找標記樣式（不是重新掃整個 `output_ring`）
2. 命中就 `marker_count.fetch_add(1)`
3. 把 `marker_tail` 更新成這次 `chunk`（或接起來後的整段）結尾的最後 51 個位元組，供下一次呼叫使用

這個做法只碰「剛進來的新資料＋上一輪留下的極短尾巴」，不會重新掃描 `output_ring` 裡的舊歷史——所以不會有「同一個標記在後續好幾個 chunk 掃描裡被重複算好幾次」的問題（bell 那段程式碼註解本來就講過「重複算不影響正確性，呼叫端只在乎有沒有變化」，但這裡因為改成看一個滑動視窗，反而天然不會重複算，比 bell 更乾淨）。也不會有「舊一輪的標記被新一輪的掃描重新命中」的風險——因為只看新進位元組，不回頭看歷史。

### 指示文字

固定措辭：

```
（可選：完成後請在新的一行印出 <<AITERM_DONE:{tab_id}>>，讓協調端提早得知你已完成，不影響任何其他行為。）
```

`{tab_id}` 在寫入前用實際的 tab id 字串取代。這段文字就是最終版本，不再另外定案。

`spawn_tab`（當帶了 `command` 時）與 `send_input`，在 `request_done_marker: true` 時，各自把這段文字接在使用者要送出的原文字後面（中間留一個換行），一次性 `pty_manager.write(...)`。

### Baseline／訊號比較的正確性

`send_input` 目前的「重設 baseline 為當下的 `bell_count()`」邏輯，必須原封不動套用到 `marker_count` 上——重設的時機點是「文字寫進 PTY 之前」量測，避免這次送出後、worker 還沒開始處理就已經存在的舊標記，被誤判成這次的完成訊號（跟現有 bell baseline 重設的理由完全一樣）。`spawn_tab` 同理，把 `marker_count` 的初始 baseline 記成 `0`。

### `TabStatus`/`WaitResult` 的 `signal` 欄位

新增而非取代既有欄位，向後相容：沒開 `request_done_marker` 的既有呼叫端，`signal` 永遠是 `Some("bell")`（或直到逾時都是 `None`），行為與回傳格式跟今天完全一樣，只是多一個欄位。

## 已知限制

- 標記不是強制訊號，跟 bell 一樣——不聽話或非 MCP-aware 的 worker（例如純 shell 腳本、或沒被告知要印標記的 agent）就是純粹落回等 bell，功能上等同沒開這個選項，不會更差也不會出錯
- `spawn()`/`spawn_with_id()` 兩處重複邏輯這次會同步複製而非合併，維持既有程式碼重複的現狀
- 理論上使用者自己手動在分頁裡打出一模一樣格式（含正確 tab_id）的字串，也會被誤判成完成訊號——機率因為 UUID 隨機性趨近於零，但非絕對零，這跟 bell 本身「重用既有通知管道、非絕對可靠」的既有共識一致，v1 不特別處理

## 測試

- Rust 單元測試：比照現有 bell 測試——送標記位元組序列到讀取迴圈，確認 `marker_count()` 正確累加；確認一般輸出（含不含 tab_id 的相似文字）不會誤觸發，除非格式完全吻合；**專門測試標記被切在兩個 chunk 交界處的情境**（例如故意分兩次 `write` 各送標記字串的前半段與後半段），確認 `marker_tail` 機制仍能正確累加
- `coordination_ops.rs` 單元測試：`request_done_marker: true` 時 `send_input`/`spawn_tab` 寫入的文字有正確附加指示文字且含正確 tab_id；`status_for`/`get_tab_status`/`wait_for_idle` 在只有 marker 命中（bell 未命中）時仍正確回報 `idle: true` 且 `signal: "marker"`；`request_done_marker: false`（預設）時寫入文字不變、行為與既有測試完全一致
- 整合測試：比照既有手法，跑一次 `spawn_tab(request_done_marker: true)` → `send_input`（送一個會印出標記的假指令）→ `wait_for_idle`，確認比純等 bell 提早返回且 `signal: "marker"`
- 手動驗證：真的對一個跑 Claude Code 的分頁開啟這個選項，確認實際多輪協調流程中，加速訊號確實比 bell 更快浮現 idle，且沒開這個選項的既有流程完全不受影響

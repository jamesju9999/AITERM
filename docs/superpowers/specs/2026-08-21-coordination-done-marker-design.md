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
- `SendInputArgs` 加一個 `request_done_marker: bool`（`#[serde(default)]`，預設 `false`）
- 設為 `true` 時，`send_input` 把要寫進 PTY 的文字，在原文字後面加上一段固定措辭、含 `tab_id` 的提示（見下方「指示文字」），一次寫入，呼叫端不必自己組措辭
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

**自我觸發風險與修正（實作階段發現，回頭修正設計）**：第一版措辭直接把完整標記 `<<AITERM_DONE:{tab_id}>>` 寫進指示文字本身。這是個真正會發生、不是理論上的缺陷——在真實 PTY 上直接驗證過：只把這段指示文字寫進去，即使目標端只是一個什麼都沒做的空 shell，`marker_count` 就已經被觸發成 1。原因是終端機的本地 echo（canonical mode 標準行為）會把寫進去的位元組原樣送回輸出流，而那正是讀取迴圈在逐位元組掃描找標記的同一條資料流——任何「要傳給對方、又要事後被逐位元組比對偵測」的完整字串，只要透過同一條會被 echo 的通道傳遞，就必然自我觸發，跟對方有沒有真的完成任何事無關。

修法：把標記拆成三段描述，段落之間插入其他文字，讓完整的 52 位元組序列不會連續出現在指示文字本身裡——只有目標端真的把三段接起來印出時，才會構成連續比對命中。固定措辭：

```
（可選：完成後請在新的一行印出一個完成標記，格式為三段直接相連、中間不留任何字元：前綴 <<AITERM_DONE: ，接著是你的識別碼 {tab_id} ，最後接上 >> 。這能讓協調端提早得知你已完成，不影響任何其他行為。）
```

`{tab_id}` 在寫入前用實際的 tab id 字串取代。這段文字就是最終版本，不再另外定案。三段（前綴、tab_id、後綴）之間都夾了其他文字（空格、中文說明），確保指示文字本身的位元組序列裡，任何連續 52 位元組都不等於完整標記——這是靠文字結構保證，不是靠時間差或機率，跟前面「跨 chunk 邊界」那種位元組層級的正確性要求同一個標準。

`send_input` 在 `request_done_marker: true` 時，先照現有行為把 `text` 加 `\r` 寫入一次，緊接著把這段指示文字**再加一次獨立的 `\r`、分開寫入**（等同連續送出兩則訊息），而不是把兩段文字用 `\n` 接成一次寫入。理由：這份檔案自己既有的迴歸測試（`send_input_terminates_the_line_with_cr_not_lf`）就是在防「raw-mode 程式對 LF 位元組行為沒有保證，只有 `\r` 被驗證過能穩定觸發 Enter」這個問題——如果把指示文字用內嵌 `\n` 接在同一次寫入裡，等於在 Claude Code 這類 raw-mode TUI 面前送出未經驗證的位元組。拆成兩次各自 `\r` 結尾的寫入，完全重用已驗證安全的終止方式，不需要為這個選用功能承擔新的正確性風險。

**只加在 `send_input`，不加在 `spawn_tab`：** 寫計畫時發現的修正。`spawn_tab` 的 `command` 語意上是「啟動一個程式」（例如打開 `claude` REPL），不是「交辦一項任務」——沒有一個具體任務讓標記去回報完成，加了也不知道要標記誰的完成。更關鍵的是正確性風險：如果把指示文字接在 `command` 後面一起寫入，等於在同一次 PTY 寫入裡塞進第二行文字，會在剛啟動的程式（例如 `claude` CLI）真正就緒、能讀取輸入之前就搶著送出去——這正是 `spawn_tab` 目前「寫入前不等 shell 就緒」這個既有行為底下的既有風險類別（見 2026-08-20 設計文件與後續冷啟動延遲討論），沒必要為了這個選用功能多開一個新的風險面。真正有意義的位置是 `send_input`：那才是協調端實際把任務內容送給一個「已經在跑、預期會讀取輸入」的 agent 的地方。

### 第二段寫入被目標端忽略（手動驗證階段發現，回頭修正設計）

`send_input` 目前把「任務文字」跟「指示文字」當成兩次幾乎零延遲、背靠背送出的獨立 `\r` 寫入。這個假設在真實 Claude Code CLI 上實測失敗：任務文字送出後，目標端要花實際時間處理（本次實測「Cogitated for 6s」），在這段忙碌期間，Claude Code 的 TUI 允許把接下來打的字元插進輸入框（所以指示文字的內容確實原樣出現在畫面上），但那個緊跟著送達的 `\r` 沒有被當成「送出」——它就這樣被吞掉，指示文字停留在輸入框裡，永遠不會被目標端處理。跟目標端本身的忙碌狀態賽跑，不是機率性的邊際案例，是這個两次背靠背寫入設計本身結構性會踩到的問題。

**修法**：`send_input` 內部在寫完第一段（任務文字）之後，若 `request_done_marker: true`，先輪詢等待目標端「因為這次任務文字而觸發的新 bell」（沿用 `wait_for_idle` 現有的 250ms 輪詢間隔跟判斷邏輯，只是這次輪詢發生在 `send_input` 函式內部，等到才送出第二段），確認目標端真的處理完第一段、回到能接受新輸入的狀態，才送出指示文字。逾時（沿用 `wait_for_idle` 的 `DEFAULT_WAIT_SECONDS = 300`）就放棄送出指示文字，把這次呼叫當成單純「只送了任務文字，沒有請求加速訊號」處理——不報錯，只是回傳字串裡誠實註明沒送出指示文字的原因。

**這是必要、不是選配的行為改變**：
- `send_input` 的函式簽名要從同步 `fn` 改成 `async fn`（內部輪詢要用 `tokio::time::sleep` 而不是同步 `std::thread::sleep`，否則會佔住整個 Tokio 執行緒）；`tools.rs` 呼叫端要補上 `.await`
- 輪詢邏輯抽成一個獨立、可注入逾時時間的私有 async 函式（例如 `wait_for_new_bell(pty_manager, tab_id, baseline, timeout) -> bool`），讓正式路徑用 `DEFAULT_WAIT_SECONDS`、測試路徑可以直接呼叫這個函式帶入極短逾時（例如數百毫秒），驗證「目標端一直不 bell，逾時後放棄送出指示文字」這條路徑時不需要真的等 300 秒
- `request_done_marker: false` 時完全不受影響——不進入這段等待邏輯，維持原本零延遲的行為

**這代表 `send_input(request_done_marker: true)` 不再是瞬間回傳的呼叫**：它現在的耗時等於「目標端處理完這次任務文字所需的時間」，跟另外呼叫一次 `wait_for_idle` 差不多量級——這是刻意的取捨，換來協調端一次呼叫就能正確完成，不需要自己分兩步管理「送任務→等待→送指示」。`SendInputArgs.request_done_marker` 的工具說明文字要更新，誠實告知呼叫端這件事（可能等到最多 300 秒），避免呼叫端誤以為這跟 `request_done_marker: false` 一樣是瞬間回傳。

### Baseline／訊號比較的正確性

`send_input` 現有的順序是**先寫入、寫完立刻讀 `bell_count()` 當新 baseline**（不是寫入前）——註解說明理由是「這個時間點量到的數字，實務上不可能已經包含對方的回覆」，寫入本身跟讀計數器都是本地、近乎瞬間的操作，對方不可能在這麼短的間隔內就已經處理完並回覆。這次比照同一個順序，`marker_count` 也在寫入之後、回傳之前量測一次。

當 `request_done_marker: true` 時（見上方「第二段寫入被目標端忽略」），會有**兩次**寫入（任務文字一次、指示文字一次，各自獨立 `\r` 結尾），中間夾了一段等待目標端因任務文字而觸發新 bell 的輪詢——baseline 的量測要放在**兩次寫入都完成之後**（不管中間那段等待是等到 bell 才送出第二段、還是逾時放棄送第二段，都要走到這裡才量測），只量一次，語意是「這整次 `send_input` 呼叫自己造成的輸出都算數，在那之前的都不算」。`spawn_tab` 同理，把 `marker_count` 的初始 baseline 記成 `0`。

### `TabStatus`/`WaitResult` 的 `signal` 欄位

新增而非取代既有欄位，向後相容：沒開 `request_done_marker` 的既有呼叫端，`signal` 永遠是 `Some("bell")`（或直到逾時都是 `None`），行為與回傳格式跟今天完全一樣，只是多一個欄位。

## 已知限制

- 標記不是強制訊號，跟 bell 一樣——不聽話或非 MCP-aware 的 worker（例如純 shell 腳本、或沒被告知要印標記的 agent）就是純粹落回等 bell，功能上等同沒開這個選項，不會更差也不會出錯
- `spawn()`/`spawn_with_id()` 兩處重複邏輯這次會同步複製而非合併，維持既有程式碼重複的現狀
- 理論上使用者自己手動在分頁裡打出一模一樣格式（含正確 tab_id）的字串，也會被誤判成完成訊號——機率因為 UUID 隨機性趨近於零，但非絕對零，這跟 bell 本身「重用既有通知管道、非絕對可靠」的既有共識一致，v1 不特別處理
- `request_done_marker: true` 時 `send_input` 不再瞬間回傳——它會等到目標端因任務文字而觸發新 bell（最長等 `DEFAULT_WAIT_SECONDS = 300` 秒）才送出指示文字，逾時就放棄送出指示文字。目標端如果從不觸發 bell（例如 `preferredNotifChannel` 沒設成 `terminal_bell`），這次呼叫就要等到滿 300 秒才會回來——這是刻意的取捨，換來協調端不用自己分兩步管理「送任務→等待→送指示」，工具說明文字必須誠實告知這件事

## 測試

- Rust 單元測試：比照現有 bell 測試——送標記位元組序列到讀取迴圈，確認 `marker_count()` 正確累加；確認一般輸出（含不含 tab_id 的相似文字）不會誤觸發，除非格式完全吻合；**專門測試標記被切在兩個 chunk 交界處的情境**（例如故意分兩次 `write` 各送標記字串的前半段與後半段），確認 `marker_tail` 機制仍能正確累加
- `coordination_ops.rs` 單元測試：`request_done_marker: true` 時 `send_input` 寫入的文字有正確附加指示文字且含正確 tab_id；`status_for`/`get_tab_status`/`wait_for_idle` 在只有 marker 命中（bell 未命中）時仍正確回報 `idle: true` 且 `signal: "marker"`；`request_done_marker: false`（預設）時寫入文字不變、行為與既有測試完全一致；**自我觸發回歸測試**——`send_input(..., request_done_marker: true)` 對一個只會原樣 echo（不主動印出標記）的目標送出後，短時間內 `marker_count`／`idle` 不能變成 true（這是實作階段發現且修正過的真實 bug，必須有測試鎖住，不能靠人工複查）
- **第二段等待邏輯回歸測試**（手動驗證階段發現且修正過的真實 bug，必須有自動化測試鎖住，不能只靠人工複查）：
  - 對一個「先模擬觸發一次 bell（代表目標端因任務文字而回到閒置），才能觀察到第二段指示文字被送出」的情境，確認指示文字真的被送達——這是既有的 `send_input_with_request_done_marker_sends_the_instruction_without_self_triggering` 測試要補上的步驟：手動 `write` 一個 bell 位元組序列到 PTY，模擬「目標端剛處理完任務文字」，`send_input` 的內部等待才會解除去送第二段
  - 對一個**從不觸發 bell** 的目標（例如單純的空 shell），確認 `send_input(..., request_done_marker: true)` 逾時後不會送出指示文字，且函式本身正常回傳（不是掛住或報錯）——這個測試必須透過內部可測試的輪詢函式（見「第二段寫入被目標端忽略」段落，帶入一個極短的逾時值），不能真的等 `DEFAULT_WAIT_SECONDS = 300` 秒
- 整合測試：比照既有手法，跑一次 `spawn_tab` → `send_input(request_done_marker: true)`（送一個會印出標記的假指令）→ `wait_for_idle`，確認比純等 bell 提早返回且 `signal: "marker"`
- 手動驗證：真的對一個跑 Claude Code 的分頁開啟這個選項，確認實際多輪協調流程中，加速訊號確實比 bell 更快浮現 idle，且沒開這個選項的既有流程完全不受影響——**這一步實測時就是抓到「第二段寫入被目標端忽略」這個 bug 的地方**：修正前，指示文字會停留在目標端輸入框裡從未被送出；修正後要重新走一次這個手動驗證，確認指示文字真的被目標端處理、印出完成標記

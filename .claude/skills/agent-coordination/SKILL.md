---
name: agent-coordination
description: 當你（協調端）要透過 AITerm 的 spawn_tab/send_input/get_tab_status/wait_for_idle 這組 MCP 工具，開新分頁指揮另一個 coding agent（例如另一個 Claude Code、Codex）去做任務時使用此 skill。適用情境：使用者說「開一個新的終端機執行 XXX」、「請另一個 agent 幫我做 YYY」、或任何需要你分派任務給子分頁並拿到結果的場景。
---

## 核心規則：送出任務後，一定要確認完成才能結束回合

**絕對不要在呼叫 `send_input` 之後就直接結束這個回合、或跟使用者回報「已經請它做了」。** `spawn_tab`/`send_input` 的回傳值只是「這次寫入有沒有成功送達分頁」的確認，**不包含子分頁實際做了什麼**。子分頁的輸出只存在於它自己的終端機畫面裡，你如果不主動查詢，就永遠不會知道結果——包括子分頁其實幾秒內就做完了，你卻讓使用者以為還在跑。

正確流程永遠是這三步，不能省略最後一步：

```
1. spawn_tab({cwd: <你自己的工作目錄>, ...})  → 拿到 tab_id（cwd 務必填，見下方「信任提示」段落）
2. send_input({tab_id, ...})                  → 任務送達（不代表做完）
3. wait_for_idle({tab_id})                    → 阻塞直到真的完成，這一步才算數
```

## 四個工具的實際行為

| 工具 | 用途 | 回傳內容 | 是否阻塞 |
|---|---|---|---|
| `spawn_tab({cwd?, command?})` | 開一個新分頁（使用者在 UI 上看得到），可選擇立即執行初始指令（例如 `"claude"`） | 純 `tab_id` 字串 | 否，立即回傳 |
| `send_input({tab_id, text, request_done_marker?})` | 對分頁送一段文字＋Enter，就像使用者打字 | 確認字串（不含分頁輸出） | `request_done_marker: false`（預設）時立即回傳；`true` 時最多阻塞 15 秒等目標端閒下來才送出加速訊號的指示文字，逾時就放棄送指示文字（不報錯，回傳訊息裡會註明） |
| `get_tab_status({tab_id})` | 查一次目前狀態，不等待 | `{idle, recent_output, signal}` | 否，立即回傳當下快照 |
| `wait_for_idle({tab_id, timeout_seconds?})` | **阻塞直到分頁閒置或逾時** | `{idle, recent_output, timed_out, signal}` | 是，預設最長等 300 秒（可傳 `timeout_seconds` 調整，上限 1800） |

`recent_output` 是終端機畫面最近 4096 bytes 的原始文字（已去除 ANSI 顏色碼），不是解析過的結構化答案——你要自己從裡面讀出你要的資訊。

**`send_input` 只能對自己用 `spawn_tab` 開出來的 `tab_id` 下指令**——使用者手動開的分頁永遠不是合法目標，這是刻意的安全限制，會回錯誤而不是靜默失敗。

## 開新分頁一定要傳自己的 `cwd`，否則極可能卡在信任提示

實測證實（用真實 `claude` CLI 反覆驗證）：只要 `spawn_tab` 開分頁時沒有指定 `cwd`（或指定了一個 `claude` 沒見過的目錄），`claude` 啟動後幾乎每一次都會停在下面這個畫面，**卡住等人按鍵，不會自己過去**：

```
Is this a project you created or one you trust? ...
❯ 1. Yes, I trust this folder
  2. No, exit
Enter to confirm · Esc to cancel
```

這是最容易被誤判成「子分頁還在初始化、卡住了」的情境——`wait_for_idle` 在這個畫面前會一直逾時（因為沒人按鍵，永遠不會有新 bell），如果你只是不斷重複 `wait_for_idle`／`get_tab_status` 死等而不去看 `recent_output` 裡實際寫了什麼，就會像真的卡住一樣、白白浪費好幾輪逾時。

**兩件事都要做：**

1. **`spawn_tab` 一定要傳 `cwd`，用你自己當下的工作目錄**（例如你能取得的 `pwd`）——這個工具本身極大機率已經被信任過，用同一個目錄開新分頁可以避開這個提示。不要讓 `cwd` 留空。
2. **就算避開了，也要學會辨認並處理這個畫面**：每次 `get_tab_status`/`wait_for_idle` 逾時拿到 `recent_output` 後，先檢查裡面有沒有出現類似 `"trust this folder"`、`"Yes, I trust this folder"` 這類字樣。如果有：
   - 呼叫 `send_input({tab_id, text: "1"})`（不需要開 `request_done_marker`）送出確認，選第一個選項信任該資料夾。
   - 送出後再呼叫一次 `wait_for_idle` 讓子分頁真正進入可以工作的狀態。
   - 不要把這次逾時當成任務失敗——這是預期中會發生的一次性關卡，處理掉就能繼續。

## `timed_out: true` 代表「還在做」，不是「失敗」或「放棄」

`wait_for_idle` 逾時只代表「在你設定的時間內沒等到」，**不代表子分頁卡住或失敗**——很可能它還在跑一個比較久的任務。遇到逾時：

1. **不要直接放棄或跟使用者說「失敗了」**——這是誤判。
2. 可以再呼叫一次 `wait_for_idle` 繼續等（同一個 `tab_id`，`wait_for_idle` 本身沒有「已經等過一次」的狀態，重新呼叫就是重新等一輪）。
3. 如果任務預期會很久，一開始就把 `timeout_seconds` 設高一點，或者把這一輪的等待拆成「先回報目前進度給使用者，下一輪再繼續等」，不要讓自己卡住整個對話不動。
4. 只有連續多次逾時、或子分頁畫面（`recent_output`）明顯顯示錯誤/當機時，才判斷成真的卡住，才需要人工介入或考慮結束任務。

## `request_done_marker` 該不該開

這是一個**選用的加速訊號**，讓子分頁的 agent 主動印出完成標記，可能比純等 terminal bell 更快讓 `wait_for_idle` 偵測到閒置。

- **只有目標端也是配合的 agent（例如另一個 Claude Code）才值得開**——它需要讀懂 `send_input` 自動附加的指示文字並照做。對純 shell 腳本或不會讀指示的程式開這個沒有意義，但也不會出錯，純粹是浪費那最多 15 秒的等待。
- 開了之後，`send_input` 這次呼叫本身會變慢（最多阻塞 15 秒），不再是瞬間回傳——這是預期行為，不是異常。
- 就算目標端完全沒反應（15 秒內不觸發任何 bell），`send_input` 也會正常回傳、只是註明沒送出加速指示——不會拋錯、不會卡死，接下來照樣呼叫 `wait_for_idle` 就好，功能上退化成沒開這個選項一樣。
- 不確定要不要開就**先不開**——bell 訊號本身已經是主要、可靠的判斷依據，這個開關只是錦上添花。

## 同時協調多個子分頁

目前沒有「等任一個先完成」的單一工具。要協調多個 agent 併行工作，你自己要維護一份 `tab_id` 清單，用下列其中一種方式輪詢：

- **依序等**：對第一個 `tab_id` 呼叫 `wait_for_idle`（給一個較短的 `timeout_seconds`，例如 30-60 秒），逾時就換下一個查 `get_tab_status`（不阻塞，只看一眼），輪流下去，直到全部回報 `idle: true`。
- **一次查全部快照**：對每個 `tab_id` 各呼叫一次 `get_tab_status`（不阻塞），累積出目前整體進度，再決定要不要針對還沒完成的那幾個繼續 `wait_for_idle`。

不要對其中一個 `tab_id` 用很長的 `timeout_seconds` 死等，那會讓其他子分頁的進度完全被你忽略。

## 常見錯誤（真實發生過，務必避免）

- ❌ 呼叫完 `spawn_tab` + `send_input` 就直接跟使用者說「已經完成」或結束回合——這時候任務可能才剛送達，子分頁根本還沒開始做。
- ❌ 把 `wait_for_idle` 逾時當成任務失敗，直接放棄或報錯給使用者。
- ❌ `spawn_tab` 沒傳 `cwd` 就開新分頁執行 `claude`——實測幾乎每次都會卡在信任提示前，且協調端容易誤判成「還在初始化」而不斷空等（見上方「開新分頁一定要傳自己的 `cwd`」段落）。
- ❌ 對從未回應過的分頁無限期重複 `wait_for_idle` 而不查看 `recent_output` 找線索（例如子分頁其實卡在信任提示、需要人工按鍵確認）。
- ❌ 開了 `request_done_marker: true` 卻預期它跟 `false` 一樣瞬間回傳——這會讓你誤判逾時邏輯。

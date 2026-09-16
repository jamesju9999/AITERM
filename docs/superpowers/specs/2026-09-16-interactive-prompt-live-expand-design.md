# 遠端終端機／本機終端機執行需要單鍵回應的互動提示時，即時窗格自動展開

日期：2026-09-16

## 起因

使用者在遠端終端機裡執行 `claude` CLI，遇到「是否信任這個資料夾」的互動
選單（方向鍵選 `Yes, I trust this folder` / `No, exit`，Enter 確認）。

實測抓取 `claude` CLI 送出的原始位元組，這個提示**不會**切換到 alternate
screen buffer（沒有 `ESC[?1049h`），純粹用游標定位（`ESC[2G`、`ESC[8G`…）
在目前畫面原地重繪。而 `RemoteTerminalView`／`TerminalView` 判斷「要不要
把 WarpInput 命令列跟卡片區隱藏、鍵盤直接原始送給遠端」的條件目前只有
`isAlternateBuffer`（`useTerminalBlocks.ts:350`，讀 `term.buffer.active.type`）。
沒有切 alt-screen 的互動選單，因此會停在「輸入指令」模式：使用者打的方向
鍵會被 `WarpInput` 當成文字輸入框裡的游標移動或歷史紀錄鍵處理，送不到遠端
——選單卡住按不動。

同一份實測也發現，這個提示在跳出來、以及使用者按下確認鍵結束時，都會送出
一組目前完全沒人處理的逃逸序列：

```
ESC[<u   ESC[>5u   ESC[>4;2m     ← 進入提示時
...
ESC[>4m  ESC[<u                  ← 使用者確認、提示結束時
```

`ESC[>Ps u` / `ESC[<u` 是 Kitty keyboard protocol 的「push / pop」——程式
主動宣告「接下來我要逐鍵收原始按鍵，不要交給 shell 的行編輯器」。這跟
alt-screen 的 `?1049h/l` 是同一類「遠端主動宣告模式切換」的訊號，只是這次
不換螢幕、只換鍵盤語意。

## 目標

偵測到遠端進入這種「要逐鍵原始輸入」的狀態時，即時窗格（live frame）自動
撐高、隱藏 WarpInput／卡片區，讓鍵盤直接原始送給遠端；訊號消失（使用者已
回應、程式退出這個模式）後恢復原本高度，回到一般輸入指令的狀態。

非目標：

- 不自動幫使用者選選項、不自動送任何按鍵。上一輪處理 agent 自動派工的
  信任提示時已經驗證過，寫死方向鍵在選項順序被上游對調時會誤觸破壞性選項
  （見專案記憶 `feedback_tui_option_order_not_fixed`）；這裡是人在看畫面，
  展開只是把畫面跟輸入通道準備好，選擇仍然由使用者自己按。
- 不處理只用 `modifyOtherKeys`（`ESC[>4;2m`）而完全不送 Kitty push/pop
  的假設情境。這次實測的目標程式兩者都送，先只認 Kitty push/pop 就足夠
  解決眼前的問題；真的遇到只有 `modifyOtherKeys`、沒有 Kitty push 的程式
  再擴充（YAGNI）。

## 偵測機制

### 用 xterm.js 既有的 parser hook，不做手動位元組掃描

`useTerminalBlocks.ts` 已經用 `term.parser.registerOscHandler(133, ...)`
掛 OSC 133 的處理器（見 `useTerminalBlocks.ts:365`）。xterm.js 的
`IParser` 同樣公開 `registerCsiHandler(id, callback)`，`id` 可以指定
`prefix`（合法範圍 `\x3c`–`\x3f`，含 `<` 跟 `>`）與 `final` byte。這正好
涵蓋 Kitty push/pop 用到的兩種序列，不需要自己對原始字串做 regex：

- `registerCsiHandler({ prefix: ">", final: "u" }, ...)` 對應 `ESC[>Ps u`
  （push，帶參數）
- `registerCsiHandler({ prefix: "<", final: "u" }, ...)` 對應 `ESC[<u`
  （pop，通常不帶參數）

用 xterm 自己的 parser 而不是手動掃 chunk 字串，是因為 xterm 已經處理好
「一個逃逸序列被拆成兩次 PTY read() / 兩次 `term.write()` 呼叫」這種邊界
情況——跟現有 OSC 133 處理是同一個理由、同一個既有先例。

### 用 depth 計數器，不是布林值

Kitty 協定的 push/pop 是可疊加的堆疊語意（可能連續 push 兩次才 pop 一次）。
用一個 `rawKeyboardDepthRef`（`useRef(0)`）記錄深度：

- push handler：`rawKeyboardDepthRef.current += 1`，然後
  `setIsRawKeyboardModeActive(true)`
- pop handler：`rawKeyboardDepthRef.current = Math.max(0, rawKeyboardDepthRef.current - 1)`，
  然後 `setIsRawKeyboardModeActive(rawKeyboardDepthRef.current > 0)`

實測觀察到 Ink 類程式每次重繪畫面都會在同一包輸出裡先 pop 再重新 push
（例如上面貼的中段序列），這是它自己的渲染實作細節，不代表使用者輸入的
提示真的結束了。用深度計數器（而非「看到 pop 就關」的布林值）搭配 xterm
parser 逐一序列觸發 callback 的天性，同一包輸出裡的 pop→push 會在極短時間
內把深度打回同樣的值，狀態不會真的翻轉成「已結束」，避免閃爍或誤判成已
回應。

新增的 `isRawKeyboardModeActive: boolean` 跟現有 `isAlternateBuffer`一起
從 `useTerminalBlocks` 回傳。

### 安全網：強制重置

`isAlternateBuffer` 是每次 `onBufferChange` 直接讀 `term.buffer.active.type`
算出來的，天生會自我校正、不會卡住。但 `rawKeyboardDepthRef` 是我們自己
維護的計數器，如果指令中途被強制中斷（斷線、卡住偵測介入送 Ctrl+C 強制
結案——見 `project_agent_command_deadlock`），遠端可能來不及送出最後一個
pop，深度會卡在 >0，畫面永遠展開收不回去。

因此在 `clearAllBlocks()` 與 `finalizeBlock` 的強制結案路徑，都要把
`rawKeyboardDepthRef.current` 明確歸零、`isRawKeyboardModeActive` 設回
`false`，當作跟自我校正的 `isAlternateBuffer` 同等可靠的保險。

## 展開行為

`TerminalView.tsx` 與 `RemoteTerminalView/index.tsx` 各自維護一份
`MIN_LIVE_ROWS` / `MAX_LIVE_ROWS` 常數與 `liveRows` state（詳見
`TerminalView.tsx:152-153`、`RemoteTerminalView/index.tsx` 對應區塊）。
兩處都新增 `EXPANDED_LIVE_ROWS = 24`——介於現有 `MAX_LIVE_ROWS`（16）跟
全螢幕高度之間，理由是這類提示通常只有十幾行，撐到跟 alt-screen 一樣的
「填滿整個主控端螢幕高度」對這個情境來說跳動過大。

兩處原本判斷「隱藏 WarpInput／卡片區、鍵盤直接原始送出」的條件式從
`isAlternateBuffer` 改成 `isAlternateBuffer || isRawKeyboardModeActive`。
`liveRows` 的行為：

- `isRawKeyboardModeActive` 從 `false → true`：`setLiveRows(EXPANDED_LIVE_ROWS)`
- `isRawKeyboardModeActive` 從 `true → false`（且沒有同時在 alt-screen
  中）：`setLiveRows(MIN_LIVE_ROWS)`，跟現有「指令完成、卡片收合」用的是
  同一個收回時機邏輯

外層容器維持 `overflow: clip`（見專案記憶 `feedback_live_frame_overflow_clip`
——這裡絕對不能改成 `auto`/`hidden`，那兩者都會讓瀏覽器把這層當成捲動
容器，在貼上/resize 時自動把畫面捲走）。`EXPANDED_LIVE_ROWS` 放不下的
內容，交給 xterm 內建的滑鼠滾輪 scrollback，不新增額外的 UI 或狀態。

沒有新增任何可見的手動切換開關或提示徽章——展開的畫面本身就是唯一的
視覺訊號，跟目前 alt-screen 展開的體驗一致，不需要額外教育使用者。

## 測試

`useTerminalBlocks.test.ts`（用既有的「建立真實 `Terminal` 實例、
`term.write()` 餵原始位元組」模式，跟 OSC 133 的測試同一套手法）：

- 餵 `ESC[>5u` 後，`isRawKeyboardModeActive` 變 `true`。
- 接著餵 `ESC[<u`，變回 `false`。
- 連續兩次 `ESC[>5u` 只餵一次 `ESC[<u`，仍然是 `true`（深度計數器沒有
  被單次 pop 打到 0）。
- 同一個模擬「重繪」場景：`ESC[<u` 緊接著 `ESC[>5u` 在同一次 `term.write()`
  呼叫裡送達，最終狀態仍是 `true`，不因為中間那個 pop 而閃成 `false`。
- 呼叫 `clearAllBlocks()` 或觸發強制結案路徑時，即使深度 >0，也會被歸零
  成 `false`。

`TerminalView.remoteLiveHeight.test.tsx` / 對應的本機分頁測試（沿用現有
mock `useTerminalBlocks` 回傳固定值的模式）：

- mock 回傳 `isRawKeyboardModeActive: true` 時，`liveRows` 對應到
  `EXPANDED_LIVE_ROWS`，WarpInput／卡片區不渲染。
- 從 `true` 變 `false` 後，`liveRows` 收回 `MIN_LIVE_ROWS`。
- `isAlternateBuffer` 為 `true` 時的既有行為不受影響（沒有把兩個狀態的
  高度計算路徑接錯）。

## 風險與取捨

- **只認 Kitty push/pop，不認 `modifyOtherKeys`。** 如果之後遇到只用
  `modifyOtherKeys`（`ESC[>4;2m`）宣告原始鍵盤模式、完全不送 Kitty
  push/pop 的程式，這次的偵測不會觸發，行為退回目前（維持一般輸入模式，
  使用者打字被當成單行指令，方向鍵按了沒反應）。跟現有「定位不到就什麼
  都不做」的保守設計是同一個風險方向——不會誤觸更糟的行為，只是不生效。
- **深度計數器不是自我校正的狀態**，仰賴「安全網強制重置」這個額外機制
  才不會卡住。如果之後在 `finalizeBlock`／`clearAllBlocks` 之外還有其他
  會提前結束一個 running 區塊的路徑，需要記得比照辦理。
- **`EXPANDED_LIVE_ROWS = 24` 是拍板值**，沒有從實際各類互動提示的行數
  統計出來。如果之後出現常態性超過 24 行又需要逐鍵輸入的提示，使用者
  只能靠滑鼠滾輪往回看，體驗會打折但不會無法使用。

## 更新（2026-09-16）：即時窗格對齊機制從 CSS 位移改成 `scrollToLine`

出貨後真機測試在**長連線（20 分鐘以上，累積大量 scrollback）**下踩到新
問題：原本的對齊機制（`liveTopRows`，位在 Windows 遠端／本機分頁）算
「提示字元的絕對列數」跟「目前捲動到哪」的差距，再用 CSS `top` 把 host
往上位移那個差距。這個算法假設「執行中的指令會持續吐出新內容，讓差距
自然收斂到 0」。`claude` CLI 的信任提示印一次就停下來等按鍵，不會持續
吐內容——連線開得越久（scrollback 越深），這個差距就卡得越住，即時窗格
被夾到只剩一兩列高，畫面幾乎全黑（但內容其實已經完整進到 xterm 的 DOM
裡，用 devtools 展開 `.xterm-rows` 可以看到，只是被裁到視野外——這點靠
真機錄影拆幀 + devtools 檢查證實過，排除是資料沒收到的可能）。

中間試過「指令執行中直接把位移歸零」，結果讓窗格瞬間跳去顯示「目前捲動
位置」最上面幾列——Windows 不清緩衝區，那個位置可能還停在上一個已經變
成卡片的指令輸出，於是重複顯示了一次舊內容（另一次真機錄影證實）。

**最終做法**：改用 xterm.js 公開的 `term.scrollToLine(line)`，直接把
viewport 捲到提示字元那個絕對列，不用再算差距、也不用等它自然收斂——
不管 scrollback 多深，捲完之後位移量恆為 0。`liveTopRows` 因此永遠是
`0`，`liveRows` 只需要被 `term.rows` 本身夾住（`Math.min(desired,
term.rows)`），不用再算「扣掉位移量後還剩多少空間」。原本的 CSS
`position: absolute; top: -Npx` 那條路徑仍然保留在程式碼裡（`liveTopRows
> 0` 才會走到），但實際上永遠不會被觸發——沒有一併刪除，因為拿掉會擴大
這次修改的範圍，风险與效益不成比例。

**已知限制**：這個 repo 目前所有測試（hook 測試用的無頭 `Terminal`、
元件測試用的假 `Terminal` 類別）都沒有真的呼叫 `.open()` 掛到 DOM 上，
而 `scrollToLine` 需要真正的 renderer 才會實際改變 `viewportY`——直接
拿無頭 Terminal 驗證過，`scrollToLine`／`scrollToTop`／`scrollToBottom`
在這個環境下全部是 no-op。這代表這次的自動化測試只能驗證「有沒有正確
呼叫 `scrollToLine`」，沒辦法像前面幾版一樣證明畫面真的會捲到對的位置，
最終正確性是靠使用者在真機（真正的遠端 Windows 連線）上重新測試確認的，
不是靠這次新增的測試。

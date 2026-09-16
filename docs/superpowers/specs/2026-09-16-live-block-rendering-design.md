# 執行中的指令跟已完成的卡片用同一套渲染，不再用固定裁切的即時窗格

日期：2026-09-16

## 起因

同一天稍早修了四輪「即時窗格對齊」的 bug（見
`docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md`）：
用一個固定大小、靠位移/`scrollToLine` 對齊到提示字元的小視窗（`.aiterm-live-frame`）
顯示「正在執行的指令」，而「已完成的指令」則轉成用 `parseAnsiToRenderedLines`
解析成一行一行的樣式化文字、放進捲動的卡片列表。這兩套顯示機制天生不同，
「小視窗要撐多高、要位移多少」這類問題本質上無解——只要視窗比實際內容小，
就一定要在「裁掉一部分」跟「對齊錯位置」之間選一個。

比對 Warp 的做法：它的「還在執行中」的區塊**跟已完成的卡片用完全同一種
顯示方式**——一行一行自然往下排、需要多高就多高，超出視窗的部分交給頁面
本來就有的捲動處理，不會被裁切、也不需要對齊到哪一行。

## 目標

「正在執行中」的指令輸出跟「已完成」的卡片用同一套渲染管線顯示，讓即時
內容自然撐開高度，不再需要 `liveRows`/`liveTopRows`/位移/`scrollToLine`
這一整套機制。

非目標：

- **全螢幕程式（alternate screen buffer，vim/htop 這類）不動。** 這類程式
  需要固定尺寸的畫面才能正常運作，現在已經是撐滿可用高度、不裁切，沒有
  這次要解決的問題。
- **不重寫終端機引擎。** 真正互動、正確處理游標定位/清行等 ANSI 語意的
  還是同一個 xterm.js `Terminal` 實例，這次只是不讓使用者直接看它的 DOM。

## 關鍵發現：已經有現成的積木可以組

指令結束時，`useTerminalBlocks.ts` 的 `finalizeBlock`
（`useTerminalBlocks.ts:250-`）已經在做這件事的「單次版」：如果有登記過
`outputStartRef`（`term.registerMarker(0)`，記錄這個指令輸出開始的絕對
列），就呼叫 `readRenderedLines(buf, marker.line, endRow, cols)`——直接
讀 xterm 即時緩衝區裡**已經算好的最終畫面狀態**（游標定位、清行這些
xterm 自己在處理位元組時就已經正確算完了），轉成一行一行的樣式化文字。
這個 marker 目前只在 Windows 平台才會登記（`useTerminalBlocks.ts:437`：
`if (hostPlatform === "windows" && ...)`）。

這次要做的事，本質上就是：**把這個「單次、只在結束時做一次」的動作，
改成「執行中每個 chunk 都做一次」，並且讓 marker 登記變成所有平台通用**
（不再只有 Windows）。不需要另外寫一套解析器，也不需要在 xterm 之外
另外追蹤游標位置或畫面狀態——`readRenderedLines` 要的東西 xterm 的即時
緩衝區裡都已經有。

## 架構

### xterm.js 的角色：只當「事實來源」，不再是使用者看到的畫面

真正互動的 `Terminal` 實例完全不變——鍵盤輸入（`term.onData`）、OSC 133 /
Kitty keyboard protocol 的 CSI 解析、alt-screen 偵測，全部照舊。改變的
只有**它的 DOM 不再顯示給使用者看**：把 host 容器用 CSS 移到畫面外
（`position: absolute; left: -99999px`），保留原本的實際尺寸（xterm 需要
真正的像素尺寸才能正確量出字元格大小、正確回報 cols/rows 給 PTY），只是
不佔用畫面上的版面、也不會被使用者看到。

全螢幕程式（`isAlternateBuffer`）使用中時，這層隱藏**不生效**——那種情況
維持現在的行為：xterm 的 DOM 直接顯示、撐滿可用高度。只有「有 running
中的區塊、且不是全螢幕程式」這個情況才把 xterm 藏起來，改顯示下面這個
新機制。

### `renderedLines` 從「結束時算一次」改成「執行中持續更新」

`useTerminalBlocks.ts` 新增一個內部函式（沿用 `finalizeBlock` 裡已經有的
邏輯，抽出來共用），在每次 `appendOutput` 被呼叫、且有登記中的 marker
時，用 `readRenderedLines(buf, marker.line, 目前游標所在的絕對列 + 1, cols)`
算出最新的 `renderedLines`，寫回這個 running 中的 block。

**節流**：如果一個指令狂送輸出（例如 `cat` 一個大檔案），每個 chunk 都
重新掃一次從頭到目前為止的所有列，效能會隨輸出量變差（`readRenderedLines`
是 O(列數) 的線性掃描，不是遞增式的）。用 `requestAnimationFrame` 節流：
同一個畫面更新週期內收到多個 chunk，只在最後一次真正呼叫
`readRenderedLines`，不會累積成 O(chunk 數 × 列數)。

marker 登記的時機從「只在 Windows 且偵測到 OSC 133 C 時」改成「任何平台、
任何一個新的 running 區塊建立時都登記」（`submitCommand`／
`beginTrackedBlock`／OSC 133 C 的復原路徑，三個建立區塊的地方都要登記）。

### 卡片列表：running 的區塊也給 `TerminalBlockCard` 畫

`TerminalView.tsx`／`RemoteTerminalView/index.tsx` 目前過濾卡片列表用的
是 `blocks.filter((b) => b.status !== "running" && b.renderedLines)`——
running 中的區塊被排除在外，只靠獨立的即時窗格顯示。改成
`blocks.filter((b) => b.renderedLines)`（不再排除 running），讓 running
中、已經有 `renderedLines` 的區塊也進到同一個捲動列表、用同一個
`TerminalBlockCard` 畫。

`TerminalBlockCard` 本身幾乎不用改：`duration`／`isFailed` 這些欄位本來
就是從 `block.endTime`／`block.status === "failed"` 算出來的，running 中
的區塊這兩個欄位本來就是 `undefined`／`false`，卡片會自然顯示成「沒有
耗時、沒有失敗標記」——但看起來會很像「還沒完成」，不夠明確。新增一個
小改動：running 中的卡片頭顯示一個持續更新的耗時（`Date.now() -
startTime`，用 `setInterval` 或依附在既有的 re-render 上更新），視覺上
對應 Warp 那個一直跳動的 `(5.748s)`。

### 鍵盤焦點：點畫面上的內容要能繼續打字

即時內容現在顯示的是靜態的 `<pre>`／`<span>`（`TerminalBlockCard` 的
既有渲染），本身不是可聚焦、可輸入的元件。使用者點下去、或本來就有
焦點在這個分頁上打字時，鍵盤事件要能送到那個被藏起來的 xterm（它自己
的 helper textarea 本來就是聚焦跟接收鍵盤事件的地方）。做法：running
中的那張卡片本體加一個 `onClick`，呼叫 `termRef.current?.focus()`；
分頁本身既有的「開啟時自動聚焦」邏輯不用改，因為它本來就是聚焦同一個
xterm 實例。

### 整批移除：`liveRows` 對齊機制

以下這些今天稍早新增/大改的東西全部整批刪除，不留死碼：

- `TerminalView.tsx`／`RemoteTerminalView/index.tsx` 的
  `MIN_LIVE_ROWS`／`MAX_LIVE_ROWS`／`EXPANDED_LIVE_ROWS` 常數
- `liveRows`／`liveTopRows`／`desiredLiveRowsRef`／
  `recomputeLiveGeometry`／`requestLiveRows` 這一組 state／ref／callback
- `syncLiveTopRef`／`promptAbsRowRef`／`onPromptStart` 這條「回報提示
  字元絕對列」的管線（`useTerminalBlocks` 的 `onPromptStart` 參數、
  OSC 133 B 分支裡呼叫它的那段）——沒有位移機制了，不需要知道提示字元
  在哪一列
- `untrackedCommandBoundaryRef`（`useTerminalBlocks` 的
  `onUntrackedCommandBoundary` 參數）——這個存在的唯一理由是「遠端觀看者
  自己輸入、沒有經過 `submitCommand` 的指令，也要讓即時窗格撐高」，
  現在 running 的區塊本來就會自然撐開高度，不需要這個訊號了。但「觀看
  已經在跑的指令」這個復原路徑（`recoverUntrackedCommand`）本身要留著，
  只是不再需要額外通知撐高。
- `.aiterm-live-frame`／`.aiterm-terminal-root` 的所有跟高度/位移相關的
  行內樣式（`liveHeightPx`／`liveTopOffsetPx`／`altBufferHeightPx` 保留，
  因為 alt-screen 情境還在用；跟 running-but-not-alt-screen 相關的
  高度/位移計算整段刪除）

`isAlternateBuffer` 偵測、`isRawKeyboardModeActive`（Kitty keyboard
protocol push/pop）偵測完全不受影響——這兩個是從 xterm parser 讀來的
訊號，跟畫面怎麼顯示無關，繼續留著（`isRawKeyboardModeActive` 仍然用來
決定要不要隱藏 WarpInput／把鍵盤導向 running 中的內容）。

## 資料流程總覽

```
PTY bytes 進來
  → term.write()（xterm 即時更新內部緩衝區、正確處理游標定位/清行）
  → appendOutput(chunk)：
      1. rawOutput 累加（不變，既有行為）
      2. 如果有登記中的 marker：用 requestAnimationFrame 節流，
         readRenderedLines(term.buffer.active, marker.line, 目前游標列+1, cols)
         算出最新 renderedLines，寫回這個 block
  → React 重新渲染：這個 block 進到 TerminalBlockCard 的畫面
    （running 中：no duration/exit code，卡片頭顯示持續更新的耗時）
  → OSC 133 D 觸發 finalizeBlock：狀態變 completed/failed，
    renderedLines 已經是最新的了（前面持續更新的結果），
    不需要再重新解析一次
```

## 測試

`useTerminalBlocks.test.ts`：

- 建立區塊後餵幾行輸出（不觸發 OSC 133 D），確認 `renderedLines`
  在指令還是 running 狀態時就已經是最新解析結果，不是 `undefined`。
- 節流：同一個 `requestAnimationFrame` 週期內連續呼叫多次
  `appendOutput`，`readRenderedLines` 只被呼叫一次（用 spy 驗證）。
- marker 在所有平台（不只 Windows）都會登記：不傳 `hostPlatform` 或傳
  `"other"`，一樣能在 running 中拿到 `renderedLines`。
- `clearAllBlocks()`／強制 finalize 時，未處理完的節流計時器要被清掉，
  不會在區塊已經不存在之後還嘗試寫入。

`TerminalView.*.test.tsx`／`RemoteTerminalView/index.test.tsx`：

- running 中的區塊（有 `renderedLines`）會出現在卡片列表裡，不再被
  `status !== "running"` 這個條件擋掉。
- 卡片頭在 running 中顯示持續更新的耗時，不顯示 exit code。
- 點卡片內容會呼叫 `term.focus()`。
- alt-screen（`isAlternateBuffer`）使用中，行為完全不變：xterm 的 DOM
  照樣顯示、撐滿可用高度，不會被這次的隱藏機制影響。
- 拿掉的：所有針對 `liveRows`／`liveTopRows`／`scrollToLine`／位移量的
  既有測試整批刪除或改寫（`TerminalView.windowsPromptAlign.test.tsx`、
  `TerminalView.windowsRunningResetOffset.test.tsx`、
  `TerminalView.remoteLiveHeight.test.tsx`、
  `RemoteTerminalView/index.test.tsx` 裡對應的幾個測試）。

## 風險與取捨

- **`readRenderedLines` 是線性掃描，不是遞增式的。** 節流用
  `requestAnimationFrame` 緩解「chunk 頻率」的問題，但單次呼叫的成本
  還是跟「這個指令目前已經印了多少行」成正比——對絕大多數互動情境
  （幾十到幾百行）完全沒問題，對真的印出數萬行的指令（`cat` 大檔案）
  執行中每一輪節流都要重新掃全部內容，會比現在「執行中完全不解析、
  結束時才解析一次」慢。這是用「執行中就能看到正確內容」換來的取捨，
  之前的機制在這種情況下的問題更嚴重（要嘛裁切看不到、要嘛整個空白）。
- **拿掉了 `onPromptStart`/`onUntrackedCommandBoundary` 這兩條訊號**，
  是這次判斷「不再需要」而整批移除的既有機制，不是本次功能的一部分
  ——如果之後發現這兩個訊號還有其他消費者依賴（目前查過沒有，只有
  `TerminalView.tsx`／`RemoteTerminalView.tsx` 自己在用），需要另外
  處理。
- **隱藏 xterm DOM 用 CSS 移到畫面外（不是 `display:none`）**，理由是
  `display:none` 的元素通常量不到真實尺寸，xterm 需要真正的像素尺寸
  才能正確計算字元格大小/回報 cols/rows。用 `position:absolute; left:
  -99999px` 保留版面尺寸但視覺上不可見，是這個 repo 已經用過的手法
  （IME 相關的 park 邏輯也是類似思路）。

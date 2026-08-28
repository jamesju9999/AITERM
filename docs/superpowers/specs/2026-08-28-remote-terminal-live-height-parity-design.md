# 遠端終端機即時窗格高度與外觀對等 Design Spec

## 背景與問題

`RemoteTerminalView`（遠端觀看者連線時看到的分頁）目前的畫面呈現策略，跟本機
`TerminalView` 完全不同：

- **本機終端機**：用 `FitAddon` 量測視窗容器的實際可用空間，算出能放下幾欄幾行，
  再呼叫 `resizePty` 告訴後端 shell「你現在的終端機大小是這麼寬」——shell 自己
  決定怎麼換行。視窗多寬，PTY 就用多寬，兩者永遠一致。即時畫面窗格
  （`.aiterm-live-frame`）的**高度**則是另一套獨立機制：JS 動態計算要顯示幾行
  （`MIN_LIVE_ROWS`=3 到 `MAX_LIVE_ROWS`=16），閒置時縮小、有指令在跑時撐高，
  指令完成變成卡片後收回——多出來的空間讓卡片自然往下延伸，整個畫面看起來
  「剛好貼合內容」。
- **遠端終端機**：xterm 的 `cols`×`rows` 是主控端說了算（`onShareViewerGranted`
  給的值，透過 `term.resize()` 套用），觀看端拿不到「反過來讓 PTY 配合自己視窗
  寬度」這個選項——那個 PTY 是主控端自己也在用的同一份，讓某個觀看者的視窗
  寬度去決定 shell 的實際寬度，會干擾主控端自己的畫面（多個觀看者視窗寬度不同
  時更沒有「該聽誰的」的答案）。目前的解法是`computeFittingFontSize`：量測目前
  字級下的字元格像素尺寸，線性外推出一個能讓整個 `cols`×`rows` 網格塞進容器
  可用空間的字級，動態縮放字體去湊。即時畫面區塊（`.aiterm-remote-terminal__screen`）
  在 CSS 上是 `flex: 1`——卡片區塊（上限 `max-height: 40%`）之外的所有空間全部
  由它吃掉，**不管有沒有指令在跑，都固定佔用這麼大的區域**。

使用者透過真機截圖回報兩個具體症狀：
1. 遠端終端機的提示字元區域沒有本機那種帶邊框、圓角的視覺樣式。
2. 遠端終端機的即時畫面區塊高度固定不變，提示字元下方留了一大片沒用到的
   空白，跟本機終端機「閒置縮小、忙碌撐高」的體驗不一致。

## 目標

讓 `RemoteTerminalView` 的即時畫面窗格外觀與高度行為，盡可能對齊
`TerminalView` 的既有機制——沿用本機已經驗證過的作法，而不是另外發明一套。

## 架構

### 拿掉自動縮放字體，改用固定字體 + 動態高度

- 移除 `computeFittingFontSize`、`recomputeFontSize`、掛在 `hostRef` 上的
  `ResizeObserver`，以及 `onShareViewerGranted`/`onFontChanged` 裡呼叫
  `recomputeFontSizeRef.current?.()` 的部分。
- 字體大小維持現有「掛載時讀 `localStorage` 使用者設定」這段程式碼不變
  （`aiterm-font-size`/`aiterm-font-family`），但**不再**被縮放邏輯覆蓋——使用者
  在設定裡調整字級/字型時，透過既有的 `aiterm:font-changed` 事件監聽套用新值
  即可，跟本機終端機的行為一致。
- `xterm` 的 `cols`/`rows` 依然照主控端在 `onShareViewerGranted`/resize 通知裡
  給的值，透過 `term.resize()` 套用——這部分維持不變，因為畫面內容本來就要跟
  主控端一致，不能因為觀看端視窗大小而改變 shell 實際看到的欄寬。

### 寬度：交給水平捲動

- 固定字體後，主控端的 `cols` 換算成實際像素寬度，可能比觀看端視窗窄或寬。
  這是觀看端架構上無法避免的限制（見上方「背景與問題」）——PTY 的實際寬度
  由主控端決定，觀看端只能選擇「縮小字體去湊」或「固定字體、寬度不夠就
  橫向捲動」，這次選後者。
- `.aiterm-remote-terminal__scroll` 現有的 `overflow: auto` 已經是為了類似情境
  準備的（原本的註解是「字級縮到最小仍塞不下時」），維持不變即可，不需要新的
  CSS 規則——拿掉自動縮放字體後，這條規則從「極端情況的後備」變成「處理寬度
  差異的常態手段」。

### 高度：比照 `TerminalView` 的動態即時窗格機制

- 新增 `liveRows` 狀態（`useState`，初始值 `MIN_LIVE_ROWS`），常數
  `MIN_LIVE_ROWS = 3`、`MAX_LIVE_ROWS = 16`，數值跟 `TerminalView.tsx` 完全一樣
  ——刻意不做成可設定的參數，保持兩邊行為一致才是這次修改的目的。
- **即時窗格的顯示邏輯**（判斷「現在有沒有指令在跑」）**不需要新增任何資料
  來源**：`RemoteTerminalView` 自己的 `useTerminalBlocks` 實例，本來就在監看
  同一份透過 `onShareViewerData` 收到的 PTY 位元組流——不管指令是觀看端自己
  透過 `WarpInput`/`submitCommand` 送出的，還是主控端自己打的、或另一個觀看者
  送的，同一份 OSC 133 標記都會被這個 hook 看到並正確處理（今天稍早完成的
  `recoverUntrackedCommand` 還原機制，讓「沒有本機追蹤區塊」的指令也能正確
  被追蹤成 `TerminalBlock`）。這代表 `blocks` 陣列裡最後一筆的 `status` 欄位，
  天然就反映「現在有沒有東西在跑」，跟本機終端機讀 `blocksRef.current` 的邏輯
  完全對稱。
- 具體邏輯（照抄 `TerminalView.tsx` 對應的兩段）：
  - 在 `onShareViewerData` 的 handler 裡，寫入 `term` 之後（用跟 Task 4-6 那次
    `appendOutput` race fix 相同的做法——包在 `term.write()` 的完成 callback
    裡，不要在呼叫之後就同步執行），檢查 `blocks` 陣列最後一筆的 `status` 是
    否為 `"running"`，是的話 `setLiveRows(MAX_LIVE_ROWS)`。
  - 新增一個 `useEffect`，依賴 `visibleBlockCount`（`blocks.filter((b) =>
    b.status !== "running" && b.renderedLines).length`，跟 `TerminalView.tsx`
    算法相同），變動時 `setLiveRows(MIN_LIVE_ROWS)`——一張新卡片完成渲染，
    代表這輪指令已經結案，窗格收回最小高度。
- **DOM 結構**：仿照 `TerminalView.tsx` 「外層包一層動態高度的 wrapper、內層
  xterm host 本身維持固定內部大小不變」的作法——`hostRef` 掛的
  `.aiterm-remote-terminal__scroll` 內部保持固定高度（不受 `liveRows` 影響，
  避免觸發 xterm 自己的 `ResizeObserver` 改變實際 row 數），外面再包一層
  `.aiterm-remote-terminal__live-frame`，用 `height: liveRows * cellHeightPx`
  決定實際露出幾行。`cellHeightPx` 的算法沿用 `TerminalView.tsx` 讀
  `_core._renderService.dimensions.css.cell.height` 這個 xterm.js 內部欄位
  的既有做法（同一個 escape hatch，這個 repo 已經有兩處先例）。

  **寫實作計畫時，這個 wrapper 的 inline style（`height`/`width`/`margin`/
  `boxSizing`/`overflow`）要直接對照 `TerminalView.tsx` 目前 `.aiterm-live-frame`
  那段 JSX 逐字核對抄過來**，不要憑印象重寫——那段程式碼裡有幾個已經除錯
  過的細節（`height` 用 `calc`扣掉 margin 造成的溢出量、`overflow: clip` 而不是
  `hidden` 避免瀏覽器把貼上/輸入的焦點捲出視野變空白），這些都是通用於任何
  「外層動態縮放高度、裡面裝一個 xterm host」的結構，不是本機終端機獨有的
  巧合，遠端這邊會遇到同樣的坑，必須照抄。`isAlternateBuffer`
  分支（TUI 全螢幕程式時 `overflow: visible`）**不用**照抄——見下方「範圍
  界定」，`RemoteTerminalView` 目前完全沒有處理這個狀態，這次也不新增。

### 卡片區塊與即時窗格合併成單一捲動容器

- 拿掉 `.aiterm-remote-terminal__blocks` 現有的 `max-height: 40%` 上限與它
  自己的 `overflow-y: auto`——改成跟 `TerminalView.tsx` 一樣，卡片列表與即時
  窗格放在同一個外層容器裡，容器本身才是唯一的捲動邊界（`overflow-y: auto`），
  卡片跟窗格各自自然佔用需要的高度，卡片可以無限往下累積。
- 新增一個 ref 掛在這個外層容器上，新增一個 `useEffect` 依賴
  `visibleBlockCount`，變動時呼叫 `scrollTo({ top: scrollHeight })` 捲到底部
  ——對應使用者這次額外提出的需求：指令完成、新卡片出現時，畫面要自動捲到
  最新的一筆，不需要使用者自己往下滑。跟 `TerminalView.tsx` 的
  `blockListRef` 用的是同一個手法。

### 視覺樣式：即時窗格加上邊框

- `.aiterm-remote-terminal__live-frame`（新的外層 wrapper）套用跟
  `TerminalView.css` 的 `.aiterm-live-frame` 完全相同的樣式：細邊框
  （`1px solid rgba(255, 255, 255, 0.08)`）、`border-radius: 8px`、
  `transition: height 0.12s ease-out`（高度變化時有平滑過場，不是瞬間跳動）。

## 對既有程式碼的影響

- `src/components/RemoteTerminalView/index.tsx`：
  - 移除 `computeFittingFontSize` 函式、`recomputeFontSize`/
    `recomputeFontSizeRef`、掛在 `hostRef` 上的 `ResizeObserver` effect。
  - `onShareViewerGranted` 的 handler 裡移除 `recomputeFontSizeRef.current?.()`
    呼叫；`onFontChanged` 裡同樣移除。
  - 新增 `liveRows` 狀態與對應的兩個 `useEffect`（撐高/收回、捲到底部）。
  - JSX 結構調整：卡片列表 + 即時窗格改成共用一個外層容器；即時窗格外面
    多包一層動態高度的 `.aiterm-remote-terminal__live-frame`。
- **DOM／CSS 結構調整前後對照**：

  現在：
  ```
  <div class="aiterm-remote-terminal">
    {banners}
    <div class="aiterm-remote-terminal__blocks">{cards}</div>      <!-- max-height:40%, 自己 overflow-y:auto -->
    <div class="aiterm-remote-terminal__screen">                   <!-- flex:1 -->
      <div class="aiterm-remote-terminal__scroll" ref={hostRef} /> <!-- overflow:auto -->
    </div>
    {aiUnsupported}
    <WarpInput />
  </div>
  ```

  改成：
  ```
  <div class="aiterm-remote-terminal">
    {banners}
    <div class="aiterm-remote-terminal__scroll-area" ref={scrollAreaRef}> <!-- 新增：flex:1, overflow-y:auto，唯一的捲動邊界 -->
      <div class="aiterm-remote-terminal__blocks">{cards}</div>          <!-- 不再有 max-height/自己的 overflow -->
      <div class="aiterm-remote-terminal__live-frame" style={{ height: liveHeightPx }}> <!-- 新增：邊框/圓角/過場動畫，取代原本 __screen 的角色 -->
        <div class="aiterm-remote-terminal__scroll" ref={hostRef} />     <!-- 不變：overflow:auto，內部固定尺寸不隨 liveRows 變動 -->
      </div>
    </div>
    {aiUnsupported}
    <WarpInput />
  </div>
  ```

- `src/components/RemoteTerminalView/index.css`：
  - 移除 `.aiterm-remote-terminal__blocks` 的 `max-height: 40%` 與
    `overflow-y: auto`。
  - 移除 `.aiterm-remote-terminal__screen` 這個 class（改用下面兩個新
    class 取代它原本的角色）。
  - 新增 `.aiterm-remote-terminal__scroll-area`：`flex: 1; min-height: 0;
    overflow-y: auto;`——取代原本 `__screen` 的 `flex: 1`，但範圍擴大到
    同時包住卡片跟即時窗格。
  - 新增 `.aiterm-remote-terminal__live-frame`：邊框
    （`1px solid rgba(255, 255, 255, 0.08)`）、`border-radius: 8px`、
    `transition: height 0.12s ease-out`，跟 `TerminalView.css` 的
    `.aiterm-live-frame` 完全相同。
  - `.aiterm-remote-terminal__scroll` 維持不變（`flex: 1; min-width: 0;
    overflow: auto;`）。

## 範圍界定

- **不處理**：讓觀看端的視窗寬度反過來影響主控端 PTY 的實際欄寬——這需要
  跟主控端協商、處理多觀看者衝突，是完全不同量級的功能，這次不碰。
- **不處理**：`isAlternateBuffer`（TUI 全螢幕程式，如 vim/htop）情境下的版面
  ——`RemoteTerminalView` 目前沒有處理這個狀態，維持現狀，不在這次範圍內。
- **共用**：`MIN_LIVE_ROWS`/`MAX_LIVE_ROWS` 這兩個常數目前分別定義在
  `TerminalView.tsx`跟（新增後的）`RemoteTerminalView/index.tsx`裡，這次不
  額外抽成共用模組——兩個檔案本來就有各自獨立的一份，抽公用模組是不相關
  的重構，不在這次目標範圍內。

## 測試策略

- 沿用專案既有測試檔案的建置方式（`RemoteTerminalView/index.test.tsx`），
  針對這次改動新增：
  1. 移除自動縮放字體後，字體大小應該直接反映 `localStorage` 設定值，不會
     被動態計算覆蓋。
  2. `liveRows` 在 `blocks` 最後一筆變成 `running` 時撐高到 `MAX_LIVE_ROWS`，
     在 `visibleBlockCount` 變動時收回 `MIN_LIVE_ROWS`。
  3. 新卡片出現（`visibleBlockCount` 變動）時，外層捲動容器的 `scrollTo`
     被呼叫、且引數是當時的 `scrollHeight`。
  4. 既有測試（例如 Resync 清空卡片、字型/主題同步）在這次改動後仍然通過，
     沒有被拿掉自動縮放字體的邏輯影響到。

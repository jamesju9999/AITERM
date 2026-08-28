# 遠端終端機工具列 Design

**Goal:** 在 `RemoteTerminalView` 頂部加一條跟本機 `TerminalView` 同樣風格的工具列，填補因為兩邊視窗頂部工具列高度不一致，而在遠端終端機分頁下方留下的空白區域，並讓遠端分頁在視覺上跟本機分頁更一致。

## 背景

同一個分支（`feat/remote-terminal-full-parity`）稍早完成「遠端終端機即時窗格動態高度」後，使用者發現全螢幕程式（例如 Claude Code CLI）在遠端分頁下方會留一大片空白。一開始懷疑是即時窗格高度算錯，但使用者自己發現真正原因：本機視窗頂部有一條工具列（分頁列＋連線狀態＋分享/書籤/Remote/Ask AI 按鈕），同樣的整體視窗高度下，本機能分給終端機內容區的空間本來就比沒有這條工具列的遠端分頁少——不是列數算錯，是兩邊工具列高度不一致。與其硬把即時窗格撐高去填補這個差距，使用者傾向直接讓遠端分頁也有一條對等的工具列，兩邊視覺結構一致，空白自然消失。

## 範圍

**加入：**
- 一條新的工具列，位置在 `RemoteTerminalView` 現有的「等待同意/唯讀/已結束」banner 之上、`.aiterm-remote-terminal__scroll-area` 之外。
- 左側：連線資訊文字（位址、連線狀態、模式、已連線時間，見下）。
- 右側：兩顆按鈕——「指令書籤」（真正可用）、「Ask AI」（視覺佔位）。

**明確不加：**
- 「分享」「Remote」按鈕——這兩個是本機（主控端）專屬的「建立/管理分享」功能，觀看端已經是被分享的一方，用不到。
- Terminal/Files 子分頁列——遠端分頁沒有檔案瀏覽功能，不需要這條列。
- Ask AI 真正呼叫 AI 的能力——遠端連線目前完全不支援 AI/Agent 指令（`handleWarpSubmit` 對 `/ai`、`/agent` 開頭直接擋掉，顯示 `aiUnsupported` 提示），這次只做視覺對齊，不擴大成新功能。

## 視覺與樣式

直接沿用 `TerminalView.css` 既有的 `.aiterm-status` class（本機工具列外層容器的樣式：背景色、padding、左右兩區 flex 排版），不另外定義一套新樣式——這樣新工具列的字體、間距、按鈕外觀會自動跟本機分頁一致，也不會因為兩邊各自維護一份相近但微妙不同的 CSS 而之後跑掉。按鈕沿用既有的 `.aiterm-btn.aiterm-btn--secondary.aiterm-btn--sm`（書籤）與 `.aiterm-btn.aiterm-btn--primary.aiterm-btn--sm`（Ask AI，主要色一樣沿用本機同款）。

## 左側：連線資訊文字

依 `phase`（`RemoteTerminalView` 既有的 `waiting | live | ended` 三態）顯示不同文字，格式統一為 `AITerm · 遠端終端機 {hostLabel} · {狀態片語}`：

| phase | 狀態片語 |
|---|---|
| `waiting` | 沿用既有翻譯鍵 `remote_terminal_waiting_approval`（「等待對方同意…」） |
| `live`，`mode === "control"` | `已連線 {elapsed} · 控制模式` |
| `live`，`mode === "read_only"` | `已連線 {elapsed} · 唯讀`（唯讀沿用既有翻譯鍵 `remote_terminal_read_only`） |
| `ended` | 新翻譯鍵 `remote_terminal_toolbar_ended`（「連線已結束」） |

`{hostLabel}`：連線當下輸入的「host:port」字串，跟現在視窗標題「遠端終端機：10.10.41.1:50281」同一份資料（`TerminalApp.tsx` 裡 `ConnectDialog.onConnected` 回傳的 `hostLabel`，目前已經存在 `tab.remoteHostLabel`，只是沒有傳進 `RemoteTerminalView`）——這次新增一個 `hostLabel` prop 把它傳進來。

`{elapsed}`：連線時間，從 `phase` **第一次**變成 `live` 的那一刻開始計時（不是從 `waiting` 開始算，等待核准的時間不算連線時間），每秒更新一次。格式比照 `TerminalBlockCard.tsx` 既有的 `formatDuration`（同樣邏輯但獨立寫一份，不跨檔案匯出私有函式），並延伸支援小時：

- `< 60` 秒：`12s`
- `< 3600` 秒：`3m45s`
- `>= 3600` 秒：`1h05m`（分鐘補零到兩位；連線可能開很久，本機那份 `formatDuration` 只給指令執行用，通常不會超過一小時，不需要處理小時，這裡的用途不同要分開處理）

新增翻譯鍵：
- `remote_terminal_toolbar_connected_prefix`：「已連線」/ "Connected" —— 拼接後變成「已連線 3m45s」
- `remote_terminal_toolbar_control_mode`：「控制模式」/ "Control mode"
- `remote_terminal_toolbar_ended`：「連線已結束」/ "Connection ended"

## 右側：兩顆按鈕

**指令書籤**（真正可用）：
- 沿用 `bookmarks_title`／`term_bookmark_tooltip` 翻譯鍵、`CommandBookmarksPicker` 元件、`addBookmark`（`RemoteTerminalView` 已經在用 `addBookmark` 了，只是目前只有卡片的書籤按鈕會呼叫）。
- 新增本地 state `bookmarksOpen`，點按鈕開啟 `<CommandBookmarksPicker onSelect={...} onClose={...} />`。
- `onSelect(cmd)`：跟 `TerminalView.tsx` 完全一樣，`window.dispatchEvent(new CustomEvent("warp-fill-command", { detail: { cmd } }))`——`WarpInput` 監聽的是這個全域事件，不管是哪個父元件render 它都會生效，不需要改 `WarpInput` 本身。

**Ask AI**（視覺佔位）：
- 沿用 `term_ai_helper_tooltip` 翻譯鍵、`SparklesIcon`。
- `onClick`：直接呼叫既有的 `setAiUnsupported(true)`（`RemoteTerminalView` 已經有這個 state，`handleWarpSubmit` 擋下 `/ai`/`/agent` 時就是設這個），顯示既有的 `aiUnsupported` banner（`remote_terminal_ai_unsupported`：「AI 指令目前不支援於遠端分頁。」），不呼叫任何 AI API。

## 資料流變更

- `TerminalApp.tsx`：`<RemoteTerminalView ... />` 呼叫處新增 `hostLabel={tab.remoteHostLabel ?? ""}` prop。
- `RemoteTerminalView` 的 `Props` interface 新增 `hostLabel: string`。
- 新增 `connectedAtRef`（`useRef<number | null>(null)`）與一個新的 `useEffect`，依賴陣列是 `[phase.kind]`：

  ```ts
  useEffect(() => {
    if (phase.kind === "live" && connectedAtRef.current === null) {
      connectedAtRef.current = Date.now();
    }
  }, [phase.kind]);
  ```

  用 `=== null` 的 ref guard，不是「偵測從非 live 變成 live 的那一刻」——`phase.kind` 變成 `"live"` 之後，後續的控制權變更（`onShareViewerControlChanged`）會用不同的 `mode` 再次呼叫 `setPhase({kind:"live", ...})`，但 `phase.kind` 這個依賴值本身沒變，這個 effect 不會重跑，`connectedAtRef` 自然只會被設定一次，不需要額外分辨「是第一次進 live 還是後續的 mode 變更」。
- 新增 `elapsedMs` state，用 `setInterval(1000)` 在 `phase.kind === "live"` 期間持續更新，元件卸載或連線結束時清掉 interval。

## 測試

- 新增/更新 `RemoteTerminalView/index.test.tsx` 測試：
  1. `waiting` 階段工具列顯示等待文字，不顯示連線時間。
  2. `live` + `control` 顯示「控制模式」與遞增的連線時間（用 `vi.useFakeTimers()` 推進時間驗證秒數變化）。
  3. `live` + `read_only` 顯示「唯讀」。
  4. `ended` 顯示「連線已結束」。
  5. 點擊「指令書籤」開啟 picker，選擇後觸發 `warp-fill-command` 事件（可用 `window.addEventListener` spy 驗證，或直接檢查 `WarpInput` 的輸入框內容被填入）。
  6. 點擊「Ask AI」顯示既有的 `aiUnsupported` 提示文字，且不觸發任何 IPC 呼叫。

## 範圍界定（不在這次做的）

- 不做「觀看端也能發起分享」或任何主控端專屬功能的對等實作。
- 不做遠端連線的 AI/Agent 真正串接。
- 不處理 `hostLabel` 為空字串（例如舊版本升級後遺留、理論上不該發生）以外的邊角案例——空字串就照樣顯示，不特別防呆，因為 `ConnectDialog` 端已經保證連線成功時一定有值。

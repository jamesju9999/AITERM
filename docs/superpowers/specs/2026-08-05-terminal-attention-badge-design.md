# 側邊欄終端機提示點 — 設計

日期：2026-08-05
狀態：已核可，待寫實作計畫

## 問題

側邊欄的終端機分頁只顯示圖示與標題，沒有任何狀態資訊。一旦分頁不是當前 active 的那個，裡面發生什麼事就完全看不到——不管是 CLI 工具停在那裡等你回答、長指令跑完了、還是指令直接掛掉。使用者必須自己記得要回去看，或者一個一個切過去確認。

Mail 分頁已經有這類提示（未讀數字 pill、連線失敗紅色 `!`），設定按鈕也有更新橘點。終端機分頁是唯一沒有的。

## 範圍

**含：** 三種狀態（等待輸入／完成／失敗）的偵測、側邊欄彩色提示點、app 失焦時的桌面通知。

**不含：**

- 「任何新輸出就亮燈」的活動指示。使用者明確排除——提示要代表**有意義的事件**，不是活動指示燈。
- 輸出靜默的啟發式判斷（見〈為什麼不用其他偵測方式〉）。
- 「跳到下一個等待中分頁」的快捷鍵。可以之後接在同一個狀態上，不屬於第一版。

## 狀態模型

`Tab` 介面（`src/components/TabBar/index.tsx`）新增：

```ts
/** 非 active 的終端機分頁發生了值得注意的事。
 *  只存在記憶體，不進 localStorage——重開 app 後這些事件已經沒有意義。 */
attention?: "waiting" | "done" | "failed";
```

**規則：後到的事件覆蓋前一個。** 不做優先權表。

每個事件代表終端機**當下的最新狀況**。若 bell 之後指令真的跑完了，那它就是跑完了，綠燈是正確的。優先權表（例如 waiting 永遠壓過 done）會製造「明明已經結束卻還亮著橘燈」的假警報，而假警報會讓使用者永久忽略這個提示。

## 偵測

### 訊號來源

| 狀態 | 訊號 | 位置 |
|---|---|---|
| `waiting` | `term.onBell()` | `TerminalView.tsx`，目前未接 |
| `done` | OSC 133 `D`，`exitCode === 0` | `useTerminalBlocks.ts:149`，已存在 |
| `failed` | OSC 133 `D`，`exitCode !== 0` | 同上 |

`useTerminalBlocks` 新增一個 `onCommandSettled?: (exitCode: number) => void`，在既有的 `finalizeBlock` 呼叫旁邊觸發。不改動 OSC handler 現有的任何邏輯——包括 Windows/ConPTY 的延遲 clear 與 Ctrl+L 重新同步（那段有明確的 root-cause 註解，是踩過坑修好的）。

### 為什麼不用其他偵測方式

**輸出靜默啟發式**（在 alternate buffer 中輸出停止 N 秒即視為等待輸入）能涵蓋不敲 bell 的 TUI，但會把跑很久的編譯、卡住的下載、開著沒動的 vim 全部誤判成「等你回答」。誤判過幾次之後使用者就會開始無視這個燈，之後再怎麼修都救不回來。

**輸出文字樣式比對**（掃 `[y/N]`、`? `、`Do you want` 等）在多語系下會漏、需要先剝掉 ANSI 跳脫字元、且每個 PTY chunk 都要跑正則。脆弱又貴。

原則：**寧可漏報，不可誤報。**

### 狀態歸屬

`TerminalView` 新增 prop：

```ts
/** 這個分頁發生了需要使用者注意的事。TerminalView 一律回報，
 *  「這個分頁是不是 active」由 TerminalApp 判斷——避免 isActive 在
 *  xterm/PTY 事件的 closure 裡變 stale。 */
onAttention?: (kind: "waiting" | "done" | "failed") => void;
```

形狀比照既有的 `onAgentProgress`。`TerminalApp` 端把事件分派到兩個**互相獨立**的去處：

```tsx
onAttention={(kind) => {
  // 提示點：條件是「這個分頁不是使用者正在看的那個」
  if (activeIdRef.current !== tab.id) {
    setTabs(prev => prev.map(t => t.id === tab.id ? { ...t, attention: kind } : t));
  }
  // 桌面通知：條件是「app 沒有 focus」——與是不是 active 分頁無關
  maybeNotify(tab.title, kind);
}}
```

這兩個條件不能合併成一個提早 return。app 失焦時，**當前 active 分頁**的終端機停下來等使用者回答，正是最需要桌面通知的情況——使用者人不在 app 前面，「它是 active 分頁」不代表有人在看。反過來，提示點對 active 分頁沒有意義：使用者一切回來就會直接看到終端機內容。

`activeIdRef` 已存在於 `TerminalApp.tsx:81`。用 ref 而非 `activeId` state 是因為這個 callback 會被 xterm 與 PTY 事件監聽器持有，直接讀 state 會拿到 stale 值——這是本專案反覆出現的問題，既有程式碼一律用 ref 解。

### 清除

一個以 `activeId` 為依賴的 effect，把切過去的那個分頁 `attention` 設回 `undefined`。

使用者選定的規則是「切到該分頁就清掉」：看過了就算讀取，與 Mail 未讀的邏輯一致。當前 active 的分頁因此永遠不會有提示點。

## 側邊欄呈現

渲染在既有的 `.aiterm-tab-icon` 內（`TabBar/index.tsx:176` 已經是 `position: relative`）。Mail 的兩個 badge 只出現在 mail 分頁、這個只出現在 terminal 分頁，位置不會互撞。

| 狀態 | 顏色 | 形狀 | 動態 |
|---|---|---|---|
| `waiting` | 橘 `#f59e0b` | 圓 | 緩慢脈動 |
| `done` | 綠 `#22c55e` | 圓 | 靜態 |
| `failed` | 紅 `#ef4444` | **方**（`border-radius: 2px`） | 靜態 |

只有 `waiting` 會動，因為只有它需要使用者**現在**做事。脈動動畫包在 `@media (prefers-reduced-motion: reduce)` 裡關掉。

**`failed` 用形狀而非只用顏色與 `done` 區分。** 紅綠是最典型的色盲失效組合，約 8% 的男性分不出 `#22c55e` 與 `#ef4444`——而「成功 vs 失敗」正是這個功能最重要的一個區別。`aria-label` 幫不了這群人：他們看得見，不用讀屏器，`aria-label` 也不會產生 tooltip。而且在 `prefers-reduced-motion` 之下三種狀態會全部退化成「一個彩色圓點」。

同一支檔案裡已有先例：`.mail-connection-badge`（`TabBar/index.css:224-226`）的註解寫明它用紅色是因為「回報的是故障而非數量」，然後 `index.tsx` 仍然在裡面放了一個字面的 `!` 字元——既有程式碼本來就不信任單靠顏色傳達故障。

每個點都有 `role="img"` 與 i18n 的 `aria-label`（en / zh-TW，字串放 `src/lib/i18n.ts`），比照 Mail badge 現有做法。點本身不含文字：語意由「顏色 + 形狀」給看得見的人，由 `aria-label` 給讀屏器使用者。

側邊欄折疊與展開時都顯示同一個點，位置不變（點錨定在圖示上，不在標題列上）。

## 桌面通知

**條件：app 沒有 focus，且狀態是 `waiting` 或 `failed`。**

- 使用者在看 AITerm 時，側邊欄的點就夠了，不需要桌面通知。
- 指令單純跑完（`done`）不緊急，不發通知。只有真正需要動手的兩種狀態才發。
- **與「是不是 active 分頁」無關**——active 分頁在 app 失焦時同樣會發通知。理由見〈狀態歸屬〉。

焦點判斷用 `getCurrentWindow().isFocused()`（Tauri 視窗狀態），不用 `document.hasFocus()`。

通知內容：標題為分頁名稱，內文為「等待你的回應」／「指令失敗」（i18n）。

### 權限取得共用

`tauri-plugin-notification` 已經安裝（`src-tauri/Cargo.toml:63`、`package.json`），`src-tauri/capabilities/default.json:23` 已授權 `notification:default`。不需要改設定。

`useMailSync.ts:62` 已有 `ensureNotificationPermission`，用模組內單一 promise 記憶化。該處註解（`useMailSync.ts:54`）明確說明這個記憶化的目的是避免同時跳出多個權限請求、以及重複打擾已經拒絕過的使用者。

若在本功能另寫一份獨立的記憶化，就會製造出那條註解正在防的 bug（兩個獨立 promise 各自請求一次）。因此抽出 `src/lib/notifyPermission.ts`，模組層級單一 promise，`useMailSync` 與本功能共用。

這是本功能正確運作的必要條件，不是順手重構——改動範圍限定為「把既有函式搬到共用模組並改由兩處引用」，行為不變。

## 測試

**`TabBar/index.test.tsx`**
- 三種 `attention` 值各自渲染對應的 class 與 aria-label。
- 沒有 `attention` 時不渲染任何點。
- 非 terminal 型別的分頁即使有 `attention` 也不渲染。
- 提示點的 class 與 Mail 的 unread / connection badge 都不相同（比照該檔第 91 行既有測試的精神：避免兩種語意不同的標記在視覺上被混為一談）。

**`TerminalApp`**
- `onAttention` 不會在當前 active 的分頁上設出提示點。
- 切換到某個有 `attention` 的分頁會清掉它。

**通知條件**
抽成純函式（輸入：是否 focus、狀態；輸出：是否發送），直接單元測試。避開在測試環境模擬 Tauri 視窗焦點。必測案例：**app 失焦 + active 分頁 + `waiting` → 應發送**（提示點與通知的條件互相獨立，這是最容易在實作時被合併掉的一條）。

## 已知限制

**全螢幕 TUI 期間沒有 OSC 133 訊號。** Claude Code、vim、lazygit 這類程式在 alternate buffer 執行期間，shell 只把它們視為「一個還在跑的指令」，OSC 133 `D` 要等到程式退出才會發出。那段時間唯一的提示來源是 bell。

**因此「Claude Code 問問題時會不會亮橘燈」取決於它有沒有敲 bell**，那是該工具自身的通知設定，AITerm 無法控制。第一版接受這個漏報。

**macOS 通知無法在 `tauri:dev` 下驗證。** 依既有紀錄，dev 模式下通知會以「終端機」的身分送出且錯誤被吞掉，桌面版的 `requestPermission` 是 no-op。通知這部分必須出正式 build 才能確認。側邊欄的提示點則可以在 dev 下直接驗證。

## 跨平台

三種訊號（xterm bell、OSC 133、Tauri 通知）在 macOS / Windows / Linux 上行為一致，沒有平台專屬 API。Windows 的 OSC 133 路徑走既有的 ConPTY 分支，本功能只在其旁邊多加一個 callback，不改變該分支的時序。

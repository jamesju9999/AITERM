# Agent 分頁關閉確認設計

日期：2026-08-25
狀態：設計完成，待實作

## 問題

Agent（程式庫助手）分頁的對話內容完全沒有持久化——`useCodeAssistant` 的 `messages` 是純 `useState`（`src/hooks/useCodeAssistant.ts:39`），沒有任何 localStorage 或後端存檔。使用者點下分頁上的 `✕`，整段跟 AI 的對話就**永久消失**，沒有任何確認、也沒有還原路徑。

LoopStudio 已經針對同樣的風險做了關閉確認，但那份實作是分頁內部私有的：判斷條件、Modal 標記、CSS class（`ls-close-*`）、i18n 字串全部綁在 `src/components/LoopStudio/`。Agent 分頁要有同樣的保護，必須決定是複製一份還是抽出共用。

## 現況調查（設計的事實基礎）

| 事實 | 位置 |
|---|---|
| `TerminalApp` 已有 close guard 機制：`closeGuardsRef: useRef<Map<string, () => Promise<boolean>>>`，配 `registerCloseGuard` / `unregisterCloseGuard` 兩個 `useCallback` | `src/components/TerminalApp.tsx:87-93` |
| `handleCloseTab` 在移除分頁**之前**先 `const guard = closeGuardsRef.current.get(id)`，有 guard 就 `await guard()`，回傳 `false` 就直接 `return` 不關 | `src/components/TerminalApp.tsx:301-306` |
| `✕` 與 `Ctrl+W` 兩條關閉路徑**都**收斂到同一個 `handleCloseTab`，不需要各自處理 | `src/components/TerminalApp.tsx:417-421`（Ctrl+W）、`:497`（`onClose={handleCloseTab}`） |
| `TabBar` 對**每一個**分頁都渲染 `✕`（只看 `isSidebarOpen`，不看是否為當前分頁），點下去直接 `onClose(tab.id)` | `src/components/TabBar/index.tsx:413-424` |
| 非當前分頁的內容容器是 `visibility: hidden` + `pointerEvents: none` + `zIndex: -1`（刻意不 unmount，否則 xterm.js resize 會 crash） | `src/components/TerminalApp.tsx:544-548` |
| LoopStudio 的 guard 回傳 `new Promise<boolean>`，把 `resolve` 存進 `closeResolveRef` 後 `setShowCloseConfirm(true)`；使用者選擇時 `handleCloseConfirm` 才 resolve | `src/components/LoopStudio/index.tsx:178-199`、`:379-383` |
| LoopStudio 的 Modal 標記與樣式：`.ls-close-overlay`（`position: absolute; inset: 0`）、`.ls-close-dialog`、`-title`、`-body`、`-actions`；按鈕已改用全域的 `aiterm-btn` / `aiterm-btn--secondary` / `aiterm-btn--danger-solid` | `src/components/LoopStudio/index.tsx:387-421`、`src/components/LoopStudio/styles.css:1052-1095` |
| `.ls-root` 與 `.ca-view` 皆**未**設定 `position`——overlay 的定位基準其實是 `TerminalApp` 那個 `position: absolute` 的分頁容器，正好覆蓋分頁區域 | `src/components/LoopStudio/styles.css:1`、`src/components/CodeAssistantView/styles.css:2`、`src/components/TerminalApp.tsx:539` |
| LoopStudio 的觸發條件：`loop.isRunning`，或 `isDirtyRef.current && (roster.goal.trim() !== "" \|\| roster.subAgents.length > 0)` | `src/components/LoopStudio/index.tsx:181-196` |
| `CodeAssistantView` 目前只收 `isActive` 一個 prop，沒有 `tabId`，也沒接任何 guard | `src/components/TerminalApp.tsx:582`、`src/components/CodeAssistantView/index.tsx:43` |
| `useCodeAssistant` 對外曝露 `messages` / `isStreaming`，沒有 abort/cancel API；但**已有** `mountedRef` 保護，事件 listener 進來時先檢查 `if (!mountedRef.current) return` | `src/hooks/useCodeAssistant.ts:39-50`、`:76`、`:157`、`:178` |
| 現有 i18n 字串命名慣例為 `ls_close_title_running` 等前綴式 key，中英兩份各自維護 | `src/lib/i18n.ts:600-607`（zh-TW）、`:1871-1872`（en） |

**兩個由調查得出、會改變設計的結論：**

1. **不需要為串流中關閉加 abort API。** `useCodeAssistant` 已有 `mountedRef` 保護，unmount 後事件會被忽略，不會 setState-after-unmount。後端請求跑完即棄，加中止機制是額外範圍。
2. **「點非當前分頁的 ✕」是既有的隱性缺陷。** `✕` 每個分頁都有，但非當前分頁是 `visibility: hidden` + `pointerEvents: none`。若 guard 在這種分頁裡渲染 Modal，使用者**看不見也點不到**，`handleCloseTab` 會 await 一個永遠不會 resolve 的 Promise，分頁默默關不掉。LoopStudio 現在就有這個缺陷，只是尚未被踩到。本設計必須一併解決。

## 範圍

**含：**

- 新增共用元件 `CloseConfirmDialog`（純呈現，不知道 close guard 存在）
- LoopStudio 改用共用元件；**判斷條件與文案完全不動**，只換渲染層
- Agent（`code-assistant`）分頁接上 close guard，條件為「執行中或已有對話」
- `handleCloseTab` 在呼叫 guard 前，若目標不是當前分頁則先切過去

**不含：**

- 其他九種分頁的關閉確認（終端機、版本控制、資料庫、知識庫、郵件等維持現狀）
- Agent 對話的持久化／還原（那是另一個題目；本設計只避免**誤觸**造成的遺失）
- 為 `useCodeAssistant` 加 abort/cancel API（見上方結論 1）
- 「輸入框有未送出文字」也算未完成內容（明確排除，避免偵測條件與測試膨脹）
- 改變 `✕` 的顯示規則或 `Ctrl+W` 的行為

## 設計

### 單元 1：`CloseConfirmDialog`（新增）

位置：`src/components/CloseConfirmDialog/index.tsx` + `styles.css`

純呈現元件，無內部狀態，不知道 close guard 的存在。介面：

```ts
interface CloseConfirmDialogProps {
  title: string;
  body: React.ReactNode;   // 允許多行／<br />，沿用 LoopStudio 現有用法
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}
```

樣式從 `LoopStudio/styles.css:1052-1090` 原樣搬移，class 前綴由 `ls-close-` 改為 `aiterm-close-`。**`position: absolute; inset: 0` 必須保留**。

已查證：`.ls-root` 與 `.ca-view` 皆**未**設定 `position`，overlay 實際是相對於 `TerminalApp.tsx:539` 那個 `position: absolute` 的分頁容器定位——正好覆蓋分頁區域。因此兩個消費端都**不需要**額外補 `position: relative`。

按鈕沿用既有的全域 `aiterm-btn` / `aiterm-btn--secondary` / `aiterm-btn--danger-solid`，不自訂。

### 單元 2：LoopStudio 改用共用元件

只替換 `index.tsx:387-421` 的 JSX，改為傳 props 給 `CloseConfirmDialog`。`registerCloseGuard` 的 effect（`:178-199`）、`handleCloseConfirm`（`:379-383`）、所有 `t.ls_close_*` 文案**一字不動**。

`styles.css:1052-1095` 的 `.ls-close-*` 規則刪除（含兩行說明按鈕已被取代的註解）。

### 單元 3：Agent 分頁接上 guard

`CodeAssistantView` 新增三個 optional props，型別與 `LoopStudio/index.tsx:64-65` 一致：

```ts
tabId?: string;
registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
unregisterCloseGuard?: (tabId: string) => void;
```

`TerminalApp.tsx:582` 對應傳入 `tabId={tab.id}` 與兩個 guard 函式。

註冊邏輯（比照 LoopStudio 的形狀）：

```
若 !isStreamingRef.current && messagesCountRef.current === 0
  → Promise.resolve(true)        // 全新空白分頁，直接關，不打擾
否則
  → new Promise(resolve => { closeResolveRef.current = resolve;
                             setShowCloseConfirm(true); })
```

文案分兩種情況，新增 i18n key（中英各一份，沿用 `ca_` 前綴慣例）：

| 情況 | 標題 | 內文 |
|---|---|---|
| `isStreaming` | AI 正在回應中 | 關閉分頁會中斷這次回應，且整段對話不會保留。 |
| 僅有對話紀錄 | 對話尚未保存 | 這個分頁的對話沒有存檔，關閉後將永久遺失。 |

按鈕文字：取消＝「取消（返回對話）」、確認＝「關閉並捨棄」。

### 單元 4：`handleCloseTab` 先切換再問

`TerminalApp.tsx:301-306` 改為：

```ts
const guard = closeGuardsRef.current.get(id);
if (guard) {
  // 非當前分頁是 visibility:hidden + pointerEvents:none，確認框畫在裡面
  // 會看不見也點不到，await 就永遠不會 resolve。先切成當前分頁再問。
  if (activeIdRef.current !== id) setActiveId(id);
  const canClose = await guard();
  if (!canClose) return;
}
```

**只在有 guard 時才切換**——沒有 guard 的分頁維持原本「直接關掉、不切換」的行為，避免無謂的閃爍。

用 `setActiveId` 而非 `selectTab`：後者內部也呼叫 `setTabs`，而 `handleCloseTab` 稍後同樣要 `setTabs`，`:310-312` 的既有註解已警告過巢狀呼叫的問題。

時序上安全：`setActiveId` 與 guard 內部的 `setShowCloseConfirm(true)` 在同一個 tick 內被 React 批次處理，Modal 渲染時該分頁已經是 `visible`。

## 資料流

```
✕（任何分頁）或 Ctrl+W（當前分頁）
   └→ handleCloseTab(id)
        ├─ 無 guard → 直接進入移除分頁的 setTabs
        └─ 有 guard → 先 setActiveId(id)（若不是當前分頁）
             └→ await guard()
                  ├─ 條件不成立 → resolve(true) → 移除分頁
                  └─ 條件成立   → 顯示 CloseConfirmDialog，Promise 掛著
                       ├─ 使用者按「關閉並捨棄」→ resolve(true)  → 移除分頁
                       └─ 使用者按「取消」      → resolve(false) → 不關，停在該分頁
```

## 實作陷阱（必須在實作時處理）

1. **guard 閉包會抓到過期的 state。** 註冊當下 `messages` 是空的；若 guard 直接閉包捕捉 `messages`，之後談了十輪，guard 仍以為分頁是空的而直接放行——**這個功能會完全靜默失效，而且沒有任何錯誤訊號**。必須用 `useRef` 同步最新的 `isStreaming` 與 `messages.length`，guard 只讀 ref。這是本功能最容易出錯的一點，測試必須專門釘住它。

2. **effect 的 cleanup 必須 unregister。** 比照 `LoopStudio/index.tsx:198`，否則分頁關閉後 Map 裡留下指向已 unmount 元件的 guard。

3. **重構 LoopStudio 前先補行為測試。** 單元 2 動到的是目前正常運作的功能，必須先有測試釘住「running 時跳確認」「dirty 時跳確認」「乾淨時直接關」「取消回傳 false」，重構後仍須全綠。

## 測試

**`CloseConfirmDialog`（新增）**
- 渲染傳入的 title / body / 兩個按鈕文字
- 點確認呼叫 `onConfirm`、點取消呼叫 `onCancel`，且各自只呼叫一次

**Agent guard（新增）**
- 空白分頁：guard 直接 resolve `true`，**不**顯示對話框
- 有對話紀錄：顯示對話框，Promise 保持 pending
- 串流中：顯示對話框，且標題／內文為串流版本
- 按「關閉並捨棄」→ resolve `true`；按「取消」→ resolve `false`
- **狀態新鮮度**：註冊 guard 後才產生對話，再呼叫 guard 仍須跳確認（釘住陷阱 1）
- unmount 後有呼叫 `unregisterCloseGuard`

**`handleCloseTab` 切換行為（新增）**
- 對非當前分頁且**有** guard：呼叫 guard 前 `activeId` 已變成該分頁
- 對非當前分頁且**無** guard：`activeId` 不變，直接關閉

**LoopStudio（重構保護，先寫後改）**
- 上述四個既有行為情境，重構前後皆須通過

## 風險

| 風險 | 緩解 |
|---|---|
| 動到目前正常運作的 LoopStudio | 先補行為測試再重構；判斷邏輯與文案完全不動，只換渲染層 |
| `aiterm-close-*` 樣式搬移後外觀跑掉 | CSS 規則原樣搬移不改數值；LoopStudio 重構後以實機確認外觀 |
| 使用者覺得多一步很煩 | 條件限縮在「執行中或已有對話」，全新空白 Agent 分頁維持一鍵關閉 |
| 共用元件搬到新位置後 overlay 定位基準改變 | 定位基準是分頁容器而非消費端根元素（已查證），搬移不影響；仍以實機檢視兩個分頁的外觀驗收 |

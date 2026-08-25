# 終端機分頁關閉確認 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 終端機分頁裡有指令在跑或 Agent 任務進行中時，按 `✕` / `Ctrl+W` 先跳確認，避免誤關中斷正在進行的工作。

**Architecture:** 沿用既有的 close guard 機制與 `CloseConfirmDialog` 共用元件（前一份計畫已建好）。`TerminalView` 自行註冊 guard，因為判斷所需的 `agentMission.active` 只存在於它內部。同時修掉 `handleRouteHintPick` 那條會因此變成迴歸的路徑。

**Tech Stack:** React 19 + TypeScript、Vitest + React Testing Library + jsdom。

前一份設計：`docs/superpowers/specs/2026-08-25-tab-close-confirm-design.md`

---

## 背景：為什麼 Task 3 是必要的而不是加分題

`TerminalApp.tsx:348-353` 有一段**寫給未來的警告**：

> 這裡刻意不 await：終端機分頁目前沒有註冊任何 close guard（只有 LoopStudio 有），所以 handleCloseTab 對它是同步跑完的。
> 但如果哪天有人幫 TerminalView 加上 close guard，這行就會變成真的非同步，於是新分頁會在使用者還沒回答確認對話框時就先開出來。屆時要改成 await，而且要處理「使用者取消關閉」的情況——那時不應該開新分頁。

本計畫 Task 2 做的正是「幫 TerminalView 加上 close guard」，所以 Task 3 **必須一起做**，否則是本次親手造成的迴歸。

而且這不是邊緣案例：`routeIntent.ts:19` 的 `fallbackRoute` 在 AI 路由失敗時**一定**會開一個帶 mission 的終端機分頁，而 RouteHint 的「換成…」存在的目的就是讓使用者從那個分頁切走——必然踩到。

---

## 現況調查（事實基礎）

| 事實 | 位置 |
|---|---|
| 「指令執行中」訊號：`blocks[blocks.length - 1]?.status === "running"`，已存在於一個回報給外層的 effect | `src/components/TerminalView.tsx:372-375` |
| 「Agent 任務進行中」訊號：`agentMission.active`（`AgentMission` 介面的欄位） | `src/hooks/useAgentMission.ts:4-15`、`src/components/TerminalView.tsx:253` |
| `TerminalView` **已經**收 `tabId` prop | `src/components/TerminalView.tsx:192` |
| `TerminalView` 主體**沒有**提前 return（`:1599` 才是唯一的 return），hooks 位置限制寬鬆 | `src/components/TerminalView.tsx:1599` |
| `TerminalView` 根 div 有 `position: relative`——overlay 會貼著終端機區域（與 `.ca-view`/`.ls-root` 不同，那兩個是靠分頁容器定位，但結果一樣正確） | `src/components/TerminalView.tsx:1600-1607` |
| `handleCloseTab` 目前回傳 `Promise<void>`，guard 否決時是裸 `return` | `src/components/TerminalApp.tsx:302-309` |
| `handleCloseTab` 三個呼叫端：`handleRouteHintPick`（需要回傳值）、`Ctrl+W`、`TabBar onClose`（後兩者忽略回傳值） | `src/components/TerminalApp.tsx:354, 424, 500` |
| 「Agent: xxx」標題的分頁是 `type: "terminal"`，由協調 MCP 生成 | `src/components/TerminalApp.tsx:228-234` |
| 共用元件 `CloseConfirmDialog` props：`{ title, body, confirmLabel, cancelLabel, onConfirm, onCancel }`，自帶樣式 | `src/components/CloseConfirmDialog/index.tsx` |

---

## 範圍

**含：**
- `TerminalView` 註冊 close guard：指令執行中 **或** Agent 任務進行中就跳確認
- `handleCloseTab` 改回傳 `Promise<boolean>`
- `handleRouteHintPick` 改成 `await`，且使用者取消時**不開新分頁**
- 6 個 i18n key（中英各一份）

**不含：**
- 閒置終端機（只有 scrollback、沒有進行中的工作）——使用者已明確確認不攔
- 其他分頁類型
- 前一份計畫留下的待辦（i18n 語系漂移守衛、LoopStudio dirty 分支測試、Escape/focus trap）

---

## Task 1：新增終端機確認框的 i18n 字串

**Files:** Modify `src/lib/i18n.ts`

- [ ] **Step 1: 加入 zh-TW 字串**

搜尋 zh-TW 區塊裡的 `terminal_tab:`，在該行**之後**插入（**不要**盲目相信行號，用搜尋定位）：

```ts
    term_close_title_mission: "Agent 任務進行中",
    term_close_title_running: "指令執行中",
    term_close_body_mission: "關閉分頁會中止這個 Agent 任務，已完成的步驟不會保留。",
    term_close_body_running: "關閉分頁會中止正在執行的指令。",
    term_close_cancel: "取消（繼續執行）",
    term_close_discard: "關閉並中止",
```

- [ ] **Step 2: 加入 en 字串**

在**英文區塊**的 `terminal_tab:` 之後插入：

```ts
    term_close_title_mission: "Agent task in progress",
    term_close_title_running: "Command still running",
    term_close_body_mission: "Closing this tab aborts the agent task. Completed steps are not kept.",
    term_close_body_running: "Closing this tab terminates the running command.",
    term_close_cancel: "Cancel (keep running)",
    term_close_discard: "Close and abort",
```

- [ ] **Step 3: 人工核對語系對稱**

執行：`grep -c "term_close_" src/lib/i18n.ts`
預期：`12`（6 個 key × 2 語系）。

> ⚠️ **`npx tsc -b` 抓不到語系不對稱**——英文物件是 `{ ...zhTW, ...enRaw }` 且 `TranslationKey` 只從 zh-TW 推導，英文缺 key 會靜默 fallback 成中文。這點在前一份計畫執行時實測確認過。所以這裡必須用 `grep -c` 人工核對。

- [ ] **Step 4: 型別檢查**

執行：`npx tsc -b`
預期：無輸出、exit code 0。

- [ ] **Step 5: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "i18n: add strings for the terminal tab close confirmation"
```

---

## Task 2：TerminalView 註冊 close guard

**Files:**
- Modify: `src/components/TerminalView.tsx`
- Test: `src/components/TerminalView.closeGuard.test.tsx`

> ⚠️ **`TerminalView.tsx` 有 2272 行，而且是使用者特別交代過不能破壞 CWD 同步的敏感區域。** 只加新程式碼，不要碰任何既有邏輯、不要重排 import、不要「順手改善」。

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/TerminalView.closeGuard.test.tsx`。

**先做探勘**：`TerminalView` 相依很多（xterm、PTY IPC、多個 hook）。掛載它需要哪些 mock 尚未實測過，**你必須自己先跑一次最小掛載實驗**，找出實際需要的 mock 集合，再寫正式測試。不要照抄別的測試檔的 mock 清單。

若掛載成本過高（例如需要 mock 超過 6~7 個模組、或 xterm 在 jsdom 下無法初始化），**停下來回報 BLOCKED**，並說明你試過什麼。屆時會改用「把判斷抽成純函式」的策略（本 repo 既有慣例：`LoopStudio/validateRoster.ts`、`HomeView/routeIntent.ts`、`lib/closeTabGuard.ts`）。

測試要涵蓋的行為：

1. 閒置（無 running block、無 active mission）→ guard 直接 resolve `true`，不顯示對話框
2. 有指令執行中 → 顯示對話框，標題為「指令執行中」，Promise 保持未定
3. Agent 任務進行中 → 顯示對話框，標題為「Agent 任務進行中」
4. 按「關閉並中止」→ resolve `true`
5. 按「取消（繼續執行）」→ resolve `false`，對話框消失
6. **註冊之後才開始執行的指令，guard 仍看得到**（釘住 stale closure）
7. unmount 時呼叫 `unregisterCloseGuard`

查詢一律用 `getByRole("heading", { level: 3 })` 與 `getByRole("button", { name: ... })`，**不要用 CSS class**——否則之後改標記會誤報。

呼叫 guard 會觸發 `setState`，**必須包在 `act()` 裡**，否則對話框不會 flush（前一份計畫實測踩過）。

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/components/TerminalView.closeGuard.test.tsx`
預期：全部失敗，訊息為「沒有註冊 close guard」之類。

**必須實際看到紅燈並回報真實輸出。**

- [ ] **Step 3: 擴充 Props**

在 `TerminalViewProps` 介面加入（型別與 `LoopStudio/index.tsx:64-65` 一致）：

```tsx
  registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
  unregisterCloseGuard?: (tabId: string) => void;
```

並在函式參數解構中加上這兩個（`tabId` 已經有了）。

- [ ] **Step 4: 加入 guard 狀態與 ref**

在 `blocks` 與 `agentMission` 都已在作用域內之後（`:375` 那個 `useEffect` 附近）加入：

```tsx
  // 關閉確認：ref 是必要的，不是風格選擇。guard 只註冊一次，若閉包捕捉
  // 當下的值，之後開始跑的指令它都看不到，會在沒有任何錯誤訊號的情況下
  // 直接放行——功能等於靜默失效。
  const isBusyRef = useRef(false);
  isBusyRef.current = blocks[blocks.length - 1]?.status === "running";
  const missionActiveRef = useRef(false);
  missionActiveRef.current = agentMission.active;

  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const closeResolveRef = useRef<((canClose: boolean) => void) | null>(null);

  const handleCloseConfirm = useCallback((canClose: boolean) => {
    setShowCloseConfirm(false);
    closeResolveRef.current?.(canClose);
    closeResolveRef.current = null;
  }, []);
```

- [ ] **Step 5: 註冊 guard**

```tsx
  useEffect(() => {
    if (!tabId || !registerCloseGuard) return;
    registerCloseGuard(tabId, () => {
      // 閒置的終端機沒有進行中的工作可失去，不要打擾使用者。
      if (!isBusyRef.current && !missionActiveRef.current) {
        return Promise.resolve(true);
      }
      return new Promise<boolean>((resolve) => {
        closeResolveRef.current = resolve;
        setShowCloseConfirm(true);
      });
    });
    return () => { unregisterCloseGuard?.(tabId); };
  }, [tabId, registerCloseGuard, unregisterCloseGuard]);
```

- [ ] **Step 6: 渲染對話框**

import 加入：

```tsx
import { CloseConfirmDialog } from "./CloseConfirmDialog";
```

在 `:1599` 那個 return 的根 `<div>` 內部**最前面**插入：

```tsx
      {showCloseConfirm && (
        <CloseConfirmDialog
          title={missionActiveRef.current ? t.term_close_title_mission : t.term_close_title_running}
          body={missionActiveRef.current ? t.term_close_body_mission : t.term_close_body_running}
          confirmLabel={t.term_close_discard}
          cancelLabel={t.term_close_cancel}
          onConfirm={() => handleCloseConfirm(true)}
          onCancel={() => handleCloseConfirm(false)}
        />
      )}
```

- [ ] **Step 7: 跑測試確認轉綠**

執行：`npx vitest run src/components/TerminalView.closeGuard.test.tsx`
預期：全部通過。

- [ ] **Step 8: 證明防線非空轉（變異驗證）**

把 Step 5 的條件暫時改成讀閉包值而非 ref：

```tsx
      if (blocks[blocks.length - 1]?.status !== "running" && !agentMission.active) {
```

執行測試。**「註冊之後才開始執行的指令」那題必須變紅。**若它仍是綠的，代表那題沒有真的驗到，必須先修測試。

改完**務必還原**，並用 `git diff` 確認沒有殘留。

- [ ] **Step 9: 全套驗證**

執行：`npx tsc -b` → 無輸出
執行：`npm run test` → 無新增失敗

- [ ] **Step 10: Commit**

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.closeGuard.test.tsx
git commit -m "feat(terminal): confirm before closing a tab with work in progress"
```

---

## Task 3：修 handleRouteHintPick（必須，見上方背景）

**Files:**
- Modify: `src/components/TerminalApp.tsx`
- Test: `src/components/TerminalApp.routeHint.test.tsx`（若掛載成本過高，見下方替代方案）

- [ ] **Step 1: 讓 `handleCloseTab` 回傳是否真的關掉了**

`src/components/TerminalApp.tsx:302` 起，把兩處 `return;` 改成回傳布林，並在成功路徑結尾回傳 `true`：

```tsx
  const handleCloseTab = useCallback(async (id: string): Promise<boolean> => {
    const canClose = await runCloseGuard(
      id,
      activeIdRef.current,
      closeGuardsRef.current.get(id),
      setActiveId,
    );
    if (!canClose) return false;
```

並在函式最後（`setTabs(...)` 之後）加上：

```tsx
    return true;
  }, []);
```

其餘內容**一字不動**。

`Ctrl+W`（`:424`）與 `TabBar onClose`（`:500`）忽略回傳值，TypeScript 允許把回傳值的函式指派給期待 `void` 的位置，不需改動。

- [ ] **Step 2: 改 `handleRouteHintPick`**

把 `src/components/TerminalApp.tsx:345-359` 的：

```tsx
  const handleRouteHintPick = useCallback((type: TabType) => {
    if (!routeHint) return;
    const { tabId: oldId, userText } = routeHint;
    // 這裡刻意不 await：終端機分頁目前沒有註冊任何 close guard（只有 LoopStudio
    // 有），所以 handleCloseTab 對它是同步跑完的。
    //
    // 但如果哪天有人幫 TerminalView 加上 close guard，這行就會變成真的非同步，
    // 於是新分頁會在使用者還沒回答確認對話框時就先開出來。屆時要改成 await，
    // 而且要處理「使用者取消關閉」的情況——那時不應該開新分頁。
    void handleCloseTab(oldId);
```

替換為：

```tsx
  const handleRouteHintPick = useCallback(async (type: TabType) => {
    if (!routeHint) return;
    const { tabId: oldId, userText } = routeHint;
    // 終端機／程式庫協助／LoopStudio 分頁都可能註冊 close guard，所以這裡
    // 一定要 await：否則確認框還開著，新分頁就先開出來搶走焦點，舊分頁會
    // 帶著一個看不見的對話框卡住關不掉。
    // 使用者取消關閉時也不該開新分頁——他要的是留在原地。
    const closed = await handleCloseTab(oldId);
    if (!closed) return;
```

其餘三行（`opts`、`handlePickerSelect`、`setRouteHint`）**不動**。

- [ ] **Step 3: 確認呼叫端能接受 async**

`handleRouteHintPick` 現在回傳 `Promise<void>`。找出它傳給誰（搜尋 `onPick` 或 `handleRouteHintPick`），確認該 prop 的型別接受回傳 Promise 的函式。若型別是 `(type: TabType) => void`，TypeScript 允許；若報錯就回報，不要自行放寬型別。

- [ ] **Step 4: 寫測試**

理想上測 `handleRouteHintPick` 的兩個行為：guard 放行→開新分頁；guard 否決→**不**開新分頁。

`TerminalApp` 掛載成本很高（xterm + 11 種分頁）。**先評估**：若無法用合理的 mock 量掛載，改成測 Step 1 的 `handleCloseTab` 契約即可——把「guard 否決時回傳 false」這件事釘住，那是 Step 2 正確性的前提。**明確回報你選了哪條路以及為什麼。**

- [ ] **Step 5: 全套驗證**

執行：`npx tsc -b` → 無輸出
執行：`npm run test` → 無新增失敗

- [ ] **Step 6: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "fix(tabs): await the close guard before replacing a mis-routed tab"
```

---

## Task 4：把 guard props 傳給 TerminalView

**Files:** Modify `src/components/TerminalApp.tsx`

- [ ] **Step 1: 傳入 props**

找到 `<TerminalView` 的 JSX（約 `:620-640`），加入兩個 prop（`tabId` 應該已經有了，若沒有也一併加）：

```tsx
                  registerCloseGuard={registerCloseGuard}
                  unregisterCloseGuard={unregisterCloseGuard}
```

**不要動任何既有的 prop。**

- [ ] **Step 2: 全套驗證**

執行：`npx tsc -b` → 無輸出
執行：`npm run test` → 無新增失敗

- [ ] **Step 3: Commit**

```bash
git add src/components/TerminalApp.tsx
git commit -m "feat(terminal): wire the close guard from TerminalApp"
```

---

## Task 5：實機驗證（兩種分頁一起驗）

前一份計畫的「程式庫協助」分頁**尚未經過實機驗證**（先前測錯分頁），這裡一併補上。

- [ ] **Step 1: 啟動**

執行：`npm run tauri:dev`

- [ ] **Step 2: 終端機分頁——閒置直接關**

開新終端機分頁，不執行任何東西，按 `✕`。
→ 預期：**沒有**確認框，直接關掉。

- [ ] **Step 3: 終端機分頁——指令執行中會攔**

在終端機分頁執行一個會跑一陣子的指令（例如 `sleep 30`），趁它還在跑時按 `✕`。
→ 預期：跳出「**指令執行中**」。按「取消（繼續執行）」→ 分頁還在、指令繼續。再按 `✕` 選「關閉並中止」→ 關掉。

- [ ] **Step 4: 終端機分頁——Agent 任務進行中會攔**

用 `/agent` 起一個任務，趁它在跑時按 `✕`。
→ 預期：跳出「**Agent 任務進行中**」。

- [ ] **Step 5: 「Agent: ...」協調分頁（使用者最初指的那種）**

用協調功能生出一個「Agent: ...」分頁，趁裡面的 agent 還在跑時按 `✕`。
→ 預期：跳確認框。

- [ ] **Step 6: 非當前分頁**

切到別的分頁，點側邊欄上那個忙碌中的終端機分頁的 `✕`。
→ 預期：自動切過去並顯示確認框。

- [ ] **Step 7: RouteHint 不再產生殭屍分頁**

在首頁輸入一句話讓 AI 路由到終端機分頁（會自動起 mission）→ 趁 mission 在跑時按 RouteHint 的「換成…」。
→ 預期：先跳確認框；選「關閉並中止」→ 舊分頁關掉、新分頁開出來；選「取消」→ **不開新分頁**，留在原地。

- [ ] **Step 8: 程式庫協助分頁（補驗前一份計畫）**

側邊欄找「**程式庫協助**」分頁（不是「Agent: ...」）。空白時關 → 不攔；有對話時關 → 跳「對話尚未保存」；串流中關 → 跳「AI 正在回應中」。

- [ ] **Step 9: LoopStudio 沒退步**

開 LoopStudio，設個目標讓它變 dirty，按 `✕` → 確認框外觀與文案與以前一致。

---

## Self-Review 對照表

| 需求 | 對應任務 |
|---|---|
| 終端機分頁：指令執行中會攔 | Task 2 Step 5（`isBusyRef`） |
| 終端機分頁：Agent 任務進行中會攔 | Task 2 Step 5（`missionActiveRef`） |
| 閒置終端機不攔 | Task 2 Step 5 的快速放行分支 + Task 5 Step 2 |
| 文案（中英） | Task 1 |
| stale closure 防線 | Task 2 Step 4（ref）+ Step 8（變異驗證） |
| `handleRouteHintPick` 迴歸修正 | Task 3 |
| 接線 | Task 4 |
| 補驗程式庫協助 | Task 5 Step 8 |

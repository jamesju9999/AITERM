# Agent 指令卡住時的偵測與接手 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agent 執行的指令若讓 shell 卡在等輸入（例如 heredoc 沒收尾），不再永久死鎖；改為偵測到長時間無輸出時跳提示，使用者選擇中斷後由 AI 帶著結果接手繼續。

**Architecture:** 用「區塊還在跑、但 PTY 一段時間完全沒有輸出」當偵測訊號（跑弱掃這類長工作會持續吐輸出，卡在 `heredoc>` 則是完全安靜）。偵測只提示、不自動殺；使用者按「中斷」才送 Ctrl+C 並強制結案，既有的完成 callback 會自然讓 agent 迴圈接手下去。

**Tech Stack:** React 19 + TypeScript、Vitest + React Testing Library + jsdom。

---

## 背景：為什麼會死鎖

使用者實際遇到的狀況：AI 產生了一個含 heredoc 的指令，`EOF` 結束標記沒有正確送達，shell 就停在 `heredoc>` 等後續內容。

死鎖的成因是兩件事扣在一起：

1. `src/components/AiPanel/index.tsx:308` 等指令完成的 Promise **沒有任何逾時**：

   ```js
   await new Promise<void>((resolve) => {
     onExecuteCommand(cmd, async (block) => { … resolve(); });
   });
   ```

   它只在區塊完成時 resolve，而區塊完成要靠 shell 送出 OSC 133 結束訊號——shell 卡在 heredoc 就永遠不會送。

2. `useTerminalBlocks.ts` 的 `submitCommand` 裡有一段防禦：發現前一個區塊還在 running 就強制結案。但它**只在送出下一個指令時才觸發**，而 agent 不等這個完成就不會送下一個。**互相等待**。

目前唯一的脫身方式是按停止鈕。任何不會自己結束的指令（heredoc、`vim`、`ssh`、等輸入的互動程式）都會踩到。

**注意**：造成畸形輸入的原因（畫面上的 `<ffffffff>` 是 zsh 顯示無法解碼位元組的方式）**尚未查明**，不在本計畫範圍。本計畫處理的是「不管什麼原因卡住都能脫身」。

---

## 現況調查（事實基礎）

| 事實 | 位置 |
|---|---|
| agent 等待指令完成的 Promise，無逾時 | `src/components/AiPanel/index.tsx:308-310` |
| `finalizeBlock` **會**呼叫完成 callback（`setTimeout(() => cb(finalBlock), 50)`）——強制結案就能讓 agent 迴圈自然接手 | `src/hooks/useTerminalBlocks.ts:131-136` |
| `appendOutput` 在每個 PTY chunk 進來且區塊 running 時被呼叫 | `src/hooks/useTerminalBlocks.ts:67-69` |
| PTY 資料進入前端的唯一入口，可在此記錄「最後輸出時間」 | `src/components/TerminalView.tsx:1117-1118` |
| `AiPanel` 由 `TerminalView` 渲染並取得 `onExecuteCommand` | `src/components/TerminalView.tsx:2010` |
| `submitCommand` 送出指令的寫法：`writePty(sessionId, clearSeq + cmd + "\r")` | `src/hooks/useTerminalBlocks.ts:299` |
| 送 Ctrl+C 的既有寫法是 `writePty(session, "\x03")`（`\r` 的用法見同檔多處） | `src/components/TerminalView.tsx:771,918` 等 |

---

## 範圍

**含：**
- 偵測「agent 指令執行中、但 PTY 超過 N 秒完全沒有輸出」
- 在 AI 面板顯示提示，附「繼續等待」與「中斷並讓 AI 接手」兩個選項
- 選擇中斷時：送 Ctrl+C → 強制結案 → agent 帶著已有輸出繼續下一步
- 選擇繼續等待時：重設計時器，不再提示直到又安靜 N 秒

**不含：**
- 自動中斷（使用者已明確要求要先問過）
- 追查畸形位元組的來源（另一個題目）
- 讓門檻可在設定頁調整（先用常數，有需要再說）
- 非 agent 情境（使用者自己打的指令）的卡住偵測

---

## Task 1：把「最後輸出時間」暴露給 AiPanel

**Files:**
- Modify: `src/components/TerminalView.tsx`
- Test: `src/components/TerminalView.idleSignal.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/TerminalView.idleSignal.test.tsx`。

掛載 `TerminalView` 的方式已經驗證過（見 `src/components/TerminalView.closeGuard.test.tsx`）：mock `@tauri-apps/api/{core,event,path}` 三個入口即可涵蓋所有 `ipc/*.ts`，再補 jsdom 缺的 `scrollTo`/`matchMedia`/`ResizeObserver`。**照那個檔案的既有寫法**，不要另外發明。

測試要驗的行為：`TerminalView` 傳給 `AiPanel` 的 `getIdleMs` prop 是一個函式，且在有 PTY 輸出之後回傳值會變小（接近 0）。

取得 prop 的方式：把 `AiPanel` mock 掉，記錄它收到的 props：

```tsx
const aiPanelProps: Record<string, unknown>[] = [];
vi.mock("./AiPanel", () => ({
  AiPanel: (props: Record<string, unknown>) => {
    aiPanelProps.push(props);
    return null;
  },
}));
```

（先確認 `AiPanel` 的實際 import 路徑與具名匯出，再照著寫。）

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/components/TerminalView.idleSignal.test.tsx`
預期：FAIL，`getIdleMs` 是 `undefined`。

**必須實際看到紅燈並回報真實輸出。**

- [ ] **Step 3: 記錄最後輸出時間**

在 `TerminalView` 元件內加一個 ref（放在其他 ref 附近即可）：

```tsx
  // Agent 卡住偵測用：PTY 最後一次吐出東西的時間。卡在 heredoc>／等輸入的
  // 互動程式是完全安靜的，而跑得好好的長指令會持續有輸出——用「安靜多久」
  // 區分兩者，比固定逾時準得多，也不會誤殺跑很久但正常的工作。
  const lastPtyOutputAtRef = useRef(Date.now());
```

在 `onPtyData` 的 callback 內（`:1117` 那個 `(bytes) => {` 的第一行）加上：

```tsx
          lastPtyOutputAtRef.current = Date.now();
```

- [ ] **Step 4: 傳給 AiPanel**

在 `<AiPanel ... />`（`:2010` 附近）加入：

```tsx
          getIdleMs={() => Date.now() - lastPtyOutputAtRef.current}
```

並在 `AiPanel` 的 props 型別加上（找到它的 `interface`／`type`，照既有風格）：

```tsx
  /** 距離 PTY 最後一次有輸出過了多久（毫秒）。用來偵測指令是不是卡住了。 */
  getIdleMs?: () => number;
```

- [ ] **Step 5: 跑測試確認轉綠**

執行：`npx vitest run src/components/TerminalView.idleSignal.test.tsx`
預期：PASS。

- [ ] **Step 6: 型別檢查與全套**

執行：`npx tsc -b` → 無輸出
執行：`npm run test` → 無新增失敗

- [ ] **Step 7: Commit**

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.idleSignal.test.tsx src/components/AiPanel/index.tsx
git commit -m "feat(agent): expose PTY idle time to the AI panel"
```

---

## Task 2：中斷指令的能力

**Files:**
- Modify: `src/components/TerminalView.tsx`
- Modify: `src/hooks/useTerminalBlocks.ts`（若需要匯出強制結案的方法）
- Test: `src/hooks/useTerminalBlocks.interrupt.test.ts`

- [ ] **Step 1: 先確認 `finalizeBlock` 是否已可從外部使用**

讀 `src/hooks/useTerminalBlocks.ts` 的回傳值（約 `:366` 附近 `useTerminalBlocks` 的 return）。若 `finalizeBlock` 已在回傳物件裡，直接用；若沒有，加進去（**只加，不要改既有欄位**）。

在報告中明確說明你走的是哪一條。

- [ ] **Step 2: 寫會紅的測試**

建立 `src/hooks/useTerminalBlocks.interrupt.test.ts`，用 `renderHook` 驗證：

1. 送出一個帶 `onComplete` 的指令後，區塊狀態是 `running`
2. 對該區塊呼叫強制結案（exit code `-1`）後，`onComplete` **會被呼叫**，且拿到的區塊 `status` 不是 `running`

第 2 點是整個設計的基石——agent 的 Promise 就是靠這個 callback 才會 resolve。若它不成立，後面全部免談。

注意 `finalizeBlock` 內是 `setTimeout(..., 50)` 才呼叫 callback，測試要處理這個延遲（`vi.useFakeTimers()` 或 `waitFor`）。

- [ ] **Step 3: 跑測試確認會紅**（若 `finalizeBlock` 本來就沒匯出）

執行：`npx vitest run src/hooks/useTerminalBlocks.interrupt.test.ts`
回報真實輸出。若這個測試一寫就是綠的（代表能力本來就在），**照 Task 3 的方式改用刻意破壞來證明它非空轉**，並在報告中說明。

- [ ] **Step 4: 實作到綠**

- [ ] **Step 5: 在 TerminalView 提供中斷函式**

在 `<AiPanel ... />` 加入：

```tsx
          onInterruptCommand={() => {
            // Ctrl+C：把 shell 從 heredoc／等輸入的狀態拉回提示字元。
            if (sessionRef.current) writePty(sessionRef.current, "\x03").catch(console.error);
            // 強制結案。finalizeBlock 會呼叫 agent 正在等的完成 callback
            // （useTerminalBlocks.ts:131-136），agent 迴圈因此自然接手下去，
            // 不需要另外通知它。
            const latest = blocksRef.current[blocksRef.current.length - 1];
            if (latest?.status === "running") finalizeBlock(latest.id, -1);
          }}
```

（`sessionRef`／`blocksRef` 的實際名稱以檔案裡既有的為準，不要新建。）

並在 `AiPanel` props 型別加上：

```tsx
  /** 中斷目前這個卡住的指令：送 Ctrl+C 並強制結案。 */
  onInterruptCommand?: () => void;
```

- [ ] **Step 6: 全套驗證與 Commit**

執行：`npx tsc -b`、`npm run test`

```bash
git add -A
git commit -m "feat(agent): add the ability to interrupt a stuck command"
```

---

## Task 3：卡住提示與接手

**Files:**
- Modify: `src/components/AiPanel/index.tsx`
- Test: `src/components/AiPanel/stuckPrompt.test.tsx`

- [ ] **Step 1: 寫會紅的測試**

建立 `src/components/AiPanel/stuckPrompt.test.tsx`。先讀 `src/components/AiPanel/AiPanel.test.tsx` 看既有的掛載與 mock 方式，**沿用它**。

要驗的行為：

1. agent 執行指令期間，`getIdleMs` 一直回傳小值 → **不**顯示卡住提示
2. `getIdleMs` 回傳超過門檻 → 顯示提示，且提示中包含「中斷」與「繼續等待」兩個按鈕
3. 按「中斷」→ `onInterruptCommand` 被呼叫一次
4. 按「繼續等待」→ 提示消失，且 `onInterruptCommand` **沒有**被呼叫
5. 指令正常完成 → 提示不會殘留

用 `vi.useFakeTimers()` 推進時間來觸發偵測。

- [ ] **Step 2: 跑測試確認會紅**

執行：`npx vitest run src/components/AiPanel/stuckPrompt.test.tsx`
回報真實輸出。

- [ ] **Step 3: 實作偵測與提示**

在 `AiPanel` 內：

```tsx
/**
 * 指令跑著卻完全沒有輸出多久，就當它可能卡住了。
 *
 * 用「安靜多久」而不是「跑多久」當判準：使用者會跑弱掃、建置這類長工作，
 * 那些會持續吐輸出；真正卡住（heredoc 等收尾、互動程式等輸入）則是全然安靜。
 * 這個值寧可寬鬆——誤判的代價是打擾使用者，而漏判只是回到原本要按停止鈕的狀態。
 */
const STUCK_IDLE_MS = 120_000;
```

在 `:308` 那個等待 Promise 的區塊加上一個 `setInterval`，每幾秒用 `getIdleMs()` 檢查一次；超過門檻就 `setState` 顯示提示。Promise resolve（指令完成）或元件卸載時務必清掉 interval。

**兩個必須處理的細節：**

- 使用者按「繼續等待」後要重設判準，否則下一次檢查會立刻又跳提示。做法：記一個 `snoozeUntilRef`，在它之前不再提示。
- 按「中斷」後**不要**自己 resolve Promise。呼叫 `onInterruptCommand()` 就好——強制結案會觸發既有的完成 callback，Promise 自然 resolve，agent 帶著已有的輸出繼續。自己 resolve 會造成雙重 resolve 與狀態不一致。

提示的文案（i18n，中英各一份，前綴沿用 AiPanel 既有的慣例）：

| key | zh-TW | en |
|---|---|---|
| `agent_stuck_title` | 指令似乎沒有反應 | Command seems unresponsive |
| `agent_stuck_body` | 這個指令已經超過兩分鐘沒有任何輸出，可能正在等待輸入。中斷後 AI 會帶著目前的結果繼續。 | This command has produced no output for over two minutes and may be waiting for input. Interrupting lets the AI continue from what it has so far. |
| `agent_stuck_wait` | 繼續等待 | Keep waiting |
| `agent_stuck_interrupt` | 中斷並繼續 | Interrupt and continue |

> ⚠️ **`npx tsc -b` 抓不到語系不對稱**——英文物件是 `{ ...zhTW, ...enRaw }` 且 `TranslationKey` 只從 zh-TW 推導，英文缺 key 會靜默 fallback 成中文。用 `grep -c "agent_stuck_" src/lib/i18n.ts` 核對，應為 `8`。

- [ ] **Step 4: 跑測試確認轉綠**

- [ ] **Step 5: 證明偵測非空轉（變異驗證）**

把門檻暫時改成一個極大值（例如 `STUCK_IDLE_MS = 999_999_999`），跑測試。**「超過門檻會顯示提示」那題必須變紅。**若它仍是綠的，代表那題沒有真的驗到偵測邏輯。

改完**務必還原**，並用 `git diff` 確認沒有殘留。

- [ ] **Step 6: 全套驗證與 Commit**

執行：`npx tsc -b`、`npm run test`

```bash
git add -A
git commit -m "feat(agent): prompt to interrupt a stuck command instead of hanging"
```

---

## Task 4：實機驗證

自動化測試用假時間、假 PTY，驗不到真正的卡住情境。

- [ ] **Step 1: 啟動**

執行：`npm run tauri:dev`

- [ ] **Step 2: 正常長指令不該被打擾**

用 agent 跑一個會持續輸出好幾分鐘的工作（例如掃描、建置）。
→ 預期：**不會**跳卡住提示（因為一直有輸出）。

- [ ] **Step 3: 真的卡住會跳提示**

讓 agent 執行一個會停在等輸入的指令。最容易複製的做法是在終端機手動製造相同狀態：送出 `cat << EOF`（不給結束標記），讓 shell 停在 `heredoc>`。
→ 預期：約兩分鐘後跳出「指令似乎沒有反應」。

- [ ] **Step 4: 「繼續等待」**

按「繼續等待」。
→ 預期：提示消失、指令沒有被中斷、shell 仍在 `heredoc>`；再過約兩分鐘會再次提示。

- [ ] **Step 5: 「中斷並繼續」**

按「中斷並繼續」。
→ 預期：終端機回到正常提示字元（Ctrl+C 生效），**AI 接著繼續做下一步**，而不是整個任務結束。

- [ ] **Step 6: 確認沒有回歸**

一般（非 agent）使用終端機、手動跑長指令、agent 正常完成的流程都照舊。

---

## Self-Review 對照表

| 需求 | 對應任務 |
|---|---|
| 偵測「還在跑但沒輸出」 | Task 1（訊號）+ Task 3（判斷） |
| 不自動殺、先問過使用者 | Task 3 Step 3 的提示 |
| 中斷後由 AI 接手 | Task 2 Step 5（Ctrl+C + 強制結案觸發既有 callback） |
| 繼續等待不會一直重複跳 | Task 3 Step 3 的 snooze |
| 長指令不被誤殺 | 用閒置而非總時長判斷 + Task 4 Step 2 |
| 偵測邏輯非空轉 | Task 3 Step 5 變異驗證 |

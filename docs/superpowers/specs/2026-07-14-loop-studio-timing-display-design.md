# Loop Studio 執行時間顯示 — 設計規格

**日期：** 2026-07-14  
**狀態：** 已核准，待實作

---

## 目標

在 Loop Studio 右側執行視窗中，為每個動作區塊加入時間資訊：
- 每個 entry 的執行開始時間
- 每個主要區塊（Agent、Verifier）的耗時
- 整個 Loop 的最終完成時間與總耗時

使用者可在設定中切換**完整版**（所有行顯示時間）與**精簡版**（只有主要區塊顯示時間）。

---

## 範疇

**包含：**
- `TraceEntry` 型別新增 `startTimestamp` 欄位
- `useOrchestratorLoop.ts` 在產生完成型 entry 時計算並記錄 startTimestamp
- `ExecutionTrace.tsx` 以 badge 形式渲染時間資訊
- `GeneralPage.tsx` 新增 `loopTimingMode` 設定選項
- localStorage 持久化設定

**不包含：**
- 匯出或複製時間資訊
- Session 儲存到磁碟的 trace 中補寫時間（已存的 session 不回填）

---

## 資料層

### TraceEntry 新增欄位

```ts
// src/hooks/useOrchestratorLoop.ts
export interface TraceEntry {
  id: string;
  kind: TraceEntryKind;
  iteration?: number;
  agentName?: string;
  text: string;
  actions?: SubAgentAction[];
  verifierDone?: boolean;
  verifierResult?: VerifierResult;
  isError?: boolean;
  timestamp: number;        // 既有：entry 建立時間（毫秒）
  startTimestamp?: number;  // 新增：對應區塊的開始時間（毫秒），僅完成型 entry 設定
}
```

### startTimestamp 計算邏輯

- **`sub_agent_done`**：往前掃 `traceBuffer`，找最後一個 `kind === "sub_agent_start"` 且 `agentName` 相同的 entry，取其 `timestamp`。
- **`verifier_result`**：記錄 `runToolLoop` 呼叫前的 `Date.now()`，傳入作為 `startTimestamp`。
- 其他 entry 不設定 `startTimestamp`（`undefined`）。

---

## 設定層

### 設定鍵

```
localStorage key: "loopTimingMode"
型別: "full" | "compact"
預設: "compact"
```

### GeneralPage 新增 UI

在 `src/components/Settings/GeneralPage.tsx` 的一般設定區塊新增：

```
Loop Studio 執行時間顯示
  ○ 精簡版（預設）— 僅主要區塊顯示時間
  ○ 完整版 — 每一行都顯示時間戳
```

以 radio button 或 select 實作，onChange 直接寫入 localStorage。

---

## UI 層

### 時間 badge 規格

所有時間資訊以右側對齊的 badge 呈現，不插入獨立時間列。每個 entry 的 badge 區域與內容主體並排（flexbox，space-between）。

**Badge 樣式分類：**

| Badge 類型 | 用途 | 範例 |
|---|---|---|
| `tb-clock` | 開始時間（灰色） | `14:22:04` |
| `tb-dur` | 耗時（綠色） | `⏱ 7s` |
| `tb-done-clk` | 完成時間（藍紫色） | `✓ 14:22:11` |
| `tb-total` | 總耗時（綠色粗體） | `總耗時 14s` |

**時間格式：**
- 時間戳：`HH:MM:SS`（本地時間，`Date` → `toLocaleTimeString("zh-TW", {hour12: false})`）
- 耗時：小於 60s 顯示 `Xs`；60s 以上顯示 `X分Ys`

### 各 entry 類型的顯示規則

| Entry 類型 | 完整版 (`full`) | 精簡版 (`compact`) |
|---|---|---|
| `iteration_start` | `tb-clock`（開始時間） | `tb-clock`（開始時間） |
| `orchestrator_action` | `tb-clock` | — |
| `sub_agent_start` | `tb-clock` | `tb-clock` |
| `sub_agent_action`（工具呼叫） | `tb-clock` | — |
| `sub_agent_done` | `tb-dur` + `tb-done-clk` | `tb-dur` + `tb-done-clk` |
| `verifier_result` | `tb-dur` + `tb-done-clk` | `tb-dur` + `tb-done-clk` |
| `loop_done` | `tb-total` + `tb-done-clk` | `tb-total` + `tb-done-clk` |
| `loop_stopped` | `tb-clock` | `tb-clock` |
| `loop_error` | `tb-clock` | `tb-clock` |

> `loop_done` 的總耗時 = `loop_done.timestamp - 第一個 iteration_start.timestamp`

### ExecutionTrace props 變更

`ExecutionTrace` 新增 prop：

```ts
interface ExecutionTraceProps {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  timingMode: "full" | "compact";  // 新增
}
```

父元件（`LoopStudio/index.tsx`）從 localStorage 讀取 `loopTimingMode` 並傳入。

---

## 檔案清單

| 檔案 | 變更類型 |
|---|---|
| `src/hooks/useOrchestratorLoop.ts` | 修改：TraceEntry 新增 `startTimestamp`；addTrace 邏輯補寫 startTimestamp |
| `src/components/LoopStudio/ExecutionTrace.tsx` | 修改：新增 `timingMode` prop；各 entry 渲染時加入時間 badge |
| `src/components/LoopStudio/styles.css` | 修改：新增 badge 樣式（`.tb`, `.tb-clock`, `.tb-dur`, `.tb-done-clk`, `.tb-total`） |
| `src/components/Settings/GeneralPage.tsx` | 修改：新增 `loopTimingMode` 設定選項 |
| `src/components/LoopStudio/index.tsx` | 修改：讀取 localStorage `loopTimingMode`，傳入 `ExecutionTrace` |

---

## 成功標準

1. Loop 執行時，每個 entry 右側依 `timingMode` 顯示對應 badge
2. `sub_agent_done` 的耗時 badge 數值正確（等於 done.timestamp - start.timestamp）
3. `loop_done` 總耗時正確（等於 done.timestamp - 第一個 iteration_start.timestamp）
4. 在設定頁切換完整版/精簡版後，下次執行立即反映新設定
5. 預設為精簡版，舊 session resume 不崩潰（startTimestamp 為 undefined 時不顯示耗時 badge 即可）

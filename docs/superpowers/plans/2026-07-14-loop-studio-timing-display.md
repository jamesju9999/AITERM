# Loop Studio 執行時間顯示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Loop Studio 執行視窗中，以 badge 形式顯示每個動作的開始時間、區塊耗時與最終完成時間，並透過設定切換完整/精簡模式。

**Architecture:** `TraceEntry` 新增 `startTimestamp` 欄位，由 `useOrchestratorLoop` 在 emit `sub_agent_done` / `verifier_result` 前即時記錄；純函式時間格式化邏輯提取至 `src/lib/timeFormat.ts`；`ExecutionTrace` 接收 `timingMode` prop 並依模式渲染時間 badge；`LoopStudioView` 從 localStorage 讀取設定並傳入；`GeneralPage` 新增設定選項。

**Tech Stack:** React 19, TypeScript, Vitest, localStorage

---

## 檔案清單

| 檔案 | 類型 | 職責 |
|---|---|---|
| `src/lib/timeFormat.ts` | 新建 | `formatTime` / `formatDuration` 純函式 |
| `src/lib/timeFormat.test.ts` | 新建 | 上述函式的 Vitest 單元測試 |
| `src/hooks/useOrchestratorLoop.ts` | 修改 | `TraceEntry.startTimestamp`；emit 前記錄開始時間 |
| `src/components/LoopStudio/styles.css` | 修改 | 時間 badge CSS（`.ls-time-meta`, `.ls-tb-*`） |
| `src/components/Settings/GeneralPage.tsx` | 修改 | 新增 `loopTimingMode` 設定 section |
| `src/components/LoopStudio/ExecutionTrace.tsx` | 修改 | 新增 `timingMode` prop，每個 entry 渲染時間 badge |
| `src/components/LoopStudio/index.tsx` | 修改 | 讀取 localStorage `loopTimingMode`，傳入 `ExecutionTrace` |

---

## Task 1：建立時間格式化工具函式

**Files:**
- Create: `src/lib/timeFormat.ts`
- Create: `src/lib/timeFormat.test.ts`

- [ ] **Step 1：寫失敗測試**

```ts
// src/lib/timeFormat.test.ts
import { describe, it, expect } from "vitest";
import { formatDuration } from "./timeFormat";

describe("formatDuration", () => {
  it("shows seconds when under 60s", () => {
    expect(formatDuration(0, 7000)).toBe("7s");
    expect(formatDuration(0, 59000)).toBe("59s");
  });
  it("shows minutes without remainder when exact", () => {
    expect(formatDuration(0, 60000)).toBe("1分");
    expect(formatDuration(0, 120000)).toBe("2分");
  });
  it("shows minutes and seconds when there is a remainder", () => {
    expect(formatDuration(0, 90000)).toBe("1分30s");
    expect(formatDuration(0, 125000)).toBe("2分5s");
  });
  it("rounds to nearest second", () => {
    expect(formatDuration(0, 7400)).toBe("7s");
    expect(formatDuration(0, 7500)).toBe("8s");
  });
  it("handles non-zero startMs", () => {
    expect(formatDuration(1000, 8000)).toBe("7s");
  });
});
```

- [ ] **Step 2：確認測試失敗**

```bash
npm run test -- timeFormat --run
```
預期：`Cannot find module './timeFormat'`

- [ ] **Step 3：實作 `timeFormat.ts`**

```ts
// src/lib/timeFormat.ts
export function formatTime(ms: number): string {
  return new Date(ms).toLocaleTimeString("zh-TW", { hour12: false });
}

export function formatDuration(startMs: number, endMs: number): string {
  const s = Math.round((endMs - startMs) / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m}分` : `${m}分${rem}s`;
}
```

- [ ] **Step 4：確認測試通過**

```bash
npm run test -- timeFormat --run
```
預期：所有 5 個 test PASS

- [ ] **Step 5：型別檢查**

```bash
npx tsc --noEmit
```
預期：無輸出

- [ ] **Step 6：Commit**

```bash
git add src/lib/timeFormat.ts src/lib/timeFormat.test.ts
git commit -m "feat(loop): add time formatting helpers"
```

---

## Task 2：`TraceEntry` 新增 `startTimestamp`，並在 emit 前記錄

**Files:**
- Modify: `src/hooks/useOrchestratorLoop.ts`

**背景：** `TraceEntry.timestamp` 是 entry 建立時間。`sub_agent_done` 需要知道對應 `sub_agent_start` 的時間點；`verifier_result` 需要知道 Verifier 開始執行的時間點。最準確的方式是在呼叫前即時記錄 `Date.now()`，不需要搜尋 traceBuffer。

- [ ] **Step 1：在 `TraceEntry` interface 新增欄位**

在 `src/hooks/useOrchestratorLoop.ts` 的 `TraceEntry` interface（目前第 55–66 行）加入新欄位：

```ts
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
  timestamp: number;
  startTimestamp?: number;  // ← 新增：對應區塊的開始時間（毫秒）
}
```

- [ ] **Step 2：在 `sub_agent_done` emit 前記錄開始時間**

找到 `runSubAgent(...)` 呼叫區塊（約第 468–492 行），在 `runSubAgent` 呼叫**之前**加入 `agentStartTs`，並在 `sub_agent_done` entry 傳入 `startTimestamp`：

```ts
// 在 runSubAgent 呼叫之前加這行：
const agentStartTs = Date.now();
const result = await runSubAgent(
  config.sessionId,
  targetAgent,
  args.task,
  { /* ...既有選項不變... */ },
);
subResult = result.answer;
iterSubAgentSummaries.push(`${args.agent_name} → ${subResult.slice(0, 100)}${subResult.length > 100 ? "..." : ""}`);
addTraceBuffered({
  kind: "sub_agent_done",
  agentName: args.agent_name,
  text: subResult,
  actions: result.actions,
  isError: result.isError,
  iteration: iter,
  startTimestamp: agentStartTs,   // ← 新增
});
```

- [ ] **Step 3：在 `verifier_result` emit 前記錄開始時間**

找到 `runToolLoop(...)` 呼叫區塊（約第 536–543 行），在呼叫**之前**加入 `verifierStartTs`，並在 `verifier_result` entry 傳入 `startTimestamp`：

```ts
const verifierStartTs = Date.now();   // ← 新增
const verifierRun = await runToolLoop(
  config.verifier.providerId,
  verifierMessages,
  ["read_file", "list_directory"],
  { sessionId: config.sessionId, effectiveRoot: config.projectDir ?? null },
  8,
  (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: config.verifier.name, text: action.tool, actions: [action], iteration: iter }),
);

// 在後面的 addTraceBuffered({ kind: "verifier_result", ... }) 加上：
addTraceBuffered({
  kind: "verifier_result",
  agentName: config.verifier.name,
  text: verifierResult.summary,
  verifierDone: verifierResult.done,
  verifierResult,
  iteration: iter,
  startTimestamp: verifierStartTs,   // ← 新增
});
```

- [ ] **Step 4：型別檢查**

```bash
npx tsc --noEmit
```
預期：無輸出

- [ ] **Step 5：Commit**

```bash
git add src/hooks/useOrchestratorLoop.ts
git commit -m "feat(loop): add startTimestamp to TraceEntry for timing calculation"
```

---

## Task 3：新增時間 badge CSS

**Files:**
- Modify: `src/components/LoopStudio/styles.css`

- [ ] **Step 1：在 `styles.css` 末尾附加 badge 樣式**

在檔案最末尾加入以下區塊：

```css
/* ── Timing Badges ── */
.ls-entry-timed {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}

.ls-entry-body {
  flex: 1;
  min-width: 0;
}

.ls-time-meta {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  flex-shrink: 0;
}

.ls-tb {
  border-radius: 10px;
  padding: 1px 7px;
  font-size: 10px;
  white-space: nowrap;
  font-family: 'SF Mono', 'Menlo', monospace;
  line-height: 1.6;
}

/* 開始時間（灰色） */
.ls-tb-clock { background: #161616; border: 1px solid #2a2a2a; color: #555; }

/* 耗時 — sub-agent（綠色）*/
.ls-tb-dur   { background: #0f2a0f; border: 1px solid #1a5a1a; color: #4c4; }

/* 耗時 — verifier（紫色）*/
.ls-tb-dur-v { background: #2a0f2a; border: 1px solid #5a1a5a; color: #c8a; }

/* 完成時間（藍紫色）*/
.ls-tb-done  { background: #0f0f2a; border: 1px solid #2a2a5a; color: #88a; }

/* 總耗時（綠色粗體）*/
.ls-tb-total { background: #0a1a0a; border: 1px solid #2a5a2a; color: #4c4; font-weight: bold; }
```

- [ ] **Step 2：Commit**

```bash
git add src/components/LoopStudio/styles.css
git commit -m "feat(loop): add timing badge CSS classes"
```

---

## Task 4：在 `GeneralPage` 新增 `loopTimingMode` 設定

**Files:**
- Modify: `src/components/Settings/GeneralPage.tsx`

- [ ] **Step 1：在元件頂端加入常數與 state**

在 `GeneralPage.tsx` 現有的 constants 區塊（`FONT_SIZE_OPTIONS` 等之後），加入：

```ts
const LOOP_TIMING_KEY = "loopTimingMode";
```

在 `GeneralPage()` 函式內現有 `useState` 宣告的末尾加入：

```ts
const [timingMode, setTimingMode] = useState<"full" | "compact">(
  () => (localStorage.getItem(LOOP_TIMING_KEY) as "full" | "compact" | null) ?? "compact"
);
```

- [ ] **Step 2：加入 handler**

在 `handleFontFamilyChange` 之後加入：

```ts
const handleTimingModeChange = (mode: "full" | "compact") => {
  setTimingMode(mode);
  localStorage.setItem(LOOP_TIMING_KEY, mode);
};
```

- [ ] **Step 3：在 JSX 加入設定 section**

在 `</div>` 結尾（`general-page` 的最後一個 section `telegram` 之後，`</div>` 之前）加入：

```tsx
<section className="settings-section">
  <h3>Loop Studio 執行時間顯示</h3>
  <p className="section-desc">控制執行記錄中時間 badge 的顯示密度</p>
  <div className="mode-list">
    {([
      { value: "compact" as const, label: "精簡版（預設）", desc: "僅主要區塊顯示時間（Iteration、Agent 開始/完成、Verifier）" },
      { value: "full"    as const, label: "完整版",         desc: "每一行都顯示開始時間戳，包含工具呼叫" },
    ]).map((opt) => (
      <label key={opt.value} className="mode-option">
        <input
          type="radio"
          name="loopTimingMode"
          value={opt.value}
          checked={timingMode === opt.value}
          onChange={() => handleTimingModeChange(opt.value)}
        />
        <div className="mode-text">
          <span className="mode-label">{opt.label}</span>
          <span className="mode-desc">{opt.desc}</span>
        </div>
      </label>
    ))}
  </div>
</section>
```

- [ ] **Step 4：型別檢查**

```bash
npx tsc --noEmit
```
預期：無輸出

- [ ] **Step 5：Commit**

```bash
git add src/components/Settings/GeneralPage.tsx
git commit -m "feat(loop): add loopTimingMode setting to GeneralPage"
```

---

## Task 5：`ExecutionTrace` 加入時間 badge 渲染

**Files:**
- Modify: `src/components/LoopStudio/ExecutionTrace.tsx`

**說明：** 每個 entry 的外層 div 加上 `ls-entry-timed` class 並用 `ls-entry-body` 包裹既有內容，然後在右側加入 `<div className="ls-time-meta">` 放置 badge。不顯示時間的情況 `ls-time-meta` 為 null。

- [ ] **Step 1：在檔案頂端加入 import 與 helper 函式**

在 `import { useState, useEffect, useRef, useCallback } from "react";` 之後加入：

```ts
import { formatTime, formatDuration } from "../../lib/timeFormat";
```

在 `interface ExecutionTraceProps` **之前**加入三個 helper 元件：

```tsx
function ClockBadge({ ts }: { ts: number }) {
  return <span className="ls-tb ls-tb-clock">{formatTime(ts)}</span>;
}

function DurBadge({ start, end, v }: { start: number; end: number; v?: boolean }) {
  return <span className={`ls-tb ${v ? "ls-tb-dur-v" : "ls-tb-dur"}`}>⏱ {formatDuration(start, end)}</span>;
}

function DoneBadge({ ts }: { ts: number }) {
  return <span className="ls-tb ls-tb-done">✓ {formatTime(ts)}</span>;
}
```

- [ ] **Step 2：更新 `ExecutionTraceProps`**

把 `interface ExecutionTraceProps` 改為：

```ts
interface ExecutionTraceProps {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  timingMode: "full" | "compact";
}
```

並更新函式簽名：

```ts
export function ExecutionTrace({ trace, isRunning, iteration, timingMode }: ExecutionTraceProps) {
```

- [ ] **Step 3：更新 `iteration_start` 渲染**

將原本的：
```tsx
if (entry.kind === "iteration_start") {
  return (
    <div key={entry.id} className="ls-trace-iteration">
      <span className="ls-trace-iter-badge">#{entry.iteration}</span>
      <span>Loop Iteration {entry.iteration}</span>
    </div>
  );
}
```

改為：
```tsx
if (entry.kind === "iteration_start") {
  return (
    <div key={entry.id} className="ls-trace-iteration ls-entry-timed">
      <div className="ls-entry-body">
        <span className="ls-trace-iter-badge">#{entry.iteration}</span>
        <span>Loop Iteration {entry.iteration}</span>
      </div>
      <div className="ls-time-meta">
        <ClockBadge ts={entry.timestamp} />
      </div>
    </div>
  );
}
```

- [ ] **Step 4：更新 `verifier_result` 渲染**

將原本外層 div：
```tsx
<div key={entry.id} className={`ls-trace-verifier ${entry.verifierDone ? "done" : "not-done"}`}>
  <div
    className="ls-verifier-header"
    onClick={() => hasDetails && toggle(entry.id)}
    style={{ cursor: hasDetails ? "pointer" : "default" }}
  >
    <span className="ls-verifier-badge">{entry.verifierDone ? "✓ 達成" : "✗ 未達成"}</span>
    <span className="ls-verifier-name">{entry.agentName}</span>
    <span className="ls-verifier-reason">{entry.text}</span>
    {hasDetails && (
      <span className="ls-collapse-toggle">{isCollapsedV ? "▶" : "▼"}</span>
    )}
  </div>
  {/* ...details... */}
</div>
```

改為（在 `ls-trace-verifier` div 內加入 `ls-entry-timed`，header 本身加上 `ls-entry-body`，右側加入時間 meta）：

```tsx
<div key={entry.id} className={`ls-trace-verifier ${entry.verifierDone ? "done" : "not-done"} ls-entry-timed`}>
  <div className="ls-entry-body">
    <div
      className="ls-verifier-header"
      onClick={() => hasDetails && toggle(entry.id)}
      style={{ cursor: hasDetails ? "pointer" : "default" }}
    >
      <span className="ls-verifier-badge">{entry.verifierDone ? "✓ 達成" : "✗ 未達成"}</span>
      <span className="ls-verifier-name">{entry.agentName}</span>
      <span className="ls-verifier-reason">{entry.text}</span>
      {hasDetails && (
        <span className="ls-collapse-toggle">{isCollapsedV ? "▶" : "▼"}</span>
      )}
    </div>
    {hasDetails && !isCollapsedV && vr && (
      <div className="ls-verifier-details">
        {vr.accomplished.length > 0 && (
          <div className="ls-verifier-section">
            <span className="ls-verifier-section-label accomplished">✓ 已完成</span>
            <ul className="ls-verifier-list">
              {vr.accomplished.map((a, i) => <li key={i}>{a}</li>)}
            </ul>
          </div>
        )}
        {vr.remaining.length > 0 && (
          <div className="ls-verifier-section">
            <span className="ls-verifier-section-label remaining">✗ 尚未完成</span>
            <ul className="ls-verifier-list">
              {vr.remaining.map((r, i) => <li key={i}>{r}</li>)}
            </ul>
          </div>
        )}
        {vr.suggestion && (
          <div className="ls-verifier-section">
            <span className="ls-verifier-section-label suggestion">→ 下一步建議</span>
            <p className="ls-verifier-suggestion">{vr.suggestion}</p>
          </div>
        )}
      </div>
    )}
  </div>
  <div className="ls-time-meta">
    {entry.startTimestamp != null && (
      <DurBadge start={entry.startTimestamp} end={entry.timestamp} v />
    )}
    <DoneBadge ts={entry.timestamp} />
  </div>
</div>
```

- [ ] **Step 5：更新 `loop_done` 渲染**

找到第一個 `iteration_start` 的 timestamp 當作 loop 開始時間，計算總耗時。在 `trace.map(entry => { ... })` 外面（`return (` 之前）計算：

```tsx
const loopStartTs = trace.find(e => e.kind === "iteration_start")?.timestamp;
```

然後把 `loop_done` 渲染改為：

```tsx
if (entry.kind === "loop_done") {
  return (
    <div key={entry.id} className="ls-trace-done ls-entry-timed">
      <div className="ls-entry-body">{entry.text}</div>
      <div className="ls-time-meta">
        {loopStartTs != null && (
          <span className="ls-tb ls-tb-total">總耗時 {formatDuration(loopStartTs, entry.timestamp)}</span>
        )}
        <DoneBadge ts={entry.timestamp} />
      </div>
    </div>
  );
}
```

- [ ] **Step 6：更新 `loop_stopped` / `loop_error` 渲染**

```tsx
if (entry.kind === "loop_stopped" || entry.kind === "loop_error") {
  return (
    <div key={entry.id} className={`ls-trace-stopped ${entry.isError ? "error" : ""} ls-entry-timed`}>
      <div className="ls-entry-body">{entry.text}</div>
      <div className="ls-time-meta">
        <ClockBadge ts={entry.timestamp} />
      </div>
    </div>
  );
}
```

- [ ] **Step 7：更新 `sub_agent_start` 渲染**

```tsx
if (entry.kind === "sub_agent_start") {
  return (
    <div key={entry.id} className="ls-trace-agent-start ls-entry-timed">
      <div className="ls-entry-body">
        <span className="ls-agent-badge">{entry.agentName}</span>
        <span className="ls-trace-task">已接收任務，執行中...</span>
      </div>
      <div className="ls-time-meta">
        <ClockBadge ts={entry.timestamp} />
      </div>
    </div>
  );
}
```

- [ ] **Step 8：更新 `sub_agent_action`（工具呼叫）渲染**

精簡版不顯示時間，完整版顯示 ClockBadge：

```tsx
if (entry.kind === "sub_agent_action") {
  const action = entry.actions?.[0];
  if (!action) return null;
  return (
    <div key={entry.id} className={`ls-trace-live-action ${action.isError ? "error" : ""} ls-entry-timed`}>
      <div className="ls-entry-body">
        <span className="ls-agent-badge">{entry.agentName}</span>
        <div className="ls-trace-action">
          <span className="ls-action-tool">{action.tool}</span>
          <pre className="ls-action-input">{action.input}</pre>
          <pre className="ls-action-output">{action.output.slice(0, 500)}{action.output.length > 500 ? "\n..." : ""}</pre>
        </div>
      </div>
      {timingMode === "full" && (
        <div className="ls-time-meta">
          <ClockBadge ts={entry.timestamp} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 9：更新 `sub_agent_done` 渲染**

```tsx
if (entry.kind === "sub_agent_done") {
  return (
    <div key={entry.id} className={`ls-trace-agent-done ${entry.isError ? "error" : ""} ls-entry-timed`}>
      <div className="ls-entry-body">
        <div className="ls-trace-collapsible-header" style={{ cursor: "default" }}>
          <span className="ls-agent-done-badge">{entry.isError ? "✗" : "✓"}</span>
          <span className="ls-agent-badge">{entry.agentName}</span>
          <span className="ls-trace-answer">{entry.text.slice(0, 120)}{entry.text.length > 120 ? "..." : ""}</span>
        </div>
      </div>
      <div className="ls-time-meta">
        {entry.startTimestamp != null && (
          <DurBadge start={entry.startTimestamp} end={entry.timestamp} />
        )}
        <DoneBadge ts={entry.timestamp} />
      </div>
    </div>
  );
}
```

- [ ] **Step 10：更新 `orchestrator_action` 渲染**

精簡版不顯示時間，完整版顯示 ClockBadge：

```tsx
if (entry.kind === "orchestrator_action") {
  return (
    <div key={entry.id} className="ls-trace-orchestrator ls-entry-timed">
      <div className="ls-entry-body">
        <span className="ls-orch-badge">Orchestrator</span>
        <p className="ls-orch-text">{entry.text.slice(0, 300)}{entry.text.length > 300 ? "..." : ""}</p>
      </div>
      {timingMode === "full" && (
        <div className="ls-time-meta">
          <ClockBadge ts={entry.timestamp} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 11：型別檢查**

```bash
npx tsc --noEmit
```
預期：無輸出

- [ ] **Step 12：Commit**

```bash
git add src/components/LoopStudio/ExecutionTrace.tsx
git commit -m "feat(loop): render timing badges in ExecutionTrace"
```

---

## Task 6：`LoopStudioView` 讀取設定並傳入 `ExecutionTrace`

**Files:**
- Modify: `src/components/LoopStudio/index.tsx`

- [ ] **Step 1：新增 `timingMode` state**

在 `src/components/LoopStudio/index.tsx` 的 `LoopStudioView` 函式內，在現有 `useState` 宣告末尾加入：

```ts
const [timingMode, setTimingMode] = useState<"full" | "compact">(
  () => (localStorage.getItem("loopTimingMode") as "full" | "compact" | null) ?? "compact"
);
```

- [ ] **Step 2：監聽 localStorage 變更（設定頁同視窗更新）**

在現有 `useEffect` 群組末尾加入（與其他 `useEffect` 並列）：

```ts
useEffect(() => {
  const sync = () => {
    setTimingMode(
      (localStorage.getItem("loopTimingMode") as "full" | "compact" | null) ?? "compact"
    );
  };
  window.addEventListener("storage", sync);
  return () => window.removeEventListener("storage", sync);
}, []);
```

> 注意：`storage` 事件只在**跨分頁**變更時觸發。同一視窗內（Settings → Loop Studio）不觸發；這是可接受的行為，使用者切換設定後下次啟動即生效，或重新載入 Loop Studio tab。

- [ ] **Step 3：傳入 `ExecutionTrace`**

找到（約第 738 行）：
```tsx
<ExecutionTrace trace={loop.trace} isRunning={loop.isRunning} iteration={loop.iteration} />
```

改為：
```tsx
<ExecutionTrace
  trace={loop.trace}
  isRunning={loop.isRunning}
  iteration={loop.iteration}
  timingMode={timingMode}
/>
```

- [ ] **Step 4：型別檢查與測試**

```bash
npx tsc --noEmit
npm run test -- --run
```
預期：型別無錯誤；所有現有測試 PASS

- [ ] **Step 5：Commit**

```bash
git add src/components/LoopStudio/index.tsx
git commit -m "feat(loop): wire timingMode from localStorage into ExecutionTrace"
```

---

## Task 7：手動驗收

- [ ] **Step 1：啟動開發環境**

```bash
npm run tauri:dev
```

- [ ] **Step 2：精簡版驗收（預設）**

1. 開啟 Loop Studio，執行一個 Loop
2. 確認每個 `iteration_start` 右側有灰色時鐘 badge（`14:22:01`）
3. 確認 `sub_agent_start` 右側有灰色時鐘 badge
4. 確認 `sub_agent_action`（工具呼叫行）右側**無 badge**
5. 確認 `sub_agent_done` 右側有綠色耗時 badge（`⏱ 7s`）+ 藍紫完成時間 badge（`✓ 14:22:11`）
6. 確認 `verifier_result` 右側有紫色耗時 badge + 藍紫完成時間 badge
7. 確認 `loop_done` 右側有綠色粗體總耗時（`總耗時 Xs`）+ 藍紫完成時間

- [ ] **Step 3：完整版驗收**

1. 開啟設定 → 一般，切換為「完整版」
2. 重新執行 Loop（或 resume）
3. 確認 `orchestrator_action` 與 `sub_agent_action` 右側現在也出現灰色時鐘 badge

- [ ] **Step 4：舊 session resume 不崩潰**

1. 從過去 Sessions 選一個舊 session Resume
2. 確認介面正常顯示，既有 entry 的 `startTimestamp` 為 `undefined` 時 badge 不顯示（不崩潰）

- [ ] **Step 5：最終 Commit**

```bash
git push origin master
```

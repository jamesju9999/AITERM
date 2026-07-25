# Agent 狀態列 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `/agent` 的生命週期協調狀態（思考/詢問AI/第N步/網路/完成/停止）從 `term.write` 移到輸入框上方的專屬 React 狀態列，根治 live-frame 裁切與完成後留白。

**Architecture:** 新增 `AgentStatusBar` 元件顯示狀態；`runAgentLoop`/`handleAiQuery` 改用 `onPhase` callback 推狀態到 TerminalView 的 `agentPhase` state，取代原本的狀態 `term.write`；實際指令與輸出照舊經 PTY 進終端機 blocks。一併還原先前兩個暫時補丁。

**Tech Stack:** React 19、TypeScript、xterm.js、Vitest + React Testing Library。

**Spec:** `docs/superpowers/specs/2026-07-25-agent-status-bar-design.md`

---

## 檔案結構

- **新增** `src/components/AgentStatusBar.tsx` — 狀態列元件 + 匯出 `AgentPhase` 型別
- **新增** `src/components/AgentStatusBar.css` — 樣式（主題感知）
- **新增** `src/components/AgentStatusBar.test.tsx` — RTL 測試
- **修改** `src/lib/i18n.ts` — 新增 `term_agent_status_*` 字串（en + zh-TW）
- **修改** `src/components/TerminalView.tsx` — `agentPhase` state、`onPhase` threading、渲染、還原兩補丁、移除狀態 `term.write`

---

## Task 1: i18n 字串

**Files:**
- Modify: `src/lib/i18n.ts`（zh-TW 區塊約 950–958；en 區塊約 1839–1847）

- [ ] **Step 1: 在 zh-TW 的 `term_agent_thinking` 那行後面新增字串**

在 `src/lib/i18n.ts` 找到 zh-TW 的這行：
```ts
    term_agent_thinking: (step: number, max: number) => `[Agent: 思考下一步... (${step}/${max})]`,
```
在其後緊接新增：
```ts
    term_agent_status_asking: "詢問 AI 中…",
    term_agent_status_running: (command: string) => `執行中：${command}`,
    term_agent_status_web_search: (query: string) => `搜尋：${query}`,
    term_agent_status_web_fetch: (url: string) => `取得：${url}`,
    term_agent_status_step: (step: number, max: number) => `步驟 ${step}/${max}`,
    term_agent_status_done: (steps: number) => `完成（${steps} 步）`,
    term_agent_status_failed: (reason: string) => `已停止：${reason}`,
    term_agent_status_dismiss: "關閉",
```

- [ ] **Step 2: 在 en 的 `term_agent_thinking` 那行後面新增對應字串**

找到 en 的這行：
```ts
    term_agent_thinking: (step: number, max: number) => `[Agent: Thinking... (${step}/${max})]`,
```
在其後緊接新增：
```ts
    term_agent_status_asking: "Asking AI…",
    term_agent_status_running: (command: string) => `Running: ${command}`,
    term_agent_status_web_search: (query: string) => `Searching: ${query}`,
    term_agent_status_web_fetch: (url: string) => `Fetching: ${url}`,
    term_agent_status_step: (step: number, max: number) => `Step ${step}/${max}`,
    term_agent_status_done: (steps: number) => `Completed (${steps} steps)`,
    term_agent_status_failed: (reason: string) => `Stopped: ${reason}`,
    term_agent_status_dismiss: "Dismiss",
```

- [ ] **Step 3: 型別檢查**

Run: `npx tsc --noEmit`
Expected: exit 0（無錯誤）

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "i18n: add agent status bar strings (en + zh-TW)"
```

---

## Task 2: AgentStatusBar 元件（TDD）

**Files:**
- Create: `src/components/AgentStatusBar.tsx`
- Create: `src/components/AgentStatusBar.css`
- Test: `src/components/AgentStatusBar.test.tsx`

- [ ] **Step 1: 先寫失敗測試**

Create `src/components/AgentStatusBar.test.tsx`:
```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";
import { AgentStatusBar, type AgentPhase } from "./AgentStatusBar";

function renderBar(status: AgentPhase, onDismiss = vi.fn()) {
  render(
    <LocaleProvider>
      <AgentStatusBar status={status} onDismiss={onDismiss} />
    </LocaleProvider>,
  );
  return onDismiss;
}

describe("AgentStatusBar", () => {
  it("shows the step counter and command for the running phase", () => {
    renderBar({ phase: "running", step: 2, maxSteps: 5, command: "ls -la" });
    expect(screen.getByText(/ls -la/)).toBeInTheDocument();
    expect(screen.getByText("步驟 2/5")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("shows the query for the web phase", () => {
    renderBar({ phase: "web", step: 1, maxSteps: 5, query: "weather taipei", webKind: "search" });
    expect(screen.getByText(/weather taipei/)).toBeInTheDocument();
  });

  it("renders a dismiss button for the done phase and fires onDismiss", async () => {
    const onDismiss = renderBar({ phase: "done", steps: 3 });
    const btn = screen.getByRole("button", { name: "關閉" });
    await userEvent.click(btn);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("renders the reason and a dismiss button for the failed phase", () => {
    renderBar({ phase: "failed", reason: "指令逾時" });
    expect(screen.getByText(/指令逾時/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關閉" })).toBeInTheDocument();
  });

  it("does not show a step counter for done", () => {
    renderBar({ phase: "done", steps: 3 });
    expect(screen.queryByText(/步驟/)).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npm run test -- --run src/components/AgentStatusBar.test.tsx`
Expected: FAIL（`Cannot find module './AgentStatusBar'`）

- [ ] **Step 3: 建立元件**

Create `src/components/AgentStatusBar.tsx`:
```tsx
import { useLocale } from "../contexts/LocaleContext";
import "./AgentStatusBar.css";

export type AgentPhase =
  | { phase: "asking"; step: number; maxSteps: number }
  | { phase: "running"; step: number; maxSteps: number; command: string }
  | { phase: "web"; step: number; maxSteps: number; query: string; webKind: "search" | "fetch" }
  | { phase: "done"; steps: number }
  | { phase: "failed"; reason: string };

interface AgentStatusBarProps {
  status: AgentPhase;
  onDismiss: () => void;
}

export function AgentStatusBar({ status, onDismiss }: AgentStatusBarProps) {
  const { t } = useLocale();

  let icon: string;
  let text: string;
  switch (status.phase) {
    case "asking":
      icon = "◐";
      text = t.term_agent_status_asking;
      break;
    case "running":
      icon = "▶";
      text = t.term_agent_status_running(status.command);
      break;
    case "web":
      icon = status.webKind === "search" ? "🔍" : "📄";
      text = status.webKind === "search"
        ? t.term_agent_status_web_search(status.query)
        : t.term_agent_status_web_fetch(status.query);
      break;
    case "done":
      icon = "✅";
      text = t.term_agent_status_done(status.steps);
      break;
    case "failed":
      icon = "⚠";
      text = t.term_agent_status_failed(status.reason);
      break;
  }

  const showStep = status.phase === "asking" || status.phase === "running" || status.phase === "web";
  const dismissible = status.phase === "done" || status.phase === "failed";
  const pulsing = status.phase === "asking" || status.phase === "running" || status.phase === "web";

  return (
    <div
      className={`aiterm-agent-status aiterm-agent-status--${status.phase}`}
      role="status"
      aria-live="polite"
    >
      <span
        className={`aiterm-agent-status__icon${pulsing ? " aiterm-agent-status__icon--pulse" : ""}`}
        aria-hidden="true"
      >
        {icon}
      </span>
      <span className="aiterm-agent-status__text">{text}</span>
      {showStep && (
        <span className="aiterm-agent-status__step">
          {t.term_agent_status_step(status.step, status.maxSteps)}
        </span>
      )}
      {dismissible && (
        <button
          type="button"
          className="aiterm-agent-status__dismiss"
          onClick={onDismiss}
          aria-label={t.term_agent_status_dismiss}
        >
          ✕
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 建立樣式**

Create `src/components/AgentStatusBar.css`:
```css
.aiterm-agent-status {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 6px 8px;
  padding: 8px 12px;
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  background: rgba(120, 120, 140, 0.10);
  font-size: 13px;
  line-height: 1.4;
  box-sizing: border-box;
}

.aiterm-agent-status__icon {
  flex-shrink: 0;
}

.aiterm-agent-status__icon--pulse {
  animation: aiterm-agent-status-pulse 1.1s ease-in-out infinite;
}

@keyframes aiterm-agent-status-pulse {
  0%, 100% { opacity: 0.45; }
  50% { opacity: 1; }
}

.aiterm-agent-status__text {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.aiterm-agent-status__step {
  flex-shrink: 0;
  opacity: 0.7;
  font-variant-numeric: tabular-nums;
}

.aiterm-agent-status__dismiss {
  flex-shrink: 0;
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  opacity: 0.6;
  padding: 0 4px;
  font-size: 13px;
}
.aiterm-agent-status__dismiss:hover { opacity: 1; }

.aiterm-agent-status--done {
  border-color: rgba(52, 199, 89, 0.4);
  background: rgba(52, 199, 89, 0.12);
}
.aiterm-agent-status--failed {
  border-color: rgba(255, 179, 64, 0.45);
  background: rgba(255, 179, 64, 0.12);
}

:root[data-theme="light"] .aiterm-agent-status {
  border-color: rgba(0, 0, 0, 0.10);
  background: rgba(0, 0, 0, 0.04);
}
@media (prefers-color-scheme: light) {
  :root:not([data-theme="dark"]) .aiterm-agent-status {
    border-color: rgba(0, 0, 0, 0.10);
    background: rgba(0, 0, 0, 0.04);
  }
}
```

- [ ] **Step 5: 執行測試確認通過**

Run: `npm run test -- --run src/components/AgentStatusBar.test.tsx`
Expected: PASS（5 tests）

- [ ] **Step 6: Commit**

```bash
git add src/components/AgentStatusBar.tsx src/components/AgentStatusBar.css src/components/AgentStatusBar.test.tsx
git commit -m "feat(agent): add AgentStatusBar component for out-of-terminal status"
```

---

## Task 3: Producer 端 — `onPhase` threading，移除狀態 term.write

**Files:**
- Modify: `src/components/TerminalView.tsx`（`handleAiQuery` 約 1430–1530；`AgentLoopParams` 約 1591–1611；`runAgentLoop` 約 1614、1666–1699）

> 注意：先前對話留下的兩個暫時補丁也在這些位置（`clearThinkingLine` 參數與 DONE/error 兩處額外 `\x1b[1A\x1b[2K`）。本 Task 會一併移除它們（spec 第 5 點）。用下列「唯一字串」定位，不要依賴行號。

- [ ] **Step 1: 匯入 `AgentPhase` 型別**

在 `src/components/TerminalView.tsx` 找到：
```tsx
import { StreamingIndicator } from "./StreamingIndicator";
```
在其後新增：
```tsx
import { AgentStatusBar, type AgentPhase } from "./AgentStatusBar";
```

- [ ] **Step 2: `AgentLoopParams` 新增 `onPhase`**

找到：
```tsx
  /** Fires after each shell command finishes; used to mirror progress to Telegram. */
  onStepComplete?: (info: AgentStepInfo) => void;
}
```
改成：
```tsx
  /** Fires after each shell command finishes; used to mirror progress to Telegram. */
  onStepComplete?: (info: AgentStepInfo) => void;
  /** Pushes agent lifecycle status to the React status bar (replaces term.write status lines). */
  onPhase?: (update: AgentPhase) => void;
}
```

- [ ] **Step 3: `handleAiQuery` 簽名 — 移除 `clearThinkingLine`，改加 `onPhase` + 步驟資訊**

找到 `handleAiQuery` 參數尾端（先前補丁加的區塊）：
```tsx
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void,
  /**
   * True when the caller (agent loop) printed a "[Agent: 思考下一步...]" line
   * just above this query. On the paths that DON'T print a following command
   * (DONE / error), that optimistic line is now stale and must be erased —
   * otherwise it lingers in the buffer until the next command redraws it.
   */
  clearThinkingLine = false,
) {
  void originalLine;
```
改成：
```tsx
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void,
  onPhase?: (update: AgentPhase) => void,
  agentStep = 0,
  agentMaxSteps = 0,
) {
  void originalLine;
```

- [ ] **Step 4: `handleAiQuery` — 移除 `→ asking AI...` 的寫入**

找到：
```tsx
  void originalLine;
  term.write("\r\x1b[2K");
  term.write("→ asking AI...\r\n");
  setStreamText("");
```
改成（刪掉那兩行 term.write）：
```tsx
  void originalLine;
  setStreamText("");
```

- [ ] **Step 5: `handleAiQuery` — DONE 路徑移除清除序列與 `clearThinkingLine`**

找到：
```tsx
    .then((resp) => {
      streamingRef.current = false;
      term.write("\x1b[1A\x1b[2K");
      
      if (resp.command === "DONE") {
        // Cursor is directly below the optimistic thinking line (the "→ asking
        // AI..." helper above was just cleared). No command follows, so erase it.
        if (clearThinkingLine) term.write("\x1b[1A\x1b[2K");
        setPreview(INITIAL_PREVIEW);
        if (onDone) onDone(resp.explanation);
        return;
      }
```
改成：
```tsx
    .then((resp) => {
      streamingRef.current = false;

      if (resp.command === "DONE") {
        setPreview(INITIAL_PREVIEW);
        if (onDone) onDone(resp.explanation);
        return;
      }
```

- [ ] **Step 6: `handleAiQuery` — auto-execute 分支發出 `running` phase**

找到：
```tsx
      if (shouldAutoExecute(mode, risk, agentActive)) {
        // Auto-execute: write a subtle confirmation line then submit.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`\r\n${riskColor}▶ ${resp.command}\x1b[0m\r\n`);
        // Pass onCommandComplete so the block hook calls it when OSC 133;D fires
        submitCommand(resp.command, onCommandComplete);
        setPreview(INITIAL_PREVIEW);
      } else {
```
改成（保留 `▶ command` 寫入，新增 onPhase）：
```tsx
      if (shouldAutoExecute(mode, risk, agentActive)) {
        // Auto-execute: write a subtle confirmation line then submit.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`\r\n${riskColor}▶ ${resp.command}\x1b[0m\r\n`);
        onPhase?.({ phase: "running", step: agentStep, maxSteps: agentMaxSteps, command: resp.command });
        // Pass onCommandComplete so the block hook calls it when OSC 133;D fires
        submitCommand(resp.command, onCommandComplete);
        setPreview(INITIAL_PREVIEW);
      } else {
```

- [ ] **Step 7: `handleAiQuery` — error 路徑移除清除序列與 `clearThinkingLine`**

找到：
```tsx
    .catch((rawErr: unknown) => {
      streamingRef.current = false;
      setStreamText("");
      term.write("\x1b[1A\x1b[2K");
      // Same as the DONE path: no command follows, so drop the stale thinking line.
      if (clearThinkingLine) term.write("\x1b[1A\x1b[2K");
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));
```
改成：
```tsx
    .catch((rawErr: unknown) => {
      streamingRef.current = false;
      setStreamText("");
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));
```

- [ ] **Step 8: `runAgentLoop` — 用 `onPhase` asking 取代思考行 term.write**

找到：
```tsx
  if (stepCount > 0) {
    term.write(`\r\n\x1b[35m${t.term_agent_thinking(stepCount + 1, maxSteps)}\x1b[0m\r\n`);
  }
```
改成（每一步都發 asking，含第 1 步）：
```tsx
  params.onPhase?.({ phase: "asking", step: stepCount + 1, maxSteps });
```

- [ ] **Step 9: `runAgentLoop` — `onWebAction` 用 `onPhase` web 取代 term.write 標籤**

找到：
```tsx
  const onWebAction = (type: "search" | "fetch", value: string) => {
    stepResolved = true; // prevent timeout from firing while waiting for web result
    const label = type === "search" ? `\x1b[36m🔍 搜尋: ${value}\x1b[0m` : `\x1b[36m📄 取得: ${value}\x1b[0m`;
    term.write(`\r\n${label}\r\n`);
    const webPromise = type === "search" ? webSearch(value) : webFetch(value);
```
改成：
```tsx
  const onWebAction = (type: "search" | "fetch", value: string) => {
    stepResolved = true; // prevent timeout from firing while waiting for web result
    params.onPhase?.({ phase: "web", step: stepCount + 1, maxSteps, query: value, webKind: type });
    const webPromise = type === "search" ? webSearch(value) : webFetch(value);
```

- [ ] **Step 10: `runAgentLoop` — 呼叫 `handleAiQuery` 時傳入新引數**

找到（先前補丁加的那行 `stepCount > 0,`）：
```tsx
    onWebAction,          // onWebAction: intercept web search/fetch commands
    stepCount > 0,        // clearThinkingLine: a thinking line was printed above (line ~1614)
  );
}
```
改成：
```tsx
    onWebAction,          // onWebAction: intercept web search/fetch commands
    params.onPhase,       // onPhase: push running-phase status to the React status bar
    stepCount + 1,        // agentStep (1-based)
    maxSteps,             // agentMaxSteps
  );
}
```

- [ ] **Step 11: 型別檢查**

Run: `npx tsc --noEmit`
Expected: exit 0

> 若出現 `onPhase` 未被任何呼叫者提供的錯誤，屬正常（Task 4 才接上）；因為 `onPhase?` 為選用，tsc 不應報錯。

- [ ] **Step 12: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "refactor(agent): emit lifecycle status via onPhase, drop status term.write

Replaces the thinking/asking-AI/web term.write lines with onPhase callbacks
and removes the temporary clearThinkingLine patch. Keeps the auto-exec
'▶ command' echo and real command output in the terminal."
```

---

## Task 4: Consumer 端 — `agentPhase` state、渲染、還原 liveRows 補丁

**Files:**
- Modify: `src/components/TerminalView.tsx`（state 區約 326；liveRows 補丁約 331–345；3 個 `runAgentLoop` 呼叫站點的 `onComplete`/`onFail`：約 460–473、921–932、1360–1369；渲染區約 1338–1341；WarpInput onSubmit 約 1344）

- [ ] **Step 1: 新增 `agentPhase` state 與 `agentStepRef`；還原 mission-end liveRows 補丁**

找到（先前補丁）：
```tsx
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // When an agent mission ends, its final "[Agent Mission Completed]"/"⚠ stopped"
  // message is written straight into the live pane via term.write. Unlike real PTY
  // output that never trips the MAX_LIVE_ROWS snap in onPtyData, so at MIN_LIVE_ROWS
  // that message lands on the last visible row and gets clipped by the frame's
  // bottom edge. Expand the pane on the active→inactive edge so it shows in full;
  // the next command creates a block and shrinks it back to MIN via the effect above.
  const prevMissionActiveRef = useRef(false);
  const missionActive = agentMission?.active ?? false;
  useEffect(() => {
    if (prevMissionActiveRef.current && !missionActive) setLiveRows(MAX_LIVE_ROWS);
    prevMissionActiveRef.current = missionActive;
  }, [missionActive]);
```
改成（移除 liveRows 補丁，改為 agentPhase state）：
```tsx
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // Agent lifecycle status shown in the AgentStatusBar above the input, driven by
  // runAgentLoop/handleAiQuery via the onPhase callback (see handleAgentPhase).
  const [agentPhase, setAgentPhase] = useState<AgentPhase | null>(null);
  const agentStepRef = useRef(0);
  const handleAgentPhase = useCallback((update: AgentPhase) => {
    if (update.phase === "asking" || update.phase === "running" || update.phase === "web") {
      agentStepRef.current = update.step;
    }
    setAgentPhase(update);
  }, []);
```

> `useCallback` 需在 import 清單。找到 `TerminalView.tsx` 頂部的 `import { ... } from "react";`，確認含 `useCallback`；若無則加入。

- [ ] **Step 2: 站點 A（Telegram remote，約 460–473）— 傳入 onPhase、done/failed 取代 term.write**

找到：
```tsx
            onComplete: (explanation?: string) => {
              if (explanation) {
                termRef.current?.write(`\r\n\x1b[36m${explanation.replace(/\n/g, "\r\n")}\x1b[0m\r\n`);
              }
              termRef.current?.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(explanation ? `Agent: ${explanation}` : "[Agent Mission Completed] 🎉");
            },
            onFail: (msg) => {
              termRef.current?.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
            },
```
改成：
```tsx
            onPhase: handleAgentPhase,
            onComplete: (explanation?: string) => {
              setAgentPhase({ phase: "done", steps: agentStepRef.current });
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(explanation ? `Agent: ${explanation}` : "[Agent Mission Completed] 🎉");
            },
            onFail: (msg) => {
              setAgentPhase({ phase: "failed", reason: msg });
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
            },
```

- [ ] **Step 3: 站點 B（約 921–932）— 同樣處理**

找到：
```tsx
              onComplete: () => {
                term.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse("[Agent Mission Completed] 🎉");
              },
              onFail: (msg) => {
                term.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
              },
              onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
```
改成：
```tsx
              onPhase: handleAgentPhase,
              onComplete: () => {
                setAgentPhase({ phase: "done", steps: agentStepRef.current });
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse("[Agent Mission Completed] 🎉");
              },
              onFail: (msg) => {
                setAgentPhase({ phase: "failed", reason: msg });
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
              },
              onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
```

- [ ] **Step 4: 站點 C（約 1360–1369）— 同樣處理**

找到：
```tsx
                  onComplete: () => {
                    termRef.current?.write(`\r\n\x1b[32m[Agent Mission Completed] 🎉\x1b[0m\r\n`);
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                  onFail: (msg) => {
                    termRef.current?.write(`\r\n\x1b[33m⚠ Agent stopped: ${msg}\x1b[0m\r\n`);
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                  onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
```
改成：
```tsx
                  onPhase: handleAgentPhase,
                  onComplete: () => {
                    setAgentPhase({ phase: "done", steps: agentStepRef.current });
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                  onFail: (msg) => {
                    setAgentPhase({ phase: "failed", reason: msg });
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                  },
                  onStepComplete: (info) => sendRemoteResponse(formatAgentStepForRemote(info)),
```

- [ ] **Step 5: 渲染 — AgentStatusBar 置於輸入框上方，agent 模式優先於 StreamingIndicator**

找到：
```tsx
      {!isAlternateBuffer && (
        preview.loading ? (
          <StreamingIndicator visible text={streamText} />
        ) : (
        <WarpInput
```
改成：
```tsx
      {!isAlternateBuffer && agentPhase && (
        <AgentStatusBar status={agentPhase} onDismiss={() => setAgentPhase(null)} />
      )}
      {!isAlternateBuffer && (
        preview.loading && !agentPhase ? (
          <StreamingIndicator visible text={streamText} />
        ) : (
        <WarpInput
```

- [ ] **Step 6: 送出下一個指令時自動關閉狀態列**

找到 WarpInput 的 `onSubmit={(cmd) => {`（約 1344）緊接的第一行，在其最前面插入清除：
```tsx
          onSubmit={(cmd) => {
            setAgentPhase(null);
            const agentQuery = parseAgentPrefix(cmd);
```

> 注意：若送出的是新的 `/agent`，`runAgentLoop` 會立即以 `onPhase` 設回 asking，故先清空無害。

- [ ] **Step 7: 型別檢查與 lint**

Run: `npx tsc --noEmit`
Expected: exit 0

Run: `npx eslint src/components/TerminalView.tsx src/components/AgentStatusBar.tsx`
Expected: 不新增錯誤（TerminalView 既有基準為 9 個既有錯誤，數量不得增加；AgentStatusBar 應為 0）

- [ ] **Step 8: Commit**

```bash
git add src/components/TerminalView.tsx
git commit -m "feat(agent): render AgentStatusBar, wire onPhase, revert liveRows patch

Adds agentPhase state driven by handleAgentPhase, shows the status bar above
the input, sets done/failed at the mission callbacks (no more completion
term.write), dismisses on next submit or ✕, and removes the temporary
mission-end setLiveRows(MAX) effect."
```

---

## Task 5: 驗證

**Files:** 無（僅執行驗證）

- [ ] **Step 1: 全套前端測試**

Run: `npm run test -- --run`
Expected: 全數通過（既有 286 + AgentStatusBar 5 = 291）

- [ ] **Step 2: 型別檢查**

Run: `npx tsc --noEmit`
Expected: exit 0

- [ ] **Step 3: Lint 基準比對**

Run: `npx eslint src/components/TerminalView.tsx 2>&1 | grep -c error`
Expected: `9`（與修改前相同，未新增）

- [ ] **Step 4: 實機手動驗證（`npm run tauri:dev`）**

依 spec「測試」節逐項確認：
1. 跑 `/agent` 任務，終端機**不再**出現 `思考下一步` / `→ asking AI` / `[Agent Mission Completed]` / `⚠ Agent stopped` / `🔍 搜尋` / `📄 取得` 等狀態文字。
2. 執行期間狀態列在輸入框上方顯示 asking / running / step N/M；live-frame **無裁切**、任務結束後**無留白**。
3. 完成顯示 `✅ 完成（N 步）`；送出下一個指令或點 ✕ 後關閉；失敗顯示 `⚠ 已停止：<原因>`。
4. 輸入/回顯正常、無游標錯位。
5. 單次 `/ai`（非 agent）行為不變（仍顯示 StreamingIndicator）。

- [ ] **Step 5: 最終確認**

使用 `superpowers:verification-before-completion` 技能，貼出上述指令的實際輸出後，再宣告完成。

---

## Self-Review（撰寫者對照 spec）

- **Spec §1 元件**：Task 2 建立 `AgentStatusBar`（含五 phase）✓
- **Spec §2 state/渲染**：Task 4 Step 1/5/6（agentPhase、優先於 StreamingIndicator、dismiss）✓
- **Spec §3 資料流 onPhase**：Task 3（asking/running/web）+ Task 4（done/failed）✓
- **Spec §4 移除 term.write**：Task 3 Step 4/5/7/8/9（asking/web/清除序列）+ Task 4 Step 2/3/4（completed/stopped）✓；保留 `▶ command`（Task 3 Step 6 保留該行）✓
- **Spec §5 還原兩補丁**：clearThinkingLine（Task 3 Step 3/5/7/10）+ liveRows effect（Task 4 Step 1）✓
- **Spec §6 錯誤處理**：onFail → failed（Task 4 Step 2–4）✓
- **Spec §7 測試**：Task 2 測試 + Task 5 全套/tsc/lint/手動 ✓
- **型別一致性**：`AgentPhase` union 於 Task 2 定義並匯出，Task 3/4 一致使用 `{ phase, step, maxSteps, command/query/webKind/steps/reason }` 欄位 ✓
- **Placeholder 掃描**：無 TBD/TODO，每個程式步驟均含完整程式碼 ✓

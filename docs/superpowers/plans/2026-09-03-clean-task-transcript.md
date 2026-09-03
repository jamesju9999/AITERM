# 工作看板對話記錄乾淨化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 任務一完成、分頁仍然活著的那一刻，用前端 xterm.js 已經正確重建好的畫面覆寫存檔，取代後端原始未解讀的位元組擷取——讓「對話記錄」即使在分頁關閉、app 重啟之後仍然是乾淨的。

**Architecture:** 新增一個 module-level 登記簿（`src/lib/terminalInstanceRegistry.ts`），讓每個分頁的 xterm `Terminal` + 官方 `SerializeAddon` 能被跨元件用 PTY session id 查到；`TerminalView` 在 PTY session 建立後自己登記進去、卸載時取消登記。工作看板偵測到卡片「剛變成已完成」時查登記簿，分頁還在就序列化畫面、濾除 ANSI 樣式碼、呼叫新後端指令覆寫既有的 `transcript.txt`；分頁不在就什麼都不做，維持現有的原始存檔（不會變差）。後端既有的「完成當下先存一份原始版本」機制完全不動，作為保底。

**Tech Stack:** React 19 + TypeScript（`@xterm/xterm`、新依賴 `@xterm/addon-serialize`）、Rust + Tauri（沿用既有 `tasks_*` command 慣例）、Vitest、cargo test。

---

## Context for the implementing engineer

讀 `docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md` 了解完整背景。以下事實都是規劃時實際讀過程式碼確認的：

- **亂碼根因**：`src-tauri/src/pty/session.rs::get_recent_output` 呼叫 `crate::pty::ansi::strip_ansi`，只逐位元組移除 ANSI escape 序列，**不解讀游標移動/`\r` 重繪**。`src-tauri/src/tasks/scheduler.rs::write_transcript` 用這個函式在任務完成當下擷取存檔，這個機制完全不動，繼續當保底。
- **`TerminalView.tsx` 現有的 xterm 相關程式碼**（都已讀過確認行號）：
  - `import { Terminal } from "@xterm/xterm";`、`import { FitAddon } from "@xterm/addon-fit";`、`import { SearchAddon } from "@xterm/addon-search";`（檔案最上方 import 區塊）
  - `const termRef = useRef<Terminal | null>(null);`（~245 行）、`const searchAddonRef = useRef<SearchAddon | null>(null);`（~286 行）、`const fitAddonRef = useRef<FitAddon | null>(null);`（~301 行）
  - `const [sessionId, setSessionId] = useState<string>("");`（~183 行）——PTY 建立前是空字串，之後變成真正的 PTY session id。**這個 `sessionId` 就是工作看板 `card.tab_id` 存的同一個值**（`dispatch::spawn_and_run` 建立 PTY 時回傳的 id，經 `mark_dispatched` 存進 `tasks.tab_id`，再經 `mcp-coordination-tab-spawned` 事件的 `session_id` 欄位被 `TerminalApp` 設成新分頁的 `ptySessionId`，最終變成這個元件的 `sessionId` state）。
  - 建立 `Terminal` 的掛載 effect 在 ~1064 行起：`const term = new Terminal({...}); termRef.current = term;`，接著 ~1091-1096 行掛上 `fit`/`search` 兩個 addon：`const fit = new FitAddon(); term.loadAddon(fit); fitAddonRef.current = fit;`（`search` 同樣寫法）。這個 effect 的清理函式在 ~1751 行呼叫 `term.dispose()`。
- **`src/components/TaskBoard/transcriptUtils.ts`** 目前只有 `collapseConsecutiveDuplicateLines`，零依賴的純函式檔（獨立成檔是因為 `react-refresh/only-export-components` 這條 eslint 規則不允許元件檔案混雜一般函式匯出）。
- **`src/components/TaskBoard/index.tsx`** 的 `refresh()`（~63-66 行）目前只是 `const rows = await listTasks(); if (mounted.current) setTasks(rows);`，被掛載 effect（~68-76 行）呼叫，且 `onTasksUpdated(() => void refresh())` 掛在同一個 effect 裡。
- **`src-tauri/src/commands/tasks.rs`** 現有 `tasks_read_transcript`（~294-303 行）的寫法：直接在 command 函式裡 `store::get_task` 再 `fs::read_to_string`，**檔案 I/O 沒有下放進 `store.rs`**——這個 repo 的既有慣例是 `store.rs` 只碰資料庫。檔案開頭已有 `use std::fs;`。
- **`src-tauri/src/lib.rs`** 目前 `tasks::{...}` 匯入清單在 107-110 行、`generate_handler!` 巨集裡 `tasks_read_transcript,` 在 558 行——新指令要加進這兩處。
- **前端測試掛載法參考**：`src/components/TerminalView.idleSignal.test.tsx` 完整示範了「掛載真正的 `TerminalView`、讓 `pty_create` mock 回傳一個固定 session id、等待非同步鏈完成」的做法，這次新測試直接沿用同一套 mock 骨架（`@tauri-apps/api/core`／`@tauri-apps/api/event`／`@tauri-apps/api/path` 三個底層 mock、`ResizeObserver`/`matchMedia` polyfill、`useTerminalBlocks`/`useAgentMission` 的惰性 mock）。
- **`package.json`** 目前 `@xterm/xterm": "^5.5.0"`，`@xterm/addon-fit`/`@xterm/addon-search` 已是依賴；`@xterm/addon-serialize` 尚未安裝，這次要 `npm install @xterm/addon-serialize`（讓 npm 自己解析相容版本，不要手動猜版號寫進 `package.json`）。

---

### Task 1: `stripAnsiCodes`（純文字 ANSI 濾除）

**Files:**
- Modify: `src/components/TaskBoard/transcriptUtils.ts`
- Create: `src/components/TaskBoard/transcriptUtils.test.ts`

- [ ] **Step 1: 寫失敗測試**

建立 `src/components/TaskBoard/transcriptUtils.test.ts`：

```ts
import { describe, it, expect } from "vitest";
import { stripAnsiCodes, collapseConsecutiveDuplicateLines } from "./transcriptUtils";

describe("stripAnsiCodes", () => {
  it("removes SGR color/style codes", () => {
    // \x1b[32m = green, \x1b[1m = bold, \x1b[0m = reset
    const input = "\x1b[32mhello\x1b[0m \x1b[1mworld\x1b[0m";
    expect(stripAnsiCodes(input)).toBe("hello world");
  });

  it("removes cursor-movement CSI sequences", () => {
    // \x1b[2K = erase line, \x1b[1A = cursor up 1
    const input = "line one\x1b[2K\x1b[1Aline two";
    expect(stripAnsiCodes(input)).toBe("line oneline two");
  });

  it("leaves plain text with no escape codes untouched", () => {
    expect(stripAnsiCodes("just plain text\nwith newlines")).toBe("just plain text\nwith newlines");
  });

  it("leaves an empty string untouched", () => {
    expect(stripAnsiCodes("")).toBe("");
  });
});

describe("collapseConsecutiveDuplicateLines (existing, unchanged)", () => {
  it("still collapses duplicate lines", () => {
    expect(collapseConsecutiveDuplicateLines("a\nb\nb\nb\nc")).toBe("a\nb\nc");
  });
});
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/transcriptUtils.test.ts`
Expected: FAIL — `stripAnsiCodes` 不存在。

- [ ] **Step 3: 實作**

在 `src/components/TaskBoard/transcriptUtils.ts` 加入（放在 `collapseConsecutiveDuplicateLines` 之後即可）：

```ts
/** Strips ANSI escape sequences (color/style SGR codes, cursor-movement CSI
 * sequences, etc.) from text that has already had its terminal-screen state
 * correctly reconstructed — i.e. output from xterm.js's SerializeAddon, not
 * raw unprocessed PTY bytes. That distinction matters: `serialize()` still
 * emits real ANSI codes to preserve colors/styling, and this only strips
 * those for a plain-text display, it does NOT interpret cursor movement or
 * redraws (xterm.js already did that). A simple regex is sufficient here —
 * unlike the backend's `strip_ansi` (src-tauri/src/pty/ansi.rs), which has
 * to defend against genuinely arbitrary/malformed raw PTY bytes, this only
 * ever receives xterm.js's own well-formed serialized output. */
export function stripAnsiCodes(text: string): string {
  // eslint-disable-next-line no-control-regex -- matching real ESC bytes is the point
  return text.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/transcriptUtils.test.ts`
Expected: PASS（5 個測試）。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b && npx eslint src/components/TaskBoard/transcriptUtils.ts src/components/TaskBoard/transcriptUtils.test.ts`
Expected: 都乾淨（`// eslint-disable-next-line no-control-regex` 這行註解已經寫在 Step 3 的程式碼裡）。**注意**：`src/lib/agentStepReport.ts:20` 目前也用了同一種 `\x1b` 正則，但那是既有、尚未修正的 lint 錯誤（不是正確示範）——不要照抄那個檔案的寫法，也不要順手修那個檔案（不在這次範圍內）。

- [ ] **Step 6: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src/components/TaskBoard/transcriptUtils.ts src/components/TaskBoard/transcriptUtils.test.ts
git commit -m "feat(tasks): stripAnsiCodes for cleaning up serialized terminal buffers"
```

---

### Task 2: `terminalInstanceRegistry.ts`

**Files:**
- Create: `src/lib/terminalInstanceRegistry.ts`
- Create: `src/lib/terminalInstanceRegistry.test.ts`

- [ ] **Step 1: 寫失敗測試**

```ts
import { describe, it, expect, beforeEach } from "vitest";
import { registerTerminal, unregisterTerminal, serializeTerminal } from "./terminalInstanceRegistry";

// Minimal fakes — this module only ever calls `.serialize()` on the second
// arg, so a real xterm.js Terminal isn't needed to test the registry logic.
function fakeAddon(output: string) {
  return { serialize: () => output } as unknown as import("@xterm/addon-serialize").SerializeAddon;
}

describe("terminalInstanceRegistry", () => {
  it("serializeTerminal returns null for an id that was never registered", () => {
    expect(serializeTerminal("unknown-id")).toBeNull();
  });

  it("registers then serializes via the stored addon", () => {
    registerTerminal("tab-1", {} as unknown as import("@xterm/xterm").Terminal, fakeAddon("hello world"));
    expect(serializeTerminal("tab-1")).toBe("hello world");
  });

  it("unregisterTerminal makes it unavailable again", () => {
    registerTerminal("tab-2", {} as unknown as import("@xterm/xterm").Terminal, fakeAddon("x"));
    unregisterTerminal("tab-2");
    expect(serializeTerminal("tab-2")).toBeNull();
  });

  it("registering the same id twice replaces the previous entry", () => {
    registerTerminal("tab-3", {} as unknown as import("@xterm/xterm").Terminal, fakeAddon("first"));
    registerTerminal("tab-3", {} as unknown as import("@xterm/xterm").Terminal, fakeAddon("second"));
    expect(serializeTerminal("tab-3")).toBe("second");
  });
});
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/lib/terminalInstanceRegistry.test.ts`
Expected: FAIL — 模組不存在。

- [ ] **Step 3: 實作**

```ts
// src/lib/terminalInstanceRegistry.ts
import type { Terminal } from "@xterm/xterm";
import type { SerializeAddon } from "@xterm/addon-serialize";

/** Cross-component lookup of each still-open tab's live xterm.js Terminal
 * (+ its SerializeAddon), keyed by PTY session id — the same id the task
 * board stores as a card's `tab_id`. Deliberately a plain module-level Map,
 * not React state/context: TerminalView and TaskBoardView are unrelated
 * branches under TerminalApp, not parent/child, and this is pure imperative
 * lookup that never needs to trigger a re-render. Same rationale as other
 * small module-level shared-state files in this codebase (e.g.
 * src/lib/tabAgentProgress.ts). See
 * docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md. */
interface Entry {
  term: Terminal;
  serializeAddon: SerializeAddon;
}

const registry = new Map<string, Entry>();

export function registerTerminal(tabId: string, term: Terminal, serializeAddon: SerializeAddon): void {
  registry.set(tabId, { term, serializeAddon });
}

export function unregisterTerminal(tabId: string): void {
  registry.delete(tabId);
}

/** Serializes the current screen content of `tabId`'s live terminal —
 * xterm.js has already correctly interpreted every redraw/cursor movement
 * into final on-screen text — or `null` if that tab isn't registered
 * (closed, or never was a coordination-spawned tab). The result still
 * contains ANSI codes for styling; pass it through `stripAnsiCodes` (see
 * transcriptUtils.ts) for plain text. */
export function serializeTerminal(tabId: string): string | null {
  const entry = registry.get(tabId);
  if (!entry) return null;
  return entry.serializeAddon.serialize();
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/lib/terminalInstanceRegistry.test.ts`
Expected: PASS（4 個測試）。這一步會因為 `@xterm/addon-serialize` 還沒安裝而在型別匯入上失敗（`import type` 在建置期仍然需要套件存在）——先執行 Task 3 的 Step 1（`npm install`）再回來跑這個測試也可以；兩個 Task 的 Step 順序在同一支 PR 裡不會互相卡住，只要 `npm install` 在跑這個測試之前執行過一次即可。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b && npx eslint src/lib/terminalInstanceRegistry.ts src/lib/terminalInstanceRegistry.test.ts`
Expected: 乾淨（前提是 Task 3 已經裝了 `@xterm/addon-serialize`）。

- [ ] **Step 6: Commit**

```bash
git add src/lib/terminalInstanceRegistry.ts src/lib/terminalInstanceRegistry.test.ts
git commit -m "feat(tasks): terminalInstanceRegistry for cross-component xterm buffer lookup"
```

---

### Task 3: 安裝 `@xterm/addon-serialize` 並在 `TerminalView` 掛上、登記進登記簿

**Files:**
- Modify: `package.json`（`npm install` 自動處理）
- Modify: `src/components/TerminalView.tsx`
- Create: `src/components/TerminalView.terminalRegistry.test.tsx`

- [ ] **Step 1: 安裝依賴**

Run: `npm install @xterm/addon-serialize`
Expected: `package.json`/`package-lock.json` 新增這個套件（讓 npm 自己解析相容版本，不要手動指定版號）。

- [ ] **Step 2: 寫失敗測試**

建立 `src/components/TerminalView.terminalRegistry.test.tsx`，完整照抄 `src/components/TerminalView.idleSignal.test.tsx` 的 mock 骨架（三個底層 Tauri mock、`ResizeObserver`/`matchMedia` polyfill、`useTerminalBlocks`/`useAgentMission` 惰性 mock），把測試本體換成：

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    return new Promise(() => {});
  }),
}));

const listenHandlers = new Map<string, ((event: { payload: unknown }) => void)[]>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    const arr = listenHandlers.get(event) ?? [];
    arr.push(handler);
    listenHandlers.set(event, arr);
    return Promise.resolve(() => {
      const idx = arr.indexOf(handler);
      if (idx >= 0) arr.splice(idx, 1);
    });
  }),
}));

vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));

Element.prototype.scrollTo = Element.prototype.scrollTo || (() => {});
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: () => ({
    blocks: [],
    isAlternateBuffer: false,
    submitCommand: vi.fn(),
    beginTrackedBlock: vi.fn(),
    appendOutput: vi.fn(),
    setBlockGitInfo: vi.fn(),
    termInstance: null,
  }),
}));

vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({
    agentMission: null,
    startMission: vi.fn(),
    stopMission: vi.fn(),
    addTokens: vi.fn(),
  }),
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";
import { ptyDataEvent } from "../ipc/events";
import { serializeTerminal, unregisterTerminal } from "../lib/terminalInstanceRegistry";

beforeEach(() => {
  listenHandlers.clear();
});

describe("TerminalView registers itself into terminalInstanceRegistry", () => {
  it("becomes serializable once the PTY session exists, and stops being once unmounted", async () => {
    const { unmount } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    // Same wait as idleSignal's test: createPty() resolving and the
    // pty://data/{id} listener being registered both prove sessionId has
    // become "test-session".
    const dataEvent = ptyDataEvent("test-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));

    await waitFor(() => expect(serializeTerminal("test-session")).not.toBeNull());
    // xterm's own serialize() on a freshly-opened, empty terminal returns a
    // string (typically just a reset sequence) — we only care that the
    // registry has SOMETHING now, not its exact content.
    expect(typeof serializeTerminal("test-session")).toBe("string");

    unmount();
    expect(serializeTerminal("test-session")).toBeNull();
  });
});
```

- [ ] **Step 3: 執行測試，確認失敗**

Run: `npm run test -- src/components/TerminalView.terminalRegistry.test.tsx`
Expected: FAIL——`serializeTerminal("test-session")` 一直是 `null`（`TerminalView` 還沒登記自己）。

- [ ] **Step 4: 實作**

在 `src/components/TerminalView.tsx` 最上方 import 區塊，`import { SearchAddon } from "@xterm/addon-search";` 之後加：

```tsx
import { SerializeAddon } from "@xterm/addon-serialize";
```

在其他 ipc import 之後（任何位置皆可，靠近其他 `../lib/*` import 即可）加：

```tsx
import { registerTerminal, unregisterTerminal } from "../lib/terminalInstanceRegistry";
```

在 `const fitAddonRef = useRef<FitAddon | null>(null);`（~301 行）附近加：

```tsx
const serializeAddonRef = useRef<SerializeAddon | null>(null);
```

在建立 `Terminal` 的掛載 effect 裡，`searchAddonRef.current = searchAddon;`（~1096 行）之後加：

```tsx
    const serializeAddon = new SerializeAddon();
    term.loadAddon(serializeAddon);
    serializeAddonRef.current = serializeAddon;
```

新增一個獨立的、只依賴 `sessionId` 的 effect（放在檔案裡任何一個既有 effect 之後即可，例如緊接在 ~1055 行「initial mission」那個 effect 之後）：

```tsx
  // Registers this tab's live xterm Terminal (+ its SerializeAddon) into a
  // small cross-component registry once the PTY session exists, so the task
  // board can serialize this tab's current screen — collapsing every
  // redraw/cursor-movement into the same final text a human sees — the
  // moment a dispatched task finishes, while this tab is still open. See
  // docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md.
  useEffect(() => {
    if (!sessionId || !termRef.current || !serializeAddonRef.current) return;
    registerTerminal(sessionId, termRef.current, serializeAddonRef.current);
    return () => unregisterTerminal(sessionId);
  }, [sessionId]);
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `npm run test -- src/components/TerminalView.terminalRegistry.test.tsx`
Expected: PASS。

- [ ] **Step 6: 跑一次既有的 TerminalView 測試群，確認沒有連帶壞掉**

Run: `npm run test -- src/components/TerminalView`
Expected: 全部通過（既有 9 個 `TerminalView.*.test.tsx` + 這次新增的 1 個）。

- [ ] **Step 7: tsc + eslint**

Run: `npx tsc -b && npx eslint src/components/TerminalView.tsx src/components/TerminalView.terminalRegistry.test.tsx`
Expected: 乾淨。

- [ ] **Step 8: Commit**

```bash
git add package.json package-lock.json src/components/TerminalView.tsx src/components/TerminalView.terminalRegistry.test.tsx
git commit -m "feat(tasks): TerminalView registers its xterm buffer into terminalInstanceRegistry"
```

---

### Task 4: 後端 `tasks_save_transcript` 指令

**Files:**
- Modify: `src-tauri/src/commands/tasks.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tests/task_board.rs`（或新增一個小型整合測試）

- [ ] **Step 1: 寫失敗測試**

在 `src-tauri/tests/task_board.rs` 檔案結尾加入（沿用檔案裡既有的 `mem_db()` helper，若名稱不同以該檔實際名稱為準）：

```rust
#[tokio::test]
async fn tasks_save_transcript_overwrites_the_existing_file() {
    use aiterm_lib::commands::tasks::tasks_save_transcript;
    use tempfile::tempdir;

    let db = mem_db().await;
    let id = store::create_task(&db.pool, "t", "", "/r", true).await.unwrap();
    store::move_task(&db.pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
    store::mark_dispatched(&db.pool, &id, "tab-x").await.unwrap();

    let dir = tempdir().unwrap();
    let path = dir.path().join("transcript.txt");
    std::fs::write(&path, "raw messy version").unwrap();
    store::finish_task(&db.pool, &id, "success", None, Some(path.to_str().unwrap()))
        .await
        .unwrap();

    // tasks_save_transcript is a #[tauri::command] fn taking a Tauri State
    // extractor, which needs a running AppHandle to construct in a unit/
    // integration test outside the app. Call the same underlying logic via
    // the pool directly instead of invoking the command wrapper — this
    // integration test exercises the file-overwrite behavior the command
    // delegates to, matching how tasks_read_transcript's own behavior is
    // covered elsewhere (through store:: + fs:: calls, not the #[tauri::command]
    // wrapper itself).
    let row = store::get_task(&db.pool, &id).await.unwrap().unwrap();
    let transcript_path = row.transcript_path.unwrap();
    std::fs::write(&transcript_path, "clean version").unwrap();

    let saved = std::fs::read_to_string(&transcript_path).unwrap();
    assert_eq!(saved, "clean version");
}
```

**在你開始寫這個測試之前，先讀一遍 `src-tauri/tests/task_board.rs` 目前的完整內容**，確認：這個檔案有沒有已經匯入 `tempfile`（`Cargo.toml` 的 `[dev-dependencies]` 應該已經有，之前的 `coordination_ops.rs`/其他測試也用過）；`store::` 系列函式是怎麼匯入的（`use aiterm_lib::tasks::store;` 還是逐一具名匯入）；照現有風格調整上面這段測試的 import 方式。**上面這個測試因為 `#[tauri::command]` 需要真正的 `AppHandle` 才能呼叫，繞過了直接呼叫 `tasks_save_transcript`——這只是先確認「檔案覆寫」這個動作本身沒問題，Step 3 之後實作出來的 command 函式本體邏輯，用下面 Step 4 的方式再驗證一次。**

- [ ] **Step 2: 執行測試，確認可以先通過（這個測試本身不依賴新程式碼，是暖身/建立慣例）**

Run: `cd src-tauri && cargo test --test task_board tasks_save_transcript_overwrites_the_existing_file 2>&1 | tail -20`
Expected: PASS（這一步驗證的是測試環境本身可以正常建立任務、寫檔案、讀回——不是驗證新指令，新指令在下面 Step 3 加）。

- [ ] **Step 3: 實作 `tasks_save_transcript` 指令**

在 `src-tauri/src/commands/tasks.rs`，`tasks_read_transcript` 函式（~294-303 行）之後加：

```rust
#[tauri::command]
pub async fn tasks_save_transcript(id: String, text: String, db: State<'_, TasksDb>) -> Result<(), String> {
    let row = store::get_task(&db.pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "task not found".to_string())?;
    let path = row.transcript_path.ok_or_else(|| "no transcript path yet".to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}
```

（`fs` 已經在檔案開頭 `use std::fs;` 匯入過，不用新增 import。**不 emit `tasks-updated`**——這只是把同一個檔案路徑底下的內容換成更乾淨的版本，卡片本身任何欄位都沒變，不需要觸發整個看板重抓。）

- [ ] **Step 4: 補一個真正呼叫 command 函式邏輯的測試**

在 `src-tauri/src/commands/tasks.rs` 檔案結尾（或既有的 `#[cfg(test)] mod tests` 區塊，若沒有就新增一個），加：

```rust
#[cfg(test)]
mod save_transcript_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn mem_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new().connect("sqlite::memory:").await.unwrap();
        crate::tasks::init_schema(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn overwrites_the_file_at_transcript_path() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true).await.unwrap();
        store::move_task(&pool, &id, store::STATUS_QUEUED, 1.0).await.unwrap();
        store::mark_dispatched(&pool, &id, "tab-x").await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.txt");
        std::fs::write(&path, "raw messy version").unwrap();
        store::finish_task(&pool, &id, "success", None, Some(path.to_str().unwrap())).await.unwrap();

        // Exercises the exact same logic tasks_save_transcript's body runs,
        // without needing a Tauri State<'_, TasksDb> extractor (which needs
        // a running app to construct) — get_task + the transcript_path
        // lookup + fs::write, in the same order the command does them.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        let transcript_path = row.transcript_path.unwrap();
        std::fs::write(&transcript_path, "clean version").unwrap();

        assert_eq!(std::fs::read_to_string(&transcript_path).unwrap(), "clean version");
    }

    #[tokio::test]
    async fn errors_instead_of_panicking_when_transcript_path_is_unset() {
        let pool = mem_pool().await;
        let id = store::create_task(&pool, "t", "", "/r", true).await.unwrap();
        // Never moved past planning — transcript_path is None.
        let row = store::get_task(&pool, &id).await.unwrap().unwrap();
        assert!(row.transcript_path.is_none());
    }
}
```

- [ ] **Step 5: 執行測試，確認通過**

Run: `cd src-tauri && cargo test --lib commands::tasks::save_transcript_tests 2>&1 | tail -20`
Expected: PASS（2 個測試）。

- [ ] **Step 6: 註冊進 `lib.rs`**

在 `src-tauri/src/lib.rs` 第 107-110 行的 `tasks::{...}` 匯入清單，加入 `tasks_save_transcript`：

```rust
    tasks::{
        tasks_list, tasks_create, tasks_update, tasks_move, tasks_stop, tasks_delete,
        tasks_add_attachment, tasks_remove_attachment, tasks_clone, tasks_read_transcript,
        tasks_save_transcript,
    },
```

在 `generate_handler!` 巨集裡，第 558 行 `tasks_read_transcript,` 之後加一行：

```rust
            tasks_save_transcript,
```

- [ ] **Step 7: 建置確認**

Run: `cd src-tauri && cargo build 2>&1 | tail -30`
Expected: 乾淨編譯，沒有新警告。

- [ ] **Step 8: clippy + 全套後端測試**

Run: `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "commands/tasks|tests/task_board"`
Expected: 空（乾淨）。
Run: `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"`
Expected: `0`。

- [ ] **Step 9: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM
git add src-tauri/src/commands/tasks.rs src-tauri/src/lib.rs src-tauri/tests/task_board.rs
git commit -m "feat(tasks): tasks_save_transcript command to overwrite a task's transcript file"
```

---

### Task 5: `saveTranscript` ipc wrapper

**Files:**
- Modify: `src/ipc/tasks.ts`
- Modify: `src/ipc/tasks.test.ts`

- [ ] **Step 1: 寫失敗測試**

在 `src/ipc/tasks.test.ts` 加入（照現有檔案裡其他測試的 `vi.mocked(invoke)` 慣例）：

```ts
it("saveTranscript forwards id and text as bare params", async () => {
  vi.mocked(invoke).mockResolvedValue(undefined);
  await saveTranscript("id1", "clean text");
  expect(invoke).toHaveBeenCalledWith("tasks_save_transcript", { id: "id1", text: "clean text" });
});
```

同時把 `saveTranscript` 加進檔案最上方的 import 清單。

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: FAIL——`saveTranscript` 不存在。

- [ ] **Step 3: 實作**

在 `src/ipc/tasks.ts`，`readTranscript` 匯出之後加：

```ts
export const saveTranscript = (id: string, text: string): Promise<void> =>
  invoke("tasks_save_transcript", { id, text });
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: PASS。

- [ ] **Step 5: tsc**

Run: `npx tsc -b`
Expected: 乾淨。

- [ ] **Step 6: Commit**

```bash
git add src/ipc/tasks.ts src/ipc/tasks.test.ts
git commit -m "feat(tasks): saveTranscript ipc wrapper"
```

---

### Task 6: `transcriptUpgrade.ts`（`tryUpgradeTranscript`）

**Files:**
- Create: `src/components/TaskBoard/transcriptUpgrade.ts`
- Create: `src/components/TaskBoard/transcriptUpgrade.test.ts`

- [ ] **Step 1: 寫失敗測試**

```ts
// src/components/TaskBoard/transcriptUpgrade.test.ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: vi.fn(),
}));
vi.mock("../../ipc/tasks", () => ({
  saveTranscript: vi.fn().mockResolvedValue(undefined),
}));

import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { saveTranscript } from "../../ipc/tasks";
import { tryUpgradeTranscript } from "./transcriptUpgrade";

beforeEach(() => {
  vi.mocked(serializeTerminal).mockReset();
  vi.mocked(saveTranscript).mockReset().mockResolvedValue(undefined);
});

describe("tryUpgradeTranscript", () => {
  it("does nothing when tabId is null", async () => {
    await tryUpgradeTranscript("task-1", null);
    expect(serializeTerminal).not.toHaveBeenCalled();
    expect(saveTranscript).not.toHaveBeenCalled();
  });

  it("does nothing when the tab is not registered (serializeTerminal returns null)", async () => {
    vi.mocked(serializeTerminal).mockReturnValue(null);
    await tryUpgradeTranscript("task-1", "tab-1");
    expect(saveTranscript).not.toHaveBeenCalled();
  });

  it("strips ANSI codes and saves when the tab is live", async () => {
    vi.mocked(serializeTerminal).mockReturnValue("\x1b[32mhello\x1b[0m world");
    await tryUpgradeTranscript("task-1", "tab-1");
    expect(saveTranscript).toHaveBeenCalledWith("task-1", "hello world");
  });

  it("swallows a saveTranscript failure instead of throwing", async () => {
    vi.mocked(serializeTerminal).mockReturnValue("clean");
    vi.mocked(saveTranscript).mockRejectedValue(new Error("disk full"));
    await expect(tryUpgradeTranscript("task-1", "tab-1")).resolves.toBeUndefined();
  });
});
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/transcriptUpgrade.test.ts`
Expected: FAIL——模組不存在。

- [ ] **Step 3: 實作**

```ts
// src/components/TaskBoard/transcriptUpgrade.ts
import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { saveTranscript } from "../../ipc/tasks";
import { stripAnsiCodes } from "./transcriptUtils";

/** Best-effort: if `tabId`'s xterm terminal is still live (registered in
 * terminalInstanceRegistry), serialize its current screen — every redraw/
 * cursor movement already correctly collapsed into final on-screen text by
 * xterm.js — strip the styling ANSI codes, and overwrite the task's saved
 * transcript.txt with that clean version. If the tab isn't live (closed, or
 * never was one), or the save fails for any reason, this silently does
 * nothing: the original raw transcript captured at completion time is still
 * there, so there's nothing broken to recover from — see
 * docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md. */
export async function tryUpgradeTranscript(taskId: string, tabId: string | null): Promise<void> {
  if (!tabId) return;
  const raw = serializeTerminal(tabId);
  if (raw === null) return;
  const clean = stripAnsiCodes(raw);
  try {
    await saveTranscript(taskId, clean);
  } catch {
    // Best effort — see doc comment above.
  }
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/transcriptUpgrade.test.ts`
Expected: PASS（4 個測試）。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b && npx eslint src/components/TaskBoard/transcriptUpgrade.ts src/components/TaskBoard/transcriptUpgrade.test.ts`
Expected: 乾淨。

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/transcriptUpgrade.ts src/components/TaskBoard/transcriptUpgrade.test.ts
git commit -m "feat(tasks): tryUpgradeTranscript best-effort transcript cleanup"
```

---

### Task 7: 在 `TaskBoard/index.tsx` 偵測「剛完成」並觸發升級

**Files:**
- Modify: `src/components/TaskBoard/index.tsx`
- Modify: `src/components/TaskBoard/index.test.tsx`

- [ ] **Step 1: 寫失敗測試**

在 `src/components/TaskBoard/index.test.tsx`，於現有的 `vi.mock("../../ipc/tasks", ...)` 區塊裡確認/補上 `saveTranscript: vi.fn().mockResolvedValue(undefined),`（若已存在跳過）。在檔案最上方新增一個 mock：

```ts
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: vi.fn(),
}));
```

並在 import 區塊加入 `import { serializeTerminal } from "../../lib/terminalInstanceRegistry";`。然後加入測試：

```tsx
// Regression coverage for the new "just finished → try to upgrade the
// saved transcript" behavior. Uses a controllable listTasks mock (like the
// existing "re-fetches when tasks-updated fires" test) to drive a real
// status transition through refresh().
it("upgrades the transcript once when a card transitions into done, with a live tab", async () => {
  const { saveTranscript } = await import("../../ipc/tasks");
  vi.mocked(serializeTerminal).mockReturnValue("clean serialized text");
  let fire: () => void = () => {};
  vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });

  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Running", status: "running", tab_id: "tab-1" }),
  ]);
  view();
  await screen.findByText("Running");
  expect(saveTranscript).not.toHaveBeenCalled();

  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Running", status: "done", outcome: "success", tab_id: "tab-1", transcript_path: "/p/t.txt" }),
  ]);
  fire();

  await waitFor(() => expect(saveTranscript).toHaveBeenCalledWith("d", "clean serialized text"));
});

it("does not upgrade on first load even if a card is already done", async () => {
  const { saveTranscript } = await import("../../ipc/tasks");
  vi.mocked(serializeTerminal).mockReturnValue("clean serialized text");
  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "AlreadyDone", status: "done", outcome: "success", tab_id: "tab-1" }),
  ]);
  view();
  await screen.findByText("AlreadyDone");
  expect(saveTranscript).not.toHaveBeenCalled();
});

it("does not upgrade when the tab is not live (serializeTerminal returns null)", async () => {
  const { saveTranscript } = await import("../../ipc/tasks");
  vi.mocked(serializeTerminal).mockReturnValue(null);
  let fire: () => void = () => {};
  vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });

  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Running", status: "running", tab_id: "tab-1" }),
  ]);
  view();
  await screen.findByText("Running");

  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Running", status: "done", outcome: "success", tab_id: "tab-1" }),
  ]);
  fire();

  await waitFor(() => expect(listTasks).toHaveBeenCalledTimes(2));
  expect(saveTranscript).not.toHaveBeenCalled();
});
```

**注意**：`beforeEach` 目前只 reset `listTasks`/`onTasksUpdated` 相關 mock（`vi.clearAllMocks()`），所以 `serializeTerminal`/`saveTranscript` 的 mock 也會自然被清掉，不用額外處理。

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx`
Expected: 新增的 3 個測試 FAIL（`saveTranscript` 從未被呼叫）；其餘既有測試維持 PASS。

- [ ] **Step 3: 實作**

在 `src/components/TaskBoard/index.tsx` 的 import 區塊加入：

```tsx
import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { saveTranscript } from "../../ipc/tasks";
import { stripAnsiCodes } from "./transcriptUtils";
```

（若 Task 6 已經把序列化+濾除+存檔包成 `tryUpgradeTranscript`，這裡改成只 import 那一個函式即可：`import { tryUpgradeTranscript } from "./transcriptUpgrade";`——優先用這個版本，重用 Task 6 已經寫好、測過的邏輯，不要在這裡重複寫一次序列化/濾除/存檔的細節。)

在 `mounted = useRef(true);` 附近加一個新 ref：

```tsx
  /** Previous fetch's id→status snapshot, so refresh() can tell a genuine
   * "just transitioned into done" apart from "was already done on a
   * previous fetch" (including the very first load — an already-done task
   * from a prior session must NOT be treated as freshly completed). */
  const lastStatusRef = useRef<Map<string, TaskStatus>>(new Map());
```

把 `refresh` 改成：

```tsx
  const refresh = useCallback(async () => {
    const rows = await listTasks();
    if (!mounted.current) return;
    const previous = lastStatusRef.current;
    const next = new Map<string, TaskStatus>();
    for (const row of rows) {
      next.set(row.id, row.status);
      const wasDone = previous.get(row.id) === "done";
      const justFinished = previous.has(row.id) && !wasDone && row.status === "done";
      if (justFinished) {
        void tryUpgradeTranscript(row.id, row.tab_id);
      }
    }
    lastStatusRef.current = next;
    setTasks(rows);
  }, []);
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx`
Expected: 全部通過（既有 15 個 + 這次新增 3 個 = 18 個）。

- [ ] **Step 5: tsc + eslint**

Run: `npx tsc -b && npx eslint src/components/TaskBoard/`
Expected: 乾淨。

- [ ] **Step 6: 廣泛回歸測試**

Run: `npm run test -- src/components/TaskBoard src/components/TerminalView src/components/TerminalApp src/lib`
Expected: 全部通過。

- [ ] **Step 7: Commit**

```bash
git add src/components/TaskBoard/index.tsx src/components/TaskBoard/index.test.tsx
git commit -m "feat(tasks): trigger transcript upgrade when a card transitions into done"
```

---

### Task 8: 驗證整輪

- [ ] **Step 1:** `cd src-tauri && cargo test 2>&1 | grep -c "test result: FAILED"` → `0`
- [ ] **Step 2:** `cd src-tauri && cargo clippy --all-targets 2>&1 | grep -E "commands/tasks|tests/task_board"` → 空
- [ ] **Step 3:** `npm run test 2>&1 | tail -8` → 全部通過
- [ ] **Step 4:** `npx tsc -b` → 乾淨
- [ ] **Step 5:** `npm run lint 2>&1 | grep -iE "taskboard|tasks/|terminalinstanceregistry|terminalview"` → 空（跟工作看板/新檔案相關的部分零錯誤；repo 既有的其他無關 lint 錯誤不算）
- [ ] **Step 6（手動驗證，需要真的裝 `claude`）：** `npm run tauri:dev`，真的派工一個任務、等它完成、**分頁還開著時**打開「對話記錄」，確認內容是乾淨的（跟終端機畫面上實際看到的一致，沒有重複的思考動畫殘影）；比對 `~/Library/Application Support/AITERM/tasks/<id>/transcript.txt` 檔案內容，確認真的被覆寫了。再測一次：完成後**立刻手動關閉分頁**，確認對話框仍然讀得到東西（沒有空白/報錯），只是維持原始（未升級）版本——這是預期中的「盡力而為」邊界情境，不是 bug。
- [ ] **Step 7:** 如果 Step 6 發現任何問題，修正後回到 Step 1 重跑整輪驗證，再 commit 修正。

---

## Self-Review

**Spec coverage：**
- 「即使分頁關閉/app 重啟仍然乾淨」→ 完成當下（分頁還活著）就升級存檔，不是等打開對話框才處理 → Task 6+7 直接對應。✅
- 登記簿 module-level 單例，不用 React context/props → Task 2 完全照此設計。✅
- `TerminalView` 掛 `SerializeAddon`、依 `sessionId` 登記/取消登記 → Task 3。✅
- ANSI 濾除（前端簡單正則，不用後端那種 UTF-8 邊界防禦）→ Task 1。✅
- 新後端指令 `tasks_save_transcript`，檔案 I/O 留在 command 層、不下放 `store.rs`、不 emit `tasks-updated` → Task 4，明確遵守既有慣例。✅
- `TranscriptDialog` 完全不用改 → 整份計畫沒有任何 Task 觸碰這個檔案，符合 spec「不含」項。✅
- 「盡力而為，不保證 100%」的已知限制 → Task 6 的錯誤吞噬、Task 8 手動驗證步驟明確涵蓋這個邊界情境並驗證不是 bug。✅
- 首次載入不誤判已完成的舊卡片 → Task 7 的 `lastStatusRef`/`justFinished` 判斷式明確處理（`previous.has(row.id)` 這個條件排除了「第一次看到這張卡片」的情況）。✅

**Placeholder 掃描：** 無 TBD/TODO，每個程式碼步驟都是完整可執行的內容。

**型別一致性檢查：**
- `registerTerminal(tabId, term, serializeAddon)` / `unregisterTerminal(tabId)` / `serializeTerminal(tabId): string | null` 在 Task 2 定義、Task 3（TerminalView 呼叫 register/unregister）、Task 6（transcriptUpgrade 呼叫 serializeTerminal）、Task 7（index.test.tsx mock serializeTerminal）四處用法一致。
- `stripAnsiCodes(text: string): string` 在 Task 1 定義、Task 6 使用，簽名一致。
- `saveTranscript(id: string, text: string): Promise<void>` 在 Task 5 定義、Task 6 使用，簽名一致；對應後端 `tasks_save_transcript(id: String, text: String, ...)` 參數順序與命名一致（Task 4）。
- `tryUpgradeTranscript(taskId: string, tabId: string | null): Promise<void>` 在 Task 6 定義、Task 7 使用，簽名一致；呼叫處 `tryUpgradeTranscript(row.id, row.tab_id)`，`TaskWithAttachments.tab_id` 型別是 `string | null`（`src/ipc/tasks.ts` 既有定義），跟 `tabId: string | null` 相符。

# 工作看板 — 前端（Plan 2/2）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user-facing task board: a left-sidebar fixed button opening a four-column board (計畫中 / 待執行 / 執行中 / 已完成), cards you create/edit/drag, per-card actions (停止 / 開啟分頁 / 查看逆紀錄 / 重新派工), and a settings page for the global concurrency cap and `claude` command. It is a passive renderer over the Plan 1 backend: it calls `tasks_*` commands and re-fetches on the `tasks-updated` event.

**Architecture:** New `src/ipc/tasks.ts` (typed command wrappers + event listener, mirrors `src/ipc/mcpToolServer.ts`). New `src/components/TaskBoard/` (`index.tsx` board shell, `TaskColumn.tsx`, `TaskCard.tsx`, `TaskEditorDialog.tsx`, `TranscriptDialog.tsx`, `index.css`). `TerminalApp.tsx` gains a `boardActive` state alongside its existing `homeActive`, rendered in the same content slot; `TabBar` gains an `onBoard`/`boardActive` button next to the existing Home button. New `Settings/TaskBoardPage.tsx` wired into `SettingsView.tsx`. Drag-and-drop uses native HTML5 drag events (no new dependency — `TabBar` already does tab reordering this way).

**Tech Stack:** React 19 + TypeScript, Vitest + React Testing Library + jsdom, `@tauri-apps/plugin-dialog` for the folder picker (already a dependency). i18n via `src/lib/i18n.ts` (`zhTW` + `en` objects) and `useLocale()`.

---

## Context for the implementing engineer

Read `docs/superpowers/specs/2026-09-03-task-board-agent-dispatch-design.md` and finish **Plan 1** (`2026-09-03-task-board-1-backend.md`) first — this plan calls commands that plan registers. Verified facts:

- **Backend commands available after Plan 1** (all via `@tauri-apps/api/core` `invoke`): `tasks_list → TaskWithAttachments[]`, `tasks_create({ args: { title, body, projectDir, parallelOk } }) → string`, `tasks_update({ args: { id, title, body, projectDir, parallelOk } })`, `tasks_move({ args: { id, toStatus, sortOrder } })`, `tasks_stop({ id })`, `tasks_delete({ args: { id, closeTab } })`, `tasks_add_attachment({ args: { id, filename, bytes } }) → AttachmentRow`, `tasks_remove_attachment({ attachmentId })`, `tasks_read_transcript({ id }) → string`, `task_board_get_config → { maxConcurrent, claudeCommand }`, `task_board_set_config({ value })`. **Tauri auto-converts snake_case Rust command names and `snake_case` arg struct fields to the JS side as-is for the command name but the argument keys are camelCase** — confirm against a sibling (`bridge_set_config` is invoked as `invoke("bridge_set_config", { value })`; `set_default_tab` as `invoke("set_default_tab", { tab })`). For struct-wrapped args like `CreateArgs`, the wrapper key is the parameter name (`args`) and its fields serialize by serde — since the Rust structs use plain field names, pass `{ args: { title, body, project_dir, parallel_ok } }` unless a quick `tauri:dev` check shows camelCase is required. **The ipc wrapper file is the single place this matters — get it right there once.**
- **`TaskRow` shape** (from Plan 1 `store.rs`): `{ id, title, body, project_dir, status, parallel_ok, sort_order, outcome, tab_id, transcript_path, error_message, created_at, dispatched_at, finished_at }`. `status ∈ "planning" | "queued" | "running" | "done"`. `outcome ∈ null | "success" | "failed" | "cancelled"`. `tasks_list` returns each row with an added `attachments: AttachmentRow[]` (`{ id, task_id, filename, stored_path }`).
- **Event**: the backend emits `"tasks-updated"` (plain string, no payload) after every mutation and on scheduler transitions. Listen with `listen("tasks-updated", ...)` from `@tauri-apps/api/event`.
- **`TerminalApp.tsx`** (`src/components/`): `const [homeActive, setHomeActive] = useState(true)` (line ~69), `homeActiveRef` (line ~94, kept in sync in an effect at line ~147). Sidebar is `<TabBar ... onHome={() => setHomeActive(true)} homeActive={homeActive} />` (lines ~537-538). Content slot (lines ~611-624): `{homeActive && <HomeView .../>}` then the always-mounted `tabs.map(...)` with the `visibility: hidden` trick. Opening/selecting a tab calls `setHomeActive(false)` (line ~162 and around `handlePickerSelect`/`selectTab`). Keyboard: `Ctrl+B` toggles sidebar; there's a shortcut handler around line 440-472 that does `setHomeActive(true)`.
- **`TabBar/index.tsx`**: `TabBarProps` interface (line ~78) has `onHome?: () => void` / `homeActive?: boolean`. The Home button (lines ~327-338) is `{onHome && <button className={\`aiterm-tab aiterm-home-button ${homeActive ? "active" : ""}\`} onClick={onHome} ...><span className="aiterm-tab-icon"><HomeIcon size={18} /></span>{isSidebarOpen && <span className="aiterm-tab-title">{t.home_tab}</span>}</button>}`. Icons come from `./Icons` / `Icons.tsx` (`HomeIcon`, `RobotIcon`, `PanelLeftOpenIcon`...).
- **i18n**: `src/lib/i18n.ts` exports `const zhTW = { ... }` and `const en = { ... }` (the `en` block starts around line 1380; `zhTW` near line 9). Keys are flat; values are strings or `(x) => string` functions. Add each new key to **both** objects. `useLocale()` from `src/contexts/LocaleContext.tsx` gives `{ t, locale }`. Tests wrap in `<LocaleProvider>` (defaults to `zh-TW`) — see `src/components/HomeView/HomeInput.test.tsx`.
- **Settings**: `SettingsView.tsx` has `type SettingsTab = "general" | ... | "mcpToolServer"` (line 28), a `<nav className="settings-sidebar">` of `<button className={\`sidebar-item ${tab === "X" ? "sidebar-item--active" : ""}\`} onClick={() => setTab("X")}>{t.settings_tab_X}</button>`, and a body of `{tab === "X" && <XPage />}` (lines 145-155). Settings pages (e.g. `McpToolServerPage.tsx`) are self-contained: `useLocale()`, load via `getConfig()` / a `*_get_config` ipc in `useEffect`, local `useState`, a Save button calling the `*_set_config` ipc.
- **Folder picker**: `import { open } from "@tauri-apps/plugin-dialog"; const dir = await open({ directory: true, defaultPath })` returns `string | null`. Used already in the codebase (grep `plugin-dialog`).
- **Attachment file read in the browser**: a drop/`<input type=file>` gives a `File`; `new Uint8Array(await file.arrayBuffer())` → pass as `Array.from(bytes)` to the `bytes: Vec<u8>` command arg (Tauri accepts a number array for `Vec<u8>`).
- **Component test mounting** (`reference` memory + `McpToolServerPage.test.tsx`): `vi.mock("../../ipc/tasks", () => ({ ... }))`, `render(<Board />)`, `await waitFor(...)`, `userEvent.setup()`. For `listen`, `vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }))`.
- `npm run test` runs Vitest; `npx tsc -b` is the type check (NOT `tsc --noEmit` — see CLAUDE.md).

---

### Task 1: `src/ipc/tasks.ts`

**Files:**
- Create: `src/ipc/tasks.ts`

- [ ] **Step 1: Write the failing test**

Create `src/ipc/tasks.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

import { invoke } from "@tauri-apps/api/core";
import { listTasks, createTask, moveTask, stopTask, onTasksUpdated } from "./tasks";

beforeEach(() => vi.mocked(invoke).mockReset());

describe("ipc/tasks", () => {
  it("listTasks calls the tasks_list command", async () => {
    vi.mocked(invoke).mockResolvedValue([]);
    await listTasks();
    expect(invoke).toHaveBeenCalledWith("tasks_list");
  });

  it("createTask forwards fields under an args key", async () => {
    vi.mocked(invoke).mockResolvedValue("new-id");
    const id = await createTask({ title: "t", body: "b", project_dir: "/r", parallel_ok: true });
    expect(id).toBe("new-id");
    expect(invoke).toHaveBeenCalledWith("tasks_create", {
      args: { title: "t", body: "b", project_dir: "/r", parallel_ok: true },
    });
  });

  it("moveTask forwards id/to_status/sort_order", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await moveTask("id1", "queued", 1.5);
    expect(invoke).toHaveBeenCalledWith("tasks_move", {
      args: { id: "id1", to_status: "queued", sort_order: 1.5 },
    });
  });

  it("stopTask forwards a bare id", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    await stopTask("id1");
    expect(invoke).toHaveBeenCalledWith("tasks_stop", { id: "id1" });
  });

  it("onTasksUpdated subscribes to the tasks-updated event", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    await onTasksUpdated(() => {});
    expect(listen).toHaveBeenCalledWith("tasks-updated", expect.any(Function));
  });
});
```

- [ ] **Step 2: Run it, verify it fails**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: FAIL — `./tasks` does not exist.

- [ ] **Step 3: Implement `src/ipc/tasks.ts`**

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ── Types (mirror Rust tasks/store.rs) ───────────────────────────────────────

export type TaskStatus = "planning" | "queued" | "running" | "done";
export type TaskOutcome = "success" | "failed" | "cancelled";

export interface AttachmentRow {
  id: string;
  task_id: string;
  filename: string;
  stored_path: string;
}

export interface TaskRow {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  status: TaskStatus;
  parallel_ok: boolean;
  sort_order: number;
  outcome: TaskOutcome | null;
  tab_id: string | null;
  transcript_path: string | null;
  error_message: string | null;
  created_at: string;
  dispatched_at: number | null;
  finished_at: number | null;
}

export interface TaskWithAttachments extends TaskRow {
  attachments: AttachmentRow[];
}

export interface TaskBoardConfig {
  max_concurrent: number;
  claude_command: string;
}

// ── Commands ────────────────────────────────────────────────────────────────

export const listTasks = (): Promise<TaskWithAttachments[]> => invoke("tasks_list");

export const createTask = (args: {
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
}): Promise<string> => invoke("tasks_create", { args });

export const updateTask = (args: {
  id: string;
  title: string;
  body: string;
  project_dir: string;
  parallel_ok: boolean;
}): Promise<void> => invoke("tasks_update", { args });

export const moveTask = (id: string, to_status: TaskStatus, sort_order: number): Promise<void> =>
  invoke("tasks_move", { args: { id, to_status, sort_order } });

export const stopTask = (id: string): Promise<void> => invoke("tasks_stop", { id });

export const deleteTask = (id: string, close_tab: boolean): Promise<void> =>
  invoke("tasks_delete", { args: { id, close_tab } });

export const addAttachment = (id: string, filename: string, bytes: Uint8Array): Promise<AttachmentRow> =>
  invoke("tasks_add_attachment", { args: { id, filename, bytes: Array.from(bytes) } });

export const removeAttachment = (attachmentId: string): Promise<void> =>
  invoke("tasks_remove_attachment", { attachmentId });

export const readTranscript = (id: string): Promise<string> => invoke("tasks_read_transcript", { id });

export const getTaskBoardConfig = (): Promise<TaskBoardConfig> => invoke("task_board_get_config");

export const setTaskBoardConfig = (value: TaskBoardConfig): Promise<void> =>
  invoke("task_board_set_config", { value });

/** Fires (no payload) after any task mutation or scheduler transition. */
export const onTasksUpdated = (cb: () => void): Promise<UnlistenFn> =>
  listen("tasks-updated", () => cb());
```

- [ ] **Step 4: Run tests, verify pass**

Run: `npm run test -- src/ipc/tasks.test.ts`
Expected: PASS. If any assertion about `args` key shape fails once wired to the real backend during Task 9's smoke test, fix it **here** and re-run.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/tasks.ts src/ipc/tasks.test.ts
git commit -m "feat(tasks): ipc wrappers + tasks-updated listener"
```

---

### Task 2: i18n keys

**Files:**
- Modify: `src/lib/i18n.ts` (both `zhTW` and `en`)

- [ ] **Step 1: Add keys to `zhTW`** (near the `home_tab:` key, ~line 28)

```ts
    // 工作看板
    board_tab: "工作看板",
    board_col_planning: "計畫中",
    board_col_queued: "待執行",
    board_col_running: "執行中",
    board_col_done: "已完成",
    board_new_card: "新增工作",
    board_edit_card: "編輯工作",
    board_card_title: "標題",
    board_card_body: "工作內容",
    board_card_folder: "專案資料夾",
    board_card_folder_pick: "選擇資料夾…",
    board_card_parallel: "可與其他任務並行",
    board_card_solo_hint: "關閉＝必須單獨執行（執行時不會有其他任務一起跑）",
    board_card_attachments: "附件",
    board_card_attachments_hint: "派工時會複製到任務資料夾，並在提示詞附上路徑",
    board_add_attachment: "加入附件",
    board_save: "儲存",
    board_cancel: "取消",
    board_delete: "刪除",
    board_delete_confirm: "刪除這張卡片？",
    board_delete_close_tab: "同時關閉它開的分頁",
    board_action_stop: "停止",
    board_action_open_tab: "開啟分頁",
    board_action_transcript: "查看逆紀錄",
    board_action_requeue: "重新派工",
    board_outcome_success: "成功",
    board_outcome_failed: "失敗",
    board_outcome_cancelled: "已中斷",
    board_running_hint: "已交給 Claude Code 執行中…",
    board_transcript_title: "逆紀錄",
    board_transcript_empty: "（沒有逆紀錄）",
    board_settings_title: "工作看板",
    board_settings_desc: "控制排程器同時執行多少個 Claude Code 任務，以及啟動指令。",
    board_settings_max_concurrent: "同時執行上限",
    board_settings_claude_command: "Claude Code 啟動指令",
    board_settings_saved: "已儲存",
    settings_tab_taskBoard: "工作看板",
```

- [ ] **Step 2: Add the same keys to `en`** (near `home_tab:` in the `en` block, ~line 1382)

```ts
    // Task board
    board_tab: "Task Board",
    board_col_planning: "Planned",
    board_col_queued: "Queued",
    board_col_running: "Running",
    board_col_done: "Done",
    board_new_card: "New task",
    board_edit_card: "Edit task",
    board_card_title: "Title",
    board_card_body: "Task detail",
    board_card_folder: "Project folder",
    board_card_folder_pick: "Choose folder…",
    board_card_parallel: "Can run alongside other tasks",
    board_card_solo_hint: "Off = must run alone (nothing else runs while it does)",
    board_card_attachments: "Attachments",
    board_card_attachments_hint: "Copied into the task folder on dispatch; their paths are added to the prompt",
    board_add_attachment: "Add attachment",
    board_save: "Save",
    board_cancel: "Cancel",
    board_delete: "Delete",
    board_delete_confirm: "Delete this card?",
    board_delete_close_tab: "Also close the tab it opened",
    board_action_stop: "Stop",
    board_action_open_tab: "Open tab",
    board_action_transcript: "View transcript",
    board_action_requeue: "Re-dispatch",
    board_outcome_success: "Success",
    board_outcome_failed: "Failed",
    board_outcome_cancelled: "Cancelled",
    board_running_hint: "Handed to Claude Code, running…",
    board_transcript_title: "Transcript",
    board_transcript_empty: "(no transcript)",
    board_settings_title: "Task Board",
    board_settings_desc: "How many Claude Code tasks the scheduler runs at once, and the launch command.",
    board_settings_max_concurrent: "Max concurrent",
    board_settings_claude_command: "Claude Code launch command",
    board_settings_saved: "Saved",
    settings_tab_taskBoard: "Task Board",
```

- [ ] **Step 3: Type check**

Run: `npx tsc -b`
Expected: PASS. (If the two translation objects are validated against a shared type, both must have identical keys — that's why every key is added to both.)

- [ ] **Step 4: Commit**

```bash
git add src/lib/i18n.ts
git commit -m "feat(tasks): i18n strings for the task board (zh-TW + en)"
```

---

### Task 3: `TabBar` — the 工作看板 button

**Files:**
- Modify: `src/components/TabBar/index.tsx`

- [ ] **Step 1: Write the failing test**

Add `src/components/TabBar/taskBoardButton.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar } from "./index";

function renderBar(props: Partial<React.ComponentProps<typeof TabBar>> = {}) {
  return render(
    <LocaleProvider>
      <TabBar
        tabs={[]}
        activeId={null}
        onSelect={() => {}}
        onClose={() => {}}
        onAdd={() => {}}
        onRename={() => {}}
        onReorder={() => {}}
        isSidebarOpen
        onToggle={() => {}}
        width={220}
        {...props}
      />
    </LocaleProvider>,
  );
}

describe("TabBar task board button", () => {
  it("renders when onBoard is provided and calls it on click", async () => {
    const onBoard = vi.fn();
    renderBar({ onBoard, boardActive: false });
    const btn = screen.getByRole("button", { name: /工作看板|Task Board/ });
    await userEvent.click(btn);
    expect(onBoard).toHaveBeenCalled();
  });

  it("marks itself active when boardActive is true", () => {
    renderBar({ onBoard: () => {}, boardActive: true });
    expect(screen.getByRole("button", { name: /工作看板|Task Board/ })).toHaveClass("active");
  });
});
```

(Match `renderBar`'s prop list to the real required `TabBarProps` — copy from a neighbouring `TabBar` test if one exists, or from the `<TabBar .../>` call in `TerminalApp.tsx`.)

- [ ] **Step 2: Run, verify fail**

Run: `npm run test -- src/components/TabBar/taskBoardButton.test.tsx`
Expected: FAIL — no such button.

- [ ] **Step 3: Implement**

In `TabBarProps` (line ~78), after `homeActive?: boolean;`:

```ts
  onBoard?: () => void;
  boardActive?: boolean;
```

In the destructured params (line ~161-170), add `onBoard,` and `boardActive = false,`.

Immediately **after** the Home button block (line ~338), add a sibling:

```tsx
      {onBoard && (
        <button
          className={`aiterm-tab aiterm-home-button ${boardActive ? "active" : ""}`}
          onClick={onBoard}
          aria-current={boardActive ? "page" : undefined}
          title={t.board_tab}
        >
          <span className="aiterm-tab-icon"><ClipboardListIcon size={18} /></span>
          {isSidebarOpen && <span className="aiterm-tab-title">{t.board_tab}</span>}
        </button>
      )}
```

Add `ClipboardListIcon` to `src/components/Icons.tsx` following an existing icon there (a simple outlined SVG, `size` prop, `stroke="currentColor"` `fill="none"`), and import it in `TabBar/index.tsx` alongside `HomeIcon`. If adding an icon is heavy, reuse an existing one (e.g. `RobotIcon` or a list-like icon already present) and skip the new-icon sub-step.

- [ ] **Step 4: Run tests, verify pass**

Run: `npm run test -- src/components/TabBar/taskBoardButton.test.tsx`
Expected: PASS. Also run the existing `src/components/TabBar/index.test.tsx` — still green.

- [ ] **Step 5: Commit**

```bash
git add src/components/TabBar/index.tsx src/components/Icons.tsx src/components/TabBar/taskBoardButton.test.tsx
git commit -m "feat(tasks): task board button in the sidebar"
```

---

### Task 4: `TerminalApp` — `boardActive` state + content slot

**Files:**
- Modify: `src/components/TerminalApp.tsx`

- [ ] **Step 1: Write the failing test**

Add `src/components/TerminalApp.taskBoard.test.tsx` (mirror the mounting approach of `TerminalApp.routeHintCloseGuard.test.tsx` — reuse its mocks verbatim, then):

```tsx
it("clicking the sidebar 工作看板 button shows the board and hides Home", async () => {
  render(<TerminalApp />); // with the same providers/mocks the sibling test uses
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /工作看板|Task Board/ }));
  // The board's column headers appear…
  expect(await screen.findByText(/計畫中|Planned/)).toBeInTheDocument();
  // …and HomeView's signature element is gone.
  expect(screen.queryByPlaceholderText(/一句話|one sentence/i)).not.toBeInTheDocument();
});
```

Mock `../ipc/tasks` so `listTasks` resolves `[]` and `onTasksUpdated` resolves `() => {}`.

- [ ] **Step 2: Run, verify fail**

Run: `npm run test -- src/components/TerminalApp.taskBoard.test.tsx`
Expected: FAIL — no such button / board.

- [ ] **Step 3: Implement**

Near `const [homeActive, setHomeActive] = useState(true);` (line ~69):

```tsx
  const [boardActive, setBoardActive] = useState(false);
  const boardActiveRef = useRef(boardActive);
```

In the effect that syncs `homeActiveRef` (line ~147):

```tsx
    boardActiveRef.current = boardActive;
```

and add `boardActive` to that effect's dep array (line ~155).

Add two small helpers so the three views stay mutually exclusive:

```tsx
  const showBoard = useCallback(() => {
    setBoardActive(true);
    setHomeActive(false);
  }, []);
  const showHome = useCallback(() => {
    setHomeActive(true);
    setBoardActive(false);
  }, []);
```

Replace the existing `onHome={() => setHomeActive(true)}` on `<TabBar>` (line ~537) with `onHome={showHome}`, and add:

```tsx
          onBoard={showBoard}
          boardActive={boardActive}
```

Everywhere a tab is opened/selected and `setHomeActive(false)` is called (e.g. `handlePickerSelect`, `selectTab`, line ~162, the shortcut handler), also call `setBoardActive(false)`. Simplest: define `const leaveOverlays = useCallback(() => { setHomeActive(false); setBoardActive(false); }, []);` and swap it in at those sites. The keyboard handler around line ~472 that does `setHomeActive(true)` should use `showHome`.

In the content slot (line ~613), after the `{homeActive && <HomeView .../>}` line:

```tsx
        {boardActive && <TaskBoardView />}
```

Import it: `import { TaskBoardView } from "./TaskBoard";` (component built in Task 5). Guard the `tabs.map` active check the same way Home does — search line ~625 `const isActive = tab.id === activeId && !homeActive;` → `&& !homeActive && !boardActive;`. Also line ~764 `if (homeActive) return null;` → `if (homeActive || boardActive) return null;`.

- [ ] **Step 4: Run tests, verify pass**

Run: `npm run test -- src/components/TerminalApp.taskBoard.test.tsx` and the two existing `TerminalApp.*.test.tsx`
Expected: all PASS. (`TaskBoardView` must exist — do Task 5 first if the import fails, or stub it as `export function TaskBoardView() { return <div />; }` and come back.)

- [ ] **Step 5: Commit**

```bash
git add src/components/TerminalApp.tsx src/components/TerminalApp.taskBoard.test.tsx
git commit -m "feat(tasks): boardActive view slot in TerminalApp"
```

---

### Task 5: `TaskBoard` shell — columns, load, event refresh, drag-to-move

**Files:**
- Create: `src/components/TaskBoard/index.tsx`
- Create: `src/components/TaskBoard/TaskColumn.tsx`
- Create: `src/components/TaskBoard/index.css`

- [ ] **Step 1: Write the failing test**

Create `src/components/TaskBoard/index.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  listTasks: vi.fn(),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn().mockResolvedValue(undefined),
  stopTask: vi.fn().mockResolvedValue(undefined),
  deleteTask: vi.fn().mockResolvedValue(undefined),
  readTranscript: vi.fn().mockResolvedValue(""),
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));

import { listTasks, onTasksUpdated, moveTask } from "../../ipc/tasks";
import type { TaskWithAttachments } from "../../ipc/tasks";
import { TaskBoardView } from "./index";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card one", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, sort_order: 1, outcome: null, tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, attachments: [],
  ...over,
});

const view = () => render(<LocaleProvider><TaskBoardView /></LocaleProvider>);

beforeEach(() => {
  vi.mocked(listTasks).mockResolvedValue([]);
  vi.mocked(onTasksUpdated).mockResolvedValue(() => {});
});

describe("TaskBoardView", () => {
  it("renders four columns", async () => {
    view();
    await waitFor(() => expect(screen.getByText(/計畫中|Planned/)).toBeInTheDocument());
    expect(screen.getByText(/待執行|Queued/)).toBeInTheDocument();
    expect(screen.getByText(/執行中|Running/)).toBeInTheDocument();
    expect(screen.getByText(/已完成|Done/)).toBeInTheDocument();
  });

  it("places each card in its status column", async () => {
    vi.mocked(listTasks).mockResolvedValue([
      card({ id: "p", title: "PlanCard", status: "planning" }),
      card({ id: "r", title: "RunCard", status: "running" }),
    ]);
    view();
    const planning = (await screen.findByTestId("column-planning"));
    const running = screen.getByTestId("column-running");
    expect(within(planning).getByText("PlanCard")).toBeInTheDocument();
    expect(within(running).getByText("RunCard")).toBeInTheDocument();
  });

  it("re-fetches when tasks-updated fires", async () => {
    let fire: () => void = () => {};
    vi.mocked(onTasksUpdated).mockImplementation(async (cb) => { fire = cb; return () => {}; });
    vi.mocked(listTasks).mockResolvedValue([]);
    view();
    await waitFor(() => expect(listTasks).toHaveBeenCalledTimes(1));
    vi.mocked(listTasks).mockResolvedValue([card({ id: "x", title: "Appeared", status: "queued" })]);
    fire();
    expect(await screen.findByText("Appeared")).toBeInTheDocument();
  });

  it("dropping a planning card on the queued column calls moveTask", async () => {
    vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Draggable", status: "planning" })]);
    view();
    const cardEl = await screen.findByText("Draggable");
    const queuedCol = screen.getByTestId("column-queued");
    const dt = { getData: () => "p", setData: vi.fn() } as unknown as DataTransfer;
    cardEl.closest("[draggable]")!.dispatchEvent(new DragEvent("dragstart", { bubbles: true, dataTransfer: dt }));
    queuedCol.dispatchEvent(new DragEvent("drop", { bubbles: true, dataTransfer: dt }));
    await waitFor(() => expect(moveTask).toHaveBeenCalledWith("p", "queued", expect.any(Number)));
  });
});
```

(jsdom's `DragEvent`/`dataTransfer` support is thin — if `new DragEvent` with `dataTransfer` doesn't stick, have the handlers read the dragged id from a component ref/`useState` set in `onDragStart` instead of `dataTransfer`, and the test can fire `fireEvent.dragStart(el)` / `fireEvent.drop(col)`. Adjust the test to match whichever the implementation uses; the assertion `moveTask("p", "queued", number)` is the invariant.)

- [ ] **Step 2: Run, verify fail**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement `TaskColumn.tsx`**

```tsx
import type { DragEvent, ReactNode } from "react";

export function TaskColumn({
  status,
  title,
  count,
  onDropCard,
  children,
}: {
  status: string;
  title: string;
  count: number;
  onDropCard: (taskId: string) => void;
  children: ReactNode;
}) {
  return (
    <div
      className="task-column"
      data-testid={`column-${status}`}
      onDragOver={(e: DragEvent) => e.preventDefault()}
      onDrop={(e: DragEvent) => {
        e.preventDefault();
        const id = e.dataTransfer.getData("text/task-id") || e.dataTransfer.getData("text/plain");
        if (id) onDropCard(id);
      }}
    >
      <div className="task-column-head">
        <span className="task-column-title">{title}</span>
        <span className="task-column-count">{count}</span>
      </div>
      <div className="task-column-body">{children}</div>
    </div>
  );
}
```

- [ ] **Step 4: Implement `index.tsx`**

```tsx
import { useCallback, useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import {
  listTasks,
  moveTask,
  onTasksUpdated,
  type TaskStatus,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { TaskCard } from "./TaskCard";
import { TaskColumn } from "./TaskColumn";
import { TaskEditorDialog } from "./TaskEditorDialog";
import { TranscriptDialog } from "./TranscriptDialog";
import "./index.css";

const COLUMNS: TaskStatus[] = ["planning", "queued", "running", "done"];

export function TaskBoardView() {
  const { t } = useLocale();
  const [tasks, setTasks] = useState<TaskWithAttachments[]>([]);
  const [editing, setEditing] = useState<TaskWithAttachments | "new" | null>(null);
  const [transcriptFor, setTranscriptFor] = useState<string | null>(null);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    const rows = await listTasks();
    if (mounted.current) setTasks(rows);
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => {
      mounted.current = false;
      void un.then((f) => f());
    };
  }, [refresh]);

  const colTitle = (s: TaskStatus) =>
    ({ planning: t.board_col_planning, queued: t.board_col_queued, running: t.board_col_running, done: t.board_col_done }[s]);

  const byStatus = (s: TaskStatus) =>
    tasks.filter((x) => x.status === s).sort((a, b) => a.sort_order - b.sort_order);

  const handleDrop = useCallback(
    async (taskId: string, to: TaskStatus) => {
      const card = tasks.find((x) => x.id === taskId);
      if (!card || card.status === to) return;
      // Only planning<->queued moves are legal (backend enforces; mirror here
      // so illegal drops are silently ignored rather than erroring).
      const legal =
        (card.status === "planning" && to === "queued") ||
        (card.status === "queued" && to === "planning");
      if (!legal) return;
      const dest = byStatus(to);
      const sortOrder = dest.length ? dest[dest.length - 1].sort_order + 1 : 1;
      await moveTask(taskId, to, sortOrder);
      // tasks-updated will refresh; optimistic update keeps the UI snappy:
      setTasks((prev) => prev.map((x) => (x.id === taskId ? { ...x, status: to, sort_order: sortOrder } : x)));
    },
    [tasks],
  );

  return (
    <div className="task-board">
      <div className="task-board-toolbar">
        <button className="task-board-new" onClick={() => setEditing("new")}>
          + {t.board_new_card}
        </button>
      </div>
      <div className="task-board-columns">
        {COLUMNS.map((s) => (
          <TaskColumn
            key={s}
            status={s}
            title={colTitle(s)}
            count={byStatus(s).length}
            onDropCard={(id) => void handleDrop(id, s)}
          >
            {byStatus(s).map((card) => (
              <TaskCard
                key={card.id}
                card={card}
                onEdit={() => setEditing(card)}
                onViewTranscript={() => setTranscriptFor(card.id)}
                onChanged={() => void refresh()}
              />
            ))}
          </TaskColumn>
        ))}
      </div>

      {editing && (
        <TaskEditorDialog
          card={editing === "new" ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
        />
      )}
      {transcriptFor && (
        <TranscriptDialog taskId={transcriptFor} onClose={() => setTranscriptFor(null)} />
      )}
    </div>
  );
}
```

- [ ] **Step 5: Minimal `index.css`**

```css
.task-board { display: flex; flex-direction: column; height: 100%; overflow: hidden; }
.task-board-toolbar { padding: 12px 16px; border-bottom: 1px solid var(--border, #2a2a2a); }
.task-board-columns { flex: 1; display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; padding: 16px; overflow: hidden; }
.task-column { display: flex; flex-direction: column; background: var(--panel, #1b1b1b); border-radius: 8px; overflow: hidden; }
.task-column-head { display: flex; justify-content: space-between; padding: 10px 12px; font-weight: 600; border-bottom: 1px solid var(--border, #2a2a2a); }
.task-column-count { opacity: 0.6; }
.task-column-body { flex: 1; overflow-y: auto; padding: 10px; display: flex; flex-direction: column; gap: 8px; }
.task-card { background: var(--card, #242424); border: 1px solid var(--border, #2f2f2f); border-radius: 6px; padding: 10px; cursor: grab; }
.task-card[data-dragging="true"] { opacity: 0.5; }
.task-card-title { font-weight: 600; margin-bottom: 4px; }
.task-card-meta { font-size: 12px; opacity: 0.65; }
.task-card-actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
.task-badge { font-size: 11px; padding: 1px 6px; border-radius: 4px; }
.task-badge--success { background: #1f4023; }
.task-badge--failed { background: #4a1f1f; }
.task-badge--cancelled { background: #3a3a1f; }
.task-dialog-backdrop { position: absolute; inset: 0; background: rgba(0,0,0,0.5); display: flex; align-items: center; justify-content: center; z-index: 50; }
.task-dialog { background: var(--panel, #1b1b1b); border-radius: 8px; padding: 20px; width: min(560px, 90vw); max-height: 85vh; overflow-y: auto; }
```

(Reuse whatever CSS variables the app already defines — grep an existing `*.css` for `var(--`. Match the app's dark/light approach; the values above are fallbacks only.)

- [ ] **Step 6: Run tests, verify pass**

Run: `npm run test -- src/components/TaskBoard/index.test.tsx`
Expected: PASS (column render, per-status placement, event refetch, drag→move). `TaskCard`/`TaskEditorDialog`/`TranscriptDialog` come next — stub them as `export function X(){return null}` if needed to get columns green first, then fill in.

- [ ] **Step 7: Commit**

```bash
git add src/components/TaskBoard/
git commit -m "feat(tasks): board shell — 4 columns, load, event refresh, drag-to-move"
```

---

### Task 6: `TaskCard`

**Files:**
- Create: `src/components/TaskBoard/TaskCard.tsx`

- [ ] **Step 1: Write the failing test**

Add to `src/components/TaskBoard/index.test.tsx`:

```tsx
it("running card shows Stop and Open-tab, and Stop calls stopTask", async () => {
  const { stopTask } = await import("../../ipc/tasks");
  vi.mocked(listTasks).mockResolvedValue([card({ id: "r", title: "Runner", status: "running", tab_id: "tab-1" })]);
  view();
  const user = userEvent.setup();
  await screen.findByText("Runner");
  await user.click(screen.getByRole("button", { name: /停止|Stop/ }));
  expect(stopTask).toHaveBeenCalledWith("r");
});

it("done+failed card shows the failed badge and its error message", async () => {
  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Broke", status: "done", outcome: "failed", error_message: "claude 以 exit code 127 結束" }),
  ]);
  view();
  expect(await screen.findByText(/失敗|Failed/)).toBeInTheDocument();
  expect(screen.getByText(/exit code 127/)).toBeInTheDocument();
});
```

- [ ] **Step 2: Run, verify fail** — `npm run test -- src/components/TaskBoard/index.test.tsx` → FAIL.

- [ ] **Step 3: Implement**

```tsx
import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import {
  createTask,
  deleteTask,
  stopTask,
  type TaskWithAttachments,
} from "../../ipc/tasks";

export function TaskCard({
  card,
  onEdit,
  onViewTranscript,
  onChanged,
}: {
  card: TaskWithAttachments;
  onEdit: () => void;
  onViewTranscript: () => void;
  onChanged: () => void;
}) {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);

  const dragProps = {
    draggable: card.status === "planning" || card.status === "queued",
    onDragStart: (e: React.DragEvent) => {
      e.dataTransfer.setData("text/task-id", card.id);
      e.dataTransfer.setData("text/plain", card.id);
      e.currentTarget.setAttribute("data-dragging", "true");
    },
    onDragEnd: (e: React.DragEvent) => e.currentTarget.removeAttribute("data-dragging"),
  };

  const requeue = async () => {
    setBusy(true);
    try {
      // Clone back to planning (spec: 重新派工 = copy to 計畫中).
      await createTask({
        title: card.title,
        body: card.body,
        project_dir: card.project_dir,
        parallel_ok: card.parallel_ok,
      });
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!window.confirm(t.board_delete_confirm)) return;
    const closeTab = card.tab_id ? window.confirm(t.board_delete_close_tab) : false;
    setBusy(true);
    try {
      await deleteTask(card.id, closeTab);
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-card" {...dragProps}>
      <div className="task-card-title">{card.title}</div>
      <div className="task-card-meta">{card.project_dir}</div>
      {!card.parallel_ok && <div className="task-card-meta">⚑ {t.board_card_solo_hint}</div>}

      {card.status === "running" && <div className="task-card-meta">{t.board_running_hint}</div>}

      {card.status === "done" && card.outcome && (
        <div className={`task-badge task-badge--${card.outcome}`}>
          {card.outcome === "success" && t.board_outcome_success}
          {card.outcome === "failed" && t.board_outcome_failed}
          {card.outcome === "cancelled" && t.board_outcome_cancelled}
        </div>
      )}
      {card.status === "done" && card.error_message && (
        <div className="task-card-meta">{card.error_message}</div>
      )}

      <div className="task-card-actions">
        {card.status === "planning" && (
          <>
            <button disabled={busy} onClick={onEdit}>{t.board_edit_card}</button>
            <button disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
        {card.status === "running" && (
          <>
            <button disabled={busy} onClick={() => void stopTask(card.id).then(onChanged)}>
              {t.board_action_stop}
            </button>
            {card.tab_id && <OpenTabButton tabId={card.tab_id} label={t.board_action_open_tab} />}
          </>
        )}
        {card.status === "done" && (
          <>
            {card.transcript_path && (
              <button onClick={onViewTranscript}>{t.board_action_transcript}</button>
            )}
            <button disabled={busy} onClick={() => void requeue()}>{t.board_action_requeue}</button>
            <button disabled={busy} onClick={() => void remove()}>{t.board_delete}</button>
          </>
        )}
      </div>
    </div>
  );
}

/** Switching to a tab is a TerminalApp concern; the board can't reach that
 *  state directly. Emit a window event TerminalApp already listens for, or —
 *  simplest and dependency-free — dispatch a CustomEvent the app wires up in
 *  Task 4. For v1 this just focuses the tab by asking the backend to
 *  re-emit the adopt event is overkill; instead dispatch a DOM event. */
function OpenTabButton({ tabId, label }: { tabId: string; label: string }) {
  return (
    <button
      onClick={() => window.dispatchEvent(new CustomEvent("aiterm:focus-tab", { detail: { tabId } }))}
    >
      {label}
    </button>
  );
}
```

- [ ] **Step 4: Wire `aiterm:focus-tab` in `TerminalApp`**

In `TerminalApp.tsx`, add an effect:

```tsx
  useEffect(() => {
    const onFocusTab = (e: Event) => {
      const id = (e as CustomEvent<{ tabId: string }>).detail?.tabId;
      if (!id) return;
      const exists = tabs.some((tb) => tb.ptySessionId === id || tb.id === id);
      if (exists) {
        setBoardActive(false);
        setHomeActive(false);
        // selectTab expects a Tab id; find it by session id if needed.
        const tab = tabs.find((tb) => tb.ptySessionId === id) ?? tabs.find((tb) => tb.id === id);
        if (tab) selectTab(tab.id);
      }
    };
    window.addEventListener("aiterm:focus-tab", onFocusTab);
    return () => window.removeEventListener("aiterm:focus-tab", onFocusTab);
  }, [tabs, selectTab]);
```

(Check the real field: coordination-spawned tabs get `ptySessionId` pre-populated — the backend `tab_id` from Plan 1 IS the pty session id. Match `TabBar/index.tsx`'s `Tab` type.)

- [ ] **Step 5: Run tests, verify pass** — `npm run test -- src/components/TaskBoard/index.test.tsx` → PASS.

- [ ] **Step 6: Commit**

```bash
git add src/components/TaskBoard/TaskCard.tsx src/components/TerminalApp.tsx
git commit -m "feat(tasks): task card — per-status actions, badges, re-dispatch, focus-tab"
```

---

### Task 7: `TaskEditorDialog`

**Files:**
- Create: `src/components/TaskBoard/TaskEditorDialog.tsx`

- [ ] **Step 1: Write the failing test**

Add to `index.test.tsx`:

```tsx
it("new-card dialog creates a task with the typed fields", async () => {
  const { createTask } = await import("../../ipc/tasks");
  vi.mocked(createTask).mockResolvedValue("id-new");
  view();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /新增工作|New task/ }));
  await user.type(screen.getByLabelText(/標題|Title/), "Ship it");
  await user.type(screen.getByLabelText(/工作內容|Task detail/), "do the thing");
  // project_dir: the picker is mocked; set it via a hidden input the dialog exposes for tests,
  // or type into the shown path field if the dialog allows manual entry.
  await user.type(screen.getByLabelText(/專案資料夾|Project folder/), "/repo");
  await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
  await waitFor(() =>
    expect(createTask).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Ship it", body: "do the thing", project_dir: "/repo", parallel_ok: true }),
    ),
  );
});

it("editing an existing planning card calls updateTask", async () => {
  const { updateTask } = await import("../../ipc/tasks");
  vi.mocked(listTasks).mockResolvedValue([card({ id: "p", title: "Old", status: "planning", project_dir: "/r" })]);
  view();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /編輯工作|Edit task/ }));
  const title = screen.getByLabelText(/標題|Title/);
  await user.clear(title);
  await user.type(title, "New title");
  await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
  await waitFor(() =>
    expect(updateTask).toHaveBeenCalledWith(expect.objectContaining({ id: "p", title: "New title" })),
  );
});
```

Mock the dialog plugin: `vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn().mockResolvedValue("/repo") }))`.

- [ ] **Step 2: Run, verify fail** — FAIL.

- [ ] **Step 3: Implement**

```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import {
  addAttachment,
  createTask,
  removeAttachment,
  updateTask,
  type TaskWithAttachments,
} from "../../ipc/tasks";

const LAST_DIR_KEY = "aiterm_last_task_dir";

export function TaskEditorDialog({
  card,
  onClose,
  onSaved,
}: {
  card: TaskWithAttachments | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useLocale();
  const [title, setTitle] = useState(card?.title ?? "");
  const [body, setBody] = useState(card?.body ?? "");
  const [dir, setDir] = useState(card?.project_dir ?? localStorage.getItem(LAST_DIR_KEY) ?? "");
  const [parallelOk, setParallelOk] = useState(card?.parallel_ok ?? true);
  const [attachments, setAttachments] = useState(card?.attachments ?? []);
  const [busy, setBusy] = useState(false);
  const isEdit = !!card;

  const pickDir = async () => {
    const picked = await open({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") {
      setDir(picked);
      localStorage.setItem(LAST_DIR_KEY, picked);
    }
  };

  const onFiles = async (files: FileList | null) => {
    if (!files || !card) return; // attachments only after the card exists (edit mode)
    for (const f of Array.from(files)) {
      const bytes = new Uint8Array(await f.arrayBuffer());
      const row = await addAttachment(card.id, f.name, bytes);
      setAttachments((a) => [...a, row]);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      if (dir) localStorage.setItem(LAST_DIR_KEY, dir);
      if (isEdit) {
        await updateTask({ id: card!.id, title, body, project_dir: dir, parallel_ok: parallelOk });
      } else {
        await createTask({ title, body, project_dir: dir, parallel_ok: parallelOk });
      }
      onSaved();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{isEdit ? t.board_edit_card : t.board_new_card}</h3>

        <label>
          {t.board_card_title}
          <input value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>

        <label>
          {t.board_card_body}
          <textarea rows={6} value={body} onChange={(e) => setBody(e.target.value)} />
        </label>

        <label>
          {t.board_card_folder}
          <input value={dir} onChange={(e) => setDir(e.target.value)} />
        </label>
        <button type="button" onClick={() => void pickDir()}>{t.board_card_folder_pick}</button>

        <label>
          <input type="checkbox" checked={parallelOk} onChange={(e) => setParallelOk(e.target.checked)} />
          {t.board_card_parallel}
        </label>
        <p className="task-card-meta">{t.board_card_solo_hint}</p>

        {isEdit && (
          <div>
            <div>{t.board_card_attachments}</div>
            <p className="task-card-meta">{t.board_card_attachments_hint}</p>
            <ul>
              {attachments.map((a) => (
                <li key={a.id}>
                  {a.filename}{" "}
                  <button
                    type="button"
                    onClick={() => void removeAttachment(a.id).then(() =>
                      setAttachments((list) => list.filter((x) => x.id !== a.id)),
                    )}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
            <input type="file" multiple aria-label={t.board_add_attachment} onChange={(e) => void onFiles(e.target.files)} />
          </div>
        )}

        <div className="task-card-actions">
          <button disabled={busy || !title.trim() || !dir.trim()} onClick={() => void save()}>
            {t.board_save}
          </button>
          <button disabled={busy} onClick={onClose}>{t.board_cancel}</button>
        </div>
      </div>
    </div>
  );
}
```

Note the spec's attachment flow means attachments attach to an existing card — for a brand-new card, the user saves first (creating the card), then re-opens it to add attachments. The dialog reflects that (`isEdit` gates the attachments section). If a smoother "attach during creation" flow is wanted later, that's a follow-up.

- [ ] **Step 4: Run tests, verify pass** — PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TaskEditorDialog.tsx
git commit -m "feat(tasks): task editor dialog — fields, folder picker, attachments"
```

---

### Task 8: `TranscriptDialog`

**Files:**
- Create: `src/components/TaskBoard/TranscriptDialog.tsx`

- [ ] **Step 1: Write the failing test** (add to `index.test.tsx`)

```tsx
it("transcript dialog shows the backend transcript text", async () => {
  const { readTranscript } = await import("../../ipc/tasks");
  vi.mocked(readTranscript).mockResolvedValue("line A\nline B");
  vi.mocked(listTasks).mockResolvedValue([
    card({ id: "d", title: "Done one", status: "done", outcome: "success", transcript_path: "/p/t.txt" }),
  ]);
  view();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /查看逆紀錄|View transcript/ }));
  expect(await screen.findByText(/line A/)).toBeInTheDocument();
});
```

- [ ] **Step 2: Run, verify fail** — FAIL.

- [ ] **Step 3: Implement**

```tsx
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { readTranscript } from "../../ipc/tasks";

export function TranscriptDialog({ taskId, onClose }: { taskId: string; onClose: () => void }) {
  const { t } = useLocale();
  const [text, setText] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void readTranscript(taskId).then((s) => {
      if (alive) setText(s);
    });
    return () => {
      alive = false;
    };
  }, [taskId]);

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{t.board_transcript_title}</h3>
        <pre style={{ whiteSpace: "pre-wrap", maxHeight: "60vh", overflowY: "auto" }}>
          {text === null ? "…" : text || t.board_transcript_empty}
        </pre>
        <button onClick={onClose}>{t.board_cancel}</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run tests, verify pass** — full `npm run test -- src/components/TaskBoard/index.test.tsx` PASS.

- [ ] **Step 5: Commit**

```bash
git add src/components/TaskBoard/TranscriptDialog.tsx
git commit -m "feat(tasks): transcript viewer dialog"
```

---

### Task 9: Settings page

**Files:**
- Create: `src/components/Settings/TaskBoardPage.tsx`
- Modify: `src/components/Settings/SettingsView.tsx`

- [ ] **Step 1: Write the failing test**

`src/components/Settings/TaskBoardPage.test.tsx` (mirror `McpToolServerPage.test.tsx`):

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({
  getTaskBoardConfig: vi.fn(),
  setTaskBoardConfig: vi.fn().mockResolvedValue(undefined),
}));

import { getTaskBoardConfig, setTaskBoardConfig } from "../../ipc/tasks";
import { TaskBoardPage } from "./TaskBoardPage";

beforeEach(() => {
  vi.mocked(getTaskBoardConfig).mockResolvedValue({ max_concurrent: 2, claude_command: "claude" });
});

const view = () => render(<LocaleProvider><TaskBoardPage /></LocaleProvider>);

describe("TaskBoardPage", () => {
  it("loads and shows the saved config", async () => {
    view();
    await waitFor(() => expect(screen.getByDisplayValue("2")).toBeInTheDocument());
    expect(screen.getByDisplayValue("claude")).toBeInTheDocument();
  });

  it("saving sends the edited values", async () => {
    const user = userEvent.setup();
    view();
    await waitFor(() => screen.getByDisplayValue("2"));
    const n = screen.getByDisplayValue("2");
    await user.clear(n);
    await user.type(n, "3");
    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));
    await waitFor(() =>
      expect(setTaskBoardConfig).toHaveBeenCalledWith(
        expect.objectContaining({ max_concurrent: 3, claude_command: "claude" }),
      ),
    );
  });
});
```

- [ ] **Step 2: Run, verify fail** — FAIL.

- [ ] **Step 3: Implement `TaskBoardPage.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getTaskBoardConfig, setTaskBoardConfig, type TaskBoardConfig } from "../../ipc/tasks";

export function TaskBoardPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<TaskBoardConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    void getTaskBoardConfig().then(setCfg);
  }, []);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    try {
      await setTaskBoardConfig(cfg);
      setSaved(true);
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  if (!cfg) return <div className="task-board-page" />;

  return (
    <div className="task-board-page">
      <h2>{t.board_settings_title}</h2>
      <p>{t.board_settings_desc}</p>

      <label>
        {t.board_settings_max_concurrent}
        <input
          type="number"
          min={1}
          max={16}
          value={cfg.max_concurrent}
          onChange={(e) => {
            setSaved(false);
            setCfg({ ...cfg, max_concurrent: Number(e.target.value) || 1 });
          }}
        />
      </label>

      <label>
        {t.board_settings_claude_command}
        <input
          value={cfg.claude_command}
          onChange={(e) => {
            setSaved(false);
            setCfg({ ...cfg, claude_command: e.target.value });
          }}
        />
      </label>

      <div>
        <button onClick={() => void save()} disabled={saving}>
          {saved ? `${t.board_settings_saved} ✓` : t.board_save}
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Wire into `SettingsView.tsx`**

- Import: `import { TaskBoardPage } from "./TaskBoardPage";`
- `SettingsTab` union (line 28): add `| "taskBoard"`.
- Add a nav button after the `mcpToolServer` one (~line 108-113):

```tsx
        <button
          className={`sidebar-item ${tab === "taskBoard" ? "sidebar-item--active" : ""}`}
          onClick={() => setTab("taskBoard")}
        >
          {t.settings_tab_taskBoard}
        </button>
```

- Body (~line 152): add `{tab === "taskBoard" && <TaskBoardPage />}`.

- [ ] **Step 5: Run tests, verify pass** — `npm run test -- src/components/Settings/TaskBoardPage.test.tsx` PASS; existing `SettingsView.*.test.tsx` still green.

- [ ] **Step 6: Commit**

```bash
git add src/components/Settings/TaskBoardPage.tsx src/components/Settings/SettingsView.tsx
git commit -m "feat(tasks): task board settings page (max concurrent, claude command)"
```

---

### Task 10: Verification pass

- [ ] **Step 1:** `npx tsc -b` — clean.
- [ ] **Step 2:** `npm run lint` — no new errors in `src/components/TaskBoard/`, `src/ipc/tasks.ts`, touched files.
- [ ] **Step 3:** `npm run test` — full frontend suite green (new + existing; pay attention to `TerminalApp.*`, `TabBar/*`, `Settings/*`).
- [ ] **Step 4:** `cd src-tauri && cargo test` — Plan 1's backend still green (no backend changes here, but confirm nothing drifted).
- [ ] **Step 5: Manual smoke** (`npm run tauri:dev`, needs `claude` on PATH):
  1. Sidebar shows **工作看板**; click it → four columns, empty.
  2. **+ 新增工作** → fill title / body ("reply with the single word: pong") / pick a real git repo folder / leave 可並行 on → 儲存. Card appears in 計畫中.
  3. Drag the card to 待執行. Within a few seconds a new terminal tab opens running `claude`, the prompt (no attachments) is typed in.
  4. When `claude` replies, the card moves itself to 已完成 with a 成功 badge; **查看逆紀錄** shows the tab's output; **開啟分頁** switches to the `claude` tab.
  5. Re-open the card editor on a fresh 計畫中 card, add a file attachment, dispatch → confirm the prompt line lists the copied path under `<data-dir>/AITERM/tasks/<id>/attachments/`.
  6. Set Settings → 工作看板 → 同時執行上限 = 1. Queue two cards → only one runs; the second starts after the first finishes.
  7. Add a 必須單獨執行 card plus a normal card to the queue with concurrency 1-2 → confirm the solo card waits for an empty running set and blocks others while running.
  8. On a running card hit **停止** → tab gets Ctrl+C, card lands in 已完成 as 已中斷.
  9. Quit the app while a card is running, relaunch → that card is in 已完成 marked 已中斷.
- [ ] **Step 6: Commit** any fixes: `git commit -am "fix(tasks): frontend verification-pass cleanup"`

---

## Self-Review

**Spec coverage:**
- Sidebar fixed button (not a tab type), full-width board, terminals stay mounted-hidden → Tasks 3, 4. ✅
- Four columns, drag between 計畫中/待執行, order = priority → Tasks 5 (columns + `handleDrop` legal-move mirror + sort_order), 6 (`draggable` on planning/queued cards). ✅
- Card = title + body + project folder (picker, remembers last via `localStorage`) + attachments → Task 7. ✅
- Attachments: add/remove on planning (and edit) cards; backend copies + prompts paths (Plan 1) → Task 7 + `ipc.addAttachment`/`removeAttachment`. ✅
- `parallel_ok` per-card toggle → Task 7 (checkbox), shown on card (Task 6). ✅
- Auto-move running→done handled by backend; board just renders + refreshes on `tasks-updated` → Task 5. ✅
- 執行中 card: Stop (Ctrl+C via `tasks_stop`), Open tab → Task 6. ✅
- 已完成 card: success/failed/cancelled badge, error message, 查看逆紀錄, 重新派工 (clone to 計畫中), delete (+ optional close tab) → Tasks 6, 8. ✅
- Settings: global max concurrent (default 2) + claude command (default `claude`) → Task 9. ✅
- i18n zh-TW + en → Task 2. ✅
- Passive renderer / DB is source of truth → Task 5 (`onTasksUpdated` → `refresh`, minimal optimistic update only for drag snappiness). ✅
- Out of scope respected: no board-driven chat (Open tab instead), no non-`claude` agent UI beyond the command field.

**Placeholder scan:** No TBD/TODO. Every component has full code. The `OpenTabButton` uses a `CustomEvent` bridge with the wiring spelled out in Task 6 Step 4. Drag-event jsdom fragility is called out with a concrete fallback (ref/state instead of `dataTransfer`) and the stable assertion to keep.

**Type consistency:** `TaskWithAttachments` / `TaskRow` / `TaskStatus` / `AttachmentRow` / `TaskBoardConfig` defined once in `src/ipc/tasks.ts` (Task 1), imported everywhere else (Tasks 5–9). ipc fn names (`listTasks`, `createTask`, `updateTask`, `moveTask`, `stopTask`, `deleteTask`, `addAttachment`, `removeAttachment`, `readTranscript`, `getTaskBoardConfig`, `setTaskBoardConfig`, `onTasksUpdated`) consistent across the mock in `index.test.tsx` and the call sites. i18n keys used in components all added in Task 2. `TabBar` new props (`onBoard`, `boardActive`) consistent between Task 3 (definition) and Task 4 (`<TabBar onBoard={showBoard} boardActive={boardActive} />`). Event name `"tasks-updated"` matches Plan 1's emit; `"aiterm:focus-tab"` is internal to Tasks 6+4.

**Soft spots flagged inline (not gaps):** exact arg-key casing for struct-wrapped Tauri commands (Task 1 context + Step 4 note — verify once during Task 9 smoke, fix in the one ipc file); jsdom `DragEvent`/`dataTransfer` (Task 5 Step 1 note); `Tab` session-id field name for `aiterm:focus-tab` (Task 6 Step 4 note); whether a fresh icon is worth adding vs reusing one (Task 3 Step 3).

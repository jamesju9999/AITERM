import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// Mount probe (done ad-hoc before writing this file, then discarded): TerminalApp
// mounts fully in jsdom with the same three low-level Tauri entry points that
// TerminalView.closeGuard.test.tsx uses (invoke/listen/homeDir cover every
// ipc/*.ts module transitively), plus @tauri-apps/api/window (TitleBar reads
// window state) and @tauri-apps/plugin-notification (TerminalApp sends
// notifications on attention events). invoke() never resolves, which is fine:
// nothing in these tests waits on it.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFocused: () => Promise.resolve(true),
    onFocusChanged: () => Promise.resolve(() => {}),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    maximize: () => Promise.resolve(),
    unmaximize: () => Promise.resolve(),
    minimize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: vi.fn(),
}));

// LoopStudioView is the tab type this file uses to exercise the close guard
// (see rationale below the imports): mounting it only requires these two
// mocks, per LoopStudio/closeGuard.test.tsx's comment — listProviders is the
// only IPC call it makes on mount, and useOrchestratorLoop is the hook whose
// isRunning flag its guard reads.
vi.mock("../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));
const fakeLoop = {
  trace: [] as unknown[],
  isRunning: false,
  iteration: 0,
  start: vi.fn(),
  stop: vi.fn(),
  resume: vi.fn(),
  pendingConfirmation: null as unknown,
};
vi.mock("../hooks/useOrchestratorLoop", () => ({
  useOrchestratorLoop: () => fakeLoop,
}));

// handleCloseTab's "closed the last tab" branch (TerminalApp.tsx ~line 322)
// recreates a fresh terminal tab whenever the tab being closed was the only
// one left — which happens in both scenarios below (the sole starting tab is
// the LoopStudio one). That fresh tab mounts a real TerminalView, so it needs
// the same two hook mocks TerminalView.closeGuard.test.tsx uses (idle: no
// running block, no active mission) plus the browser API polyfills xterm.js
// calls unconditionally on mount.
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

// HomeView itself is not what this test exercises (routing logic + AI query
// live in HomeInput/routeIntent, covered elsewhere). Stubbing it to two
// buttons lets the test drive TerminalApp's real state machine — selectTab,
// then handleAiRouted — without choreographing HomeView's internals.
type HomeViewProps = {
  tabs: { id: string; type: string }[];
  onSelectTab: (id: string) => void;
  onAiRouted: (tabId: string, route: { type: string; userText: string; fallback: boolean }) => void;
};
vi.mock("./HomeView", () => ({
  HomeView: ({ tabs, onSelectTab, onAiRouted }: HomeViewProps) => (
    <div>
      <button onClick={() => onSelectTab(tabs[0].id)}>select-first-tab</button>
      <button
        onClick={() => onAiRouted(tabs[0].id, { type: tabs[0].type, userText: "test goal", fallback: false })}
      >
        trigger-route-hint
      </button>
    </div>
  ),
}));

// Captures the callback TerminalApp registers for a coordination-spawned tab
// (the scheduler dispatching a queued task-board card spawns one of these),
// so a test can fire it directly instead of routing a real Tauri event
// through the generic `listen` mock above (which many other listeners in
// this file also share, making it impractical to isolate just this one).
let capturedCoordinationSpawn:
  | ((payload: { session_id: string; command: string | null }) => void)
  | null = null;
// Set by a test that wants cleanup's unlisten to blow up the way Tauri's
// really does (see `unlistenOnCleanup`); reset in beforeEach.
let coordinationUnlisten: () => void = () => {};

vi.mock("../ipc/mcpToolServer", () => ({
  onCoordinationTabSpawned: vi.fn((cb: (payload: { session_id: string; command: string | null }) => void) => {
    capturedCoordinationSpawn = cb;
    return Promise.resolve(() => coordinationUnlisten());
  }),
}));

// Same capture trick for the enterprise events — they share the generic
// `listen` mock above otherwise, which several other listeners also use.
let capturedTaskReady: ((payload: TaskReadyPayload) => void) | null = null;
vi.mock("../ipc/enterprise", () => ({
  onEnterpriseTaskReceived: vi.fn(() => Promise.resolve(() => {})),
  onEnterpriseTaskReady: vi.fn((cb: (payload: TaskReadyPayload) => void) => {
    capturedTaskReady = cb;
    return Promise.resolve(() => {});
  }),
  onEnterpriseSkillInstalled: vi.fn(() => Promise.resolve(() => {})),
  enterpriseAcceptTask: vi.fn(),
  enterpriseRejectTask: vi.fn(),
}));

// TaskBoardView's own ipc — mocked so the board mounts cleanly (no real invoke).
vi.mock("../ipc/tasks", () => ({
  listTasks: vi.fn().mockResolvedValue([]),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn(),
  markTaskDone: vi.fn(),
  stopTask: vi.fn(),
  deleteTask: vi.fn(),
  readTranscript: vi.fn().mockResolvedValue(""),
  saveTranscript: vi.fn(),
  createTask: vi.fn(),
  updateTask: vi.fn(),
  cloneTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
  getTaskBoardConfig: vi.fn().mockResolvedValue({
    max_concurrent: 2,
    claude_command: "claude",
    auto_close_finished_tabs: true,
    notify_desktop_on_finish: true,
    notify_telegram_on_finish: true,
    stuck_timeout_secs: 1200,
  }),
  setTaskBoardConfig: vi.fn(),
}));

// The board is per-project now: TaskBoardView opens on the project overview and
// only renders the four columns once a project is opened. One project here, so
// each test below can click into it — without this the overview would hit the
// real `invoke` (the global mock never resolves, so the list would hang empty).
vi.mock("../ipc/projects", () => ({
  listProjects: vi.fn().mockResolvedValue([
    {
      id: "p1",
      name: "demo-project",
      description: "",
      path: "/p/demo",
      status: "ok",
      counts: { planning: 0, queued: 0, running: 0, done: 0 },
      error: null,
    },
  ]),
  removeProject: vi.fn(),
  openProject: vi.fn(),
  createProject: vi.fn(),
  usedDirs: vi.fn().mockResolvedValue([]),
}));

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import type { TaskReadyPayload } from "../ipc/enterprise";

beforeEach(() => {
  fakeLoop.isRunning = false;
  localStorage.clear();
  coordinationUnlisten = () => {};
});

/** Counts *tabs* with this title, not every node that happens to contain the
 *  text — the active tab echoes its title elsewhere in the shell, so a plain
 *  getAllByText over-counts and would pass even with the bug present. */
function tabTitles(title: string): Element[] {
  return Array.from(document.querySelectorAll(".aiterm-tab-title")).filter(
    (el) => el.textContent === title
  );
}

function taskReady(overrides: Partial<TaskReadyPayload> = {}): TaskReadyPayload {
  return {
    task_id: "task-1",
    title: "Ship it",
    description: "do the thing",
    spec_content: "",
    repo_dir: "/repo",
    work_branch: "work",
    max_steps: 5,
    on_complete: "pr",
    ...overrides,
  } as TaskReadyPayload;
}

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>
  );
}

/** Opens the Task Board view slot and drills into the one mocked project, so
 * the four columns are on screen — the board itself, not just the project
 * overview the slot now opens on. */
async function openBoard(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("button", { name: /工作看板|Task Board/ }));
  await user.click(await screen.findByText("demo-project"));
  expect(await screen.findByText(/計畫中|Planned/)).toBeInTheDocument();
}

describe("TerminalApp: Task Board view slot (Task 4)", () => {
  it("clicking the sidebar 工作看板 button shows the board and hides Home", async () => {
    renderApp();
    const user = userEvent.setup();
    await openBoard(user);
    // HomeView (stubbed) unmounts once homeActive flips false.
    expect(screen.queryByText("select-first-tab")).not.toBeInTheDocument();
  });

  // Regression test for a real bug: dragging a card to 待執行 dispatches it
  // (the scheduler spawns a `claude` session and TerminalApp adopts it as a
  // new tab via this same event), and the pre-existing "switch focus to a
  // newly agent-spawned tab" heuristic only checked whether the
  // *background* terminal tab was busy — it had no idea the user might be
  // looking at the Task Board overlay, and yanked them away from it the
  // instant a card got dispatched.
  it("does not switch away from the task board when a coordination tab spawns", async () => {
    renderApp();
    const user = userEvent.setup();
    await openBoard(user);

    expect(capturedCoordinationSpawn).not.toBeNull();
    capturedCoordinationSpawn!({ session_id: "spawned-session", command: "claude" });

    // Still on the board — not yanked to the newly spawned terminal tab.
    expect(await screen.findByText(/計畫中|Planned/)).toBeInTheDocument();
  });

  // Regression test for a real bug caught on video: dispatching ONE card put
  // TWO identical "Agent: ..." tabs in the sidebar. The backend had spawned a
  // single session (one worktree, one `claude`), so both tabs were views of
  // the *same* PTY session — the handler had simply run twice, because a
  // stale listener survived a failed `unlisten()` (Tauri's unlisten rejects
  // with "listeners[eventId].handlerId is undefined" and the old listener
  // stays registered; the dev log had 17 of those).
  //
  // One PTY session is one tab, no matter how many listeners fire.
  it("adopts a session only once even if the spawn event is handled twice", async () => {
    renderApp();
    const user = userEvent.setup();
    await openBoard(user);

    expect(capturedCoordinationSpawn).not.toBeNull();
    capturedCoordinationSpawn!({ session_id: "dup-session", command: "claude" });
    capturedCoordinationSpawn!({ session_id: "dup-session", command: "claude" });

    await waitFor(() => expect(screen.getAllByText("Agent: claude")).toHaveLength(1));
  });

  // Same duplicate-listener exposure as the coordination spawn above: this
  // handler also creates a tab, so a doubled listener means two tabs for one
  // enterprise task.
  it("adopts an enterprise task only once even if the ready event is handled twice", async () => {
    renderApp();
    const user = userEvent.setup();
    await openBoard(user);

    expect(capturedTaskReady).not.toBeNull();
    capturedTaskReady!(taskReady());
    capturedTaskReady!(taskReady());

    await waitFor(() => expect(tabTitles("⚙ Ship it")).toHaveLength(1));
  });

  // Tauri's `_unlisten` calls the webview's `unregisterListener` FIRST and
  // only then invokes the backend, so when that throws (it does — the dev log
  // had 17 of them) the rejection escaped as an unhandled promise rejection
  // with no clue which event it came from. `unlistenOnCleanup` exists for
  // exactly this; TerminalApp just wasn't using it.
  it("does not leak an unhandled rejection when cleanup's unlisten throws", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    coordinationUnlisten = () => { throw new TypeError("listeners[eventId] is undefined"); };

    const { unmount } = renderApp();
    await screen.findByText("select-first-tab");
    unmount();

    await waitFor(() =>
      expect(warn.mock.calls.some((c) => String(c[0]).includes("mcp-coordination-tab-spawned"))).toBe(true)
    );
    warn.mockRestore();
  });
});

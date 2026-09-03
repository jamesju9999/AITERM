import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
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

// TaskBoardView's own ipc — mocked so the board mounts cleanly (no real invoke).
vi.mock("../ipc/tasks", () => ({
  listTasks: vi.fn().mockResolvedValue([]),
  onTasksUpdated: vi.fn().mockResolvedValue(() => {}),
  moveTask: vi.fn(),
  stopTask: vi.fn(),
  deleteTask: vi.fn(),
  readTranscript: vi.fn().mockResolvedValue(""),
  createTask: vi.fn(),
  updateTask: vi.fn(),
  cloneTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
  getTaskBoardConfig: vi.fn().mockResolvedValue({ max_concurrent: 2, claude_command: "claude" }),
  setTaskBoardConfig: vi.fn(),
}));

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";

beforeEach(() => {
  fakeLoop.isRunning = false;
  localStorage.clear();
});

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>
  );
}

describe("TerminalApp: Task Board view slot (Task 4)", () => {
  it("clicking the sidebar 工作看板 button shows the board and hides Home", async () => {
    renderApp();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /工作看板|Task Board/ }));
    expect(await screen.findByText(/計畫中|Planned/)).toBeInTheDocument();
    // HomeView (stubbed) unmounts once homeActive flips false.
    expect(screen.queryByText("select-first-tab")).not.toBeInTheDocument();
  });
});

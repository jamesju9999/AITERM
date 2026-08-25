import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
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

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import { SESSION_TABS_KEY } from "../lib/sessionTabs";

beforeEach(() => {
  fakeLoop.isRunning = false;
  fakeLoop.stop.mockClear();
  localStorage.clear();
  // Seed the sole starting tab as a LoopStudio tab. Why LoopStudio and not
  // the default terminal tab: at the point these Task 3 changes land,
  // TerminalView has not yet been wired to registerCloseGuard/
  // unregisterCloseGuard (that's Task 4 — a separate commit after this one).
  // LoopStudioView is the one tab type that already had a close guard wired
  // before either task, so it's the only way to exercise "handleRouteHintPick
  // awaits a real, denying guard" against this commit's code, independent of
  // Task 4 landing.
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify([{ title: "Loop Studio", type: "loop-studio" }]));
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

/** Drives TerminalApp to the point where RouteHint is visible on the sole
 *  starting LoopStudio tab, then picks "database" from its switch dropdown —
 *  the same UI action a user takes to say "no, I wanted a database tab". */
async function pickDatabaseFromRouteHint() {
  renderApp();
  // HomeView (stubbed) unmounts once homeActive flips false, so fire the
  // route first while it's still on screen, then switch to the tab —
  // routeHint itself doesn't depend on order (it's keyed by tabId, not by
  // homeActive), and RouteHint only renders once both are true.
  await userEvent.click(screen.getByText("trigger-route-hint"));
  await userEvent.click(screen.getByText("select-first-tab"));

  const select = await screen.findByRole("combobox");
  await userEvent.selectOptions(select, "database");
}

describe("TerminalApp: RouteHint pick awaits the close guard (Task 3)", () => {
  it("guard 放行（LoopStudio 閒置）：舊分頁換成使用者選的新分頁類型", async () => {
    // fakeLoop.isRunning stays false (set in beforeEach): the close guard
    // resolves true immediately, same as a genuinely idle LoopStudio tab.
    await pickDatabaseFromRouteHint();

    // The tab bar should now show 2 tabs: the auto-recreated terminal
    // (closing the sole tab hits the "last tab" recreation branch in
    // handleCloseTab) plus the newly picked database tab. TabBar doesn't use
    // role="tab"; each real tab (excluding the Home button, Add-Tab button,
    // and Settings footer entry) carries this title.
    await screen.findAllByTitle(/Switch to Tab/);
    expect(screen.getAllByTitle(/Switch to Tab/).length).toBe(2);
  });

  it("guard 否決（LoopStudio 執行中，使用者按取消）：不開新分頁，維持原分頁", async () => {
    fakeLoop.isRunning = true;
    renderApp();
    await userEvent.click(screen.getByText("trigger-route-hint"));
    await userEvent.click(screen.getByText("select-first-tab"));

    const select = await screen.findByRole("combobox");
    // fire the pick — this awaits handleCloseTab, which awaits the guard,
    // which shows LoopStudioView's confirm dialog and stays pending.
    await act(async () => {
      await userEvent.selectOptions(select, "database");
    });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Loop 正在執行中");
    expect(screen.getAllByTitle(/Switch to Tab/).length).toBe(1);

    await userEvent.click(screen.getByRole("button", { name: "取消（繼續編輯）" }));

    // Cancelled: still exactly one tab, and the loop was never told to stop
    // (proof the cancel path really short-circuited handleRouteHintPick
    // before it reached handlePickerSelect).
    expect(screen.getAllByTitle(/Switch to Tab/).length).toBe(1);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
    expect(fakeLoop.stop).not.toHaveBeenCalled();
  });
});

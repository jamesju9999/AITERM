import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import { useEffect } from "react";
import { MemoryRouter } from "react-router-dom";

// Same mount-probe rationale as TerminalApp.routeHintCloseGuard.test.tsx:
// TerminalApp mounts fully in jsdom with these low-level Tauri entry points
// mocked; invoke() never resolves, which is fine since nothing here waits on it.
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
vi.mock("@tauri-apps/plugin-notification", () => ({ sendNotification: vi.fn() }));

// Stub LoopStudioView the same way TerminalApp.routeHintCloseGuard.test.tsx
// stubs HomeView: gives full synchronous control over the registered close
// guard (instead of depending on useOrchestratorLoop's async state), and
// exposes the tabId TerminalApp assigned it (a crypto.randomUUID() the test
// can't predict) so we can target it with the exact same aiterm:close-tab
// CustomEvent useAutoCloseFinishedTabs dispatches.
let guardResolvesTo: boolean | "pending" = true;
vi.mock("./LoopStudio", () => ({
  LoopStudioView: ({
    tabId,
    registerCloseGuard,
    unregisterCloseGuard,
  }: {
    tabId: string;
    registerCloseGuard?: (id: string, guard: () => Promise<boolean>) => void;
    unregisterCloseGuard?: (id: string) => void;
  }) => {
    useEffect(() => {
      registerCloseGuard?.(tabId, () =>
        guardResolvesTo === "pending" ? new Promise<boolean>(() => {}) : Promise.resolve(guardResolvesTo),
      );
      return () => unregisterCloseGuard?.(tabId);
    }, [tabId, registerCloseGuard, unregisterCloseGuard]);
    return <div data-testid="loop-tab-id">{tabId}</div>;
  },
}));

// Closing the sole tab hits handleCloseTab's "last tab" branch, which
// recreates a fresh terminal tab — that mounts a real TerminalView, so it
// needs the same hook mocks + browser API polyfills
// TerminalApp.routeHintCloseGuard.test.tsx uses for the same reason.
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

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import { SESSION_TABS_KEY } from "../lib/sessionTabs";

beforeEach(() => {
  guardResolvesTo = true;
  localStorage.clear();
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify([{ title: "Loop Studio", type: "loop-studio" }]));
});

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

describe("TerminalApp: aiterm:close-tab respects skipGuard", () => {
  it("skipGuard: true — 就算 guard 會一直卡著（pending）也照樣立刻關閉", async () => {
    guardResolvesTo = "pending";
    renderApp();
    const tabIdEl = await screen.findByTestId("loop-tab-id");
    const id = tabIdEl.textContent!;

    await act(async () => {
      window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: id, skipGuard: true } }));
      await Promise.resolve();
    });

    expect(screen.queryByTestId("loop-tab-id")).not.toBeInTheDocument();
  });

  it("沒有 skipGuard 時，guard 卡著就不會關（既有行為不變）", async () => {
    guardResolvesTo = "pending";
    renderApp();
    const tabIdEl = await screen.findByTestId("loop-tab-id");
    const id = tabIdEl.textContent!;

    await act(async () => {
      window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: id } }));
      await Promise.resolve();
    });

    expect(screen.getByTestId("loop-tab-id")).toBeInTheDocument();
  });

  it("沒有 skipGuard、guard 放行（閒置）時，跟原本一樣正常關閉", async () => {
    guardResolvesTo = true;
    renderApp();
    const tabIdEl = await screen.findByTestId("loop-tab-id");
    const id = tabIdEl.textContent!;

    await act(async () => {
      window.dispatchEvent(new CustomEvent("aiterm:close-tab", { detail: { tabId: id } }));
      await Promise.resolve();
    });

    expect(screen.queryByTestId("loop-tab-id")).not.toBeInTheDocument();
  });
});

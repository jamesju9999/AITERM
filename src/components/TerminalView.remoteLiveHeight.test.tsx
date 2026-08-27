import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.idleSignal.test.tsx (verified there to
// mount TerminalView fully in jsdom).
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    return new Promise(() => {});
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
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

// Captures every call TerminalView makes into useTerminalBlocks, so the test
// can grab the 9th positional arg (onUntrackedCommandBoundary) and invoke it
// directly — exactly like useTerminalBlocks itself would when it sees an
// OSC 133 C/D pair with no corresponding local block (a remote-viewer-issued
// command).
const useTerminalBlocksCalls: unknown[][] = [];
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    return {
      blocks: [],
      isAlternateBuffer: false,
      submitCommand: vi.fn(),
      beginTrackedBlock: vi.fn(),
      appendOutput: vi.fn(),
      setBlockGitInfo: vi.fn(),
      finalizeBlock: vi.fn(),
      termInstance: null,
    };
  },
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

beforeEach(() => {
  useTerminalBlocksCalls.length = 0;
});

describe("TerminalView 遠端指令觸發的即時窗格高度", () => {
  it("untracked 指令結束後不會縮回 MIN_LIVE_ROWS，避免已經畫出的輸出被裁切消失", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));
    const onUntrackedCommandBoundary = useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1][8] as (
      kind: "start" | "end",
    ) => void;
    expect(typeof onUntrackedCommandBoundary).toBe("function");

    const liveFrame = () => container.querySelector(".aiterm-live-frame") as HTMLElement;
    const idleHeight = liveFrame().style.height;

    act(() => onUntrackedCommandBoundary("start"));
    const expandedHeight = liveFrame().style.height;
    expect(expandedHeight).not.toBe(idleHeight);

    act(() => onUntrackedCommandBoundary("end"));
    // This is the bug: a remote-issued command never becomes a local block,
    // so it never gets promoted into a card the way a locally-submitted
    // command's output does. Shrinking back down here just clips already
    // rendered output out of view with no way to scroll back to it — it
    // must stay expanded.
    expect(liveFrame().style.height).toBe(expandedHeight);
  });
});

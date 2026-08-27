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
  it("untracked 指令有多行輸出時，結束後高度剛好放得下這些行——不會被裁掉，也不會卡在 MAX 留一大截空白", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const lastCall = () => useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1];
    await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));
    await waitFor(() => expect(lastCall()[1]).toBeTruthy());

    const term = lastCall()[1] as {
      write: (data: string, cb?: () => void) => void;
      buffer: { active: { cursorY: number } };
    };
    const onUntrackedCommandBoundary = lastCall()[8] as (kind: "start" | "end") => void;
    expect(typeof onUntrackedCommandBoundary).toBe("function");

    const liveFrame = () => container.querySelector(".aiterm-live-frame") as HTMLElement;
    const idleHeight = liveFrame().style.height;

    act(() => onUntrackedCommandBoundary("start"));
    const expandedHeight = liveFrame().style.height;
    expect(expandedHeight).not.toBe(idleHeight);

    // 5 行輸出，游標最後停在第 5 行（index 4）——遠比 MAX_LIVE_ROWS(16) 短，
    // 也遠比 MIN_LIVE_ROWS(3) 長。
    await act(async () => {
      await new Promise<void>((resolve) => term.write("l1\r\nl2\r\nl3\r\nl4\r\nl5", resolve));
    });

    act(() => onUntrackedCommandBoundary("end"));
    const settledHeight = liveFrame().style.height;
    // 這是先前那版修復的 bug：只在 "start" 撐高、"end" 完全不處理，導致
    // 窗格永遠卡在 MAX_LIVE_ROWS，在只有 5 行輸出時，畫面下方會留一大截
    // 用不到的空白，跟原本終端機的樣子不一致（實機截圖證實）。
    expect(settledHeight).not.toBe(expandedHeight);
    // 也不能收回到閒置的 MIN_LIVE_ROWS——那樣就退回上一版的 bug：已經畫出
    // 來的 5 行輸出被裁掉只剩最上面幾行。
    expect(settledHeight).not.toBe(idleHeight);
  });
});

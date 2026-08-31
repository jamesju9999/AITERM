import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { useState } from "react";

// Same mocking setup as TerminalView.remoteLiveHeight.test.tsx (verified there
// to mount TerminalView fully in jsdom).
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

// Unlike TerminalView.remoteLiveHeight.test.tsx's static mock, `blocks` here
// is real React state so the test can simulate a command actually completing
// (visibleBlockCount changing from 0 to 1) and observe how the live-frame
// height reacts to that transition.
type MockBlock = { id: string; status: string; renderedLines?: unknown[] };
let pushBlocks: ((blocks: MockBlock[]) => void) | null = null;
const useTerminalBlocksCalls: unknown[][] = [];
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    const [blocks, setBlocks] = useState<MockBlock[]>([]);
    pushBlocks = setBlocks;
    return {
      blocks,
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
  pushBlocks = null;
});

async function renderAndExpand(platform: string) {
  const originalPlatform = navigator.platform;
  Object.defineProperty(navigator, "platform", { value: platform, configurable: true });

  const { container } = render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );

  const lastCall = () => useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1];
  await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));
  const onUntrackedCommandBoundary = lastCall()[8] as (kind: "start" | "end") => void;
  expect(typeof onUntrackedCommandBoundary).toBe("function");

  const liveFrame = () => container.querySelector(".aiterm-live-frame") as HTMLElement;

  // 用既有的「撐到 MAX」入口把 liveRows 從閒置值(MIN)撐開，才能觀察到
  // 下面 visibleBlockCount 一變是否把它收回去——兩者高度一樣的話就看不出
  // 差異，見 TerminalView.remoteLiveHeight.test.tsx 的同一手法。
  act(() => onUntrackedCommandBoundary("start"));
  const expandedHeight = liveFrame().style.height;

  return { liveFrame, expandedHeight, originalPlatform };
}

describe("TerminalView Windows 上 liveRows 不隨區塊完成立刻收縮", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("Windows：一個區塊變成 completed 後，即時窗格維持撐開的高度，不收回 MIN_LIVE_ROWS", async () => {
    const { liveFrame, expandedHeight, originalPlatform } = await renderAndExpand("Win32");
    try {
      // 實機截圖證實的 bug：這裡如果無條件收回 MIN_LIVE_ROWS，遲來的
      // 自訂 prompt（例如 oh-my-posh）真正印出提示字元時，區塊早就不是
      // running 中，窗格再也不會撐開，提示字元被裁掉看不到。
      act(() => {
        pushBlocks!([
          { id: "1", status: "completed", renderedLines: [{ spans: [{ text: "l1" }] }] },
        ]);
      });

      expect(liveFrame().style.height).toBe(expandedHeight);
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("非 Windows：一個區塊變成 completed 後，即時窗格照舊收回 MIN_LIVE_ROWS（沒有回歸）", async () => {
    const { liveFrame, expandedHeight, originalPlatform } = await renderAndExpand("MacIntel");
    try {
      act(() => {
        pushBlocks!([
          { id: "1", status: "completed", renderedLines: [{ spans: [{ text: "l1" }] }] },
        ]);
      });

      expect(liveFrame().style.height).not.toBe(expandedHeight);
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

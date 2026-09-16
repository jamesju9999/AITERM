import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { Terminal } from "@xterm/xterm";

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

async function renderOn(platform: string) {
  const originalPlatform = navigator.platform;
  Object.defineProperty(navigator, "platform", { value: platform, configurable: true });

  const { container } = render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );
  await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));

  const lastCall = () => useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1];
  const host = () => container.querySelector(".aiterm-terminal-root") as HTMLElement;
  // 10th positional arg — the OSC 133 B callback that reports the prompt's
  // absolute buffer row.
  const onPromptStart = lastCall()[9] as (absoluteRow: number) => void;
  // 2nd positional arg — the real xterm.js Terminal instance TerminalView
  // passes into the hook. Needed to spy on scrollToLine: this jsdom
  // environment never calls .open() on a real DOM element, so xterm's
  // scroll methods are no-ops here (verified directly) — the only thing a
  // test in this file can check is that the code *asks* it to scroll to
  // the right place, not that the viewport actually moved.
  const term = lastCall()[1] as Terminal;
  return { host, onPromptStart, originalPlatform, term };
}

describe("Windows 即時窗格對齊到提示字元那一行", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("剛開分頁、提示字元在第 0 列時不用捲動", async () => {
    // 實機回報的回歸：先前的置底對齊假設「提示字元永遠在最後一列」，但那
    // 只有畫面填滿之後才成立。剛開啟的分頁提示字元在最上面、下方全是空行，
    // 錨底於是露出一整片空白，使用者完全看不到提示字元。
    const { host, onPromptStart, term, originalPlatform } = await renderOn("Win32");
    try {
      const scrollSpy = vi.spyOn(term, "scrollToLine");
      act(() => onPromptStart(0));
      expect(scrollSpy).toHaveBeenCalledWith(0);
      // 不再靠 CSS 位移對齊——真正的對齊靠 scrollToLine 改變 xterm 內部
      // 的捲動位置，host 本身永遠不設 top/position。
      expect(host().style.position).not.toBe("absolute");
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("畫面填滿、提示字元落在較下方時，直接把 viewport 捲到那一行", async () => {
    // 舊版做法是算「現在捲到哪」跟「提示字元在哪」的差距，用 CSS 位移
    // DOM 元素——這在連線夠久、scrollback 夠深時會失效（見
    // TerminalView.windowsRunningResetOffset.test.tsx 的完整說明：`claude`
    // CLI 這類印一次就停下來等按鍵的程式，差距永遠收斂不到 0，窗格被夾到
    // 只剩一兩列，實機錄影證實幾乎全黑）。改用 term.scrollToLine() 直接
    // 指定 viewport 位置，不用再算差距，也不再需要任何 CSS 位移。
    const { host, onPromptStart, term, originalPlatform } = await renderOn("Win32");
    try {
      const scrollSpy = vi.spyOn(term, "scrollToLine");
      act(() => onPromptStart(5));
      expect(scrollSpy).toHaveBeenCalledWith(5);
      expect(host().style.position).not.toBe("absolute");
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("非 Windows：緩衝區仍會被清空、提示字元本來就在第 0 列，不需要呼叫 scrollToLine", async () => {
    const { host, onPromptStart, term, originalPlatform } = await renderOn("MacIntel");
    try {
      const scrollSpy = vi.spyOn(term, "scrollToLine");
      act(() => onPromptStart(5));
      expect(scrollSpy).not.toHaveBeenCalled();
      expect(host().style.position).not.toBe("absolute");
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

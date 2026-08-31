import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

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
  return { host, onPromptStart, originalPlatform };
}

describe("Windows 即時窗格對齊到提示字元那一行", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("剛開分頁、提示字元在第 0 列時不位移——固定錨底會讓整個窗格是空白的", async () => {
    // 實機回報的回歸：先前的置底對齊假設「提示字元永遠在最後一列」，但那
    // 只有畫面填滿之後才成立。剛開啟的分頁提示字元在最上面、下方全是空行，
    // 錨底於是露出一整片空白，使用者完全看不到提示字元。
    const { host, onPromptStart, originalPlatform } = await renderOn("Win32");
    try {
      act(() => onPromptStart(0));
      expect(host().style.position).not.toBe("absolute");
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("畫面填滿、提示字元落在較下方時，host 往上位移讓那一行成為第一個可見列", async () => {
    const { host, onPromptStart, originalPlatform } = await renderOn("Win32");
    try {
      act(() => onPromptStart(5));
      expect(host().style.position).toBe("absolute");
      // 往上位移，不是往下——負的 top 才會把上面幾列推出裁切範圍。
      const top = parseFloat(host().style.top);
      expect(top).toBeLessThan(0);
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("非 Windows：緩衝區仍會被清空、提示字元本來就在第 0 列，完全不位移", async () => {
    const { host, onPromptStart, originalPlatform } = await renderOn("MacIntel");
    try {
      act(() => onPromptStart(5));
      expect(host().style.position).not.toBe("absolute");
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

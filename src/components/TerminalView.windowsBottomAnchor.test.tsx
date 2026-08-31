import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
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

  const frame = container.querySelector(".aiterm-live-frame") as HTMLElement;
  const host = container.querySelector(".aiterm-terminal-root") as HTMLElement;
  return { frame, host, originalPlatform };
}

describe("Windows 即時窗格置底對齊", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("Windows：xterm host 錨定在窗格底部，露出的是畫面最下方幾行（提示字元所在處）", async () => {
    // Windows 上不再清空 xterm 緩衝區（見 useTerminalBlocks 的 OSC 133 D
    // 分支），所以 ConPTY 的整個畫面都還在：舊輸出在上、新提示字元在最
    // 下面那一列。窗格預設從頂端裁切，那樣露出的會是最舊的幾行、看不到
    // 提示字元；錨在底部才會露出提示字元與新輸出。
    const { frame, host, originalPlatform } = await renderOn("Win32");
    try {
      expect(frame.style.position).toBe("relative");
      expect(host.style.position).toBe("absolute");
      expect(host.style.bottom).toBe("0px");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("非 Windows：維持原本由頂端裁切的版面（緩衝區仍會被清空，不需要錨底）", async () => {
    const { host, originalPlatform } = await renderOn("MacIntel");
    try {
      expect(host.style.position).not.toBe("absolute");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

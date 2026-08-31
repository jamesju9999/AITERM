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

describe("TerminalView 區塊完成後的 liveRows 收縮", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("Windows：一個區塊變成 completed 後，即時窗格跟其他平台一樣收回 MIN_LIVE_ROWS", async () => {
    const { liveFrame, expandedHeight, originalPlatform } = await renderAndExpand("Win32");
    try {
      // 這裡原本斷言的是相反的行為（Windows 維持撐開、永不收縮），用來閃避
      // 「遲來的自訂 prompt（例如 oh-my-posh）印出提示字元時已經不是 running
      // 中，窗格不會再撐開，提示字元被裁掉」這個實機 bug。那個閃避已經不需要
      // 了，而且有害：Windows 現在不清空 xterm 緩衝區（見 useTerminalBlocks
      // 的 OSC 133 D 分支），整個 ConPTY 畫面都還在，窗格若維持撐開就會把
      // 舊輸出跟卡片一起顯示、變成重複內容。
      //
      // 之所以可以安全收縮，是因為窗格改成錨定底部（見
      // TerminalView.windowsBottomAnchor.test.tsx）：露出的永遠是畫面最下方
      // 幾行，而提示字元一定在最後一行——不管它多晚才印出來，都必定在視野
      // 內。當初那個 bug 的成因是從「頂端」裁切，現在的裁切方向相反。
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

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.windowsPromptAlign.test.tsx (verified
// there to mount TerminalView fully in jsdom).
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

let mockIsAlternateBuffer = false;
const useTerminalBlocksCalls: unknown[][] = [];
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    return {
      blocks: [],
      isAlternateBuffer: mockIsAlternateBuffer,
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
  mockIsAlternateBuffer = false;
});

async function mount() {
  const { container } = render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );
  await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));
  const host = container.querySelector(".aiterm-terminal-root") as HTMLElement;
  return { container, host };
}

function wheelOn(el: HTMLElement, deltaY: number) {
  const ev = new WheelEvent("wheel", { deltaY, bubbles: true, cancelable: true });
  el.dispatchEvent(ev);
  return ev;
}

describe("即時窗格的滑鼠捲動鎖定", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("一般狀態：滾輪不捲動 xterm 自己的 scrollback（會露出已經變成卡片的舊輸出）", async () => {
    // 不再清空緩衝區之後，xterm 保有完整的 ConPTY 畫面，往回捲就會看到
    // 上方卡片已經呈現過的同一段輸出。即時窗格的定位是「只顯示現在這一
    // 段」，捲動歷史請用卡片列表，所以這裡把滾輪攔下來。
    const { host } = await mount();
    const ev = wheelOn(host, 120);
    expect(ev.defaultPrevented).toBe(true);
  });

  it("滾輪改成捲動外層面板，不是整個卡住不動", async () => {
    const { container, host } = await mount();
    const scroller = container.querySelector('[data-aiterm-live-scroll-target="1"]') as HTMLElement;
    expect(scroller).toBeTruthy();
    scroller.scrollTop = 0;

    wheelOn(host, 120);

    expect(scroller.scrollTop).toBe(120);
  });

  it("全螢幕 TUI（alternate buffer）時放行——vim/htop 這類程式自己要用滾輪", async () => {
    mockIsAlternateBuffer = true;
    const { host } = await mount();
    const ev = wheelOn(host, 120);
    expect(ev.defaultPrevented).toBe(false);
  });
});

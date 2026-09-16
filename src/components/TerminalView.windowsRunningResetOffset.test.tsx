import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

// Same setup as TerminalView.idleSignal.test.tsx: needs a real pty://data
// listener captured so a chunk can be driven through onPtyData, exactly like
// the real Tauri event bridge would.
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    return new Promise(() => {});
  }),
}));

const listenHandlers = new Map<string, ((event: { payload: unknown }) => void)[]>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    const arr = listenHandlers.get(event) ?? [];
    arr.push(handler);
    listenHandlers.set(event, arr);
    return Promise.resolve(() => {
      const idx = arr.indexOf(handler);
      if (idx >= 0) arr.splice(idx, 1);
    });
  }),
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

// Same mocking pattern as TerminalView.remoteLiveHeight.test.tsx's
// mockIsRawKeyboardModeActive: a module-level mutable `blocks` array so a
// test can put a "running" block in place before driving a PTY chunk
// through, since the fix under test branches on
// `blocksRef.current[...]?.status === "running"`.
let mockBlocks: TerminalBlock[] = [];
const useTerminalBlocksCalls: unknown[][] = [];
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    return {
      blocks: mockBlocks,
      isAlternateBuffer: false,
      isRawKeyboardModeActive: false,
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
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
  useTerminalBlocksCalls.length = 0;
  mockBlocks = [];
});

describe("Windows 本機分頁：不管 scrollback 多深，指令執行中都能撐到 MAX_LIVE_ROWS", () => {
  it("提示字元在很深的絕對列時，指令執行中窗格仍然撐到完整的 MAX_LIVE_ROWS，不會被夾小", async () => {
    // 第三版（動態夾法）在長連線、scrollback 夠深時還是不夠：算「現在捲到
    // 哪」跟「提示字元在哪」的差距，假設指令會持續吐出新內容讓差距自然
    // 收斂到 0，但 claude CLI 的信任提示印一次就停下來等按鍵，差距永遠
    // 收斂不了，窗格被夾到只剩一兩列（實機錄影證實：連線開 20 分鐘後，
    // 畫面幾乎全黑）。改用 term.scrollToLine() 直接指定 viewport 位置，
    // 位移量恆為 0，liveRows 只需要被 term.rows 本身夾住。
    const originalPlatform = navigator.platform;
    Object.defineProperty(navigator, "platform", { value: "Win32", configurable: true });
    try {
      const buildTree = () => (
        <LocaleProvider>
          <MemoryRouter>
            <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
          </MemoryRouter>
        </LocaleProvider>
      );
      const { container, rerender } = render(buildTree());

      const dataEvent = ptyDataEvent("test-session");
      await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
      const handlers = listenHandlers.get(dataEvent)!;
      await waitFor(() => expect(useTerminalBlocksCalls.length).toBeGreaterThan(0));

      const host = () => container.querySelector(".aiterm-terminal-root") as HTMLElement;

      // useTerminalBlocks 被整個 mock 掉，不會真的解析 OSC 133，所以不能
      // 靠餵 B 序列觸發 onPromptStart——直接呼叫 TerminalView 傳給 hook 的
      // 第 10 個參數（onPromptStart 本人），跟
      // TerminalView.windowsPromptAlign.test.tsx 同一個手法。用一個很大的
      // 絕對列數（5000）代表連線累積了非常深的 scrollback 之後才畫出的
      // 提示字元。
      const lastCall = () => useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1];
      const onPromptStart = lastCall()[9] as (absoluteRow: number) => void;
      expect(typeof onPromptStart).toBe("function");
      act(() => {
        onPromptStart(5000);
      });

      // 不再靠 CSS 位移對齊，host 永遠不設 top（真正的對齊靠
      // term.scrollToLine() 改變 xterm 內部的捲動位置，這個 jsdom 環境下
      // 的無頭 Terminal 沒有掛載真正的 DOM renderer，量不到那個效果，只
      // 能驗證這裡不再依賴 CSS 位移）。
      await waitFor(() => {
        expect(host().style.top).toBe("");
      });

      // 使用者送出 `claude`，建立一個 running 中的區塊。單純改
      // mockBlocks 這個模組層級變數不會自動觸發重新渲染——mock 的回傳值
      // 只有在 React 真的重新呼叫這個 hook 時才會更新，所以要手動
      // rerender 一次，讓 blocksRef（跟著 `blocks` 用 effect 同步）真的
      // 讀到新值。
      mockBlocks = [
        {
          id: "b1",
          command: "claude",
          status: "running",
          startTime: Date.now(),
          rawOutput: "",
        },
      ];
      act(() => {
        rerender(buildTree());
      });

      act(() => {
        handlers.forEach((h) => h({ payload: { base64: btoa("some interactive output\r\n") } }));
      });

      // 撐到完整的 MAX_LIVE_ROWS(16)：round(16 * 14 * 1.1) = 246px——不是
      // 被位移量夾小的值，不管提示字元在多深的絕對列都一樣。
      const liveFrame = () => container.querySelector(".aiterm-live-frame") as HTMLElement;
      await waitFor(() => {
        expect(liveFrame().style.height).toBe(`${Math.round(16 * 14 * 1.1)}px`);
      });
      expect(host().style.top).toBe("");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

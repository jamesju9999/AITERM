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

describe("Windows 本機分頁：指令執行中歸零提示字元位移", () => {
  it("指令執行中收到 PTY 輸出時，不管上一次提示字元位移多大都歸零，不追著舊位置跑到畫面外", async () => {
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
      // 絕對列數（500）代表連線累積了很深的 scrollback 之後才畫出的提示
      // 字元——不需要真的餵那麼多行內容，這裡只在意「位移量一開始確實是
      // 正的、有效的」這件事,不是位移量算出來的精確數字。
      const lastCall = () => useTerminalBlocksCalls[useTerminalBlocksCalls.length - 1];
      const onPromptStart = lastCall()[9] as (absoluteRow: number) => void;
      expect(typeof onPromptStart).toBe("function");
      act(() => {
        onPromptStart(500);
      });

      await waitFor(() => {
        expect(host().style.position).toBe("absolute");
      });
      const idleTop = parseFloat(host().style.top);
      expect(idleTop).toBeLessThan(0);

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

      // 前景程式輸出，只把畫面再往前推進一點點（遠小於先前累積的深度）。
      act(() => {
        handlers.forEach((h) => h({ payload: { base64: btoa("some interactive output\r\n") } }));
      });

      await waitFor(() => {
        expect(host().style.top).toBe("");
      });
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

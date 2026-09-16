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

describe("Windows 本機分頁：指令執行中窗格能撐多高由當下位移量動態夾住", () => {
  it("位移量遠大於主控端列數時，窗格想撐到 MAX_LIVE_ROWS 也會被夾到剩下的空間，不會超過列數", async () => {
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
      const idleTopStyle = host().style.top;
      expect(parseFloat(idleTopStyle)).toBeLessThan(0);

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

      // 區塊剛變成 running、還沒有任何輸出回來這一刻，位移量不該被提早
      // 歸零或重算——這正是「指令執行中直接歸零位移」那個修法造成重複
      // 顯示舊內容的那一刻：舊內容還在畫面上，位移量被錯誤地丟掉，暴露
      // 出 Windows 從不清空緩衝區留下的、已經變成卡片的輸出。
      expect(host().style.top).toBe(idleTopStyle);

      // 前景程式輸出，只把畫面再往前推進一點點（遠小於先前累積的深度—— 一
      // 行還不夠讓 24 列的畫面往下捲動，viewportY 實際上仍是 0）。
      act(() => {
        handlers.forEach((h) => h({ payload: { base64: btoa("some interactive output\r\n") } }));
      });

      const liveFrame = () => container.querySelector(".aiterm-live-frame") as HTMLElement;
      await waitFor(() => {
        // 位移量沒有真的縮小（promptAbsRow=500 遠大於任何一行輸出能推進的
        // 量），所以維持跟閒置時一樣：不會像「直接歸零」那個舊修法一樣
        // 瞬間跳去顯示目前捲動位置最上面幾列，那樣會把已經變成卡片的上一
        // 個指令輸出重複顯示一次（另一次實機回報）。
        expect(host().style.top).toBe(idleTopStyle);
      });
      // 窗格「想要」撐到 MAX_LIVE_ROWS(16)，但 term.rows(24) 扣掉位移量
      // (23) 只剩 1 列空間，夾到 MIN_LIVE_ROWS(3) 的下限——不是撐好撐滿的
      // 16 列（round(16 * 14 * 1.1) = 246px，加上位移會遠遠超過 24 列）。
      // jsdom 沒有真正的字元格尺寸，走 14*1.1 的 fallback，跟
      // TerminalView.windowsPromptAlign.test.tsx 同一個假設。
      expect(liveFrame().style.height).toBe(`${Math.round(3 * 14 * 1.1)}px`);
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });
});

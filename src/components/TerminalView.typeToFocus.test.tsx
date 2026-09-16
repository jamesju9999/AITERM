import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, fireEvent, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// Same mocking setup as TerminalView.remoteBurstOutput.test.tsx — real
// useTerminalBlocks, real @xterm/xterm.
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
  listenHandlers.clear();
});

describe("點選卡片列表的文字之後，開始打字會自動把焦點轉回指令輸入框", () => {
  it("焦點掉回 document.body（選完文字的典型狀態）時，按下可列印字元會聚焦 WarpInput 並讓這個字元正常輸入", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const textarea = await waitFor(() => {
      const el = container.querySelector(".warp-input-textarea") as HTMLTextAreaElement | null;
      expect(el).not.toBeNull();
      return el!;
    });

    // 模擬「選完卡片裡的文字之後，焦點掉回 body」——不是點在 WarpInput 上。
    act(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    expect(document.activeElement).not.toBe(textarea);

    act(() => {
      fireEvent.keyDown(window, { key: "l" });
    });

    expect(document.activeElement).toBe(textarea);
  });

  it("已經有輸入框（例如搜尋列）持有焦點時，不搶走焦點", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const warpTextarea = await waitFor(() => {
      const el = container.querySelector(".warp-input-textarea") as HTMLTextAreaElement | null;
      expect(el).not.toBeNull();
      return el!;
    });

    // 用一個真實的、目前持有焦點的 input 取代「另一個輸入框」的角色。
    const decoy = document.createElement("input");
    document.body.appendChild(decoy);
    act(() => {
      decoy.focus();
    });
    expect(document.activeElement).toBe(decoy);

    act(() => {
      fireEvent.keyDown(window, { key: "l" });
    });

    // 焦點應該留在 decoy 身上，不會被搶到 WarpInput。
    expect(document.activeElement).toBe(decoy);
    expect(document.activeElement).not.toBe(warpTextarea);
    document.body.removeChild(decoy);
  });

  it("Cmd/Ctrl 組合鍵（例如 Cmd+C 複製）不會觸發自動聚焦", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const textarea = await waitFor(() => {
      const el = container.querySelector(".warp-input-textarea") as HTMLTextAreaElement | null;
      expect(el).not.toBeNull();
      return el!;
    });

    act(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    expect(document.activeElement).not.toBe(textarea);

    act(() => {
      fireEvent.keyDown(window, { key: "c", metaKey: true });
    });

    expect(document.activeElement).not.toBe(textarea);
  });
});

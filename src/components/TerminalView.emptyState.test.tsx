import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
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
import { ptyDataEvent } from "../ipc/events";

beforeEach(() => {
  listenHandlers.clear();
});

describe("終端機還沒跑過任何指令時顯示快捷提示的空狀態", () => {
  it("剛開好時顯示空狀態提示；跑完第一個指令、卡片出現後就消失", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );

    // 剛掛載、還沒有任何區塊時：空狀態提示要在，一般的卡片列表不能在。
    await waitFor(() => {
      expect(container.querySelector('[data-testid="block-list-empty"]')).toBeInTheDocument();
    });
    expect(container.querySelector(".aiterm-block-list")).not.toBeInTheDocument();

    const dataEvent = ptyDataEvent("test-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
    const handlers = listenHandlers.get(dataEvent)!;

    const send = (chunk: string) => {
      const bytes = new TextEncoder().encode(chunk);
      const base64 = btoa(String.fromCharCode(...bytes));
      act(() => {
        handlers.forEach((h) => h({ payload: { base64 } }));
      });
    };

    send("user@host:~$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07");
    send("file1\r\n");
    send("\x1b]133;D;0\x07");

    // 跑完第一個指令、卡片真的出現之後，空狀態提示要換成一般卡片列表。
    await waitFor(() => {
      expect(container.querySelector(".aiterm-block-list")).toBeInTheDocument();
    });
    expect(container.querySelector('[data-testid="block-list-empty"]')).not.toBeInTheDocument();
  });
});

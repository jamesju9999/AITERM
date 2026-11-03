import React from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// 接手後端已啟動的指令（派工分頁）：沒有 OSC 133 C 也要有區塊可畫。
// 其餘 mock 同 TerminalView.emptyState.test.tsx。
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

describe("接手後端已啟動指令的分頁（externalSessionId + externalCommand）", () => {
  it("沒收到任何 OSC 133 也補建 running 卡片，後來的輸出畫得進去", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView
            tabId="tab-1"
            externalSessionId="adopted-session"
            externalCommand="claude"
            registerCloseGuard={() => {}}
            unregisterCloseGuard={() => {}}
          />
        </MemoryRouter>
      </LocaleProvider>,
    );

    const dataEvent = ptyDataEvent("adopted-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
    const bytes = new TextEncoder().encode("CLAUDE-WELCOME\r\n");
    const base64 = btoa(String.fromCharCode(...bytes));
    act(() => {
      listenHandlers.get(dataEvent)!.forEach((h) => h({ payload: { base64 } }));
    });

    await waitFor(() => {
      expect(container.querySelector(".aiterm-block-list")).toBeInTheDocument();
    });
    expect(container.querySelector('[data-testid="block-list-empty"]')).not.toBeInTheDocument();
  });

  it("沒有 externalCommand 時維持空狀態（一般接手分頁不受影響）", async () => {
    const { container } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView tabId="tab-2" externalSessionId="adopted-2" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
        </MemoryRouter>
      </LocaleProvider>,
    );
    await waitFor(() => {
      expect(container.querySelector('[data-testid="block-list-empty"]')).toBeInTheDocument();
    });
  });

  // StrictMode 會把 effect 跑兩次：接手補建只能有一張卡，不能兩張。
  it("在 StrictMode 下也只補建一張卡", async () => {
    const { container } = render(
      <React.StrictMode>
        <LocaleProvider>
          <MemoryRouter>
            <TerminalView
              tabId="tab-3"
              externalSessionId="adopted-strict"
              externalCommand="claude"
              registerCloseGuard={() => {}}
              unregisterCloseGuard={() => {}}
            />
          </MemoryRouter>
        </LocaleProvider>
      </React.StrictMode>,
    );

    const dataEvent = ptyDataEvent("adopted-strict");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
    const base64 = btoa("CLAUDE-WELCOME\r\n");
    act(() => {
      listenHandlers.get(dataEvent)!.forEach((h) => h({ payload: { base64 } }));
    });

    await waitFor(() => expect(container.querySelector(".aiterm-block-card")).toBeInTheDocument());
    expect(container.querySelectorAll(".aiterm-block-card")).toHaveLength(1);
  });
});

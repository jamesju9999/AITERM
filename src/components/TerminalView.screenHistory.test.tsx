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

const historyHook = vi.fn();
const getHistory = vi.fn(() => ["較早的畫面"]);
vi.mock("./SuggestionSidebar/useScreenHistory", () => ({
  useScreenHistory: (...args: unknown[]) => { historyHook(...args); return { getHistory }; },
}));
let sidebarProps: Record<string, unknown> | null = null;
vi.mock("./SuggestionSidebar/SuggestionSidebar", () => ({
  SuggestionSidebar: (p: Record<string, unknown>) => { sidebarProps = p; return <div data-testid="stub-sidebar" />; },
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";

beforeEach(() => {
  listenHandlers.clear();
  localStorage.clear();
  historyHook.mockClear();
  sidebarProps = null;
});

const mount = () =>
  render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );

describe("TerminalView 的畫面歷史接線", () => {
  it("側欄還沒打開時就已經在記錄（hook 掛在 TerminalView，不是側欄）", async () => {
    const { container } = mount();
    await waitFor(() => expect(container.querySelector(".aiterm-sugg-toggle")).not.toBeNull());
    expect(container.querySelector('[data-testid="stub-sidebar"]')).toBeNull();
    await waitFor(() => {
      const sidArg = historyHook.mock.calls.map((c) => c[0]).find((s) => s === "test-session");
      expect(sidArg).toBe("test-session");
    });
    const last = historyHook.mock.calls[historyHook.mock.calls.length - 1];
    expect(last[1]).toBe(false); // 沒有 AI 工具在跑
    expect(typeof last[2]).toBe("function");
    expect((last[2] as () => number)()).toBeGreaterThanOrEqual(0);
  });

  it("開啟側欄後，把同一個 getHistory 交給側欄", async () => {
    const { container } = mount();
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(sidebarProps).not.toBeNull());
    expect(sidebarProps!.getHistory).toBe(getHistory);
  });
});

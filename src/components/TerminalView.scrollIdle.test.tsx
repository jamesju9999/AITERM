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
import { ptyDataEvent } from "../ipc/events";

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

describe("捲動造成的輸出不算 AI 在工作", () => {
  // Claude Code 這類全螢幕程式在 alternate buffer 裡，滾輪會被 xterm 轉成按鍵送進 PTY，
  // 程式重畫畫面的輸出不能讓側欄以為「AI 剛回完一輪」而自動重跑建議。
  async function openSidebar() {
    const r = mount();
    const toggle = () => r.container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(sidebarProps).not.toBeNull());
    const dataEvent = ptyDataEvent("test-session");
    await waitFor(() => expect(listenHandlers.has(dataEvent)).toBe(true));
    const emit = (s: string) => act(() => {
      const bytes = new TextEncoder().encode(s);
      listenHandlers.get(dataEvent)!.forEach((h) => h({ payload: { base64: btoa(String.fromCharCode(...bytes)) } }));
    });
    const host = r.container.querySelector(".aiterm-terminal-root") as HTMLElement;
    const wheel = () => act(() => { host.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, bubbles: true, cancelable: true })); });
    const idle = () => (sidebarProps!.getIdleMs as () => number)();
    const historyIdle = () => (historyHook.mock.calls[historyHook.mock.calls.length - 1][2] as () => number)();
    return { emit, wheel, idle, historyIdle };
  }

  it("output right after a wheel does not reset the idle clock (sidebar and screen history)", async () => {
    const { emit, wheel, idle, historyIdle } = await openSidebar();
    const now = vi.spyOn(Date, "now");
    now.mockReturnValue(1_000_000);
    emit("AI 回覆\r\n");
    now.mockReturnValue(1_010_000);
    expect(idle()).toBe(10_000);
    wheel();
    now.mockReturnValue(1_010_100);
    emit("重畫的畫面\r\n");
    now.mockReturnValue(1_012_000);
    expect(idle()).toBe(12_000);
    expect(historyIdle()).toBe(12_000);
    now.mockRestore();
  });

  it("ordinary output, and output well after the wheel stops, still counts", async () => {
    const { emit, wheel, idle } = await openSidebar();
    const now = vi.spyOn(Date, "now");
    now.mockReturnValue(2_000_000);
    emit("a\r\n");
    now.mockReturnValue(2_005_000);
    emit("b\r\n");
    expect(idle()).toBe(0);
    wheel();
    now.mockReturnValue(2_008_000);
    emit("AI 繼續輸出\r\n");
    expect(idle()).toBe(0);
    now.mockRestore();
  });
});

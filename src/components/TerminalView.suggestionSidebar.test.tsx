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

beforeEach(() => {
  listenHandlers.clear();
  localStorage.clear();
});

const mount = () =>
  render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );

describe("建議側欄與終端機並排", () => {
  it("開關按鈕開出的側欄是終端機的兄弟欄（並排），不是疊在終端機裡；再按一次關閉", async () => {
    const { container } = mount();
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    expect(toggle().getAttribute("aria-pressed")).toBe("false");
    expect(container.querySelector(".aiterm-sugg-sidebar")).toBeNull();

    await act(async () => { toggle().click(); });
    await waitFor(() => expect(container.querySelector(".aiterm-sugg-sidebar")).not.toBeNull());
    expect(toggle().getAttribute("aria-pressed")).toBe("true");

    const split = container.querySelector(".aiterm-sugg-split") as HTMLElement;
    const main = split.querySelector(".aiterm-sugg-split__main") as HTMLElement;
    const side = split.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    // 側欄與終端機主欄是同一個 flex 列的兩個直接子元素。
    expect(side.parentElement).toBe(split);
    expect(main.parentElement).toBe(split);
    // 側欄不在終端機主欄裡面，終端機也不在側欄裡面。
    expect(main.contains(side)).toBe(false);
    expect(side.contains(main)).toBe(false);
    expect(main.querySelector(".aiterm-block-list, [data-testid=\"block-list-empty\"]")).not.toBeNull();
    // 沒有偵測到 AI 工具時，也能手動開啟，並明講是手動。
    expect(side.textContent).toContain("手動開啟");

    await act(async () => { toggle().click(); });
    expect(container.querySelector(".aiterm-sugg-sidebar")).toBeNull();
  });

  it("Ask AI 面板仍然是終端機主欄裡的浮層，不會跑進側欄", async () => {
    const { container } = mount();
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(container.querySelector(".aiterm-sugg-sidebar")).not.toBeNull());
    const main = container.querySelector(".aiterm-sugg-split__main") as HTMLElement;
    const side = container.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    await waitFor(() => expect(container.querySelector(".aiterm-ai-panel")).not.toBeNull());
    const panel = container.querySelector(".aiterm-ai-panel") as HTMLElement;
    expect(main.contains(panel)).toBe(true);
    expect(side.contains(panel)).toBe(false);
  });

  it("大目標：帶入分頁已存的目標，儲存後回報給 TerminalApp", async () => {
    const onGoal = vi.fn();
    const { container, getByRole, getByText } = render(
      <LocaleProvider>
        <MemoryRouter>
          <TerminalView
            tabId="tab-1"
            registerCloseGuard={() => {}}
            unregisterCloseGuard={() => {}}
            initialSuggestionGoal="把舊系統轉成網頁版"
            onSuggestionGoalChange={onGoal}
          />
        </MemoryRouter>
      </LocaleProvider>,
    );
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(getByText("把舊系統轉成網頁版")).toBeInTheDocument());

    await act(async () => { getByRole("button", { name: "編輯大目標" }).click(); });
    const box = getByRole("textbox", { name: "大目標" }) as HTMLTextAreaElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
      setter.call(box, "改成雲端版");
      box.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => { getByRole("button", { name: "儲存" }).click(); });
    expect(onGoal).toHaveBeenCalledWith("改成雲端版");
    expect(getByText("改成雲端版")).toBeInTheDocument();
  });
});

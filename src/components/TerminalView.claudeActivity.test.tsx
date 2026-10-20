import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    if (cmd === "claude_activity_snapshot") return Promise.resolve({ nodes: [], main_tool: null, turn: 0 });
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
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver { observe() {} unobserve() {} disconnect() {} }
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

vi.mock("../hooks/useAgentMission", () => ({
  useAgentMission: () => ({ agentMission: null, startMission: vi.fn(), stopMission: vi.fn(), addTokens: vi.fn() }),
}));

import { TerminalView } from "./TerminalView";
import { LocaleProvider } from "../contexts/LocaleContext";

beforeEach(() => {
  listenHandlers.clear();
  localStorage.clear();
  localStorage.setItem("aiterm-claude-activity-collapsed", "0");
});

const mount = () =>
  render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" registerCloseGuard={() => {}} unregisterCloseGuard={() => {}} />
      </MemoryRouter>
    </LocaleProvider>,
  );

const emit = (payload: unknown) =>
  act(async () => {
    for (const h of listenHandlers.get("claude-activity://test-session") ?? []) h({ payload });
  });

const node = (id: string, description: string) => ({
  id, parent: null, agent_type: "general-purpose", description, status: "running",
  started_ms: 1, ended_ms: null, current_tool: "Bash: ls", summary: null, tokens: null, tool_count: null, turn: 1,
});
const snap = (...nodes: ReturnType<typeof node>[]) => ({ nodes, main_tool: null, turn: 1 });

describe("終端機分頁的 Claude 協作側欄", () => {
  it("沒有任何 subagent 事件時不出現側欄（版面與以前完全一樣）", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    expect(container.querySelector(".aiterm-cact")).toBeNull();
    expect(container.querySelector(".aiterm-sugg-split__side")).toBeNull();
  });

  it("收到事件後，側欄在終端機旁（兄弟欄）顯示 subagent", async () => {
    const { container, getByText } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit(snap(node("a1", "整理檔案")));
    await waitFor(() => expect(getByText("整理檔案")).toBeInTheDocument());
    const split = container.querySelector(".aiterm-sugg-split") as HTMLElement;
    const side = split.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    const main = split.querySelector(".aiterm-sugg-split__main") as HTMLElement;
    expect(side.querySelector(".aiterm-cact")).not.toBeNull();
    expect(main.contains(side)).toBe(false);
    expect(side.querySelector(".aiterm-cact--fill")).not.toBeNull();
  });

  it("與建議側欄同時開時上下並排，面板不再獨佔整欄", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit(snap(node("a1", "整理檔案")));
    const toggle = () => container.querySelector(".aiterm-sugg-toggle") as HTMLButtonElement;
    await waitFor(() => expect(toggle()).not.toBeNull());
    await act(async () => { toggle().click(); });
    await waitFor(() => expect(container.querySelector(".aiterm-sugg-sidebar")).not.toBeNull());
    const side = container.querySelector(".aiterm-sugg-split__side") as HTMLElement;
    expect(side.querySelector(".aiterm-cact")).not.toBeNull();
    expect(side.querySelector(".aiterm-cact--fill")).toBeNull();
    expect(side.querySelector(".aiterm-sugg-sidebar")).not.toBeNull();
  });

  it("設定關閉時，即使有 subagent 事件也不出現側欄", async () => {
    localStorage.setItem("aiterm-claude-activity-enabled", "0");
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit(snap(node("a1", "整理檔案")));
    expect(container.querySelector(".aiterm-cact")).toBeNull();
    expect(container.querySelector(".aiterm-sugg-split__side")).toBeNull();
  });
});

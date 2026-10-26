import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "pty_create") return Promise.resolve("test-session");
    if (cmd === "claude_activity_snapshot") return Promise.resolve({ nodes: [], main_tool: null, main_tool_count: 0, main_recent: [], tools: [], turn: 0 });
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
  started_ms: 1, ended_ms: null, current_tool: "Bash: ls", summary: null, tokens: null, tool_count: null, last_event_ms: 1_000, turn: 1,
});
const snap = (...nodes: ReturnType<typeof node>[]) => ({ nodes, main_tool: null, main_tool_count: 0, main_recent: [], tools: [], turn: 1 });

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

  it("只有主 Claude 在用工具、還沒有 subagent 時，也顯示側欄（只有根節點）", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit({ nodes: [], main_tool: "Bash: ls", main_tool_count: 2, main_recent: ["Read: a", "Bash: ls"], tools: [], turn: 1 });
    await waitFor(() => expect(container.querySelector(".aiterm-cact")).not.toBeNull());
    expect(container.querySelector('[data-node-id="__root__"]')).not.toBeNull();
  });

  it("主 Claude 計數為 0 且沒有 subagent（例如離開 claude 後）時不顯示", async () => {
    const { container } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit({ nodes: [], main_tool: null, main_tool_count: 0, main_recent: [], tools: [], turn: 0 });
    expect(container.querySelector(".aiterm-cact")).toBeNull();
  });

  // CI 的機器比本機慢三四倍（本機 1.2 秒、CI 曾跑到 5.1 秒踩到預設的 5 秒上限），單獨放寬這一條，斷言不動。
  it("終端機標題列有「協作圖」開關：有活動才出現，點一下收起／重新顯示側欄，並與設定同步", async () => {
    const { container, getByRole, queryByRole } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    expect(queryByRole("button", { name: /協作圖/ })).toBeNull();
    await emit(snap(node("a1", "整理檔案")));
    const btn = await waitFor(() => getByRole("button", { name: /協作圖/ }));
    expect(btn.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector(".aiterm-cact")).not.toBeNull();
    await act(async () => { btn.click(); });
    expect(container.querySelector(".aiterm-cact")).toBeNull();
    expect(getByRole("button", { name: /協作圖/ }).getAttribute("aria-pressed")).toBe("false");
    expect(localStorage.getItem("aiterm-claude-activity-enabled")).toBe("0");
    await act(async () => { getByRole("button", { name: /協作圖/ }).click(); });
    expect(container.querySelector(".aiterm-cact")).not.toBeNull();
  }, 20_000);

  it("設定關閉時按鈕仍在（有活動），可以從這裡重新打開", async () => {
    localStorage.setItem("aiterm-claude-activity-enabled", "0");
    const { container, getByRole } = mount();
    await waitFor(() => expect(listenHandlers.has("claude-activity://test-session")).toBe(true));
    await emit(snap(node("a1", "整理檔案")));
    const btn = await waitFor(() => getByRole("button", { name: /協作圖/ }));
    expect(container.querySelector(".aiterm-cact")).toBeNull();
    await act(async () => { btn.click(); });
    expect(container.querySelector(".aiterm-cact")).not.toBeNull();
  });
});

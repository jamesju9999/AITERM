import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";

// 掛載骨架同 TerminalView.closeGuard.test.tsx（三個 Tauri 入口 + 兩個 hook mock）。
// 差別：get_config 要回真的設定，才能驗證「設定 → WarpInput 灰字建議」整條接線。
const configState = { value: {} as Record<string, unknown> };
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "get_config") return Promise.resolve(configState.value);
    return new Promise(() => {});
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
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

vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: () => ({
    blocks: [],
    isAlternateBuffer: false,
    submitCommand: vi.fn(),
    beginTrackedBlock: vi.fn(),
    appendOutput: vi.fn(),
    setBlockGitInfo: vi.fn(),
    termInstance: null,
  }),
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

const BASE = { execution_mode: "graded", submit_shortcut: "enter", max_agent_steps: 5 };

beforeEach(() => {
  localStorage.clear();
  localStorage.setItem("aiterm-command-history", JSON.stringify(["echo hello-world"]));
});

async function mountAndType(text: string) {
  render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalView tabId="tab-1" />
      </MemoryRouter>
    </LocaleProvider>,
  );
  const ta = (await screen.findByPlaceholderText(/Enter/)) as HTMLTextAreaElement;
  // 讓 refreshConfig 的 getConfig().then(...) 先落地，再輸入。
  await new Promise((r) => setTimeout(r, 0));
  fireEvent.change(ta, { target: { value: text } });
  return ta;
}
const ghost = () => document.querySelector(".warp-input-ghost-suggestion");

describe("TerminalView：設定的接受鍵會傳到輸入框", () => {
  it("設定 tab：輸入開頭後出現灰字，Tab 補上", async () => {
    configState.value = { ...BASE, suggestion_accept_key: "tab" };
    const ta = await mountAndType("echo he");
    await waitFor(() => expect(ghost()?.textContent).toBe("llo-world"));
    expect(fireEvent.keyDown(ta, { key: "Tab" })).toBe(false);
    expect(ta.value).toBe("echo hello-world");
  });

  it("設定 off：沒有灰字", async () => {
    configState.value = { ...BASE, suggestion_accept_key: "off" };
    await mountAndType("echo he");
    await new Promise((r) => setTimeout(r, 20));
    expect(ghost()).toBeNull();
  });

  it("設定 right：→ 補上、Tab 不攔", async () => {
    configState.value = { ...BASE, suggestion_accept_key: "right" };
    const ta = await mountAndType("echo he");
    await waitFor(() => expect(ghost()).not.toBeNull());
    expect(fireEvent.keyDown(ta, { key: "Tab" })).toBe(true);
    expect(fireEvent.keyDown(ta, { key: "ArrowRight" })).toBe(false);
    expect(ta.value).toBe("echo hello-world");
  });

  it("舊版設定沒有這個欄位：預設用 Tab", async () => {
    configState.value = { ...BASE };
    const ta = await mountAndType("echo he");
    await waitFor(() => expect(ghost()?.textContent).toBe("llo-world"));
    expect(fireEvent.keyDown(ta, { key: "Tab" })).toBe(false);
  });
});

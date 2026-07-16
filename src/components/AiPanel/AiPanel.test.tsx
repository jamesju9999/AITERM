import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// Mock Tauri before importing AiPanel (which imports useMcpChat).
const DEFAULT_CONFIG = {
  default_provider: null, providers: [], execution_mode: "graded",
  submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
  default_tab: "terminal", enterprise_server_url: null, enterprise_device_id: null, enterprise_policy: null,
};

// Per-command mock registry: tests can push response objects.
const aiChatQueue: { content: string; tool_calls?: unknown[]; tool_calling_unsupported?: boolean }[] = [];
// Records the `messages` array sent on each "ai_chat" invoke call, in order.
const aiChatCalls: { role: string; content: unknown }[][] = [];

const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: { messages?: { role: string; content: unknown }[] }) => {
    if (cmd === "get_config") return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "get_mcp_tools") return Promise.resolve([]);
    if (cmd === "ai_chat") {
      aiChatCalls.push(payload?.messages ?? []);
      const next = aiChatQueue.shift();
      if (next) return Promise.resolve({ tool_calls: [], tool_calling_unsupported: false, ...next });
      return Promise.resolve({ content: "", tool_calls: [], tool_calling_unsupported: false });
    }
    return Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));
vi.mock("../../contexts/LocaleContext", () => ({
  useLocale: () => ({
    locale: "zh-TW",
    t: {
      mcp_toggle_on: (n: number) => `⚙ MCP (${n})`,
      mcp_toggle_off: "⚙ MCP",
      mcp_toggle_no_servers: "請先在設定中新增 MCP Server",
    },
    setLocale: () => {},
  }),
}));

import { AiPanel } from "./index";

beforeEach(() => {
  aiChatQueue.length = 0;
  aiChatCalls.length = 0;
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

describe("AiPanel", () => {
  it("hides the panel via CSS class when isOpen=false", () => {
    const { container } = render(
      <AiPanel
        sessionId="s1"
        isOpen={false}
        providerName="Ollama (llama3)"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    // Panel element exists but has the hidden class — content stays mounted
    // so the chat hook keeps its listener alive while the user toggles.
    const panel = container.querySelector(".aiterm-ai-panel");
    expect(panel).not.toBeNull();
    expect(panel).toHaveClass("aiterm-ai-panel-hidden");
    // Textarea exists in DOM (not unmounted).
    // aria-hidden="true" on the panel root means Testing Library excludes it
    // from the accessible tree — use { hidden: true } to find it anyway.
    expect(screen.getByRole("textbox", { hidden: true })).toBeInTheDocument();
  });

  it("autofocuses the textarea when transitioning to open", () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox");
    expect(textbox).toHaveFocus();
  });

  it("calls onClose when Escape pressed", async () => {
    const onClose = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={onClose}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("sends a message when Enter pressed", async () => {
    aiChatQueue.push({ content: "好的" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
  });

  it("🗑 New Chat button clears messages", async () => {
    aiChatQueue.push({ content: "ok" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hi");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /New Chat/ }));
    expect(screen.queryByText("ok")).toBeNull();
  });

  it("Agent Mode carries the prior conversation into the next AI call", async () => {
    aiChatQueue.push({ content: "這是計畫內容，要執行嗎？" });
    aiChatQueue.push({ content: "好的，已完成" });

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    // Enable Agent Mode.
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;

    // First turn: ask the AI to propose a plan (it replies with no <cmd> tag,
    // i.e. it's done for this turn and waiting on the user).
    await userEvent.type(textbox, "請規劃整理資料夾的計畫");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("這是計畫內容，要執行嗎？")).toBeInTheDocument());

    // Second turn: ask it to execute the plan just proposed.
    await userEvent.type(textbox, "請執行計畫");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的，已完成")).toBeInTheDocument());

    // The second ai_chat call's message history must include the first
    // turn's user request and the AI's proposed plan, not just this turn's
    // new message.
    expect(aiChatCalls.length).toBe(2);
    const secondCallContents = aiChatCalls[1].map((m) => m.content);
    expect(secondCallContents).toContain("請規劃整理資料夾的計畫");
    expect(secondCallContents).toContain("這是計畫內容，要執行嗎？");
    expect(secondCallContents).toContain("請執行計畫");
  });

  it("provider badge calls onOpenProviderPalette when clicked", async () => {
    const onOpenProviderPalette = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Claude"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={onOpenProviderPalette}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Claude/ }));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });
});

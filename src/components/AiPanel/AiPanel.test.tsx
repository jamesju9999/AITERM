import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";

// Mock Tauri before importing AiPanel (which imports useMcpChat).
const DEFAULT_CONFIG = {
  default_provider: null, providers: [], execution_mode: "graded",
  submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
  default_tab: "terminal", enterprise_server_url: null, enterprise_device_id: null, enterprise_policy: null,
};

// Per-command mock registry: tests can push response objects.
const aiChatQueue: { content: string; tool_calls?: unknown[]; tool_calling_unsupported?: boolean }[] = [];
// Records the sessionId + `messages` array sent on each "ai_chat" invoke
// call, in order. sessionId lets tests distinguish real chat turns from the
// background tab-summary calls fired by summarizeConversation (session id
// suffixed "-summary") — see realChatCalls() below.
const aiChatCalls: { sessionId: string; messages: { role: string; content: unknown }[] }[] = [];

/** Real conversational turns only — excludes the background summary calls
 *  fired by the title-bar AI summary feature (see summarizeTab.ts). */
const realChatCalls = () => aiChatCalls.filter((c) => !c.sessionId.endsWith("-summary"));

const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: { sessionId?: string; messages?: { role: string; content: unknown }[] }) => {
    if (cmd === "get_config") return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "get_mcp_tools") return Promise.resolve([]);
    if (cmd === "ai_chat") {
      const sessionId = payload?.sessionId ?? "";
      aiChatCalls.push({ sessionId, messages: payload?.messages ?? [] });
      // Background tab-summary calls (see summarizeTab.ts) must not consume
      // aiChatQueue — that queue holds scripted replies for the real
      // conversational turns under test, and letting the summary call steal
      // an entry desyncs which reply lands on which turn.
      if (sessionId.endsWith("-summary")) {
        return Promise.resolve({ content: "", tool_calls: [], tool_calling_unsupported: false });
      }
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
    expect(realChatCalls().length).toBe(2);
    const secondCallContents = realChatCalls()[1].messages.map((m) => m.content);
    expect(secondCallContents).toContain("請規劃整理資料夾的計畫");
    expect(secondCallContents).toContain("這是計畫內容，要執行嗎？");
    expect(secondCallContents).toContain("請執行計畫");
  });

  it("Agent Mode recurses to a second AI call after executing a <cmd>", async () => {
    aiChatQueue.push({ content: "<cmd>ls</cmd>" });
    aiChatQueue.push({ content: "完成了" });

    const onExecuteCommand = vi.fn(
      (_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
        onComplete?.({ id: "b1", command: "ls", rawOutput: "file.txt", status: "completed", exitCode: 0, startTime: Date.now() });
      },
    );

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    // The recursive call only fires after onExecuteCommand's onComplete runs
    // (synchronously here) and the loop re-invokes itself via the ref.
    await waitFor(() => expect(screen.getByText("完成了")).toBeInTheDocument());

    expect(onExecuteCommand).toHaveBeenCalledWith("ls", expect.any(Function));
    expect(realChatCalls().length).toBe(2);
    const secondCallContents = realChatCalls()[1].messages.map((m) => m.content);
    expect(secondCallContents.some((c) => typeof c === "string" && c.includes("ls"))).toBe(true);
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

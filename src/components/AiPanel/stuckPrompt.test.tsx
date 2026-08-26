import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";

// setInterval callbacks fire outside any React-managed event, so the state
// update they trigger (setStuckPromptVisible) isn't flushed until the next
// `act()` boundary. Without this wrapper, advancing fake timers updates React
// state but the DOM assertion right after can run before React commits it.
async function advance(ms: number) {
  await act(async () => { await vi.advanceTimersByTimeAsync(ms); });
}

// Mock Tauri before importing AiPanel (which imports useMcpChat).
// Copied from AiPanel.test.tsx — see that file for the rationale behind each
// piece (real translations, per-command invoke registry, etc).
const DEFAULT_CONFIG = {
  default_provider: null, providers: [], execution_mode: "graded",
  submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
  default_tab: "terminal", enterprise_server_url: null, enterprise_device_id: null, enterprise_policy: null,
};

const aiChatQueue: { content: string; tool_calls?: unknown[]; tool_calling_unsupported?: boolean; tool_fallback_reason?: string }[] = [];

const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: { messages?: { role: string; content: unknown }[] }) => {
    if (cmd === "get_config") return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "get_mcp_tools") return Promise.resolve([]);
    if (cmd === "ai_chat") {
      void payload;
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
// 用**真實**的 translations，不要手寫只有幾個 key 的替身——見 AiPanel.test.tsx
// 裡對這個坑的說明。
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>(
    "../../lib/i18n",
  );
  return {
    useLocale: () => ({
      locale: "zh-TW" as const,
      t: translations["zh-TW"],
      setLocale: () => {},
    }),
  };
});

import { AiPanel } from "./index";

beforeEach(() => {
  aiChatQueue.length = 0;
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

/**
 * Starts an Agent-mode run whose <cmd> never completes on its own (the mock
 * onExecuteCommand captures — but does not invoke — onComplete), simulating a
 * shell parked at a stuck prompt (e.g. `heredoc>`). Tests drive time forward
 * and flip `idleMs.current` to control what getIdleMs() reports.
 */
async function renderStuckAgent(opts: {
  idleMs: { current: number };
  onInterruptCommand?: () => void;
}) {
  aiChatQueue.push({ content: "<cmd>cat <<EOF</cmd>" });

  let capturedOnComplete: ((block: TerminalBlock) => void) | undefined;
  const onExecuteCommand = vi.fn(
    (_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      capturedOnComplete = onComplete;
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
      getIdleMs={() => opts.idleMs.current}
      onInterruptCommand={opts.onInterruptCommand}
    />,
  );

  await act(async () => { await Promise.resolve(); });

  fireEvent.click(screen.getByTitle(/啟用 Agent 模式/));

  const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
  fireEvent.change(textbox, { target: { value: "整理暫存檔" } });
  fireEvent.keyDown(textbox, { key: "Enter" });

  await vi.waitFor(() => expect(onExecuteCommand).toHaveBeenCalled());

  return {
    onExecuteCommand,
    completeCommand: (block: TerminalBlock) => capturedOnComplete?.(block),
  };
}

describe("AiPanel — 卡住提示與接手", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("getIdleMs 一直回傳小值時，不顯示卡住提示", async () => {
    const idleMs = { current: 500 };
    await renderStuckAgent({ idleMs });

    await advance(130_000);

    expect(screen.queryByText("指令似乎沒有反應")).toBeNull();
  });

  it("getIdleMs 超過門檻時顯示提示，含中斷與繼續等待兩個按鈕", async () => {
    const idleMs = { current: 130_000 };
    await renderStuckAgent({ idleMs });

    await advance(6_000);

    expect(screen.getByText("指令似乎沒有反應")).toBeInTheDocument();
    expect(screen.getByText("中斷並繼續")).toBeInTheDocument();
    expect(screen.getByText("繼續等待")).toBeInTheDocument();
  });

  it("按下「中斷並繼續」呼叫 onInterruptCommand 一次", async () => {
    const idleMs = { current: 130_000 };
    const onInterruptCommand = vi.fn();
    await renderStuckAgent({ idleMs, onInterruptCommand });

    await advance(6_000);
    expect(screen.getByText("指令似乎沒有反應")).toBeInTheDocument();

    fireEvent.click(screen.getByText("中斷並繼續"));

    expect(onInterruptCommand).toHaveBeenCalledTimes(1);
  });

  it("按下「繼續等待」後提示消失，且不呼叫 onInterruptCommand；也不會立刻又跳出來", async () => {
    const idleMs = { current: 130_000 };
    const onInterruptCommand = vi.fn();
    await renderStuckAgent({ idleMs, onInterruptCommand });

    await advance(6_000);
    expect(screen.getByText("指令似乎沒有反應")).toBeInTheDocument();

    fireEvent.click(screen.getByText("繼續等待"));

    expect(screen.queryByText("指令似乎沒有反應")).toBeNull();
    expect(onInterruptCommand).not.toHaveBeenCalled();

    // idleMs is still reporting a stuck value — without a reset criterion,
    // the very next check would immediately re-show the prompt.
    await advance(6_000);
    expect(screen.queryByText("指令似乎沒有反應")).toBeNull();
  });

  it("指令正常完成後，提示不會殘留", async () => {
    const idleMs = { current: 130_000 };
    const { completeCommand } = await renderStuckAgent({ idleMs });

    await advance(6_000);
    expect(screen.getByText("指令似乎沒有反應")).toBeInTheDocument();

    await act(async () => {
      completeCommand({
        id: "b1", command: "cat <<EOF", rawOutput: "done",
        status: "completed", exitCode: 0, startTime: Date.now(),
      });
      await vi.runOnlyPendingTimersAsync();
    });

    expect(screen.queryByText("指令似乎沒有反應")).toBeNull();

    // And the interval must actually be torn down — advancing further must
    // not resurrect it.
    await advance(130_000);
    expect(screen.queryByText("指令似乎沒有反應")).toBeNull();
  });
});

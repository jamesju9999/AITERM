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

const aiChatQueue: { content: string | null; tool_calls?: unknown[]; tool_calling_unsupported?: boolean; tool_fallback_reason?: string }[] = [];

// ai_chat 被呼叫了幾次——用來確認停止後迴圈真的不再繼續問 AI。
const aiChatCount = { n: 0 };

const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: { messages?: { role: string; content: unknown }[] }) => {
    if (cmd === "get_config") return Promise.resolve(DEFAULT_CONFIG);
    if (cmd === "get_mcp_tools") return Promise.resolve([]);
    if (cmd === "terminal_command_gate") return Promise.resolve({ allowed: true, reason: null });
    if (cmd === "ai_chat") {
      void payload;
      aiChatCount.n++;
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

// 模型呼叫內建終端機工具 run_command 的回覆。
const runCmdCall = (command: string) => ({
  content: null,
  tool_calls: [{ id: `c-${command}`, tool_name: "aiterm_terminal__run_command", server_id: "aiterm-terminal", args: { command } }],
});

beforeEach(() => {
  // ⚡ 開關存在 localStorage，不清的話測試之間會互相汙染。
  localStorage.clear();
  aiChatQueue.length = 0;
  aiChatCount.n = 0;
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

/**
 * Starts an Agent-mode run whose run_command never completes on its own (the mock
 * onExecuteCommand captures — but does not invoke — onComplete), simulating a
 * shell parked at a stuck prompt (e.g. `heredoc>`). Tests drive time forward
 * and flip `idleMs.current` to control what getIdleMs() reports.
 */
async function renderStuckAgent(opts: {
  idleMs: { current: number };
  onInterruptCommand?: () => void;
  onAgentAborted?: () => void;
}) {
  aiChatQueue.push(runCmdCall("cat <<EOF"));

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
      onAgentAborted={opts.onAgentAborted}
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

describe("AiPanel — 停止鍵", () => {
  it("按停止時通知外層（onAgentAborted），讓代 agent 扣住的完成 callback 能立刻放行", async () => {
    // 情境：agent 的指令因權限不足失敗，TerminalView 正扣著它的完成 callback
    // 等使用者回應提權橫幅。停止鍵以前只送 Ctrl+C——shell 早就回到提示字元，
    // 沒有指令會因此結束，agent 就一直停在「執行中」直到使用者回應橫幅。
    const onAgentAborted = vi.fn();
    const { completeCommand } = await renderStuckAgent({ idleMs: { current: 0 }, onAgentAborted });

    fireEvent.click(screen.getByTitle("停止"));
    expect(onAgentAborted).toHaveBeenCalledTimes(1);
    expect(aiChatCount.n).toBe(1);

    // 外層放行後（指令的完成回呼照樣會來），已停止的這一輪不可以再把結果交回 AI 繼續問。
    await act(async () => {
      completeCommand({ id: "b1", command: "DISM", status: "failed", exitCode: 740, startTime: 0, rawOutput: "" });
    });
    await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
    expect(aiChatCount.n).toBe(1);
    expect(screen.queryByTitle("停止")).toBeNull();
  });
});

describe("AiPanel — 舊指令晚到的完成回呼", () => {
  beforeEach(() => { vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  // 停止後指令 A 的完成回呼可能很晚才來（例如外層扣著它等提權橫幅）；那時新的指令 B 已在跑，
  // A 的回呼不可以把 B 的卡住偵測關掉。
  it("不會關掉目前這個指令的卡住偵測", async () => {
    const idleMs = { current: 130_000 };
    const completions: ((block: TerminalBlock) => void)[] = [];
    const onExecuteCommand = vi.fn((_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (onComplete) completions.push(onComplete);
    });
    aiChatQueue.push(runCmdCall("cat <<EOF"));
    render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand} onOpenProviderPalette={vi.fn()}
        getIdleMs={() => idleMs.current} onInterruptCommand={vi.fn()} />,
    );
    await act(async () => { await Promise.resolve(); });
    fireEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textbox, { target: { value: "第一個" } });
    fireEvent.keyDown(textbox, { key: "Enter" });
    await vi.waitFor(() => expect(onExecuteCommand).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByTitle("停止"));

    aiChatQueue.push(runCmdCall("sleep 100"));
    fireEvent.change(textbox, { target: { value: "第二個" } });
    fireEvent.keyDown(textbox, { key: "Enter" });
    await vi.waitFor(() => expect(onExecuteCommand).toHaveBeenCalledTimes(2));

    // A 的完成回呼現在才到。
    await act(async () => {
      completions[0]({ id: "a", command: "cat <<EOF", rawOutput: "", status: "completed", exitCode: 0, startTime: 0 });
    });

    await advance(6_000);
    expect(screen.getByText("指令似乎沒有反應")).toBeInTheDocument();
  });
});

import { createRef } from "react";
import { act } from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import type { AiChatReply, RemoteCtx } from "../../ipc/ai";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { translations } from "../../lib/i18n";

// ── Mocks ────────────────────────────────────────────────────────────────
const { mockInvokeAiChatCtx } = vi.hoisted(() => ({
  mockInvokeAiChatCtx: vi.fn<(...args: unknown[]) => Promise<AiChatReply>>(),
}));
vi.mock("../../ipc/ai", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/ai")>();
  return { ...actual, invokeAiChatCtx: mockInvokeAiChatCtx };
});

// `useRemoteAiChat` 整個換成可控假實作：`messages` 是 module 層的可變陣列，
// `addMessage` 真的把訊息推進去——RemoteAiPanel 每次因為自己的
// agentRunning/agentStep 等 state 改變而重新 render 時，這個 mock hook 都會
// 重新被呼叫、讀到最新的陣列內容，不需要額外的 useState 讓它「有反應」。
const chatMock = vi.hoisted(() => {
  const state = {
    messages: [] as { role: string; content: unknown }[],
    error: null as string | null,
    isStreaming: false,
    streamBuf: "",
    sessions: [] as unknown[],
  };
  const addMessage = vi.fn((m: { role: string; content: unknown }) => {
    state.messages = [...state.messages, m];
  });
  const send = vi.fn();
  const clear = vi.fn(() => { state.messages = []; });
  const loadMessages = vi.fn();
  const deleteSession = vi.fn();
  const resend = vi.fn();
  const reset = () => {
    state.messages = [];
    state.error = null;
    state.isStreaming = false;
    state.streamBuf = "";
    state.sessions = [];
    addMessage.mockClear();
    send.mockClear();
    clear.mockClear();
    loadMessages.mockClear();
    deleteSession.mockClear();
    resend.mockClear();
  };
  return { state, addMessage, send, clear, loadMessages, deleteSession, resend, reset };
});
vi.mock("../../hooks/useRemoteAiChat", () => ({
  useRemoteAiChat: () => ({
    messages: chatMock.state.messages,
    isStreaming: chatMock.state.isStreaming,
    streamBuf: chatMock.state.streamBuf,
    error: chatMock.state.error,
    send: chatMock.send,
    addMessage: chatMock.addMessage,
    clear: chatMock.clear,
    loadMessages: chatMock.loadMessages,
    deleteSession: chatMock.deleteSession,
    resend: chatMock.resend,
    sessions: chatMock.state.sessions,
  }),
}));

import { RemoteAiPanel, type RemoteAiPanelHandle } from "./RemoteAiPanel";

const t = translations["zh-TW"];

function aiReply(content: string | null): AiChatReply {
  return { content, tool_calls: [], tool_calling_unsupported: false, tool_fallback_reason: null };
}

function fakeBlock(rawOutput: string, exitCode = 0): TerminalBlock {
  return {
    id: "block-1",
    command: "ls",
    status: exitCode === 0 ? "completed" : "failed",
    exitCode,
    startTime: Date.now(),
    rawOutput,
  };
}

function renderPanel(overrides: Partial<{
  isControl: boolean;
  maxSteps: number;
  sharedAbortRef: React.MutableRefObject<boolean>;
}> = {}) {
  const ref = createRef<RemoteAiPanelHandle>();
  const submitCommand = vi.fn<(cmd: string, onComplete?: (b: TerminalBlock) => void) => void>();
  const sharedAbortRef = overrides.sharedAbortRef ?? { current: false };
  const buildRemoteCtx = (): RemoteCtx => ({ os: "linux", shell: "bash", cwd: "/home/user", recentOutput: null });

  render(
    <RemoteAiPanel
      ref={ref}
      connId="conn-1"
      buildRemoteCtx={buildRemoteCtx}
      submitCommand={submitCommand}
      isControl={overrides.isControl ?? true}
      maxSteps={overrides.maxSteps ?? 5}
      providerName="Test Provider"
      providerId="test-provider"
      onOpenProviderPalette={vi.fn()}
      isOpen
      onClose={vi.fn()}
      sharedAbortRef={sharedAbortRef}
    />,
  );

  return { ref, submitCommand, sharedAbortRef };
}

async function flushMicrotasks(times = 6) {
  for (let i = 0; i < times; i++) {
    await Promise.resolve();
  }
}

function placeholder(): string | null {
  return screen.getByRole("textbox").getAttribute("placeholder");
}

beforeEach(() => {
  chatMock.reset();
  mockInvokeAiChatCtx.mockReset();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("RemoteAiPanel", () => {
  it("submitAgent asks the AI, and a <cmd> reply executes via submitCommand", async () => {
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("<cmd>ls -la</cmd>"));
    const { ref, submitCommand } = renderPanel();

    await act(async () => {
      ref.current!.submitAgent("list the files");
      await flushMicrotasks();
    });

    expect(mockInvokeAiChatCtx).toHaveBeenCalledTimes(1);
    const [messages] = mockInvokeAiChatCtx.mock.calls[0];
    expect(messages).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ role: "system" }),
        expect.objectContaining({ role: "user", content: "list the files" }),
      ]),
    );

    expect(submitCommand).toHaveBeenCalledTimes(1);
    expect(submitCommand.mock.calls[0][0]).toBe("ls -la");
  });

  it("feeds the finished command's output into the next AI call", async () => {
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("<cmd>ls -la</cmd>"));
    const { ref, submitCommand } = renderPanel();

    await act(async () => {
      ref.current!.submitAgent("list the files");
      await flushMicrotasks();
    });

    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("all done"));
    const onComplete = submitCommand.mock.calls[0][1]!;

    await act(async () => {
      onComplete(fakeBlock("total 0\ndrwxr-xr-x  hello-output-marker", 0));
      await flushMicrotasks();
    });

    expect(mockInvokeAiChatCtx).toHaveBeenCalledTimes(2);
    const [secondMessages] = mockInvokeAiChatCtx.mock.calls[1];
    const joined = JSON.stringify(secondMessages);
    expect(joined).toContain("hello-output-marker");
    expect(joined).toContain("exit code 0");
  });

  it("a reply without <cmd> ends the loop — that reply is the final summary", async () => {
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("The goal is already done."));
    const { ref } = renderPanel();

    await act(async () => {
      ref.current!.submitAgent("check status");
      await flushMicrotasks();
    });

    const lastCall = chatMock.addMessage.mock.calls[chatMock.addMessage.mock.calls.length - 1];
    expect(lastCall[0]).toEqual({ role: "assistant", content: "The goal is already done." });
    expect(placeholder()).not.toMatch(/執行中/);
  });

  it("abort() stops the loop — no further AI calls or commands after aborting", async () => {
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("<cmd>ls -la</cmd>"));
    const { ref, submitCommand } = renderPanel();

    await act(async () => {
      ref.current!.submitAgent("goal");
      await flushMicrotasks();
    });

    const onComplete = submitCommand.mock.calls[0][1]!;

    act(() => {
      ref.current!.abort();
    });

    await act(async () => {
      onComplete(fakeBlock("some output", 0));
      await flushMicrotasks();
    });

    expect(mockInvokeAiChatCtx).toHaveBeenCalledTimes(1);
    expect(submitCommand).toHaveBeenCalledTimes(1);
  });

  it("isControl=false makes submitAgent and send no-ops", async () => {
    const { ref, submitCommand } = renderPanel({ isControl: false });

    await act(async () => {
      ref.current!.submitAgent("goal");
      ref.current!.send("hi");
      await flushMicrotasks();
    });

    expect(mockInvokeAiChatCtx).not.toHaveBeenCalled();
    expect(submitCommand).not.toHaveBeenCalled();
    expect(chatMock.send).not.toHaveBeenCalled();
  });

  it("a per-step timeout fires when submitCommand's onComplete never comes back", async () => {
    vi.useFakeTimers();
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("<cmd>ls -la</cmd>"));
    const { ref, submitCommand } = renderPanel();

    act(() => {
      ref.current!.submitAgent("goal");
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(submitCommand).toHaveBeenCalledTimes(1);
    // onComplete deliberately never invoked — simulates a connection with no
    // OSC-133 shell integration.

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });

    const lastCall = chatMock.addMessage.mock.calls[chatMock.addMessage.mock.calls.length - 1];
    expect(lastCall[0]).toEqual({ role: "assistant", content: t.remote_agent_no_shell_integration });
    expect(placeholder()).not.toMatch(/執行中/);
  });

  it("an externally-set sharedAbortRef stops the loop on its next check", async () => {
    mockInvokeAiChatCtx.mockResolvedValueOnce(aiReply("<cmd>ls -la</cmd>"));
    const sharedAbortRef = { current: false };
    const { ref, submitCommand } = renderPanel({ sharedAbortRef });

    await act(async () => {
      ref.current!.submitAgent("goal");
      await flushMicrotasks();
    });

    const onComplete = submitCommand.mock.calls[0][1]!;
    sharedAbortRef.current = true;

    await act(async () => {
      onComplete(fakeBlock("some output", 0));
      await flushMicrotasks();
    });

    expect(mockInvokeAiChatCtx).toHaveBeenCalledTimes(1);
    expect(submitCommand).toHaveBeenCalledTimes(1);
  });
});

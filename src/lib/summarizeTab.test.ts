import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { summarizeCommands } from "./summarizeTab";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";

beforeEach(() => {
  invokeMock.mockReset();
});

function block(command: string, overrides: Partial<TerminalBlock> = {}): TerminalBlock {
  return {
    id: Math.random().toString(36).slice(2),
    command,
    status: "completed",
    startTime: Date.now(),
    rawOutput: "",
    ...overrides,
  };
}

describe("summarizeCommands", () => {
  it("returns null and makes no AI call when there are no commands", async () => {
    const result = await summarizeCommands([], "sess-1", "zh-TW");
    expect(result).toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("returns null and makes no AI call when all commands are blank/whitespace", async () => {
    const result = await summarizeCommands([block("   "), block("")], "sess-1", "zh-TW");
    expect(result).toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("calls ai_chat with a summary-suffixed session id and returns the trimmed reply", async () => {
    invokeMock.mockResolvedValue({ content: "  查詢本機 IP  ", tool_calls: [], tool_calling_unsupported: false });

    const result = await summarizeCommands(
      [block("ifconfig en0", { cwd: "/Users/jamesju/Downloads" })],
      "sess-1",
      "zh-TW",
    );

    expect(result).toBe("查詢本機 IP");
    expect(invokeMock).toHaveBeenCalledWith(
      "ai_chat",
      expect.objectContaining({ sessionId: "sess-1-summary" }),
    );

    const promptText = (invokeMock.mock.calls[0][1] as { messages: { content: string }[] }).messages[0].content;
    expect(promptText).toContain("/Users/jamesju/Downloads");
  });

  it("returns null when invokeAiChat rejects", async () => {
    invokeMock.mockRejectedValue(new Error("network error"));
    const result = await summarizeCommands([block("ls")], "sess-1", "zh-TW");
    expect(result).toBeNull();
  });

  it("returns null when the reply content is empty/whitespace", async () => {
    invokeMock.mockResolvedValue({ content: "   ", tool_calls: [], tool_calling_unsupported: false });
    const result = await summarizeCommands([block("ls")], "sess-1", "zh-TW");
    expect(result).toBeNull();
  });

  it("builds an English prompt for the en locale", async () => {
    invokeMock.mockResolvedValue({ content: "check local IP", tool_calls: [], tool_calling_unsupported: false });

    const result = await summarizeCommands([block("ifconfig en0", { cwd: "/tmp" })], "sess-1", "en");

    expect(result).toBe("check local IP");

    const promptText = (invokeMock.mock.calls[0][1] as { messages: { content: string }[] }).messages[0].content;
    expect(promptText).toContain("Working directory: /tmp");
    expect(promptText).toContain("shell commands a user ran in one terminal session");
  });

  it("only includes the last 10 commands in the prompt sent to invokeAiChat", async () => {
    invokeMock.mockResolvedValue({ content: "summary", tool_calls: [], tool_calling_unsupported: false });

    const many: TerminalBlock[] = [];
    for (let i = 0; i < 14; i++) many.push(block(`command-${i}`));
    await summarizeCommands(many, "sess-1", "zh-TW");

    const promptText = (invokeMock.mock.calls[0][1] as { messages: { content: string }[] }).messages[0].content;
    expect(promptText).not.toContain("command-0");
    expect(promptText).not.toContain("command-3");
    expect(promptText).toContain("command-4");
    expect(promptText).toContain("command-13");
  });
});

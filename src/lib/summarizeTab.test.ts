import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

import { summarizeConversation } from "./summarizeTab";
import type { McpChatMessage } from "../hooks/useMcpChat";

beforeEach(() => {
  invokeMock.mockReset();
});

const userMsg = (text: string): McpChatMessage => ({ role: "user", content: text });
const assistantMsg = (text: string): McpChatMessage => ({ role: "assistant", content: text });

describe("summarizeConversation", () => {
  it("returns null when there is no assistant reply yet", async () => {
    const result = await summarizeConversation([userMsg("hello")], "sess-1", "zh-TW");
    expect(result).toBeNull();
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("calls ai_chat with a summary-suffixed session id and returns the trimmed reply", async () => {
    invokeMock.mockResolvedValue({ content: "  除錯 OAuth 流程  ", tool_calls: [], tool_calling_unsupported: false });

    const result = await summarizeConversation(
      [userMsg("幫我看一下這段程式碼"), assistantMsg("好的，我看看")],
      "sess-1",
      "zh-TW",
    );

    expect(result).toBe("除錯 OAuth 流程");
    expect(invokeMock).toHaveBeenCalledWith(
      "ai_chat",
      expect.objectContaining({ sessionId: "sess-1-summary" }),
    );
  });

  it("returns null when invokeAiChat rejects", async () => {
    invokeMock.mockRejectedValue(new Error("network error"));

    const result = await summarizeConversation(
      [userMsg("hi"), assistantMsg("hello")],
      "sess-1",
      "zh-TW",
    );

    expect(result).toBeNull();
  });

  it("returns null when the reply content is empty", async () => {
    invokeMock.mockResolvedValue({ content: "   ", tool_calls: [], tool_calling_unsupported: false });

    const result = await summarizeConversation(
      [userMsg("hi"), assistantMsg("hello")],
      "sess-1",
      "zh-TW",
    );

    expect(result).toBeNull();
  });

  it("only includes the last 10 messages in the prompt sent to invokeAiChat", async () => {
    invokeMock.mockResolvedValue({ content: "summary", tool_calls: [], tool_calling_unsupported: false });

    const many: McpChatMessage[] = [];
    for (let i = 0; i < 14; i++) {
      many.push(userMsg(`user-msg-${i}`));
      many.push(assistantMsg(`assistant-msg-${i}`));
    }
    // 28 messages total; only the last 10 should appear in the prompt.
    await summarizeConversation(many, "sess-1", "zh-TW");

    const call = invokeMock.mock.calls[0];
    const promptText = (call[1] as { messages: { content: string }[] }).messages[0].content as string;
    expect(promptText).not.toContain("user-msg-0");
    expect(promptText).toContain("assistant-msg-13");
  });
});

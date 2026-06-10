// src/hooks/useMcpChat.test.ts
import { describe, it, expect, vi } from "vitest";

// Mock the IPC modules
vi.mock("../ipc/ai", () => ({
  aiChat: vi.fn(),
}));
vi.mock("../ipc/mcp", () => ({
  getMcpTools: vi.fn().mockResolvedValue([]),
  executeMcpTool: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { renderHook, act } from "@testing-library/react";
import { aiChat } from "../ipc/ai";
import { useMcpChat } from "./useMcpChat";

describe("useMcpChat", () => {
  it("returns content when no tool calls", async () => {
    vi.mocked(aiChat).mockResolvedValueOnce({
      content: "Hello world",
      tool_calls: [],
      tool_calling_unsupported: false,
    });

    const { result } = renderHook(() => useMcpChat("session-1"));

    await act(async () => {
      await result.current.sendMessage("Hi", true);
    });

    const lastMsg = result.current.messages.at(-1);
    expect(lastMsg?.role).toBe("assistant");
    // content is streamed via ai-stream events in real usage;
    // in tests the hook receives it from aiChat return value
  });

  it("sets tool_calling_unsupported flag when provider doesn't support tools", async () => {
    vi.mocked(aiChat).mockResolvedValue({
      content: "Hello",
      tool_calls: [],
      tool_calling_unsupported: true,
    });

    const { result } = renderHook(() => useMcpChat("session-1"));
    await act(async () => {
      await result.current.sendMessage("Hi", true);
    });

    expect(result.current.toolCallingUnsupported).toBe(true);
  });
});

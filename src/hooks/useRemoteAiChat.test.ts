import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const invokeChatCtx = vi.fn();
vi.mock("../ipc/ai", async (imp) => ({ ...(await imp<any>()), invokeAiChatCtx: (...a: unknown[]) => invokeChatCtx(...a) }));

const listeners: Array<(e: unknown) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_n: string, cb: (e: unknown) => void) => { listeners.push(cb); return Promise.resolve(() => {}); },
}));
vi.mock("../contexts/LocaleContext", () => ({ useLocale: () => ({ t: { chat_empty: "empty" }, locale: "en" }) }));

import { useRemoteAiChat } from "./useRemoteAiChat";
const CTX = { os: "linux", shell: null, cwd: null, recentOutput: null };

beforeEach(() => {
  invokeChatCtx.mockReset().mockResolvedValue({ content: "answer", tool_calls: [], tool_calling_unsupported: false });
  listeners.length = 0;
  localStorage.clear();
});

describe("useRemoteAiChat", () => {
  it("send() calls invokeAiChatCtx with connId and appends the assistant reply", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("hi"); });
    expect(invokeChatCtx).toHaveBeenCalledWith(
      expect.arrayContaining([{ role: "user", content: "hi" }]), CTX, "conn-1", undefined, "en", true,
    );
    expect(result.current.messages.map(m => m.content)).toContain("answer");
  });

  it("passes providerId through to invokeAiChatCtx when given", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX, "openai"));
    await act(async () => { await result.current.send("hi"); });
    expect(invokeChatCtx).toHaveBeenCalledWith(
      expect.arrayContaining([{ role: "user", content: "hi" }]), CTX, "conn-1", "openai", "en", true,
    );
  });

  it("accumulates ai-stream deltas for matching connId/kind into streamBuf", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await waitFor(() => expect(listeners.length).toBeGreaterThan(0));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "chat", delta: "par", done: false } }));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "chat", delta: "tial", done: false } }));
    expect(result.current.streamBuf).toBe("partial");
    // wrong id / kind ignored
    act(() => listeners[0]({ payload: { session_id: "other", kind: "chat", delta: "X", done: false } }));
    act(() => listeners[0]({ payload: { session_id: "conn-1", kind: "query", delta: "Y", done: false } }));
    expect(result.current.streamBuf).toBe("partial");
  });

  it("persists a session to aiterm-remote-chat-sessions", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("remember me"); });
    const raw = localStorage.getItem("aiterm-remote-chat-sessions");
    expect(raw).toContain("remember me");
  });

  it("clear() empties messages", async () => {
    const { result } = renderHook(() => useRemoteAiChat("conn-1", () => CTX));
    await act(async () => { await result.current.send("x"); });
    act(() => result.current.clear());
    expect(result.current.messages).toEqual([]);
  });
});

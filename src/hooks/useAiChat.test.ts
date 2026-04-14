import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mock the Tauri modules BEFORE importing the hook.
const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// Import AFTER the mocks are set up.
import { useAiChat } from "./useAiChat";

// Capture the event callback so tests can fire fake stream events.
type StreamPayload = {
  session_id: string;
  kind: "query" | "chat";
  delta: string;
  done: boolean;
};
let lastEventCallback: ((e: { payload: StreamPayload }) => void) | null = null;
let unlistenSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  unlistenSpy = vi.fn();
  lastEventCallback = null;
  listenMock.mockImplementation((_event: string, cb: typeof lastEventCallback) => {
    lastEventCallback = cb;
    return Promise.resolve(unlistenSpy);
  });
});

afterEach(() => {
  lastEventCallback = null;
});

function fireStream(payload: Partial<StreamPayload>) {
  const full: StreamPayload = {
    session_id: "s1",
    kind: "chat",
    delta: "",
    done: false,
    ...payload,
  };
  act(() => {
    lastEventCallback?.({ payload: full });
  });
}

describe("useAiChat", () => {
  it("starts with empty state", () => {
    const { result } = renderHook(() => useAiChat("s1"));
    expect(result.current.messages).toEqual([]);
    expect(result.current.streamBuf).toBe("");
    expect(result.current.isStreaming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("send appends user then assistant on success", async () => {
    invokeMock.mockResolvedValueOnce({ content: "來試試 <cmd>ls</cmd>" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("列出檔案");
    });

    expect(result.current.messages).toHaveLength(2);
    expect(result.current.messages[0]).toEqual({ role: "user", content: "列出檔案" });
    expect(result.current.messages[1]).toEqual({
      role: "assistant",
      content: "來試試 <cmd>ls</cmd>",
    });
    expect(result.current.isStreaming).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it("send keeps user message and sets error on failure", async () => {
    invokeMock.mockRejectedValueOnce({ kind: "network", message: "boom" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("試試看");
    });

    expect(result.current.messages).toEqual([
      { role: "user", content: "試試看" },
    ]);
    expect(result.current.error).toEqual({ kind: "network", message: "boom" });
    expect(result.current.isStreaming).toBe(false);
  });

  it("stream event updates streamBuf (non-done chunks)", async () => {
    // Keep invoke pending so we can observe streaming state.
    let resolveInvoke: (v: { content: string }) => void = () => {};
    invokeMock.mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolveInvoke = r;
        }),
    );
    const { result } = renderHook(() => useAiChat("s1"));

    let sendPromise: Promise<void> = Promise.resolve();
    act(() => {
      sendPromise = result.current.send("hi");
    });

    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    fireStream({ delta: "hello " });
    fireStream({ delta: "world" });
    await waitFor(() => expect(result.current.streamBuf).toBe("hello world"));

    // Non-matching kind or session should be ignored.
    fireStream({ kind: "query", delta: "!!!" });
    fireStream({ session_id: "other", delta: "???" });
    expect(result.current.streamBuf).toBe("hello world");

    // Finish the invoke.
    await act(async () => {
      resolveInvoke({ content: "hello world" });
      await sendPromise;
    });

    expect(result.current.streamBuf).toBe("");
    expect(result.current.messages[1].content).toBe("hello world");
  });

  it("resend does not duplicate user message", async () => {
    // First call fails.
    invokeMock.mockRejectedValueOnce({ kind: "network", message: "fail" });
    // Second call succeeds.
    invokeMock.mockResolvedValueOnce({ content: "好的" });

    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("試");
    });
    expect(result.current.messages).toHaveLength(1);
    expect(result.current.error).not.toBeNull();

    await act(async () => {
      await result.current.resend();
    });
    // Only user + assistant — no duplicate user.
    expect(result.current.messages).toHaveLength(2);
    expect(result.current.messages[0]).toEqual({ role: "user", content: "試" });
    expect(result.current.messages[1]).toEqual({ role: "assistant", content: "好的" });
  });

  it("resend is a no-op when messages empty or last is not user", async () => {
    invokeMock.mockResolvedValue({ content: "x" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.resend();
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("clear resets messages, error, and streamBuf", async () => {
    invokeMock.mockResolvedValueOnce({ content: "answer" });
    const { result } = renderHook(() => useAiChat("s1"));

    await act(async () => {
      await result.current.send("q");
    });
    expect(result.current.messages).toHaveLength(2);

    act(() => {
      result.current.clear();
    });
    expect(result.current.messages).toEqual([]);
    expect(result.current.error).toBeNull();
    expect(result.current.streamBuf).toBe("");
  });

  it("truncates history to 20 messages when sending", async () => {
    invokeMock.mockResolvedValue({ content: "ok" });
    const { result } = renderHook(() => useAiChat("s1"));

    // Send 11 rounds (22 messages → should truncate to 20 before invoke).
    for (let i = 0; i < 11; i++) {
      await act(async () => {
        await result.current.send(`q${i}`);
      });
    }

    // Last call's messages arg must have length ≤ 20.
    const lastCall = invokeMock.mock.calls[invokeMock.mock.calls.length - 1];
    // invokeAiChat calls invoke("ai_chat", { messages, sessionId })
    const args = lastCall[1] as { messages: unknown[] };
    expect(args.messages.length).toBeLessThanOrEqual(20);
  });
});

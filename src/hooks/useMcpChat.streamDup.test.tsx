import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// 模擬 Tauri：listen() 的 promise 由測試控制何時 resolve（真實世界裡它是跨 IPC 的非同步呼叫），
// handler 在 resolve 時才算登記；unlisten 才會把它拿掉；事件會送給所有仍登記的 handler。
type Handler = (e: { payload: { session_id: string; kind: string; delta: string; done: boolean } }) => void;
const active = new Set<Handler>();
const pending: Array<() => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, h: Handler) =>
    new Promise<() => void>((resolve) => {
      pending.push(() => {
        active.add(h);
        resolve(() => { active.delete(h); });
      });
    }),
}));
vi.mock("../ipc/ai", () => ({ aiChat: vi.fn() }));
vi.mock("../ipc/mcp", () => ({ executeMcpTool: vi.fn() }));

import { LocaleProvider } from "../contexts/LocaleContext";
import { useMcpChat } from "./useMcpChat";
import { useRemoteAiChat } from "./useRemoteAiChat";

const flush = () => act(async () => { while (pending.length) pending.shift()!(); });
const emit = (sid: string, delta: string) =>
  act(async () => {
    for (const h of [...active]) h({ payload: { session_id: sid, kind: "chat", delta, done: false } });
  });

beforeEach(() => { active.clear(); pending.length = 0; });

/**
 * 重現：effect 的 cleanup 在 listen() resolve 之前就跑掉（StrictMode 雙重掛載、或 session 來回切換），
 * 拿不到 unlisten 函式，那個 listener 就永遠留著。之後同一個 session 又有新的 listener，
 * 每個串流 delta 就被處理兩次——畫面上每個字都出現兩次。
 */
describe("串流 delta 不可被處理兩次", () => {
  it("useMcpChat：session a → b → a 之後，a 的 delta 只加一次", async () => {
    const { result, rerender } = renderHook(({ id }) => useMcpChat(id), {
      wrapper: LocaleProvider,
      initialProps: { id: "a" },
    });
    rerender({ id: "b" });
    rerender({ id: "a" });
    await flush();
    await emit("a", "我");
    await emit("a", "已");
    expect(result.current.streamBuf).toBe("我已");
  });

  it("useMcpChat：解除登記失敗的舊 listener 不會影響結果，且 resolve 之後會被補解除", async () => {
    const { rerender } = renderHook(({ id }) => useMcpChat(id), {
      wrapper: LocaleProvider,
      initialProps: { id: "a" },
    });
    rerender({ id: "b" });
    await flush();
    // 只剩目前這個 session 的一個 listener；先前被取代的那個在 resolve 後已被解除。
    expect(active.size).toBe(1);
  });

  it("useRemoteAiChat：同樣的情形也只加一次", async () => {
    const { result, rerender } = renderHook(({ id }) => useRemoteAiChat(id, () => ({} as never)), {
      wrapper: LocaleProvider,
      initialProps: { id: "a" },
    });
    rerender({ id: "b" });
    rerender({ id: "a" });
    await flush();
    await emit("a", "你");
    await emit("a", "好");
    expect(result.current.streamBuf).toBe("你好");
  });
});

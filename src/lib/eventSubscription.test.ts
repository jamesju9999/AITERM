import { beforeEach, describe, expect, it, vi } from "vitest";

import { unlistenOnCleanup } from "./eventSubscription";

describe("unlistenOnCleanup", () => {
  beforeEach(() => vi.clearAllMocks());

  it("解除一次", async () => {
    const un = vi.fn();
    const cleanup = unlistenOnCleanup(Promise.resolve(un), "evt");
    cleanup();
    await vi.waitFor(() => expect(un).toHaveBeenCalledTimes(1));
  });

  // StrictMode 雙重掛載、HMR 都可能讓 cleanup 跑兩次；第二次一定失敗，
  // 因為第一次已經把 eventId 從登記簿移掉了。
  it("重複呼叫只會解除一次", async () => {
    const un = vi.fn();
    const cleanup = unlistenOnCleanup(Promise.resolve(un), "evt");
    cleanup();
    cleanup();
    cleanup();
    await vi.waitFor(() => expect(un).toHaveBeenCalledTimes(1));
  });

  // 這正是原本 console 出現 "undefined is not an object" 未處理 rejection
  // 的情況。要印出來（後端可能還留著監聽器），但不能讓它變成未處理的
  // rejection。
  it("解除失敗時印出警告而不是丟出未處理的 rejection", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const cleanup = unlistenOnCleanup(
      Promise.resolve(() => {
        throw new Error("listeners[eventId] is undefined");
      }),
      "task-finished",
    );
    cleanup();
    await vi.waitFor(() => expect(warn).toHaveBeenCalled());
    expect(String(warn.mock.calls[0][0])).toContain("task-finished");
    warn.mockRestore();
  });

  // cleanup 可能在 listen() 還沒 resolve 時就跑（快速掛載又卸載）。
  it("在 listen 尚未 resolve 時呼叫，仍然會在 resolve 後解除", async () => {
    const un = vi.fn();
    let settle: (f: () => void) => void = () => {};
    const pending = new Promise<() => void>((r) => {
      settle = r;
    });
    const cleanup = unlistenOnCleanup(pending, "evt");
    cleanup();
    expect(un).not.toHaveBeenCalled();
    settle(un);
    await vi.waitFor(() => expect(un).toHaveBeenCalledTimes(1));
  });
});

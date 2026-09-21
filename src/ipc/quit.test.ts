import { describe, it, expect, vi, beforeEach } from "vitest";

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { onQuitRequested, QUIT_REQUESTED_EVENT } from "./quit";

beforeEach(() => {
  listenMock.mockReset();
});

describe("quit ipc", () => {
  it("onQuitRequested 訂閱 Rust 端的事件名並轉呼叫 callback", async () => {
    const unlisten = vi.fn();
    listenMock.mockImplementation((_name: string, handler: () => void) => {
      handler();
      return Promise.resolve(unlisten);
    });
    const cb = vi.fn();
    const got = await onQuitRequested(cb);
    expect(listenMock.mock.calls[0][0]).toBe("app://quit-requested");
    expect(QUIT_REQUESTED_EVENT).toBe("app://quit-requested");
    expect(cb).toHaveBeenCalledTimes(1);
    expect(got).toBe(unlisten);
  });
});

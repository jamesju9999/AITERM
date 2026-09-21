import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { setQuitConfirmed, onQuitRequested, QUIT_REQUESTED_EVENT } from "./quit";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("quit ipc", () => {
  it("setQuitConfirmed 呼叫 set_quit_confirmed", async () => {
    invokeMock.mockResolvedValue(undefined);
    await setQuitConfirmed();
    expect(invokeMock).toHaveBeenCalledWith("set_quit_confirmed");
  });

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

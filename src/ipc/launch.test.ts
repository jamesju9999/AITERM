import { describe, it, expect, vi, beforeEach } from "vitest";

const invokeMock = vi.fn();
const listenMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: (...a: unknown[]) => listenMock(...a) }));

import { takeLaunchRequests, onLaunchRequestPending } from "./launch";

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
});

describe("launch ipc", () => {
  it("takeLaunchRequests invokes take_launch_requests and returns its result", async () => {
    invokeMock.mockResolvedValue([{ cwd: "/a", script: null, command: null }]);
    await expect(takeLaunchRequests()).resolves.toEqual([{ cwd: "/a", script: null, command: null }]);
    expect(invokeMock).toHaveBeenCalledWith("take_launch_requests");
  });

  it("onLaunchRequestPending listens on launch-request-pending and ignores the payload", async () => {
    let handler: ((e: { payload: unknown }) => void) | undefined;
    listenMock.mockImplementation((_name: string, h: (e: { payload: unknown }) => void) => {
      handler = h;
      return Promise.resolve(() => {});
    });
    const cb = vi.fn();
    await onLaunchRequestPending(cb);
    expect(listenMock.mock.calls[0][0]).toBe("launch-request-pending");
    handler?.({ payload: null });
    expect(cb).toHaveBeenCalledTimes(1);
  });
});

import { describe, expect, it, vi, beforeEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";

const shareStartMock = vi.fn();
const shareStopMock = vi.fn();
const shareStatusMock = vi.fn();
const shareViewersMock = vi.fn();
const shareKickMock = vi.fn();
const shareRevokeMock = vi.fn();
let viewersChangedCb: (() => void) | null = null;

vi.mock("../ipc/share", () => ({
  shareStart: (...a: unknown[]) => shareStartMock(...a),
  shareStop: (...a: unknown[]) => shareStopMock(...a),
  shareStatus: (...a: unknown[]) => shareStatusMock(...a),
  shareViewers: (...a: unknown[]) => shareViewersMock(...a),
  shareKick: (...a: unknown[]) => shareKickMock(...a),
  shareRevokeControl: (...a: unknown[]) => shareRevokeMock(...a),
  onShareViewersChanged: (cb: () => void) => {
    viewersChangedCb = cb;
    return Promise.resolve(() => {});
  },
}));

import { useShareHost } from "./useShareHost";

beforeEach(() => {
  shareStartMock.mockReset().mockResolvedValue({ sharing: true, code: "559207", port: 47823 });
  shareStopMock.mockReset().mockResolvedValue({ sharing: false, code: null, port: null });
  shareStatusMock.mockReset().mockResolvedValue({ sharing: false, code: null, port: null });
  shareViewersMock.mockReset().mockResolvedValue([]);
  shareKickMock.mockReset().mockResolvedValue(undefined);
  shareRevokeMock.mockReset().mockResolvedValue(undefined);
  viewersChangedCb = null;
});

describe("useShareHost", () => {
  it("starts out not sharing", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await waitFor(() => expect(shareStatusMock).toHaveBeenCalledWith("tab-1"));
    expect(result.current.sharing).toBe(false);
    expect(result.current.code).toBeNull();
  });

  it("exposes the code and address after starting", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });
    expect(shareStartMock).toHaveBeenCalledWith("tab-1");
    expect(result.current.code).toBe("559207");
    expect(result.current.port).toBe(47823);
  });

  it("re-reads the viewer list when the backend says it changed", async () => {
    // 事件刻意不帶內容——收到就重讀，避免推播的資料跟查詢的資料對不上。
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });

    shareViewersMock.mockResolvedValue([
      { viewerId: "v1", displayName: "Alice", mode: "control" },
    ]);
    await act(async () => {
      viewersChangedCb?.();
    });

    await waitFor(() => expect(result.current.viewers).toHaveLength(1));
    expect(result.current.viewers[0].displayName).toBe("Alice");
  });

  it("clears its state when sharing stops", async () => {
    const { result } = renderHook(() => useShareHost("tab-1"));
    await act(async () => {
      await result.current.start();
    });
    await act(async () => {
      await result.current.stop();
    });
    expect(result.current.sharing).toBe(false);
    expect(result.current.code).toBeNull();
    expect(result.current.viewers).toEqual([]);
  });
});

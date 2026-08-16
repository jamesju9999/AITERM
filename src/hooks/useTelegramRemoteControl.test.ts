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
import { useTelegramRemoteControl } from "./useTelegramRemoteControl";

let unlistenSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  unlistenSpy = vi.fn();
  localStorage.clear();
  listenMock.mockImplementation(() => Promise.resolve(unlistenSpy));
});

afterEach(() => {
  localStorage.clear();
});

describe("useTelegramRemoteControl", () => {
  it("不是 remote 分頁時（isRemoteEnabled 預設 false）不註冊 listener", () => {
    renderHook(() => useTelegramRemoteControl("tab-1", true, () => {}));

    expect(listenMock).not.toHaveBeenCalled();
  });

  it("是 remote 分頁時（isRemoteEnabled 打開）會註冊 listener", async () => {
    const { result } = renderHook(() =>
      useTelegramRemoteControl("tab-1", true, () => {})
    );

    act(() => {
      result.current.setIsRemoteEnabled(true);
    });

    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));
    expect(listenMock).toHaveBeenCalledWith(
      "telegram-message-received",
      expect.any(Function)
    );
  });

  it("從 remote 分頁變成非 remote 分頁時會 unlisten（回歸的核心機制）", async () => {
    const { result } = renderHook(() =>
      useTelegramRemoteControl("tab-1", true, () => {})
    );

    act(() => {
      result.current.setIsRemoteEnabled(true);
    });
    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));
    expect(unlistenSpy).not.toHaveBeenCalled();

    act(() => {
      result.current.setIsRemoteEnabled(false);
    });

    await waitFor(() => expect(unlistenSpy).toHaveBeenCalledTimes(1));
  });

  it("isRemoteEnabled 不會從 localStorage 讀回來（刻意不持久化）", () => {
    // 舊的（已拿掉的）持久化邏輯會用 `aiterm-remote:${tabId}` 當 key。
    // 就算 localStorage 裡有 "true"，開機當下也必須是 false。
    localStorage.setItem("aiterm-remote:tab-1", "true");

    const { result } = renderHook(() =>
      useTelegramRemoteControl("tab-1", true, () => {})
    );

    expect(result.current.isRemoteEnabled).toBe(false);
    expect(listenMock).not.toHaveBeenCalled();
  });

  it("開啟後不會把狀態寫回 localStorage", async () => {
    const { result } = renderHook(() =>
      useTelegramRemoteControl("tab-1", true, () => {})
    );

    act(() => {
      result.current.setIsRemoteEnabled(true);
    });
    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));

    expect(localStorage.getItem("aiterm-remote:tab-1")).toBeNull();
  });
});

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
  it("ownerKey !== remoteOwner 時（沒有人擁有）不註冊 listener", () => {
    renderHook(() => useTelegramRemoteControl("tab-1", null, () => {}, () => {}));

    expect(listenMock).not.toHaveBeenCalled();
  });

  it("ownerKey === remoteOwner 時會註冊 listener", async () => {
    const { result, rerender } = renderHook(
      ({ remoteOwner }: { remoteOwner: string | null }) =>
        useTelegramRemoteControl("tab-1", remoteOwner, () => {}, () => {}),
      { initialProps: { remoteOwner: null as string | null } },
    );

    expect(result.current.isRemoteEnabled).toBe(false);
    rerender({ remoteOwner: "tab-1" });

    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));
    expect(listenMock).toHaveBeenCalledWith(
      "telegram-message-received",
      expect.any(Function)
    );
  });

  it("從擁有變成不擁有時會 unlisten", async () => {
    const { rerender } = renderHook(
      ({ remoteOwner }: { remoteOwner: string | null }) =>
        useTelegramRemoteControl("tab-1", remoteOwner, () => {}, () => {}),
      { initialProps: { remoteOwner: "tab-1" as string | null } },
    );

    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));
    expect(unlistenSpy).not.toHaveBeenCalled();

    rerender({ remoteOwner: null });

    await waitFor(() => expect(unlistenSpy).toHaveBeenCalledTimes(1));
  });

  it("toggleRemote：未擁有時要求成為擁有者，擁有時要求關閉（傳 null）", () => {
    const onRemoteOwnerChange = vi.fn();
    const { result, rerender } = renderHook(
      ({ remoteOwner }: { remoteOwner: string | null }) =>
        useTelegramRemoteControl("tab-1", remoteOwner, onRemoteOwnerChange, () => {}),
      { initialProps: { remoteOwner: null as string | null } },
    );

    act(() => {
      result.current.toggleRemote();
    });
    expect(onRemoteOwnerChange).toHaveBeenLastCalledWith("tab-1");

    rerender({ remoteOwner: "tab-1" });
    act(() => {
      result.current.toggleRemote();
    });
    expect(onRemoteOwnerChange).toHaveBeenLastCalledWith(null);
  });

  // 這是這次修的核心：兩個實例（例如一個終端機分頁、一個資料庫分頁）用不同
  // ownerKey 呼叫這個 hook，同一個 remoteOwner 只能讓其中一個判定自己可以
  // 監聽。舊介面用各呼叫端自己傳的 canListen（有的傳可見性、有的傳互斥 id）
  // 保證這件事，四個呼叫端不一致，於是能同時為真——同一則 Telegram 指令被
  // 執行兩次。新介面把唯一性收斂成單一推導公式，結構上不可能同時為真。
  it("兩個不同 ownerKey 的實例，同一個 remoteOwner 下只有一個會註冊", async () => {
    renderHook(() => useTelegramRemoteControl("tab-1", "tab-1", () => {}, () => {}));
    renderHook(() => useTelegramRemoteControl("tab-2", "tab-1", () => {}, () => {}));

    // 給 tab-2（不該註冊的那個）足夠的 microtask 機會，確認不是「還沒
    // 註冊」而是「不會註冊」。
    await waitFor(() => expect(listenMock).toHaveBeenCalledTimes(1));
    await Promise.resolve();
    expect(listenMock).toHaveBeenCalledTimes(1);
  });

  it("isRemoteEnabled 不會從 localStorage 讀回來（刻意不持久化）", () => {
    // 舊的（已拿掉的）持久化邏輯會用 `aiterm-remote:${tabId}` 當 key。
    // 就算 localStorage 裡有 "true"，開機當下也必須是 false——因為
    // isRemoteEnabled 完全由 remoteOwner（同樣不持久化）推導，沒有自己的
    // state 可以持久化。
    localStorage.setItem("aiterm-remote:tab-1", "true");

    const { result } = renderHook(() =>
      useTelegramRemoteControl("tab-1", null, () => {}, () => {})
    );

    expect(result.current.isRemoteEnabled).toBe(false);
    expect(listenMock).not.toHaveBeenCalled();
  });
});

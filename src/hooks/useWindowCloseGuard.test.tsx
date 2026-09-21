import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import type { BusyTab } from "../lib/busyProbe";

// 捕捉 hook 註冊的兩個入口，測試手動觸發。
let closeCb: ((e: { preventDefault: () => void }) => void) | undefined;
let quitCb: (() => void) | undefined;
const destroy = vi.fn(() => Promise.resolve());
const unlistenClose = vi.fn();
const unlistenQuit = vi.fn();
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: (cb: typeof closeCb) => { closeCb = cb; return Promise.resolve(unlistenClose); },
    destroy: () => destroy(),
  }),
}));
vi.mock("../ipc/quit", () => ({
  QUIT_REQUESTED_EVENT: "app://quit-requested",
  onQuitRequested: (cb: () => void) => { quitCb = cb; return Promise.resolve(unlistenQuit); },
}));

import { useWindowCloseGuard } from "./useWindowCloseGuard";

const busyTab: BusyTab = { tabId: "t1", title: "Terminal", reason: "command" };

beforeEach(() => {
  closeCb = undefined;
  quitCb = undefined;
  destroy.mockClear();
  unlistenClose.mockClear();
  unlistenQuit.mockClear();
});

async function mount(getBusyTabs: () => BusyTab[]) {
  const hook = renderHook(() => useWindowCloseGuard(getBusyTabs));
  await act(async () => {}); // 讓 listen 的 promise resolve
  return hook;
}

function fireClose() {
  const preventDefault = vi.fn();
  return { preventDefault, run: () => act(async () => { closeCb!({ preventDefault }); }) };
}

describe("useWindowCloseGuard", () => {
  it("全部閒置：攔下原生關閉、自己 destroy，不出現確認狀態", async () => {
    // quit() 內部把任何失敗都吞成 console.error 繼續 destroy，所以光看 destroy 有沒有被呼叫
    // 分辨不出「乾淨路徑」與「中間某步悄悄失敗」——要一併斷言沒有錯誤被記錄。
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const { result } = await mount(() => []);
    const ev = fireClose();
    await ev.run();
    expect(err).not.toHaveBeenCalled();
    err.mockRestore();

    expect(ev.preventDefault).toHaveBeenCalled(); // 由我們自己 destroy，不讓預設流程跑第二次
    expect(destroy).toHaveBeenCalledTimes(1);
    expect(result.current.pending).toBeNull();
  });

  it("有忙碌分頁：不 destroy，pending 帶出清單", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();

    expect(destroy).not.toHaveBeenCalled();
    expect(result.current.pending).toEqual([busyTab]);
  });

  it("確認：destroy 視窗", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();
    await act(async () => { result.current.confirm(); });

    expect(destroy).toHaveBeenCalledTimes(1);
  });

  it("取消：清掉 pending，且之後可以再次觸發關閉", async () => {
    const { result } = await mount(() => [busyTab]);
    await fireClose().run();
    act(() => result.current.cancel());
    expect(result.current.pending).toBeNull();

    await fireClose().run();
    expect(result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("重入：確認框已顯示時再次觸發，不覆蓋既有清單", async () => {
    let tabs: BusyTab[] = [busyTab];
    const { result } = await mount(() => tabs);
    await fireClose().run();
    const first = result.current.pending;

    tabs = [{ tabId: "t2", title: "Loop", reason: "loop" }];
    await fireClose().run();
    expect(result.current.pending).toBe(first);
  });

  it("Cmd+Q 入口（後端事件）走同一條路：忙碌時出現確認狀態", async () => {
    const { result } = await mount(() => [busyTab]);
    await act(async () => { quitCb!(); });
    expect(result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("Cmd+Q 入口：閒置時直接 destroy（選單的 Quit 已被換成只發事件，不會自己結束程式）", async () => {
    await mount(() => []);
    await act(async () => { quitCb!(); });
    expect(destroy).toHaveBeenCalledTimes(1);
  });

  // 釘住最容易靜默失效的點：listener 只註冊一次，之後忙碌狀態才變化，仍要看得到。
  it("讀最新的 getBusyTabs（不可閉包捕捉註冊當下的版本）", async () => {
    const hook = renderHook(({ fn }) => useWindowCloseGuard(fn), { initialProps: { fn: (): BusyTab[] => [] } });
    await act(async () => {});
    hook.rerender({ fn: () => [busyTab] });

    await fireClose().run();
    expect(hook.result.current.pending).toEqual([busyTab]);
    expect(destroy).not.toHaveBeenCalled();
  });

  it("unmount 解除兩個監聽", async () => {
    const { unmount } = await mount(() => []);
    unmount();
    await act(async () => {});
    expect(unlistenClose).toHaveBeenCalledTimes(1);
    expect(unlistenQuit).toHaveBeenCalledTimes(1);
  });
});

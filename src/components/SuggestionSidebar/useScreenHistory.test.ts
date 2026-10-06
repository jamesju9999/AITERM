import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";

let screenText: string | null = "";
const serializeTerminal = vi.fn((_id: string) => screenText);
vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: (id: string) => serializeTerminal(id),
}));

import { useScreenHistory } from "./useScreenHistory";

const advance = (ms: number) => act(async () => { await vi.advanceTimersByTimeAsync(ms); });
const lines = (tag: string, n = 8) => Array.from({ length: n }, (_, i) => `${tag}-${i}`).join("\n");

beforeEach(() => {
  vi.useFakeTimers();
  serializeTerminal.mockClear();
  screenText = lines("A");
});
afterEach(() => vi.useRealTimers());

/** 模擬一輪：忙碌 busyMs 之後轉閒置。 */
async function round(idleRef: { v: number }, busyMs = 2000) {
  idleRef.v = 100; await advance(busyMs);
  idleRef.v = 60_000; await advance(1000);
}

describe("useScreenHistory", () => {
  it("records the settled screen each time the terminal goes from busy to idle while an AI tool runs", async () => {
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    expect(result.current.getHistory()).toEqual([]);
    screenText = lines("A"); await round(idle);
    screenText = lines("B"); await round(idle);
    const h = result.current.getHistory();
    expect(h).toHaveLength(2);
    expect(h[0]).toContain("A-0");
    expect(h[1]).toContain("B-0");
    expect(serializeTerminal).toHaveBeenCalledWith("s1");
  });

  it("does not record while no AI tool is running", async () => {
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", false, () => idle.v));
    await round(idle);
    expect(result.current.getHistory()).toEqual([]);
    expect(serializeTerminal).not.toHaveBeenCalled();
  });

  it("does not record when the terminal stays idle (nothing happened)", async () => {
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    await advance(5000);
    expect(result.current.getHistory()).toEqual([]);
  });

  it("reads the screen once per busy-to-idle transition, not on every idle poll afterwards", async () => {
    const idle = { v: 60_000 };
    renderHook(() => useScreenHistory("s1", true, () => idle.v));
    await round(idle);
    serializeTerminal.mockClear();
    await advance(10_000); // 一直閒置
    expect(serializeTerminal).not.toHaveBeenCalled();
  });

  it("keeps recording when the sidebar is closed — the hook has no dependency on it", async () => {
    // 這個 hook 只吃 aiCliRunning／getIdleMs，與側欄是否開啟無關；這裡確認不需要任何側欄狀態就能記錄。
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    await round(idle);
    expect(result.current.getHistory()).toHaveLength(1);
  });

  it("merges a redraw of the same screen instead of piling up copies", async () => {
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    screenText = lines("A"); await round(idle);
    screenText = lines("A") + "\nA-extra"; await round(idle);
    expect(result.current.getHistory()).toHaveLength(1);
  });

  it("clears the history when the AI tool ends, and starts fresh for the next one", async () => {
    const idle = { v: 60_000 };
    const { result, rerender } = renderHook(({ running }) => useScreenHistory("s1", running, () => idle.v), {
      initialProps: { running: true },
    });
    await round(idle);
    expect(result.current.getHistory()).toHaveLength(1);
    rerender({ running: false });
    expect(result.current.getHistory()).toEqual([]);
    rerender({ running: true });
    screenText = lines("Z"); await round(idle);
    const h = result.current.getHistory();
    expect(h).toHaveLength(1);
    expect(h[0]).toContain("Z-0");
  });

  it("clears the history when the session (tab) changes", async () => {
    const idle = { v: 60_000 };
    const { result, rerender } = renderHook(({ sid }) => useScreenHistory(sid, true, () => idle.v), {
      initialProps: { sid: "s1" },
    });
    await round(idle);
    expect(result.current.getHistory()).toHaveLength(1);
    rerender({ sid: "s2" });
    expect(result.current.getHistory()).toEqual([]);
  });

  it("ignores a missing screen (terminal not registered)", async () => {
    const idle = { v: 60_000 };
    const { result } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    screenText = null; await round(idle);
    expect(result.current.getHistory()).toEqual([]);
  });

  it("stops polling on unmount", async () => {
    const idle = { v: 60_000 };
    const { unmount } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    unmount();
    serializeTerminal.mockClear();
    await round(idle);
    expect(serializeTerminal).not.toHaveBeenCalled();
  });

  it("returns a stable getHistory function", () => {
    const idle = { v: 60_000 };
    const { result, rerender } = renderHook(() => useScreenHistory("s1", true, () => idle.v));
    const first = result.current.getHistory;
    rerender();
    expect(result.current.getHistory).toBe(first);
  });
});

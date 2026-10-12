import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const setLabels = vi.fn((_a: string, _b: string, _c: string) => Promise.resolve());
vi.mock("../ipc/resident", () => ({
  setTrayLabels: (a: string, b: string, c: string) => setLabels(a, b, c),
}));

import { LocaleProvider, useLocale } from "../contexts/LocaleContext";
import { useTrayLabels } from "./useTrayLabels";

beforeEach(() => {
  setLabels.mockClear();
  localStorage.clear();
});

function useBoth() {
  useTrayLabels();
  return useLocale();
}

describe("useTrayLabels", () => {
  it("掛載時把目前語系的三個托盤選單文字送給後端", async () => {
    renderHook(() => useTrayLabels(), { wrapper: LocaleProvider });
    await act(async () => {});
    expect(setLabels).toHaveBeenCalledTimes(1);
    expect(setLabels).toHaveBeenCalledWith("顯示 AITerm", "新分頁", "結束");
  });

  it("切換語系後重新送出新的文字", async () => {
    const { result } = renderHook(() => useBoth(), { wrapper: LocaleProvider });
    await act(async () => {});
    await act(async () => { result.current.setLocale("en"); });
    expect(setLabels).toHaveBeenLastCalledWith("Show AITerm", "New Tab", "Quit");
    expect(setLabels).toHaveBeenCalledTimes(2);
  });

  it("後端失敗（例如非桌面環境、托盤沒建成）不丟例外", async () => {
    setLabels.mockRejectedValueOnce("no tray");
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    renderHook(() => useTrayLabels(), { wrapper: LocaleProvider });
    await act(async () => {});
    expect(err).not.toHaveBeenCalled(); // 靜默吞掉：托盤標籤只是錦上添花
    err.mockRestore();
  });
});

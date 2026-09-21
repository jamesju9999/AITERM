import { describe, it, expect, vi } from "vitest";
import { collectBusyTabs, type BusyProbe } from "./busyProbe";

const titles: Record<string, string> = { a: "Terminal", b: "Loop Studio" };
const titleOf = (id: string) => titles[id];

describe("collectBusyTabs", () => {
  it("全部閒置：回空陣列", () => {
    const probes = new Map<string, BusyProbe>([["a", () => null], ["b", () => null]]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([]);
  });

  it("只列出忙碌的分頁，順序為註冊順序，附標題與原因", () => {
    const probes = new Map<string, BusyProbe>([
      ["a", () => "command"],
      ["b", () => null],
      ["c", () => "loop"],
    ]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([
      { tabId: "a", title: "Terminal", reason: "command" },
      { tabId: "c", title: "c", reason: "loop" }, // 查不到標題 → 退回 tabId
    ]);
  });

  it("探針丟例外：視為閒置並印警告，不能讓整個關視窗流程壞掉", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const probes = new Map<string, BusyProbe>([
      ["a", () => { throw new Error("boom"); }],
      ["b", () => "streaming"],
    ]);
    expect(collectBusyTabs(probes, titleOf)).toEqual([
      { tabId: "b", title: "Loop Studio", reason: "streaming" },
    ]);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});

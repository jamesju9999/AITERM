import { describe, expect, it } from "vitest";
import { setTabAgentProgress } from "./tabAgentProgress";
import type { Tab } from "../components/TabBar";

const tabs: Tab[] = [
  { id: "a", title: "A", type: "terminal", agentProgress: { done: 1, total: 3 } },
  { id: "b", title: "B", type: "terminal" },
];

describe("setTabAgentProgress", () => {
  it("只更新符合 tabId 的那個分頁，其餘分頁原封不動", () => {
    const result = setTabAgentProgress(tabs, "b", { done: 2, total: 5 });
    expect(result.find((t) => t.id === "a")).toEqual(tabs[0]);
    expect(result.find((t) => t.id === "b")?.agentProgress).toEqual({ done: 2, total: 5 });
  });

  it("傳 undefined 會清掉該分頁的 agentProgress", () => {
    const result = setTabAgentProgress(tabs, "a", undefined);
    expect(result.find((t) => t.id === "a")?.agentProgress).toBeUndefined();
    // 其餘分頁不受影響
    expect(result.find((t) => t.id === "b")).toEqual(tabs[1]);
  });

  it("找不到符合的 tabId 時，回傳的陣列跟原本內容相等", () => {
    const result = setTabAgentProgress(tabs, "nonexistent", { done: 9, total: 9 });
    expect(result).toEqual(tabs);
  });
});

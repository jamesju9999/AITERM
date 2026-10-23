import { describe, expect, it } from "vitest";
import { autoplayDelayMs, clampStep, stepBounds, tourEdges } from "./tour";
import type { Architecture, Edge } from "./model";
import type { TourStep } from "./qa";

const e = (from: string, to: string, kind: Edge["kind"] = "import"): Edge => ({ from, to, kind, source: "scan", confidence: 1 });
const view: Architecture = {
  layers: [], modules: [],
  edges: [e("a", "b"), e("c", "b", "ipc"), e("b", "a", "http"), e("a", "d")],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};
const steps = (...m: string[]): TourStep[] => m.map((module) => ({ module, caption: "x" }));

describe("tourEdges", () => {
  it("正向與反向的邊都回傳（多條全回）", () => {
    expect(tourEdges(view, steps("a", "b"), 0)).toEqual([view.edges[0], view.edges[2]]);
  });
  it("只有反向邊也算", () => {
    expect(tourEdges(view, steps("b", "c"), 0)).toEqual([view.edges[1]]);
  });
  it("沒有直接邊回空陣列（不走間接路徑）", () => {
    expect(tourEdges(view, steps("c", "d"), 0)).toEqual([]);
  });
  it("連續兩步同一模組回空陣列", () => {
    expect(tourEdges(view, steps("a", "a"), 0)).toEqual([]);
  });
  it("最後一步與越界回空陣列", () => {
    expect(tourEdges(view, steps("a", "b"), 1)).toEqual([]);
    expect(tourEdges(view, steps("a", "b"), 5)).toEqual([]);
    expect(tourEdges(view, steps("a", "b"), -1)).toEqual([]);
  });
  it("空步驟回空陣列", () => {
    expect(tourEdges(view, [], 0)).toEqual([]);
  });
  it("只看 index 與 index+1", () => {
    expect(tourEdges(view, steps("c", "a", "d"), 1)).toEqual([view.edges[3]]);
  });
});

describe("stepBounds / clampStep", () => {
  it("首尾旗標", () => {
    expect(stepBounds(steps("a", "b", "c"), 0)).toEqual({ first: true, last: false });
    expect(stepBounds(steps("a", "b", "c"), 1)).toEqual({ first: false, last: false });
    expect(stepBounds(steps("a", "b", "c"), 2)).toEqual({ first: false, last: true });
  });
  it("單步同時是首尾；空步驟也同時是首尾", () => {
    expect(stepBounds(steps("a"), 0)).toEqual({ first: true, last: true });
    expect(stepBounds([], 0)).toEqual({ first: true, last: true });
  });
  it("clampStep 夾進範圍，空步驟為 0", () => {
    expect(clampStep(steps("a", "b", "c"), -3)).toBe(0);
    expect(clampStep(steps("a", "b", "c"), 1)).toBe(1);
    expect(clampStep(steps("a", "b", "c"), 9)).toBe(2);
    expect(clampStep([], 4)).toBe(0);
  });
  it("自動播放間隔 3 秒", () => {
    expect(autoplayDelayMs).toBe(3000);
  });
});

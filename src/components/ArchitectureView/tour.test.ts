import { describe, expect, it } from "vitest";
import { autoplayDelayMs, clampStep, stepBounds, tourEdges, tourReversedEdges, tourRoute } from "./tour";
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
  it("沒有直接邊時沿既有連線走最短路：c→b→a→d 每一段都回傳", () => {
    expect(tourEdges(view, steps("c", "d"), 0)).toEqual([view.edges[1], view.edges[0], view.edges[2], view.edges[3]]);
  });
  it("兩個模組在圖上不相通時回空陣列", () => {
    expect(tourEdges(view, steps("a", "z"), 0)).toEqual([]);
    expect(tourRoute(view, steps("a", "z"), 0).modules).toEqual([]);
    expect(tourRoute(view, steps("a", "z"), 0).gap).toEqual({ from: "a", to: "z" });
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

describe("tourRoute", () => {
  it("有路（直接或間接）、同模組、最後一步時都沒有 gap", () => {
    expect(tourRoute(view, steps("a", "b"), 0).gap).toBeUndefined();
    expect(tourRoute(view, steps("c", "d"), 0).gap).toBeUndefined();
    expect(tourRoute(view, steps("a", "a"), 0).gap).toBeUndefined();
    expect(tourRoute(view, steps("a", "b"), 1).gap).toBeUndefined();
  });
  it("modules 含起訖與沿途模組，直接連線就只有兩個", () => {
    expect(tourRoute(view, steps("a", "b"), 0).modules).toEqual(["a", "b"]);
    expect(tourRoute(view, steps("c", "d"), 0).modules).toEqual(["c", "b", "a", "d"]);
  });
});

describe("tourReversedEdges", () => {
  it("這一步走的方向與連線方向相反（資料回傳）時，該連線標為反向", () => {
    // 連線是 a→b；導覽從 b 回到 a
    expect([...tourReversedEdges(view, steps("b", "a"), 0)]).toEqual([view.edges[0]]);
  });
  it("方向一致時不標；兩個方向的連線都存在時只有反向那條被標", () => {
    expect(tourReversedEdges(view, steps("a", "d"), 0).size).toBe(0);
    // a→b 與 b→a 都有：從 a 走到 b，只有 b→a 是反向
    expect([...tourReversedEdges(view, steps("a", "b"), 0)]).toEqual([view.edges[2]]);
  });
  it("只有反向邊（c→b）而導覽從 b 到 c 時標為反向", () => {
    expect([...tourReversedEdges(view, steps("b", "c"), 0)]).toEqual([view.edges[1]]);
  });
  it("間接路徑：每一段各自判斷方向（c→b 與行進方向一致、a→b 與行進方向相反）", () => {
    // c 走到 d 的路是 c→b→a→d：c→b 順向；b→a(edges[2]) 順向；a→b(edges[0]) 逆向；a→d 順向
    expect([...tourReversedEdges(view, steps("c", "d"), 0)]).toEqual([view.edges[0]]);
  });
  it("沒有連線或最後一步時為空", () => {
    expect(tourReversedEdges(view, steps("a", "z"), 0).size).toBe(0);
    expect(tourReversedEdges(view, steps("a", "b"), 1).size).toBe(0);
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

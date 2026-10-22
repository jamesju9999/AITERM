import { describe, expect, it } from "vitest";
import {
  layoutGraph, edgePath, edgeMidpoint, fitScale, layerColor, traceChain,
  NODE_W, NODE_H, COL_GAP, ROW_GAP, PAD, LANE_PAD, LANE_W, HEADER_H, EDGE_BULGE, MIN_SCALE, LAYER_PALETTE,
} from "./layout";
import type { Architecture } from "./model";

const arch: Architecture = {
  layers: [{ id: "a", name: "A" }, { id: "b", name: "B" }],
  modules: [
    { id: "m1", layer: "a", name: "m1", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m2", layer: "a", name: "m2", files: [], tech: [], summary: "", dataflow: "" },
    { id: "m3", layer: "b", name: "m3", files: [], tech: [], summary: "", dataflow: "" },
  ],
  edges: [
    { from: "m1", to: "m3", kind: "http", source: "ai", confidence: 0.8 },
    { from: "m2", to: "m3", kind: "import", source: "scan", confidence: 1 },
  ],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

describe("layoutGraph", () => {
  it("每層一條泳道、卡片在泳道內由上往下排列", () => {
    const l = layoutGraph(arch);
    const laneX0 = PAD;
    expect(l.nodes.m1).toEqual({ x: laneX0 + LANE_PAD, y: PAD + HEADER_H });
    expect(l.nodes.m2).toEqual({ x: laneX0 + LANE_PAD, y: PAD + HEADER_H + NODE_H + ROW_GAP });
    expect(l.nodes.m3.x).toBe(PAD + LANE_W + COL_GAP + LANE_PAD);
    expect(l.columns).toHaveLength(2);
    expect(l.columns[0]).toMatchObject({ layerId: "a", x: laneX0, y: PAD, w: LANE_W, count: 2 });
    expect(l.columns[1].x).toBe(PAD + LANE_W + COL_GAP);
  });

  it("所有泳道等高，並完整包住最長那一欄；畫布寬高含外距", () => {
    const l = layoutGraph(arch);
    const laneH = HEADER_H + 2 * NODE_H + ROW_GAP + LANE_PAD;
    expect(l.columns.every((c) => c.h === laneH)).toBe(true);
    expect(l.nodes.m2.y + NODE_H + LANE_PAD).toBe(l.columns[0].y + laneH);
    expect(l.width).toBe(PAD * 2 + 2 * LANE_W + COL_GAP);
    expect(l.height).toBe(PAD * 2 + laneH);
  });
});

describe("fitScale", () => {
  it("放得下時不縮放", () => {
    expect(fitScale(800, 1000)).toBe(1);
    expect(fitScale(1000, 1000)).toBe(1);
  });
  it("太寬時依比例縮小，但不低於 0.6", () => {
    expect(fitScale(1000, 800)).toBeCloseTo(0.8);
    expect(fitScale(2000, 800)).toBe(MIN_SCALE);
    expect(MIN_SCALE).toBe(0.6);
  });
  it("量不到寬度時回傳 1", () => {
    expect(fitScale(1000, 0)).toBe(1);
    expect(fitScale(0, 500)).toBe(1);
  });
});

describe("layerColor", () => {
  it("已知分層有固定顏色，彼此不同", () => {
    const ids = ["frontend", "transport", "backend", "data", "other"];
    const cs = ids.map((id, i) => layerColor(id, i));
    expect(new Set(cs).size).toBe(ids.length);
    expect(layerColor("frontend", 9)).toBe(layerColor("frontend", 0));
  });
  it("未知分層依序號取調色盤並循環", () => {
    expect(layerColor("x", 0)).toBe(layerColor("y", 0));
    expect(layerColor("x", 1)).not.toBe(layerColor("x", 0));
    expect(layerColor("x", LAYER_PALETTE.length)).toBe(layerColor("x", 0));
  });
});

describe("edgePath", () => {
  const A = { x: 100, y: 100 };
  it("不同欄且目標在右：來源右緣中點 → 目標左緣中點", () => {
    const B = { x: 100 + LANE_W + COL_GAP, y: 300 };
    const d = edgePath(A, B);
    expect(d.startsWith(`M ${A.x + NODE_W} ${A.y + NODE_H / 2} C`)).toBe(true);
    expect(d.endsWith(`${B.x} ${B.y + NODE_H / 2}`)).toBe(true);
  });
  it("目標在左邊：來源左緣 → 目標右緣", () => {
    const B = { x: A.x - LANE_W - COL_GAP, y: 300 };
    const d = edgePath(A, B);
    expect(d.startsWith(`M ${A.x} ${A.y + NODE_H / 2} C`)).toBe(true);
    expect(d.endsWith(`${B.x + NODE_W} ${B.y + NODE_H / 2}`)).toBe(true);
  });
  it("同一欄：兩端都在右緣，並向右凸出到泳道間隙而不穿過卡片", () => {
    const B = { x: A.x, y: A.y + NODE_H + ROW_GAP };
    const d = edgePath(A, B);
    const right = A.x + NODE_W;
    expect(d.startsWith(`M ${right} ${A.y + NODE_H / 2} C`)).toBe(true);
    expect(d.endsWith(`${right} ${B.y + NODE_H / 2}`)).toBe(true);
    const nums = d.match(/-?\d+(\.\d+)?/g)!.map(Number);
    const xs = nums.filter((_, i) => i % 2 === 0);
    expect(Math.max(...xs)).toBe(right + EDGE_BULGE);
    expect(xs.every((x) => x >= right)).toBe(true);
  });
});

describe("edgeMidpoint", () => {
  it("不同欄：落在兩端點正中", () => {
    const A = { x: 100, y: 100 };
    const B = { x: 100 + LANE_W + COL_GAP, y: 300 };
    const m = edgeMidpoint(A, B);
    expect(m.x).toBeCloseTo((A.x + NODE_W + B.x) / 2);
    expect(m.y).toBeCloseTo((A.y + B.y + NODE_H) / 2);
  });
  it("同一欄：貝茲中點在右緣外 0.75 倍凸出量", () => {
    const A = { x: 100, y: 100 };
    const B = { x: 100, y: 100 + NODE_H + ROW_GAP };
    const m = edgeMidpoint(A, B);
    expect(m.x).toBeCloseTo(A.x + NODE_W + 0.75 * EDGE_BULGE);
    expect(m.y).toBeCloseTo((A.y + B.y + NODE_H) / 2);
  });
});

describe("traceChain", () => {
  it("從選中的模組往上下游走完整條呼叫鏈", () => {
    const chain = traceChain(arch, "m3");
    expect([...chain.modules].sort()).toEqual(["m1", "m2", "m3"]);
    expect(chain.edges).toHaveLength(2);
    expect([...traceChain(arch, "m1").modules].sort()).toEqual(["m1", "m3"]);
  });
});

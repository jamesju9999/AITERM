import { describe, expect, it } from "vitest";
import { layoutGraph, edgePath, traceChain, NODE_W, NODE_H, COL_GAP, ROW_GAP, PAD } from "./layout";
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
  it("每層一欄、層內由上往下排列", () => {
    const l = layoutGraph(arch);
    expect(l.nodes.m1).toEqual({ x: PAD, y: PAD + 40 });
    expect(l.nodes.m2).toEqual({ x: PAD, y: PAD + 40 + NODE_H + ROW_GAP });
    expect(l.nodes.m3.x).toBe(PAD + NODE_W + COL_GAP);
    expect(l.columns).toHaveLength(2);
    expect(l.width).toBeGreaterThan(l.nodes.m3.x + NODE_W);
    expect(l.height).toBeGreaterThan(l.nodes.m2.y + NODE_H);
  });
});

describe("edgePath", () => {
  it("從來源右緣到目標左緣畫三次貝茲曲線", () => {
    const l = layoutGraph(arch);
    const d = edgePath(l.nodes.m1, l.nodes.m3);
    expect(d.startsWith(`M ${l.nodes.m1.x + NODE_W} ${l.nodes.m1.y + NODE_H / 2} C`)).toBe(true);
    expect(d.endsWith(`${l.nodes.m3.x} ${l.nodes.m3.y + NODE_H / 2}`)).toBe(true);
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

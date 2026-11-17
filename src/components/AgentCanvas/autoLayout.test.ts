import { describe, expect, it } from "vitest";
import { autoLayout, type LayoutNode } from "./autoLayout";

const n = (id: string, kind: LayoutNode["kind"], y = 0, h = 100): LayoutNode => ({ id, kind, x: 0, y, w: 250, h });

describe("autoLayout", () => {
  it("依流程由左到右：工程師 → 審核者 → 協調者 → 驗收者", () => {
    const pos = autoLayout([n("v", "verifier"), n("o", "orchestrator"), n("r", "reviewer"), n("c", "coder")]);
    const xs = ["c", "r", "o", "v"].map((id) => pos.get(id)!.x);
    expect(xs).toEqual([...xs].sort((a, b) => a - b));
    expect(new Set(xs).size).toBe(4);
  });

  it("沒放的角色不留空欄", () => {
    const pos = autoLayout([n("c", "coder"), n("o", "orchestrator")]);
    expect(pos.get("o")!.x - pos.get("c")!.x).toBe(250 + 110);
  });

  it("同一欄依原本的上下順序排，不重疊", () => {
    const pos = autoLayout([n("c2", "coder", 500, 180), n("c1", "coder", 10, 120)]);
    expect(pos.get("c1")!.y).toBeLessThan(pos.get("c2")!.y);
    expect(pos.get("c2")!.y).toBeGreaterThanOrEqual(pos.get("c1")!.y + 120);
  });

  it("矮的欄對齊最高那欄的垂直中心", () => {
    const pos = autoLayout([n("c1", "coder", 0, 100), n("c2", "coder", 1, 100), n("r", "reviewer", 0, 100)]);
    const coderMid = (pos.get("c1")!.y + pos.get("c2")!.y + 100) / 2;
    expect(pos.get("r")!.y + 50).toBe(coderMid);
  });

  it("產出規格的工程師排在最左邊一欄，其他工程師往右一欄", () => {
    const spec = { ...n("s", "coder", 300), spec: true };
    const pos = autoLayout([n("c1", "coder", 0), spec, n("c2", "coder", 200), n("r", "reviewer")]);
    expect(pos.get("s")!.x).toBeLessThan(pos.get("c1")!.x);
    expect(pos.get("c1")!.x).toBe(pos.get("c2")!.x);
    expect(pos.get("r")!.x).toBeGreaterThan(pos.get("c1")!.x);
  });

  it("定時器排在最左一欄，不蓋到任何角色（回歸：自動排列後被其他角色蓋住）", () => {
    const nodes: LayoutNode[] = [
      n("c1", "coder", 0, 200),
      n("c2", "coder", 300, 200),
      n("r", "reviewer", 0, 100),
      { id: "s1", kind: "schedule", x: 0, y: 0, w: 200, h: 90 },
      { id: "s2", kind: "schedule", x: 0, y: 10, w: 200, h: 90 },
    ];
    const pos = autoLayout(nodes);
    const rect = (id: string) => {
      const node = nodes.find((x) => x.id === id)!;
      const p = pos.get(id)!;
      return { l: p.x, t: p.y, r: p.x + node.w, b: p.y + node.h };
    };
    for (const a of nodes) {
      for (const b of nodes) {
        if (a.id >= b.id) continue;
        const A = rect(a.id);
        const B = rect(b.id);
        const overlap = A.l < B.r && B.l < A.r && A.t < B.b && B.t < A.b;
        expect(overlap, `${a.id} 與 ${b.id} 重疊`).toBe(false);
      }
    }
    expect(pos.get("s1")!.x).toBeLessThan(pos.get("c1")!.x);
    expect(pos.get("s1")!.y).toBeLessThan(pos.get("s2")!.y);
  });
});

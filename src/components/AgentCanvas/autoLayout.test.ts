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
});

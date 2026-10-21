import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { CARD_H, CARD_W, GAP_X, GAP_Y, PAD, ROOT_ID, ROOT_W, elbowPath, layoutGraph } from "./graphLayout";

const n = (id: string, parent: string | null, started = 0): ActivityNode => ({
  id, parent, agent_type: "general-purpose", description: null, status: "running",
  started_ms: started, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: 0, turn: 1,
});
const pos = (g: ReturnType<typeof layoutGraph>, id: string) => g.boxes.get(id)!;

describe("layoutGraph", () => {
  it("沒有節點：只有根，圖有一個卡片的大小", () => {
    const g = layoutGraph([]);
    expect([...g.boxes.keys()]).toEqual([ROOT_ID]);
    expect(g.edges).toEqual([]);
    expect(g.width).toBe(PAD * 2 + ROOT_W);
    expect(g.height).toBe(PAD * 2 + CARD_H);
  });

  it("subagent 接在根後面，每往下一層就往右一欄", () => {
    const g = layoutGraph([n("a", null, 1), n("a1", "a", 2)]);
    expect(pos(g, ROOT_ID).x).toBe(PAD);
    expect(pos(g, "a").x).toBe(PAD + ROOT_W + GAP_X);
    expect(pos(g, "a1").x).toBe(PAD + ROOT_W + GAP_X + (CARD_W + GAP_X));
    expect(g.width).toBe(PAD * 2 + ROOT_W + 2 * (GAP_X + CARD_W));
    expect(g.edges.map((e) => [e.from, e.to])).toEqual([[ROOT_ID, "a"], ["a", "a1"]]);
  });

  it("同一層由上往下依開始順序排，父節點垂直置中在子節點中間", () => {
    const g = layoutGraph([n("b", null, 2), n("a", null, 1), n("a1", "a", 3), n("a2", "a", 4)]);
    const row = (id: string) => (pos(g, id).y - PAD) / (CARD_H + GAP_Y);
    expect(row("a1")).toBe(0);
    expect(row("a2")).toBe(1);
    expect(row("a")).toBe(0.5);
    expect(row("b")).toBe(2);
    expect(row(ROOT_ID)).toBe(1.25);
    expect(g.height).toBe(PAD * 2 + 3 * CARD_H + 2 * GAP_Y);
  });

  it("同一層失敗的排在最前面，其餘依開始順序", () => {
    const failed = { ...n("f", null, 9), status: "failed" as const };
    const g = layoutGraph([n("a", null, 1), n("b", null, 2), failed]);
    const row = (id: string) => (pos(g, id).y - PAD) / (CARD_H + GAP_Y);
    expect([row("f"), row("a"), row("b")]).toEqual([0, 1, 2]);
  });

  it("父節點不存在或循環參照的節點接到根，不消失、不無限迴圈", () => {
    const g = layoutGraph([n("orphan", "gone", 1), n("x", "y", 2), n("y", "x", 3)]);
    expect([...g.boxes.keys()].sort()).toEqual([ROOT_ID, "orphan", "x", "y"].sort());
    expect(g.edges.filter((e) => e.from === ROOT_ID).map((e) => e.to).sort()).toEqual(["orphan", "x"]);
  });

  it("連線的狀態跟著子節點（流動動畫看這個）", () => {
    const done = { ...n("a", null, 1), status: "done" as const };
    const g = layoutGraph([done, n("b", null, 2)]);
    expect(g.edges.find((e) => e.to === "a")!.status).toBe("done");
    expect(g.edges.find((e) => e.to === "b")!.status).toBe("running");
  });

  it("預設寬度的側欄（340px）放得下根＋第一層 subagent，不必橫向捲動", () => {
    expect(layoutGraph([n("a", null, 1), n("b", null, 2)]).width).toBeLessThanOrEqual(340);
  });

  it("連線路徑從父卡右緣中點到子卡左緣中點", () => {
    const g = layoutGraph([n("a", null, 1)]);
    const e = g.edges[0];
    const r = pos(g, ROOT_ID);
    const a = pos(g, "a");
    expect(e.d.startsWith(`M ${r.x + ROOT_W} ${r.y + CARD_H / 2}`)).toBe(true);
    expect(e.d.endsWith(`${a.x} ${a.y + CARD_H / 2}`)).toBe(true);
  });
});

describe("layoutGraph：單行工具卡比父卡矮時不重疊", () => {
  it("每個只掛一個工具的 subagent 之間仍保持間距", () => {
    const mk = (id: string, parent: string | null, ms: number): ActivityNode => ({
      id, parent, agent_type: "general-purpose", description: id, status: "done", started_ms: ms, ended_ms: ms + 1,
      current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: ms, turn: 1,
    });
    const g = layoutGraph([
      mk("a", null, 1), mk("tool:a1", "a", 2),
      mk("b", null, 3), mk("tool:b1", "b", 4),
      mk("c", null, 5), mk("tool:c1", "c", 6),
    ]);
    const ys = ["a", "b", "c"].map((id) => g.boxes.get(id)!.y);
    expect(ys[1] - ys[0]).toBeGreaterThanOrEqual(CARD_H + GAP_Y);
    expect(ys[2] - ys[1]).toBeGreaterThanOrEqual(CARD_H + GAP_Y);
  });
});

describe("elbowPath", () => {
  it("同一高度走直線，不同高度走圓角折線並抵達終點", () => {
    expect(elbowPath(0, 10, 50, 10)).toBe("M 0 10 L 50 10");
    const d = elbowPath(0, 0, 60, 40);
    expect(d.startsWith("M 0 0")).toBe(true);
    expect(d).toContain("Q");
    expect(d.endsWith("L 60 40")).toBe(true);
  });
  it("同一父節點的子連線共用同一條垂直主幹", () => {
    const trunk = (d: string) => d.match(/Q (\S+) /)![1];
    expect(trunk(elbowPath(10, 50, 70, 10))).toBe(trunk(elbowPath(10, 50, 70, 90)));
  });
});

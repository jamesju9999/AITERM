import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { buildTimeline } from "./timeline";

const n = (id: string, started: number, ended: number | null, parent: string | null = null): ActivityNode => ({
  id, parent, agent_type: "general-purpose", description: id, status: ended == null ? "running" : "done",
  started_ms: started, ended_ms: ended, current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: 0, turn: 1,
});

describe("buildTimeline", () => {
  it("沒有節點：空", () => {
    expect(buildTimeline([], 0)).toEqual({ rows: [], spanMs: 0 });
  });

  it("橫條的位置與寬度是佔整段時間的百分比；平行的條左緣重疊", () => {
    const t = buildTimeline([n("b", 1000, 5000), n("a", 1000, 3000)], 9999);
    expect(t.spanMs).toBe(4000);
    const by = Object.fromEntries(t.rows.map((r) => [r.node.id, r]));
    expect(by.a.left).toBe(0);
    expect(by.a.width).toBe(50);
    expect(by.b.left).toBe(0);
    expect(by.b.width).toBe(100);
  });

  it("依開始時間排序；晚開始的條往右", () => {
    // id 的字母序刻意跟時間順序相反：排序鍵若退化成 id，這條就會紅。
    const t = buildTimeline([n("a-late", 3000, 4000), n("z-early", 0, 1000)], 9999);
    expect(t.rows.map((r) => r.node.id)).toEqual(["z-early", "a-late"]);
    expect(t.rows[1].left).toBe(75);
    expect(t.rows[1].width).toBe(25);
  });

  it("執行中的條算到現在，並決定整段時間的長度", () => {
    const t = buildTimeline([n("done", 0, 1000), n("run", 500, null)], 4000);
    expect(t.spanMs).toBe(4000);
    expect(t.rows.find((r) => r.node.id === "run")!.width).toBe(87.5);
  });

  it("極短的條仍有最小寬度，且不會超出右緣", () => {
    const t = buildTimeline([n("long", 0, 100_000), n("blip", 100_000, 100_000)], 0);
    const blip = t.rows.find((r) => r.node.id === "blip")!;
    expect(blip.width).toBeGreaterThanOrEqual(1.5);
    expect(blip.left + blip.width).toBeLessThanOrEqual(100.0001);
  });

  it("巢狀帶縮排深度；孤兒與循環不炸", () => {
    const t = buildTimeline([n("a", 0, 10), n("a1", 1, 5, "a"), n("x", 2, 6, "y"), n("y", 3, 7, "x"), n("o", 4, 8, "gone")], 0);
    const depth = Object.fromEntries(t.rows.map((r) => [r.node.id, r.depth]));
    expect(depth.a).toBe(0);
    expect(depth.a1).toBe(1);
    expect(depth.o).toBe(0);
    expect(t.rows).toHaveLength(5);
  });
});

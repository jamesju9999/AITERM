import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { DONE_GROUP_ID, foldFinished } from "./fold";

const n = (id: string, parent: string | null, status: ActivityNode["status"] = "done", started = 1): ActivityNode => ({
  id, parent, agent_type: "general-purpose", description: id, status, started_ms: started,
  ended_ms: status === "running" ? null : started + 1000, current_tool: null, summary: null, tokens: null, tool_count: null, turn: 1,
});
const label = (c: number) => `已完成 ${c} 個`;

describe("foldFinished", () => {
  it("少於 3 棵完成的子樹就不折", () => {
    const nodes = [n("a", null), n("b", null)];
    expect(foldFinished(nodes, label)).toEqual({ nodes, folded: 0 });
  });

  it("3 棵以上完成的子樹折成一張群組卡，數的是節點數（含巢狀）", () => {
    const r = foldFinished([n("a", null), n("a1", "a"), n("b", null), n("c", null)], label);
    expect(r.folded).toBe(4);
    expect(r.nodes.map((x) => x.id)).toEqual([DONE_GROUP_ID]);
    expect(r.nodes[0].description).toBe("已完成 4 個");
    expect(r.nodes[0].parent).toBeNull();
  });

  it("執行中、失敗、中斷的子樹留著；只有整棵都 done 才折", () => {
    const nodes = [
      n("a", null), n("b", null), n("c", null),
      n("run", null, "running", 5),
      n("bad", null, "failed", 6),
      n("p", null), n("pc", "p", "running", 7), // 父完成、子還在跑：整棵不算完成
    ];
    const r = foldFinished(nodes, label);
    expect(r.folded).toBe(3);
    expect(r.nodes.map((x) => x.id)).toEqual([DONE_GROUP_ID, "run", "bad", "p", "pc"]);
  });

  it("群組卡的時間涵蓋所有被折的節點", () => {
    const r = foldFinished([n("a", null, "done", 10), n("b", null, "done", 20), n("c", null, "done", 30)], label);
    expect(r.nodes[0].started_ms).toBe(10);
    expect(r.nodes[0].ended_ms).toBe(1030);
  });

  it("空陣列不折", () => {
    expect(foldFinished([], label)).toEqual({ nodes: [], folded: 0 });
  });
});

import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { flattenTree } from "./flattenTree";

const n = (id: string, parent: string | null, started = 0): ActivityNode => ({
  id, parent, agent_type: "general-purpose", description: null, status: "running",
  started_ms: started, ended_ms: null, current_tool: null, summary: null, tokens: null, tool_count: null,
});

describe("flattenTree", () => {
  it("把子節點排在父節點底下並帶縮排深度", () => {
    const rows = flattenTree([n("a", null, 1), n("a1", "a", 2), n("b", null, 3), n("a1x", "a1", 4)]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([["a", 0], ["a1", 1], ["a1x", 2], ["b", 0]]);
  });

  it("父節點不存在（被裁掉或事件遺失）就當根節點，不會消失", () => {
    const rows = flattenTree([n("orphan", "gone", 1)]);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([["orphan", 0]]);
  });

  it("循環參照不會無限迴圈，每個節點只出現一次", () => {
    const rows = flattenTree([n("x", "y", 1), n("y", "x", 2)]);
    expect(rows.map((r) => r.node.id).sort()).toEqual(["x", "y"]);
  });

  it("空陣列回空陣列", () => {
    expect(flattenTree([])).toEqual([]);
  });
});

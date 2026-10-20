import { describe, expect, it } from "vitest";
import type { ActivityNode, ToolCall } from "../../ipc/claudeActivity";
import { attachTools, toolsToNodes, TOOL_PREFIX, TOOLS_MORE_PREFIX, TOOL_WINDOW } from "./toolNodes";

const tc = (id: string, started: number, over: Partial<ToolCall> = {}): ToolCall => ({
  id, parent: null, tool: "Bash", hint: `Bash: ${id}`, category: "shell", status: "done",
  started_ms: started, ended_ms: started + 5, turn: 1, ...over,
});
const label = (n: number) => `更早 ${n} 個`;
const opts = { all: false, moreLabel: label };

describe("toolsToNodes", () => {
  it("每個工具一個偽節點：id 加前綴、類別放 agent_type、提示放 description", () => {
    const [n] = toolsToNodes([tc("a", 10, { parent: "ag1", category: "web", hint: "WebFetch: x" })], 1, opts);
    expect(n).toMatchObject({ id: `${TOOL_PREFIX}a`, parent: "ag1", agent_type: "web", description: "WebFetch: x", status: "done", started_ms: 10, ended_ms: 15, turn: 1 });
  });

  it("只取指定輪次", () => {
    const nodes = toolsToNodes([tc("a", 1), tc("b", 2, { turn: 2 })], 2, opts);
    expect(nodes.map((n) => n.id)).toEqual([`${TOOL_PREFIX}b`]);
  });

  it("每個呼叫者只留最近 N 個已完成的，更早的收成一張群組卡（數量正確、排在最前）", () => {
    const tools = Array.from({ length: TOOL_WINDOW + 3 }, (_, i) => tc(`t${i}`, i * 10));
    const nodes = toolsToNodes(tools, 1, opts);
    const more = nodes.find((n) => n.id.startsWith(TOOLS_MORE_PREFIX));
    expect(more?.description).toBe("更早 3 個");
    expect(nodes.filter((n) => n.id.startsWith(TOOL_PREFIX)).map((n) => n.id.slice(TOOL_PREFIX.length)))
      .toEqual(["t3", "t4", "t5", "t6", "t7", "t8"]);
    expect(more!.started_ms).toBeLessThan(nodes.find((n) => n.id === `${TOOL_PREFIX}t3`)!.started_ms + 1);
  });

  it("失敗、執行中、中斷的永遠留著，即使很舊", () => {
    const tools = [
      tc("old-fail", 0, { status: "failed" }),
      tc("old-run", 1, { status: "running", ended_ms: null }),
      ...Array.from({ length: TOOL_WINDOW + 2 }, (_, i) => tc(`t${i}`, 100 + i)),
    ];
    const ids = toolsToNodes(tools, 1, opts).map((n) => n.id);
    expect(ids).toContain(`${TOOL_PREFIX}old-fail`);
    expect(ids).toContain(`${TOOL_PREFIX}old-run`);
  });

  it("不同呼叫者各自計算視窗", () => {
    const tools = [
      ...Array.from({ length: TOOL_WINDOW + 1 }, (_, i) => tc(`m${i}`, i)),
      ...Array.from({ length: TOOL_WINDOW }, (_, i) => tc(`s${i}`, i, { parent: "ag1" })),
    ];
    const mores = toolsToNodes(tools, 1, opts).filter((n) => n.id.startsWith(TOOLS_MORE_PREFIX));
    expect(mores).toHaveLength(1);
    expect(mores[0].parent).toBeNull();
  });

  it("all=true 時全部展開、沒有群組卡", () => {
    const tools = Array.from({ length: TOOL_WINDOW + 4 }, (_, i) => tc(`t${i}`, i));
    const nodes = toolsToNodes(tools, 1, { all: true, moreLabel: label });
    expect(nodes).toHaveLength(TOOL_WINDOW + 4);
  });
});

describe("attachTools", () => {
  const agent = (id: string): ActivityNode => ({
    id, parent: null, agent_type: "x", description: id, status: "done", started_ms: 0, ended_ms: 1,
    current_tool: null, summary: null, tokens: null, tool_count: null, last_event_ms: 0, turn: 1,
  });

  it("工具的呼叫者不在畫面上（例如已折進群組）就丟掉；主 Claude 的工具保留", () => {
    const tools = toolsToNodes([tc("keep", 1, { parent: "ag1" }), tc("gone", 2, { parent: "ag2" }), tc("main", 3)], 1, opts);
    const out = attachTools([agent("ag1")], tools).map((n) => n.id);
    expect(out).toEqual(["ag1", `${TOOL_PREFIX}keep`, `${TOOL_PREFIX}main`]);
  });
});

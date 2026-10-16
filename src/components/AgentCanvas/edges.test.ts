import { describe, expect, it } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import { computeEdges, diffPulses, EDGE_COLORS, edgeActivity, edgePath } from "./edges";

const ag = (id: string, kind: AgentRow["kind"]): AgentRow => ({
  id, kind, name: id, role: "", prompt_prefix: "", project_dir: "/r", canvas_x: 0, canvas_y: 0,
  worktree_path: null, branch: null, paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false,
});

const card = (over: Partial<TaskRow>): TaskRow => ({
  id: "t", title: "t", body: "", project_dir: "/r", label: null, status: "queued", parallel_ok: true,
  interactive: false, sort_order: 0, outcome: null, tab_id: null, transcript_path: null, error_message: null,
  created_at: "", dispatched_at: null, finished_at: null, ai_summary: null, archived_at: null, session_id: null,
  session_path: null, use_bridge: false, bridge_tiers: null, worktree_path: null, worktree_branch: null,
  isolate_worktree: null, agent_id: null, start_sha: null, end_sha: null, card_kind: "work", module_id: null,
  review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null, ...over,
});

const C1 = ag("c1", "coder");
const C2 = ag("c2", "coder");
const R = ag("r", "reviewer");
const O = ag("o", "orchestrator");

describe("computeEdges", () => {
  it("Coder→Reviewer→Orchestrator", () => {
    expect(computeEdges([C1, C2, R, O])).toEqual([
      { from: "c1", to: "r" }, { from: "c2", to: "r" }, { from: "r", to: "o" },
    ]);
  });
  it("沒有 Reviewer 時 Coder 直接接 Orchestrator", () => {
    expect(computeEdges([C1, O])).toEqual([{ from: "c1", to: "o" }]);
  });
  it("有產出規格的工程師時，協調者派工給其他工程師（虛線）", () => {
    const spec = { ...C1, produces_spec: true };
    expect(computeEdges([spec, C2, R, O])).toEqual([
      { from: "c1", to: "r" }, { from: "c2", to: "r" }, { from: "r", to: "o" },
      { from: "o", to: "c2", kind: "dispatch" },
    ]);
  });
  it("只有 Coder 沒有連線", () => {
    expect(computeEdges([C1, C2])).toEqual([]);
  });
});

describe("edgeActivity", () => {
  const modA = card({ id: "A", agent_id: "c1", status: "done", outcome: "success" });
  it("Reviewer 正在審 c1 的模組 → 只有 c1→r 亮黃", () => {
    const tasks = [modA, card({ id: "RA", card_kind: "review", module_id: "A", agent_id: "r", status: "running" })];
    const act = edgeActivity(computeEdges([C1, C2, R, O]), [C1, C2, R, O], tasks);
    expect(act.get("c1>r")).toBe(EDGE_COLORS.review);
    expect(act.has("c2>r")).toBe(false);
    expect(act.has("r>o")).toBe(false);
  });
  it("排隊中的審查是藍色，合併中是綠色", () => {
    const tasks = [
      modA,
      card({ id: "RA", card_kind: "review", module_id: "A", agent_id: "r", status: "queued" }),
      card({ id: "M", card_kind: "merge", agent_id: "o", status: "running" }),
    ];
    const act = edgeActivity(computeEdges([C1, R, O]), [C1, R, O], tasks);
    expect(act.get("c1>r")).toBe(EDGE_COLORS.handoff);
    expect(act.get("r>o")).toBe(EDGE_COLORS.pass);
  });
});

describe("diffPulses", () => {
  const agents = [C1, R, O];
  const modA = card({ id: "A", agent_id: "c1", status: "done", outcome: "success" });
  it("第一次載入不產生光點", () => {
    expect(diffPulses(null, [modA], agents)).toEqual([]);
  });
  it("新的審查卡：c1→r 藍色", () => {
    const rv = card({ id: "RA", card_kind: "review", module_id: "A", agent_id: "r" });
    expect(diffPulses([modA], [modA, rv], agents)).toEqual([
      { key: "RA:in", from: "c1", to: "r", color: EDGE_COLORS.handoff, reverse: false },
    ]);
  });
  it("審查剛通過：r→o 綠色；新的修正卡：沿 c1→r 反向橘色", () => {
    const rvOld = card({ id: "RA", card_kind: "review", module_id: "A", agent_id: "r", status: "running" });
    const rvPass = { ...rvOld, status: "done" as const, verdict: "pass" as const };
    expect(diffPulses([modA, rvOld], [modA, rvPass], agents).map((p) => p.key)).toEqual(["RA:pass"]);
    const fx = card({ id: "FA", card_kind: "fix", module_id: "A", agent_id: "c1" });
    expect(diffPulses([modA], [modA, fx], agents)).toEqual([
      { key: "FA:fix", from: "c1", to: "r", color: EDGE_COLORS.fix, reverse: true },
    ]);
  });

  it("拆解卡：r→o 綠；拆出的實作卡：o→Coder 紫", () => {
    const spec = card({ id: "S", agent_id: "c1", status: "done", outcome: "success", is_spec: true });
    const plan = card({ id: "P", card_kind: "plan", module_id: "S", agent_id: "o" });
    expect(diffPulses([spec], [spec, plan], agents).map((p) => [p.key, p.from, p.to])).toEqual([["P:plan", "r", "o"]]);
    const impl = card({ id: "I", agent_id: "c1", from_spec: "S" });
    expect(diffPulses([spec, plan], [spec, plan, impl], agents)).toEqual([
      { key: "I:dispatch", from: "o", to: "c1", color: EDGE_COLORS.dispatch, reverse: false },
    ]);
  });

  it("派工線：工程師手上有規格拆出來、還沒做完的卡就亮紫色", () => {
    const spec = { ...C1, produces_spec: true };
    const agents = [spec, C2, O];
    const edges = computeEdges(agents);
    const impl = card({ id: "I", agent_id: "c2", from_spec: "S", status: "running" });
    expect(edgeActivity(edges, agents, [impl]).get("o>c2")).toBe(EDGE_COLORS.dispatch);
    expect(edgeActivity(edges, agents, [{ ...impl, status: "done" }]).has("o>c2")).toBe(false);
    expect(edgeActivity(edges, agents, [{ ...impl, from_spec: null }]).has("o>c2")).toBe(false);
  });
});

describe("edgePath", () => {
  /** 路徑的終點與最後一個控制點：箭頭方向由這兩點決定。 */
  const tail = (d: string) => {
    const nums = d.match(/-?\d+(\.\d+)?/g)!.map(Number);
    return { cx: nums[4], cy: nums[5], x: nums[6], y: nums[7] };
  };

  it("下游在右邊：右緣接左緣，從左邊水平進來", () => {
    const t = tail(edgePath({ x: 0, y: 0, w: 200, h: 100 }, { x: 400, y: 0, w: 200, h: 100 }));
    expect([t.x, t.y]).toEqual([400, 50]);
    expect(t.cx).toBeLessThan(t.x);
  });

  it("兩張卡左右重疊、下游在上面：接到下游的下緣，從下面往上進來，箭頭不會被卡片蓋住", () => {
    // 工程師3（下）→ 審核者（上），左右有重疊。
    const t = tail(edgePath({ x: 670, y: 920, w: 340, h: 400 }, { x: 895, y: 300, w: 345, h: 115 }));
    expect([t.x, t.y]).toEqual([895 + 345 / 2, 415]);
    expect(t.cy).toBeGreaterThan(t.y);
  });

  it("兩張卡左右重疊、下游在下面：接到下游的上緣，從上面往下進來", () => {
    const t = tail(edgePath({ x: 0, y: 0, w: 200, h: 100 }, { x: 50, y: 300, w: 200, h: 100 }));
    expect([t.x, t.y]).toEqual([150, 300]);
    expect(t.cy).toBeLessThan(t.y);
  });
});

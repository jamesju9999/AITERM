import { describe, expect, it } from "vitest";
import type { ActivityNode, ActivitySnapshot } from "../../ipc/claudeActivity";
import { detectTransitions, findStuck, STUCK_MS } from "./alerts";

const NOW = 1_000_000;
const n = (id: string, status: ActivityNode["status"], over: Partial<ActivityNode> = {}): ActivityNode => ({
  id, parent: null, agent_type: "x", description: id, status, started_ms: NOW - 5_000,
  ended_ms: status === "running" ? null : NOW - 1_000, current_tool: null, summary: null, tokens: null,
  tool_count: null, last_event_ms: NOW - 1_000, turn: 1, ...over,
});
const snap = (...nodes: ActivityNode[]): ActivitySnapshot => ({
  nodes, main_tool: null, main_tool_count: 0, main_recent: [], tools: [], turn: 1,
});

describe("detectTransitions", () => {
  it("running→failed 發失敗警示", () => {
    const a = detectTransitions(snap(n("a", "running"), n("b", "running")), snap(n("a", "failed"), n("b", "running")), NOW);
    expect(a).toEqual([{ kind: "failed", id: "a", description: "a" }]);
  });

  it("最後一個 running 結束 → 全部完成（含數量與失敗數）", () => {
    const a = detectTransitions(snap(n("a", "running"), n("b", "done")), snap(n("a", "done"), n("b", "done")), NOW);
    expect(a).toEqual([{ kind: "all_done", count: 2, failed: 0 }]);
  });

  it("同一次轉換既全完成又有失敗：只發一則全完成（帶失敗數），不重複吵", () => {
    const a = detectTransitions(snap(n("a", "running"), n("b", "running")), snap(n("a", "failed"), n("b", "done")), NOW);
    expect(a).toEqual([{ kind: "all_done", count: 2, failed: 1 }]);
  });

  it("沒有 running→0 的轉換、也沒有新失敗時不發", () => {
    expect(detectTransitions(snap(n("a", "running")), snap(n("a", "running"), n("b", "running")), NOW)).toEqual([]);
    expect(detectTransitions(snap(n("a", "done")), snap(n("a", "done")), NOW)).toEqual([]);
  });

  it("本來就失敗的不重複發", () => {
    const f = n("a", "failed");
    expect(detectTransitions(snap(f, n("b", "running")), snap(f, n("b", "running")), NOW)).toEqual([]);
  });

  it("很久以前就結束的失敗（例如重新掛載後讀到的舊快照）不發", () => {
    const old = n("a", "failed", { ended_ms: NOW - 120_000 });
    expect(detectTransitions(snap(), snap(old), NOW)).toEqual([]);
  });

  it("中斷（interrupted）不算失敗；全部結束時仍算全完成", () => {
    const a = detectTransitions(snap(n("a", "running")), snap(n("a", "interrupted")), NOW);
    expect(a).toEqual([{ kind: "all_done", count: 1, failed: 0 }]);
  });

  it("清空（離開 claude）不發", () => {
    expect(detectTransitions(snap(n("a", "running")), snap(), NOW)).toEqual([]);
  });
});

describe("findStuck", () => {
  it("running 且超過門檻沒事件的節點才算卡住", () => {
    const nodes = [
      n("stuck", "running", { last_event_ms: NOW - STUCK_MS }),
      n("fresh", "running", { last_event_ms: NOW - STUCK_MS + 1 }),
      n("done", "done", { last_event_ms: NOW - STUCK_MS * 2 }),
    ];
    expect(findStuck(nodes, NOW, new Set()).map((x) => x.id)).toEqual(["stuck"]);
  });

  it("已經提醒過的不再提醒", () => {
    const nodes = [n("stuck", "running", { last_event_ms: NOW - STUCK_MS })];
    expect(findStuck(nodes, NOW, new Set(["stuck"]))).toEqual([]);
  });
});

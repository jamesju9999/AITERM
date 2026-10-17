import { describe, expect, it } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import type { TaskRow, TaskStatus } from "../../ipc/tasks";
import { deriveAgentStatus } from "./deriveAgentStatus";

const agent = (over: Partial<AgentRow> = {}): AgentRow => ({
  id: "a1", kind: "coder", name: "Coder Agent1", role: "Senior Coder", prompt_prefix: "",
  project_dir: "/r", canvas_x: 0, canvas_y: 0, worktree_path: null, branch: null,
  paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false, ...over,
});

const card = (status: TaskStatus, outcome: TaskRow["outcome"] = null, agent_id: string | null = "a1"): TaskRow => ({
  id: Math.random().toString(36), title: "t", body: "", project_dir: "/r", label: null, status,
  parallel_ok: true, interactive: false, sort_order: 0, outcome, tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, ai_summary: null,
  archived_at: null, session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  worktree_path: null, worktree_branch: null, isolate_worktree: null, agent_id, start_sha: null, end_sha: null, card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null,
});

describe("deriveAgentStatus", () => {
  it("沒有卡片是 idle", () => {
    expect(deriveAgentStatus(agent(), [])).toBe("idle");
  });

  it("只看自己的卡片", () => {
    expect(deriveAgentStatus(agent(), [card("running", null, "other")])).toBe("idle");
  });

  it("有卡片在跑是 coding，即使還有排隊的", () => {
    expect(deriveAgentStatus(agent(), [card("queued"), card("running")])).toBe("coding");
  });

  it("只有排隊的是 waiting", () => {
    expect(deriveAgentStatus(agent(), [card("done", "success"), card("queued")])).toBe("waiting");
  });

  it("全部成功完成是 done", () => {
    expect(deriveAgentStatus(agent(), [card("done", "success"), card("done", "success")])).toBe("done");
  });

  it("完成但中間有失敗（已跳過）不是 done", () => {
    expect(deriveAgentStatus(agent(), [card("done", "success"), card("done", "failed")])).toBe("idle");
  });

  it("驗收者執行中是 verifying", () => {
    expect(deriveAgentStatus(agent({ kind: "verifier" }), [card("running")])).toBe("verifying");
  });

  it("執行中的字依 Agent 種類", () => {
    expect(deriveAgentStatus(agent({ kind: "reviewer" }), [card("running")])).toBe("reviewing");
    expect(deriveAgentStatus(agent({ kind: "orchestrator" }), [card("running")])).toBe("merging");
  });

  it("有模組狀態時以模組為準：還在等審查不算完成，卡住的模組亮紅燈", () => {
    const done = [card("done", "success")];
    expect(deriveAgentStatus(agent(), done, ["passed", "awaiting_review"])).toBe("idle");
    expect(deriveAgentStatus(agent(), done, ["passed", "attention"])).toBe("attention");
    expect(deriveAgentStatus(agent(), done, ["passed", "passed"])).toBe("done");
  });

  it("Orchestrator：有模組沒通過就不是完成，但也不亮紅燈", () => {
    const o = agent({ kind: "orchestrator" });
    expect(deriveAgentStatus(o, [card("done", "success")], ["passed", "attention"])).toBe("idle");
    expect(deriveAgentStatus(o, [card("done", "success")], ["passed"])).toBe("done");
  });

  it("暫停優先於一切", () => {
    expect(deriveAgentStatus(agent({ paused_task_id: "x" }), [card("queued")])).toBe("attention");
  });
});

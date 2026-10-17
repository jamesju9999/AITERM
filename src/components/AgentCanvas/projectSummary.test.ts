import { describe, expect, it } from "vitest";

import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import { projectSummary, shortBranch } from "./projectSummary";

let n = 0;
const card = (over: Partial<TaskRow>): TaskRow => ({
  id: `c${n++}`, title: "t", body: "", project_dir: "/repo", label: null, status: "done",
  parallel_ok: true, interactive: false, sort_order: 0, outcome: "success", tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, ai_summary: null,
  archived_at: null, session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  worktree_path: null, worktree_branch: null, isolate_worktree: null, agent_id: "c1", start_sha: null, end_sha: null,
  card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null,
  ...over,
});
const agent = (id: string, kind: AgentRow["kind"]): AgentRow => ({
  id, kind, name: id, role: "", prompt_prefix: "", project_dir: "/repo", canvas_x: 0, canvas_y: 0,
  worktree_path: null, branch: null, paused_task_id: null, created_at: "", last_merge_signature: null, produces_spec: false,
});

const coder = agent("c1", "coder");
const orch = agent("o1", "orchestrator");
const merge = (over: Partial<TaskRow>) => card({ agent_id: "o1", card_kind: "merge", finished_at: 10, ...over });

describe("projectSummary 有驗收者時", () => {
  const ver = agent("v1", "verifier");
  const agents = [coder, orch, ver];
  const m = merge({ id: "MG", verdict: null });
  const verify = (over: Partial<TaskRow>) => card({ agent_id: "v1", card_kind: "verify", review_of: "MG", finished_at: 20, ...over });

  it("合併完成、驗收還沒開始或進行中：驗收中", () => {
    expect(projectSummary(agents, [card({ id: "A" }), m])).toEqual({ kind: "verifying", total: 1 });
    expect(projectSummary(agents, [card({ id: "A" }), m, verify({ status: "running", outcome: null })])).toEqual({ kind: "verifying", total: 1 });
  });

  it("驗收退回／要你決定", () => {
    const failed = verify({ verdict: "fail", review_reasons: JSON.stringify(["登入", "訂單"]) });
    expect(projectSummary(agents, [card({ id: "A" }), m, failed])).toEqual({ kind: "verify_failed", total: 1, items: 2 });
    expect(projectSummary(agents, [card({ id: "A" }), m, verify({ verdict: "attention", review_reasons: "[]" })])).toEqual({
      kind: "verify_attention",
      total: 1,
    });
  });

  it("驗收通過：照合併卡記的程式庫結果", () => {
    const tasks = [card({ id: "A" }), merge({ id: "MG", verdict: "repo_merged" }), verify({ verdict: "pass" })];
    expect(projectSummary(agents, tasks)).toEqual({ kind: "in_repo", total: 1, repoDir: "/repo" });
  });

  it("沒有驗收者時完全照舊", () => {
    expect(projectSummary([coder, orch], [card({ id: "A" }), m])).toEqual({ kind: "integrated", total: 1 });
  });
});

describe("projectSummary", () => {
  it("沒有指派給工程師的模組：沒有東西可總結", () => {
    expect(projectSummary([coder, orch], [card({ agent_id: null })])).toEqual({ kind: "none" });
  });

  it("還有模組沒通過：進行中，數出通過與需處理的數量", () => {
    const tasks = [card({ id: "A" }), card({ id: "B", status: "running", outcome: null }), card({ id: "C", outcome: "failed" })];
    expect(projectSummary([coder, orch], tasks)).toEqual({ kind: "working", passed: 1, total: 3, attention: 1 });
  });

  it("全部通過、沒有協調者：全部完成", () => {
    expect(projectSummary([coder], [card({ id: "A" })])).toEqual({ kind: "all_passed", total: 1 });
  });

  it("全部通過、合併卡還在排隊或執行：合併中", () => {
    expect(projectSummary([coder, orch], [card({ id: "A" }), merge({ status: "running", outcome: null })])).toEqual({
      kind: "merging",
      total: 1,
    });
  });

  it("合併完成：看最新那張合併卡有沒有合回程式庫", () => {
    const done = (verdict: TaskRow["verdict"]) => projectSummary([coder, orch], [card({ id: "A" }), merge({ verdict })]);
    expect(done("repo_merged")).toEqual({ kind: "in_repo", total: 1, repoDir: "/repo" });
    expect(done("repo_dirty")).toEqual({ kind: "repo_pending", total: 1, reason: "repo_dirty" });
    expect(done(null)).toEqual({ kind: "integrated", total: 1 });
  });

  it("只看最新一張合併卡", () => {
    const tasks = [card({ id: "A" }), merge({ verdict: "repo_merged", finished_at: 5 }), merge({ verdict: "repo_diverged", finished_at: 9 })];
    expect(projectSummary([coder, orch], tasks)).toEqual({ kind: "repo_pending", total: 1, reason: "repo_diverged" });
  });
});

describe("shortBranch", () => {
  it("Agent 分支只留 uuid 前 8 碼，其他名稱原樣", () => {
    expect(shortBranch("aiterm-agent/bf7cb7ff-3c88-4149-b761-c4b4eadac581")).toBe("aiterm-agent/bf7cb7ff");
    expect(shortBranch("feature/login")).toBe("feature/login");
  });
});

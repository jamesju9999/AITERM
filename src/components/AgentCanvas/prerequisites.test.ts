import { describe, expect, it } from "vitest";

import type { TaskRow } from "../../ipc/tasks";
import { idList, waitingPrerequisites } from "./prerequisites";

let n = 0;
const card = (over: Partial<TaskRow>): TaskRow => ({
  id: `c${n++}`, title: "t", body: "", project_dir: "/r", label: null, status: "done",
  parallel_ok: true, interactive: false, sort_order: 0, outcome: "success", tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, ai_summary: null,
  archived_at: null, session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  worktree_path: null, worktree_branch: null, isolate_worktree: null, agent_id: "a1", start_sha: null, end_sha: null,
  card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: "S",
  ...over,
});

describe("waitingPrerequisites", () => {
  const theirs = card({ id: "Y", title: "Y 模組", agent_id: "a2" });
  const mine = card({ id: "M", title: "我的前置", agent_id: "a1" });
  const waiting = card({ id: "X", status: "queued", outcome: null, depends_on: JSON.stringify(["Y", "M"]) });

  // 模組一做完就會建出審查卡（後端 after_card_finished），這裡照樣放一張。
  const review = card({ id: "R", card_kind: "review", module_id: "Y", review_of: "Y", status: "queued", outcome: null, agent_id: "r1" });

  it("別位工程師的前置還沒審過：在等它；自己的前置不算", () => {
    expect(waitingPrerequisites(waiting, [theirs, mine, waiting, review], true).map((c) => c.title)).toEqual(["Y 模組"]);
  });

  it("前置審查通過就不等", () => {
    const pass = { ...review, status: "done" as const, outcome: "success" as const, verdict: "pass" as const };
    expect(waitingPrerequisites(waiting, [theirs, mine, waiting, pass], true)).toEqual([]);
  });

  it("已經開工或沒有相依：不算在等", () => {
    expect(waitingPrerequisites({ ...waiting, status: "running" }, [theirs, waiting, review], true)).toEqual([]);
    expect(waitingPrerequisites(card({ status: "queued" }), [theirs], true)).toEqual([]);
  });

  it("idList 容忍空值與壞掉的 JSON", () => {
    expect(idList(null)).toEqual([]);
    expect(idList("nope")).toEqual([]);
    expect(idList('["a","b"]')).toEqual(["a", "b"]);
  });
});

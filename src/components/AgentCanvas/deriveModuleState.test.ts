import { describe, expect, it } from "vitest";

import type { TaskRow } from "../../ipc/tasks";
import { deriveModuleState } from "./deriveModuleState";

let n = 0;
const card = (over: Partial<TaskRow>): TaskRow => ({
  id: `c${n++}`, title: "t", body: "", project_dir: "/r", label: null, status: "done",
  parallel_ok: true, interactive: false, sort_order: 0, outcome: "success", tab_id: null, transcript_path: null,
  error_message: null, created_at: "", dispatched_at: null, finished_at: null, ai_summary: null,
  archived_at: null, session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  worktree_path: null, worktree_branch: null, isolate_worktree: null, agent_id: "c1", start_sha: null, end_sha: null,
  card_kind: "work", module_id: null, review_of: null, verdict: null, review_reasons: null, module_override: false, is_spec: false, from_spec: null,
  ...over,
});

const mod = (over: Partial<TaskRow> = {}) => card({ id: "M", ...over });
const review = (of: string, over: Partial<TaskRow> = {}) =>
  card({ card_kind: "review", module_id: "M", review_of: of, ...over });
const fix = (of: string, over: Partial<TaskRow> = {}) =>
  card({ card_kind: "fix", module_id: "M", review_of: of, ...over });

describe("deriveModuleState", () => {
  it("模組本身的狀態", () => {
    expect(deriveModuleState(mod({ status: "planning", outcome: null }), [], true)).toBe("planning");
    expect(deriveModuleState(mod({ status: "queued", outcome: null }), [], true)).toBe("queued");
    expect(deriveModuleState(mod({ status: "running", outcome: null }), [], true)).toBe("coding");
    expect(deriveModuleState(mod({ outcome: "failed" }), [], true)).toBe("failed");
  });

  it("驗收退回的修正卡接在通過的審查後面，模組重新打開（與後端同規則）", () => {
    const m = mod();
    const r = review("M", { id: "R", verdict: "pass" });
    expect(deriveModuleState(m, [r], true)).toBe("passed");
    expect(deriveModuleState(m, [r, fix("R", { id: "F", status: "queued", outcome: null })], true)).toBe("fix_queued");
    const fDone = fix("R", { id: "F" });
    expect(deriveModuleState(m, [r, fDone], true)).toBe("awaiting_review");
    expect(deriveModuleState(m, [r, fDone, review("F", { verdict: "pass" })], true)).toBe("passed");
  });

  it("沒有審查者時，驗收退回的修正卡接在模組後面", () => {
    expect(deriveModuleState(mod(), [fix("M", { status: "running", outcome: null })], false)).toBe("coding");
  });

  it("驗收退回的修正不算審查退回的輪數", () => {
    const chain = [
      review("M", { id: "R1", verdict: "pass" }),
      fix("R1", { id: "F1" }),
      review("F1", { id: "R2", verdict: "fail" }),
      fix("R2", { id: "F2" }),
      review("F2", { id: "R3", verdict: "fail" }),
    ];
    expect(deriveModuleState(mod(), chain, true)).toBe("fix_queued");
  });

  it("沒有任何審查的完成模組算通過", () => {
    expect(deriveModuleState(mod(), [], true)).toBe("passed");
  });

  it("審查排隊／進行中／通過", () => {
    const m = mod();
    expect(deriveModuleState(m, [review("M", { status: "queued", outcome: null })], true)).toBe("awaiting_review");
    expect(deriveModuleState(m, [review("M", { status: "running", outcome: null })], true)).toBe("reviewing");
    expect(deriveModuleState(m, [review("M", { verdict: "pass" })], true)).toBe("passed");
  });

  it("不通過→修正排隊→修正中→再審→通過", () => {
    const m = mod();
    const r1 = review("M", { id: "R1", verdict: "fail" });
    expect(deriveModuleState(m, [r1], true)).toBe("fix_queued");
    const f1q = fix("R1", { id: "F1", status: "queued", outcome: null });
    expect(deriveModuleState(m, [r1, f1q], true)).toBe("fix_queued");
    const f1r = { ...f1q, status: "running" as const };
    expect(deriveModuleState(m, [r1, f1r], true)).toBe("coding");
    const f1 = { ...f1q, status: "done" as const, outcome: "success" as const };
    expect(deriveModuleState(m, [r1, f1], true)).toBe("awaiting_review");
    expect(deriveModuleState(m, [r1, f1], false)).toBe("passed");
    expect(deriveModuleState(m, [r1, f1, review("F1", { verdict: "pass" })], true)).toBe("passed");
  });

  it("第三次不通過是 attention，放行後算通過", () => {
    const chain = [
      review("M", { id: "R1", verdict: "fail" }),
      fix("R1", { id: "F1" }),
      review("F1", { id: "R2", verdict: "fail" }),
      fix("R2", { id: "F2" }),
      review("F2", { id: "R3", verdict: "fail" }),
    ];
    expect(deriveModuleState(mod(), chain, true)).toBe("attention");
    expect(deriveModuleState(mod({ module_override: true }), chain, true)).toBe("passed");
  });

  it("審查卡本身失敗是 review_failed", () => {
    expect(deriveModuleState(mod(), [review("M", { outcome: "failed" })], true)).toBe("review_failed");
  });

  it("不看別的模組的鏈", () => {
    expect(deriveModuleState(mod(), [review("M", { module_id: "OTHER", verdict: "fail" })], true)).toBe("passed");
  });

  it("規格模組：審查通過後等協調者拆解", () => {
    const m = mod({ is_spec: true });
    const pass = review("M", { verdict: "pass" });
    const plan = (over: Partial<TaskRow>) => card({ card_kind: "plan", module_id: "M", agent_id: "o", ...over });
    expect(deriveModuleState(m, [pass], true)).toBe("passed");
    expect(deriveModuleState(m, [pass, plan({ status: "running", outcome: null })], true)).toBe("dispatching");
    expect(deriveModuleState(m, [pass, plan({ outcome: "failed" })], true)).toBe("dispatch_failed");
    expect(deriveModuleState(m, [pass, plan({})], true)).toBe("passed");
    // 一般模組不受拆解卡影響
    expect(deriveModuleState(mod(), [pass, plan({ status: "running", outcome: null })], true)).toBe("passed");
  });
});

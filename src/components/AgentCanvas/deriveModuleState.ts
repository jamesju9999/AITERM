import type { TaskRow } from "../../ipc/tasks";

/** 與後端 `tasks::workflow::MAX_FIX_ROUNDS` 相同。 */
export const MAX_FIX_ROUNDS = 2;

/**
 * 模組（使用者指派給 Coder 的 work 卡）目前的狀態。規則與後端
 * `tasks::workflow::module_state` 逐條相同——後端用它決定何時合併，前端用它
 * 畫 chip，兩邊必須一致。
 */
export type ModuleState =
  | "planning"
  | "queued"
  | "coding"
  | "fix_queued"
  | "awaiting_review"
  | "reviewing"
  | "passed"
  | "attention"
  | "failed"
  | "review_failed"
  | "dispatching"
  | "dispatch_failed";

/** 規格模組通過審查後還要等協調者拆解完成（與後端 spec_gate 相同）。 */
function specGate(module: TaskRow, chain: TaskRow[]): ModuleState {
  if (!module.is_spec) return "passed";
  const plan = chain.find((c) => c.card_kind === "plan");
  if (!plan) return "passed";
  if (plan.status === "queued" || plan.status === "running") return "dispatching";
  return plan.outcome === "success" ? "passed" : "dispatch_failed";
}

/** 沿著 模組 → 審查 → 修正 → 審查 … 的鏈走到尾端。 */
function walk(module: TaskRow, chain: TaskRow[]): { tip: TaskRow; fails: number } {
  let cur = module;
  let tip = module;
  let fails = 0;
  for (;;) {
    const review = chain.find((c) => c.card_kind === "review" && c.review_of === cur.id);
    if (!review) break;
    tip = review;
    if (review.verdict !== "fail") break;
    fails += 1;
    const fix = chain.find((c) => c.card_kind === "fix" && c.review_of === review.id);
    if (!fix) break;
    tip = fix;
    cur = fix;
  }
  return { tip, fails };
}

export function deriveModuleState(module: TaskRow, all: TaskRow[], hasReviewer: boolean): ModuleState {
  const chain = all.filter((c) => c.module_id === module.id);
  const { tip, fails } = walk(module, chain);
  const isWork = tip.card_kind !== "review";
  if (isWork) {
    if (tip.status === "running") return "coding";
    if (tip.status === "queued") return tip.card_kind === "fix" ? "fix_queued" : "queued";
    if (tip.status === "planning") return "planning";
  } else {
    if (tip.status === "running") return "reviewing";
    if (tip.status === "queued") return "awaiting_review";
  }
  if (module.module_override) return "passed";
  if (isWork) {
    if (tip.outcome !== "success") return "failed";
    if (tip.card_kind === "work" || !hasReviewer) return specGate(module, chain);
    return "awaiting_review";
  }
  if (tip.verdict === "pass") return specGate(module, chain);
  if (tip.verdict === "fail") return fails > MAX_FIX_ROUNDS ? "attention" : "fix_queued";
  return "review_failed";
}

/** 這個模組被退回過幾輪（verdict = fail 的審查數）。 */
export function failCount(module: TaskRow, all: TaskRow[]): number {
  return all.filter((c) => c.module_id === module.id && c.card_kind === "review" && c.verdict === "fail").length;
}

import type { IntakeItem } from "../ipc/intake";

export type FlowKey = "received" | "approve" | "run" | "answer" | "verify" | "reply";
export type FlowState = "done" | "current" | "todo" | "skipped" | "failed";

/** 一題從收件到回覆的六個階段，與目前各自的狀態。 */
export function flowSteps(item: Pick<IntakeItem, "status" | "kind" | "verified" | "verification" | "error">): { key: FlowKey; state: FlowState }[] {
  const s = item.status;
  const answered = s === "draft" || s === "done";
  const approve: FlowState = s === "pending" ? "current" : s === "rejected" ? "failed" : "done";
  const run: FlowState = s === "queued" ? "current" : s === "running" || s === "on_board" || answered ? "done" : "todo";
  const answer: FlowState = s === "running" || s === "on_board" ? "current" : answered ? "done" : "todo";
  let verify: FlowState = "todo";
  if (answered) {
    if (item.kind === "code_change" || item.verification) verify = item.verified === false ? "failed" : "done";
    else verify = "skipped";
  }
  const reply: FlowState = s === "done" ? "done" : s === "draft" ? (item.error ? "failed" : "current") : "todo";
  return [
    { key: "received", state: "done" },
    { key: "approve", state: approve },
    { key: "run", state: run },
    { key: "answer", state: answer },
    { key: "verify", state: verify },
    { key: "reply", state: reply },
  ];
}

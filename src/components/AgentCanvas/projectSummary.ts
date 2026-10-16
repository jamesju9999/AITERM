import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import { deriveModuleState, type ModuleState } from "./deriveModuleState";

/** 頂列那一行「整個專案做到哪了」。 */
export type ProjectSummary =
  | { kind: "none" }
  | { kind: "working"; passed: number; total: number; attention: number }
  /** 全部通過，沒有協調者（不會合併）。 */
  | { kind: "all_passed"; total: number }
  | { kind: "merging"; total: number }
  /** 已合進整合分支，不知道有沒有合回程式庫（舊的合併卡沒有記錄）。 */
  | { kind: "integrated"; total: number }
  | { kind: "in_repo"; total: number; repoDir: string }
  | { kind: "repo_pending"; total: number; reason: "repo_dirty" | "repo_merge_in_progress" | "repo_diverged" };

const NEEDS_YOU: ModuleState[] = ["attention", "review_failed", "failed", "dispatch_failed"];

export function projectSummary(agents: AgentRow[], tasks: TaskRow[]): ProjectSummary {
  const coders = new Set(agents.filter((a) => a.kind === "coder").map((a) => a.id));
  const hasReviewer = agents.some((a) => a.kind === "reviewer");
  const modules = tasks.filter((c) => c.card_kind === "work" && c.agent_id && coders.has(c.agent_id));
  if (modules.length === 0) return { kind: "none" };
  const states = modules.map((m) => deriveModuleState(m, tasks, hasReviewer));
  const total = modules.length;
  const passed = states.filter((s) => s === "passed").length;
  if (passed < total) {
    return { kind: "working", passed, total, attention: states.filter((s) => NEEDS_YOU.includes(s)).length };
  }
  const orch = agents.find((a) => a.kind === "orchestrator");
  if (!orch) return { kind: "all_passed", total };
  const merges = tasks.filter((c) => c.agent_id === orch.id && c.card_kind === "merge");
  if (merges.some((c) => c.status === "queued" || c.status === "running")) return { kind: "merging", total };
  const last = merges
    .filter((c) => c.status === "done" && c.outcome === "success")
    .sort((a, b) => (b.finished_at ?? 0) - (a.finished_at ?? 0))[0];
  if (!last) return { kind: "merging", total };
  switch (last.verdict) {
    case "repo_merged":
      return { kind: "in_repo", total, repoDir: last.project_dir };
    case "repo_dirty":
    case "repo_merge_in_progress":
    case "repo_diverged":
      return { kind: "repo_pending", total, reason: last.verdict };
    default:
      return { kind: "integrated", total };
  }
}

/** `aiterm-agent/<uuid>` 只留 uuid 前 8 碼；完整名稱放在 title。 */
export function shortBranch(branch: string): string {
  return branch.replace(/^(aiterm-agent\/[0-9a-f]{8})-[0-9a-f-]+$/, "$1");
}

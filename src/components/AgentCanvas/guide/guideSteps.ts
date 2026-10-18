import type { AgentRow } from "../../../ipc/agents";
import type { TaskRow } from "../../../ipc/tasks";

/**
 * Agent 協作的上手步驟。上手清單（B）與情境提示（C）共用這份定義；完成與否
 * 全部從桌布狀態推導，不靠「使用者按過哪個按鈕」。
 */
export const STEP_IDS = ["coder", "module", "assign", "reviewer", "orchestrator", "verifier", "merged", "verified"] as const;
export type StepId = (typeof STEP_IDS)[number];

export const HINT_IDS = ["add_module", "assign_module", "add_reviewer", "add_orchestrator", "add_verifier"] as const;
export type HintId = (typeof HINT_IDS)[number];

export type GuideAgent = Pick<AgentRow, "id" | "kind">;
export type GuideCard = Pick<TaskRow, "card_kind" | "agent_id" | "status" | "outcome">;

/** 指向哪裡：`guide` = 有 `data-guide` 的元素；`agent` = 那位 Agent 的卡片。 */
export interface Hint {
  id: HintId;
  anchor: { guide: string } | { agent: string };
}

/** 進度全域、只增不減，存在這台電腦（跟團隊範本、我的角色一樣）。 */
export interface GuideState {
  tourSeen: boolean;
  checklistHidden: boolean;
  hintsOff: boolean;
  done: StepId[];
  dismissedHints: HintId[];
}

export const DEFAULT_GUIDE: GuideState = { tourSeen: false, checklistHidden: false, hintsOff: false, done: [], dismissedHints: [] };

const KEY = "aiterm.agentCanvas.guide";

export function loadGuide(): GuideState {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (!v || typeof v !== "object") return DEFAULT_GUIDE;
    const o = v as Record<string, unknown>;
    const bools = [o.tourSeen, o.checklistHidden, o.hintsOff];
    if (!bools.every((b) => typeof b === "boolean") || !Array.isArray(o.done) || !Array.isArray(o.dismissedHints)) return DEFAULT_GUIDE;
    return {
      tourSeen: o.tourSeen as boolean,
      checklistHidden: o.checklistHidden as boolean,
      hintsOff: o.hintsOff as boolean,
      done: STEP_IDS.filter((s) => (o.done as unknown[]).includes(s)),
      dismissedHints: HINT_IDS.filter((h) => (o.dismissedHints as unknown[]).includes(h)),
    };
  } catch {
    return DEFAULT_GUIDE;
  }
}

export function saveGuide(state: GuideState): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // 存不了（私密視窗、配額）就只留在這次的畫面
  }
}

export function observedSteps(agents: GuideAgent[], tasks: GuideCard[]): Set<StepId> {
  const out = new Set<StepId>();
  const kinds = new Set(agents.map((a) => a.kind));
  if (kinds.has("coder")) out.add("coder");
  if (kinds.has("reviewer")) out.add("reviewer");
  if (kinds.has("orchestrator")) out.add("orchestrator");
  if (kinds.has("verifier")) out.add("verifier");
  const work = tasks.filter((c) => c.card_kind === "work");
  if (work.length > 0) out.add("module");
  if (work.some((c) => c.agent_id)) out.add("assign");
  if (tasks.some((c) => c.card_kind === "merge" && c.status === "done" && c.outcome === "success")) out.add("merged");
  if (tasks.some((c) => c.card_kind === "verify" && c.status === "done")) out.add("verified");
  return out;
}

/** 把這次看到的完成步驟併進進度。沒有新的就回傳同一個物件，呼叫端可以跳過寫入。 */
export function withObserved(state: GuideState, observed: Set<StepId>): GuideState {
  const added = STEP_IDS.filter((s) => observed.has(s) && !state.done.includes(s));
  if (added.length === 0) return state;
  return { ...state, done: STEP_IDS.filter((s) => state.done.includes(s) || observed.has(s)) };
}

/**
 * 目前這個專案卡在哪一步，回傳最多一個提示（依序取第一個成立的）。跟全域進度
 * 無關，所以新專案仍然有引導。沒有工程師時不給——空桌布有自己的引導。
 */
export function contextHint(agents: GuideAgent[], tasks: GuideCard[]): Hint | null {
  const coders = agents.filter((a) => a.kind === "coder");
  if (coders.length === 0) return null;
  const work = tasks.filter((c) => c.card_kind === "work");
  if (work.length === 0) return { id: "add_module", anchor: { guide: "library" } };
  const library = work.filter((c) => c.status === "planning" && !c.agent_id);
  const idle = coders.find((a) => !work.some((c) => c.agent_id === a.id));
  if (library.length > 0 && idle) return { id: "assign_module", anchor: { agent: idle.id } };
  if (!work.some((c) => c.agent_id)) return null;
  const has = (k: GuideAgent["kind"]) => agents.some((a) => a.kind === k);
  if (!has("reviewer")) return { id: "add_reviewer", anchor: { guide: "pool-reviewer" } };
  if (!has("orchestrator")) return { id: "add_orchestrator", anchor: { guide: "pool-orchestrator" } };
  if (!has("verifier")) return { id: "add_verifier", anchor: { guide: "pool-verifier" } };
  return null;
}

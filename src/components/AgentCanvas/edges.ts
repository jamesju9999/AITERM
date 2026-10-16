import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";

/** 一條自動建立的連線（上游 → 下游）。 */
export interface Edge {
  from: string;
  to: string;
  /** `dispatch` = 協調者把拆好的實作卡派給工程師（規格先行），畫成虛線。 */
  kind?: "dispatch";
}

/**
 * 自動連線：每個 Coder → Reviewer；Reviewer → Orchestrator。沒有 Reviewer 時
 * Coder 直接接 Orchestrator。每個專案各最多一個 Reviewer／Orchestrator（後端保證）。
 */
export function computeEdges(agents: AgentRow[]): Edge[] {
  const reviewer = agents.find((a) => a.kind === "reviewer");
  const orch = agents.find((a) => a.kind === "orchestrator");
  const out: Edge[] = [];
  for (const c of agents.filter((a) => a.kind === "coder")) {
    if (reviewer) out.push({ from: c.id, to: reviewer.id });
    else if (orch) out.push({ from: c.id, to: orch.id });
  }
  if (reviewer && orch) out.push({ from: reviewer.id, to: orch.id });
  // 規格先行：有產出規格的工程師時，協調者會把拆出來的實作卡派給其他工程師。
  if (orch && agents.some((a) => a.kind === "coder" && a.produces_spec)) {
    for (const c of agents.filter((a) => a.kind === "coder" && !a.produces_spec)) {
      out.push({ from: orch.id, to: c.id, kind: "dispatch" });
    }
  }
  return out;
}

export const EDGE_COLORS = {
  handoff: "#3b82f6",
  review: "#eab308",
  pass: "#22c55e",
  fix: "#f97316",
  dispatch: "#a855f7",
} as const;

/** 模組卡所屬的 Coder。review/fix 卡要透過 module_id 回推。 */
function coderOf(card: TaskRow, byId: Map<string, TaskRow>): string | null {
  if (card.card_kind === "work" || card.card_kind === "fix") {
    const module = card.card_kind === "work" ? card : byId.get(card.module_id ?? "");
    return module?.agent_id ?? null;
  }
  return byId.get(card.module_id ?? "")?.agent_id ?? null;
}

/**
 * 持續流動的光棒：下游正在處理上游交來的東西。
 * - Reviewer 正在審某個 Coder 的模組 → 那條 Coder→Reviewer 黃色
 * - Reviewer 佇列裡有某個 Coder 的模組 → 藍色（交件中）
 * - Orchestrator 正在合併 → 進 Orchestrator 的線全部綠色
 */
export function edgeActivity(edges: Edge[], agents: AgentRow[], tasks: TaskRow[]): Map<string, string> {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const kind = new Map(agents.map((a) => [a.id, a.kind]));
  const out = new Map<string, string>();
  for (const e of edges) {
    const key = `${e.from}>${e.to}`;
    const downstream = tasks.filter((t) => t.agent_id === e.to);
    if (e.kind === "dispatch") {
      // 工程師手上還有從規格拆出來、沒做完的實作卡。
      if (downstream.some((t) => t.from_spec && (t.status === "queued" || t.status === "running"))) {
        out.set(key, EDGE_COLORS.dispatch);
      }
      continue;
    }
    if (kind.get(e.to) === "orchestrator") {
      if (downstream.some((t) => t.status === "running")) out.set(key, EDGE_COLORS.pass);
      continue;
    }
    const fromCoder = (t: TaskRow) => coderOf(t, byId) === e.from;
    if (downstream.some((t) => t.status === "running" && fromCoder(t))) out.set(key, EDGE_COLORS.review);
    else if (downstream.some((t) => t.status === "queued" && fromCoder(t))) out.set(key, EDGE_COLORS.handoff);
  }
  return out;
}

/** 一次性的交接光點。`reverse` = 從下游往上游（退回修正）。 */
export interface Pulse {
  key: string;
  from: string;
  to: string;
  color: string;
  reverse: boolean;
}

/**
 * 比對前後兩次資料，找出剛發生的交接：
 * - 新出現的審查卡：Coder → Reviewer（藍）
 * - 審查剛判定通過：Reviewer → Orchestrator（綠）
 * - 新出現的修正卡：Reviewer → Coder（橘，沿著同一條線反向）
 * - 新出現的合併卡：Reviewer（或 Coder）→ Orchestrator（綠）
 * `prev` 為 null（第一次載入）時不產生任何光點。
 */
export function diffPulses(prev: TaskRow[] | null, next: TaskRow[], agents: AgentRow[]): Pulse[] {
  if (!prev) return [];
  const before = new Map(prev.map((t) => [t.id, t]));
  const byId = new Map(next.map((t) => [t.id, t]));
  const reviewer = agents.find((a) => a.kind === "reviewer");
  const orch = agents.find((a) => a.kind === "orchestrator");
  const out: Pulse[] = [];
  for (const t of next) {
    const old = before.get(t.id);
    const coder = coderOf(t, byId);
    if (!old && t.card_kind === "review" && reviewer && coder) {
      out.push({ key: `${t.id}:in`, from: coder, to: reviewer.id, color: EDGE_COLORS.handoff, reverse: false });
    }
    if (!old && t.card_kind === "fix" && reviewer && coder) {
      out.push({ key: `${t.id}:fix`, from: coder, to: reviewer.id, color: EDGE_COLORS.fix, reverse: true });
    }
    if (old && t.card_kind === "review" && t.verdict === "pass" && old.verdict !== "pass" && reviewer && orch) {
      out.push({ key: `${t.id}:pass`, from: reviewer.id, to: orch.id, color: EDGE_COLORS.pass, reverse: false });
    }
    // 規格通過、交給協調者拆解：Reviewer（沒有 Reviewer 就是規格撰寫師）→ Orchestrator。
    if (!old && t.card_kind === "plan" && orch) {
      const from = reviewer?.id ?? byId.get(t.module_id ?? "")?.agent_id;
      if (from) out.push({ key: `${t.id}:plan`, from, to: orch.id, color: EDGE_COLORS.pass, reverse: false });
    }
    // 協調者把拆出來的實作卡派給 Coder。
    if (!old && t.card_kind === "work" && t.from_spec && t.agent_id && orch) {
      out.push({ key: `${t.id}:dispatch`, from: orch.id, to: t.agent_id, color: EDGE_COLORS.dispatch, reverse: false });
    }
    if (!old && t.card_kind === "merge" && orch) {
      const sources = reviewer ? [reviewer.id] : agents.filter((a) => a.kind === "coder").map((a) => a.id);
      for (const s of sources) {
        out.push({ key: `${t.id}:merge:${s}`, from: s, to: orch.id, color: EDGE_COLORS.pass, reverse: false });
      }
    }
  }
  return out;
}

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * 兩張卡左右分得開：從上游右緣中點畫到下游左緣中點（下游在左邊時反過來）。
 * 左右有重疊（上下疊著）：改接上下緣——否則最後一段與箭頭會畫在下游卡片底下被蓋住。
 */
export function edgePath(a: Box, b: Box): string {
  const overlapX = a.x < b.x + b.w && b.x < a.x + a.w;
  if (overlapX) {
    const x1 = a.x + a.w / 2;
    const x2 = b.x + b.w / 2;
    const down = b.y + b.h / 2 >= a.y + a.h / 2;
    const y1 = down ? a.y + a.h : a.y;
    const y2 = down ? b.y : b.y + b.h;
    const dy = Math.max(40, Math.abs(y2 - y1) / 2) * (down ? 1 : -1);
    return `M${x1},${y1} C${x1},${y1 + dy} ${x2},${y2 - dy} ${x2},${y2}`;
  }
  let x1 = a.x + a.w;
  let x2 = b.x;
  if (b.x + b.w / 2 < a.x + a.w / 2) {
    x1 = a.x;
    x2 = b.x + b.w;
  }
  const y1 = a.y + a.h / 2;
  const y2 = b.y + b.h / 2;
  const dx = Math.max(40, Math.abs(x2 - x1) / 2) * (x2 >= x1 ? 1 : -1);
  return `M${x1},${y1} C${x1 + dx},${y1} ${x2 - dx},${y2} ${x2},${y2}`;
}


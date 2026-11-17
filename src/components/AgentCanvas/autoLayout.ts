import type { AgentKind } from "../../ipc/agents";

/** 由左到右的流程順序：產出規格的工程師（釐清需求、拆工作）→ 其他工程師 → 審核者 → 協調者 → 驗收者。沒放的角色不佔欄。 */
const COLUMNS: ((n: LayoutNode) => boolean)[] = [
  (n) => n.kind === "coder" && !!n.spec,
  (n) => n.kind === "coder" && !n.spec,
  (n) => n.kind === "reviewer",
  (n) => n.kind === "orchestrator",
  (n) => n.kind === "verifier",
];
const ORIGIN = 40;
const COL_GAP = 110;
const ROW_GAP = 40;

export interface LayoutNode {
  id: string;
  kind: AgentKind | "schedule";
  /** 原本的位置：同一欄裡依原本的上下順序排，使用者排過的順序不會被打亂。 */
  x: number;
  y: number;
  w: number;
  h: number;
  /** 產出規格的工程師：排在最前面一欄。 */
  spec?: boolean;
  /** 定時器才有：它叫起的角色。定時器排在那個角色那一欄的左邊、跟它同高。 */
  targetId?: string;
}

const SCHED_GAP = 24;

/**
 * 依流程自動排列：每個角色一欄、欄與欄之間留空給連線；同一欄由上往下排，
 * 每一欄都對齊最高那一欄的垂直中心，連線就不會斜穿整個畫面。
 */
export function autoLayout(nodes: LayoutNode[]): Map<string, { x: number; y: number }> {
  const agents = nodes.filter((n) => n.kind !== "schedule");
  const schedules = nodes.filter((n) => n.kind === "schedule");
  const columns = COLUMNS.map((inCol) => agents.filter(inCol).sort((a, b) => a.y - b.y || a.x - b.x)).filter((col) => col.length > 0);
  const heightOf = (col: LayoutNode[]) => col.reduce((s, n) => s + n.h, 0) + ROW_GAP * (col.length - 1);
  const tallest = Math.max(0, ...columns.map(heightOf));

  // 先算每個角色的 y（跟欄的 x 無關），定時器要對齊它叫起的角色。
  const agentY = new Map<string, number>();
  for (const col of columns) {
    let y = ORIGIN + (tallest - heightOf(col)) / 2;
    for (const n of col) {
      agentY.set(n.id, y);
      y += n.h + ROW_GAP;
    }
  }

  // 定時器歸到它目標所在的那一欄（找不到目標的歸第一欄），排在那一欄的左邊：
  // 連線只跨一小段、不會斜穿別的角色。
  const colIndexOf = new Map<string, number>();
  columns.forEach((col, i) => col.forEach((n) => colIndexOf.set(n.id, i)));
  const byCol = new Map<number, LayoutNode[]>();
  for (const sch of schedules) {
    const i = (sch.targetId && colIndexOf.get(sch.targetId)) || 0;
    byCol.set(i, [...(byCol.get(i) ?? []), sch]);
  }

  const out = new Map<string, { x: number; y: number }>();
  const placeSchedules = (list: LayoutNode[], x: number): number => {
    const target = (n: LayoutNode) => agents.find((a) => a.id === n.targetId);
    const wanted = (n: LayoutNode) => {
      const t = target(n);
      return t ? (agentY.get(t.id) ?? ORIGIN) + t.h / 2 - n.h / 2 : ORIGIN;
    };
    const sorted = [...list].sort((a, b) => wanted(a) - wanted(b) || a.y - b.y);
    let bottom = -Infinity;
    for (const n of sorted) {
      const y = Math.max(wanted(n), bottom + SCHED_GAP);
      out.set(n.id, { x: Math.round(x), y: Math.round(y) });
      bottom = y + n.h;
    }
    return Math.max(...list.map((n) => n.w)) + COL_GAP;
  };

  let x = ORIGIN;
  if (columns.length === 0 && schedules.length > 0) placeSchedules(schedules, x);
  columns.forEach((col, i) => {
    const pre = byCol.get(i);
    if (pre) x += placeSchedules(pre, x);
    for (const n of col) out.set(n.id, { x: Math.round(x), y: Math.round(agentY.get(n.id)!) });
    x += Math.max(...col.map((n) => n.w)) + COL_GAP;
  });
  return out;
}

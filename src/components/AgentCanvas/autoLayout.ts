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
  /** 定時器才有：它叫起的角色。定時器疊在那個角色的正上方（同一欄），不放在欄與欄之間——那裡是交件連線走的地方。 */
  targetId?: string;
}

/** 定時器與它下面的角色之間的距離（比欄內卡片間距小，看得出是一組）。 */
const SCHED_GAP = 24;

/**
 * 依流程自動排列：每個角色一欄、欄與欄之間留空給連線；同一欄由上往下排，
 * 每一欄都對齊最高那一欄的垂直中心，連線就不會斜穿整個畫面。
 */
export function autoLayout(nodes: LayoutNode[]): Map<string, { x: number; y: number }> {
  const agents = nodes.filter((n) => n.kind !== "schedule");
  const schedules = nodes.filter((n) => n.kind === "schedule");
  const columns = COLUMNS.map((inCol) => agents.filter(inCol).sort((a, b) => a.y - b.y || a.x - b.x)).filter((col) => col.length > 0);

  // 定時器疊在目標角色的正上方；找不到目標的疊在第一個角色上面。
  const firstId = columns[0]?.[0]?.id;
  const above = new Map<string, LayoutNode[]>();
  for (const sch of schedules) {
    const id = sch.targetId && agents.some((a) => a.id === sch.targetId) ? sch.targetId : firstId;
    if (!id) continue;
    above.set(id, [...(above.get(id) ?? []), sch].sort((a, b) => a.y - b.y));
  }
  const stackH = (n: LayoutNode) => (above.get(n.id) ?? []).reduce((s, x) => s + x.h + SCHED_GAP, 0);

  // 欄高要把疊在上面的定時器算進去，這樣整欄仍然垂直置中對齊。
  const heightOf = (col: LayoutNode[]) => col.reduce((s, n) => s + stackH(n) + n.h, 0) + ROW_GAP * (col.length - 1);
  const tallest = Math.max(0, ...columns.map(heightOf));

  const out = new Map<string, { x: number; y: number }>();
  let x = ORIGIN;
  for (const col of columns) {
    let y = ORIGIN + (tallest - heightOf(col)) / 2;
    for (const n of col) {
      for (const sch of above.get(n.id) ?? []) {
        out.set(sch.id, { x: Math.round(x), y: Math.round(y) });
        y += sch.h + SCHED_GAP;
      }
      out.set(n.id, { x: Math.round(x), y: Math.round(y) });
      y += n.h + ROW_GAP;
    }
    x += Math.max(...col.map((n) => n.w)) + COL_GAP;
  }
  // 完全沒有角色時，定時器自己疊在左上。
  if (columns.length === 0) {
    let y = ORIGIN;
    for (const sch of [...schedules].sort((a, b) => a.y - b.y)) {
      out.set(sch.id, { x: ORIGIN, y });
      y += sch.h + SCHED_GAP;
    }
  }
  return out;
}

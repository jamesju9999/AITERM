import type { AgentKind } from "../../ipc/agents";

/** 由左到右的流程順序：沒放的角色不佔欄。 */
const COLUMN_ORDER: AgentKind[] = ["coder", "reviewer", "orchestrator", "verifier"];
const ORIGIN = 40;
const COL_GAP = 110;
const ROW_GAP = 40;

export interface LayoutNode {
  id: string;
  kind: AgentKind;
  /** 原本的位置：同一欄裡依原本的上下順序排，使用者排過的順序不會被打亂。 */
  x: number;
  y: number;
  w: number;
  h: number;
}

/**
 * 依流程自動排列：每個角色一欄、欄與欄之間留空給連線；同一欄由上往下排，
 * 每一欄都對齊最高那一欄的垂直中心，連線就不會斜穿整個畫面。
 */
export function autoLayout(nodes: LayoutNode[]): Map<string, { x: number; y: number }> {
  const columns = COLUMN_ORDER.map((k) =>
    nodes.filter((n) => n.kind === k).sort((a, b) => a.y - b.y || a.x - b.x),
  ).filter((col) => col.length > 0);
  const heightOf = (col: LayoutNode[]) => col.reduce((s, n) => s + n.h, 0) + ROW_GAP * (col.length - 1);
  const tallest = Math.max(0, ...columns.map(heightOf));

  const out = new Map<string, { x: number; y: number }>();
  let x = ORIGIN;
  for (const col of columns) {
    let y = ORIGIN + (tallest - heightOf(col)) / 2;
    for (const n of col) {
      out.set(n.id, { x: Math.round(x), y: Math.round(y) });
      y += n.h + ROW_GAP;
    }
    x += Math.max(...col.map((n) => n.w)) + COL_GAP;
  }
  return out;
}

import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { isToolId } from "./toolNodes";

/** 圖上代表「你正在對話的那個 Claude」的虛擬根節點。 */
export const ROOT_ID = "__root__";
/** 根節點只放「Claude」、執行中數量與主 Claude 目前的工具，做窄一點，讓預設寬度的側欄放得下第一層 subagent。 */
export const ROOT_W = 110;
export const CARD_W = 160;
export const CARD_H = 92;
/** 工具卡只有單行（圖示＋指令＋耗時），比 subagent 卡矮很多，同樣高度放得下更多個。 */
export const TOOL_H = 36;
export const GAP_X = 36;
export const GAP_Y = 14;
export const PAD = 12;

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface GraphEdge {
  from: string;
  to: string;
  /** 跟著子節點的狀態：running 的連線會流動。 */
  status: ActivityStatus;
  d: string;
}
export interface Graph {
  boxes: Map<string, Box>;
  edges: GraphEdge[];
  width: number;
  height: number;
}

/** 第 `depth` 欄的左緣：第 0 欄是窄的根，之後每欄 CARD_W。 */
export function colX(depth: number): number {
  return depth === 0 ? PAD : PAD + ROOT_W + GAP_X + (depth - 1) * (CARD_W + GAP_X);
}

/**
 * 由左到右的分層樹狀排版：每深一層往右一欄；葉節點由上往下依序占一列，
 * 父節點垂直置中在子節點中間。卡片高度固定，所以不必量 DOM。
 * 父節點不在清單內（被裁掉、事件遺失）或循環參照的節點接到根，保證每個節點恰好出現一次。
 */
export function layoutGraph(nodes: ActivityNode[]): Graph {
  const ids = new Set(nodes.map((x) => x.id));
  const children = new Map<string, ActivityNode[]>();
  const rootKids: ActivityNode[] = [];
  for (const x of nodes) {
    if (x.parent && ids.has(x.parent) && x.parent !== x.id) {
      const list = children.get(x.parent) ?? [];
      list.push(x);
      children.set(x.parent, list);
    } else {
      rootKids.push(x);
    }
  }
  // 失敗的排在同一層最前面（最需要看到），其餘依開始順序。
  const byStart = (a: ActivityNode, b: ActivityNode) =>
    Number(b.status === "failed") - Number(a.status === "failed") || a.started_ms - b.started_ms;
  for (const list of children.values()) list.sort(byStart);
  // 循環中的節點沒有任何根可達：接到根。
  const reachable = new Set<string>();
  const mark = (x: ActivityNode) => {
    if (reachable.has(x.id)) return;
    reachable.add(x.id);
    for (const c of children.get(x.id) ?? []) mark(c);
  };
  rootKids.forEach(mark);
  for (const x of nodes) {
    if (!reachable.has(x.id)) {
      rootKids.push(x);
      mark(x);
    }
  }

  const heightOf = (id: string) => (isToolId(id) ? TOOL_H : CARD_H);
  /** 各節點中心線的 y；葉節點由上往下堆疊，父節點置中在子節點之間。 */
  const centers = new Map<string, number>();
  const order: string[] = [];
  const depths = new Map<string, number>();
  const edgesRaw: { from: string; to: string; status: ActivityStatus }[] = [];
  let nextY = PAD;
  const placed = new Set<string>();
  const place = (id: string, kids: ActivityNode[], depth: number) => {
    depths.set(id, depth);
    const fresh = kids.filter((k) => !placed.has(k.id));
    if (fresh.length === 0) {
      const h = heightOf(id);
      centers.set(id, nextY + h / 2);
      order.push(id);
      nextY += h + GAP_Y;
      return;
    }
    for (const k of fresh) placed.add(k.id);
    const startY = nextY;
    const startIdx = order.length;
    for (const k of fresh) {
      edgesRaw.push({ from: id, to: k.id, status: k.status });
      place(k.id, children.get(k.id) ?? [], depth + 1);
    }
    // 子節點（例如單行工具卡）疊起來比父卡矮時，父卡會蓋到相鄰的卡：
    // 把整組子孫往下推，讓這一塊至少有父卡自己的高度。
    const own = heightOf(id);
    const span = nextY - GAP_Y - startY;
    if (span < own) {
      const delta = (own - span) / 2;
      for (const d of order.slice(startIdx)) centers.set(d, centers.get(d)! + delta);
      nextY += own - span;
    }
    centers.set(id, (centers.get(fresh[0].id)! + centers.get(fresh[fresh.length - 1].id)!) / 2);
    order.push(id);
  };
  rootKids.sort(byStart);
  place(ROOT_ID, rootKids, 0);

  const boxes = new Map<string, Box>();
  let maxDepth = 0;
  for (const [id, cy] of centers) {
    const depth = depths.get(id)!;
    maxDepth = Math.max(maxDepth, depth);
    const h = heightOf(id);
    boxes.set(id, { x: colX(depth), y: cy - h / 2, w: depth === 0 ? ROOT_W : CARD_W, h });
  }
  const edges: GraphEdge[] = edgesRaw.map((e) => {
    const a = boxes.get(e.from)!;
    const b = boxes.get(e.to)!;
    const x1 = a.x + a.w;
    const y1 = a.y + a.h / 2;
    const x2 = b.x;
    const y2 = b.y + b.h / 2;
    const dx = GAP_X / 2;
    return { ...e, d: `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}` };
  });
  return {
    boxes,
    edges,
    width: colX(maxDepth) + (maxDepth === 0 ? ROOT_W : CARD_W) + PAD,
    height: nextY - GAP_Y + PAD,
  };
}

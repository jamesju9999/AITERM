import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";

/** 圖上代表「你正在對話的那個 Claude」的虛擬根節點。 */
export const ROOT_ID = "__root__";
export const CARD_W = 190;
export const CARD_H = 74;
export const GAP_X = 56;
export const GAP_Y = 14;
export const PAD = 12;

export interface Box {
  x: number;
  y: number;
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
  const byStart = (a: ActivityNode, b: ActivityNode) => a.started_ms - b.started_ms;
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

  const rows = new Map<string, number>();
  const depths = new Map<string, number>();
  const edgesRaw: { from: string; to: string; status: ActivityStatus }[] = [];
  let nextRow = 0;
  const placed = new Set<string>();
  const place = (id: string, kids: ActivityNode[], depth: number) => {
    depths.set(id, depth);
    const fresh = kids.filter((k) => !placed.has(k.id));
    if (fresh.length === 0) {
      rows.set(id, nextRow++);
      return;
    }
    for (const k of fresh) placed.add(k.id);
    for (const k of fresh) {
      edgesRaw.push({ from: id, to: k.id, status: k.status });
      place(k.id, children.get(k.id) ?? [], depth + 1);
    }
    rows.set(id, (rows.get(fresh[0].id)! + rows.get(fresh[fresh.length - 1].id)!) / 2);
  };
  rootKids.sort(byStart);
  place(ROOT_ID, rootKids, 0);

  const boxes = new Map<string, Box>();
  let maxDepth = 0;
  for (const [id, row] of rows) {
    const depth = depths.get(id)!;
    maxDepth = Math.max(maxDepth, depth);
    boxes.set(id, { x: PAD + depth * (CARD_W + GAP_X), y: PAD + row * (CARD_H + GAP_Y) });
  }
  const edges: GraphEdge[] = edgesRaw.map((e) => {
    const a = boxes.get(e.from)!;
    const b = boxes.get(e.to)!;
    const x1 = a.x + CARD_W;
    const y1 = a.y + CARD_H / 2;
    const x2 = b.x;
    const y2 = b.y + CARD_H / 2;
    const dx = GAP_X / 2;
    return { ...e, d: `M ${x1} ${y1} C ${x1 + dx} ${y1}, ${x2 - dx} ${y2}, ${x2} ${y2}` };
  });
  return {
    boxes,
    edges,
    width: PAD * 2 + (maxDepth + 1) * CARD_W + maxDepth * GAP_X,
    height: PAD * 2 + nextRow * CARD_H + (nextRow - 1) * GAP_Y,
  };
}

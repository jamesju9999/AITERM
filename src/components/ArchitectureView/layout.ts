import type { Architecture, Edge } from "./model";

export const NODE_W = 200;
export const NODE_H = 92;
/** 泳道與泳道之間的間隙（同欄連線的凸出弧線就走這裡）。 */
export const COL_GAP = 64;
export const ROW_GAP = 14;
export const PAD = 24;
/** 泳道內側留白（卡片與泳道邊框的距離）。 */
export const LANE_PAD = 14;
/** 泳道標題列高度（含與第一張卡片之間的空隙）。 */
export const HEADER_H = 44;
export const LANE_W = NODE_W + LANE_PAD * 2;
/** 同欄連線向右凸出的距離。 */
export const EDGE_BULGE = 56;
export const MIN_SCALE = 0.6;

export interface Pt { x: number; y: number }
export interface Lane { layerId: string; name: string; x: number; y: number; w: number; h: number; count: number }
export interface Layout {
  nodes: Record<string, Pt>;
  columns: Lane[];
  width: number;
  height: number;
}

export function layoutGraph(arch: Architecture): Layout {
  const nodes: Record<string, Pt> = {};
  const counts = arch.layers.map((l) => arch.modules.filter((m) => m.layer === l.id).length);
  const maxRows = Math.max(0, ...counts);
  const laneH = HEADER_H + maxRows * NODE_H + Math.max(0, maxRows - 1) * ROW_GAP + LANE_PAD;
  const columns: Lane[] = arch.layers.map((layer, col) => {
    const x = PAD + col * (LANE_W + COL_GAP);
    arch.modules.filter((m) => m.layer === layer.id).forEach((m, row) => {
      nodes[m.id] = { x: x + LANE_PAD, y: PAD + HEADER_H + row * (NODE_H + ROW_GAP) };
    });
    return { layerId: layer.id, name: layer.name, x, y: PAD, w: LANE_W, h: laneH, count: counts[col] };
  });
  const n = arch.layers.length;
  return {
    nodes,
    columns,
    width: PAD * 2 + n * LANE_W + Math.max(0, n - 1) * COL_GAP,
    height: PAD * 2 + laneH,
  };
}

/** 舞台比可視區寬時等比縮小（不低於 MIN_SCALE）；量不到尺寸時不縮。 */
export function fitScale(stageWidth: number, viewportWidth: number): number {
  if (stageWidth <= 0 || viewportWidth <= 0 || stageWidth <= viewportWidth) return 1;
  return Math.max(MIN_SCALE, viewportWidth / stageWidth);
}

const FIXED_LAYER_COLORS: Record<string, string> = {
  frontend: "#38bdf8",
  transport: "#a78bfa",
  backend: "#34d399",
  data: "#fbbf24",
  other: "#94a3b8",
};
export const LAYER_PALETTE = ["#f472b6", "#fb923c", "#2dd4bf", "#818cf8", "#a3e635", "#f87171"];

/** 分層的固定代表色；AI 自訂的分層依序號從調色盤取。 */
export function layerColor(layerId: string, index: number): string {
  return FIXED_LAYER_COLORS[layerId] ?? LAYER_PALETTE[((index % LAYER_PALETTE.length) + LAYER_PALETTE.length) % LAYER_PALETTE.length];
}

function edgeGeom(from: Pt, to: Pt): [Pt, Pt, Pt, Pt] {
  const y1 = from.y + NODE_H / 2;
  const y2 = to.y + NODE_H / 2;
  if (from.x === to.x) {
    // 同欄：兩端都從右緣出入，向右凸出到泳道間隙，不會穿過同欄其他卡片。
    const x = from.x + NODE_W;
    return [{ x, y: y1 }, { x: x + EDGE_BULGE, y: y1 }, { x: x + EDGE_BULGE, y: y2 }, { x, y: y2 }];
  }
  const fwd = to.x > from.x;
  const x1 = fwd ? from.x + NODE_W : from.x;
  const x2 = fwd ? to.x : to.x + NODE_W;
  const dx = Math.max(40, Math.abs(x2 - x1) / 2) * (fwd ? 1 : -1);
  return [{ x: x1, y: y1 }, { x: x1 + dx, y: y1 }, { x: x2 - dx, y: y2 }, { x: x2, y: y2 }];
}

/** 不同欄：側邊到側邊的貝茲曲線；同欄：向右凸出的弧線。 */
export function edgePath(from: Pt, to: Pt): string {
  const [a, b, c, d] = edgeGeom(from, to);
  return `M ${a.x} ${a.y} C ${b.x} ${b.y} ${c.x} ${c.y} ${d.x} ${d.y}`;
}

/** 曲線 t=0.5 的位置，用來放連線標籤。 */
export function edgeMidpoint(from: Pt, to: Pt): Pt {
  const [a, b, c, d] = edgeGeom(from, to);
  return { x: (a.x + 3 * b.x + 3 * c.x + d.x) / 8, y: (a.y + 3 * b.y + 3 * c.y + d.y) / 8 };
}

/** 從某模組出發，往上游與下游各走到底，回傳整條呼叫鏈上的模組與連線。 */
export function traceChain(arch: Architecture, start: string): { modules: Set<string>; edges: Edge[] } {
  const modules = new Set<string>([start]);
  const edges = new Set<Edge>();
  const walk = (dir: "down" | "up") => {
    const queue = [start];
    const visited = new Set<string>([start]);
    while (queue.length) {
      const cur = queue.shift()!;
      for (const e of arch.edges) {
        const [near, far] = dir === "down" ? [e.from, e.to] : [e.to, e.from];
        if (near !== cur) continue;
        edges.add(e);
        modules.add(far);
        if (!visited.has(far)) { visited.add(far); queue.push(far); }
      }
    }
  };
  walk("down");
  walk("up");
  return { modules, edges: [...edges] };
}

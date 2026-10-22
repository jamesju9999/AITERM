import type { Architecture, Edge } from "./model";

export const NODE_W = 180;
export const NODE_H = 56;
export const COL_GAP = 120;
export const ROW_GAP = 20;
export const PAD = 24;
const HEADER_H = 40;

export interface Pt { x: number; y: number }
export interface Layout {
  nodes: Record<string, Pt>;
  columns: { layerId: string; name: string; x: number }[];
  width: number;
  height: number;
}

export function layoutGraph(arch: Architecture): Layout {
  const nodes: Record<string, Pt> = {};
  const columns: Layout["columns"] = [];
  let maxRows = 0;
  arch.layers.forEach((layer, col) => {
    const x = PAD + col * (NODE_W + COL_GAP);
    columns.push({ layerId: layer.id, name: layer.name, x });
    const mods = arch.modules.filter((m) => m.layer === layer.id);
    maxRows = Math.max(maxRows, mods.length);
    mods.forEach((m, row) => {
      nodes[m.id] = { x, y: PAD + HEADER_H + row * (NODE_H + ROW_GAP) };
    });
  });
  return {
    nodes,
    columns,
    width: PAD * 2 + arch.layers.length * NODE_W + Math.max(0, arch.layers.length - 1) * COL_GAP,
    height: PAD * 2 + HEADER_H + maxRows * NODE_H + Math.max(0, maxRows - 1) * ROW_GAP,
  };
}

/** 來源右緣中點 → 目標左緣中點；目標在左邊時也畫得出可讀的曲線。 */
export function edgePath(from: Pt, to: Pt): string {
  const x1 = from.x + NODE_W;
  const y1 = from.y + NODE_H / 2;
  const x2 = to.x;
  const y2 = to.y + NODE_H / 2;
  const dx = Math.max(40, Math.abs(x2 - x1) / 2);
  return `M ${x1} ${y1} C ${x1 + dx} ${y1} ${x2 - dx} ${y2} ${x2} ${y2}`;
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

import type { Architecture, Edge, EdgeKind } from "./model";

export interface FlowOpts { maxDepth?: number; maxPaths?: number }
/** 防止稠密圖爆炸：列舉到這個數量就停止（再依長度排序截斷）。 */
const ENUM_CAP = 2000;

function enumerate(edges: Edge[], start: string, dir: "down" | "up", maxDepth: number): Edge[][] {
  const out: Edge[][] = [];
  const onPath = new Set<string>([start]);
  const walk = (cur: string, path: Edge[]) => {
    if (out.length >= ENUM_CAP) return;
    let extended = false;
    if (path.length < maxDepth) {
      for (const e of edges) {
        const [near, far] = dir === "down" ? [e.from, e.to] : [e.to, e.from];
        if (near !== cur || onPath.has(far)) continue;
        extended = true;
        onPath.add(far);
        walk(far, [...path, e]);
        onPath.delete(far);
      }
    }
    // 只收「走到底」的路徑（不收仍可延伸的前綴）。
    if (!extended && path.length > 0) out.push(path);
  };
  walk(start, []);
  return out;
}

/** 由 start 往下游、以及到 start 的上游的簡單路徑（上游路徑的邊依流向排列）；最長者優先，同長依發現順序。 */
export function flowPaths(arch: Architecture, start: string, opts: FlowOpts = {}): { down: Edge[][]; up: Edge[][] } {
  const maxDepth = opts.maxDepth ?? 6;
  const maxPaths = opts.maxPaths ?? 8;
  const rank = (ps: Edge[][]) => ps.map((p, i) => ({ p, i })).sort((a, b) => b.p.length - a.p.length || a.i - b.i).slice(0, maxPaths).map((x) => x.p);
  return {
    down: rank(enumerate(arch.edges, start, "down", maxDepth)),
    up: rank(enumerate(arch.edges, start, "up", maxDepth).map((p) => [...p].reverse())),
  };
}

export interface GroupedEdge { other: string; kind: EdgeKind; mutual: boolean; edges: Edge[] }

/** 與 selfId 相連的邊，依（對方模組, kind）合併；兩個方向都有則 mutual。自環與無關的邊略過。 */
export function groupEdges(edges: Edge[], selfId: string): GroupedEdge[] {
  const groups = new Map<string, GroupedEdge & { out: boolean; inn: boolean }>();
  for (const e of edges) {
    if (e.from === e.to) continue;
    const isOut = e.from === selfId;
    if (!isOut && e.to !== selfId) continue;
    const other = isOut ? e.to : e.from;
    const key = `${other}\u0000${e.kind}`;
    let g = groups.get(key);
    if (!g) groups.set(key, (g = { other, kind: e.kind, mutual: false, edges: [], out: false, inn: false }));
    g.edges.push(e);
    if (isOut) g.out = true; else g.inn = true;
    g.mutual = g.out && g.inn;
  }
  return [...groups.values()].map(({ other, kind, mutual, edges: es }) => ({ other, kind, mutual, edges: es }));
}

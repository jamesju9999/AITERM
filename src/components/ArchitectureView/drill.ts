import type { ScanResult } from "../../ipc/architecture";
import type { Architecture, Edge, Module } from "./model";

/** 鑽取路徑鍵：模組 id 以 \u0000 連接。 */
export function drillKey(path: string[]): string {
  return path.join("\u0000");
}

export function filesOfModule(arch: Architecture, moduleId: string): string[] {
  return arch.modules.find((m) => m.id === moduleId)?.files ?? [];
}

function commonDirPrefix(files: string[]): string[] {
  if (files.length === 0) return [];
  const dirs = files.map((f) => f.split("/").slice(0, -1));
  const first = dirs[0];
  let n = first.length;
  for (const d of dirs) {
    let i = 0;
    while (i < n && i < d.length && d[i] === first[i]) i++;
    n = i;
  }
  return first.slice(0, n);
}

/**
 * 只靠掃描結果產生模組內部的圖。節點＝共同前綴下的第一層目錄（群）或直接在前綴下的檔案；
 * 欄位＝import 最長路徑深度（呼叫者在左）。環：DFS 遇到回邊就不計入深度，連線仍保留。
 * 取捨：超過 maxNodes 時依度數（相異連線數）降冪、同度數依 id 字母序保留。
 */
export function buildDrillGraph(scan: ScanResult, files: string[], maxNodes = 60): { arch: Architecture; hidden: number } {
  const prefix = commonDirPrefix(files);
  const nodeOf = new Map<string, string>(); // 檔案路徑 -> 節點 id
  const nodeFiles = new Map<string, string[]>();
  const nodeName = new Map<string, string>();
  for (const f of files) {
    const segs = f.split("/");
    const rel = segs.slice(prefix.length);
    const id = rel.length > 1 ? [...prefix, rel[0]].join("/") : f;
    nodeOf.set(f, id);
    nodeName.set(id, rel.length > 1 ? rel[0] : segs[segs.length - 1]);
    (nodeFiles.get(id) ?? nodeFiles.set(id, []).get(id)!).push(f);
  }

  // 彙整連線（兩端都在 files 內且分屬不同節點）。
  const pairs = new Map<string, [string, string]>();
  for (const e of scan.edges) {
    const a = nodeOf.get(e.from);
    const b = nodeOf.get(e.to);
    if (a === undefined || b === undefined || a === b) continue;
    pairs.set(`${a}\u0000${b}`, [a, b]);
  }

  // 超過上限：取度數最高者。
  let keep = [...nodeFiles.keys()];
  let hidden = 0;
  if (keep.length > maxNodes) {
    const deg = new Map<string, number>();
    for (const [a, b] of pairs.values()) {
      deg.set(a, (deg.get(a) ?? 0) + 1);
      deg.set(b, (deg.get(b) ?? 0) + 1);
    }
    keep = keep
      .sort((x, y) => (deg.get(y) ?? 0) - (deg.get(x) ?? 0) || (x < y ? -1 : x > y ? 1 : 0))
      .slice(0, maxNodes);
    hidden = nodeFiles.size - keep.length;
  }
  const kept = new Set(keep);
  const order = [...nodeFiles.keys()].filter((id) => kept.has(id));
  const edgePairs = [...pairs.values()].filter(([a, b]) => kept.has(a) && kept.has(b));

  // 深度：DFS 找回邊，其餘邊依拓樸順序做最長路徑。
  const out = new Map<string, string[]>();
  const indeg = new Map<string, number>();
  for (const id of order) { out.set(id, []); indeg.set(id, 0); }
  for (const [a, b] of edgePairs) { out.get(a)!.push(b); indeg.set(b, indeg.get(b)! + 1); }
  const state = new Map<string, 1 | 2>(); // 1 走訪中、2 完成
  const forward = new Map<string, string[]>(); // 去掉回邊後的邊
  const post: string[] = [];
  const dfs = (u: string) => {
    state.set(u, 1);
    forward.set(u, []);
    for (const v of out.get(u)!) {
      if (state.get(v) === 1) continue; // 回邊
      forward.get(u)!.push(v);
      if (!state.has(v)) dfs(v);
    }
    state.set(u, 2);
    post.push(u);
  };
  for (const id of order) if (indeg.get(id) === 0 && !state.has(id)) dfs(id);
  for (const id of order) if (!state.has(id)) dfs(id);
  const depth = new Map<string, number>(order.map((id) => [id, 0]));
  for (const u of [...post].reverse()) {
    for (const v of forward.get(u)!) depth.set(v, Math.max(depth.get(v)!, depth.get(u)! + 1));
  }

  const maxDepth = Math.max(-1, ...depth.values());
  const layers = Array.from({ length: maxDepth + 1 }, (_, i) => ({ id: `d${i}`, name: `第 ${i + 1} 層` }));
  const modules: Module[] = order.map((id) => ({
    id, layer: `d${depth.get(id)}`, name: nodeName.get(id)!, files: nodeFiles.get(id)!, tech: [], summary: "", dataflow: "",
  }));
  const edges: Edge[] = edgePairs.map(([from, to]) => ({ from, to, kind: "import", source: "scan", confidence: 1 }));
  return {
    arch: { layers, modules, edges, meta: { scannedAt: new Date().toISOString(), gitHead: scan.git_head, analyzed: false, dropped: 0 } },
    hidden,
  };
}

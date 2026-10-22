import type { ScanResult } from "../../ipc/architecture";

export type EdgeKind = "import" | "ipc" | "http" | "sql";

export interface Layer { id: string; name: string }
export interface Module {
  id: string;
  layer: string;
  name: string;
  files: string[];
  tech: string[];
  summary: string;
  dataflow: string;
}
export interface Edge {
  from: string;
  to: string;
  kind: EdgeKind;
  source: "scan" | "ai";
  confidence: number;
  evidence?: { file: string; line: number };
  note?: string;
}
export interface Architecture {
  layers: Layer[];
  modules: Module[];
  edges: Edge[];
  meta: { scannedAt: string; gitHead: string | null; analyzed: boolean; dropped: number };
}

export const BASE_LAYERS: Layer[] = [
  { id: "frontend", name: "前端" },
  { id: "transport", name: "傳輸 / API" },
  { id: "backend", name: "後端" },
  { id: "data", name: "資料存取" },
  { id: "other", name: "其他" },
];

export function moduleIdOf(path: string): string {
  const segs = path.split("/");
  if (segs.length === 1) return "(root)";
  return segs.slice(0, Math.min(2, segs.length - 1)).join("/");
}

const RULES: [string, RegExp][] = [
  ["data", /(^|\/)(db|database|models?|repo|repositories|migrations?|schema|store|dao|orm)(\/|$)/i],
  ["transport", /(^|\/)(ipc|api|routes?|router|controllers?|rpc|graphql)(\/|$)/i],
  ["frontend", /(^|\/)(components?|pages?|views?|ui|web|client|frontend|app|hooks|styles)(\/|$)|^src$/i],
  ["backend", /(^|\/)(server|backend|services?|commands|src-tauri|core|handlers?)(\/|$)/i],
];

export function guessLayer(moduleId: string): string {
  for (const [layer, re] of RULES) if (re.test(moduleId)) return layer;
  return "other";
}

/** 只用掃描結果產生的預設圖：分層靠路徑關鍵字猜，連線全是實線 import。 */
export function buildBaseGraph(scan: ScanResult): Architecture {
  const byModule = new Map<string, string[]>();
  for (const f of scan.files) {
    const id = moduleIdOf(f.path);
    (byModule.get(id) ?? byModule.set(id, []).get(id)!).push(f.path);
  }
  const modules: Module[] = [...byModule.entries()].map(([id, files]) => ({
    id, layer: guessLayer(id), name: id, files, tech: [], summary: "", dataflow: "",
  }));
  const seen = new Set<string>();
  const edges: Edge[] = [];
  for (const e of scan.edges) {
    const from = moduleIdOf(e.from);
    const to = moduleIdOf(e.to);
    const key = `${from}→${to}`;
    if (from === to || seen.has(key)) continue;
    seen.add(key);
    edges.push({ from, to, kind: "import", source: "scan", confidence: 1 });
  }
  const used = new Set(modules.map((m) => m.layer));
  return {
    layers: BASE_LAYERS.filter((l) => used.has(l.id)),
    modules,
    edges,
    meta: { scannedAt: new Date().toISOString(), gitHead: scan.git_head, analyzed: false, dropped: 0 },
  };
}

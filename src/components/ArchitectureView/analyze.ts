import { stripFence } from "../TaskBoard/refinePrompts";
import type { ScanResult } from "../../ipc/architecture";
import type { Architecture, Edge, EdgeKind, Module } from "./model";

export const ANALYZE_SYSTEM = [
  "你是軟體架構分析師。根據專案的檔案概況、依賴清單與關鍵檔案內容，把專案切成「前端 → 傳輸（IPC／API）→ 後端 → 資料存取」等層，並找出跨層呼叫。",
  "只輸出一個 JSON 物件，不要程式碼區塊、不要說明。格式：",
  '{"layers":[{"id":"frontend","name":"前端"}],',
  ' "modules":[{"id":"短英文id","layer":"layers 裡的 id","name":"顯示名稱","files":["檔案路徑或以 / 結尾的目錄"],"tech":["技術或框架"],"summary":"一句話用途","dataflow":"資料如何進出，2-3 句"}],',
  ' "edges":[{"from":"模組id","to":"模組id","kind":"ipc|http|sql|import","confidence":0.0到1.0,"evidence":{"file":"檔案路徑","line":行號},"note":"這條連線在做什麼"}]}',
  "規則：",
  "- files 只能使用我提供的檔案路徑或其目錄，不可虛構。",
  "- 每條 edge 必須附 evidence（真實檔案與行號）；沒有把握就不要寫。",
  "- confidence 反映你對這條連線的把握。",
  "- 用繁體中文寫 name／summary／dataflow／note。",
].join("\n");

const KINDS: EdgeKind[] = ["import", "ipc", "http", "sql"];
const KEY_RE = /(main|index|app|server|routes?|router|controller|command|handler|api|db|database|schema|model|store|lib)\b/i;

export function pickKeyFiles(scan: ScanResult, max = 12): string[] {
  const scored = scan.files
    .map((f) => ({ path: f.path, score: (KEY_RE.test(f.path.split("/").pop() ?? "") ? 2 : 0) + (f.lang === "sql" ? 2 : 0) + (f.path.split("/").length <= 2 ? 1 : 0) }))
    .filter((x) => x.score > 0)
    .sort((a, b) => b.score - a.score || a.path.localeCompare(b.path));
  return scored.slice(0, max).map((x) => x.path);
}

export function buildAnalyzeMessage(scan: ScanResult, base: Architecture, files: [string, string][]): string {
  const mods = base.modules
    .map((m) => `- ${m.id}（${m.files.length} 檔）：${m.files.slice(0, 4).join(", ")}${m.files.length > 4 ? " …" : ""}`)
    .join("\n");
  const imports = base.edges.map((e) => `${e.from} → ${e.to}`).join("\n") || "（無）";
  const body = files.map(([p, c]) => `### ${p}\n${c}`).join("\n\n");
  return [
    `依賴：${scan.deps.join(", ") || "（無）"}`,
    `檔案概況（目錄分組）：\n${mods}`,
    `已確認的 import 關係（模組層級）：\n${imports}`,
    `關鍵檔案內容：\n${body}`,
  ].join("\n\n");
}

interface RawAnalysis {
  layers?: { id?: unknown; name?: unknown }[];
  modules?: Record<string, unknown>[];
  edges?: Record<string, unknown>[];
}

export function parseAnalysis(reply: string, scan: ScanResult): Architecture | null {
  let raw: RawAnalysis;
  try {
    raw = JSON.parse(stripFence(reply.trim()).trim()) as RawAnalysis;
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.modules)) return null;

  let dropped = 0;
  const lineCount = new Map(scan.files.map((f) => [f.path, f.lines]));
  const layers = (raw.layers ?? [])
    .filter((l) => typeof l.id === "string" && typeof l.name === "string")
    .map((l) => ({ id: l.id as string, name: l.name as string }));
  const layerIds = new Set(layers.map((l) => l.id));

  const modules: Module[] = [];
  for (const m of raw.modules) {
    const id = typeof m.id === "string" ? m.id : "";
    const layer = typeof m.layer === "string" ? m.layer : "";
    const patterns = Array.isArray(m.files) ? m.files.filter((x): x is string => typeof x === "string") : [];
    const files = scan.files
      .map((f) => f.path)
      .filter((p) => patterns.some((pat) => (pat.endsWith("/") ? p.startsWith(pat) : p === pat)));
    if (!id || !layerIds.has(layer) || files.length === 0) { dropped++; continue; }
    modules.push({
      id, layer, files,
      name: typeof m.name === "string" ? m.name : id,
      tech: Array.isArray(m.tech) ? m.tech.filter((x): x is string => typeof x === "string") : [],
      summary: typeof m.summary === "string" ? m.summary : "",
      dataflow: typeof m.dataflow === "string" ? m.dataflow : "",
    });
  }
  const moduleIds = new Set(modules.map((m) => m.id));

  const edges: Edge[] = [];
  for (const e of raw.edges ?? []) {
    const ev = e.evidence as { file?: unknown; line?: unknown } | undefined;
    const kind = e.kind as EdgeKind;
    const total = typeof ev?.file === "string" ? lineCount.get(ev.file) : undefined;
    const ok =
      typeof e.from === "string" && typeof e.to === "string" &&
      moduleIds.has(e.from) && moduleIds.has(e.to) &&
      KINDS.includes(kind) &&
      total !== undefined && typeof ev?.line === "number" && ev.line >= 1 && ev.line <= total;
    if (!ok) { dropped++; continue; }
    const c = typeof e.confidence === "number" ? e.confidence : 0.5;
    edges.push({
      from: e.from as string, to: e.to as string, kind, source: "ai",
      confidence: Math.min(1, Math.max(0, c)),
      evidence: { file: ev!.file as string, line: ev!.line as number },
      note: typeof e.note === "string" ? e.note : undefined,
    });
  }

  return {
    layers, modules, edges,
    meta: { scannedAt: new Date().toISOString(), gitHead: scan.git_head, analyzed: true, dropped },
  };
}

/** 把掃描確認的檔案層級 import 映射成 AI 模組間的實線；模組內部的不算。 */
export function mergeScanEdges(ai: Architecture, scan: ScanResult): Architecture {
  const fileToModule = new Map<string, string>();
  for (const m of ai.modules) for (const f of m.files) fileToModule.set(f, m.id);
  const seen = new Set<string>();
  const scanEdges: Edge[] = [];
  for (const e of scan.edges) {
    const from = fileToModule.get(e.from);
    const to = fileToModule.get(e.to);
    if (!from || !to || from === to) continue;
    const key = `${from}→${to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    scanEdges.push({ from, to, kind: "import", source: "scan", confidence: 1 });
  }
  // AI 的 import 連線若掃描已確認同一對模組，只留 scan 那條。
  const aiEdges = ai.edges.filter((e) => !(e.kind === "import" && seen.has(`${e.from}→${e.to}`)));
  return { ...ai, edges: [...scanEdges, ...aiEdges] };
}

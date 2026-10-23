import { stripFence } from "../TaskBoard/refinePrompts";
import type { ScanResult } from "../../ipc/architecture";
import type { Architecture, Edge } from "./model";

export const SELECT_SYSTEM = [
  "你是軟體架構分析師。使用者會問一個關於這個專案架構的問題，我會提供架構摘要與全部可讀的檔案路徑。",
  "你的工作只有一件事：挑出回答這個問題最需要讀的模組與檔案。",
  "只輸出一個 JSON 物件，不要程式碼區塊、不要說明：",
  '{"modules":["模組id"],"files":["檔案路徑"]}',
  "規則：",
  "- modules 只能用摘要裡出現的模組 id。",
  "- files 只能用我提供的路徑，最多 8 個，挑與問題最相關的。",
].join("\n");

export const ANSWER_SYSTEM = [
  "你是軟體架構分析師。根據架構摘要與我提供的檔案內容，回答使用者關於這個專案架構的問題。",
  "只輸出一個 JSON 物件，不要程式碼區塊、不要額外說明。格式：",
  '{"answer":"Markdown 回答",',
  ' "walkthrough":{"title":"導覽標題","steps":[{"module":"模組id","caption":"這一步在做什麼","evidence":{"file":"檔案路徑","line":行號}}]},',
  ' "diagram":"Mermaid 原始碼"}',
  "規則：",
  "- answer 是 Markdown，使用繁體中文，直接回答問題。",
  "- walkthrough.steps[].module 必須是我提供的模組 id，並依資料流／控制流的先後順序排列，3 到 10 步。",
  "- evidence 是選填；只有在你確實從我提供的檔案看到時才寫，file:line 必須真實，不可虛構。",
  "- diagram 是 Mermaid 原始碼（sequenceDiagram 或 flowchart LR），不要加 markdown 程式碼圍欄，節點 id 只用 ASCII 英數字與底線。",
  "- 如果問題與流程無關，walkthrough 的 steps 給空陣列、diagram 給空字串。",
].join("\n");

const SUMMARY_MAX = 12000;

function edgeLine(e: Edge): string {
  const parts: string[] = [e.source];
  if (e.note) parts.push(e.note);
  if (e.evidence) parts.push(`${e.evidence.file}:${e.evidence.line}`);
  return `${e.from} -${e.kind}-> ${e.to} (${parts.join(", ")})`;
}

/** 緊湊的架構摘要；超過 12000 字時先砍連線（模組永遠完整），並註明省略幾條。 */
export function archSummary(arch: Architecture): string {
  const mods = ["模組（id | 名稱 | 層 | 摘要 | 技術）："].concat(
    arch.modules.map((m) => `${m.id} | ${m.name} | ${m.layer} | ${m.summary} | ${m.tech.join(", ")}`),
  );
  let text = mods.join("\n");
  if (arch.edges.length === 0) return text;
  text += "\n連線：";
  const lines = arch.edges.map(edgeLine);
  // 預留省略說明的空間，保證最終長度不超過上限。
  const reserve = 30;
  let shown = 0;
  for (const l of lines) {
    if (text.length + 1 + l.length + reserve > SUMMARY_MAX) break;
    text += "\n" + l;
    shown++;
  }
  if (shown < lines.length) text += `\n…省略 ${lines.length - shown} 條連線`;
  return text;
}

export function buildSelectMessage(arch: Architecture, question: string): string {
  const paths = [...new Set(arch.modules.flatMap((m) => m.files))].join("\n");
  return [`架構摘要：\n${archSummary(arch)}`, `可用檔案路徑：\n${paths}`, `問題：${question}`].join("\n\n");
}

export function buildAnswerMessage(arch: Architecture, question: string, files: [string, string][]): string {
  const body = files.map(([p, c]) => `### ${p}\n${c}`).join("\n\n");
  return [`架構摘要：\n${archSummary(arch)}`, `檔案內容：\n${body || "（無）"}`, `問題：${question}`].join("\n\n");
}

function parseObject(reply: string): Record<string, unknown> | null {
  try {
    const v: unknown = JSON.parse(stripFence(reply.trim()).trim());
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const MAX_FILES = 8;
const FALLBACK_FILES_PER_MODULE = 3;

export function parseSelect(reply: string, arch: Architecture, scan: ScanResult): { modules: string[]; files: string[] } {
  const raw = parseObject(reply);
  if (!raw) return { modules: [], files: [] };
  const known = new Map(arch.modules.map((m) => [m.id, m]));
  const inScan = new Set(scan.files.map((f) => f.path));
  const strs = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);

  const modules = [...new Set(strs(raw.modules).filter((id) => known.has(id)))];
  const files = [...new Set(strs(raw.files).filter((p) => inScan.has(p)))];
  // 選了模組卻沒有任何有效檔案屬於它：補它自己的前幾個檔案，避免第二段讀不到東西。
  for (const id of modules) {
    const own = known.get(id)!.files;
    if (files.some((p) => own.includes(p))) continue;
    for (const p of own.filter((q) => inScan.has(q)).slice(0, FALLBACK_FILES_PER_MODULE)) {
      if (!files.includes(p)) files.push(p);
    }
  }
  return { modules, files: files.slice(0, MAX_FILES) };
}

export interface TourStep { module: string; caption: string; evidence?: { file: string; line: number } }
export interface Tour { title: string; steps: TourStep[] }
export interface QaAnswer { answer: string; tour: Tour | null; diagram: string; dropped: number }

const DEFAULT_TOUR_TITLE = "流程導覽";

export function parseAnswer(reply: string, arch: Architecture, scan: ScanResult): QaAnswer | null {
  const raw = parseObject(reply);
  if (!raw || typeof raw.answer !== "string" || !raw.answer.trim()) return null;

  const moduleIds = new Set(arch.modules.map((m) => m.id));
  const lineCount = new Map(scan.files.map((f) => [f.path, f.lines]));
  const wt = raw.walkthrough && typeof raw.walkthrough === "object" ? (raw.walkthrough as Record<string, unknown>) : null;
  const rawSteps = wt && Array.isArray(wt.steps) ? (wt.steps as unknown[]) : [];

  // dropped 只計「整步被丟掉」；只有 evidence 無效時保留該步、不計入。
  let dropped = 0;
  const steps: TourStep[] = [];
  for (const s of rawSteps) {
    const o = s && typeof s === "object" ? (s as Record<string, unknown>) : null;
    if (!o || typeof o.module !== "string" || !moduleIds.has(o.module) || typeof o.caption !== "string" || !o.caption.trim()) {
      dropped++;
      continue;
    }
    const step: TourStep = { module: o.module, caption: o.caption };
    const ev = o.evidence && typeof o.evidence === "object" ? (o.evidence as Record<string, unknown>) : null;
    if (ev && typeof ev.file === "string" && typeof ev.line === "number" && Number.isInteger(ev.line)) {
      const total = lineCount.get(ev.file);
      if (total !== undefined && ev.line >= 1 && ev.line <= total) step.evidence = { file: ev.file, line: ev.line };
    }
    steps.push(step);
  }

  const title = wt && typeof wt.title === "string" && wt.title.trim() ? wt.title : DEFAULT_TOUR_TITLE;
  const diagram = typeof raw.diagram === "string" ? stripFence(raw.diagram).trim() : "";
  return { answer: raw.answer, tour: steps.length >= 2 ? { title, steps } : null, diagram, dropped };
}

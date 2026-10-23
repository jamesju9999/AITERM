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

type JsonObject = Record<string, unknown>;

/** JSON.parse；結果是一般物件回物件，是其他合法 JSON（陣列、null、數字…）回 "other"，語法錯誤回 undefined。 */
function tryParse(s: string): JsonObject | "other" | undefined {
  try {
    const v: unknown = JSON.parse(s);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as JsonObject) : "other";
  } catch {
    return undefined;
  }
}

interface Scanned { out: string; stack: string[]; inString: boolean; esc: boolean; ok: boolean; commas: number[] }

/**
 * 單趟掃描：把字串字面值裡的真實換行／tab 轉成跳脫序列、拿掉 } ] 前的尾逗號，
 * 同時記下尚未關閉的括號堆疊（截斷救援用）。括號對不上時 ok=false。
 */
function scanJson(text: string): Scanned {
  let out = "";
  const stack: string[] = [];
  const commas: number[] = [];
  let inString = false;
  let esc = false;
  let ok = true;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (esc) { out += ch; esc = false; }
      else if (ch === "\\") { out += ch; esc = true; }
      else if (ch === '"') { out += ch; inString = false; }
      else if (ch === "\n") out += "\\n";
      else if (ch === "\r") out += "\\r";
      else if (ch === "\t") out += "\\t";
      else out += ch;
      continue;
    }
    if (ch === '"') { inString = true; out += ch; }
    else if (ch === "{") { stack.push("}"); out += ch; }
    else if (ch === "[") { stack.push("]"); out += ch; }
    else if (ch === "}" || ch === "]") {
      let j = out.length;
      while (j > 0 && /\s/.test(out[j - 1])) j--;
      if (j > 0 && out[j - 1] === ",") out = out.slice(0, j - 1);
      if (stack.pop() !== ch) ok = false;
      out += ch;
    } else if (ch === ",") { commas.push(out.length); out += ch; }
    else out += ch;
  }
  return { out, stack, inString, esc, ok, commas };
}

/** 把被截斷的掃描結果補成完整 JSON 文字：先關字串，再依相反順序關括號。 */
function closeJson(sc: Scanned): string {
  let s = sc.out;
  if (sc.inString) {
    if (sc.esc) s = s.slice(0, -1); // 結尾懸著一個反斜線
    s = s.replace(/\\u[0-9a-fA-F]{0,3}$/, ""); // 被截斷的 \uXXXX
    s += '"';
  } else {
    s = s.replace(/[\s,]+$/, "");
  }
  return s + [...sc.stack].reverse().join("");
}

function salvageTruncated(text: string): JsonObject | null {
  const sc = scanJson(text);
  if (!sc.ok || (sc.stack.length === 0 && !sc.inString)) return null; // 結構錯亂，或根本沒截斷（那就是別的問題）
  const first = tryParse(closeJson(sc));
  if (first && first !== "other") return first;
  // 結尾落在半個鍵或冒號後：逐步退回到前一個逗號再補括號。
  const cuts = sc.commas.filter((c) => c < sc.out.length).reverse().slice(0, 200);
  for (const c of cuts) {
    const prefix = scanJson(sc.out.slice(0, c));
    const r = tryParse(closeJson(prefix));
    if (r && r !== "other") return r;
  }
  return null;
}

/** 從 start 的 { 開始，字串感知地找到使深度歸零的 }；找不到回 -1。 */
function balancedEnd(text: string, start: number): number {
  let depth = 0;
  let inString = false;
  for (let i = start; i < text.length; i++) {
    const ch = text[i];
    if (inString) {
      if (ch === "\\") i++;
      else if (ch === '"') inString = false;
    } else if (ch === '"') inString = true;
    else if (ch === "{" || ch === "[") depth++;
    else if (ch === "}" || ch === "]") {
      depth--;
      if (depth === 0) return ch === "}" ? i : -1;
    }
  }
  return -1;
}

/**
 * 從 AI 回覆撈出一個 JSON 物件。依序嘗試：
 * (a) 去圍欄後直接解；(b) 第一個 { 到最後一個 }（前後有說明文字、圍欄不是整份回覆）；
 * (c) 修補：字串內的真實換行／tab、尾逗號；(d) 被輸出上限截斷：補上缺的引號與括號。
 * 陣列或其他非物件的合法 JSON 回 null。
 */
export function extractJsonObject(reply: string): JsonObject | null {
  return extractJson(reply).value;
}

/** 同 extractJsonObject，另外回報物件是否靠 (d) 補括號才解開（= 原文被截斷）。 */
function extractJson(reply: string): { value: JsonObject | null; salvaged: boolean } {
  const none = (value: JsonObject | null) => ({ value, salvaged: false });
  const text = stripFence(reply.trim()).trim();
  if (!text) return none(null);
  const direct = tryParse(text);
  if (direct) return none(direct === "other" ? null : direct);

  const start = text.indexOf("{");
  if (start === -1) return none(null);
  // 候選結尾：括號配對收口的位置（後面還有帶花括號的說明文字也不怕），再來是最後一個 }。
  const ends = [balancedEnd(text, start), text.lastIndexOf("}")].filter((e, i, a) => e > start && a.indexOf(e) === i);
  for (const end of ends) {
    const sub = text.slice(start, end + 1);
    const b = tryParse(sub);
    if (b && b !== "other") return none(b);
    const sc = scanJson(sub);
    if (sc.ok && sc.stack.length === 0 && !sc.inString) {
      const c = tryParse(sc.out);
      if (c && c !== "other") return none(c);
    }
  }
  const salvaged = salvageTruncated(text.slice(start));
  return { value: salvaged, salvaged: salvaged !== null };
}

/** 結構壞到修不了時的最後手段：只把 "answer" 的字串值寬鬆地讀出來。 */
function looseAnswer(text: string): string | null {
  const m = /"answer"\s*:\s*"/.exec(text);
  if (!m) return null;
  const closing = /\s*(?:,|\}|$)/y;
  const simple: Record<string, string> = { n: "\n", t: "\t", r: "\r", b: "\b", f: "\f" };
  let out = "";
  let i = m.index + m[0].length;
  while (i < text.length) {
    const c = text[i];
    if (c === "\\") {
      const n = text[i + 1];
      if (n === "u" && /^[0-9a-fA-F]{4}$/.test(text.slice(i + 2, i + 6))) { out += String.fromCharCode(parseInt(text.slice(i + 2, i + 6), 16)); i += 6; }
      else { out += n === undefined ? "" : (simple[n] ?? n); i += 2; }
    } else if (c === '"') {
      closing.lastIndex = i + 1;
      if (closing.test(text)) break;
      out += c; // 沒跳脫的內嵌引號：容忍
      i++;
    } else { out += c; i++; }
  }
  return out.trim() ? out : null;
}

const MAX_FILES = 8;
const FALLBACK_FILES_PER_MODULE = 3;

export function parseSelect(reply: string, arch: Architecture, scan: ScanResult): { modules: string[]; files: string[] } {
  const raw = extractJsonObject(reply);
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
export interface QaAnswer { answer: string; tour: Tour | null; diagram: string; dropped: number; truncated: boolean }

const DEFAULT_TOUR_TITLE = "流程導覽";

export function parseAnswer(reply: string, arch: Architecture, scan: ScanResult): QaAnswer | null {
  const { value: raw, salvaged } = extractJson(reply);
  if (raw) {
    if (typeof raw.answer === "string" && raw.answer.trim()) return buildAnswer(raw, raw.answer, arch, scan, salvaged);
    // 解得出物件但沒有 answer：可能是截斷在 answer 之前；嘗試寬鬆掃描，否則視為失敗。
    const loose = looseAnswer(stripFence(reply.trim()));
    return loose ? { answer: loose, tour: null, diagram: "", dropped: 0, truncated: salvaged } : null;
  }
  const text = stripFence(reply.trim()).trim();
  if (!text) return null;
  const loose = looseAnswer(text);
  if (loose) {
    // 掃得出 answer 但整個物件解不開：物件沒收口才算被截斷。
    const start = text.indexOf("{");
    return { answer: loose, tour: null, diagram: "", dropped: 0, truncated: start !== -1 && balancedEnd(text, start) === -1 };
  }
  // 看起來是失敗的 JSON 嘗試（或根本是合法的非物件 JSON）：讓 UI 顯示錯誤，而不是把半截 JSON 當答案。
  if (/^(?:\{|\[|```\s*json\b)/i.test(reply.trim()) || tryParse(text) !== undefined) return null;
  return { answer: text, tour: null, diagram: "", dropped: 0, truncated: false };
}

function buildAnswer(raw: JsonObject, answer: string, arch: Architecture, scan: ScanResult, truncated: boolean): QaAnswer {
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
  return { answer, tour: steps.length >= 2 ? { title, steps } : null, diagram, dropped, truncated };
}

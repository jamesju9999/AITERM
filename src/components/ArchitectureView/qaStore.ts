import type { QaMessage } from "./QaPanel";
import type { QaAnswer, Tour, TourStep } from "./qa";

const VERSION = 1;
/** 記錄檔最多留幾則訊息（問與答各算一則），超過就丟最舊的。 */
export const MAX_STORED = 100;

type Stored = Exclude<QaMessage, { role: "error" }>;

/** 只存使用者與 AI 的訊息；錯誤泡泡是一次性的，不存。 */
export function serializeQa(messages: QaMessage[]): string {
  const keep = messages.filter((m): m is Stored => m.role !== "error").slice(-MAX_STORED);
  return JSON.stringify({ v: VERSION, messages: keep });
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

function parseStep(v: unknown): TourStep | null {
  if (!isObj(v) || typeof v.module !== "string" || typeof v.caption !== "string") return null;
  const ev = v.evidence;
  if (isObj(ev) && typeof ev.file === "string" && typeof ev.line === "number") {
    return { module: v.module, caption: v.caption, evidence: { file: ev.file, line: ev.line } };
  }
  return { module: v.module, caption: v.caption };
}

function parseTour(v: unknown): Tour | null {
  if (!isObj(v) || !Array.isArray(v.steps)) return null;
  const steps = v.steps.map(parseStep).filter((s): s is TourStep => s !== null);
  return steps.length > 0 ? { title: typeof v.title === "string" ? v.title : "", steps } : null;
}

function parseQa(v: unknown): QaAnswer | null {
  if (!isObj(v) || typeof v.answer !== "string") return null;
  return {
    answer: v.answer,
    tour: parseTour(v.tour),
    diagram: typeof v.diagram === "string" ? v.diagram : "",
    dropped: typeof v.dropped === "number" ? v.dropped : 0,
    truncated: v.truncated === true,
  };
}

/** 讀回記錄檔；壞掉的整份或個別項目一律丟掉，不拋例外。 */
export function parseStoredQa(json: string): QaMessage[] {
  let raw: unknown;
  try { raw = JSON.parse(json); } catch { return []; }
  if (!isObj(raw) || !Array.isArray(raw.messages)) return [];
  const out: QaMessage[] = [];
  for (const m of raw.messages) {
    if (!isObj(m) || typeof m.id !== "string") continue;
    if (m.role === "user" && typeof m.text === "string") out.push({ id: m.id, role: "user", text: m.text });
    else if (m.role === "ai") {
      const qa = parseQa(m.qa);
      if (qa) out.push({ id: m.id, role: "ai", qa });
    }
  }
  return out.slice(-MAX_STORED);
}

export interface MdLabels { title: string; exportedAt: string; question: string; tour: string; diagram: string; truncated: string }

/** 比內容裡任何連續反引號都長的圍欄，內容含 ``` 也不會提早關閉。 */
function fence(body: string): string {
  const longest = Math.max(2, ...(body.match(/`+/g) ?? []).map((s) => s.length));
  return "`".repeat(longest + 1);
}

const pad = (n: number) => String(n).padStart(2, "0");

/** 整段對話轉成 Markdown 文件。moduleName 把導覽步驟的模組 id 換成看得懂的名稱。 */
export function qaToMarkdown(
  messages: QaMessage[],
  project: string,
  when: Date,
  L: MdLabels,
  moduleName: (id: string) => string = (id) => id,
): string {
  const stamp = `${when.getFullYear()}-${pad(when.getMonth() + 1)}-${pad(when.getDate())} ${pad(when.getHours())}:${pad(when.getMinutes())}`;
  const out: string[] = [`# ${L.title} — ${project}`, "", `${L.exportedAt}：${stamp}`, ""];
  let n = 0;
  for (const m of messages) {
    if (m.role === "user") {
      n++;
      out.push(`## ${L.question} ${n}：${m.text.replace(/\s*\n\s*/g, " ")}`, "");
    } else if (m.role === "ai") {
      out.push(m.qa.answer.trim(), "");
      if (m.qa.tour) {
        out.push(`### ${L.tour}：${m.qa.tour.title}`, "");
        m.qa.tour.steps.forEach((s, i) => {
          const ev = s.evidence ? `（\`${s.evidence.file}:${s.evidence.line}\`）` : "";
          out.push(`${i + 1}. **${moduleName(s.module)}**：${s.caption}${ev}`);
        });
        out.push("");
      }
      if (m.qa.diagram) {
        const f = fence(m.qa.diagram);
        out.push(`### ${L.diagram}`, "", `${f}mermaid`, m.qa.diagram.trim(), f, "");
      }
      if (m.qa.truncated) out.push(`> ${L.truncated}`, "");
    }
  }
  return out.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd() + "\n";
}

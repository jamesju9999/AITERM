import { stripFence } from "../TaskBoard/refinePrompts";

/**
 * 完成定義：需求先行建卡前，請 AI 把需求寫成幾條「做到什麼算完成」。
 * 附在卡片內文，規格撰寫與驗收都要照它逐條涵蓋——單一 session 靠使用者盯著判斷做完了沒，
 * Agent 協作沒有人盯，所以要先把「完成」說清楚。
 */

export const MIN_CRITERIA = 1;
export const MAX_CRITERIA = 7;
const CRITERION_MAX = 160;

export function buildDoneSystem(): string {
  return [
    "你要把使用者的需求，寫成「做到什麼算完成」的驗收條件清單。",
    `- ${MAX_CRITERIA} 條以內（簡單的需求 3 條左右就夠），每條一句話。`,
    "- 每一條都必須能實際驗證：看得到的畫面行為、能執行的指令與預期輸出、能呼叫的介面與回應。",
    "- 不要寫「程式碼乾淨」「效能良好」這類無法驗證的句子；需求沒提到的功能不要自己加。",
    "- 用需求使用的語言書寫。",
    "",
    '只輸出一個 JSON 物件，不要其他文字：{"criteria":["…","…"]}',
  ].join("\n");
}

export function buildDoneMessage(requirement: string, brief: string): string {
  const parts = [`本次需求：\n${requirement.trim()}`];
  if (brief.trim() && brief.trim() !== requirement.trim()) parts.unshift(`專案簡介：\n${brief.trim()}`);
  return parts.join("\n\n");
}

/** 解析 AI 回覆；不是預期格式、或一條可用的都沒有就回空陣列。 */
export function parseDoneCriteria(reply: string): string[] {
  const text = stripFence(reply.trim()).trim();
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) return [];
  try {
    const v: unknown = JSON.parse(text.slice(start, end + 1));
    const raw = v && typeof v === "object" ? (v as { criteria?: unknown }).criteria : null;
    if (!Array.isArray(raw)) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const c of raw) {
      if (typeof c !== "string") continue;
      const s = c.replace(/\s+/g, " ").trim().slice(0, CRITERION_MAX);
      if (!s || seen.has(s)) continue;
      seen.add(s);
      out.push(s);
      if (out.length >= MAX_CRITERIA) break;
    }
    return out.length >= MIN_CRITERIA ? out : [];
  } catch {
    return [];
  }
}

/** 把完成定義附在卡片內文後面。沒有條目就原樣回傳。 */
export function withDoneCriteria(body: string, criteria: string[]): string {
  if (criteria.length === 0) return body;
  const list = criteria.map((c) => `- ${c}`).join("\n");
  return `${body.trimEnd()}\n\n## 完成定義\n${list}\n（規格的驗收條件要涵蓋以上每一條；驗收時逐條實際驗證並附上證據。）`;
}

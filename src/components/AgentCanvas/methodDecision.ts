import type { Reference } from "../../ipc/agents";

/**
 * 空桌布的「方法討論」：先跟 AI 多輪討論這批工作的做法，確認後才建團隊。
 * 對話走純文字補全；AI 準備好提案時在回覆裡附一個 ```decision JSON 區塊，這裡負責解析。
 */

export type Approach = "single" | "batch" | "generator";

export interface MethodDecision {
  approach: Approach;
  /** 為什麼選這個做法（依據討論裡的哪一點）。 */
  reason: string;
  /** 試跑計畫與驗證方式；可為空。 */
  trial: string;
  /** 整理後的需求，確認後取代使用者原本輸入的第一句。 */
  requirement: string;
}

export interface DiscussionTurn {
  role: "user" | "assistant";
  content: string;
}

const APPROACHES: readonly Approach[] = ["single", "batch", "generator"];

const APPROACH_LABEL: Record<Approach, string> = {
  single: "單張卡（每項工作各一張模組卡）",
  batch: "批次卡（同一份做法 × 一份清單）",
  generator: "先做產生器（一張卡寫並驗證產生器，再由它大量輸出）",
};

export function buildDiscussionSystem(): string {
  return [
    "你在協助使用者決定 AITerm「Agent 協作」這批工作的做法，只討論、不執行，也不要假裝已經建了任何東西。",
    "",
    "可選的做法：",
    ...APPROACHES.map((a) => `- ${a}：${APPROACH_LABEL[a]}`),
    "",
    "批次只在四項全符合時才建議：有明確清單且通常 20 項以上；每項用同一份 template；驗證方式相同且可機械判斷；項目之間沒有依賴。",
    "能被產生器涵蓋的部分，優先建議先做產生器；批次只處理產生器處理不了的例外項目。",
    "不確定就先問使用者（清單有多大、每項是否同型、怎麼驗證），一次問一到兩個問題，不要一次丟一長串。",
    "",
    "當你有把握提案時，在回覆最後附上一個 decision 區塊（回覆其餘部分用使用者的語言簡短說明）：",
    '```decision\n{"approach":"single|batch|generator","reason":"為什麼","trial":"試跑計畫與驗證方式，沒有就留空字串","requirement":"整理後的完整需求"}\n```',
    "使用者還有疑問或想調整時，就繼續討論，之後再附新的 decision 區塊；以最後一個為準。",
  ].join("\n");
}

/** `deps.complete` 只收 user 訊息，所以把整段對話攤平成一則，並附上專案背景。 */
export function buildDiscussionMessage(turns: DiscussionTurn[], brief: string, refs: Reference[]): string {
  const parts: string[] = [];
  if (brief.trim()) parts.push(`專案簡介：\n${brief.trim()}`);
  parts.push(
    refs.length
      ? `參考資料夾（唯讀，例如舊系統原始碼）：\n${refs.map((r) => `- ${r.path}${r.note ? `：${r.note}` : ""}`).join("\n")}`
      : "參考資料夾：沒有",
  );
  parts.push("到目前為止的對話：\n" + turns.map((x) => `${x.role === "user" ? "使用者" : "你"}：${x.content}`).join("\n\n"));
  parts.push("請接著回覆使用者。");
  return parts.join("\n\n");
}

const BLOCK = /```decision\s*\n([\s\S]*?)```/g;

/** 取回覆裡最後一個 decision 區塊；格式不對一律回 null（當作還在討論）。 */
export function parseDecision(reply: string): MethodDecision | null {
  const blocks = [...reply.matchAll(BLOCK)];
  const last = blocks[blocks.length - 1];
  if (!last) return null;
  let raw: unknown;
  try {
    raw = JSON.parse(last[1].trim());
  } catch {
    return null;
  }
  if (!raw || typeof raw !== "object") return null;
  const o = raw as Record<string, unknown>;
  const approach = o.approach;
  const requirement = typeof o.requirement === "string" ? o.requirement.trim() : "";
  if (typeof approach !== "string" || !APPROACHES.includes(approach as Approach) || !requirement) return null;
  return {
    approach: approach as Approach,
    reason: typeof o.reason === "string" ? o.reason.trim() : "",
    trial: typeof o.trial === "string" ? o.trial.trim() : "",
    requirement,
  };
}

/** 寫進需求卡與專案簡介的文字；讓後面的角色看得到決定與理由。 */
export function formatDecision(d: MethodDecision): string {
  const lines = [`做法決定：${APPROACH_LABEL[d.approach]}`];
  if (d.reason) lines.push(`理由：${d.reason}`);
  if (d.trial) lines.push(`試跑計畫：${d.trial}`);
  return lines.join("\n");
}

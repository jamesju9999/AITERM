/** 一個分頁最多幾個里程碑。 */
export const MAX_MILESTONES = 10;
/** 每個里程碑文字的長度上限（字元）。 */
export const MAX_MILESTONE_CHARS = 120;

export interface Milestone {
  id: string;
  text: string;
  done: boolean;
}

/** 一個分頁的里程碑，以及它是依哪個目標拆出來的（目標改了就能提醒「要不要重新拆解」）。 */
export interface MilestoneState {
  forGoal: string;
  items: Milestone[];
}

const squash = (t: string) => t.replace(/\s+/g, "").toLowerCase();
const newId = () => crypto.randomUUID();

export function newMilestone(text: string): Milestone {
  return { id: newId(), text: text.trim().slice(0, MAX_MILESTONE_CHARS), done: false };
}

/** 抓出第一個 `[` 到最後一個 `]`（或 `{`…`}`）之間的 JSON；模型常包 code fence 或加說明。 */
function extractJson(raw: string | null | undefined, open: "[" | "{"): unknown {
  if (!raw) return undefined;
  const close = open === "[" ? "]" : "}";
  const a = raw.indexOf(open);
  const b = raw.lastIndexOf(close);
  if (a < 0 || b <= a) return undefined;
  try { return JSON.parse(raw.slice(a, b + 1)); } catch { return undefined; }
}

/** 解析 AI 拆解出來的里程碑文字。任何一步失敗回 []。 */
export function parseMilestonePlan(raw: string | null | undefined): string[] {
  const data = extractJson(raw, "[");
  if (!Array.isArray(data)) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of data) {
    if (typeof item !== "string") continue;
    const text = item.trim().slice(0, MAX_MILESTONE_CHARS);
    const key = squash(text);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push(text);
    if (out.length >= MAX_MILESTONES) break;
  }
  return out;
}

export function buildMilestonePlanRequest(goal: string, languageDirective: string): string {
  return [
    "以下是使用者為自己的終端機工作設定的「大目標」。請把它拆成 5 到 8 個里程碑。",
    "規則：依實際執行的先後順序排列；每個里程碑是一個可以明確判斷「完成了沒」的具體成果，不是模糊的活動；",
    `每個一句話、${MAX_MILESTONE_CHARS} 字以內；不要加編號或前言；不要新增使用者目標裡沒有的範圍。`,
    "只回傳 JSON 字串陣列，例如 [\"里程碑一\",\"里程碑二\"]，不要其他文字。",
    languageDirective,
    "",
    `大目標：${goal.trim()}`,
  ].join("\n");
}

/**
 * 請 AI 依畫面判斷哪些里程碑已經完成。里程碑用 1 起算的編號呈現（不用 uuid，
 * 長 id 模型容易抄錯），回傳時再對回 id。
 */
export function buildMilestoneCheckRequest(
  goal: string,
  milestones: readonly Milestone[],
  history: string,
  current: string,
  languageDirective: string,
): string {
  return [
    "以下是使用者的大目標、里程碑清單（含目前的完成狀態），以及終端機畫面。",
    "請只依畫面上看得到的證據，判斷哪些「尚未完成」的里程碑現在其實已經完成。沒有明確證據的不要列入，寧可漏報也不要誤報。",
    "只回傳 JSON 物件：{\"done\":[已完成的里程碑編號…],\"note\":\"一句話說明依據\"}，不要其他文字。",
    languageDirective,
    "",
    `大目標：${goal.trim()}`,
    "里程碑：",
    ...milestones.map((x, i) => `${i + 1}. [${x.done ? "已完成" : "未完成"}] ${x.text}`),
    "",
    ...(history
      ? ["較早的畫面（舊→新）：", "```", history, "```", ""]
      : []),
    "目前的終端機畫面：",
    "```",
    current,
    "```",
  ].join("\n");
}

/** 解析「檢查進度」的回覆。只回尚未完成、編號合法、不重複的項目；note 沒有就是 ""。 */
export function parseMilestoneCheck(
  raw: string | null | undefined,
  milestones: readonly Milestone[],
): { doneIds: string[]; note: string } {
  const data = extractJson(raw, "{");
  if (!data || typeof data !== "object") return { doneIds: [], note: "" };
  const { done, note } = data as { done?: unknown; note?: unknown };
  const doneIds: string[] = [];
  if (Array.isArray(done)) {
    for (const n of done) {
      const idx = typeof n === "number" ? n : typeof n === "string" && /^\d+$/.test(n.trim()) ? Number(n) : NaN;
      if (!Number.isInteger(idx) || idx < 1 || idx > milestones.length) continue;
      const ms = milestones[idx - 1];
      if (ms.done || doneIds.includes(ms.id)) continue;
      doneIds.push(ms.id);
    }
  }
  return { doneIds, note: typeof note === "string" ? note.trim() : "" };
}

/**
 * 重新拆解時：**已完成的項目原樣保留**（同一個 id），其餘由新的計畫取代；
 * 和已完成項目同名的計畫項不重複加入；總數不超過上限，已完成的優先。
 */
export function mergePlanKeepingDone(existing: readonly Milestone[], planned: readonly string[]): Milestone[] {
  const kept = existing.filter((x) => x.done).slice(0, MAX_MILESTONES);
  const keys = new Set(kept.map((x) => squash(x.text)));
  const out = [...kept];
  for (const text of planned) {
    if (out.length >= MAX_MILESTONES) break;
    const key = squash(text);
    if (!key || keys.has(key)) continue;
    keys.add(key);
    out.push(newMilestone(text));
  }
  return out;
}

/** 讀回持久化的資料時的防線：形狀不對的丟掉，沒有可用項目就當作沒有。 */
export function sanitizeMilestoneState(raw: unknown): MilestoneState | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const { forGoal, items } = raw as { forGoal?: unknown; items?: unknown };
  if (!Array.isArray(items)) return undefined;
  const ids = new Set<string>();
  const clean: Milestone[] = [];
  for (const it of items) {
    if (!it || typeof it !== "object") continue;
    const { id, text, done } = it as { id?: unknown; text?: unknown; done?: unknown };
    if (typeof id !== "string" || !id || ids.has(id)) continue;
    if (typeof text !== "string" || !text.trim()) continue;
    ids.add(id);
    clean.push({ id, text: text.trim().slice(0, MAX_MILESTONE_CHARS), done: done === true });
    if (clean.length >= MAX_MILESTONES) break;
  }
  if (clean.length === 0) return undefined;
  return { forGoal: typeof forGoal === "string" ? forGoal : "", items: clean };
}

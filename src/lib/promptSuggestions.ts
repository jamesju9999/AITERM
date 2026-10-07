export interface PromptSuggestion {
  title: string;
  prompt: string;
}

export const MAX_SUGGESTIONS = 5;
export const MAX_PROMPT_CHARS = 500;
/** 使用者設定的大目標上限（字元）。夠寫一段話，又不會把請求撐大。 */
export const MAX_GOAL_CHARS = 500;
/** 送給 AI 的畫面內容上限（字元）。只留尾端——最新的對話才決定下一步。 */
export const MAX_SCREEN_CHARS = 8000;

/**
 * 解析 AI 回的建議。模型常把 JSON 包在 code fence 或前後加說明，所以只抓
 * 第一個 `[` 到最後一個 `]` 之間的內容；任何一步失敗都回 []，由呼叫端顯示錯誤。
 */
export function parseSuggestions(raw: string | null | undefined): PromptSuggestion[] {
  if (!raw) return [];
  const start = raw.indexOf("[");
  const end = raw.lastIndexOf("]");
  if (start < 0 || end <= start) return [];
  let data: unknown;
  try {
    data = JSON.parse(raw.slice(start, end + 1));
  } catch {
    return [];
  }
  if (!Array.isArray(data)) return [];

  const seen = new Set<string>();
  const out: PromptSuggestion[] = [];
  for (const item of data) {
    if (!item || typeof item !== "object") continue;
    const { title, prompt } = item as Record<string, unknown>;
    if (typeof title !== "string" || typeof prompt !== "string") continue;
    const t = title.trim();
    const p = prompt.trim().slice(0, MAX_PROMPT_CHARS);
    if (!t || !p || seen.has(p)) continue;
    seen.add(p);
    out.push({ title: t, prompt: p });
    if (out.length >= MAX_SUGGESTIONS) break;
  }
  return out;
}

/**
 * 組出給 AI 的使用者訊息。`languageDirective` 來自 i18n 的 languageDirective()。
 * 有 `goal` 時，建議必須朝這個大目標推進；沒有（空白也算）時，與沒傳完全相同。
 */
export function buildSuggestionRequest(
  screen: string,
  languageDirective: string,
  goal?: string,
  /** 已經 formatHistoryForPrompt 整理過的「較早畫面」；空字串或沒給＝不加這一節。 */
  history?: string,
  /** 已經 formatMilestonesForPrompt 整理過的里程碑一節；空字串或沒給＝不加。 */
  milestones?: string,
): string {
  const tail = screen.length > MAX_SCREEN_CHARS ? screen.slice(-MAX_SCREEN_CHARS) : screen;
  const g = goal?.trim().slice(0, MAX_GOAL_CHARS) ?? "";
  return [
    "以下是使用者終端機目前的畫面內容（可能是 AI 命令列工具的對話）。",
    ...(g
      ? [
          `使用者的大目標：${g}`,
          "請判斷畫面上已經完成到哪裡，每個提示詞都要讓使用者朝這個大目標再往前一步；已經做完的事不要再建議。",
        ]
      : []),
    ...(milestones ? [milestones] : []),
    `請依內容判斷使用者接下來最可能想做的事，提出 3 到 ${MAX_SUGGESTIONS} 個「下一步提示詞」。`,
    "每個提示詞要能直接貼給 AI 命令列工具當下一個指令，具體、可執行，不要建議破壞性操作。",
    `只回傳 JSON 陣列，格式：[{"title":"12 字內的短標題","prompt":"完整提示詞"}]，不要其他文字。`,
    languageDirective,
    "",
    ...(history
      ? [
          "較早的畫面（舊→新，用來判斷已經做過什麼；已完成的事不要再建議）：",
          "```",
          history,
          "```",
          "",
        ]
      : []),
    "終端機畫面：",
    "```",
    tail,
    "```",
  ].join("\n");
}

/** 請 AI 把使用者草擬的大目標改寫得更清楚、具體。只改寫，不新增使用者沒說的需求。 */
export function buildGoalPolishRequest(goal: string, languageDirective: string): string {
  const draft = goal.trim().slice(0, MAX_GOAL_CHARS);
  return [
    "以下是使用者為自己的終端機工作草擬的「大目標」。請把它改寫得更清楚、具體、好執行。",
    "規則：保留原意；不要新增使用者沒提到的需求、技術或步驟；不要加標題、編號、前言或解釋；",
    `精簡到 ${MAX_GOAL_CHARS} 字以內，用一到三句話寫完；只回傳改寫後的目標文字本身。`,
    languageDirective,
    "",
    "草稿：",
    draft,
  ].join("\n");
}

/**
 * 清理 AI 回的純文字結果：拿掉 code fence 與整段外面包的引號，截到上限。空白回 ""。
 * 多行內容原樣保留。
 */
export function cleanGeneratedText(raw: string | null | undefined, max: number): string {
  if (!raw) return "";
  let t = raw.trim();
  const fence = /^```[a-zA-Z]*\n([\s\S]*?)\n?```$/.exec(t);
  if (fence) t = fence[1].trim();
  const pairs: [string, string][] = [['"', '"'], ["“", "”"], ["「", "」"], ["'", "'"]];
  for (const [open, close] of pairs) {
    if (t.length >= 2 && t.startsWith(open) && t.endsWith(close) && !t.slice(1, -1).includes(open)) {
      t = t.slice(1, -1).trim();
      break;
    }
  }
  return t.slice(0, max);
}

/** 清理 AI 回的目標潤飾結果。 */
export function cleanPolishedGoal(raw: string | null | undefined): string {
  return cleanGeneratedText(raw, MAX_GOAL_CHARS);
}

/** 提示詞助手：使用者的粗略需求上限（字元）。 */
export const MAX_ASSIST_REQUEST_CHARS = 1000;
/** 提示詞助手：改寫後提示詞的上限（字元）。 */
export const MAX_ASSIST_PROMPT_CHARS = 2000;
/** 提示詞助手：參考的終端機畫面只取尾端這麼多字元。 */
export const MAX_ASSIST_SCREEN_CHARS = 4000;

export interface PromptAssistInput {
  request: string;
  languageDirective: string;
  goal?: string;
  /** 已整理好的里程碑一節（formatMilestonesForPrompt）。 */
  milestones?: string;
  /** 已遮罩（若開啟）的終端機畫面；這裡只取尾端。 */
  screen?: string;
}

/**
 * 請 AI 把使用者的粗略需求改寫成一則可以直接貼給 AI 命令列工具的提示詞。
 * 只改寫，不新增需求；資訊不足的地方用〈…〉標出待補，不要編造。
 */
export function buildPromptAssistRequest({ request, languageDirective, goal, milestones, screen }: PromptAssistInput): string {
  const need = request.trim().slice(0, MAX_ASSIST_REQUEST_CHARS);
  const g = goal?.trim().slice(0, MAX_GOAL_CHARS) ?? "";
  const tail = screen && screen.length > MAX_ASSIST_SCREEN_CHARS ? screen.slice(-MAX_ASSIST_SCREEN_CHARS) : (screen ?? "");
  return [
    "你是提示詞助手。使用者正在對一個 AI 命令列工具（例如 Claude Code）下指令，這是他的粗略需求。",
    "請把它改寫成一則清楚、具體、可以直接貼給該工具的提示詞：說明要做什麼、範圍與限制、期望的產出。",
    "規則：保留原意；不要新增使用者沒要求的需求、技術或步驟；不要編造路徑、檔名或名稱——資訊不足的地方用〈…〉標出請使用者補；",
    "不要加標題、前言或解釋；只回傳提示詞本身。",
    languageDirective,
    "",
    ...(g ? [`使用者的大目標：${g}`] : []),
    ...(milestones ? [milestones] : []),
    ...(tail.trim() ? ["", "目前的終端機畫面（供判斷情境，不要照抄）：", "```", tail, "```"] : []),
    "",
    "使用者的粗略需求：",
    need,
  ].join("\n");
}

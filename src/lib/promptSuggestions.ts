export interface PromptSuggestion {
  title: string;
  prompt: string;
  /** AI 挑出的最推薦一張；只有 AI 明確標了才有，不會自己補。 */
  recommended?: true;
  /** 推薦理由，只會出現在推薦的那張上。 */
  reason?: string;
}

/**
 * 建議側欄所有 AI 請求共用的系統提示詞。刻意不是「終端機助手」：那種提示詞會叫模型把指令包進
 * `<cmd>` 標籤，讓「改寫提示詞」「只回 JSON」這類請求的結果被污染。
 */
export const PLAIN_COMPLETION_SYSTEM_PROMPT =
  "You are a precise assistant inside a developer tool. Follow the user's instructions exactly and reply with only the content they ask for, as plain text (or JSON when they ask for JSON). Do not add commentary, and do not wrap the answer in tags.";

export const MAX_SUGGESTIONS = 5;
export const MAX_PROMPT_CHARS = 500;
/** 推薦理由上限（字元）。請求裡要 20 字內，這裡留餘裕只防失控。 */
export const MAX_REASON_CHARS = 40;
/** 使用者設定的大目標上限（字元）。夠寫一段話，又不會把請求撐大。 */
export const MAX_GOAL_CHARS = 500;
/** 送給 AI 的畫面內容上限（字元）。只留尾端——最新的對話才決定下一步。 */
export const MAX_SCREEN_CHARS = 8000;
/** 有 AI 工具對話記錄時，畫面只留這麼多——完整內容已經在對話記錄裡。 */
export const MAX_SCREEN_WITH_TRANSCRIPT_CHARS = 3000;

/**
 * 捲動緩衝區裡會有 TUI 重畫留下的中間畫面（spinner、重複的提示列）。同一行出現多次只留
 * 最後一次；連續空行壓成一行；去掉行尾空白。
 */
export function squeezeRepeatedLines(text: string): string {
  const lines = text.split(/\r?\n/).map((l) => l.trimEnd());
  const lastIndex = new Map<string, number>();
  lines.forEach((l, i) => { if (l) lastIndex.set(l, i); });
  const out: string[] = [];
  lines.forEach((l, i) => {
    if (l) { if (lastIndex.get(l) === i) out.push(l); }
    else if (out.length > 0 && out[out.length - 1] !== "") out.push("");
  });
  return out.join("\n");
}

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
  let pick: PromptSuggestion | null = null;
  for (const item of data) {
    if (!item || typeof item !== "object") continue;
    const { title, prompt, recommended, reason } = item as Record<string, unknown>;
    if (typeof title !== "string" || typeof prompt !== "string") continue;
    const t = title.trim();
    const p = prompt.trim().slice(0, MAX_PROMPT_CHARS);
    if (!t || !p || seen.has(p)) continue;
    seen.add(p);
    const s: PromptSuggestion = { title: t, prompt: p };
    // 只認第一個字面 true；其他張就算也標了或帶了理由都丟掉。
    if (recommended === true && !pick) {
      s.recommended = true;
      const r = typeof reason === "string" ? reason.trim().slice(0, MAX_REASON_CHARS) : "";
      if (r) s.reason = r;
      pick = s;
    }
    out.push(s);
    if (out.length >= MAX_SUGGESTIONS) break;
  }
  // 推薦的那張移到最上面，其餘維持 AI 給的順序。
  return pick ? [pick, ...out.filter((s) => s !== pick)] : out;
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
  /** AI 工具自己的對話記錄（最近兩輪，已遮罩）；空字串或沒給＝不加，畫面照舊。 */
  transcript?: string,
): string {
  const maxScreen = transcript ? MAX_SCREEN_WITH_TRANSCRIPT_CHARS : MAX_SCREEN_CHARS;
  const tail = screen.length > maxScreen ? screen.slice(-maxScreen) : screen;
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
    "從中挑出剛好一個你最推薦的（最能推進目前工作的那一步），在那一項加上 \"recommended\":true 和 20 字內的 \"reason\" 說明為什麼；其他項不要加這兩個欄位。",
    `只回傳 JSON 陣列，格式：[{"title":"12 字內的短標題","prompt":"完整提示詞"},{"title":"…","prompt":"…","recommended":true,"reason":"推薦理由"}]，不要其他文字。`,
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
    ...(transcript
      ? [
          "AI 工具的對話記錄（最近兩輪，完整內容；畫面可能只顯示其中一部分）：",
          "```",
          transcript,
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
  // 有些模型（尤其被當成終端機助手提示過的）會把整段包進 <cmd>…</cmd>；只在「整段都被包住」時拆掉。
  const cmd = /^<cmd>([\s\S]*)<\/cmd>$/.exec(t);
  if (cmd && !cmd[1].includes("<cmd>")) t = cmd[1].trim();
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

export interface PromptSuggestion {
  title: string;
  prompt: string;
}

export const MAX_SUGGESTIONS = 5;
export const MAX_PROMPT_CHARS = 500;
/** 送給 AI 的畫面內容上限（字元）。只留尾端——最新的對話才決定下一步。 */
const MAX_SCREEN_CHARS = 8000;

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

/** 組出給 AI 的使用者訊息。`languageDirective` 來自 i18n 的 languageDirective()。 */
export function buildSuggestionRequest(screen: string, languageDirective: string): string {
  const tail = screen.length > MAX_SCREEN_CHARS ? screen.slice(-MAX_SCREEN_CHARS) : screen;
  return [
    "以下是使用者終端機目前的畫面內容（可能是 Claude Code 的對話）。",
    `請依內容判斷使用者接下來最可能想做的事，提出 3 到 ${MAX_SUGGESTIONS} 個「下一步提示詞」。`,
    "每個提示詞要能直接貼給 Claude Code 當下一個指令，具體、可執行，不要建議破壞性操作。",
    `只回傳 JSON 陣列，格式：[{"title":"12 字內的短標題","prompt":"完整提示詞"}]，不要其他文字。`,
    languageDirective,
    "",
    "終端機畫面：",
    "```",
    tail,
    "```",
  ].join("\n");
}

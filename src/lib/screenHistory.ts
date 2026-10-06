import { collapseConsecutiveDuplicateLines, stripAnsiCodes } from "../components/TaskBoard/transcriptUtils";

/** 最多保留幾張「穩定畫面」。 */
export const MAX_SCREENS = 10;
/** 保留的畫面總字元上限。超過就從最舊的丟。 */
export const MAX_HISTORY_CHARS = 24_000;
/** 放進一次建議請求的「較早畫面」字元上限（目前畫面另算）。 */
export const PROMPT_HISTORY_BUDGET = 9_000;
/** 新畫面的非空行有多少比例出現在另一張裡，就視為同一畫面的重繪／小更新。 */
export const SAME_SCREEN_OVERLAP = 0.7;

const contentLines = (screen: string) => screen.split("\n").map((l) => l.trim()).filter(Boolean);

/**
 * 把序列化出來的終端機畫面整理成可比較、可送給 AI 的純文字：去 ANSI、行尾空白、
 * 連續重複行（TUI 一直重畫的旋轉動畫與進度行），並去掉前後空行。
 */
export function normalizeScreen(raw: string): string {
  const text = collapseConsecutiveDuplicateLines(
    stripAnsiCodes(raw).split("\n").map((l) => l.trimEnd()).join("\n"),
  );
  return text.replace(/^\n+|\s+$/g, "");
}

function overlap(a: string[], b: string[]): number {
  if (a.length === 0) return 0;
  const set = new Set(b);
  return a.filter((l) => set.has(l)).length / a.length;
}

/**
 * 兩張畫面是不是「同一個畫面的不同時刻」。**對稱**：思考到一半的半成品畫面，
 * 它的行幾乎全在後來的完成畫面裡，也要算同一張，否則歷史裡會塞滿同一輪的半成品。
 */
export function isSameScreen(a: string, b: string): boolean {
  const la = contentLines(a);
  const lb = contentLines(b);
  if (la.length === 0 || lb.length === 0) return false;
  return Math.max(overlap(la, lb), overlap(lb, la)) >= SAME_SCREEN_OVERLAP;
}

/** 回傳新陣列，不改動傳入的陣列。 */
export function pushScreen(history: readonly string[], raw: string): string[] {
  const screen = normalizeScreen(raw);
  if (!screen) return [...history];
  const next = [...history];
  const last = next[next.length - 1];
  if (last !== undefined && isSameScreen(screen, last)) {
    // 同一畫面：留較新的。但新畫面若比舊的短一半以上（例如被清掉只剩一行），
    // 它不該把一張有內容的畫面蓋掉。
    if (contentLines(screen).length * 2 >= contentLines(last).length) next[next.length - 1] = screen;
  } else {
    next.push(screen);
  }
  while (next.length > MAX_SCREENS) next.shift();
  let total = next.reduce((n, s) => n + s.length, 0);
  while (next.length > 1 && total > MAX_HISTORY_CHARS) total -= (next.shift() as string).length;
  return next;
}

const SEPARATOR = "\n--------\n";

/**
 * 組成放進請求的「較早畫面」文字：舊→新。最新那張若其實就是目前畫面就略過（不重複送），
 * 超過 `budget` 時從最舊的開始丟；只剩一張又超過時取它的尾端。沒有可用的歷史回 ""。
 */
export function formatHistoryForPrompt(history: readonly string[], current: string, budget: number): string {
  const older = history.length > 0 && isSameScreen(history[history.length - 1], current)
    ? history.slice(0, -1)
    : [...history];
  const picked: string[] = [];
  let used = 0;
  for (let i = older.length - 1; i >= 0; i--) {
    const cost = older[i].length + (picked.length > 0 ? SEPARATOR.length : 0);
    if (used + cost <= budget) {
      picked.unshift(older[i]);
      used += cost;
    } else {
      if (picked.length === 0) picked.unshift(older[i].slice(-budget));
      break;
    }
  }
  return picked.join(SEPARATOR);
}

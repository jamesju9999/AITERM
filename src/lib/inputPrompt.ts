/**
 * 看一列文字是不是「指令正在等使用者輸入」的提問。
 *
 * 刻意保守：認不出來就回 null（不猜）。呼叫端只在「最後一張卡片 running、最後一列有內容」
 * 時才呼叫；輸出繼續增加、最後一列變了，結果自然就變成 null。
 */
export type InputPromptKind = "yesno" | "anykey" | "password";

export interface InputPrompt {
  kind: InputPromptKind;
  /** 快速回答按鈕要送的文字（不含換行；空字串＝只送 Enter）。password 沒有選項。 */
  options: string[];
}

// 提問必須落在行尾（允許尾端空白、問號、冒號、句點）。
const TAIL = String.raw`[\s?？:：.…]*$`;

const PASSWORD = new RegExp(String.raw`(password|passphrase|passcode|密碼|口令)[^\n]{0,40}[:：]\s*$`, "i");
const ANY_KEY = new RegExp(String.raw`(press any key|按任意鍵|按下任意鍵|請按任意鍵)[^\n]{0,30}$`, "i");
// [Y/n] [y/N] (Y/N) (yes/no) [yes/no]
const SLASH_YN = new RegExp(String.raw`[\[(]\s*(y(?:es)?)\s*/\s*(n(?:o)?)\s*[\])]` + TAIL, "i");
// winget：[Y] 是  [N] 否:  /  [Y] Yes  [N] No:
const BRACKET_YN = new RegExp(String.raw`\[\s*(y)\s*\]\s*\S+\s+\[\s*(n)\s*\]\s*\S+` + TAIL, "i");

export function detectInputPrompt(line: string): InputPrompt | null {
  const text = line.trimEnd();
  if (text.trim() === "") return null;

  if (PASSWORD.test(text)) return { kind: "password", options: [] };
  if (ANY_KEY.test(text)) return { kind: "anykey", options: [""] };

  const bracket = text.match(BRACKET_YN);
  if (bracket) return { kind: "yesno", options: ["Y", "N"] };

  const slash = text.match(SLASH_YN);
  if (slash) {
    const full = slash[1].length > 1 || slash[2].length > 1;
    return { kind: "yesno", options: full ? ["yes", "no"] : ["Y", "N"] };
  }
  return null;
}

/** 一張卡片的最後一列（有內容的）文字；沒有就回空字串。 */
export function lastLineText(lines: { spans: { text: string }[] }[] | undefined): string {
  if (!lines) return "";
  for (let i = lines.length - 1; i >= 0; i--) {
    const t = lines[i].spans.map((s) => s.text).join("");
    if (t.trim() !== "") return t;
  }
  return "";
}

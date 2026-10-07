/**
 * 把文字裡疑似敏感的內容（金鑰、token、密碼、私鑰、連線字串裡的帳密）換成 MASK。
 *
 * 這是**規則比對，不是保證**：自訂格式的密鑰、沒有敏感字眼的賦值、被換行切碎的 token 都可能漏掉。
 * 規則刻意保守——寧可漏遮，也不要把正常文字遮成亂碼讓 AI 看不懂。
 *
 * 遮罩後保留名稱與型態（`password=[已遮罩]`），AI 仍看得出「這裡有一個密碼」。
 * 結果可重複套用：已遮罩的文字再遮一次不會改變、也不會多算。
 *
 * 呼叫端要**先遮罩、再截斷**：截斷若從 token 中間切開，會留下一段沒被遮的殘片。
 */
export const MASK = "[已遮罩]";

export interface RedactResult {
  text: string;
  /** 遮了幾處。 */
  count: number;
  /** 出現過哪些類型（去重）。 */
  kinds: string[];
}

const PRIVATE_KEY = "[A-Z ]*PRIVATE KEY(?: BLOCK)?";

const TOKEN_RULES: [kind: string, re: RegExp][] = [
  ["aws-access-key", /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g],
  ["github-token", /\bgh[pousr]_[A-Za-z0-9]{36,}\b/g],
  ["github-token", /\bgithub_pat_[A-Za-z0-9_]{22,}/g],
  ["slack-token", /\bxox[abprs]-[A-Za-z0-9-]{10,}/g],
  ["anthropic-key", /\bsk-ant-[A-Za-z0-9_-]{20,}/g],
  ["api-key", /\bsk-[A-Za-z0-9_-]{20,}/g],
  ["google-api-key", /\bAIza[0-9A-Za-z_-]{35}\b/g],
  ["npm-token", /\bnpm_[A-Za-z0-9]{36}\b/g],
  ["jwt", /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}/g],
];

// 名稱要「以敏感字結尾」：tokenizer、author 這類不算，GITHUB_TOKEN、mySecretKey、DB_PASSWORD 算。
const SENSITIVE_NAME =
  "[\\w.-]*?(?:password|passwd|pwd|secret[_-]?key|client[_-]?secret|secret|api[_-]?key|access[_-]?key|private[_-]?key|token)";
const ASSIGNMENT = new RegExp(
  `(["']?)(${SENSITIVE_NAME})(["']?)(\\s*[:=]\\s*)("[^"\\n]*"|'[^'\\n]*'|[^\\s,;&}"')\\]]+)`,
  "gi",
);

function isPlaceholder(value: string): boolean {
  const v = value.replace(/^["']|["']$/g, "");
  if (v.length < 6) return true;
  if (/^(?:true|false|null|none|undefined)$/i.test(v)) return true;
  if (/^[<$]/.test(v)) return true; // <your-token>、${TOKEN}、$TOKEN
  if (/^your/i.test(v)) return true;
  if (/^(.)\1+$/.test(v)) return true; // xxxxxxxx、********
  return false;
}

export function redactSecrets(input: string): RedactResult {
  let text = input;
  let count = 0;
  const kinds = new Set<string>();
  const hit = (kind: string) => { count += 1; kinds.add(kind); return MASK; };

  // 1. 私鑰：完整區塊；只有 BEGIN 沒有 END（畫面被截掉）遮到結尾；只有 END（BEGIN 被截掉）連同前面的 base64 行。
  text = text.replace(new RegExp(`-----BEGIN ${PRIVATE_KEY}-----[\\s\\S]*?-----END ${PRIVATE_KEY}-----`, "g"), () => hit("private-key"));
  text = text.replace(new RegExp(`-----BEGIN ${PRIVATE_KEY}-----[\\s\\S]*$`), () => hit("private-key"));
  text = text.replace(
    new RegExp(`(^|\\n)(?:[A-Za-z0-9+/=]{16,}[ \\t]*\\n)+-----END ${PRIVATE_KEY}-----`, "g"),
    (_m, lead: string) => lead + hit("private-key"),
  );

  // 2. 網址內嵌帳密：只遮密碼。
  text = text.replace(/([a-z][a-z0-9+.-]*:\/\/[^\s:@/]+:)([^\s@/]+)(@)/gi, (m, a: string, pw: string, c: string) =>
    pw === MASK ? m : a + hit("url-credentials") + c);

  // 3. HTTP 認證。
  text = text.replace(/(\bauthorization\s*[:=]\s*["']?)(bearer|basic|token)(\s+)([^\s"']+)/gi,
    (m, a: string, scheme: string, sp: string, value: string) =>
      value.includes(MASK) || value.length < 6 ? m : a + scheme + sp + hit("http-auth"));
  text = text.replace(/\b(Bearer)(\s+)([A-Za-z0-9._~+/=-]{20,})/g, (_m, a: string, sp: string) => a + sp + hit("http-auth"));

  // 4. 已知格式的 token。
  for (const [kind, re] of TOKEN_RULES) text = text.replace(re, () => hit(kind));

  // 5. 名稱含敏感字的賦值。
  text = text.replace(ASSIGNMENT, (m, q1: string, name: string, q2: string, sep: string, value: string) => {
    if (isPlaceholder(value)) return m;
    return q1 + name + q2 + sep + hit(/pass|pwd/i.test(name) ? "password" : "secret");
  });

  return { text, count, kinds: [...kinds] };
}

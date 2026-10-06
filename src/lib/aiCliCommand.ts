/**
 * 這一行指令是不是在啟動某個 AI 命令列工具（claude、codex、gemini…）？
 *
 * 沿用 `isClaudeCommand` 的取捨：只認「第一個 token 的檔名剛好是清單內的名字」。
 * 刻意不支援環境變數前綴（`FOO=1 claude`）與 `npx xxx`——支援它們要把單純的
 * 字串比對變成 shell 語法解析，而漏報的代價只是這次不會自動開啟側欄，使用者
 * 還有手動開啟可用。
 */

/** 內建清單。新增工具改這裡；使用者自己的另外存（見 parseCustomNames）。 */
export const BUILTIN_AI_CLI_NAMES: readonly string[] = [
  "claude", "codex", "gemini", "aider", "opencode", "qwen", "amp", "copilot",
];

export const CUSTOM_AI_CLI_STORAGE_KEY = "aiterm-ai-cli-names";

const EXECUTABLE_EXT = /\.(exe|cmd|bat|ps1)$/;

/** 把使用者輸入或指令檔名收斂成比對用的名字；不合法回 null。 */
export function normalizeCliName(raw: string): string | null {
  const name = raw.trim().toLowerCase().replace(EXECUTABLE_EXT, "");
  if (!name) return null;
  // 含空白或路徑分隔符的不是「一個指令名」，收進清單只會永遠比對不到。
  if (/[\s/\\]/.test(name)) return null;
  return name;
}

export function isAiCliCommand(cmd: string, customNames: readonly string[] = []): boolean {
  const first = cmd.trim().split(/\s+/)[0];
  if (!first) return false;
  // Windows 的路徑用反斜線，POSIX 用斜線——兩種都要切。
  const base = first.split(/[/\\]/).pop();
  if (!base) return false;
  const name = base.toLowerCase().replace(EXECUTABLE_EXT, "");
  return BUILTIN_AI_CLI_NAMES.includes(name) || customNames.includes(name);
}

/** 讀 localStorage 的 JSON 字串。任何壞資料都回 []，不丟錯。 */
export function parseCustomNames(raw: string | null | undefined): string[] {
  if (!raw) return [];
  let data: unknown;
  try { data = JSON.parse(raw); } catch { return []; }
  if (!Array.isArray(data)) return [];
  const out: string[] = [];
  for (const item of data) {
    if (typeof item !== "string") continue;
    const name = normalizeCliName(item);
    if (!name || BUILTIN_AI_CLI_NAMES.includes(name) || out.includes(name)) continue;
    out.push(name);
  }
  return out;
}

export function loadCustomNames(): string[] {
  try { return parseCustomNames(localStorage.getItem(CUSTOM_AI_CLI_STORAGE_KEY)); } catch { return []; }
}

export function saveCustomNames(names: readonly string[]): void {
  try { localStorage.setItem(CUSTOM_AI_CLI_STORAGE_KEY, JSON.stringify(names)); } catch { /* ignore */ }
}

import { invokeAiChat } from "../ipc/ai";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import type { Locale } from "./i18n";

const MAX_CONTEXT_COMMANDS = 10;

function buildSummaryPrompt(commands: string[], cwd: string | undefined, locale: Locale): string {
  const list = commands.join("\n");
  const cwdLine = cwd ? (locale === "zh-TW" ? `工作目錄：${cwd}\n` : `Working directory: ${cwd}\n`) : "";

  return locale === "zh-TW"
    ? `以下是使用者在一個終端機工作階段執行的指令。請用不超過 20 個字生成一句精簡的中文摘要，描述這個終端機在做什麼（作為分頁標題）。不要標點符號結尾、不要加引號、只輸出摘要本身。\n\n${cwdLine}${list}`
    : `Below are the shell commands a user ran in one terminal session. Write a concise summary (40 characters or fewer) in English describing what this terminal is for (used as a tab title). No trailing punctuation, no quotes, output only the summary itself.\n\n${cwdLine}${list}`;
}

/**
 * One-shot AI call that summarizes a terminal tab's recent executed shell
 * commands into a short title-bar-friendly identifier. Returns null on any
 * failure (no commands, network error, provider not configured, empty reply)
 * — callers should treat null as "leave the tab title as it was", never
 * surface an error.
 */
export async function summarizeCommands(
  blocks: TerminalBlock[],
  sessionId: string,
  locale: Locale,
): Promise<string | null> {
  const commands = blocks
    .map((b) => b.command.trim())
    .filter((c) => c.length > 0)
    .slice(-MAX_CONTEXT_COMMANDS);
  if (commands.length === 0) return null;

  const cwd = blocks[blocks.length - 1]?.cwd;

  try {
    const prompt = buildSummaryPrompt(commands, cwd, locale);
    const reply = await invokeAiChat(
      [{ role: "user", content: prompt }],
      `${sessionId}-summary`,
      undefined,
      false,
      locale,
    );
    const summary = reply.content?.trim();
    return summary || null;
  } catch {
    return null;
  }
}

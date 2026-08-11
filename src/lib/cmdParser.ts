export type CmdPart =
  | { type: "text"; content: string }
  | { type: "cmd"; content: string; multiline: boolean };

/**
 * Parse assistant text into alternating text spans and <cmd> tag captures.
 * Uses a non-greedy regex so nested tags yield the innermost match first.
 * Unclosed tags are treated as plain text.
 */
const CMD_OPEN = "<cmd>";

/**
 * Cut streaming text at the first `<cmd>` so the tag never reaches the UI.
 *
 * Mid-stream the tag is half-arrived, so `parseCmdTags` either shows it as raw
 * text (no closing tag yet) or turns it into a clickable ▶ — which the Agent is
 * about to run itself, so a click would execute it twice. The command belongs
 * to the final message, not the live preview.
 *
 * Also drops a trailing partial opening tag (`<`, `<c`, `<cm`, …), since the
 * tag is assembled one delta at a time.
 */
export function truncateAtCmdTag(text: string): string {
  const idx = text.indexOf(CMD_OPEN);
  if (idx >= 0) return text.slice(0, idx);
  for (let len = CMD_OPEN.length - 1; len > 0; len--) {
    if (text.endsWith(CMD_OPEN.slice(0, len))) return text.slice(0, text.length - len);
  }
  return text;
}

export function parseCmdTags(text: string): CmdPart[] {
  if (text === "") return [];
  const parts: CmdPart[] = [];
  const regex = /<cmd>([\s\S]*?)<\/cmd>/g;
  let lastIdx = 0;
  let match: RegExpExecArray | null;
  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIdx) {
      parts.push({ type: "text", content: text.slice(lastIdx, match.index) });
    }
    const content = match[1].trim();
    parts.push({ type: "cmd", content, multiline: content.includes("\n") });
    lastIdx = match.index + match[0].length;
  }
  if (lastIdx < text.length) {
    parts.push({ type: "text", content: text.slice(lastIdx) });
  }
  return parts;
}

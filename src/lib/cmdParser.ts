export type CmdPart =
  | { type: "text"; content: string }
  | { type: "cmd"; content: string; multiline: boolean };

/**
 * Parse assistant text into alternating text spans and <cmd> tag captures.
 * Uses a non-greedy regex so nested tags yield the innermost match first.
 * Unclosed tags are treated as plain text.
 */
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

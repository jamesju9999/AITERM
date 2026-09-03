/** Collapses runs of consecutive identical lines down to one copy. Claude
 * Code's TUI repaints the same spinner/status line many times per second,
 * and the raw scrollback capture (no terminal-screen-state reconstruction)
 * records every one of those repaints verbatim — this is the single
 * biggest, most generic source of noise in it. Deliberately NOT tied to any
 * specific spinner glyph or app-specific pattern; a real clean transcript
 * would need a full terminal emulator to reconstruct final screen state,
 * out of scope here. */
export function collapseConsecutiveDuplicateLines(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  for (const line of lines) {
    if (out.length === 0 || out[out.length - 1] !== line) out.push(line);
  }
  return out.join("\n");
}

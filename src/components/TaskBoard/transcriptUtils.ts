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

/** Strips ANSI escape sequences (color/style SGR codes, cursor-movement CSI
 * sequences, etc.) from text that has already had its terminal-screen state
 * correctly reconstructed — i.e. output from xterm.js's SerializeAddon, not
 * raw unprocessed PTY bytes. That distinction matters: `serialize()` still
 * emits real ANSI codes to preserve colors/styling, and this only strips
 * those for a plain-text display, it does NOT interpret cursor movement or
 * redraws (xterm.js already did that). A simple regex is sufficient here —
 * unlike the backend's `strip_ansi` (src-tauri/src/pty/ansi.rs), which has
 * to defend against genuinely arbitrary/malformed raw PTY bytes, this only
 * ever receives xterm.js's own well-formed serialized output. */
export function stripAnsiCodes(text: string): string {
  // eslint-disable-next-line no-control-regex -- matching real ESC bytes is the point
  return text.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, "");
}

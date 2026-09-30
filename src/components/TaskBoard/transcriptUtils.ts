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
 * sequences, DEC private-mode sequences like the alternate-screen-buffer
 * toggle, etc.) from text that has already had its terminal-screen state
 * correctly reconstructed — i.e. output from xterm.js's SerializeAddon, not
 * raw unprocessed PTY bytes. That distinction matters: `serialize()` still
 * emits real ANSI codes to preserve colors/styling/modes, and this only
 * strips those for a plain-text display, it does NOT interpret cursor
 * movement or redraws (xterm.js already did that). A simple regex is
 * sufficient here — unlike the backend's `strip_ansi`
 * (src-tauri/src/pty/ansi.rs), which has to defend against genuinely
 * arbitrary/malformed raw PTY bytes, this only ever receives xterm.js's own
 * well-formed serialized output. The `\??` makes the DEC private-mode
 * marker (e.g. `\x1b[?1049h` to enter the alt screen buffer, `\x1b[?2004h`
 * for bracketed paste) optional, since those are otherwise identical CSI
 * sequences and TUI apps like Claude Code use them constantly. */
export function stripAnsiCodes(text: string): string {
  // eslint-disable-next-line no-control-regex -- matching real ESC bytes is the point
  return text.replace(/\x1b\[\??[0-9;]*[a-zA-Z]/g, "");
}

export interface TranscriptTurn {
  prompt: string;
  output: string;
}

export interface ParsedTranscript {
  /** Everything before the first user prompt. */
  preamble: string;
  turns: TranscriptTurn[];
}

const PROMPT_LINE = /^\s*❯ (\S.*)$/;
// Session-log transcripts (src-tauri/src/tasks/session_log.rs) mark each user
// message with a column-0 `使用者：`; continuation lines are indented, so an
// embedded copy of the marker can't collide with a real one.
const SESSION_PROMPT_LINE = /^使用者：(.*)$/;
const RULE_LINE = /^\s*─{10,}\s*$/;

const trimBlankEdges = (lines: string[]): string => lines.join("\n").replace(/^\n+|\s+$/g, "");

/** Splits a transcript into one turn per user prompt. Prefers the session-log
 * format (`使用者：text`); when that marker is absent, falls back to the
 * serialized-terminal format (`❯ text`). A `❯` line sandwiched between two `─` rules is the live input
 * box (bare, or holding text the user hasn't sent yet), not a prompt. */
export function parseTranscriptTurns(text: string): ParsedTranscript {
  const lines = text.split("\n");
  const sessionFormat = lines.some((l) => SESSION_PROMPT_LINE.test(l));
  const preamble: string[] = [];
  const turns: TranscriptTurn[] = [];
  let current: { prompt: string; out: string[] } | null = null;
  const flush = () => {
    if (current) turns.push({ prompt: current.prompt, output: trimBlankEdges(current.out) });
  };
  lines.forEach((line, i) => {
    const m = (sessionFormat ? SESSION_PROMPT_LINE : PROMPT_LINE).exec(line);
    const inInputBox =
      !sessionFormat && RULE_LINE.test(lines[i - 1] ?? "") && RULE_LINE.test(lines[i + 1] ?? "");
    if (m && !inInputBox) {
      flush();
      current = { prompt: m[1].trim() || "…", out: [] };
    } else {
      (current ? current.out : preamble).push(line);
    }
  });
  flush();
  return { preamble: trimBlankEdges(preamble), turns };
}

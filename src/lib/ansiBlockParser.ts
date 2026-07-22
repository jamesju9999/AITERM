import { Terminal } from "@xterm/xterm";

export interface RenderedSpan {
  text: string;
  fg?: string;
  bg?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
}

export interface RenderedLine {
  spans: RenderedSpan[];
}

const ANSI_PALETTE = [
  "#000000", "#cd3131", "#0dbc79", "#e5e510",
  "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
  "#666666", "#f14c4c", "#23d18b", "#f5f543",
  "#3b8eea", "#d670d6", "#29b8db", "#e5e5e5",
];

function paletteColor(value: number): string {
  if (value < 16) return ANSI_PALETTE[value];
  // TODO: 256-color extended palette (indices 16-255) falls back to a
  // grayscale-ish approximation rather than the real xterm-256 cube/ramp
  // table. This range is hit more often than the color count suggests
  // (many CLI tools default to 256-color output); a real table would be
  // a follow-up if inaccurate colors here become noticeable.
  const gray = Math.min(255, value * 3).toString(16).padStart(2, "0");
  return `#${gray}${gray}${gray}`;
}

function cellColor(isRGB: boolean, isPalette: boolean, isDefault: boolean, value: number): string | undefined {
  if (isDefault) return undefined;
  if (isRGB) return `#${value.toString(16).padStart(6, "0")}`;
  if (isPalette) return paletteColor(value);
  return undefined;
}

function spansEqual(a: RenderedSpan, b: Omit<RenderedSpan, "text">): boolean {
  return a.fg === b.fg && a.bg === b.bg && a.bold === b.bold && a.italic === b.italic && a.underline === b.underline;
}

interface CellInfo {
  chars: string;
  style: Omit<RenderedSpan, "text">;
  hasStyle: boolean;
}

/**
 * Reads a cell's char + style info, or null if the cell is a zero-width
 * continuation cell (the second half of a wide/CJK character) that carries
 * no content of its own and must be skipped, not rendered as a space.
 */
function readCell(cell: import("@xterm/xterm").IBufferCell): CellInfo | null {
  if (cell.getWidth() === 0) return null;
  const chars = cell.getChars() || " ";
  const style: Omit<RenderedSpan, "text"> = {
    fg: cellColor(cell.isFgRGB(), cell.isFgPalette(), cell.isFgDefault(), cell.getFgColor()),
    bg: cellColor(cell.isBgRGB(), cell.isBgPalette(), cell.isBgDefault(), cell.getBgColor()),
    bold: !!cell.isBold(),
    italic: !!cell.isItalic(),
    underline: !!cell.isUnderline(),
  };
  const hasStyle = style.fg !== undefined || style.bg !== undefined || style.bold || style.italic || style.underline;
  return { chars, style, hasStyle };
}

/**
 * Parses raw ANSI byte output (as captured from a PTY) into structured,
 * styled lines by replaying it through a headless (unmounted) xterm.js
 * instance — reusing xterm's own battle-tested ANSI/VT parser instead of
 * hand-rolling one.
 */
export async function parseAnsiToRenderedLines(raw: string, cols: number, rows = 30): Promise<RenderedLine[]> {
  const term = new Terminal({ cols, rows, scrollback: 10000, convertEol: false, allowProposedApi: true });

  await new Promise<void>((resolve) => term.write(raw, resolve));

  const buffer = term.buffer.active;
  const lines: RenderedLine[] = [];

  for (let y = 0; y < buffer.length; y++) {
    const line = buffer.getLine(y);
    if (!line) continue;

    // Determine the last column that carries real content (a non-space
    // char, or any styling) so trailing blank cells past it are never
    // turned into spans at all. Trimming *after* merging doesn't work:
    // trailing blank cells sharing the same (default) style as the
    // preceding text merge into one span, e.g. "hello" + 75 spaces, so a
    // whole-span trailing-blank check never fires.
    let lastContentX = -1;
    for (let x = cols - 1; x >= 0; x--) {
      const cell = line.getCell(x);
      if (!cell) continue;
      const info = readCell(cell);
      if (!info) continue; // zero-width continuation cell of a wide char
      if (info.chars.trim() !== "" || info.hasStyle) {
        lastContentX = x;
        break;
      }
    }

    const spans: RenderedSpan[] = [];
    let current: RenderedSpan | null = null;

    for (let x = 0; x <= lastContentX; x++) {
      const cell = line.getCell(x);
      if (!cell) continue;
      const info = readCell(cell);
      if (!info) continue; // zero-width continuation cell of a wide char

      if (current && spansEqual(current, info.style)) {
        current.text += info.chars;
      } else {
        current = { text: info.chars, ...info.style };
        spans.push(current);
      }
    }

    lines.push({ spans });
  }

  // Drop trailing fully-empty lines (unwritten rows past the last content).
  while (lines.length && lines[lines.length - 1].spans.length === 0) {
    lines.pop();
  }

  term.dispose();
  return lines;
}

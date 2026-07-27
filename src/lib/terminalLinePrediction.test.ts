import { describe, expect, it } from "vitest";
import { readLineExcludingInlinePrediction, type PredictionAwareLine } from "./terminalLinePrediction";

function makeCell(chars: string, opts: { dim?: boolean; italic?: boolean } = {}) {
  return {
    getChars: () => chars,
    isDim: () => (opts.dim ? 1 : 0),
    isItalic: () => (opts.italic ? 1 : 0),
  };
}

// `text` is the full rendered line; `styled` marks the columns (by index)
// that carry the shell's inline-prediction styling (dim/italic).
function makeLine(text: string, styledFrom: number | null, style: { dim?: boolean; italic?: boolean } = { dim: true }): PredictionAwareLine {
  return {
    getCell: (x: number) => {
      if (x >= text.length) return makeCell("");
      const isStyled = styledFrom !== null && x >= styledFrom;
      return makeCell(text[x], isStyled ? style : {});
    },
    translateToString: (_trimRight?: boolean, startColumn = 0, endColumn = text.length) =>
      text.slice(startColumn, endColumn),
  };
}

describe("readLineExcludingInlinePrediction", () => {
  it("strips a dim inline suggestion that starts exactly at the cursor (PSReadLine 'dir' bug)", () => {
    // User typed "dir", cursor sits right after it at column 3; PSReadLine
    // renders a dim predicted "r" continuation right after the cursor.
    const line = makeLine("dirr", 3, { dim: true });
    expect(readLineExcludingInlinePrediction(line, 3)).toBe("dir");
  });

  it("strips an italic inline suggestion the same way", () => {
    const line = makeLine("dirr", 3, { italic: true });
    expect(readLineExcludingInlinePrediction(line, 3)).toBe("dir");
  });

  it("keeps normally-styled trailing text when the cursor sits mid-line (edit-then-Enter without moving to end)", () => {
    // "git status" typed as "gt status", cursor moved left twice to insert
    // "i", giving "gi|t status" (| = cursor at column 2). No prediction is
    // shown mid-line, so the tail after the cursor is real content.
    const line = makeLine("git status", null);
    expect(readLineExcludingInlinePrediction(line, 2)).toBe("git status");
  });

  it("returns the full trimmed line when the cursor is at the true end (no prediction shown)", () => {
    const line = makeLine("dir", null);
    expect(readLineExcludingInlinePrediction(line, 3)).toBe("dir");
  });

  it("returns empty string for an undefined line", () => {
    expect(readLineExcludingInlinePrediction(undefined, 0)).toBe("");
  });
});

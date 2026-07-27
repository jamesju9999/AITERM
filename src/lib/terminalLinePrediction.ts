// Shells like PowerShell (PSReadLine InlineView) and zsh (autosuggestions)
// render an inline "ghost" completion suggestion after the cursor, styled
// dim and/or italic, without actually moving the cursor past it. Reading the
// rendered line naively (as TerminalView does to recover the exact command
// the user typed) picks up that ghost text as if it were real input. Since
// the cursor sits at the end of the *real* typed text, any styled run
// starting exactly at the cursor column is prediction, not input — but text
// after the cursor with normal styling is real (e.g. the user pressed Left
// to fix a typo mid-line, then Enter without moving back to the end), so it
// must be kept.
export interface PredictionAwareCell {
  getChars(): string;
  isDim(): number;
  isItalic(): number;
}

export interface PredictionAwareLine {
  getCell(x: number): PredictionAwareCell | undefined;
  translateToString(trimRight?: boolean, startColumn?: number, endColumn?: number): string;
}

export function readLineExcludingInlinePrediction(
  line: PredictionAwareLine | undefined,
  cursorX: number,
): string {
  if (!line) return "";
  const cellAtCursor = line.getCell(cursorX);
  const isInlinePrediction =
    !!cellAtCursor &&
    cellAtCursor.getChars() !== "" &&
    (!!cellAtCursor.isDim() || !!cellAtCursor.isItalic());
  return isInlinePrediction ? line.translateToString(true, 0, cursorX) : line.translateToString(true);
}

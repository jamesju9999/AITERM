/**
 * If `s` consists of the same non-empty chunk repeated 2 or 3 times back to
 * back with nothing else, return just one copy of that chunk. Otherwise
 * return `s` unchanged. Bounded to a small, explicit set of repeat counts
 * (checked via plain string slicing, not a backtracking regex) so this
 * stays fast and predictable on any input, including ones that don't match.
 *
 * Used to clean up PtyManager's backfilled terminal output: xterm's own
 * initial fit() call (right after a tab connects) resizes from the default
 * 80x24 to the container's real size, and that resize is forwarded to the
 * backend PTY — which, like any real terminal resize, makes the shell
 * redraw its prompt in place (SIGWINCH). That redraw lands in the ring
 * buffer right alongside the shell's own original connect-time draw. Since
 * a redraw overwrites the same line (carriage return, not newline) and the
 * backfill source strips ANSI/control bytes, the two draws arrive back to
 * back with no separator at all — not as two newline-delimited lines — so
 * this detects "the whole string is the same chunk repeated" directly
 * rather than assuming any particular separator.
 */
export function collapseWholeStringRepeat(s: string): string {
  for (const reps of [2, 3]) {
    if (s.length === 0 || s.length % reps !== 0) continue;
    const chunkLen = s.length / reps;
    const chunk = s.slice(0, chunkLen);
    if (chunk.repeat(reps) === s) return chunk;
  }
  return s;
}

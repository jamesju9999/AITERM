import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { saveTranscript } from "../../ipc/tasks";
import { stripAnsiCodes } from "./transcriptUtils";

/** Best-effort: if `tabId`'s xterm terminal is still live (registered in
 * terminalInstanceRegistry), serialize its current screen — every redraw/
 * cursor movement already correctly collapsed into final on-screen text by
 * xterm.js — strip the styling ANSI codes, and overwrite the task's saved
 * transcript.txt with that clean version. If the tab isn't live (closed, or
 * never was one), or the save fails for any reason, this silently does
 * nothing: the original raw transcript captured at completion time is still
 * there, so there's nothing broken to recover from — see
 * docs/superpowers/specs/2026-09-03-clean-task-transcript-design.md. */
export async function tryUpgradeTranscript(taskId: string, tabId: string | null): Promise<void> {
  if (!tabId) return;
  const raw = serializeTerminal(tabId);
  if (raw === null) return;
  const clean = stripAnsiCodes(raw);
  try {
    await saveTranscript(taskId, clean);
  } catch {
    // Best effort — see doc comment above.
  }
}

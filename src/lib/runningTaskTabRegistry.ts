/** Which tab ids currently have a `running` Task Board task attached to
 * them — kept in sync by TaskBoardView's own task-list polling (see its
 * `useEffect` calling `setRunningTaskTabs`). `TerminalView`'s close guard
 * consults this (via `isRunningTaskTab`) on top of its own generic "shell
 * command still running" check, so closing a tab mid-task warns the user
 * even in cases where the shell's own OSC133 command-tracking wouldn't
 * catch it (e.g. an interactive task the user hasn't typed anything into
 * yet, or one waiting on a slow `claude` cold start).
 *
 * Known limitation, real not theoretical: TaskBoardView unmounts when its
 * overlay isn't shown (switching to Home or a terminal tab), and its own
 * `onTasksUpdated` listener stops with it — so this registry can go stale
 * while the board isn't open. That cuts both ways:
 *   - a task finishes while the board is closed → its tab stays reported
 *     as "running" until the board is reopened and refreshes. Benign: an
 *     unnecessary confirmation on an already-finished task.
 *   - a NEW task gets auto-dispatched while the board is closed → its tab
 *     was never added here at all. This is the case that actually matters:
 *     `TerminalApp.tsx`'s `onCoordinationTabSpawned` auto-focuses a freshly
 *     spawned tab specifically when the board is NOT the active view (see
 *     its own `viewingOverlay` comment), so "board closed" is exactly the
 *     situation where a brand-new task's tab becomes the visible, focused
 *     one right away — and every guard signal (isBusyRef, missionActiveRef,
 *     this registry) is unpopulated in that same instant. Closing that tab
 *     immediately (e.g. a reflexive Ctrl+W on what looks like a blank
 *     terminal) currently proceeds with NO warning — a genuine false
 *     negative, not just a stale positive. Not fixed here: doing so
 *     without breaking the "replace the whole set" semantics above needs
 *     `TerminalApp`'s always-mounted spawn handler to register proactively
 *     while TaskBoardView's effect only ever removes what it can positively
 *     confirm has finished — tracked as follow-up work, not implemented in
 *     this module yet. */
const runningTaskTabs = new Set<string>();

/** Replaces the whole tracked set — call with every currently-running
 * task's tab id on each Task Board refresh. */
export function setRunningTaskTabs(tabIds: Iterable<string>): void {
  runningTaskTabs.clear();
  for (const id of tabIds) runningTaskTabs.add(id);
}

export function isRunningTaskTab(tabId: string): boolean {
  return runningTaskTabs.has(tabId);
}

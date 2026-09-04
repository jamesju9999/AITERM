/** Which tab ids currently have a `running` Task Board task attached to
 * them — kept in sync by TaskBoardView's own task-list polling (see its
 * `useEffect` calling `setRunningTaskTabs`). `TerminalView`'s close guard
 * consults this (via `isRunningTaskTab`) on top of its own generic "shell
 * command still running" check, so closing a tab mid-task warns the user
 * even in cases where the shell's own OSC133 command-tracking wouldn't
 * catch it (e.g. an interactive task the user hasn't typed anything into
 * yet, or one waiting on a slow `claude` cold start).
 *
 * Known limitation: TaskBoardView unmounts when its overlay isn't shown
 * (switching to Home or a terminal tab), and its own `onTasksUpdated`
 * listener stops with it — so this registry can go stale (still reporting
 * a tab as "running" after its task actually finished) while the board
 * isn't open. That's an accepted tradeoff: the failure mode is an
 * unnecessary confirmation on an already-finished task, not silently
 * losing a real warning for one still running. */
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

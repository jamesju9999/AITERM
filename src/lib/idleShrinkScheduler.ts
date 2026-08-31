// A late-arriving custom shell prompt (oh-my-posh etc.) can still be
// printing after a command's block has already flipped to "completed", so
// shrinking the live pane immediately clips it. But never shrinking (the
// previous fix for that) leaves the pane tall enough that a later window
// resize can make ConPTY's real, never-cleared screen buffer visibly
// resurface old output that the app only ever hid client-side. Waiting for
// a settle period of real PTY silence — re-armed whenever new output
// arrives — gets both: the late prompt gets its chance, and by the time the
// user actually resizes, the pane is already back down, clipping anything
// ConPTY later replays.
export const IDLE_SHRINK_SETTLE_MS = 500;

export class IdleShrinkScheduler {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private readonly getLastOutputAt: () => number;
  private readonly onSettle: () => void;

  constructor(getLastOutputAt: () => number, onSettle: () => void) {
    this.getLastOutputAt = getLastOutputAt;
    this.onSettle = onSettle;
  }

  arm(): void {
    this.cancel();
    this.check();
  }

  cancel(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  private check = (): void => {
    const remaining = IDLE_SHRINK_SETTLE_MS - (Date.now() - this.getLastOutputAt());
    if (remaining <= 0) {
      this.timer = null;
      this.onSettle();
      return;
    }
    this.timer = setTimeout(this.check, remaining);
  };
}

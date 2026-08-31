// On Windows, ConPTY responds to a PTY resize by re-transmitting its
// currently-visible screen content (its own screen-buffer model, unlike a
// Unix pty) — always starting with this exact sequence (hide cursor, cursor
// to 1;1). Confirmed via real-machine diagnostic logging: a burst of
// resizePty() calls fired within a few ms of each other can make ConPTY emit
// several such full-screen repaints in a row, and an early one in that burst
// can carry a stale reflow — content from a completely different,
// already-scrolled-past directory listing — sandwiched between two correct
// ones. Only the LAST repaint in a tight burst reflects the true current
// screen.
export const RESIZE_REPAINT_PREFIX = "\x1b[?25l\x1b[H";

const ARM_MS = 300;
const SETTLE_MS = 24;

/**
 * Coalesces the ConPTY resize-repaint burst described above: while armed
 * (shortly after noteResize()), consecutive full-screen-repaint chunks are
 * held instead of written immediately, keeping only the most recent one.
 * Real (non-repaint) output flushes whatever is held first, preserving
 * order. Never armed for alternate-buffer content (vim/htop/etc. redraw
 * their whole screen constantly and legitimately — coalescing those would
 * drop real frames).
 */
export class ResizeRepaintGate {
  private armedUntil = -Infinity;
  private pendingText: string | null = null;
  private pendingWrite: ((text: string) => void) | null = null;
  private settleTimer: ReturnType<typeof setTimeout> | null = null;

  noteResize(): void {
    this.armedUntil = Date.now() + ARM_MS;
  }

  handleChunk(text: string, isAlternateBuffer: boolean, write: (text: string) => void): void {
    const armed = !isAlternateBuffer && Date.now() < this.armedUntil;
    if (!armed || !text.startsWith(RESIZE_REPAINT_PREFIX)) {
      this.flush();
      write(text);
      return;
    }
    this.pendingText = text;
    this.pendingWrite = write;
    if (this.settleTimer) clearTimeout(this.settleTimer);
    this.settleTimer = setTimeout(() => this.flush(), SETTLE_MS);
  }

  dispose(): void {
    if (this.settleTimer) clearTimeout(this.settleTimer);
    this.settleTimer = null;
    this.pendingText = null;
    this.pendingWrite = null;
  }

  private flush(): void {
    if (this.settleTimer) {
      clearTimeout(this.settleTimer);
      this.settleTimer = null;
    }
    if (this.pendingText !== null && this.pendingWrite) {
      const text = this.pendingText;
      const write = this.pendingWrite;
      this.pendingText = null;
      this.pendingWrite = null;
      write(text);
    }
  }
}

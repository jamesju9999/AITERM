import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

const writePtyMock = vi.fn().mockResolvedValue(undefined);
vi.mock("../ipc/pty", () => ({
  writePty: (...args: unknown[]) => writePtyMock(...args),
}));

import { useTerminalBlocks } from "./useTerminalBlocks";

let term: Terminal;

beforeEach(() => {
  writePtyMock.mockClear();
  term = new Terminal({ cols: 80, rows: 24 });
});

afterEach(() => {
  term.dispose();
});

describe("useTerminalBlocks — interrupt (force-finalize a stuck block)", () => {
  it("force-finalizing a running block (exit code -1) fires its onComplete with a non-running status", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));
    const onComplete = vi.fn();

    act(() => {
      result.current.submitCommand("cat <<EOF", onComplete);
    });
    // No OSC 133 D will ever arrive — this simulates a shell stuck at a
    // heredoc continuation prompt, waiting for input that never comes.
    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].status).toBe("running");

    const stuckBlockId = result.current.blocks[0].id;

    act(() => {
      result.current.finalizeBlock(stuckBlockId, -1);
    });

    // finalizeBlock's callback fire is behind a headless-parse promise plus
    // a setTimeout(..., 50) — waitFor handles that without fake timers.
    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });

    const finalBlock = onComplete.mock.calls[0][0];
    expect(finalBlock.status).not.toBe("running");
    expect(finalBlock.status).toBe("failed");
    expect(finalBlock.exitCode).toBe(-1);

    // The hook's own state must reflect the same outcome, not just the
    // argument passed to the callback.
    expect(result.current.blocks[0].status).toBe("failed");
  });
});

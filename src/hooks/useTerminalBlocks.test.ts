import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

const writePtyMock = vi.fn().mockResolvedValue(undefined);
vi.mock("../ipc/pty", () => ({
  writePty: (...args: unknown[]) => writePtyMock(...args),
}));

import { useTerminalBlocks } from "./useTerminalBlocks";

async function writeToTerm(term: Terminal, data: string) {
  await new Promise<void>((resolve) => term.write(data, resolve));
}

let term: Terminal;

beforeEach(() => {
  writePtyMock.mockClear();
  term = new Terminal({ cols: 80, rows: 24 });
});

afterEach(() => {
  term.dispose();
});

describe("useTerminalBlocks", () => {
  it("creates a running block on submitCommand and appends PTY output into rawOutput", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("echo hi");
    });
    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].status).toBe("running");

    await act(async () => {
      await writeToTerm(term, "\x1b]133;C\x07");
    });

    act(() => {
      result.current.appendOutput("hi\r\n");
    });
    expect(result.current.blocks[0].rawOutput).toBe("hi\r\n");
  });

  it("marks block completed with exit code 0 and freezes rawOutput on OSC 133 D", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("echo hi");
    });
    act(() => {
      result.current.appendOutput("hi\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].status).toBe("completed");
    });
    expect(result.current.blocks[0].exitCode).toBe(0);
    expect(result.current.blocks[0].rawOutput).toBe("hi\r\n");
    expect(result.current.blocks[0].endTime).toBeDefined();
  });

  it("marks block failed with non-zero exit code and produces renderedLines", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("false");
    });
    act(() => {
      result.current.appendOutput("boom\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;1\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].renderedLines).toBeDefined();
    });
    expect(result.current.blocks[0].status).toBe("failed");
    expect(result.current.blocks[0].exitCode).toBe(1);
    expect(result.current.blocks[0].renderedLines?.[0].spans.map((s) => s.text).join("")).toBe("boom");
  });

  it("does not cross-contaminate rawOutput between two sequential blocks", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });

    act(() => { result.current.submitCommand("cmd2"); });
    act(() => { result.current.appendOutput("second\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });

    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(2);
      expect(result.current.blocks[1].status).toBe("completed");
    });
    expect(result.current.blocks[0].rawOutput).toBe("first\r\n");
    expect(result.current.blocks[1].rawOutput).toBe("second\r\n");
  });

  it("fires onComplete exactly once with the final renderedLines-populated block on normal completion", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));
    const onComplete = vi.fn();

    act(() => {
      result.current.submitCommand("echo hi", onComplete);
    });
    act(() => {
      result.current.appendOutput("hi\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(onComplete).toHaveBeenCalledTimes(1);
    });

    const finalBlock = onComplete.mock.calls[0][0];
    expect(finalBlock.status).toBe("completed");
    expect(finalBlock.exitCode).toBe(0);
    expect(finalBlock.renderedLines).toBeDefined();
    expect(finalBlock.renderedLines[0].spans.map((s: { text: string }) => s.text).join("")).toBe("hi");

    // Give any stray async work a chance to run, then confirm no duplicate fire.
    await new Promise((resolve) => setTimeout(resolve, 60));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it("finalizes an orphaned running block as interrupted and fires its onComplete when submitCommand is called again before OSC 133 D", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));
    const onComplete1 = vi.fn();
    const onComplete2 = vi.fn();

    act(() => {
      result.current.submitCommand("cmd1", onComplete1);
    });
    act(() => {
      result.current.appendOutput("first\r\n");
    });

    // cmd2 is submitted before cmd1's OSC 133 D ever fires (caller race /
    // shell that doesn't reliably emit D). cmd1 must not be silently orphaned.
    act(() => {
      result.current.submitCommand("cmd2", onComplete2);
    });

    await waitFor(() => {
      expect(onComplete1).toHaveBeenCalledTimes(1);
    });
    const orphanedBlock = onComplete1.mock.calls[0][0];
    expect(orphanedBlock.command).toBe("cmd1");
    expect(orphanedBlock.status).toBe("failed");
    expect(orphanedBlock.exitCode).toBe(-1);
    expect(orphanedBlock.rawOutput).toBe("first\r\n");
    expect(orphanedBlock.renderedLines).toBeDefined();

    expect(onComplete2).not.toHaveBeenCalled();

    act(() => {
      result.current.appendOutput("second\r\n");
    });
    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(onComplete2).toHaveBeenCalledTimes(1);
    });
    const secondBlock = onComplete2.mock.calls[0][0];
    expect(secondBlock.command).toBe("cmd2");
    expect(secondBlock.status).toBe("completed");
    expect(secondBlock.rawOutput).toBe("second\r\n");

    // onComplete1 must never fire again — proves its completionCallbacksRef
    // entry was deleted (not left stale or double-fired) when cmd1 was
    // finalized.
    expect(onComplete1).toHaveBeenCalledTimes(1);
  });

  it("beginTrackedBlock creates a running block without writing to the PTY", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.beginTrackedBlock("ls -l");
    });

    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].command).toBe("ls -l");
    expect(result.current.blocks[0].status).toBe("running");
    expect(writePtyMock).not.toHaveBeenCalled();

    act(() => {
      result.current.appendOutput("total 0\r\n");
    });
    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].status).toBe("completed");
    });
    expect(result.current.blocks[0].rawOutput).toBe("total 0\r\n");
  });

  it("beginTrackedBlock does not create a second block while one is already running", () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => {
      result.current.submitCommand("cmd1");
    });
    act(() => {
      result.current.beginTrackedBlock("cmd2");
    });

    expect(result.current.blocks).toHaveLength(1);
    expect(result.current.blocks[0].command).toBe("cmd1");
  });

  it("submitCommand('clear') wipes existing blocks and does not create a block for itself", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });
    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(1);
    });

    act(() => {
      result.current.submitCommand("clear");
    });

    expect(result.current.blocks).toHaveLength(0);
    expect(writePtyMock).toHaveBeenLastCalledWith("session-1", expect.stringContaining("clear\r"));
  });

  it("beginTrackedBlock('clear') wipes existing blocks without writing to the PTY", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });
    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(1);
    });
    writePtyMock.mockClear();

    act(() => {
      result.current.beginTrackedBlock("clear");
    });

    expect(result.current.blocks).toHaveLength(0);
    expect(writePtyMock).not.toHaveBeenCalled();
  });

  it("submitCommand('cls') wipes existing blocks and does not create a block for itself", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });
    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(1);
    });

    act(() => {
      result.current.submitCommand("cls");
    });

    expect(result.current.blocks).toHaveLength(0);
    expect(writePtyMock).toHaveBeenLastCalledWith("session-1", expect.stringContaining("cls\r"));
  });

  it("calls onCommandSettled with the parsed exit code on OSC 133 D", async () => {
    const onCommandSettled = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, onCommandSettled),
    );

    act(() => {
      result.current.submitCommand("false");
    });
    act(() => {
      result.current.appendOutput("boom\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;1\x07");
    });

    await waitFor(() => {
      expect(onCommandSettled).toHaveBeenCalledWith(1);
    });
  });

  it("beginTrackedBlock('CLS') wipes existing blocks without writing to the PTY (case-insensitive)", async () => {
    const { result } = renderHook(() => useTerminalBlocks("session-1", term));

    act(() => { result.current.submitCommand("cmd1"); });
    act(() => { result.current.appendOutput("first\r\n"); });
    await act(async () => { await writeToTerm(term, "\x1b]133;D;0\x07"); });
    await waitFor(() => {
      expect(result.current.blocks).toHaveLength(1);
    });
    writePtyMock.mockClear();

    act(() => {
      result.current.beginTrackedBlock("CLS");
    });

    expect(result.current.blocks).toHaveLength(0);
    expect(writePtyMock).not.toHaveBeenCalled();
  });
});

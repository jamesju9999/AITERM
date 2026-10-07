import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal } from "@xterm/xterm";

vi.mock("../ipc/pty", () => ({ writePty: vi.fn().mockResolvedValue(undefined) }));

import { useTerminalBlocks, QUIET_PROMPT_MS } from "./useTerminalBlocks";

let term: Terminal;
beforeEach(() => {
  vi.useFakeTimers();
  term = new Terminal({ cols: 80, rows: 24 });
});
afterEach(() => {
  vi.useRealTimers();
  term.dispose();
});

const write = (data: string) => new Promise<void>((r) => term.write(data, r));

describe("quiet + prompt fallback completion (shell without OSC 133)", () => {
  it("settles a block with unknown exit when output goes quiet on a prompt line", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s", term));
    const onComplete = vi.fn();
    act(() => result.current.submitCommand("ls", onComplete));

    await act(async () => {
      result.current.appendOutput("a.txt\r\nsh-3.2$ ");
    });
    expect(onComplete).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(QUIET_PROMPT_MS + 200);
    });
    expect(onComplete).toHaveBeenCalledTimes(1);
    const b = onComplete.mock.calls[0][0];
    expect(b.exitUnknown).toBe(true);
    expect(b.exitCode).toBeUndefined();
    expect(b.status).toBe("completed");
  });

  it("does not settle while output keeps arriving", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s", term));
    const onComplete = vi.fn();
    act(() => result.current.submitCommand("build", onComplete));
    for (let i = 0; i < 4; i++) {
      await act(async () => {
        result.current.appendOutput("working\r\n");
        await vi.advanceTimersByTimeAsync(QUIET_PROMPT_MS - 300);
      });
    }
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("does not settle when the last line is not a prompt (quiet but still running)", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s", term));
    const onComplete = vi.fn();
    act(() => result.current.submitCommand("sleep 100", onComplete));
    await act(async () => {
      result.current.appendOutput("starting...");
      await vi.advanceTimersByTimeAsync(QUIET_PROMPT_MS * 3);
    });
    expect(onComplete).not.toHaveBeenCalled();
  });

  it("does nothing for blocks nobody is waiting on", async () => {
    const { result } = renderHook(() => useTerminalBlocks("s", term));
    act(() => result.current.submitCommand("ls"));
    await act(async () => {
      result.current.appendOutput("x\r\nsh-3.2$ ");
      await vi.advanceTimersByTimeAsync(QUIET_PROMPT_MS * 2);
    });
    expect(result.current.blocks[0].status).toBe("running");
  });

  it("OSC 133 D still wins and carries the real exit code", async () => {
    // xterm 的 write callback 靠真的 setTimeout，這條要用真計時器。
    vi.useRealTimers();
    const { result } = renderHook(() => useTerminalBlocks("s", term));
    const onComplete = vi.fn();
    act(() => result.current.submitCommand("false", onComplete));
    await act(async () => {
      result.current.appendOutput("sh-3.2$ ");
      await write("\x1b]133;D;1\x07");
      await new Promise((r) => setTimeout(r, 120));
    });
    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(onComplete.mock.calls[0][0].exitCode).toBe(1);
    expect(onComplete.mock.calls[0][0].exitUnknown).toBeUndefined();
  });
});

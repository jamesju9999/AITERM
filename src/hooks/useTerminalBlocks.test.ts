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

  it("calls onCommandSettled with 0 on a successful command", async () => {
    const onCommandSettled = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, onCommandSettled),
    );

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
      expect(onCommandSettled).toHaveBeenCalledWith(0);
    });
  });

  it("does not call onCommandSettled when D carries no exit code (cmd.exe's PROMPT)", async () => {
    const onCommandSettled = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, onCommandSettled),
    );

    act(() => {
      result.current.submitCommand("dir");
    });
    act(() => {
      result.current.appendOutput("boom\r\n");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].status).toBe("completed");
    });
    expect(onCommandSettled).not.toHaveBeenCalled();
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

  it("calls onCommandStarted with the command text on submitCommand", () => {
    const onCommandStarted = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, undefined, onCommandStarted),
    );

    act(() => {
      result.current.submitCommand("claude");
    });

    expect(onCommandStarted).toHaveBeenCalledWith("claude");
  });

  it("calls onCommandStarted with the command text on beginTrackedBlock", () => {
    const onCommandStarted = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, undefined, onCommandStarted),
    );

    act(() => {
      result.current.beginTrackedBlock("claude");
    });

    expect(onCommandStarted).toHaveBeenCalledWith("claude");
  });

  it("calls onCommandStarted for 'clear' even though it creates no block", () => {
    // Pins the deliberate decision to report "what did the user run" rather
    // than "which blocks were created" — clear/cls return early before any
    // block exists, but onCommandStarted must still fire.
    const onCommandStarted = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, undefined, onCommandStarted),
    );

    act(() => {
      result.current.submitCommand("clear");
    });

    expect(onCommandStarted).toHaveBeenCalledWith("clear");
    expect(result.current.blocks).toHaveLength(0);
  });

  it("uses hostPlatform instead of navigator.platform for the Windows ConPTY resync", async () => {
    // 模擬「觀看端自己在 Windows 上跑，但主控端是別的系統」——這正是
    // hostPlatform 存在的理由：navigator.platform 量到的是觀看端的平台，
    // 用它來判斷「要不要送 ConPTY 專屬的 Ctrl+L」在跨平台分享時會誤判。
    const originalPlatform = navigator.platform;
    Object.defineProperty(navigator, "platform", { value: "Win32", configurable: true });

    try {
      const { result } = renderHook(() =>
        useTerminalBlocks(
          "session-1",
          term,
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          "other",
        ),
      );

      act(() => {
        result.current.submitCommand("echo hi");
      });
      writePtyMock.mockClear();

      await act(async () => {
        await writeToTerm(term, "\x1b]133;D;0\x07");
      });

      await waitFor(() => {
        expect(result.current.blocks[0].status).toBe("completed");
      });

      // hostPlatform 是 "other"，即使 navigator.platform 說是 Windows，
      // 也不該送出 ConPTY 專屬的 Ctrl+L 同步位元組。
      expect(writePtyMock).not.toHaveBeenCalledWith("session-1", "\x0c");
    } finally {
      Object.defineProperty(navigator, "platform", { value: originalPlatform, configurable: true });
    }
  });

  it("routes writes through a custom write function instead of writePty", async () => {
    // hostPlatform 明確傳 "other"（而不是留給預設值去讀 navigator.platform）
    // ——這個測試只關心「write 有沒有接對」，不該讓斷言的期待值隨著執行測試
    // 的機器 navigator.platform 是什麼而變動（"other" 時 submitCommand 會在
    // 指令前面加 "\x15" 清行，"windows" 時不會，見 submitCommand 內部的
    // clearSeq 邏輯）。
    const writeMock = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        undefined,
        undefined,
        undefined,
        writeMock,
        "other",
      ),
    );

    act(() => {
      result.current.submitCommand("echo hi");
    });

    expect(writeMock).toHaveBeenCalledWith("\x15echo hi\r");
    // 忘記接新的 write、其實還是寫去本機 PTY 的話，這裡就會抓到。
    expect(writePtyMock).not.toHaveBeenCalled();
  });

  it("routes the Windows ConPTY resync byte through a custom write function too", async () => {
    // The two tests above check hostPlatform and write independently. This one
    // checks the combination that actually matters for the remote-viewer use
    // case: a Windows host's ConPTY resync byte (\x0c) must go through the
    // injected write, not fall back to writePty.
    const writeMock = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        undefined,
        undefined,
        undefined,
        writeMock,
        "windows",
      ),
    );

    act(() => {
      result.current.submitCommand("echo hi");
    });
    writeMock.mockClear();

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(result.current.blocks[0].status).toBe("completed");
    });

    expect(writeMock).toHaveBeenCalledWith("\x0c");
    expect(writePtyMock).not.toHaveBeenCalled();
  });

  it("signals onUntrackedCommandBoundary when OSC 133 C/D fire with no locally-tracked block", async () => {
    // 實機測試抓到的 bug：遠端觀看者拿到控制權時送進來的指令不會經過
    // submitCommand/beginTrackedBlock（本機分頁能收到的只有 shell 回顯的
    // 輸出，跟本機打字是兩條不同的路），但 shell 自己送出的 OSC 133 C/D
    // 完全不知道指令是哪裡來的、照樣會發生。這個測試不呼叫 submitCommand，
    // 直接把 OSC 序列寫進終端機，模擬「有指令在跑，但沒有本機區塊」。
    const boundaryMock = vi.fn();
    renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        boundaryMock,
      ),
    );

    await act(async () => {
      await writeToTerm(term, "\x1b]133;C\x07");
    });
    expect(boundaryMock).toHaveBeenCalledWith("start");

    boundaryMock.mockClear();
    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });
    expect(boundaryMock).toHaveBeenCalledWith("end");
  });

  it("does not signal onUntrackedCommandBoundary when a local block already covers the command", async () => {
    // 對照組：指令是透過 submitCommand 送出的（本機分頁的正常路徑），
    // 已經有一個 running 中的區塊在追蹤——這種情況不該多送一次
    // onUntrackedCommandBoundary，那個信號是給「完全沒有本機區塊」的
    // 情境用的，兩套機制不該疊加。
    const boundaryMock = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        undefined,
        boundaryMock,
      ),
    );

    act(() => {
      result.current.submitCommand("echo hi");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;C\x07");
    });
    expect(boundaryMock).not.toHaveBeenCalled();

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });
    expect(boundaryMock).not.toHaveBeenCalled();
  });
});

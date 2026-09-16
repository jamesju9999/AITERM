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

  it("uses hostPlatform instead of navigator.platform for the Windows-only immediate clear", async () => {
    // 模擬「觀看端自己在 Windows 上跑，但主控端是別的系統」——這正是
    // hostPlatform 存在的理由：navigator.platform 量到的是觀看端的平台，
    // 用它來判斷「要不要在 D 標記當下立刻清畫面」在跨平台分享時會誤判。
    const originalPlatform = navigator.platform;
    Object.defineProperty(navigator, "platform", { value: "Win32", configurable: true });
    const onLiveClearMock = vi.fn();

    try {
      const { result } = renderHook(() =>
        useTerminalBlocks(
          "session-1",
          term,
          undefined,
          onLiveClearMock,
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
      // 也不該走 Windows 專屬的立即清畫面分支。
      expect(onLiveClearMock).not.toHaveBeenCalled();
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

  it("Windows：OSC 133 D 之後不清空 xterm 緩衝區——清了會讓 PSReadLine 的絕對定位對不上", async () => {
    // 實機 [cursor-diag] log 證實的根因：term.clear() 把游標所在行搬成第 0
    // 行、其餘丟棄，於是 xterm 認為提示字元在第 1 列；但 ConPTY 從來不知道
    // 這件事發生過，仍然認為它在第 12 列。PSReadLine 每次按鍵都用絕對定位
    // （ESC[12;62H）重繪輸入行，於是重繪全部落在 xterm 裡那塊空白的第 12
    // 列，使用者眼前的提示字元行再也不更新——症狀是「輸入第一個字之後就
    // 卡住、無法編輯」。zsh/bash 用相對移動重繪，不受影響，所以這是
    // Windows 專屬的修正；舊輸出改由即時窗格的高度裁切隱藏，不動緩衝區。
    const writeMock = vi.fn();
    const onLiveClearMock = vi.fn();
    const clearSpy = vi.spyOn(term, "clear");
    const { result: winResult } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        onLiveClearMock,
        undefined,
        undefined,
        writeMock,
        "windows",
      ),
    );

    act(() => {
      winResult.current.submitCommand("echo hi");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(winResult.current.blocks[0].status).toBe("completed");
    });
    await waitFor(() => {
      expect(onLiveClearMock).toHaveBeenCalledTimes(1);
    });

    expect(clearSpy).not.toHaveBeenCalled();
    clearSpy.mockRestore();
  });

  it("Windows：卡片內容取自畫面上的 xterm，不重播 ConPTY 的絕對座標輸出", async () => {
    // ConPTY 用絕對座標畫在自己固定大小的畫面上：畫面填滿後，每一行都是先
    // \r\n 捲動、再 ESC[16;1H 畫到最後一列。畫面上的 xterm 跟 ConPTY 同樣
    // 16 列、游標也在同一列，所以顯示正確；拿同樣的位元組到另一個終端機
    // 從左上角重播，全部都會疊在同一列，只剩最後一行（實機截圖：dir 的
    // 清單中間整段消失）。
    const liveTerm = new Terminal({ cols: 80, rows: 16 });
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", liveTerm, undefined, undefined, undefined, undefined, vi.fn(), "windows"),
    );
    const feed = async (data: string) => {
      await act(async () => {
        await writeToTerm(liveTerm, data);
      });
      act(() => {
        result.current.appendOutput(data);
      });
    };

    for (let i = 1; i <= 15; i++) await writeToTerm(liveTerm, `old ${i}\r\n`);
    await writeToTerm(liveTerm, "PS C:\\> \x1b]133;B\x07");

    act(() => {
      result.current.submitCommand("dir");
    });
    await feed("dir\r\n\x1b]133;C\x07");

    const files = Array.from({ length: 25 }, (_, i) => `file-${String(i + 1).padStart(2, "0")}`);
    let conptyPaint = "\x1b[16;1H" + files[0] + "\x1b[K";
    for (const f of files.slice(1)) conptyPaint += "\r\n\x1b[16;1H" + f + "\x1b[K";
    await feed(conptyPaint);
    await feed("\r\n\x1b]133;D;0\x07");

    await waitFor(() => {
      expect(result.current.blocks[0].renderedLines).toBeDefined();
    });
    const text = result.current.blocks[0].renderedLines!.map((l) => l.spans.map((s) => s.text).join("").trimEnd());
    expect(text).toEqual(files);
    liveTerm.dispose();
  });

  it("Windows 沒有 C 標記（cmd.exe 的 PROMPT 只送 D/A）時，退回重播原始輸出", async () => {
    const { result } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, undefined, undefined, vi.fn(), "windows"),
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
      expect(result.current.blocks[0].renderedLines).toBeDefined();
    });
    expect(result.current.blocks[0].renderedLines!.map((l) => l.spans.map((s) => s.text).join(""))).toEqual(["hi"]);
  });

  it("非 Windows：維持原本清空緩衝區的行為（zsh/bash 用相對移動重繪，不受影響）", async () => {
    const clearSpy = vi.spyOn(term, "clear");
    const { result: macResult } = renderHook(() =>
      useTerminalBlocks("session-1", term, undefined, undefined, undefined, undefined, undefined, "other"),
    );

    act(() => {
      macResult.current.submitCommand("echo hi");
    });

    await act(async () => {
      await writeToTerm(term, "\x1b]133;D;0\x07");
    });

    await waitFor(() => {
      expect(macResult.current.blocks[0].status).toBe("completed");
    });
    await waitFor(() => {
      expect(clearSpy).toHaveBeenCalled();
    });
    clearSpy.mockRestore();
  });

  it("clears the live pane on Windows after OSC 133 D without sending any resync byte", async () => {
    // 前兩版修法（Ctrl+L 重試、真正的 PTY resize）都已被實機證據推翻，
    // 見 useTerminalBlocks.ts 該段落的註解——兩者都無法在提示字元根本
    // 還沒被印出來的當下變出真正的文字。現在的行為只清畫面，不嘗試用
    // 任何 PTY 層級的把戲搶救內容；真正的根因（TerminalView.tsx 的
    // liveRows 收縮太早）在那邊修，不在這裡。
    const writeMock = vi.fn();
    const onLiveClearMock = vi.fn();
    const { result } = renderHook(() =>
      useTerminalBlocks(
        "session-1",
        term,
        undefined,
        onLiveClearMock,
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

    await waitFor(() => {
      expect(onLiveClearMock).toHaveBeenCalledTimes(1);
    });
    expect(writeMock).not.toHaveBeenCalledWith("\x0c");
    expect(writePtyMock).not.toHaveBeenCalled();
  });

  describe("remote-viewer command text recovery (OSC 133 B/C)", () => {
    it("recovers the typed command text and calls beginTrackedBlock when no local block is tracked", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      // 模擬 shell 實際回顯的位元組序列：提示字元文字 → B 標記（提示字元
      // 結束、輸入開始）→ 使用者打的指令文字（遠端觀看者的按鍵，經由 shell
      // 回顯出現在畫面上）→ Enter 的換行回顯 → C 標記。
      await act(async () => {
        await writeToTerm(term, "user@host:~$ \x1b]133;B\x07ls -la\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe("ls -la");
      expect(result.current.blocks[0].status).toBe("running");
    });

    it("recovers a command that auto-wraps across multiple rows", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));
      // term 是 80 欄；prompt「user@host:~$ 」佔 13 欄，所以這個 95 字元的
      // 指令一定會自動換行到第二行。
      const longCommand = "echo " + "a".repeat(90);

      await act(async () => {
        await writeToTerm(term, `user@host:~$ \x1b]133;B\x07${longCommand}\r\n\x1b]133;C\x07`);
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe(longCommand);
    });

    it("recovers correctly when the prompt itself spans multiple rows before B fires", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "== host ==\r\nprompt> \x1b]133;B\x07pwd\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(1);
      expect(result.current.blocks[0].command).toBe("pwd");
    });

    it("does not create a block when the user pressed Enter with nothing typed", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "user@host:~$ \x1b]133;B\x07\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(0);
    });

    it("does not glue together rows separated by a real newline (e.g. a multi-line paste), only genuine auto-wraps", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      // 兩行短短的內容，中間是真正的換行字元（不是欄寬自動換行）——模擬
      // 遠端觀看者貼上多行內容後，才按下真正送出的 Enter。
      await act(async () => {
        await writeToTerm(term, "user@host:~$ \x1b]133;B\x07echo hi\r\necho bye\r\n\x1b]133;C\x07");
      });

      expect(result.current.blocks).toHaveLength(0);
    });
  });

  describe("isRawKeyboardModeActive（Kitty keyboard protocol push/pop）", () => {
    it("收到 CSI > u（push）後變 true", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));
      expect(result.current.isRawKeyboardModeActive).toBe(false);

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });

      expect(result.current.isRawKeyboardModeActive).toBe(true);
    });

    it("push 後收到 CSI < u（pop）變回 false", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("連續兩次 push 只 pop 一次，仍然是 true（深度計數器，不是布林開關）", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("同一次 write 裡先 pop 再 push（Ink 類程式每次重繪的實際模式），最終狀態仍是 true", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);
    });

    it("clearAllBlocks() 會把卡在 push 狀態的旗標強制歸零", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      act(() => {
        result.current.clearAllBlocks();
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("finalizeBlock 強制結案時（例如卡住偵測介入）會把卡在 push 狀態的旗標強制歸零", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("claude");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      const blockId = result.current.blocks[0].id;
      act(() => {
        result.current.finalizeBlock(blockId, -1);
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });
  });

  describe("執行中的區塊持續更新 renderedLines（跟已完成卡片同一套渲染）", () => {
    it("running 中收到輸出時，renderedLines 就已經是最新解析結果，不用等指令結束", async () => {
      // 見設計文件 docs/superpowers/specs/2026-09-16-live-block-rendering-design.md：
      // 不再是「執行中完全不解析、結束時才解析一次」，而是每個 chunk 都
      // 持續更新，讓執行中的內容能跟已完成卡片用同一套渲染顯示。
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("echo hi");
      });
      // OSC 133 C：登記 marker，這個指令輸出的第一行就是現在游標所在行。
      await act(async () => {
        await writeToTerm(term, "\x1b]133;C\x07");
      });
      // 真正把位元組寫進畫面（跟 appendOutput 一樣，模擬呼叫端兩者一起做）。
      await act(async () => {
        await writeToTerm(term, "hi\r\n");
      });
      act(() => {
        result.current.appendOutput("hi\r\n");
      });

      await waitFor(() => {
        expect(result.current.blocks[0].renderedLines).toBeDefined();
      });
      // 還在 running，還沒送 D。
      expect(result.current.blocks[0].status).toBe("running");
      expect(result.current.blocks[0].renderedLines?.[0].spans.map((s) => s.text).join("")).toBe("hi");
    });

    it("互動選單畫完整個框之後把游標移回選到的那一項，renderedLines 仍要包含游標之後已經畫出來的行", async () => {
      // 實機抓到的 bug：claude CLI 的信任提示這類互動選單，會先把整個框
      // （含最下面的提示文字）畫完，再用 CSI 游標定位把游標搬回目前選到
      // 的那一項（顯示反白），游標因此停在框的中段，不是最下面。原本
      // `endRow` 只看「游標目前所在列」，會把游標之後、已經畫出來但游標
      // 沒有停留的內容切掉——見 scheduleLiveRender 內 endRow 那段的說明
      // （改成無條件往下多讀一整個螢幕高度，交給 readRenderedLines 自己
      // 捨棄結尾空白列，不再嘗試用游標位置精準判斷內容畫到哪）。
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("claude");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b]133;C\x07");
      });
      // 畫出四行框內容，游標自然落在第 4 行（最後一行）。
      await act(async () => {
        await writeToTerm(term, "line1\r\nline2\r\nline3\r\nline4\r\n");
      });
      act(() => {
        result.current.appendOutput("line1\r\nline2\r\nline3\r\nline4\r\n");
      });
      await waitFor(() => {
        expect(result.current.blocks[0].renderedLines?.length).toBe(4);
      });

      // 選單接著把游標搬回「line2」那一行（CSI cursor up 3 行，從 line4 那一行往上數）並且原地
      // 重繪成不同的文字（模擬套用反白樣式表示選取）——這正是互動選單的
      // 典型畫法。刻意換成一段跟原本「line2」不同的文字（不是原地重繪
      // 一模一樣的內容、也不是空字串）：這個字串只可能來自「第二輪真的
      // 重新掃描過緩衝區」的結果，逼 waitFor 一定要等到第二輪 setTimeout
      // 真正觸發、讀到新內容為止——如果只斷言「四行都還在」，會被第一輪
      // 就已經成立的殘留值矇混過去，等不到第二輪真正執行就通過，驗不出
      // 這裡要測的東西（這個地雷已經在本檔踩過一次）。
      await act(async () => {
        await writeToTerm(term, "\x1b[3A\rLINE2-SELECTED");
      });
      act(() => {
        result.current.appendOutput("\rLINE2-SELECTED");
      });

      // 第二輪重新掃描後，line2 變成新文字，且 line3/line4 仍然要在——
      // 不能因為游標搬回中段就把它們切掉。
      await waitFor(() => {
        const lines = result.current.blocks[0].renderedLines;
        expect(lines?.map((l) => l.spans.map((s) => s.text).join(""))).toEqual(["line1", "LINE2-SELECTED", "line3", "line4"]);
      });
    });

    it("不限 Windows：任何平台 running 中都能拿到即時 renderedLines（marker 登記已經通用化）", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term, undefined, undefined, undefined, undefined, undefined, "other"));

      act(() => {
        result.current.submitCommand("echo hi");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b]133;C\x07");
      });
      await act(async () => {
        await writeToTerm(term, "hi\r\n");
      });
      act(() => {
        result.current.appendOutput("hi\r\n");
      });

      await waitFor(() => {
        expect(result.current.blocks[0].renderedLines).toBeDefined();
      });
    });

    it("同一個畫面更新週期內連續多次 appendOutput，只在最後一次真正重新掃描緩衝區", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("echo hi");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b]133;C\x07");
      });

      const timeoutSpy = vi.spyOn(globalThis, "setTimeout");
      await act(async () => {
        await writeToTerm(term, "a\r\n");
        result.current.appendOutput("a\r\n");
        await writeToTerm(term, "b\r\n");
        result.current.appendOutput("b\r\n");
        await writeToTerm(term, "c\r\n");
        result.current.appendOutput("c\r\n");
      });
      // 三次 appendOutput 只排一次節流計時器——第二、三次呼叫時已經有一個
      // 排定中的，直接被擋掉（見 scheduleLiveRender 的
      // `if (liveRenderRafRef.current !== null) return;`）。只算帶
      // scheduleLiveRender 那個節流延遲（16ms）的呼叫——xterm.js 自己的
      // write() 內部也會排 setTimeout，spy 在 globalThis 上連那些一起量
      // 得到，不能直接比總呼叫次數。
      const throttleCalls = timeoutSpy.mock.calls.filter(([, delay]) => delay === 16);
      expect(throttleCalls).toHaveLength(1);
      timeoutSpy.mockRestore();

      await waitFor(() => {
        expect(result.current.blocks[0].renderedLines?.length).toBe(3);
      });
    });

    it("clearAllBlocks() 會取消排程中的 rAF，不會在區塊已經不存在之後還嘗試寫入", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("echo hi");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b]133;C\x07");
      });
      await act(async () => {
        await writeToTerm(term, "hi\r\n");
      });
      act(() => {
        result.current.appendOutput("hi\r\n");
        result.current.clearAllBlocks();
      });

      // 等超過一個動畫影格的時間，確認沒有任何東西悄悄把 blocks 填回來。
      await new Promise((r) => setTimeout(r, 50));
      expect(result.current.blocks).toHaveLength(0);
    });
  });
});
